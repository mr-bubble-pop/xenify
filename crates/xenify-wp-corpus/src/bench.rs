//! How well do language models proofread? Runs models over the frozen test
//! set (passages with one mistake each, the same passages corrected, and
//! clean informal passages) and scores them like the rule checker, alone
//! and combined with it.

use crate::ai::{call, client, run_pool};
use xenify_wp_check::{Category, Checker, Ignores, Settings};
use xenify_wp_doc::NodeId;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// One passage to check.
#[derive(Clone)]
struct Passage {
    id: usize,
    text: String,
    kind: Kind,
}

#[derive(Clone, PartialEq)]
enum Kind {
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

fn load(path: &Path) -> Result<Vec<Passage>, String> {
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

fn squash(s: &str) -> String {
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
