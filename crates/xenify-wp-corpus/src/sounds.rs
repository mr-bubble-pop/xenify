//! Sound-alike words: generating correct example sentences for every word of
//! every confusable set, training the context model, and scoring it.

use crate::ai::{call, client, run_pool};
use xenify_wp_check::sound_alike::{self, Model, Threshold};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// One example sentence using `word` (of set `set`) correctly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Example {
    pub set: usize,
    pub word: String,
    pub text: String,
    /// Request it came from (for resuming).
    pub job: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

fn read(path: &Path) -> Result<Vec<Example>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| e.to_string()))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn append(path: &Path, items: &[Example]) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    for it in items {
        writeln!(f, "{}", serde_json::to_string(it).unwrap()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Whole-word, case-insensitive match (curly apostrophes count as straight).
fn uses(text: &str, word: &str) -> bool {
    sound_alike::tokens(text).iter().any(|(_, w)| w == word)
}

const SETTINGS: &[&str] = &[
    "a work email",
    "a school essay",
    "a social media post",
    "a news report",
    "a short story",
    "a product review",
    "technical instructions",
    "a text message to a friend",
    "a recipe",
    "a travel blog",
    "a formal letter",
    "a sports report",
];

fn sentences_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_sentences",
        "description": "Return the sentences.",
        "parameters": {"type": "object", "required": ["sentences"], "properties": {"sentences": {"type": "array", "items": {"type": "string"}}}}
    }})
}

pub fn generate(
    out: &Path,
    model: &str,
    per_word: usize,
    batch: usize,
    budget: f64,
    threads: usize,
) -> Result<(), String> {
    let have: HashSet<String> = read(out)?.into_iter().map(|e| e.job).collect();
    let mut jobs = Vec::new();
    for (si, set) in sound_alike::sets().iter().enumerate() {
        for m in set {
            for round in 0..per_word.div_ceil(batch) {
                let key = format!("{si}:{}:{round}", m.word);
                if !have.contains(&key) {
                    jobs.push((si, m.clone(), round, key));
                }
            }
        }
    }
    eprintln!("{} requests to make", jobs.len());
    let c = client()?;
    let file = Mutex::new(());
    let spent = run_pool(
        jobs,
        threads,
        budget,
        |j| j.3.clone(),
        |(si, m, round, key)| {
            let others: Vec<String> = sound_alike::sets()[*si]
                .iter()
                .filter(|o| o.word != m.word)
                .map(|o| format!("\"{}\"", o.word))
                .collect();
            let settings: Vec<&str> = SETTINGS.iter().cycle().skip(round * 4).take(4).copied().collect();
            let user = format!(
                "Write {batch} different, natural sentences (8 to 25 words each) that use the word \"{}\" correctly, meaning: {}.\n\
             Spell it exactly \"{}\" (capitalised only at the start of a sentence). Don't use {} in the same sentence.\n\
             Mix these kinds of writing: {}. Vary where the word comes in the sentence, the topic and the words around it; \
             include everyday and less common uses of this meaning. Every sentence must be correct standard English.",
                m.word,
                m.meaning,
                m.word,
                others.join(" or "),
                settings.join(", ")
            );
            let (args, cost) = call(
                &c,
                model,
                "You write example sentences for an English usage dictionary.",
                &user,
                sentences_tool(),
                0.9,
            )?;
            let items: Vec<Example> = args["sentences"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .map(str::trim)
                        .filter(|s| uses(s, &m.word))
                        .map(|s| Example {
                            set: *si,
                            word: m.word.clone(),
                            text: s.to_string(),
                            job: key.clone(),
                            verified: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            if items.is_empty() {
                return Err("no usable sentences".into());
            }
            let _g = file.lock().unwrap();
            append(out, &items)?;
            Ok(cost)
        },
    );
    eprintln!("spent ${spent:.4}");
    Ok(())
}

fn verdicts_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_verdicts",
        "description": "One verdict per sentence, in order.",
        "parameters": {"type": "object", "required": ["verdicts"], "properties": {"verdicts": {"type": "array", "items": {
            "type": "object", "required": ["n", "ok"], "properties": {"n": {"type": "integer"}, "ok": {"type": "boolean"}}}}}}
    }})
}

pub fn verify(input: &Path, out: &Path, model: &str, budget: f64, threads: usize) -> Result<(), String> {
    let done: HashSet<String> = read(out)?.into_iter().map(|e| format!("{}|{}", e.job, e.text)).collect();
    let todo: Vec<Example> =
        read(input)?.into_iter().filter(|e| !done.contains(&format!("{}|{}", e.job, e.text))).collect();
    let mut batches: Vec<Vec<Example>> = Vec::new();
    for e in todo {
        match batches.last_mut() {
            Some(b) if b.len() < 20 && b[0].word == e.word && b[0].set == e.set => b.push(e),
            _ => batches.push(vec![e]),
        }
    }
    eprintln!("{} batches to verify", batches.len());
    let c = client()?;
    let file = Mutex::new((0usize, 0usize));
    let spent = run_pool(
        batches,
        threads,
        budget,
        |b| b[0].job.clone(),
        |b| {
            let m = sound_alike::sets()[b[0].set].iter().find(|m| m.word == b[0].word).cloned().unwrap();
            let mut user = format!(
                "Each sentence should use the word \"{}\" correctly, meaning: {}. A sentence is ok only if that word is the right word there \
             (not a sound-alike that should be used instead), with that meaning, and the sentence is correct standard English.\n\n",
                m.word, m.meaning
            );
            for (n, e) in b.iter().enumerate() {
                user.push_str(&format!("{n}. {}\n", e.text));
            }
            let (args, cost) = call(&c, model, "You are an expert English editor.", &user, verdicts_tool(), 0.0)?;
            let empty = Vec::new();
            let v = args["verdicts"].as_array().unwrap_or(&empty);
            let items: Vec<Example> = b
                .iter()
                .enumerate()
                .filter_map(|(n, e)| {
                    let ok = v.iter().find(|x| x["n"].as_u64() == Some(n as u64))?["ok"].as_bool()?;
                    Some(Example { verified: Some(ok), ..e.clone() })
                })
                .collect();
            let mut g = file.lock().unwrap();
            g.0 += items.iter().filter(|e| e.verified == Some(true)).count();
            g.1 += items.iter().filter(|e| e.verified == Some(false)).count();
            append(out, &items)?;
            Ok(cost)
        },
    );
    let g = file.lock().unwrap();
    eprintln!("ok {}, rejected {}; spent ${spent:.4}", g.0, g.1);
    Ok(())
}

/// Splits verified examples into training and held-out (every fifth, per word).
fn split(path: &Path) -> Result<(Vec<Example>, Vec<Example>), String> {
    let all: Vec<Example> = read(path)?.into_iter().filter(|e| e.verified != Some(false)).collect();
    let (mut train, mut test) = (Vec::new(), Vec::new());
    let mut count: std::collections::HashMap<(usize, String), usize> = Default::default();
    for e in all {
        let k = count.entry((e.set, e.word.clone())).or_default();
        *k += 1;
        if k.is_multiple_of(5) { test.push(e) } else { train.push(e) }
    }
    Ok((train, test))
}

pub fn train(input: &Path, out: &Path, min_count: u32) -> Result<(), String> {
    let (train, test) = split(input)?;
    let ex: Vec<(usize, String, String)> = train.iter().map(|e| (e.set, e.word.clone(), e.text.clone())).collect();
    let tsv = sound_alike::train(&ex, min_count);
    std::fs::write(out, &tsv).map_err(|e| e.to_string())?;
    eprintln!(
        "trained on {} sentences ({} held out): {} lines, {} KB → {}",
        train.len(),
        test.len(),
        tsv.lines().count(),
        tsv.len() / 1024,
        out.display()
    );
    Ok(())
}

/// Scores `model` on the held-out sentences: each correct sentence should
/// pass, and the same sentence with the word swapped for each other member
/// of its set should be caught.
pub fn eval(input: &Path, model_path: &Path, margins: &[f64], support: usize) -> Result<(), String> {
    let (_, test) = split(input)?;
    let model = Model::parse(&std::fs::read_to_string(model_path).map_err(|e| e.to_string())?);
    println!("held-out sentences: {}", test.len());
    println!("| margin | caught swaps | right fix | false alarms |\n|---:|---:|---:|---:|");
    let mut per_set_fa: std::collections::BTreeMap<String, usize> = Default::default();
    for &margin in margins {
        let th = Threshold { margin, support };
        let (mut swaps, mut caught, mut fixed, mut fa) = (0, 0, 0, 0);
        for e in &test {
            if !sound_alike::find(&model, &e.text, th).is_empty() {
                fa += 1;
                if margin == margins[0] {
                    *per_set_fa.entry(e.word.clone()).or_default() += 1;
                }
            }
            for other in sound_alike::sets()[e.set].iter().filter(|m| m.word != e.word) {
                // Swap the first whole-word use (keeping a capital).
                let toks = sound_alike::tokens(&e.text);
                let Some((r, _)) = toks.iter().find(|(_, w)| *w == e.word) else { continue };
                let orig = &e.text[r.clone()];
                let rep = if orig.chars().next().is_some_and(char::is_uppercase) {
                    let mut c = other.word.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                } else {
                    other.word.clone()
                };
                let swapped = format!("{}{}{}", &e.text[..r.start], rep, &e.text[r.end..]);
                swaps += 1;
                let found = sound_alike::find(&model, &swapped, th);
                if let Some(f) = found.iter().find(|f| f.0.start == r.start) {
                    caught += 1;
                    if f.2 == e.word {
                        fixed += 1;
                    }
                }
            }
        }
        println!(
            "| {margin} | {caught}/{swaps} ({:.0}%) | {:.0}% | {fa}/{} ({:.2}%) |",
            100.0 * caught as f64 / swaps as f64,
            100.0 * fixed as f64 / swaps as f64,
            test.len(),
            100.0 * fa as f64 / test.len() as f64
        );
    }
    let mut worst: Vec<_> = per_set_fa.into_iter().collect();
    worst.sort_by_key(|w| std::cmp::Reverse(w.1));
    println!("false alarms at margin {} by word: {:?}", margins[0], &worst[..worst.len().min(15)]);
    Ok(())
}
