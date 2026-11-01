//! Sorting the ~950 rules into a few groups for the settings UI, and
//! readable titles for them. Harper doesn't say what a rule checks until it
//! fires, so the group comes from words in the rule's description.

/// The groups, in the order the UI shows them.
pub const RULE_GROUPS: [&str; 10] = [
    "Sound-alikes and mix-ups",
    "Grammar",
    "Punctuation and spacing",
    "Capital letters",
    "Compound words",
    "Phrases and idioms",
    "Wordiness and style",
    "Abbreviations",
    "Names, brands and places",
    "More word fixes",
];

/// Description words for each group, tried in this order (first match wins).
const KEYWORDS: &[(&str, &[&str])] = &[
    ("Abbreviations", &["initialism", "abbreviation", "acronym"]),
    ("Names, brands and places", &["names", "brand", "trademark", "place name", "proper noun", "compan"]),
    ("Compound words", &["compound", "hyphen"]),
    ("Capital letters", &["capitaliz", "capitalis", "uppercase", "lowercase", "capital "]),
    (
        "Punctuation and spacing",
        &[
            "comma",
            "punctuation",
            "apostrophe",
            "quotation",
            "quote",
            "period",
            "full stop",
            "spacing",
            "space",
            "ellipsis",
            "dash",
            "semicolon",
            "colon",
            "exclamation",
            "question mark",
        ],
    ),
    (
        "Sound-alikes and mix-ups",
        &[
            "malaprop",
            "eggcorn",
            "confus",
            "homophone",
            "mix",
            "mistak",
            "typo",
            "misspell",
            "instead of",
            "intended",
            "was meant",
            "is meant",
        ],
    ),
    (
        "Wordiness and style",
        &[
            "redundan",
            "wordy",
            "concise",
            "simpl",
            "unnecessar",
            "repeat",
            "clich",
            "synonym",
            "jargon",
            "filler",
            "weasel",
            "passive",
            "overused",
            "formal",
            "inclusive",
            "clarity",
            "boring",
            "hedging",
            "curse",
            "contraction",
        ],
    ),
    (
        "Grammar",
        &[
            "article",
            "agree",
            "verb",
            "plural",
            "singular",
            "tense",
            "pronoun",
            "noun",
            "adjective",
            "adverb",
            "preposition",
            "possessive",
            "subject",
            "negat",
            "comparative",
            "superlative",
            "modal",
            "infinitive",
            "word order",
            "auxiliary",
            "inflection",
            "joining word",
        ],
    ),
    (
        "Phrases and idioms",
        &[
            "phrase",
            "expression",
            "idiom",
            "saying",
            "corrects",
            "changes `",
            "replaces `",
            "nonstandard",
            "variant",
            "fixes `",
        ],
    ),
];

/// Which of [`RULE_GROUPS`] a rule belongs in, from its description.
pub fn rule_group(description: &str) -> &'static str {
    let d = description.to_lowercase();
    KEYWORDS.iter().find(|(_, words)| words.iter().any(|w| d.contains(w))).map(|(g, _)| *g).unwrap_or("More word fixes")
}

/// A rule's name as words: `AcceptExcept` → "Accept except",
/// `DotInitialisms` → "Dot initialisms", `AOkHyphen` → "A ok hyphen".
pub fn rule_title(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev_lower = chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit();
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let prev_upper = chars[i - 1].is_uppercase();
            if prev_lower || (prev_upper && next_lower) {
                out.push(' ');
            }
        }
        // Keep runs of capitals (acronyms) as they are; lower-case the rest.
        let in_acronym = c.is_uppercase()
            && (chars.get(i + 1).is_some_and(|n| n.is_uppercase())
                || (i > 0 && chars[i - 1].is_uppercase() && chars.get(i + 1).is_none_or(|n| !n.is_lowercase())));
        if i == 0 || in_acronym { out.push(c) } else { out.extend(c.to_lowercase()) }
    }
    out
}

/// Whether a description says anything (some Harper rules only have
/// placeholder text).
pub fn is_useful_description(description: &str) -> bool {
    !description.is_empty()
        && !description.starts_with("A collection of linters")
        && !description.starts_with("A linter skeleton")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(rule_title("AcceptExcept"), "Accept except");
        assert_eq!(rule_title("DotInitialisms"), "Dot initialisms");
        assert_eq!(rule_title("UseHTTPSLinks"), "Use HTTPS links");
        assert_eq!(rule_title("AOkHyphen"), "A ok hyphen");
    }

    #[test]
    fn groups() {
        assert_eq!(rule_group("Corrects `at lest` to `at least`."), "Phrases and idioms");
        assert_eq!(rule_group("Looks for incorrect spacing inside the closed compound `someone`."), "Compound words");
        assert_eq!(rule_group("Expands an initialism."), "Abbreviations");
        assert_eq!(rule_group("Something else entirely."), "More word fixes");
        assert!(KEYWORDS.iter().all(|(g, _)| RULE_GROUPS.contains(g)));
    }
}
