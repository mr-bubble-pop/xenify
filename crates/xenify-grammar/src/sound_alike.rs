//! Sound-alike words used in the wrong place ("spirits, bear and wine"),
//! judged by context: each word of a confusable set keeps different company
//! (beer: wine, pint, brewery; bear: grizzly, cub, with me), so the words
//! around it say which one was meant.
//!
//! The context counts in `rules/homophones/model.tsv` are learned by
//! `xenify-wp-corpus sounds train` from AI-generated sentences that use each
//! word correctly; the sets and meanings are in `rules/homophones/sets.txt`.
//! It's a small naive-Bayes model: for each candidate word, add up how much
//! more often each nearby word appeared with it than with the word written.

use harper_core::linting::{Lint, LintKind, Linter, Suggestion};
use harper_core::{Document, Span};
use std::collections::HashMap;
use std::sync::OnceLock;

const SETS: &str = include_str!("../rules/homophones/sets.txt");
const MODEL: &str = include_str!("../rules/homophones/model.tsv");
const FREQ: &str = include_str!("../rules/homophones/freq.tsv");

/// How common a set word is (counts from books and generated text).
fn frequency(word: &str) -> f64 {
    static F: OnceLock<HashMap<String, f64>> = OnceLock::new();
    F.get_or_init(|| {
        FREQ.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (w, n) = l.split_once('\t')?;
                Some((w.to_string(), n.trim().parse().ok()?))
            })
            .collect()
    })
    .get(word)
    .copied()
    .unwrap_or(0.0)
}

/// Extra evidence needed to suggest `best` for `written`: none when `best`
/// is as common, more the rarer it is ("beach" → "beech", "in" → "inn").
fn rarity_penalty(written: &str, best: &str) -> f64 {
    // Half the log ratio: enough to stop "beach" → "beech", not "through" → "threw".
    0.5 * ((frequency(written) + 1.0).ln() - (frequency(best) + 1.0).ln()).max(0.0)
}

/// Contexts where "to" is right whatever the model thinks: "as to", and at
/// the end of a phrase after a verb or adjective ("I'll be glad to.").
fn to_is_fine(toks: &[Tok], t: usize) -> bool {
    if toks[t].1 != "to" {
        return false;
    }
    let prev = t.checked_sub(1).map(|p| toks[p].1.as_str()).unwrap_or("");
    let at_end = toks.get(t + 1).is_none_or(|n| !n.1.chars().any(char::is_alphanumeric));
    const OBJECTS: &[&str] =
        &["me", "you", "him", "her", "us", "them", "it", "this", "that", "i", "we", "they", "he", "she"];
    prev == "as" || (at_end && !OBJECTS.contains(&prev))
}

/// Words within this many positions count as context.
const WINDOW: usize = 6;
/// Laplace smoothing.
const ALPHA: f64 = 0.5;

/// A word of a confusable set, with what it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub word: String,
    pub meaning: String,
    /// Never suggested in place of another word of the set (written `!pi` in
    /// sets.txt): it's rare, so "enjoying pie" mustn't become "pi", but "apple
    /// pi" can still become "pie".
    pub never_suggest: bool,
}

/// The confusable sets.
pub fn sets() -> &'static [Vec<Member>] {
    static S: OnceLock<Vec<Vec<Member>>> = OnceLock::new();
    S.get_or_init(|| parse_sets(SETS))
}

pub fn parse_sets(src: &str) -> Vec<Vec<Member>> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split('|')
                .filter_map(|part| {
                    let (w, m) = part.split_once('=')?;
                    let w = w.trim();
                    Some(Member {
                        word: w.trim_start_matches('!').to_lowercase(),
                        meaning: m.trim().to_string(),
                        never_suggest: w.starts_with('!'),
                    })
                })
                .collect()
        })
        .filter(|s: &Vec<Member>| s.len() >= 2)
        .collect()
}

/// Lowercase words and punctuation marks with their byte ranges.
pub fn tokens(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start = None;
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '\u{2019}' || c == '-';
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        if let Some(s) = start {
            if is_word(c) {
                continue;
            }
            let w = text[s..i].trim_end_matches(['\'', '\u{2019}', '-']);
            if !w.is_empty() {
                out.push((s..s + w.len(), w.to_lowercase().replace('\u{2019}', "'")));
            }
            start = None;
        }
        if is_word(c) && !(c == '\'' || c == '\u{2019}' || c == '-') {
            start = Some(i);
        } else if matches!(c, '.' | ',' | '!' | '?' | ';' | ':') {
            out.push((i..i + 1, c.to_string()));
        }
    }
    out
}

/// A token: byte range, lowercase text, part-of-speech tag (from Harper's tagger).
pub type Tok = (std::ops::Range<usize>, String, String);

/// Tokens of `text` with part-of-speech tags.
pub fn tagged(text: &str) -> Vec<Tok> {
    tagged_doc(&Document::new_plain_english_curated(text), text)
}

/// Tokens of `text` with tags from its already-parsed `doc`.
pub fn tagged_doc(doc: &Document, text: &str) -> Vec<Tok> {
    // Char index → tag.
    let mut tags: HashMap<usize, String> = HashMap::new();
    for t in doc.get_tokens() {
        if let harper_core::TokenKind::Word(Some(m)) = &t.kind
            && let Some(tag) = m.pos_tag
        {
            tags.insert(t.span.start, format!("{tag:?}"));
        }
    }
    let char_of: HashMap<usize, usize> = text.char_indices().enumerate().map(|(c, (b, _))| (b, c)).collect();
    tokens(text)
        .into_iter()
        .map(|(r, w)| {
            let tag = char_of
                .get(&r.start)
                .and_then(|c| tags.get(c))
                .cloned()
                .unwrap_or_else(|| if w.chars().any(char::is_alphanumeric) { "X".into() } else { w.clone() });
            (r, w, tag)
        })
        .collect()
}

/// The context features of the token at `t`.
pub fn features(toks: &[Tok], t: usize) -> Vec<String> {
    let at = |k: isize, tag: bool| -> String {
        let k = t as isize + k;
        if k < 0 {
            "<s>".into()
        } else {
            toks.get(k as usize)
                .map(|(_, w, p)| if tag { p.clone() } else { w.clone() })
                .unwrap_or_else(|| "</s>".into())
        }
    };
    let mut f = vec![
        format!("L1:{}", at(-1, false)),
        format!("R1:{}", at(1, false)),
        format!("L2:{} {}", at(-2, false), at(-1, false)),
        format!("R2:{} {}", at(1, false), at(2, false)),
        format!("P-1:{}", at(-1, true)),
        format!("P+1:{}", at(1, true)),
        format!("P+2:{} {}", at(1, true), at(2, true)),
        format!("P-1+1:{} {}", at(-1, true), at(1, true)),
    ];
    let lo = t.saturating_sub(WINDOW);
    let hi = (t + WINDOW + 1).min(toks.len());
    for (k, (_, w, _)) in toks.iter().enumerate().take(hi).skip(lo) {
        if k != t && w.chars().any(char::is_alphanumeric) {
            f.push(format!("W:{w}"));
        }
    }
    f.sort();
    f.dedup();
    f
}

/// Context counts: (set index, word) → (occurrences, feature → count).
#[derive(Default)]
pub struct Model {
    words: HashMap<(usize, String), (f64, HashMap<String, f64>)>,
    /// Distinct features per set (for smoothing).
    vocab: HashMap<usize, f64>,
}

impl Model {
    /// Parses `model.tsv`: `#n<TAB>set<TAB>word<TAB>count` and `set<TAB>word<TAB>feature<TAB>count` lines.
    pub fn parse(src: &str) -> Self {
        let mut m = Model::default();
        let mut vocab: HashMap<usize, std::collections::HashSet<String>> = HashMap::new();
        for line in src.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            match parts.as_slice() {
                ["#n", set, word, n] => {
                    let (Ok(set), Ok(n)) = (set.parse::<usize>(), n.parse::<f64>()) else { continue };
                    m.words.entry((set, word.to_string())).or_default().0 = n;
                }
                [set, word, feat, n] if !set.starts_with('#') => {
                    let (Ok(set), Ok(n)) = (set.parse::<usize>(), n.parse::<f64>()) else { continue };
                    m.words.entry((set, word.to_string())).or_default().1.insert(feat.to_string(), n);
                    vocab.entry(set).or_default().insert(feat.to_string());
                }
                _ => {}
            }
        }
        m.vocab = vocab.into_iter().map(|(k, v)| (k, v.len() as f64)).collect();
        m
    }

    /// The bundled model.
    pub fn bundled() -> &'static Model {
        static M: OnceLock<Model> = OnceLock::new();
        M.get_or_init(|| Model::parse(MODEL))
    }

    /// log P(feature | word), smoothed.
    fn logp(&self, set: usize, word: &str, f: &str) -> f64 {
        let Some((n, counts)) = self.words.get(&(set, word.to_string())) else { return f64::NEG_INFINITY };
        let v = self.vocab.get(&set).copied().unwrap_or(1.0);
        ((counts.get(f).copied().unwrap_or(0.0) + ALPHA) / (n + ALPHA * v)).ln()
    }

    /// For the word at token `t` (a member of set `set`): the member the
    /// context favours instead, with the margin (summed, capped log-odds) and
    /// the number of context features the model has seen. `None` if the
    /// written word is fine.
    pub fn better(&self, set: usize, toks: &[Tok], t: usize) -> Option<(String, f64, usize)> {
        let written = &toks[t].1;
        let members = &sets()[set];
        let feats = features(toks, t);
        let seen: Vec<&String> = feats
            .iter()
            .filter(|f| {
                members
                    .iter()
                    .any(|m| self.words.get(&(set, m.word.clone())).is_some_and(|(_, c)| c.contains_key(f.as_str())))
            })
            .collect();
        let (best, margin) = members
            .iter()
            .filter(|m| m.word != *written && !m.never_suggest)
            .map(|m| {
                let s: f64 = seen
                    .iter()
                    .map(|f| {
                        let w = if f.starts_with("W:") { 0.5 } else { 1.0 };
                        w * (self.logp(set, &m.word, f) - self.logp(set, written, f)).clamp(-CAP, CAP)
                    })
                    .sum();
                (m.word.clone(), s)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        (margin > 0.0).then_some((best, margin, seen.len()))
    }
}

/// The most one context feature can count for (log-odds).
const CAP: f64 = 2.5;

/// Training: counts context features of each word in correct sentences.
/// `examples` are (set index, word, sentence). Returns `model.tsv` text,
/// keeping features seen at least `min_count` times.
pub fn train(examples: &[(usize, String, String)], min_count: u32) -> String {
    let mut n: HashMap<(usize, String), u32> = HashMap::new();
    let mut c: HashMap<(usize, String, String), u32> = HashMap::new();
    for (set, word, text) in examples {
        let toks = tagged(text);
        for (t, (_, w, _)) in toks.iter().enumerate() {
            if w != word {
                continue;
            }
            *n.entry((*set, word.clone())).or_default() += 1;
            for f in features(&toks, t) {
                *c.entry((*set, word.clone(), f)).or_default() += 1;
            }
        }
    }
    let mut lines: Vec<String> = n.iter().map(|((s, w), k)| format!("#n\t{s}\t{w}\t{k}")).collect();
    lines.extend(c.iter().filter(|(_, k)| **k >= min_count).map(|((s, w, f), k)| format!("{s}\t{w}\t{f}\t{k}")));
    lines.sort();
    format!(
        "# Xenify Write sound-alike context counts (generated by `xenify-wp-corpus sounds train`; MIT)\n{}\n",
        lines.join("\n")
    )
}

/// Which set (if any) `word` belongs to.
pub fn set_of(word: &str) -> Option<usize> {
    static IDX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    IDX.get_or_init(|| {
        sets().iter().enumerate().flat_map(|(i, s)| s.iter().map(move |m| (m.word.clone(), i))).collect()
    })
    .get(word)
    .copied()
}

/// How sure the context must be before a word is flagged (log-odds), and
/// how many context words must have been seen.
#[derive(Clone, Copy, Debug)]
pub struct Threshold {
    pub margin: f64,
    pub support: usize,
}

pub const DEFAULT_THRESHOLD: Threshold = Threshold { margin: 10.0, support: 3 };

/// Finds sound-alikes in `text` with `model`; (byte range, written, suggested, margin).
pub fn find(model: &Model, text: &str, th: Threshold) -> Vec<(std::ops::Range<usize>, String, String, f64)> {
    find_in(model, &tagged(text), text, th)
}

/// [`find`] on already-tagged tokens.
pub fn find_in(
    model: &Model,
    toks: &[Tok],
    text: &str,
    th: Threshold,
) -> Vec<(std::ops::Range<usize>, String, String, f64)> {
    let mut out = Vec::new();
    for (t, (r, w, _)) in toks.iter().enumerate() {
        let Some(set) = set_of(w) else { continue };
        if let Some((best, margin, support)) = model.better(set, toks, t)
            && margin >= th.margin + rarity_penalty(w, &best)
            && support >= th.support
            && !to_is_fine(toks, t)
        {
            out.push((r.clone(), text[r.clone()].to_string(), best, margin));
        }
    }
    out
}

fn meaning(word: &str) -> Option<&'static str> {
    sets().iter().flatten().find(|m| m.word == word).map(|m| m.meaning.as_str())
}

/// The checker rule.
#[derive(Default)]
pub struct SoundAlike;

impl Linter for SoundAlike {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        let src: String = document.get_source().iter().collect();
        // Byte offset → char index.
        let chars: Vec<usize> = src.char_indices().map(|(i, _)| i).chain([src.len()]).collect();
        let to_char = |b: usize| chars.partition_point(|&x| x < b);
        find_in(Model::bundled(), &tagged_doc(document, &src), &src, DEFAULT_THRESHOLD)
            .into_iter()
            .map(|(r, written, best, _)| {
                let fixed = if written.chars().next().is_some_and(char::is_uppercase) {
                    let mut c = best.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                } else {
                    best.clone()
                };
                let lw = written.to_lowercase().replace('\u{2019}', "'");
                let message = match (meaning(&best), meaning(&lw)) {
                    (Some(b), Some(w)) => format!("Did you mean `{best}` ({b})? `{lw}` means {w}."),
                    _ => format!("Did you mean `{best}`?"),
                };
                Lint {
                    span: Span::new(to_char(r.start), to_char(r.end)),
                    lint_kind: LintKind::WordChoice,
                    suggestions: vec![Suggestion::ReplaceWith(fixed.chars().collect())],
                    message,
                    priority: 40,
                }
            })
            .collect()
    }

    fn description(&self) -> &str {
        "Finds sound-alike words used in the wrong place, judged by the words around them: `spirits, bear and wine` → `beer`."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_parse() {
        assert!(sets().len() > 80);
        assert!(sets().iter().all(|s| s.iter().all(|m| !m.word.is_empty() && !m.meaning.is_empty())));
        assert_eq!(set_of("beer"), set_of("bear"));
        // Each word is in only one set.
        let mut all: Vec<&str> = sets().iter().flatten().map(|m| m.word.as_str()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "a word is in two sets");
    }

    #[test]
    fn learns_context_from_examples() {
        let set = set_of("bare").unwrap();
        let ex: Vec<(usize, String, String)> = [
            ("bare", "She walked on the sand in bare feet."),
            ("bare", "The walls of the room were bare and white."),
            ("bare", "He ran across the lawn in bare feet."),
            ("bear", "A grizzly bear wandered into the woods."),
            ("bear", "The bear and her cub slept in the cave."),
            ("bear", "Please bear with me while I check."),
        ]
        .iter()
        .map(|(w, t)| (set, w.to_string(), t.to_string()))
        .collect();
        let m = Model::parse(&train(&ex, 1));
        let toks = tagged("The kids ran outside in bear feet.");
        let t = toks.iter().position(|(_, w, _)| w == "bear").unwrap();
        assert_eq!(m.better(set, &toks, t).map(|b| b.0), Some("bare".into()));
        let toks = tagged("A grizzly bear in the woods.");
        let t = toks.iter().position(|(_, w, _)| w == "bear").unwrap();
        assert!(m.better(set, &toks, t).is_none());
    }
}
