//! Fair scores for the whole-document test: a fix counts as right when it is
//! ours or when two judge models agree it is correct, and the test passages
//! themselves are checked, so no correct answer is ever counted as wrong.
//! Works on saved replies (`bench --doc … `'s `.replies.jsonl`); every
//! verdict is cached, so a rerun only asks about what's new.

use crate::ai::{call, client, run_pool};
use crate::bench::{Kind, Passage, documents, load, squash};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use xenify_wp_doc::{Document, NodeId};

/// Something to ask the judges.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Question {
    /// Is the original really wrong, and is the expected sentence a right fix?
    Mistake { original: String, expected: String },
    /// Is this correct standard English?
    Correct { sentence: String },
    /// Is the candidate a right fix of the original?
    Fix { original: String, candidate: String },
    /// Is there really a mistake in these words of a correct sentence?
    Alarm { sentence: String, flagged: String, replacement: String },
}

impl Question {
    fn key(&self) -> String {
        serde_json::to_string(&match self {
            Question::Mistake { original, expected } => json!(["mistake", original, expected]),
            Question::Correct { sentence } => json!(["correct", sentence]),
            Question::Fix { original, candidate } => json!(["fix", original, candidate]),
            Question::Alarm { sentence, flagged, replacement } => json!(["alarm", sentence, flagged, replacement]),
        })
        .unwrap()
    }

    fn ask(&self) -> String {
        match self {
            Question::Mistake { original, expected } => format!(
                "Original: {original}\nCorrected: {expected}\nIs there a real mistake in the original (spelling, grammar, punctuation, a wrong or missing word) AND is the corrected version a right way to fix it? Yes only if both."
            ),
            Question::Correct { sentence } => {
                format!(
                    "Sentence: {sentence}\nIs this correct standard written English, with no real mistake? (Style choices are not mistakes.)"
                )
            }
            Question::Fix { original, candidate } => format!(
                "Original: {original}\nFixed: {candidate}\nIs the fixed version correct standard written English, with the same meaning, and does it change only what needed fixing (small punctuation choices are fine)?"
            ),
            Question::Alarm { sentence, flagged, replacement } => format!(
                "Sentence: {sentence}\nA checker marked \"{flagged}\" and suggested \"{replacement}\".\nIs there a real mistake in the marked words (spelling, grammar, punctuation, a wrong or missing word), not just a style preference?"
            ),
        }
    }
}

const SYSTEM: &str = "You are a careful English editor judging a grammar checker's test. Answer each numbered question \
with yes or no and a few words why. Real mistakes only: style preferences, formality and optional commas are not mistakes. \
Several fixes can be right.";

fn tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_verdicts",
        "description": "One verdict per question.",
        "parameters": {"type": "object", "required": ["verdicts"], "properties": {"verdicts": {"type": "array", "items": {
            "type": "object", "required": ["n", "yes", "why"],
            "properties": {
                "n": {"type": "integer", "description": "The question number"},
                "yes": {"type": "boolean"},
                "why": {"type": "string", "description": "A few words"}
            }}}}}
    }})
}

/// Both judges' answers to one question.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    Yes,
    No,
    /// The judges disagree (or one didn't answer): a person decides.
    Split,
}

type Cache = HashMap<(String, String), (bool, String)>;

fn read_cache(path: &Path) -> Cache {
    let mut c = Cache::new();
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line)
            && let (Some(j), Some(k), Some(y)) = (v["judge"].as_str(), v["key"].as_str(), v["yes"].as_bool())
        {
            c.insert((j.into(), k.into()), (y, v["why"].as_str().unwrap_or("").into()));
        }
    }
    c
}

/// Asks every judge every question it hasn't answered yet.
fn ask_all(questions: &[Question], judges: &[String], cache_path: &Path, budget: f64) -> Result<Cache, String> {
    let cache = Mutex::new(read_cache(cache_path));
    let c = client()?;
    for judge in judges {
        let todo: Vec<Question> = {
            let have = cache.lock().unwrap();
            questions.iter().filter(|q| !have.contains_key(&(judge.clone(), q.key()))).cloned().collect()
        };
        eprintln!("{judge}: {} questions to ask ({} cached)", todo.len(), questions.len() - todo.len());
        let batches: Vec<Vec<Question>> = todo.chunks(15).map(|b| b.to_vec()).collect();
        run_pool(
            batches,
            8,
            budget,
            |b| format!("{judge} {}", b.len()),
            |b| {
                let mut user = String::new();
                for (n, q) in b.iter().enumerate() {
                    let _ = writeln!(user, "Question {n}:\n{}\n", q.ask());
                }
                let (args, cost) = call(&c, judge, SYSTEM, &user, tool(), 0.0)?;
                let mut lines = String::new();
                let mut have = cache.lock().unwrap();
                for v in args["verdicts"].as_array().into_iter().flatten() {
                    let (Some(q), Some(yes)) = (v["n"].as_u64().and_then(|n| b.get(n as usize)), v["yes"].as_bool())
                    else {
                        continue;
                    };
                    let why = v["why"].as_str().unwrap_or("").to_string();
                    let _ = writeln!(lines, "{}", json!({"judge": judge, "key": q.key(), "yes": yes, "why": why}));
                    have.insert((judge.clone(), q.key()), (yes, why));
                }
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(cache_path)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, lines.as_bytes()))
                    .map_err(|e| e.to_string())?;
                Ok(cost)
            },
        );
    }
    Ok(cache.into_inner().unwrap())
}

fn verdict(cache: &Cache, judges: &[String], q: &Question) -> Verdict {
    let answers: Vec<bool> = judges.iter().filter_map(|j| cache.get(&(j.clone(), q.key())).map(|a| a.0)).collect();
    if answers.len() < judges.len() {
        Verdict::Split
    } else if answers.iter().all(|a| *a) {
        Verdict::Yes
    } else if answers.iter().all(|a| !*a) {
        Verdict::No
    } else {
        Verdict::Split
    }
}

/// One model's suggestions per passage of one document.
struct Reply {
    model: String,
    document: usize,
    /// Per passage: (byte range, replacement).
    found: Vec<Vec<(std::ops::Range<usize>, String)>>,
    seconds: f64,
    /// It reported issues but no fix for any of them (a provider dropping
    /// that part of the answer): counts for finding, not for fixing.
    no_fixes: bool,
}

fn read_replies(path: &Path, docs: &[Vec<Passage>]) -> Result<Vec<Reply>, String> {
    let mut out = Vec::new();
    for line in std::fs::read_to_string(path).map_err(|e| e.to_string())?.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let Some(n) = v["document"].as_u64().map(|n| n as usize).filter(|n| *n < docs.len()) else { continue };
        let d = &docs[n];
        let lines: Vec<String> = d.iter().map(|p| p.text.replace('\n', " ")).collect();
        let doc = Document::from_plain_text(&lines.join("\n"));
        let ids: Vec<NodeId> = doc.paragraphs().map(|p| p.id).collect();
        let (found, dropped) = xenify_wp_ai::parse_review(&json!({"choices": [{"message": v["reply"]}]}), &doc);
        // An empty reply isn't an answer: leave it out rather than count it
        // as "found nothing" (a reply that says "no issues" is an answer).
        let m = &v["reply"];
        let empty = m["tool_calls"].is_null() && !m["content"].as_str().unwrap_or("").contains('{');
        if empty || (found.is_empty() && dropped == 0 && v.get("part").is_none()) {
            continue;
        }
        let no_fixes = found.iter().all(|s| s.replacement.as_deref().unwrap_or("").trim().is_empty());
        let mut per = vec![Vec::new(); d.len()];
        for s in found {
            if let Some(k) = ids.iter().position(|id| *id == s.para) {
                per[k].push((s.range, s.replacement.unwrap_or_default()));
            }
        }
        let model: String = v["model"].as_str().unwrap_or("?").into();
        // Parts of one document (sent together) make one answer.
        if v["part"].as_u64().unwrap_or(0) > 0
            && let Some(r) = out.iter_mut().rev().find(|r: &&mut Reply| r.model == model && r.document == n)
        {
            for (mine, more) in r.found.iter_mut().zip(per) {
                mine.extend(more);
            }
            r.no_fixes &= no_fixes;
            continue;
        }
        out.push(Reply { model, document: n, found: per, seconds: v["seconds"].as_f64().unwrap_or(0.0), no_fixes });
    }
    Ok(out)
}

/// The suggestions on a mistake passage that touch the wrong words.
fn hits<'a>(
    p: &Passage,
    wrong: &str,
    f: &'a [(std::ops::Range<usize>, String)],
) -> Vec<&'a (std::ops::Range<usize>, String)> {
    let Some(start) = p.text.find(wrong) else { return Vec::new() };
    let span = start..start + wrong.len();
    f.iter().filter(|(r, _)| r.start <= span.end + 1 && span.start <= r.end + 1).collect()
}

fn applied(text: &str, r: &std::ops::Range<usize>, fix: &str) -> String {
    let mut t = text.to_string();
    t.replace_range(r.clone(), fix);
    t
}

#[derive(Default)]
struct Score {
    documents: usize,
    no_fix_documents: usize,
    /// Mistakes found in replies without fixes (not counted for fixing).
    caught_unfixed: usize,
    mistakes: usize,
    caught: usize,
    right: usize,
    split_fix: usize,
    correct: usize,
    alarmed: usize,
    split_alarm: usize,
    informal: usize,
    informal_changed: usize,
    seconds: Vec<f64>,
}

pub fn rescore(
    input: &Path,
    dir: &Path,
    size: usize,
    count: usize,
    judges: &[String],
    budget: f64,
) -> Result<(), String> {
    let mut docs = documents(&load(input)?, size);
    docs.truncate(count);
    let mut replies = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".replies.jsonl"))
        .collect();
    files.sort();
    for f in &files {
        replies.extend(read_replies(f, &docs)?);
    }
    eprintln!("{} replies from {} files", replies.len(), files.len());

    // Every question, without repeats.
    let mut qs: BTreeMap<String, Question> = BTreeMap::new();
    let mut add = |q: Question| {
        qs.entry(q.key()).or_insert(q);
    };
    for p in docs.iter().flatten() {
        match &p.kind {
            Kind::Mistake { corrected, .. } => {
                add(Question::Mistake { original: p.text.clone(), expected: corrected.clone() })
            }
            Kind::Clean => add(Question::Correct { sentence: p.text.clone() }),
            Kind::Informal(_) => {}
        }
    }
    for r in &replies {
        for (p, f) in docs[r.document].iter().zip(&r.found) {
            match &p.kind {
                Kind::Mistake { wrong, corrected, .. } => {
                    for (range, fix) in hits(p, wrong, f).into_iter().filter(|(_, fix)| !fix.trim().is_empty()) {
                        let candidate = applied(&p.text, range, fix);
                        if squash(&candidate) != squash(corrected) {
                            add(Question::Fix { original: p.text.clone(), candidate });
                        }
                    }
                }
                Kind::Clean => {
                    for (range, fix) in f {
                        add(Question::Alarm {
                            sentence: p.text.clone(),
                            flagged: p.text[range.clone()].to_string(),
                            replacement: fix.clone(),
                        });
                    }
                }
                Kind::Informal(_) => {}
            }
        }
    }
    let questions: Vec<Question> = qs.into_values().collect();
    let cache_path = dir.join("judged.jsonl");
    let cache = ask_all(&questions, judges, &cache_path, budget)?;
    let v = |q: &Question| verdict(&cache, judges, q);

    // Which test passages stand: a mistake both judges confirm, a correct
    // passage both judges call correct. Anything else is left out.
    let mut dropped = Vec::new();
    let keep = |p: &Passage| match &p.kind {
        Kind::Mistake { corrected, .. } => {
            v(&Question::Mistake { original: p.text.clone(), expected: corrected.clone() }) == Verdict::Yes
        }
        Kind::Clean => v(&Question::Correct { sentence: p.text.clone() }) == Verdict::Yes,
        Kind::Informal(_) => true,
    };
    for p in docs.iter().flatten() {
        if !keep(p) {
            let what = if matches!(p.kind, Kind::Clean) { "said to be correct" } else { "said to have a mistake" };
            dropped.push(format!("- ({what}) {}", p.text));
        }
    }

    let mut scores: BTreeMap<String, Score> = BTreeMap::new();
    let mut review = String::new();
    for r in &replies {
        let s = scores.entry(r.model.clone()).or_default();
        s.seconds.push(r.seconds);
        s.documents += 1;
        if r.no_fixes {
            s.no_fix_documents += 1;
        }
        for (p, f) in docs[r.document].iter().zip(&r.found) {
            if !keep(p) {
                continue;
            }
            match &p.kind {
                Kind::Mistake { wrong, corrected, .. } => {
                    s.mistakes += 1;
                    let h = hits(p, wrong, f);
                    if h.is_empty() {
                        continue;
                    }
                    s.caught += 1;
                    // Found, but no fix offered (only a note): counts for
                    // finding, not for fixing.
                    let h: Vec<_> = h.into_iter().filter(|(_, fix)| !fix.trim().is_empty()).collect();
                    if r.no_fixes || h.is_empty() {
                        s.caught_unfixed += 1;
                        continue;
                    }
                    let mut best = Verdict::No;
                    for (range, fix) in &h {
                        let candidate = applied(&p.text, range, fix);
                        let this = if squash(&candidate) == squash(corrected) {
                            Verdict::Yes
                        } else {
                            v(&Question::Fix { original: p.text.clone(), candidate: candidate.clone() })
                        };
                        if this == Verdict::Split {
                            let _ = writeln!(
                                review,
                                "- fix ({}) {}  \n  → {candidate}  \n  ours: {corrected}",
                                r.model, p.text
                            );
                        }
                        if this == Verdict::Yes || (this == Verdict::Split && best == Verdict::No) {
                            best = this;
                        }
                    }
                    match best {
                        Verdict::Yes => s.right += 1,
                        Verdict::Split => s.split_fix += 1,
                        Verdict::No => {}
                    }
                }
                Kind::Clean => {
                    s.correct += 1;
                    // A false alarm unless both judges say the marked words
                    // really are wrong (then nobody's to blame: say so).
                    let mut worst = Verdict::Yes;
                    for (range, fix) in f {
                        let q = Question::Alarm {
                            sentence: p.text.clone(),
                            flagged: p.text[range.clone()].to_string(),
                            replacement: fix.clone(),
                        };
                        match v(&q) {
                            Verdict::No => worst = Verdict::No,
                            Verdict::Split if worst == Verdict::Yes => {
                                worst = Verdict::Split;
                                let _ = writeln!(
                                    review,
                                    "- alarm ({}) {}  \n  marked **{}** → **{fix}**",
                                    r.model,
                                    p.text,
                                    &p.text[range.clone()]
                                );
                            }
                            _ => {}
                        }
                    }
                    match worst {
                        Verdict::No => s.alarmed += 1,
                        Verdict::Split => s.split_alarm += 1,
                        Verdict::Yes => {}
                    }
                }
                Kind::Informal(_) => {
                    s.informal += 1;
                    if !f.is_empty() {
                        s.informal_changed += 1;
                    }
                }
            }
        }
    }

    let pct =
        |a: usize, b: usize| if b == 0 { "–".to_string() } else { format!("{:.0}%", 100.0 * a as f64 / b as f64) };
    let mut md = format!(
        "# Whole-document test, judged\n\n{} documents of {size} passages sent whole with Write's AI check prompt. \
         A fix is right when it gives our corrected sentence or when both judges ({}) say it is correct English with the \
         same meaning. A flag on a correct sentence is a false alarm when both judges say the marked words are fine; \
         passages the judges didn't both confirm are left out ({} of {}). Unsure: the judges disagreed (listed for a person \
         in review.md). Answered: documents with a usable reply (empty replies are left out). Right fix is out of the \
         mistakes found with a fix offered; found with no fix (a note only) is shown on its own.\n\n| Model | Answered | Found | Found, no fix offered | Right fix (of fixes offered) | Unsure fixes | False alarms | Unsure alarms | Changed informal writing | Seconds per document |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
        docs.len(),
        judges.join(", "),
        dropped.len(),
        docs.iter().flatten().filter(|p| !matches!(p.kind, Kind::Informal(_))).count()
    );
    let mut rows: Vec<_> = scores.into_iter().collect();
    // Best first: found and fixed, as a share of the mistakes in answered documents.
    let rank = |s: &Score| s.right * 1000 / s.mistakes.max(1) + s.caught_unfixed * 1000 / s.mistakes.max(1);
    rows.sort_by_key(|r| std::cmp::Reverse(rank(&r.1)));
    for (model, mut s) in rows {
        s.seconds.sort_by(f64::total_cmp);
        let _ = writeln!(
            md,
            "| {model} | {} | {} | {} | {} | {} | {} | {} | {} | {:.0} |",
            if s.no_fix_documents > 0 {
                format!("{}/{count} ({} without fixes)", s.documents, s.no_fix_documents)
            } else {
                format!("{}/{count}", s.documents)
            },
            pct(s.caught, s.mistakes),
            pct(s.caught_unfixed, s.caught),
            pct(s.right, s.caught - s.caught_unfixed),
            s.split_fix,
            pct(s.alarmed, s.correct),
            s.split_alarm,
            pct(s.informal_changed, s.informal),
            s.seconds.get(s.seconds.len() / 2).copied().unwrap_or(0.0)
        );
    }
    std::fs::write(dir.join("judged.md"), &md).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("review.md"),
        format!(
            "# For a person to decide\n\n## Judges disagreed\n\n{review}\n## Test passages left out\n\n{}\n",
            dropped.join("\n")
        ),
    )
    .map_err(|e| e.to_string())?;
    println!("{md}");
    Ok(())
}
