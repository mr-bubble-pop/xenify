//! Readability and style checks (our own word lists, no third-party data).

use crate::{Category, Local, Profile};
use unicode_segmentation::UnicodeSegmentation;

/// Forms of "to be" that start a passive construction.
const BE: &[&str] = &["am", "is", "are", "was", "were", "be", "been", "being"];

/// Common irregular past participles (for passive voice).
const PARTICIPLES: &[&str] = &[
    "begun",
    "bitten",
    "blown",
    "broken",
    "brought",
    "built",
    "bought",
    "caught",
    "chosen",
    "done",
    "drawn",
    "driven",
    "eaten",
    "fallen",
    "felt",
    "found",
    "forgotten",
    "forgiven",
    "frozen",
    "given",
    "gone",
    "grown",
    "heard",
    "held",
    "hidden",
    "hit",
    "hurt",
    "kept",
    "known",
    "laid",
    "led",
    "left",
    "lent",
    "lost",
    "made",
    "meant",
    "met",
    "paid",
    "put",
    "read",
    "ridden",
    "run",
    "said",
    "seen",
    "sent",
    "set",
    "shown",
    "shut",
    "sold",
    "spent",
    "spoken",
    "stolen",
    "struck",
    "sung",
    "sunk",
    "taken",
    "taught",
    "thought",
    "thrown",
    "told",
    "understood",
    "won",
    "worn",
    "written",
];

/// Words ending in -ly that aren't adverbs.
const NOT_ADVERBS: &[&str] = &[
    "only",
    "family",
    "early",
    "likely",
    "reply",
    "apply",
    "supply",
    "holy",
    "ugly",
    "fly",
    "july",
    "italy",
    "daily",
    "weekly",
    "monthly",
    "yearly",
    "friendly",
    "lovely",
    "lonely",
    "silly",
    "belly",
    "jelly",
    "rally",
    "ally",
    "bully",
    "comply",
    "rely",
    "imply",
    "multiply",
    "assembly",
    "anomaly",
    "butterfly",
    "costly",
    "elderly",
    "curly",
    "hilly",
    "orderly",
    "timely",
    "lively",
    "kindly",
    "deadly",
    "manly",
    "scholarly",
    "ghostly",
    "homely",
    "wily",
    "oily",
];

/// Wordy phrases and plainer alternatives (written for Xenify Write).
const WORDY: &[(&str, &str)] = &[
    ("in order to", "to"),
    ("due to the fact that", "because"),
    ("at this point in time", "now"),
    ("at the present time", "now"),
    ("in the event that", "if"),
    ("for the purpose of", "to"),
    ("in spite of the fact that", "although"),
    ("with regard to", "about"),
    ("with respect to", "about"),
    ("in close proximity to", "near"),
    ("a large number of", "many"),
    ("a majority of", "most"),
    ("on a daily basis", "daily"),
    ("prior to", "before"),
    ("subsequent to", "after"),
    ("in the near future", "soon"),
    ("has the ability to", "can"),
    ("is able to", "can"),
    ("make a decision", "decide"),
    ("give consideration to", "consider"),
    ("utilize", "use"),
    ("utilise", "use"),
    ("commence", "start"),
    ("facilitate", "help"),
    ("in addition to", "besides"),
    ("it is important to note that", ""),
    ("needless to say", ""),
    ("each and every", "each"),
    ("first and foremost", "first"),
    ("the fact that", "that"),
    // Nouns doing a verb's job ("make a decision" → "decide").
    ("make an assessment of", "assess"),
    ("perform an analysis of", "analyse"),
    ("carry out an analysis of", "analyse"),
    ("conduct an investigation into", "investigate"),
    ("carry out an evaluation of", "evaluate"),
    ("provide assistance to", "help"),
    ("provide assistance with", "help with"),
    ("reach a conclusion", "conclude"),
    ("take into consideration", "consider"),
    ("make a recommendation", "recommend"),
    ("have a discussion about", "discuss"),
    ("come to an agreement", "agree"),
    ("make use of", "use"),
    ("is in compliance with", "complies with"),
    ("are in compliance with", "comply with"),
    ("give an explanation of", "explain"),
    ("provide an explanation of", "explain"),
    ("make an improvement to", "improve"),
    ("perform the implementation of", "implement"),
    ("is dependent on", "depends on"),
    ("are dependent on", "depend on"),
    ("has a requirement for", "needs"),
    ("have a requirement for", "need"),
    ("is responsible for the management of", "manages"),
    ("for the reason that", "because"),
    ("in the process of", ""),
    ("at the end of the day", ""),
    ("going forward", ""),
];

fn is_word(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

pub(crate) fn check(text: &str, s: &Profile) -> Vec<Local> {
    let mut out = Vec::new();
    for (start, sentence) in text.split_sentence_bound_indices() {
        let words: Vec<(usize, &str)> = sentence.split_word_bound_indices().filter(|(_, w)| is_word(w)).collect();
        let n = words.len();
        let trimmed = sentence.trim_end();
        let range = start..start + trimmed.len();
        if n > s.very_long_sentence {
            out.push(Local {
                range: range.clone(),
                category: Category::Readability,
                rule: "VeryLongSentence".into(),
                message: format!("This sentence has {n} words. Splitting it would make it much easier to read."),
                suggestions: Vec::new(),
                feature: None,
                strong: false,
            });
        } else if n > s.long_sentence {
            out.push(Local {
                range: range.clone(),
                category: Category::Readability,
                rule: "LongSentence".into(),
                message: format!("This sentence has {n} words. Consider splitting it."),
                suggestions: Vec::new(),
                feature: None,
                strong: false,
            });
        }
        let lower: Vec<String> = words.iter().map(|(_, w)| w.to_lowercase()).collect();
        if s.passive_voice {
            for i in 0..n.saturating_sub(1) {
                let next = &lower[i + 1];
                let participle = (next.ends_with("ed") && next.len() > 3) || PARTICIPLES.contains(&next.as_str());
                if BE.contains(&lower[i].as_str()) && participle {
                    let (a, b) = (words[i].0, words[i + 1].0 + words[i + 1].1.len());
                    out.push(Local {
                        range: start + a..start + b,
                        category: Category::Style,
                        rule: "PassiveVoice".into(),
                        message: "Passive voice. An active verb is often clearer (who did it?).".into(),
                        suggestions: Vec::new(),
                        feature: None,
                        strong: false,
                    });
                }
            }
        }
        if s.adverbs {
            for (i, w) in &words {
                let l = w.to_lowercase();
                if l.len() > 4 && l.ends_with("ly") && !NOT_ADVERBS.contains(&l.as_str()) {
                    out.push(Local {
                        range: start + i..start + i + w.len(),
                        category: Category::Readability,
                        rule: "Adverb".into(),
                        message: "Adverb. A stronger verb often says it better.".into(),
                        suggestions: Vec::new(),
                        feature: None,
                        strong: false,
                    });
                }
            }
        }
    }
    if s.wordy_phrases {
        let low = text.to_lowercase();
        // Lower-casing can change byte lengths for some scripts: only use it when it didn't.
        if low.len() == text.len() {
            for (phrase, plain) in WORDY {
                let mut from = 0;
                while let Some(i) = low[from..].find(phrase) {
                    let at = from + i;
                    let end = at + phrase.len();
                    let bounded = low[..at].chars().next_back().is_none_or(|c| !c.is_alphanumeric())
                        && low[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
                    if bounded {
                        let msg = if plain.is_empty() {
                            format!("\"{}\" can usually go.", &text[at..end])
                        } else {
                            format!("Wordy: \"{}\" could be \"{plain}\".", &text[at..end])
                        };
                        out.push(Local {
                            range: at..end,
                            category: Category::Style,
                            rule: "WordyPhrase".into(),
                            message: msg,
                            suggestions: vec![plain.to_string()],
                            feature: None,
                            strong: false,
                        });
                    }
                    from = end;
                }
            }
        }
    }
    out
}

/// Document statistics for the Proofing panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Readability {
    pub words: usize,
    pub sentences: usize,
    pub syllables: usize,
    /// Flesch–Kincaid grade level.
    pub grade: f32,
    /// Flesch reading ease (0–100, higher is easier).
    pub ease: f32,
}

/// Rough syllable count of an English word.
fn syllables(word: &str) -> usize {
    let w: Vec<char> = word.to_lowercase().chars().filter(|c| c.is_alphabetic()).collect();
    if w.is_empty() {
        return 0;
    }
    let vowel = |c: char| "aeiouy".contains(c);
    let mut n = 0;
    let mut prev = false;
    for &c in &w {
        let v = vowel(c);
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    // A silent final e.
    if w.len() > 2 && w[w.len() - 1] == 'e' && !vowel(w[w.len() - 2]) && n > 1 {
        n -= 1;
    }
    n.max(1)
}

pub fn readability(texts: &[&str]) -> Readability {
    let mut r = Readability::default();
    for t in texts {
        for s in t.unicode_sentences() {
            let words: Vec<&str> = s.unicode_words().collect();
            if words.is_empty() {
                continue;
            }
            r.sentences += 1;
            r.words += words.len();
            r.syllables += words.iter().map(|w| syllables(w)).sum::<usize>();
        }
    }
    if r.words > 0 && r.sentences > 0 {
        let wps = r.words as f32 / r.sentences as f32;
        let spw = r.syllables as f32 / r.words as f32;
        r.grade = (0.39 * wps + 11.8 * spw - 15.59).max(0.0);
        r.ease = (206.835 - 1.015 * wps - 84.6 * spw).clamp(0.0, 100.0);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_checks() {
        let s = Profile { adverbs: true, ..Default::default() };
        let t = "The report was written by the team in order to explain things really clearly.";
        let found = check(t, &s);
        let rules: Vec<(&str, &str)> = found.iter().map(|l| (l.rule.as_str(), &t[l.range.clone()])).collect();
        assert!(rules.contains(&("PassiveVoice", "was written")), "{rules:?}");
        assert!(rules.contains(&("WordyPhrase", "in order to")), "{rules:?}");
        assert!(rules.contains(&("Adverb", "clearly")), "{rules:?}");
        assert!(!rules.iter().any(|r| r.1 == "only"));
        let long = vec!["word"; 30].join(" ") + ".";
        assert!(check(&long, &s).iter().any(|l| l.rule == "LongSentence"));
    }

    #[test]
    fn readability_scores() {
        assert_eq!(syllables("readability"), 5);
        assert_eq!(syllables("the"), 1);
        let easy = readability(&["The cat sat. The dog ran."]);
        let hard = readability(&[
            "Institutional considerations necessitate comprehensive organisational restructuring initiatives.",
        ]);
        assert!(easy.grade < hard.grade);
        assert_eq!(easy.sentences, 2);
    }
}
