//! English grammar, style and tone checking, offline, on top of
//! [Harper](https://writewithharper.com).
//!
//! Adds Xenify's own rules (pronoun case, its/it's, noun–verb agreement,
//! sound-alike words judged by context, embarrassing real-word typos, joining
//! words, word order, commas around names…), writing **profiles** (academic,
//! workplace email, social post…) that decide how informal writing is
//! treated, clarity checks for work writing, and a plain-English explanation
//! with examples for every rule.
//!
//! ```
//! use xenify_grammar::{Checker, Settings};
//! let mut checker = Checker::new(Settings::default(), &[]);
//! for issue in checker.check("Me and him went to the store yesterday.") {
//!     println!("{:?} {}: {}", issue.range, issue.rule, issue.message);
//! }
//! ```
//!
//! Harper uses non-thread-safe internals, so a [`Checker`] is created and
//! used on one thread.

pub mod clarity;
pub mod consistency;
mod explain;
mod grammar;
mod profile;
mod register;
mod rule_groups;
mod rules;
pub mod sound_alike;
mod style;

use harper_core::linting::{LintGroup, LintKind, Suggestion};
use harper_core::spell::{Dictionary, FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::{DictWordMetadata, Document as HDoc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

pub use explain::{Explanation, code_candidates, error_codes, explain, explain_code};
pub use grammar::verb_forms;
pub use profile::{
    Feature, Level, Profile, Profiles, built_in as built_in_profiles, read_profile, slug, write_profile,
};
pub use rule_groups::{RULE_GROUPS, is_useful_description, rule_group, rule_title};
pub use style::{Readability, readability};

/// Rules about commas that style guides prefer but that don't change the
/// meaning; shown only when the profile asks for [`Profile::optional_commas`].
pub const OPTIONAL_PUNCTUATION: &[&str] = &[
    "DiscourseMarkers",
    "HelloGreeting",
    "IntroCueCommaBeforeThanks",
    "IncludingButNotLimitedToPunctuation",
    "GreetingComma",
    "ListColon",
    "Spotter:comma_intro",
];

/// Whether `rule` is an optional-comma rule (see [`OPTIONAL_PUNCTUATION`]).
pub fn is_optional_punctuation(rule: &str) -> bool {
    OPTIONAL_PUNCTUATION.contains(&rule)
}

/// The names of Xenify's own rules (added on top of Harper's).
pub fn own_rules() -> Vec<&'static str> {
    rules::WEIR_RULES.iter().map(|(n, _)| *n).chain(grammar::NAMES.iter().copied()).chain(["SoundAlike"]).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Dialect {
    #[default]
    American,
    British,
    Canadian,
    Australian,
    Indian,
}

impl Dialect {
    /// The variety of English the computer is set to (`LC_ALL`, `LC_MESSAGES`,
    /// `LANG`: `en_AU.UTF-8` → Australian). US English when unknown.
    pub fn from_locale() -> Dialect {
        ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."))
            .and_then(|v| Dialect::parse(&v))
            .unwrap_or_default()
    }

    /// From a locale or short name: `en_AU.UTF-8`, `en-GB`, `au`, `british`…
    pub fn parse(s: &str) -> Option<Dialect> {
        let s = s.to_ascii_lowercase();
        let region = s.split(['.', '@', ':']).next().unwrap_or("").rsplit(['_', '-']).next().unwrap_or("");
        Some(match region {
            "us" | "american" => Dialect::American,
            "gb" | "uk" | "british" | "ie" | "nz" | "za" => Dialect::British,
            "ca" | "canadian" => Dialect::Canadian,
            "au" | "australian" => Dialect::Australian,
            "in" | "indian" => Dialect::Indian,
            _ if s.starts_with("en") => Dialect::American,
            _ => return None,
        })
    }

    pub const ALL: [Dialect; 5] =
        [Dialect::American, Dialect::British, Dialect::Canadian, Dialect::Australian, Dialect::Indian];

    pub fn name(self) -> &'static str {
        match self {
            Dialect::American => "English (US)",
            Dialect::British => "English (UK)",
            Dialect::Canadian => "English (Canada)",
            Dialect::Australian => "English (Australia)",
            Dialect::Indian => "English (India)",
        }
    }

    fn harper(self) -> harper_core::Dialect {
        match self {
            Dialect::American => harper_core::Dialect::American,
            Dialect::British => harper_core::Dialect::British,
            Dialect::Canadian => harper_core::Dialect::Canadian,
            Dialect::Australian => harper_core::Dialect::Australian,
            Dialect::Indian => harper_core::Dialect::Indian,
        }
    }
}

/// What kind of problem an issue is (its squiggle colour and filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Category {
    Spelling,
    Grammar,
    /// Style and clarity (wordiness, passive voice, repetition).
    Style,
    /// Readability (long sentences, adverbs).
    Readability,
    /// Tone and register: informal features the writing profile points out.
    Register,
}

impl Category {
    pub const ALL: [Category; 5] =
        [Category::Spelling, Category::Grammar, Category::Style, Category::Readability, Category::Register];

    pub fn name(self) -> &'static str {
        match self {
            Category::Spelling => "Spelling",
            Category::Grammar => "Grammar",
            Category::Style => "Style",
            Category::Readability => "Readability",
            Category::Register => "Tone & register",
        }
    }
}

/// Rules that are off until the user turns them on: too often wrong.
/// MissingPreposition is right about 1 time in 3 and offers no fix; in a
/// review of their suggestions (2026-10-18) CompoundNouns ("man go" →
/// "mango"), AvoidCurses ("asses" → "butts") and OxfordComma (commas where
/// there's no list) were wrong more often than right, and so was
/// PhrasalVerbAsCompoundNoun ("checkout", "setup", "login" are everyday nouns).
pub const OFF_BY_DEFAULT: &[&str] =
    &["MissingPreposition", "CompoundNouns", "AvoidCurses", "OxfordComma", "PhrasalVerbAsCompoundNoun"];

/// User-configurable checking options (Settings › Proofing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub dialect: Dialect,
    /// Rules turned off everywhere (by name); a profile can turn off more.
    pub disabled_rules: BTreeSet<String>,
    /// Rules from [`OFF_BY_DEFAULT`] the user turned on.
    pub turned_on: BTreeSet<String>,
    /// Don't flag unknown capitalised words mid-sentence (names, brands).
    pub ignore_names: bool,
    /// The profile for documents that don't choose one.
    pub default_profile: String,
    /// The profile in use (resolved by the app from the document's choice
    /// or the default; not saved with the settings).
    #[serde(skip)]
    pub profile: Profile,
}

impl Settings {
    /// Rules turned off everywhere: the user's, plus [`OFF_BY_DEFAULT`]
    /// ones not turned on.
    pub fn rules_off(&self) -> BTreeSet<String> {
        let mut off = self.disabled_rules.clone();
        off.extend(OFF_BY_DEFAULT.iter().filter(|r| !self.turned_on.contains(**r)).map(|r| r.to_string()));
        off
    }

    /// Sets the rules turned off everywhere (as [`Settings::rules_off`] gives them).
    pub fn set_rules_off(&mut self, off: BTreeSet<String>) {
        self.turned_on = OFF_BY_DEFAULT.iter().filter(|r| !off.contains(**r)).map(|r| r.to_string()).collect();
        self.disabled_rules = off.into_iter().filter(|r| !OFF_BY_DEFAULT.contains(&r.as_str())).collect();
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            dialect: Dialect::American,
            disabled_rules: BTreeSet::new(),
            turned_on: BTreeSet::new(),
            ignore_names: true,
            default_profile: "standard".into(),
            profile: Profile::default(),
        }
    }
}

/// One problem found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    /// Byte range in the checked text.
    pub range: Range<usize>,
    pub category: Category,
    /// Rule name (for "turn this rule off").
    pub rule: String,
    pub message: String,
    /// Replacements for the range (an empty string removes it).
    pub suggestions: Vec<String>,
    /// For tone & register issues: the informal feature found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<Feature>,
    /// The profile flags (rather than just notes) this feature.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strong: bool,
}

impl Issue {
    /// A key that identifies this issue in this sentence context (for "ignore once").
    pub fn ignore_key(&self, text: &str) -> String {
        let word = text.get(self.range.clone()).unwrap_or("");
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        format!("{}:{}:{:x}", self.rule, word, h.finish())
    }
}

/// Per-paragraph issues without the paragraph id (cacheable by text).
#[derive(Clone)]
struct Local {
    range: Range<usize>,
    category: Category,
    rule: String,
    message: String,
    suggestions: Vec<String>,
    feature: Option<Feature>,
    strong: bool,
}

pub struct Checker {
    settings: Settings,
    linter: LintGroup,
    cache: HashMap<u64, Vec<Local>>,
    user_words: BTreeSet<String>,
    /// Correct words the dictionary misses, lower-case (rules/dictionary/accepted.txt).
    accepted: std::collections::HashSet<String>,
}

fn category_of(kind: LintKind) -> Category {
    match kind {
        LintKind::Spelling => Category::Spelling,
        LintKind::Agreement
        | LintKind::Grammar
        | LintKind::Punctuation
        | LintKind::Capitalization
        | LintKind::BoundaryError
        | LintKind::Malapropism
        | LintKind::Eggcorn
        | LintKind::Typo
        | LintKind::Miscellaneous => Category::Grammar,
        LintKind::Readability => Category::Readability,
        _ => Category::Style,
    }
}

impl Checker {
    /// A checker with `settings`, accepting the words in `user_words`.
    pub fn new(settings: Settings, user_words: &[String]) -> Self {
        let mut dict = MergedDictionary::new();
        dict.add_dictionary(FstDictionary::curated());
        let mut mine = MutableDictionary::new();
        mine.extend_words(user_words.iter().map(|w| (w.chars().collect::<Vec<_>>(), DictWordMetadata::default())));
        dict.add_dictionary(Arc::new(mine));
        let mut linter = LintGroup::new_curated(Arc::new(dict), settings.dialect.harper());
        rules::add_to(&mut linter);
        for rule in settings.rules_off().iter().chain(&settings.profile.disabled_rules) {
            linter.config.set_rule_enabled(rule, false);
        }
        let accepted = accepted_words(settings.dialect).map(str::to_string).collect();
        Self { settings, linter, cache: HashMap::new(), user_words: user_words.iter().cloned().collect(), accepted }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Harper's rules with their descriptions (for the settings UI).
    pub fn rules(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            self.linter.all_descriptions().into_iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
        v.sort();
        v
    }

    fn key(&self, text: &str) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        h.finish()
    }

    /// Issues in one paragraph's text (cached by text).
    fn check_text(&mut self, text: &str) -> Vec<Local> {
        let key = self.key(text);
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        let mut out = Vec::new();
        if !text.trim().is_empty() {
            // Char index → byte offset.
            let bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
            let doc = HDoc::new_plain_english_curated(text);
            // By rule, so each issue carries the rule's name (for "turn this
            // rule off" and per-rule scoring).
            let lints: Vec<(String, harper_core::linting::Lint)> = self
                .linter
                .organized_lints(&doc)
                .into_iter()
                .flat_map(|(rule, lints)| lints.into_iter().map(move |l| (rule.clone(), l)))
                .collect();
            for (rule, lint) in lints {
                let (s, e) = (lint.span.start.min(bytes.len() - 1), lint.span.end.min(bytes.len() - 1));
                let range = bytes[s]..bytes[e];
                let word = &text[range.clone()];
                // Accepted words: no spelling flag, and no "split this word" either.
                if (lint.lint_kind == LintKind::Spelling || rule == "SplitWords")
                    && self.accepted.contains(&word.to_lowercase())
                {
                    continue;
                }
                if lint.lint_kind == LintKind::Spelling
                    && (self.user_words.contains(word)
                        || skip_spelling(text, range.start, word, &lint.suggestions, &self.settings))
                {
                    continue;
                }
                if known_false_alarm(&rule, text, &range) || inside_word(text, &range) {
                    continue;
                }
                // "Sam" → "SAM", "Mia" → "MIA": a name written as a name isn't an acronym.
                let all_caps = |c: &Vec<char>| c.iter().all(|c| !c.is_lowercase());
                if rule == "OrthographicConsistency"
                    && word.chars().next().is_some_and(char::is_uppercase)
                    && word.chars().skip(1).all(char::is_lowercase)
                    && lint.suggestions.iter().all(|s| matches!(s, Suggestion::ReplaceWith(c) if all_caps(c)))
                {
                    continue;
                }
                let mut suggestions: Vec<String> = lint
                    .suggestions
                    .iter()
                    .take(6)
                    .map(|s| match s {
                        Suggestion::ReplaceWith(c) => c.iter().collect(),
                        Suggestion::InsertAfter(c) => format!("{word}{}", c.iter().collect::<String>()),
                        Suggestion::Remove => String::new(),
                    })
                    .filter(|s: &String| !offensive(s))
                    .collect();
                // "I need it ASAP" → "as soon as possible": an expanded
                // abbreviation mid-sentence starts with a small letter.
                let mid_sentence = !text[..range.start].trim_end().is_empty()
                    && !text[..range.start].trim_end().ends_with(['.', '!', '?', ':', '"', '\u{201c}']);
                if mid_sentence && word.chars().count() > 1 && word.chars().all(|c| c.is_uppercase()) {
                    for s in &mut suggestions {
                        let words: Vec<&str> = s.split(' ').collect();
                        // Only when it became several words ("As soon as possible"), not "Usa" → "USA".
                        if words.len() > 1 && words[1..].iter().all(|w| w.chars().all(|c| !c.is_uppercase())) {
                            let mut c = s.chars();
                            *s = c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default();
                        }
                    }
                }
                // A spelling guess far from what was typed is noise ("nigiri" → "nigh"):
                // better to mark the word unknown than to suggest something silly.
                if lint.lint_kind == LintKind::Spelling {
                    suggestions.retain(|s| close_spelling(word, s));
                    // With a near-perfect match, drop the worse ones ("occured" → "occurred", not "obscured").
                    let best = suggestions.iter().map(|s| edit_distance(word, s)).min().unwrap_or(0);
                    suggestions.retain(|s| edit_distance(word, s) <= best.max(1));
                }
                out.push(Local {
                    range,
                    category: category_of(lint.lint_kind),
                    rule,
                    message: lint.message.clone(),
                    suggestions,
                    feature: None,
                    strong: false,
                });
            }
            // "This sentence has no verb" is wrong when the verb is just misspelled ("The delay occured").
            let misspelt: Vec<Range<usize>> =
                out.iter().filter(|l| l.category == Category::Spelling).map(|l| l.range.clone()).collect();
            out.retain(|l| {
                l.rule != "NoVerb" || !misspelt.iter().any(|r| r.start < l.range.end && l.range.start < r.end)
            });
            // A sound-alike flag adds nothing where another rule already marks the words.
            let marked: Vec<Range<usize>> =
                out.iter().filter(|l| l.rule != "SoundAlike").map(|l| l.range.clone()).collect();
            out.retain(|l| {
                l.rule != "SoundAlike" || !marked.iter().any(|r| r.start < l.range.end && l.range.start < r.end)
            });
            out.extend(style::check(text, &self.settings.profile));
            out.extend(clarity::check(text, &self.settings.profile));
            register::apply(text, &self.settings.profile, &mut out);
            // Words the user added to their dictionary aren't informal notes either.
            out.retain(|l| {
                l.category != Category::Register
                    || !self.user_words.iter().any(|w| w.eq_ignore_ascii_case(&text[l.range.clone()]))
            });
        }
        if self.cache.len() > 20_000 {
            self.cache.clear();
        }
        self.cache.insert(key, out.clone());
        out
    }

    /// The issues in `text` (one paragraph), as the settings and profile ask.
    pub fn check(&mut self, text: &str) -> Vec<Issue> {
        if !self.settings.enabled {
            return Vec::new();
        }
        let cats = self.settings.profile.categories.clone();
        let optional = self.settings.profile.optional_commas;
        let issues = self
            .check_text(text)
            .into_iter()
            .filter(|l| cats.contains(&l.category))
            .filter(|l| optional || !is_optional_punctuation(&l.rule))
            .map(|l| Issue {
                range: l.range,
                category: l.category,
                rule: l.rule,
                message: l.message,
                suggestions: l.suggestions,
                feature: l.feature,
                strong: l.strong,
            })
            .collect();
        let mut issues = merge_same_words(text, without_notes_on_errors(issues));
        for i in issues.iter_mut().filter(|i| i.category == Category::Grammar && i.suggestions.len() > 1) {
            self.drop_fixes_that_break(text, i);
        }
        issues
    }

    /// Drops suggestions that would leave a new grammar mistake right where
    /// they go: for "Their is a lot", "They're" and "There's" give "They're
    /// is", so only "There" is offered. If every suggestion breaks something
    /// they're all kept.
    fn drop_fixes_that_break(&mut self, text: &str, issue: &mut Issue) {
        let r = issue.range.clone();
        // The edit and the word after it.
        let after = text[r.end..].trim_start();
        let next_end = after.find(|c: char| c.is_whitespace() || ".,;:!?".contains(c)).unwrap_or(after.len());
        let tail = text.len() - after.len() + next_end - r.end;
        let near = |l: &Local, len: usize| {
            matches!(l.category, Category::Grammar) && l.range.start <= r.start + len + tail && l.range.end >= r.start
        };
        let before: Vec<String> =
            self.check_text(text).into_iter().filter(|l| near(l, r.len()) && l.range != r).map(|l| l.rule).collect();
        let ok: Vec<String> = issue
            .suggestions
            .iter()
            .filter(|s| {
                let fixed = format!("{}{}{}", &text[..r.start], s, &text[r.end..]);
                !self.check_text(&fixed).iter().any(|l| near(l, s.len()) && !before.contains(&l.rule))
            })
            .cloned()
            .collect();
        if !ok.is_empty() {
            issue.suggestions = ok;
        }
    }

    /// The issues in a whole document, given as paragraphs in order: each
    /// paragraph's issues plus the whole-document checks (a request buried
    /// at the end of an email, acronyms used before they're spelled out).
    /// Each issue comes with its paragraph's index.
    pub fn check_paragraphs(&mut self, paras: &[String]) -> Vec<(usize, Issue)> {
        let mut out: Vec<(usize, Issue)> =
            paras.iter().enumerate().flat_map(|(i, t)| self.check(t).into_iter().map(move |x| (i, x))).collect();
        out.extend(self.check_whole(paras));
        out
    }

    /// Only the whole-document checks (see [`Checker::check_paragraphs`]).
    pub fn check_whole(&self, paras: &[String]) -> Vec<(usize, Issue)> {
        if !self.settings.enabled || !self.settings.profile.categories.contains(&Category::Style) {
            return Vec::new();
        }
        let p = &self.settings.profile;
        let mut found = clarity::check_document(paras, p);
        if p.consistency {
            found.extend(consistency::check(paras));
        }
        if p.word_choice {
            found.extend(consistency::check_words(paras));
        }
        found
            .into_iter()
            .filter(|(_, i)| {
                !self.settings.rules_off().contains(&i.rule) && !self.settings.profile.disabled_rules.contains(&i.rule)
            })
            .collect()
    }
}

/// Rules that guess wildly when a word is just misspelled: "adress" → "a
/// dress", "informations" → "in formations" / "deformations".
const LONG_SHOTS: [&str; 2] = ["SplitWords", "WrongNegative"];
const MAX_SUGGESTIONS: usize = 3;

/// One issue per stretch of words. Several rules often flag the same word
/// ("informations": not countable, misspelled, "in formations"); the most
/// specific explains it and the others only add suggestions. Long-shot
/// guesses are dropped when a better rule flagged the word, and spelling
/// suggestions with an apostrophe the writer didn't type ("information's")
/// go last. Notes on tone (Register) stay separate.
/// Drops style and readability notes on words that already have a spelling or
/// grammar error ("what were left": fix `were` first, not the passive voice).
fn without_notes_on_errors(mut issues: Vec<Issue>) -> Vec<Issue> {
    let errors: Vec<Range<usize>> = issues
        .iter()
        .filter(|i| matches!(i.category, Category::Spelling | Category::Grammar))
        .map(|i| i.range.clone())
        .collect();
    issues.retain(|i| {
        !matches!(i.category, Category::Style | Category::Readability)
            || !errors.iter().any(|e| e.start < i.range.end && i.range.start < e.end)
    });
    issues
}

fn merge_same_words(text: &str, issues: Vec<Issue>) -> Vec<Issue> {
    let rank = |i: &Issue| {
        if LONG_SHOTS.contains(&i.rule.as_str()) {
            2
        } else if i.rule == "SpellCheck" {
            1
        } else {
            0
        }
    };
    let mut out: Vec<Issue> = Vec::new();
    let mut groups: Vec<Vec<Issue>> = Vec::new();
    for i in issues {
        if i.category == Category::Register {
            // One tone note per word; a masked word ("****ed") isn't a suggestion.
            let mut i = i;
            i.suggestions.retain(|s| !s.contains("**"));
            match out.iter_mut().find(|o| o.category == Category::Register && o.range == i.range) {
                Some(o) if o.suggestions.is_empty() => *o = i,
                Some(_) => {}
                None => out.push(i),
            }
        } else if let Some(g) = groups.iter_mut().find(|g| g[0].range == i.range) {
            g.push(i);
        } else {
            groups.push(vec![i]);
        }
    }
    for mut g in groups {
        g.sort_by_key(rank);
        let best = rank(&g[0]);
        let typed_apostrophe = text[g[0].range.clone()].contains(['\'', '’']);
        let mut all: Vec<String> = Vec::new();
        for i in g.iter().filter(|i| rank(i) == best || rank(i) < 2) {
            let mut own = i.suggestions.clone();
            if i.rule == "SpellCheck" && !typed_apostrophe {
                // Keep "that's" for "thats", not "acre's" for "adress".
                let typed = text[i.range.clone()].to_lowercase();
                own.retain(|s| !s.contains(['\'', '’']) || s.replace(['\'', '’'], "").to_lowercase() == typed);
                own.sort_by_key(|s| s.contains(['\'', '’']));
            }
            for s in own {
                if !all.contains(&s) {
                    all.push(s);
                }
            }
        }
        all.truncate(MAX_SUGGESTIONS.max(g[0].suggestions.len()));
        let mut first = g.swap_remove(0);
        if first.rule == "SpellCheck" {
            // Harper says "Did you mean to spell `x` this way?"
            let word = &text[first.range.clone()];
            first.message = match all.first() {
                Some(s) => format!("“{word}” may be misspelled. Did you mean “{s}”?"),
                None => format!("“{word}” isn't in the dictionary. Check the spelling, or add it to your dictionary."),
            };
        }
        first.strong |= g.iter().any(|i| i.strong);
        first.suggestions = all;
        out.push(first);
    }
    out.sort_by_key(|i| (i.range.start, i.range.end));
    out
}

/// Known wrong flags, found by running the checker over edited published
/// text (public-domain books) and correct AI-written text. The rules stay on;
/// only these patterns are dropped. Each has a test in `known_false_alarms_are_dropped`.
fn known_false_alarm(rule: &str, text: &str, range: &Range<usize>) -> bool {
    let flagged = text[range.clone()].trim().to_lowercase();
    let split = |t: &str| -> Vec<String> {
        t.split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let next_words = |n: usize| -> Vec<String> { split(&text[range.end..]).into_iter().take(n).collect() };
    let prev_words = |n: usize| -> Vec<String> {
        let mut v = split(&text[..range.start]);
        v.drain(..v.len().saturating_sub(n));
        v
    };
    let prev = prev_words(1).pop().unwrap_or_default();
    let next = next_words(1).pop().unwrap_or_default();
    // "He had had enough", "I knew that that was wrong": doubled on purpose.
    if rule == "RepeatedWords" && matches!(flagged.as_str(), "had had" | "that that") {
        return true;
    }
    // "Or are you…", "And then…": a conjunction starting a sentence isn't someone addressed.
    if rule == "AddressComma" && matches!(flagged.as_str(), "or" | "and" | "but" | "so" | "nor" | "yet") {
        return true;
    }
    const PREPS: &[&str] = &[
        "with",
        "of",
        "in",
        "on",
        "to",
        "for",
        "from",
        "by",
        "at",
        "into",
        "onto",
        "through",
        "under",
        "over",
        "about",
        "upon",
        "within",
        "without",
        "across",
        "against",
        "despite",
        "towards",
        "toward",
        "among",
        "between",
        "behind",
        "beneath",
        "beside",
        "beyond",
        "around",
        "along",
        "inside",
        "outside",
        "throughout",
        "during",
    ];
    match rule {
        // "he sprang", "she flung": past tenses read as plain verbs.
        // "he pooh-poohed": part of a hyphenated word.
        "PronounVerbAgreement" => IRREGULAR_PAST.contains(&flagged.as_str()) || text[range.end..].starts_with('-'),
        "SimplePastToPastParticiple" => text[range.end..].starts_with('-'),
        // Harper's pattern is noun + be/have + adjective + noun. Not a missing
        // preposition: a clause after the adjective ("afraid it was", "certain
        // that…"), or "have" meaning possess ("they had little housework").
        "MissingPreposition" => {
            let after = next_words(3);
            const CLAUSE: &[&str] = &["that", "what", "which", "how", "whether", "who", "if", "why", "where", "when"];
            const PRONOUNS: &[&str] = &["it", "he", "she", "they", "we", "i", "you", "there"];
            const VERBS: &[&str] = &[
                "was", "is", "were", "are", "had", "has", "have", "would", "will", "could", "can", "did", "does", "do",
                "should", "might", "must",
            ];
            // A pronoun starting a clause has its own verb: "afraid it was", "sure they had".
            let pronoun_clause = after.get(1).is_some_and(|w| PRONOUNS.contains(&w.as_str()))
                && after.get(2).is_some_and(|v| VERBS.contains(&v.as_str()) || v.ends_with("ed"));
            let clause = after.get(1).is_some_and(|w| CLAUSE.contains(&w.as_str())) || pronoun_clause;
            let possess = matches!(flagged.as_str(), "had" | "has" | "have")
                || after.first().is_some_and(|w| matches!(w.as_str(), "had" | "has" | "have"));
            clause || possess
        }
        // "every one of them" is correct.
        "Everyone" => next == "of",
        // "a passion", "a crossing", "a fondness": nouns that are countable too.
        "MassNouns" => {
            let noun = flagged.rsplit(' ').next().unwrap_or("");
            noun.ends_with("ing")
                || noun.ends_with("ings")
                || noun.ends_with("ness")
                || noun.ends_with("nesses")
                || noun.ends_with("ities")
                || COUNTABLE_TOO.contains(&noun)
        }
        // "with its icy fingers": after a preposition, "its" is possessive.
        // "its largest listed company", "give its best": a superlative after "its" is possessive.
        "ItsForItIs" => {
            PREPS.contains(&prev.as_str())
                || (next.ends_with("est") && next.len() > 4)
                || matches!(next.as_str(), "best" | "worst" | "most" | "least" | "own")
        }
        // "quite a rage", "quite another fashion": "quite" meaning "really".
        "QuiteQuiet" => {
            flagged == "quite"
                && matches!(
                    next.as_str(),
                    "a" | "an"
                        | "another"
                        | "the"
                        | "some"
                        | "as"
                        | "so"
                        | "right"
                        | "sure"
                        | "true"
                        | "different"
                        | "possibly"
                        | "likely"
                        | "enough"
                        | "well"
                        | "often"
                        | "simply"
                        | "clearly"
                )
        }
        // "Thus it was that…", "So it is that…": a set phrase, no comma.
        "DiscourseMarkers" => {
            let n = next_words(3).join(" ");
            ["it was that", "it is that", "it came about", "it happened"].iter().any(|p| n.starts_with(p))
        }
        // A capital isn't needed after an abbreviation ("Glassware Co. was", "per
        // cent. profit") or after a short exclamation inside a sentence ("Ah! at
        // last…", "Nasty! they are…"), common in fiction and quoted speech.
        "SentenceCapitalization" => {
            let before = text[..range.start].trim_end();
            let abbrev = before.ends_with('.') && ABBREVIATIONS.contains(&prev.as_str());
            let exclamation = (before.ends_with('!') || before.ends_with('?')) && {
                let start = before[..before.len() - 1].rfind(['.', '!', '?', '\n']).map_or(0, |i| i + 1);
                split(&before[start..]).len() <= 2
            };
            abbrev || exclamation
        }
        // "as if a demon were after him": a subjunctive. "…the world wonderful
        // things have been…": a plural noun right before the verb agrees with it.
        "NounSubjectAgreement" => {
            let earlier = prev_words(12).join(" ");
            let subjunctive = flagged == "were"
                && (earlier.contains("as if")
                    || earlier.contains("as though")
                    || earlier.contains("wish")
                    || earlier.split(' ').any(|w| w == "if"));
            // Not when the noun ends an "of/in…" phrase: "Each of the students have" is a real mistake.
            let after_prep = prev_words(4).iter().any(|w| PREPS.contains(&w.as_str()));
            let plural_before = !after_prep
                && matches!(flagged.as_str(), "are" | "were" | "have" | "do")
                && prev.len() > 3
                && prev.ends_with('s')
                && !prev.contains(['\'', '\u{2019}'])
                && !prev.ends_with("ss")
                && !prev.ends_with("us")
                && !prev.ends_with("is");
            // "keen that the club not attach itself": a subjunctive after "that … not".
            let not_subjunctive = prev == "not" && earlier.split(' ').any(|w| w == "that");
            // "Lifeboat Associates was a company": a name ending in -s is one thing.
            let name_s = flagged == "was" || flagged == "is";
            let name_s = name_s && {
                let before = text[..range.start].trim_end();
                let last = before.rsplit(' ').next().unwrap_or("");
                // Not "the Trojans was": a team or group with "the" is plural.
                let the = split(before).iter().rev().nth(1).is_some_and(|w| w == "the");
                last.starts_with(char::is_uppercase) && last.ends_with('s') && before.contains(' ') && !the
            };
            subjunctive || plural_before || not_subjunctive || name_s
        }
        // "for they got", "the visit they were to make": "they" as a subject.
        "TheyToThem" => {
            matches!(
                next.as_str(),
                "were"
                    | "was"
                    | "are"
                    | "had"
                    | "have"
                    | "got"
                    | "did"
                    | "do"
                    | "would"
                    | "could"
                    | "should"
                    | "will"
                    | "can"
                    | "might"
                    | "must"
                    | "went"
                    | "came"
                    | "said"
                    | "knew"
                    | "saw"
                    | "made"
                    | "took"
                    | "wanted"
            ) || next.ends_with("ed")
        }
        // "show who was master": "who" is the subject, so no inversion.
        // "they always are.", "as she always was to see…", "there never was a…":
        // an adverb before a stressed or clause-ending "be" is correct.
        "WordOrder" => {
            let subject_who = ["who was ", "who is ", "who were "].iter().any(|p| flagged.starts_with(p));
            let two = split(&text[range.clone()]);
            let adverb_be = two.len() == 2
                && matches!(two[1].as_str(), "am" | "is" | "are" | "was" | "were")
                && (text[range.end..]
                    .starts_with(|c: char| c.is_ascii_punctuation() || c == '\u{2019}' || c == '\u{201d}')
                    || matches!(next.as_str(), "a" | "an" | "the" | "as" | "to" | "more" | "such" | "so" | "any")
                    || prev == "there"
                    || prev_words(3).iter().any(|w| w == "as"));
            subject_who || adverb_be
        }
        // "The grass about us was on fire": "us" is after a preposition, not the subject.
        "PronounInflectionBe" => {
            let two = prev_words(2);
            two.len() == 2
                && matches!(two[1].as_str(), "us" | "them" | "me" | "him" | "her" | "you")
                && PREPS.contains(&two[0].as_str())
        }
        // "talked awhile" is correct; only "for awhile" needs "a while".
        "AWhile" => prev != "for",
        // "excited over all these prospects": "over" + "all", not "overall".
        "Overall" => {
            next == "other"
                || next.ends_with('s') && next.len() > 3
                || matches!(
                    next.as_str(),
                    "the"
                        | "these"
                        | "those"
                        | "this"
                        | "that"
                        | "his"
                        | "her"
                        | "their"
                        | "its"
                        | "my"
                        | "our"
                        | "your"
                        | "of"
                )
        }
        // "Was it chance that Mr. Gillingham…?": the full stop of "Mr." is not the
        // end of a question. "And so well did he face it, that…": an inverted
        // statement, not a question.
        "QuestionMark" => {
            let before = &text[..range.start];
            let abbrev = ABBREVIATIONS.contains(&prev.as_str()) && !before.ends_with(' ');
            // "Jim West Jr. was the heir…": the "question" only starts after an abbreviation.
            let after_abbrev = before.rfind('.').is_some_and(|d| {
                let w = split(&before[..d]).pop().unwrap_or_default();
                ABBREVIATIONS.contains(&w.as_str())
            });
            let abbrev = abbrev || after_abbrev;
            let start = before.rfind(['!', '?', '\n']).map_or(0, |i| i + 1);
            let opener = split(&before[start..]).into_iter().take(3).collect::<Vec<_>>().join(" ");
            let inverted = [
                "so ",
                "and so ",
                "nor ",
                "neither ",
                "never ",
                "not only",
                "only then",
                "little did",
                "rarely ",
                "seldom ",
                "hardly ",
                "scarcely ",
            ]
            .iter()
            .any(|o| format!("{opener} ").starts_with(o));
            abbrev || inverted
        }
        // "if Mr. Brown is…", "that Mr. Smith requires": a title after a
        // joining word or preposition, not someone being addressed.
        "AddressComma" => {
            let words = split(&text[range.clone()]);
            // Someone is addressed after a greeting, thanks or "you" ("I love you Mom"),
            // or with an address word ("eat Grandma").
            let addressing = words.first().is_some_and(|w| {
                grammar::GREETINGS.contains(&w.as_str())
                    || grammar::THANKS.contains(&w.as_str())
                    || matches!(w.as_str(), "you" | "thank" | "see" | "love" | "miss" | "ya")
            }) || words.last().is_some_and(|w| grammar::ADDRESS_WORDS.contains(&w.as_str()));
            // "old Hugh respect it", "No Othello ever…", "Uncle Bob said": the word before
            // belongs to the name phrase, and more words follow, so no one is addressed.
            // (A name elsewhere often is addressed: "Meet me at the park Luis at 5pm.")
            let more_words = text[range.end..].trim_start().starts_with(char::is_alphabetic);
            let part_of_name = words.first().is_some_and(|w| {
                matches!(
                    w.as_str(),
                    "no" | "old"
                        | "young"
                        | "little"
                        | "poor"
                        | "like"
                        | "the"
                        | "a"
                        | "an"
                        | "uncle"
                        | "aunt"
                        | "cousin"
                        | "brother"
                        | "sister"
                        | "saint"
                        | "king"
                        | "queen"
                        | "lord"
                        | "lady"
                        | "father"
                        | "mother"
                        | "servant"
                        | "friend"
                )
            });
            let name_after_other = words.len() == 2 && !addressing && more_words && part_of_name;
            name_after_other
                || words.len() == 2
                    && ABBREVIATIONS.contains(&words[1].as_str())
                    && (PREPS.contains(&words[0].as_str())
                        || matches!(
                            words[0].as_str(),
                            "if" | "that"
                                | "when"
                                | "and"
                                | "but"
                                | "or"
                                | "because"
                                | "as"
                                | "than"
                                | "where"
                                | "while"
                                | "unless"
                                | "until"
                                | "whether"
                                | "though"
                                | "although"
                                | "since"
                                | "which"
                                | "who"
                        ))
        }
        // "May turned to her husband": the name May. "would later deny", "would
        // soon become": an adverb between the modal and the verb.
        "ModalBeAdjective" => {
            (text[range.clone()].starts_with('M') && flagged == "may" && next != "be")
                || matches!(
                    next.as_str(),
                    "later"
                        | "soon"
                        | "never"
                        | "always"
                        | "also"
                        | "often"
                        | "still"
                        | "then"
                        | "eventually"
                        | "again"
                        | "rather"
                        | "probably"
                        | "certainly"
                        | "usually"
                        | "only"
                        | "even"
                        | "not"
                )
        }
        // "non-existent", "co-educational", "post-season": hyphenated prefixes are
        // standard. Only the open spelling ("pre season", "post war") is flagged.
        "DisjointPrefixes" => flagged.contains('-'),
        // "National League A", "Division 3A": a letter used as a label, not "a".
        // "an honourable", "an hour": a silent h takes "an".
        "AnA" => {
            let label = text[range.clone()] == *"A"
                && prev_words(1).first().is_some_and(|w| {
                    text[..range.start].trim_end().ends_with(|c: char| c.is_uppercase() || c.is_ascii_digit())
                        || matches!(w.as_str(), "league" | "division" | "group" | "class" | "plan" | "type" | "grade")
                });
            let silent_h =
                flagged == "an" && ["honour", "honor", "hour", "heir", "honest"].iter().any(|h| next.starts_with(h));
            label || silent_h
        }
        // "the youngest of whom is", "to whom did you speak": after a preposition "whom" is right.
        // "the youngest of whom is", "both of whom would": partitive "of whom".
        // "To whom did you speak?": a question, "whom" is the object.
        // ("no course to whom has opened…" is still flagged: there "who" is meant.)
        "WhomSubjectOfVerb" => {
            prev == "of"
                || (PREPS.contains(&prev.as_str())
                    && matches!(
                        next.as_str(),
                        "did"
                            | "do"
                            | "does"
                            | "should"
                            | "would"
                            | "could"
                            | "can"
                            | "will"
                            | "shall"
                            | "must"
                            | "might"
                    ))
        }
        // "(i) the acquisition…": a roman numeral in a list.
        "CapitalizePersonalPronouns" => text[..range.start].ends_with('(') && text[range.end..].starts_with(')'),
        // "a way he had of laughing": "had" + "of" + -ing.
        "HadOf" => next.ends_with("ing"),
        // "ways of saying no in English".
        "NotIn" => matches!(prev.as_str(), "say" | "says" | "said" | "saying" | "answer" | "answered"),
        // "crept in while Buck…": "in" belongs to the verb, "while" starts a clause.
        "InAWhile" => {
            let after = text[range.end..].trim_start();
            after.starts_with(char::is_uppercase)
                || matches!(
                    next.as_str(),
                    "the" | "a" | "an" | "he" | "she" | "they" | "we" | "i" | "you" | "it" | "his" | "her" | "their"
                )
        }
        // "his own dead little one": "little one" means a child.
        // "some twenty people": "some" + a number means "about".
        "QuantifierNumeralConflict" => {
            flagged.ends_with("little one")
                || flagged.ends_with("little ones")
                || (flagged.starts_with("some ") && !flagged.ends_with(" one"))
        }
        _ => false,
    }
}

/// Abbreviations that end with a full stop mid-sentence.
const ABBREVIATIONS: &[&str] = &[
    "co", "inc", "ltd", "corp", "bros", "cent", "mr", "mrs", "ms", "dr", "st", "jr", "sr", "vs", "etc", "approx", "no",
    "vol", "fig", "dept", "est", "govt", "prof", "rev", "gen", "col", "capt", "lt", "sgt", "ave", "rd", "mt",
];

/// Nouns Harper treats as mass nouns that are also countable ("a passion for…").
const COUNTABLE_TOO: &[&str] = &[
    "passion",
    "thief",
    "prayer",
    "pride",
    "lust",
    "fungus",
    "ease",
    "awe",
    "affection",
    "knowledge",
    "violence",
    "likelihood",
    "appreciation",
    "agitation",
    "expansion",
    "dilation",
    "disgust",
    "froth",
    "lather",
    "slumber",
    "rent",
    "longing",
    "conventionalities",
    "witcheries",
    "dishonesties",
    "antivenom",
    "warmth",
    "excitations",
    "hydrogens",
];

/// Irregular past-tense verbs ("sprang", "flung"), which Harper's
/// PronounVerbAgreement can mistake for plain verbs after "he"/"she"/"it".
const IRREGULAR_PAST: &[&str] = &[
    "arose",
    "awoke",
    "beheld",
    "bent",
    "bit",
    "bled",
    "blew",
    "bore",
    "bound",
    "bred",
    "brought",
    "built",
    "burst",
    "caught",
    "chose",
    "clung",
    "crept",
    "dealt",
    "dug",
    "dove",
    "drew",
    "drank",
    "drove",
    "dwelt",
    "fed",
    "fell",
    "felt",
    "fled",
    "flew",
    "flung",
    "forbade",
    "forgave",
    "forgot",
    "froze",
    "gave",
    "grew",
    "ground",
    "hid",
    "held",
    "hung",
    "kept",
    "knelt",
    "knew",
    "laid",
    "leapt",
    "learnt",
    "led",
    "leant",
    "lent",
    "lit",
    "lost",
    "meant",
    "met",
    "paid",
    "rang",
    "rode",
    "rose",
    "sank",
    "sang",
    "saw",
    "sat",
    "sent",
    "shook",
    "shone",
    "shot",
    "shrank",
    "slept",
    "slid",
    "slung",
    "smelt",
    "sold",
    "sought",
    "spat",
    "sped",
    "spent",
    "spoke",
    "sprang",
    "spun",
    "stank",
    "stole",
    "stood",
    "strode",
    "struck",
    "strove",
    "stuck",
    "stung",
    "swam",
    "swept",
    "swore",
    "swung",
    "taught",
    "thought",
    "threw",
    "thrust",
    "told",
    "took",
    "tore",
    "trod",
    "understood",
    "unwound",
    "wept",
    "woke",
    "won",
    "wore",
    "wound",
    "wove",
    "wrote",
    "wrung",
    // Same form as the present: "she cast", "he split".
    "cast",
    "split",
    "rent",
    "put",
    "hit",
    "set",
    "cut",
    "shut",
    "let",
    "hurt",
    "spread",
    "quit",
    "bet",
    "cost",
    "slit",
    "burst",
    "thrust",
    "shed",
    "spat",
];

/// Correct words the dictionary misses (or marks as another dialect's), for `dialect`.
fn accepted_words(dialect: Dialect) -> impl Iterator<Item = &'static str> {
    let commonwealth = dialect != Dialect::American;
    let mut section = "";
    include_str!("../rules/dictionary/accepted.txt").lines().filter_map(move |l| {
        let l = l.trim();
        if let Some(name) = l.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            return None;
        }
        let wanted = section == "all" || (section == "commonwealth" && commonwealth);
        (wanted && !l.is_empty() && !l.starts_with('#')).then_some(l)
    })
}

/// Titles written with a full stop ("Mrs. Liu", "Dr. Kim").
const TITLES: &[&str] =
    &["mr", "mrs", "ms", "dr", "st", "prof", "sr", "jr", "rev", "gen", "capt", "lt", "col", "sgt", "hon", "mx"];

/// Unknown words not worth flagging: single letters and digits (pieces of
/// `Wi-Fi`, `f/2.8`), the second half of compounds like `kind-hearted`, and,
/// when [`Settings::ignore_names`] is on, names: capitalised words in
/// mid-sentence, and at the start of a sentence unless the word looks like a
/// slip on a common opening word ("Becuase", "Wich").
fn skip_spelling(text: &str, start: usize, word: &str, suggestions: &[Suggestion], settings: &Settings) -> bool {
    if word.chars().count() <= 1 || word.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    // "kind-hearted", "good-natured", "long-legged": noun + "-ed" after a hyphen.
    let after_hyphen = text[..start].ends_with('-') && text[..start.saturating_sub(1)].ends_with(char::is_alphabetic);
    if after_hyphen {
        let dict = FstDictionary::curated();
        let lower = word.to_lowercase();
        if [lower.strip_suffix("ed"), lower.strip_suffix('d')]
            .into_iter()
            .flatten()
            .any(|stem| dict.contains_word_str(stem))
        {
            return true;
        }
    }
    if !settings.ignore_names {
        return false;
    }
    // Name particles before a name ("Fond du Lac", "Ludwig van Beethoven", "Juan del Castillo"),
    // and "et al.".
    let lower = word.to_lowercase();
    let next_capital = text[start + word.len()..].trim_start().starts_with(char::is_uppercase);
    if (next_capital && NAME_PARTICLES.contains(&lower.as_str()))
        || (lower == "al" && text[..start].trim_end().to_lowercase().ends_with("et"))
    {
        return true;
    }
    let capitalised = word.chars().next().is_some_and(char::is_uppercase);
    if !capitalised {
        return false;
    }
    let before = text[..start].trim_end();
    // "Mrs. Liu", "Jonathan H. McAllister": a full stop after a title or an
    // initial doesn't end the sentence.
    let after_title = before.strip_suffix('.').is_some_and(|b| {
        let last = b.rsplit(|c: char| !c.is_alphabetic()).next().unwrap_or("");
        TITLES.contains(&last.to_lowercase().as_str())
            || (last.chars().count() == 1 && last.chars().all(char::is_uppercase))
    });
    let sentence_start = !after_title
        && (before.is_empty()
            || before.ends_with(['.', '!', '?', ':', '"', '\u{201c}', '(', '\n', '*', '#', '-', '\u{2022}']));
    if !sentence_start {
        return true;
    }
    // A possessive at the start of a sentence is a name: "Dickon's glance".
    // (Harper's word may or may not include the "'s".)
    let rest = &text[start + word.len()..];
    let possessive = |t: &str| t.starts_with("'s") || t.starts_with("\u{2019}s");
    if possessive(rest) || word.ends_with("'s") || word.ends_with("\u{2019}s") {
        return true;
    }
    // Otherwise a name, unless it looks like a slip: one edit from a
    // suggestion ("Recieve", "Occured"), or near a common opening word ("Wich").
    let lower: Vec<char> = word.to_lowercase().chars().collect();
    let slip = |c: &Vec<char>| {
        let c: Vec<char> = c.iter().flat_map(|c| c.to_lowercase()).collect();
        one_edit(&lower, &c) || COMMON_OPENERS.contains(&c.iter().collect::<String>().as_str())
    };
    word.chars().count() >= 3
        && word.chars().skip(1).all(char::is_lowercase)
        && !suggestions.iter().any(|s| matches!(s, Suggestion::ReplaceWith(c) if slip(c)))
}

/// Whether replacing `original` with `suggestion` only uses real words:
/// every word it adds is in the dictionary (or the user's accepted words) and
/// none is on the never-suggest list. Rejects made-up forms like "mous",
/// "tos apply" or "enemie".
pub fn sensible_suggestion(original: &str, suggestion: &str) -> bool {
    if offensive(suggestion) {
        return false;
    }
    let dict = FstDictionary::curated();
    let had: std::collections::HashSet<String> =
        original.split(|c: char| !c.is_alphabetic() && c != '\'').map(str::to_lowercase).collect();
    suggestion.split(|c: char| !c.is_alphabetic() && c != '\'' && c != '\u{2019}').filter(|w| !w.is_empty()).all(|w| {
        let l = w.to_lowercase().replace('\u{2019}', "'");
        had.contains(&l) || dict.contains_word_str(&l) || dict.contains_word_str(w)
    })
}

/// Words never suggested by any rule (slurs, crude words): `rules/lists/never-suggest.txt`.
fn offensive(suggestion: &str) -> bool {
    static WORDS: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    let words = WORDS.get_or_init(|| {
        include_str!("../rules/lists/never-suggest.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_lowercase)
            .collect()
    });
    suggestion.split(|c: char| !c.is_alphabetic() && c != '-').any(|w| words.contains(&w.to_lowercase()))
}

/// A flag that starts or ends inside a word ("Ge[o]rge", "tak[in]g",
/// "f***[ing]"): a tokenising slip, never a real mistake.
fn inside_word(text: &str, range: &Range<usize>) -> bool {
    let wordy = |c: char| c.is_alphanumeric() || c == '*';
    let starts_inside = text[..range.start].chars().next_back().is_some_and(wordy)
        && text[range.clone()].chars().next().is_some_and(|c| c.is_alphanumeric());
    let ends_inside = text[range.end..].chars().next().is_some_and(wordy)
        && text[range.clone()].chars().next_back().is_some_and(|c| c.is_alphanumeric());
    starts_inside || ends_inside
}

/// Whether a spelling suggestion is close to the typed word: at most one
/// change for short words, two for longer ones (letters compared without case).
fn close_spelling(typed: &str, suggestion: &str) -> bool {
    let limit = if typed.chars().count() <= 4 { 1 } else { 2 };
    edit_distance(typed, suggestion) <= limit
}

/// Letters changed, added, removed or swapped to get from `typed` to `suggestion` (ignoring case).
fn edit_distance(typed: &str, suggestion: &str) -> usize {
    let a: Vec<char> = typed.to_lowercase().chars().collect();
    let b: Vec<char> = suggestion.to_lowercase().chars().collect();
    // Levenshtein distance, with an adjacent swap counted as one change.
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Whether `a` and `b` differ by one inserted, deleted, changed or swapped letter.
fn one_edit(a: &[char], b: &[char]) -> bool {
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (sa, sb) = (&a[p..], &b[p..]);
    match (sa.len(), sb.len()) {
        (x, y) if x == y => {
            x == 0 || sa[1..] == sb[1..] || (x >= 2 && sa[0] == sb[1] && sa[1] == sb[0] && sa[2..] == sb[2..])
        }
        (x, y) if x + 1 == y => sa == &sb[1..],
        (x, y) if y + 1 == x => &sa[1..] == sb,
        _ => false,
    }
}

/// Lower-case words that are part of a name when a capitalised word follows.
const NAME_PARTICLES: &[&str] = &[
    "von", "van", "der", "den", "du", "de", "del", "della", "da", "di", "dos", "das", "la", "le", "bin", "ibn", "al",
    "el", "ter", "zu",
];

/// Words that often start a sentence: a capitalised slip near one of these is
/// a typo, not a name.
const COMMON_OPENERS: &[&str] = &[
    "the",
    "this",
    "that",
    "these",
    "those",
    "there",
    "their",
    "they",
    "then",
    "than",
    "which",
    "what",
    "when",
    "where",
    "while",
    "whether",
    "who",
    "why",
    "how",
    "however",
    "because",
    "but",
    "and",
    "also",
    "although",
    "after",
    "before",
    "since",
    "until",
    "unless",
    "if",
    "in",
    "on",
    "at",
    "as",
    "for",
    "from",
    "with",
    "without",
    "about",
    "again",
    "all",
    "any",
    "some",
    "many",
    "most",
    "more",
    "much",
    "each",
    "every",
    "our",
    "your",
    "my",
    "his",
    "her",
    "its",
    "we",
    "you",
    "she",
    "he",
    "it",
    "i",
    "please",
    "thanks",
    "thank",
    "hello",
    "dear",
    "regards",
    "sorry",
    "yes",
    "no",
    "not",
    "now",
    "here",
    "just",
    "still",
    "even",
    "only",
    "maybe",
    "perhaps",
    "probably",
    "actually",
    "basically",
    "finally",
    "first",
    "second",
    "next",
    "last",
    "today",
    "tomorrow",
    "yesterday",
    "unfortunately",
    "fortunately",
    "hopefully",
    "therefore",
    "otherwise",
    "meanwhile",
    "instead",
    "although",
    "though",
    "during",
    "through",
    "should",
    "could",
    "would",
    "will",
    "can",
    "did",
    "does",
    "do",
    "have",
    "has",
    "had",
    "was",
    "were",
    "are",
    "is",
    "let",
    "make",
    "see",
    "note",
    "remember",
    "once",
    "both",
    "either",
    "neither",
    "other",
    "another",
    "such",
    "same",
    "so",
    "too",
    "very",
    "well",
    "good",
    "great",
    "new",
    "one",
    "two",
    "three",
    "every",
    "everyone",
    "everything",
    "someone",
    "something",
    "nothing",
    "never",
    "always",
    "often",
    "sometimes",
    "usually",
    "already",
    "almost",
    "anyway",
    "besides",
    "according",
    "especially",
    "obviously",
    "clearly",
    "certainly",
    "definitely",
    "really",
    "recently",
    "currently",
    // Sign-offs.
    "sincerely",
    "best",
    "kind",
    "warm",
    "warmest",
    "cheers",
    "yours",
    "cordially",
    "respectfully",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn words(c: &mut Checker, text: &str, cat: Category) -> Vec<String> {
        c.check(text).into_iter().filter(|i| i.category == cat).map(|i| text[i.range].to_string()).collect()
    }

    #[test]
    fn dialect_from_locale_names() {
        assert_eq!(Dialect::parse("en_AU.UTF-8"), Some(Dialect::Australian));
        assert_eq!(Dialect::parse("en-GB"), Some(Dialect::British));
        assert_eq!(Dialect::parse("en_NZ.UTF-8"), Some(Dialect::British));
        assert_eq!(Dialect::parse("en_US"), Some(Dialect::American));
        assert_eq!(Dialect::parse("au"), Some(Dialect::Australian));
        assert_eq!(Dialect::parse("de_DE.UTF-8"), None);
    }

    #[test]
    fn informal_spelling_beats_missing_capital() {
        let mut c = Checker::new(Settings::default(), &[]);
        let text = "wot does it feel like?";
        let i = c.check(text);
        assert!(
            i.iter().any(|i| i.range == (0..3) && i.suggestions.first().map(String::as_str) == Some("What")),
            "{i:?}"
        );
        assert_eq!(i.iter().filter(|i| i.range == (0..3)).count(), 1, "{i:?}");
    }

    #[test]
    fn dictionary_words_skip_informal_notes() {
        let p = Profiles::default().get("academic");
        let mut c = Checker::new(Settings { profile: p.clone(), ..Settings::default() }, &[]);
        let text = "We met this arvo.";
        let noted = |c: &mut Checker| {
            c.check(text).iter().any(|i| i.category == Category::Register && &text[i.range.clone()] == "arvo")
        };
        assert!(noted(&mut c));
        let mut c = Checker::new(Settings { profile: p, ..Settings::default() }, &["arvo".to_string()]);
        assert!(!noted(&mut c));
    }

    #[test]
    fn one_issue_per_word_with_sensible_fixes() {
        let mut c = Checker::new(Settings::default(), &[]);
        let text = "We need more informations. The adress is wrong.";
        let issues = c.check(text);
        let on = |w: &str| issues.iter().filter(|i| &text[i.range.clone()] == w).collect::<Vec<_>>();
        let info = on("informations");
        assert_eq!(info.len(), 1, "{info:?}");
        assert_eq!(info[0].suggestions[0], "information");
        assert!(info[0].suggestions.iter().all(|s| s != "in formations" && s != "deformations"), "{info:?}");
        let adress = on("adress");
        assert_eq!(adress.len(), 1);
        assert_eq!(adress[0].suggestions[0], "address");
        assert!(!adress[0].suggestions.contains(&"a dress".to_string()), "{adress:?}");
    }

    #[test]
    fn finds_spelling_and_grammar() {
        let mut c = Checker::new(Settings::default(), &[]);
        let text = "This is an test of the speling checker.";
        let issues = c.check(text);
        let sp = issues.iter().find(|i| &text[i.range.clone()] == "speling").expect("spelling");
        assert_eq!(sp.category, Category::Spelling);
        assert!(sp.suggestions.iter().any(|s| s == "spelling"), "{:?}", sp.suggestions);
        assert!(
            issues.iter().any(|i| i.category == Category::Grammar && text[i.range.clone()].contains("an")),
            "{issues:#?}"
        );
    }

    #[test]
    fn known_false_alarms_are_dropped() {
        // Optional commas ("However,") are right, just not needed: tested in optional_commas_follow_the_profile.
        let p = Profile { optional_commas: false, ..Profile::default() };
        let mut c = Checker::new(Settings { profile: p, ..Settings::default() }, &[]);
        let flags = |c: &mut Checker, t: &str| {
            c.check(t)
                .into_iter()
                .filter(|i| matches!(i.category, Category::Grammar | Category::Spelling))
                .map(|i| i.rule)
                .collect::<Vec<_>>()
        };
        for t in [
            "While he dug the bed, he whistled.",
            "Then she sprang to her feet.",
            "He was certain that Gunch had seen him.",
            "She spoke to every one of them.",
            "He threw himself into a passion.",
            "We came to a crossing in the road.",
        ]
        .iter()
        .chain(PROBE)
        .chain(PROBE2)
        {
            let found = flags(&mut c, t);
            assert!(found.is_empty() || found == ["CompoundNouns"], "{t}: {found:?}");
        }
        // The same rules still catch the real mistakes.
        for (t, rule) in [
            ("Can you give me an advice?", "MassNouns"),
            ("Its raining again today.", "ItsForItIs|ItsContraction"),
            ("It is very quite here at night.", "QuiteQuiet"),
            ("The results of the test was clear.", "NounSubjectAgreement"),
            ("We waited for awhile.", "AWhile"),
            ("They was late again.", "PronounInflectionBe"),
            ("Can you tell me where is the station?", "WordOrder"),
            ("Thanks John for the update.", "AddressComma"),
            ("I love you Mom.", "AddressComma"),
            ("I admire your work Jessica.", "AddressComma"),
            ("Please finalize the budget report John.", "AddressComma"),
            ("I always am late.", "WordOrder"),
            ("Each of the students have a laptop.", "NounSubjectAgreement"),
            ("The results of the test was clear.", "NounSubjectAgreement"),
            ("It is a uninhabited island.", "AnA"),
            ("Who do you think you are.", "QuestionMark"),
            ("It would nice if Harper could detect this.", "ModalBeAdjective"),
            ("There is no course to whom has opened the most PRs.", "WhomSubjectOfVerb"),
            ("My hobby is de extinction of the dinosaurs.", "DisjointPrefixes"),
            ("The team's have submitted their reports.", "NounSubjectAgreement"),
            (
                "The 2012 season was the first year under Kiffin that the Trojans was eligible for post-season play.",
                "NounSubjectAgreement",
            ),
        ] {
            let all: Vec<String> = c.check(t).into_iter().map(|i| i.rule).collect();
            assert!(all.iter().any(|r| rule.split('|').any(|x| x == r)), "{t}: {all:?}");
        }
        // (Capitals are a texting-style note, off in the standard profile.)
        let t = "We went home. it was late.";
        assert!(!known_false_alarm("SentenceCapitalization", t, &(14..16)));
        let t = "The Glassware Co. was upstairs.";
        assert!(known_false_alarm("SentenceCapitalization", t, &(18..21)));
    }

    #[test]
    fn accepted_words_follow_the_dialect() {
        let flagged = |d: Dialect, t: &str| {
            let mut c = Checker::new(Settings { dialect: d, ..Settings::default() }, &[]);
            words(&mut c, t, Category::Spelling)
        };
        assert_eq!(flagged(Dialect::American, "Afterwards he spoke uncertainly."), Vec::<String>::new());
        assert!(flagged(Dialect::Australian, "She leant on the gate and dreamed.").is_empty());
        assert_eq!(flagged(Dialect::American, "She leant on the gate."), ["leant"]);
    }

    #[test]
    fn user_words_are_accepted() {
        let mut c = Checker::new(Settings::default(), &["Parleyvello".to_string()]);
        assert!(words(&mut c, "Parleyvello is fine.", Category::Spelling).is_empty());
        // (Name skipping off: an unknown capitalised word at the start would pass as a name.)
        let mut c = Checker::new(Settings { ignore_names: false, ..Settings::default() }, &[]);
        assert_eq!(words(&mut c, "Parleyvello is fine.", Category::Spelling), ["Parleyvello"]);
    }

    #[test]
    fn names_mid_sentence_are_not_spelling_errors() {
        let text = "I met Okonkwo at McKinsey. Okonkwo was there. Sincerly, me.";
        let mut c = Checker::new(Settings::default(), &[]);
        assert_eq!(words(&mut c, text, Category::Spelling), ["Sincerly"]);
        let mut off = Checker::new(Settings { ignore_names: false, ..Settings::default() }, &[]);
        assert_eq!(words(&mut off, text, Category::Spelling).len(), 4);
        // Slips at the start of a sentence are still typos; compound halves aren't.
        let text = "Becuase it rained. Wich one? Recieve it. Marilla's kind-hearted, good-natured friend came.";
        assert_eq!(words(&mut c, text, Category::Spelling), ["Becuase", "Wich", "Recieve"]);
        // Name particles and "et al." aren't misspellings; on their own they still are.
        let text = "Fond du Lac lies north of Juan del Castillo's farm, as Smith et al. found it, the speling end.";
        assert_eq!(words(&mut c, text, Category::Spelling), ["speling"]);
    }

    #[test]
    fn what_was_left_without_a_passive_note() {
        let mut c = Checker::new(Settings::default(), &[]);
        let t = "I gave him what were left of the day before. They took too much, I said nervously.";
        let found: Vec<(String, String)> = c.check(t).into_iter().map(|i| (i.rule, t[i.range].to_string())).collect();
        assert_eq!(found, [("WhatWasLeft".to_string(), "were".to_string())]);
    }

    #[test]
    fn missing_preposition_is_off_until_turned_on() {
        let t = "The proposal states that its deadline is next Friday.";
        let rules = |settings: Settings| -> Vec<String> {
            Checker::new(settings, &[]).check(t).into_iter().map(|i| i.rule).collect()
        };
        assert!(!rules(Settings::default()).contains(&"MissingPreposition".to_string()));
        let mut on = Settings::default();
        let mut off = on.rules_off();
        off.remove("MissingPreposition");
        on.set_rules_off(off);
        assert!(on.disabled_rules.is_empty());
        assert!(rules(on).contains(&"MissingPreposition".to_string()));
    }

    const PROBE2: &[&str] = &[
        "It is the first co-educational school in Jordan.",
        "Programs were virtually non-existent until 1933.",
        "Nasty! they are going into the church, too.",
        "Ah! at last it would turn no farther.",
        "They are a current member of National League A and play at home.",
        "He is seen as an honorable and a great character.",
        "The Giants were keen that the club not attach itself to a faith.",
        "Lifeboat Associates was a company that acted as a broker.",
        "When the matter was explained he pooh-poohed the panic altogether.",
        "However there is an antivenom for both funnel-web bites.",
        "The family experiences a warmth of feeling as they say goodbye.",
        "Coca-Cola Hellenic was its largest listed company by value.",
        "I believe it will give its best to me in return.",
        "Was it only a coincidence that Mr. Gillingham had turned up just then?",
        "And so well did he face it, that the wolves drew back.",
        "Although he himself would later deny having received it.",
        "Walsh has nine children, the youngest of whom is named Dylan.",
        "To whom did you speak in the dim hallway?",
        "We approved the following actions: (i) the acquisition of shares.",
        "He gave over all attempts at placation.",
        "He saw Spitz run out his tongue in a way he had of laughing.",
        "She couldn't remember enough ways of saying no in English.",
        "They had crept in while Buck and Spitz were fighting.",
        "Jim West Jr. was the heir to the fortune.",
    ];

    const PROBE: &[&str] = &[
        "They could feel the cold reaching out for them with its icy, death-dealing fingers.",
        "His sensible face, with its not unfriendly cynicism, seemed appropriate.",
        "In spite of his invalid back Colin sat up in bed in quite a healthy rage.",
        "Later on he saved John Thornton's life in quite another fashion.",
        "The Esthonia Glassware Co. was on the top floor.",
        "He wanted the thirty or forty per cent. profit which a pioneer deserves.",
        "They are the only payment that Mr. Smith requires.",
        "Remember that if Mr. Brown is all he is reported to be, it is a wonder.",
        "I was afraid it was Mark.",
        "Their houses were so convenient that they had little housework.",
        "For every yard there was a man, working as if a demon were after him.",
        "In each century since the beginning of the world wonderful things have been discovered.",
        "They laughed at his ideas, for they got all they wanted without either.",
        "One of the things they talked of was the visit they were to make to her cottage.",
        "He was determined to show who was master in that house.",
        "The curly grass about us was on fire now.",
        "I studied it awhile, thinking of the time.",
        "They lay and talked softly awhile.",
        "I am so excited over all these prospects that I can scarcely sleep.",
        "May turned to her husband with a smile.",
        "Every word of it brought back some word of his own dead little one.",
        "You know how kind they always are.",
        "There never was a more humble man than George Darling.",
        "She was delighted, as she always was to see any man in her house.",
        // (Not "have you ever read Hamlet?": it reads like "…been to Paris Sam?".)
        "Men like old Hugh respect it.",
        "There were some twenty people in the room.",
        "She cast a glance at the door.",
    ];

    /// Real typos from the corpus, moved to the start of a sentence: how many
    /// would the name rule hide? `cargo test -p xenify-grammar -- --ignored --nocapture typos_at_start`
    #[test]
    #[ignore]
    fn typos_at_start() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/proofing-corpus/indep-clean.jsonl");
        let mut c = Checker::new(Settings::default(), &[]);
        let (mut n, mut hidden) = (0, Vec::new());
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if !v["error_type"].as_str().is_some_and(|t| t.starts_with("spelling")) {
                continue;
            }
            for m in v["mistakes"].as_array().unwrap() {
                let wrong = m["wrong"].as_str().unwrap();
                if wrong.contains(' ') || !wrong.chars().all(char::is_alphabetic) {
                    continue;
                }
                let mut ch = wrong.chars();
                let cap: String = ch.next().unwrap().to_uppercase().chain(ch).collect();
                let text = format!("{cap} was the word.");
                if c.check(&format!("It was {wrong} again.")).iter().all(|i| i.rule != "SpellCheck") {
                    continue; // not flagged mid-sentence either
                }
                n += 1;
                if c.check(&text).iter().all(|i| i.rule != "SpellCheck") {
                    hidden.push(cap);
                }
            }
        }
        println!("{} of {n} typos hidden at the start of a sentence: {hidden:?}", hidden.len());
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let mut c = Checker::new(Settings::default(), &[]);
        assert_eq!(words(&mut c, "Café naïve résumé, and a mispeled word.", Category::Spelling), ["mispeled"]);
    }

    #[test]
    fn optional_commas_follow_the_profile() {
        let rules = |optional: bool| {
            let p = Profile { optional_commas: optional, ..Profile::default() };
            let mut c = Checker::new(Settings { profile: p, ..Settings::default() }, &[]);
            ["Hi Sam\nThe report is attached.", "However we decided to wait."]
                .iter()
                .flat_map(|t| c.check(t))
                .map(|i| i.rule)
                .collect::<Vec<_>>()
        };
        let strict = rules(true);
        assert!(
            strict.contains(&"GreetingComma".to_string()) && strict.contains(&"DiscourseMarkers".to_string()),
            "{strict:?}"
        );
        let relaxed = rules(false);
        assert!(!relaxed.iter().any(|r| is_optional_punctuation(r)), "{relaxed:?}");
    }

    #[test]
    fn whole_document_checks_report_paragraph_indexes() {
        let p = Profile { define_acronyms: true, ..Profile::default() };
        let mut c = Checker::new(Settings { profile: p, ..Settings::default() }, &[]);
        let paras = vec!["Hello.".to_string(), "Our TCO dropped.".to_string()];
        assert!(c.check_paragraphs(&paras).iter().any(|(i, x)| *i == 1 && x.rule == "UndefinedAcronym"));
    }
}
