//! The "something looks wrong" detector: train on correct sentences, then
//! see how many mistakes fall in a flagged sentence and how many correct
//! sentences get flagged.

use crate::read_items;
use crate::suspect_model::Model;
use xenify_wp_check::clarity::sentences;
use std::path::Path;

/// Correct sentences from sound-alike example files (skipping rejected ones).
fn training_sentences(files: &[std::path::PathBuf]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        for line in text.lines() {
            let v: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
            if v["verified"] == false {
                continue;
            }
            if let Some(t) = v["text"].as_str() {
                out.push(t.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn train(files: &[std::path::PathBuf], out: &Path, min: f64, top: usize) -> Result<(), String> {
    let s = training_sentences(files)?;
    eprintln!("training on {} sentences", s.len());
    let m = Model::train_lexical(s.iter().map(String::as_str), top);
    let tsv = m.to_tsv(min);
    std::fs::write(out, &tsv).map_err(|e| e.to_string())?;
    eprintln!("{} lines, {} KB → {}", tsv.lines().count(), tsv.len() / 1024, out.display());
    Ok(())
}

/// Sentence-level scores on a corpus: recall (the sentence with the mistake
/// is flagged) and false alarms (a sentence of a corrected passage is flagged).
pub fn eval(corpus: &Path, model: &Path, thresholds: &[f64]) -> Result<(), String> {
    let m = Model::parse(&std::fs::read_to_string(model).map_err(|e| e.to_string())?);
    let items = read_items(corpus)?;
    // (surprise of the mistake's sentence near the mistake, error type) and clean sentence scores.
    let mut mistakes: Vec<(f64, String)> = Vec::new();
    let mut clean: Vec<f64> = Vec::new();
    for it in &items {
        let (Some(kind), Some(mk), Some(corr)) = (&it.error_type, it.mistakes.first(), &it.corrected) else { continue };
        let Some(start) = it.text.find(&mk.wrong) else { continue };
        let span = start..start + mk.wrong.len();
        for s in sentences(&it.text) {
            if s.start <= span.start && span.start < s.end.max(s.start + 1) {
                let sent = &it.text[s.clone()];
                let worst = m.surprises(sent).into_iter().map(|(_, x)| x).fold(f64::MIN, f64::max);
                mistakes.push((worst, kind.clone()));
            }
        }
        for s in sentences(corr) {
            let worst = m.surprises(&corr[s]).into_iter().map(|(_, x)| x).fold(f64::MIN, f64::max);
            clean.push(worst);
        }
    }
    println!("{} mistake sentences, {} correct sentences", mistakes.len(), clean.len());
    println!("| threshold | mistakes flagged | correct sentences flagged |\n|---:|---:|---:|");
    for &t in thresholds {
        let hit = mistakes.iter().filter(|(x, _)| *x >= t).count();
        let fa = clean.iter().filter(|x| **x >= t).count();
        println!(
            "| {t} | {hit}/{} ({:.0}%) | {fa}/{} ({:.0}%) |",
            mistakes.len(),
            100.0 * hit as f64 / mistakes.len() as f64,
            clean.len(),
            100.0 * fa as f64 / clean.len() as f64
        );
    }
    // The "never seen, though both halves are common" signal.
    for k in [2.0, 5.0, 10.0, 20.0, 50.0] {
        let (mut hit, mut n, mut fa, mut nc) = (0, 0, 0, 0);
        for it in &items {
            let (Some(_), Some(mk), Some(corr)) = (&it.error_type, it.mistakes.first(), &it.corrected) else {
                continue;
            };
            let Some(start) = it.text.find(&mk.wrong) else { continue };
            let span = start..start + mk.wrong.len();
            n += 1;
            // Flagged near the mistake.
            if m.unseen(&it.text, k).iter().any(|(r, _)| r.start <= span.end + 1 && span.start <= r.end + 1) {
                hit += 1;
            }
            for s in sentences(corr) {
                nc += 1;
                if !m.unseen(&corr[s], k).is_empty() {
                    fa += 1;
                }
            }
        }
        println!(
            "unseen, halves ≥{k}: mistakes flagged at the spot {hit}/{n} ({:.0}%), correct sentences flagged {fa}/{nc} ({:.0}%)",
            100.0 * hit as f64 / n as f64,
            100.0 * fa as f64 / nc as f64
        );
    }
    // By kind at the middle threshold.
    let t = thresholds[thresholds.len() / 2];
    let mut kinds: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for (x, k) in &mistakes {
        let e = kinds.entry(k.clone()).or_default();
        e.0 += 1;
        if *x >= t {
            e.1 += 1;
        }
    }
    println!("\nflagged by kind at threshold {t}:");
    for (k, (n, h)) in kinds {
        println!("  {k:26} {:.0}%", 100.0 * h as f64 / n as f64);
    }
    Ok(())
}
