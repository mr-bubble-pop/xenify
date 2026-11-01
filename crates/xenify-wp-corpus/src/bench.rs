//! How well do language models proofread? Runs models over the frozen test
//! set (passages with one mistake each, the same passages corrected, and
//! clean informal passages) and scores them like the rule checker, alone
//! and combined with it.

use crate::ai::{call, client, run_pool};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;
use xenify_wp_check::{Category, Checker, Ignores, Settings};
use xenify_wp_doc::NodeId;

/// One passage to check.
#[derive(Clone)]
pub(crate) struct Passage {
    pub(crate) id: usize,
    pub(crate) text: String,
    pub(crate) kind: Kind,
}

#[derive(Clone, PartialEq)]
pub(crate) enum Kind {
    /// Has one mistake: (wrong words, corrected passage, mistake type).
    Mistake { wrong: String, corrected: String, error_type: String },
    /// Correct standard English (a corrected passage).
    Clean,
    /// Correct informal English (a register).
    Informal(String),
}

const SYSTEM: &str = "You are a careful proofreader. Find genuine mistakes in spelling, grammar, punctuation and word choice \
(wrong word, missing word, wrong word order). Do not report style preferences, and do not treat deliberate informal language, \
slang, texting style or dialect as mistakes. If a passage has no mistakes, report nothing for it.";

fn tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_mistakes",
        "description": "Report every mistake found, for all passages.",
        "parameters": {"type": "object", "required": ["mistakes"], "properties": {"mistakes": {"type": "array", "items": {
            "type": "object", "required": ["passage", "quote", "fix"],
            "properties": {
                "passage": {"type": "integer", "description": "The passage number"},
                "quote": {"type": "string", "description": "The wrong words, copied exactly (a few words)"},
                "fix": {"type": "string", "description": "What they should be"}
            }}}}}
    }})
}

pub(crate) fn load(path: &Path) -> Result<Vec<Passage>, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in s.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let text = v["text"].as_str().unwrap_or_default().to_string();
        if let (Some(e), Some(w), Some(c)) =
            (v["error_type"].as_str(), v["mistakes"][0]["wrong"].as_str(), v["corrected"].as_str())
        {
            out.push(Passage {
                id: out.len(),
                text,
                kind: Kind::Mistake { wrong: w.into(), corrected: c.into(), error_type: e.into() },
            });
            out.push(Passage { id: out.len(), text: c.to_string(), kind: Kind::Clean });
        } else if v["register"] != "standard" {
            out.push(Passage {
                id: out.len(),
                text,
                kind: Kind::Informal(v["register"].as_str().unwrap_or_default().into()),
            });
        }
    }
    Ok(out)
}

pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A reported mistake placed in its passage: (byte range, fix).
type Found = (std::ops::Range<usize>, String);

fn place(text: &str, quote: &str) -> Option<std::ops::Range<usize>> {
    let q = quote.trim().trim_matches('"');
    if q.is_empty() {
        return None;
    }
    text.find(q).map(|i| i..i + q.len()).or_else(|| {
        let (lt, lq) = (text.to_lowercase(), q.to_lowercase());
        (lt.len() == text.len()).then(|| lt.find(&lq).map(|i| i..i + lq.len())).flatten()
    })
}

/// Scores of one checker (a model, the rules, or both).
#[derive(Default)]
struct Tally {
    mistakes: usize,
    caught: usize,
    fixed: usize,
    clean: usize,
    clean_flagged: usize,
    informal: usize,
    informal_flagged: usize,
    by_type: HashMap<String, (usize, usize)>,
}

impl Tally {
    fn add(&mut self, p: &Passage, found: &[Found]) {
        match &p.kind {
            Kind::Mistake { wrong, corrected, error_type } => {
                self.mistakes += 1;
                let t = self.by_type.entry(error_type.clone()).or_default();
                t.0 += 1;
                let Some(start) = p.text.find(wrong) else { return };
                let span = start..start + wrong.len();
                let hits: Vec<&Found> =
                    found.iter().filter(|(r, _)| r.start <= span.end + 1 && span.start <= r.end + 1).collect();
                if !hits.is_empty() {
                    self.caught += 1;
                    t.1 += 1;
                }
                if hits.iter().any(|(r, fix)| {
                    let mut s = p.text.clone();
                    s.replace_range(r.clone(), fix);
                    squash(&s) == squash(corrected)
                }) {
                    self.fixed += 1;
                }
            }
            Kind::Clean => {
                self.clean += 1;
                if !found.is_empty() {
                    self.clean_flagged += 1;
                }
            }
            Kind::Informal(_) => {
                self.informal += 1;
                if !found.is_empty() {
                    self.informal_flagged += 1;
                }
            }
        }
    }

    fn row(&self, name: &str, extra: &str) -> String {
        let pct = |a: usize, b: usize| {
            if b == 0 { "–".to_string() } else { format!("{:.0}%", 100.0 * a as f64 / b as f64) }
        };
        format!(
            "| {name} | {} | {} | {} | {} |{extra}",
            pct(self.caught, self.mistakes),
            pct(self.fixed, self.mistakes),
            pct(self.clean_flagged, self.clean),
            pct(self.informal_flagged, self.informal)
        )
    }
}

/// The rule checker's findings per passage (Spelling/Grammar/Style/Register on
/// mistakes, Spelling/Grammar elsewhere — as in the scoreboard).
fn rules(passages: &[Passage]) -> HashMap<usize, Vec<Found>> {
    let mut c = Checker::new(Settings::default(), &[]);
    passages
        .iter()
        .map(|p| {
            let issues = c.check_paragraph(NodeId(0), &p.text, &Ignores::default());
            let keep = |cat: Category| match p.kind {
                Kind::Mistake { .. } => !matches!(cat, Category::Readability | Category::Register),
                _ => matches!(cat, Category::Spelling | Category::Grammar),
            };
            (
                p.id,
                issues
                    .into_iter()
                    .filter(|i| keep(i.category))
                    .map(|i| (i.range, i.suggestions.first().cloned().unwrap_or_default()))
                    .collect(),
            )
        })
        .collect()
}

/// Per mistake kind: (passages, found).
type TypeTally = HashMap<String, (usize, usize)>;

pub fn bench(
    input: &Path,
    models: &[String],
    out: &Path,
    batch: usize,
    budget: f64,
    threads: usize,
) -> Result<(), String> {
    let passages = load(input)?;
    eprintln!("{} passages", passages.len());
    let by_rules = rules(&passages);
    let mut md = String::from(
        "# Proofreading benchmark\n\nCaught: share of mistakes found (anywhere on the wrong words). Right fix: the suggested fix gives the corrected passage. \
         False alarms: share of correct passages with anything reported. Informal flagged: share of clean informal passages \
         (slang, texting, dialect) with anything reported.\n\n| Checker | Caught | Right fix | False alarms (standard) | Informal flagged | Cost | Seconds per passage |\n|---|---:|---:|---:|---:|---:|---:|\n",
    );
    let mut t = Tally::default();
    for p in &passages {
        t.add(p, &by_rules[&p.id]);
    }
    let _ = writeln!(md, "{}", t.row("Xenify rules (offline)", " – | – |"));
    let mut per_type: Vec<(String, TypeTally)> = vec![("rules".into(), t.by_type)];
    let c = client()?;
    for model in models {
        eprintln!("== {model}");
        let found: Mutex<HashMap<usize, Vec<Found>>> = Mutex::new(HashMap::new());
        let batches: Vec<Vec<Passage>> = passages.chunks(batch).map(|c| c.to_vec()).collect();
        let started = Instant::now();
        let spent = run_pool(
            batches,
            threads,
            budget,
            |b| format!("{model} #{}", b[0].id),
            |b| {
                let mut user = String::from("Check each passage. Report each mistake with its passage number.\n\n");
                for (n, p) in b.iter().enumerate() {
                    let _ = writeln!(user, "Passage {n}: {}", p.text.replace('\n', " "));
                }
                let (args, cost) = call(&c, model, SYSTEM, &user, tool(), 0.0)?;
                let mut f = found.lock().unwrap();
                for p in b {
                    f.entry(p.id).or_default();
                }
                for m in args["mistakes"].as_array().into_iter().flatten() {
                    let Some(p) = m["passage"].as_u64().and_then(|n| b.get(n as usize)) else { continue };
                    if let Some(r) = place(&p.text, m["quote"].as_str().unwrap_or_default()) {
                        f.entry(p.id).or_default().push((r, m["fix"].as_str().unwrap_or_default().to_string()));
                    }
                }
                Ok(cost)
            },
        );
        let secs = started.elapsed().as_secs_f64();
        let found = found.into_inner().unwrap();
        let answered = passages.iter().filter(|p| found.contains_key(&p.id)).count();
        if answered < passages.len() / 2 {
            let _ = writeln!(
                md,
                "| {model} | (failed: {answered} of {} passages answered) | | | | ${spent:.2} | |",
                passages.len()
            );
            continue;
        }
        let (mut alone, mut both) = (Tally::default(), Tally::default());
        for p in passages.iter().filter(|p| found.contains_key(&p.id)) {
            let f = &found[&p.id];
            alone.add(p, f);
            let mut all = f.clone();
            all.extend(by_rules[&p.id].iter().cloned());
            both.add(p, &all);
        }
        let speed = secs * threads as f64 / answered as f64;
        let note = if answered < passages.len() {
            format!(" ({answered}/{} answered)", passages.len())
        } else {
            String::new()
        };
        let _ = writeln!(md, "{}", alone.row(&format!("{model}{note}"), &format!(" ${spent:.2} | {speed:.2} |")));
        let _ = writeln!(md, "{}", both.row(&format!("{model} + rules"), " | |"));
        per_type.push((model.clone(), alone.by_type));
        std::fs::write(out, &md).map_err(|e| e.to_string())?;
    }
    // Caught per kind of mistake.
    let _ = write!(md, "\n## Caught by kind of mistake\n\n| Kind |");
    for (name, _) in &per_type {
        let _ = write!(md, " {} |", name.rsplit('/').next().unwrap_or(name));
    }
    let _ = write!(md, "\n|---|{}\n", "---:|".repeat(per_type.len()));
    let mut kinds: Vec<&String> = per_type[0].1.keys().collect();
    kinds.sort();
    for k in kinds {
        let _ = write!(md, "| {k} |");
        for (_, m) in &per_type {
            let (n, c) = m.get(k).copied().unwrap_or_default();
            let _ =
                write!(md, " {} |", if n == 0 { "–".into() } else { format!("{:.0}%", 100.0 * c as f64 / n as f64) });
        }
        md.push('\n');
    }
    std::fs::write(out, &md).map_err(|e| e.to_string())?;
    println!("{md}");
    Ok(())
}

// ---------------------------------------------------------------------------
// "Told it's wrong": can a model fix a passage when told it may have a mistake?

fn corrections_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_corrections",
        "description": "The corrected passages, in order.",
        "parameters": {"type": "object", "required": ["passages"], "properties": {"passages": {"type": "array", "items": {
            "type": "object", "required": ["n", "corrected"],
            "properties": {"n": {"type": "integer"}, "corrected": {"type": "string", "description": "The passage corrected with as few changes as possible, or unchanged if it has no mistake"}}}}}}
    }})
}

const HINT_SYSTEM: &str = "You are a careful proofreader. Each passage was flagged because it may contain one mistake \
(spelling, grammar, punctuation, a wrong or missing word, word order). Correct it with as few changes as possible. \
If it has no mistake, return it exactly as it is. Don't rephrase, don't change style, and don't treat informal language or dialect as mistakes.";

/// Normalises for comparison: single spaces, straight quotes.
fn norm(s: &str) -> String {
    squash(&s.replace(['\u{2019}', '\u{2018}'], "'").replace(['\u{201c}', '\u{201d}'], "\""))
}

/// The byte range of `a` that differs from `b` (after the common prefix and suffix).
fn changed(a: &str, b: &str) -> std::ops::Range<usize> {
    let p = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let s = a.bytes().rev().zip(b.bytes().rev()).take_while(|(x, y)| x == y).count();
    let end = a.len().saturating_sub(s).max(p);
    p..end
}

/// Scores of the hinted test.
#[derive(Default)]
struct Hinted {
    mistakes: usize,
    exact: usize,
    right_place: usize,
    clean: usize,
    left_alone: usize,
}

pub fn bench_hint(
    input: &Path,
    models: &[String],
    out: &Path,
    batch: usize,
    budget: f64,
    threads: usize,
) -> Result<(), String> {
    let passages: Vec<Passage> = load(input)?.into_iter().filter(|p| !matches!(p.kind, Kind::Informal(_))).collect();
    eprintln!("{} passages", passages.len());
    let mut md = String::from(
        "# Told-it's-wrong benchmark\n\nEach passage is presented as \"may contain a mistake\". Exact fix: the output equals the corrected passage. \
         Right place: the model changed the words where the mistake was. Left alone: a correct passage came back unchanged.\n\n\
         | Model | Exact fix | Right place | Correct passages left alone | Cost |\n|---|---:|---:|---:|---:|\n",
    );
    let c = client()?;
    for model in models {
        eprintln!("== {model}");
        let outs: Mutex<HashMap<usize, String>> = Mutex::new(HashMap::new());
        let batches: Vec<Vec<Passage>> = passages.chunks(batch).map(|c| c.to_vec()).collect();
        let spent = run_pool(
            batches,
            threads,
            budget,
            |b| format!("{model} #{}", b[0].id),
            |b| {
                let mut user = String::from("Correct each passage (or return it unchanged).\n\n");
                for (n, p) in b.iter().enumerate() {
                    let _ = writeln!(user, "Passage {n}: {}", p.text.replace('\n', " "));
                }
                let (args, cost) = call(&c, model, HINT_SYSTEM, &user, corrections_tool(), 0.0)?;
                let mut o = outs.lock().unwrap();
                for r in args["passages"].as_array().into_iter().flatten() {
                    if let (Some(n), Some(t)) = (r["n"].as_u64(), r["corrected"].as_str())
                        && let Some(p) = b.get(n as usize)
                    {
                        o.insert(p.id, t.to_string());
                    }
                }
                Ok(cost)
            },
        );
        let outs = outs.into_inner().unwrap();
        let answered = outs.len();
        if answered < passages.len() / 2 {
            let _ = writeln!(md, "| {model} | (failed: {answered} of {} answered) | | | ${spent:.2} |", passages.len());
            std::fs::write(out, &md).map_err(|e| e.to_string())?;
            continue;
        }
        let mut h = Hinted::default();
        for p in &passages {
            let Some(o) = outs.get(&p.id) else { continue };
            match &p.kind {
                Kind::Mistake { wrong, corrected, .. } => {
                    h.mistakes += 1;
                    if norm(o) == norm(corrected) {
                        h.exact += 1;
                    }
                    if let Some(start) = p.text.find(wrong.as_str()) {
                        let r = changed(&p.text, o);
                        if !r.is_empty() && r.start <= start + wrong.len() + 1 && start <= r.end + 1
                            || norm(o) == norm(corrected)
                        {
                            h.right_place += 1;
                        }
                    }
                }
                Kind::Clean => {
                    h.clean += 1;
                    if norm(o) == norm(&p.text) {
                        h.left_alone += 1;
                    }
                }
                Kind::Informal(_) => {}
            }
        }
        let pct = |a: usize, b: usize| format!("{:.0}%", 100.0 * a as f64 / b.max(1) as f64);
        let note = if answered < passages.len() {
            format!(" ({answered}/{} answered)", passages.len())
        } else {
            String::new()
        };
        let _ = writeln!(
            md,
            "| {model}{note} | {} | {} | {} | ${spent:.2} |",
            pct(h.exact, h.mistakes),
            pct(h.right_place, h.mistakes),
            pct(h.left_alone, h.clean)
        );
        std::fs::write(out, &md).map_err(|e| e.to_string())?;
    }
    println!("{md}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Whole documents: the way Write's AI check sends them (one request, its own
// prompt), so models are scored on what people get, and timed on real GPUs.

/// Where to send the documents: OpenRouter (the AI settings), or any
/// OpenAI-style address such as LocalAI on a rented GPU.
pub struct Endpoint {
    pub url: Option<String>,
    /// The environment variable holding the key for `url`.
    pub key_env: Option<String>,
    /// Send each document in parts of about this many words, all at once.
    pub parts: Option<usize>,
    /// Ask the model not to think first.
    pub no_thinking: bool,
}

/// A small fixed shuffle, so every run builds the same documents.
fn shuffle<T>(v: &mut [T], mut seed: u64) {
    for i in (1..v.len()).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v.swap(i, (seed >> 33) as usize % (i + 1));
    }
}

/// Documents of `size` passages: about half with a mistake, the rest correct
/// (standard and informal). A passage and its own corrected copy never meet.
pub(crate) fn documents(passages: &[Passage], size: usize) -> Vec<Vec<Passage>> {
    let (mut wrong, mut clean, mut informal) = (Vec::new(), Vec::new(), Vec::new());
    let mut nth = 0;
    for p in passages {
        match p.kind {
            // Mistakes and their corrected copies come in pairs: the even
            // pairs give mistakes, the odd ones correct passages.
            Kind::Mistake { .. } => {
                if nth % 2 == 0 {
                    wrong.push(p.clone());
                }
            }
            Kind::Clean => {
                if nth % 2 == 1 {
                    clean.push(p.clone());
                }
                nth += 1;
            }
            Kind::Informal(_) => informal.push(p.clone()),
        }
    }
    shuffle(&mut wrong, 1);
    shuffle(&mut clean, 2);
    shuffle(&mut informal, 3);
    let (nw, ni) = (size / 2, size / 8);
    let mut docs = Vec::new();
    let (mut w, mut c, mut i) = (wrong.into_iter(), clean.into_iter(), informal.into_iter());
    loop {
        let mut d: Vec<Passage> = w.by_ref().take(nw).collect();
        if d.len() < nw {
            break;
        }
        d.extend(i.by_ref().take(ni));
        let rest = size - d.len();
        d.extend(c.by_ref().take(rest));
        if d.len() < size {
            break;
        }
        shuffle(&mut d, docs.len() as u64 + 7);
        docs.push(d);
    }
    docs
}

/// Sends a review, as Write does, with a few tries for network trouble. A
/// model whose providers can't fill in the tool is asked for the same JSON
/// in plain text instead.
fn ask(c: &xenify_openrouter::OpenRouter, body: &Value) -> Result<Value, String> {
    use xenify_openrouter::AiError;
    let mut body = body.clone();
    let mut tries = 0;
    loop {
        match c.chat(&body) {
            Ok(r) => return Ok(r),
            Err(AiError::Api { status: 400 | 404, .. }) if body.get("tools").is_some() => {
                body = xenify_wp_ai::without_tools(&body);
            }
            // A model that took the whole time once would only take it again.
            Err(AiError::Network(e)) if e.contains("timeout") => return Err(format!("network error: {e}")),
            Err(e @ (AiError::Network(_) | AiError::Api { status: 429 | 500..=599, .. })) => {
                tries += 1;
                if tries >= 4 {
                    return Err(e.to_string());
                }
                std::thread::sleep(std::time::Duration::from_secs(5 * tries));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// One document's answer: its number, what was found per passage, tokens and cost.
type DocResult = (usize, Vec<Vec<Found>>, usize, f64);

#[allow(clippy::too_many_arguments)]
pub fn bench_doc(
    input: &Path,
    models: &[String],
    out: &Path,
    size: usize,
    count: usize,
    budget: f64,
    threads: usize,
    at: &Endpoint,
) -> Result<(), String> {
    use xenify_wp_ai::{Focus, Voice};
    use xenify_wp_doc::Document;
    let mut docs = documents(&load(input)?, size);
    docs.truncate(count);
    let words: usize =
        docs.iter().flatten().map(|p| p.text.split_whitespace().count()).sum::<usize>() / docs.len().max(1);
    eprintln!("{} documents of {size} passages (about {words} words each)", docs.len());
    let c = match &at.url {
        Some(url) => {
            let key = at.key_env.as_deref().and_then(|k| std::env::var(k).ok()).unwrap_or_default();
            xenify_openrouter::OpenRouter::with_key(url, &key)
        }
        None => client()?,
    };
    let mut md = format!(
        "# Whole-document benchmark\n\n{} documents of {size} passages, about {words} words each, about half with one mistake. \
         Each is sent whole with Write's own AI check prompt (focus: grammar and spelling; at most 25 issues).\n\n\
         Caught: share of mistakes found. Right fix: the fix gives the corrected passage. False alarms: share of correct \
         passages with anything reported. Informal flagged: the same for slang, texting and dialect. Not placed: reported \
         words that aren't in the text. Seconds: per document, from asking to the whole answer.\n\n\
         | Model | Caught | Right fix | False alarms (standard) | Informal flagged | Not placed | Seconds per document | Cost |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|\n",
        docs.len()
    );
    for model in models {
        eprintln!("== {model}");
        let results: Mutex<Vec<DocResult>> = Mutex::new(Vec::new());
        let jobs: Vec<(usize, Vec<Passage>)> = docs.iter().cloned().enumerate().collect();
        let spent = run_pool(
            jobs,
            threads,
            budget,
            |(n, _)| format!("{model} document {n}"),
            |(n, d)| {
                let text: Vec<String> = d.iter().map(|p| p.text.replace('\n', " ")).collect();
                let doc = Document::from_plain_text(&text.join("\n"));
                let ids: Vec<NodeId> = doc.paragraphs().map(|p| p.id).collect();
                if ids.len() != d.len() {
                    return Err(format!("{} paragraphs for {} passages", ids.len(), d.len()));
                }
                // The whole document at once, or in parts sent together (as Write does).
                let parts = match at.parts {
                    Some(words) => xenify_wp_ai::review_parts(&doc, &ids, words),
                    None => vec![ids.clone()],
                };
                let started = Instant::now();
                let replies: Vec<Result<Value, String>> = std::thread::scope(|sc| {
                    let handles: Vec<_> = parts
                        .iter()
                        .map(|part| {
                            let (c, doc) = (&c, &doc);
                            sc.spawn(move || {
                                let mut body = xenify_wp_ai::review_request(
                                    model,
                                    doc,
                                    part,
                                    &[Focus::Grammar],
                                    &Voice::default(),
                                );
                                if at.no_thinking {
                                    // OpenRouter's form; LocalAI and Ollama honour only
                                    // reasoning_effort, and Ollama thinks again when
                                    // OpenRouter's form is there too.
                                    if at.url.is_some() {
                                        body["reasoning_effort"] = json!("none");
                                    } else {
                                        body["reasoning"] = json!({"enabled": false});
                                    }
                                }
                                ask(c, &body)
                            })
                        })
                        .collect();
                    handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("crashed".into()))).collect()
                });
                let secs = started.elapsed().as_secs_f64();
                let (mut found, mut dropped, mut cost, mut answered) = (Vec::new(), 0, 0.0, false);
                for (k, r) in replies.iter().enumerate() {
                    let Ok(resp) = r else { continue };
                    let (f, d) = xenify_wp_ai::parse_review(resp, &doc);
                    let m = &resp["choices"][0]["message"];
                    answered |= !f.is_empty()
                        || d > 0
                        || !m["tool_calls"].is_null()
                        || m["content"].as_str().unwrap_or("").contains('{');
                    found.extend(f);
                    dropped += d;
                    cost += resp["usage"]["cost"].as_f64().unwrap_or(0.0);
                    // Every reply beside the results, to see why a model scored as it did.
                    let line = json!({"model": model, "document": n, "part": k, "seconds": secs, "reply": m});
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(out.with_extension("replies.jsonl"))
                        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes()));
                }
                if let Some(Err(e)) = replies.iter().find(|r| r.is_err()) {
                    return Err(e.clone());
                }
                if !answered {
                    return Err("no answer in the reply".into());
                }
                let mut per: Vec<Vec<Found>> = vec![Vec::new(); d.len()];
                for s in found {
                    if let Some(k) = ids.iter().position(|id| *id == s.para) {
                        per[k].push((s.range, s.replacement.unwrap_or_default()));
                    }
                }
                results.lock().unwrap().push((*n, per, dropped, secs));
                Ok(cost)
            },
        );
        let results = results.into_inner().unwrap();
        if results.is_empty() {
            let _ = writeln!(md, "| {model} | (failed: no document answered) | | | | | | ${spent:.2} |");
            std::fs::write(out, &md).map_err(|e| e.to_string())?;
            continue;
        }
        let mut t = Tally::default();
        let (mut dropped, mut secs) = (0, Vec::new());
        let mut alarms = format!("# False alarms: {model}\n\nCorrect passages with something reported.\n\n");
        for (n, per, d, s) in &results {
            for (p, f) in docs[*n].iter().zip(per) {
                t.add(p, f);
                if matches!(p.kind, Kind::Clean | Kind::Informal(_)) {
                    for (r, fix) in f {
                        let kind = if p.kind == Kind::Clean { "standard" } else { "informal" };
                        let _ = writeln!(alarms, "- ({kind}) {}  \n  **{}** → **{fix}**", p.text, &p.text[r.clone()]);
                    }
                }
            }
            dropped += d;
            secs.push(*s);
        }
        secs.sort_by(f64::total_cmp);
        let note = if results.len() < docs.len() {
            format!(" ({}/{} answered)", results.len(), docs.len())
        } else {
            String::new()
        };
        let _ = writeln!(
            md,
            "{}",
            t.row(&format!("{model}{note}"), &format!(" {dropped} | {:.1} | ${spent:.2} |", secs[secs.len() / 2]))
        );
        std::fs::write(out, &md).map_err(|e| e.to_string())?;
        let name = model.replace('/', "_");
        std::fs::write(out.with_file_name(format!("{name}.false-alarms.md")), alarms).map_err(|e| e.to_string())?;
    }
    println!("{md}");
    Ok(())
}

/// Examples from a saved whole-document run (its `.replies.jsonl`), without
/// asking the model again: false alarms, wrong fixes and misses.
pub fn samples(input: &Path, replies: &Path, size: usize, per: usize) -> Result<(), String> {
    use xenify_wp_doc::Document;
    let docs = documents(&load(input)?, size);
    let text = std::fs::read_to_string(replies).map_err(|e| e.to_string())?;
    let (mut alarms, mut wrong, mut missed) = (Vec::new(), Vec::new(), Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let Some(d) = v["document"].as_u64().and_then(|n| docs.get(n as usize)) else { continue };
        let lines: Vec<String> = d.iter().map(|p| p.text.replace('\n', " ")).collect();
        let doc = Document::from_plain_text(&lines.join("\n"));
        let ids: Vec<NodeId> = doc.paragraphs().map(|p| p.id).collect();
        let resp = json!({"choices": [{"message": v["reply"]}]});
        let (found, _) = xenify_wp_ai::parse_review(&resp, &doc);
        for (k, p) in d.iter().enumerate() {
            let mine: Vec<_> = found.iter().filter(|s| s.para == ids[k]).collect();
            let shown = |s: &&xenify_wp_ai::Suggestion| {
                format!("**{}** → **{}**", &p.text[s.range.clone()], s.replacement.as_deref().unwrap_or("(no fix)"))
            };
            match &p.kind {
                Kind::Mistake { wrong: w, corrected, .. } => {
                    let Some(start) = p.text.find(w.as_str()) else { continue };
                    let span = start..start + w.len();
                    let hits: Vec<_> = mine
                        .iter()
                        .filter(|s| s.range.start <= span.end + 1 && span.start <= s.range.end + 1)
                        .collect();
                    if hits.is_empty() {
                        missed.push(format!("- {}  \n  expected: {corrected}", p.text));
                    } else if !hits.iter().any(|s| {
                        let mut t = p.text.clone();
                        t.replace_range(s.range.clone(), s.replacement.as_deref().unwrap_or(""));
                        squash(&t) == squash(corrected)
                    }) {
                        wrong.push(format!(
                            "- {}  \n  model: {}  \n  expected: {corrected}",
                            p.text,
                            hits.iter().map(|s| shown(s)).collect::<Vec<_>>().join("; ")
                        ));
                    }
                }
                Kind::Clean | Kind::Informal(_) => {
                    for s in &mine {
                        let kind = if p.kind == Kind::Clean { "standard" } else { "informal" };
                        alarms.push(format!("- ({kind}) {}  \n  model: {}  ({})", p.text, shown(s), s.explanation));
                    }
                }
            }
        }
    }
    let pick = |v: &[String]| {
        // Spread across the run rather than the first document only.
        let step = (v.len() / per.max(1)).max(1);
        v.iter().step_by(step).take(per).cloned().collect::<Vec<_>>().join("\n")
    };
    println!(
        "## False alarms ({} in all)\n\n{}\n\n## Found, but not our fix ({} in all)\n\n{}\n\n## Missed ({} in all)\n\n{}",
        alarms.len(),
        pick(&alarms),
        wrong.len(),
        pick(&wrong),
        missed.len(),
        pick(&missed)
    );
    Ok(())
}
