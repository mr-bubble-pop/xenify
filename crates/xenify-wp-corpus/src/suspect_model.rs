#![allow(dead_code)] // The failed "suspicion" experiment, kept for reference.
//! "Something looks wrong here": a statistical model of normal English that
//! marks sentences containing word sequences correct text almost never has
//! ("we need finish", "around corner we"), without knowing the fix.
//!
//! Sentences become *hybrid* sequences: grammar words stay as words (the,
//! to, of, is, and…) and content words become their part of speech, so
//! "we need finish the report" → `we VERB VERB the NOUN`. Counts of these
//! sequences, learned from correct sentences, show which ones are normal; a
//! three-step sequence whose two halves are common but which never occurs
//! whole is suspicious.

use xenify_wp_check::sound_alike::tagged;
use std::collections::HashMap;
use std::ops::Range;

/// Tags kept as words (closed word classes).
const KEEP: &[&str] = &["DET", "ADP", "AUX", "PRON", "PART", "CCONJ", "SCONJ"];

/// The hybrid sequence of `text`, keeping only grammar words as words.
pub fn hybrid(text: &str) -> Vec<(Range<usize>, String)> {
    hybrid_with(text, &std::collections::HashSet::new())
}

/// The hybrid sequence of `text` with each token's byte range, between `<s>`
/// and `</s>`; words in `lexical` (common words) also stay as words.
pub fn hybrid_with(text: &str, lexical: &std::collections::HashSet<String>) -> Vec<(Range<usize>, String)> {
    let mut out = vec![(0..0, "<s>".to_string())];
    for (r, w, tag) in tagged(text) {
        let keep = KEEP.contains(&tag.as_str()) || !w.chars().any(char::is_alphanumeric) || lexical.contains(&w);
        let tok = if keep {
            w
        } else if w.chars().all(|c| c.is_ascii_digit()) {
            "NUM".into()
        } else {
            tag
        };
        out.push((r, tok));
    }
    out.push((text.len()..text.len(), "</s>".into()));
    out
}

#[derive(Default)]
pub struct Model {
    bi: HashMap<String, f64>,
    tri: HashMap<String, f64>,
    /// Bigram counts by first token.
    from: HashMap<String, f64>,
    /// Common words kept as words.
    lexical: std::collections::HashSet<String>,
    /// Total trigrams counted.
    total: f64,
}

fn key(parts: &[&str]) -> String {
    parts.join(" ")
}

impl Model {
    /// Counts from correct sentences.
    pub fn train<'a>(sentences: impl IntoIterator<Item = &'a str>) -> Model {
        Self::train_lexical(sentences, 0)
    }

    /// As [`Model::train`], keeping the `top` most common words as words.
    pub fn train_lexical<'a>(sentences: impl IntoIterator<Item = &'a str>, top: usize) -> Model {
        let sentences: Vec<&str> = sentences.into_iter().collect();
        let mut m = Model::default();
        if top > 0 {
            let mut freq: HashMap<String, usize> = HashMap::new();
            for s in &sentences {
                for (_, w) in xenify_wp_check::sound_alike::tokens(s) {
                    *freq.entry(w).or_default() += 1;
                }
            }
            let mut f: Vec<(String, usize)> = freq.into_iter().collect();
            f.sort_by_key(|w| std::cmp::Reverse(w.1));
            m.lexical = f.into_iter().take(top).map(|(w, _)| w).collect();
        }
        for s in sentences {
            let h = hybrid_with(s, &m.lexical);
            let toks: Vec<&str> = h.iter().map(|(_, t)| t.as_str()).collect();
            for w in toks.windows(2) {
                *m.bi.entry(key(w)).or_default() += 1.0;
                *m.from.entry(w[0].to_string()).or_default() += 1.0;
            }
            for w in toks.windows(3) {
                *m.tri.entry(key(w)).or_default() += 1.0;
                m.total += 1.0;
            }
        }
        m
    }

    /// `model.tsv` text (counts below `min` dropped).
    pub fn to_tsv(&self, min: f64) -> String {
        let mut lines = vec![format!("#total\t{}", self.total)];
        lines.extend(self.lexical.iter().map(|w| format!("#word\t{w}")));
        lines.extend(self.bi.iter().filter(|(_, c)| **c >= min).map(|(k, c)| format!("2\t{k}\t{c}")));
        lines.extend(self.tri.iter().filter(|(_, c)| **c >= min).map(|(k, c)| format!("3\t{k}\t{c}")));
        lines.sort();
        lines.join("\n") + "\n"
    }

    pub fn parse(src: &str) -> Model {
        let mut m = Model::default();
        for line in src.lines() {
            let p: Vec<&str> = line.split('\t').collect();
            match p.as_slice() {
                ["#total", n] => m.total = n.parse().unwrap_or(0.0),
                ["#word", w] => {
                    m.lexical.insert(w.to_string());
                }
                ["2", k, c] => {
                    let c: f64 = c.parse().unwrap_or(0.0);
                    if let Some((first, _)) = k.split_once(' ') {
                        *m.from.entry(first.to_string()).or_default() += c;
                    }
                    m.bi.insert(k.to_string(), c);
                }
                ["3", k, c] => {
                    m.tri.insert(k.to_string(), c.parse().unwrap_or(0.0));
                }
                _ => {}
            }
        }
        m
    }

    /// How unexpected each three-step sequence in `text` is: (byte range of
    /// the middle token's neighbourhood, surprise). Surprise is the log of
    /// how often the sequence would be expected from its halves, over how
    /// often it was seen.
    pub fn surprises(&self, text: &str) -> Vec<(Range<usize>, f64)> {
        let h = hybrid_with(text, &self.lexical);
        let mut out = Vec::new();
        for i in 1..h.len().saturating_sub(1) {
            let (a, b, c) = (&h[i - 1].1, &h[i].1, &h[i + 1].1);
            let ab = self.bi.get(&key(&[a, b])).copied().unwrap_or(0.0);
            let bc = self.bi.get(&key(&[b, c])).copied().unwrap_or(0.0);
            let abc = self.tri.get(&key(&[a, b, c])).copied().unwrap_or(0.0);
            let b_count: f64 = ab.max(1.0); // ~ how often b follows a
            // Expected count of abc if b→c were independent of a: ab * P(c | b).
            let p_c_given_b = bc / self.bi_from(b).max(1.0);
            let expected = b_count * p_c_given_b;
            let surprise = ((expected + 0.1) / (abc + 0.1)).ln();
            let start = if h[i - 1].0.is_empty() { h[i].0.start } else { h[i - 1].0.start };
            let end = if h[i + 1].0.is_empty() { h[i].0.end } else { h[i + 1].0.end };
            out.push((start..end, surprise));
        }
        out
    }

    /// Three-step sequences in `text` never seen in training although both
    /// halves were seen at least `k` times: (byte range, the sequence).
    pub fn unseen(&self, text: &str, k: f64) -> Vec<(Range<usize>, String)> {
        let h = hybrid_with(text, &self.lexical);
        let mut out = Vec::new();
        for i in 1..h.len().saturating_sub(1) {
            let (a, b, c) = (&h[i - 1].1, &h[i].1, &h[i + 1].1);
            let ab = self.bi.get(&key(&[a, b])).copied().unwrap_or(0.0);
            let bc = self.bi.get(&key(&[b, c])).copied().unwrap_or(0.0);
            let abc = self.tri.get(&key(&[a, b, c])).copied().unwrap_or(0.0);
            if abc == 0.0 && ab >= k && bc >= k {
                let start = if h[i - 1].0.is_empty() { h[i].0.start } else { h[i - 1].0.start };
                let end = if h[i + 1].0.is_empty() { h[i].0.end } else { h[i + 1].0.end };
                out.push((start..end, key(&[a, b, c])));
            }
        }
        out
    }

    /// How many bigrams start with `b`.
    fn bi_from(&self, b: &str) -> f64 {
        self.from.get(b).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_keeps_grammar_words() {
        let h: Vec<String> = hybrid("We need to finish the report.").into_iter().map(|(_, t)| t).collect();
        assert_eq!(h.first().map(String::as_str), Some("<s>"));
        assert!(
            h.contains(&"to".to_string()) && h.contains(&"the".to_string()) && h.contains(&"we".to_string()),
            "{h:?}"
        );
        assert!(!h.contains(&"report".to_string()), "content words become tags: {h:?}");
    }

    #[test]
    fn unseen_sequence_is_surprising() {
        let good = [
            "We need to finish the report.",
            "They need to send the file.",
            "I need to check the logs.",
            "You need to sign the form.",
        ];
        let m = Model::train(good.iter().copied().cycle().take(40));
        let worst = |t: &str| m.surprises(t).into_iter().map(|(_, s)| s).fold(f64::MIN, f64::max);
        assert!(worst("We need finish the report.") > worst("We need to finish the report.") + 1.0);
    }
}
