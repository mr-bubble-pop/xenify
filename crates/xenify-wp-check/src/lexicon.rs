//! Dictionary and thesaurus: Open English WordNet (CC BY 4.0, derived from
//! Princeton WordNet), packed by `scripts/fetch-oewn.sh` into
//! `assets/lexicon/oewn.txt.z` and compiled in. Looks words up by their base
//! form ("happier" → "happy", "went" → "go") and puts replacements back into
//! the clicked word's form ("happier" → "gladder", "Ran" → "Sprinted").
//! Everything is local.

use std::collections::HashMap;
use std::sync::OnceLock;

const PACKED: &[u8] = include_bytes!("../../../assets/lexicon/oewn.txt.z");

/// The credit the licence asks for, shown wherever the dictionary is.
pub const ATTRIBUTION: &str = "Open English WordNet (CC BY 4.0), based on Princeton WordNet";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartOfSpeech {
    Noun,
    Verb,
    Adjective,
    Adverb,
}

impl PartOfSpeech {
    fn from_code(c: &str) -> Option<Self> {
        match c {
            "n" => Some(Self::Noun),
            "v" => Some(Self::Verb),
            "a" | "s" => Some(Self::Adjective),
            "r" => Some(Self::Adverb),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Noun => "noun",
            Self::Verb => "verb",
            Self::Adjective => "adjective",
            Self::Adverb => "adverb",
        }
    }
}

/// How a word was inflected from its base form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inflection {
    Base,
    /// Noun plural or verb third person ("-s").
    S,
    Past,
    Ing,
    Comparative,
    Superlative,
    /// An irregular form whose kind isn't recorded ("went"); replacements
    /// are offered in their base form.
    Irregular,
}

struct Synset {
    words: Vec<String>,
    definition: String,
    examples: Vec<String>,
    broader: Vec<u32>,
    similar: Vec<u32>,
}

struct Entry {
    word: String,
    pos: PartOfSpeech,
    pronunciation: String,
    /// Synset numbers, most common meaning first, each with its antonyms.
    senses: Vec<(u32, Vec<String>)>,
    forms: Vec<String>,
}

/// The loaded dictionary.
pub struct Lexicon {
    synsets: Vec<Synset>,
    narrower: Vec<Vec<u32>>,
    entries: Vec<Entry>,
    /// Lower-cased word → its entries (one per part of speech).
    by_word: HashMap<String, Vec<u32>>,
    /// Lower-cased irregular form → (entry, …).
    by_form: HashMap<String, Vec<u32>>,
}

/// One meaning of a word looked up.
#[derive(Clone, Debug, PartialEq)]
pub struct Meaning {
    pub pos: PartOfSpeech,
    pub definition: String,
    pub examples: Vec<String>,
    /// Other words for this meaning (the word itself left out).
    pub synonyms: Vec<String>,
    pub antonyms: Vec<String>,
    /// Closely related meanings' words (adjectives): "glad", "joyful"…
    pub similar: Vec<String>,
    /// "A kind of …".
    pub broader: Vec<String>,
    /// Kinds of it (a few).
    pub narrower: Vec<String>,
}

/// A word looked up: each base form found, with its meanings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lookup {
    pub entries: Vec<Found>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// The dictionary form ("happy" for "happier").
    pub word: String,
    pub pos: PartOfSpeech,
    pub pronunciation: String,
    pub inflection: Inflection,
    pub meanings: Vec<Meaning>,
}

impl Lookup {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

static LEXICON: OnceLock<Lexicon> = OnceLock::new();

/// The dictionary, loading it on first use (a fraction of a second; call
/// [`preload`] at start-up to do it in the background).
pub fn lexicon() -> &'static Lexicon {
    LEXICON.get_or_init(|| Lexicon::parse(&unpack()).expect("bundled dictionary is well-formed"))
}

/// Loads the dictionary on a background thread.
pub fn preload() {
    if LEXICON.get().is_none() {
        std::thread::spawn(|| {
            lexicon();
        });
    }
}

fn unpack() -> String {
    let bytes = miniz_oxide::inflate::decompress_to_vec_zlib(PACKED).expect("bundled dictionary decompresses");
    String::from_utf8(bytes).expect("bundled dictionary is UTF-8")
}

fn split(s: &str, sep: char) -> impl Iterator<Item = &str> {
    s.split(sep).filter(|x| !x.is_empty())
}

fn numbers(s: &str) -> Vec<u32> {
    split(s, ',').filter_map(|n| n.parse().ok()).collect()
}

impl Lexicon {
    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        lines.next().filter(|l| l.starts_with("OEWN "))?;
        let n: usize = lines.next()?.strip_prefix("S ")?.parse().ok()?;
        let mut synsets = Vec::with_capacity(n);
        for line in lines.by_ref().take(n) {
            let f: Vec<&str> = line.split('\t').collect();
            let [pos, words, definition, examples, broader, similar] = f[..] else { return None };
            PartOfSpeech::from_code(pos)?;
            synsets.push(Synset {
                words: split(words, '|').map(str::to_string).collect(),
                definition: definition.to_string(),
                examples: split(examples, '|').map(str::to_string).collect(),
                broader: numbers(broader),
                similar: numbers(similar),
            });
        }
        let mut narrower = vec![Vec::new(); synsets.len()];
        for (i, s) in synsets.iter().enumerate() {
            for &b in &s.broader {
                narrower.get_mut(b as usize)?.push(i as u32);
            }
        }
        let n: usize = lines.next()?.strip_prefix("E ")?.parse().ok()?;
        let mut entries = Vec::with_capacity(n);
        let mut by_word: HashMap<String, Vec<u32>> = HashMap::with_capacity(n);
        let mut by_form: HashMap<String, Vec<u32>> = HashMap::new();
        for line in lines.take(n) {
            let f: Vec<&str> = line.split('\t').collect();
            let [word, pos, pronunciation, senses, forms] = f[..] else { return None };
            let senses = split(senses, ',')
                .filter_map(|s| {
                    let (num, ants) = s.split_once('=').unwrap_or((s, ""));
                    Some((num.parse().ok()?, split(ants, '/').map(str::to_string).collect()))
                })
                .collect();
            let i = entries.len() as u32;
            let forms: Vec<String> = split(forms, '|').map(str::to_string).collect();
            for form in &forms {
                by_form.entry(form.to_lowercase()).or_default().push(i);
            }
            by_word.entry(word.to_lowercase()).or_default().push(i);
            entries.push(Entry {
                word: word.to_string(),
                pos: PartOfSpeech::from_code(pos)?,
                pronunciation: pronunciation.to_string(),
                senses,
                forms,
            });
        }
        Some(Lexicon { synsets, narrower, entries, by_word, by_form })
    }

    /// Number of meanings and of words.
    pub fn size(&self) -> (usize, usize) {
        (self.synsets.len(), self.entries.len())
    }

    /// True when `word` (in any form) is in the dictionary.
    pub fn knows(&self, word: &str) -> bool {
        !self.base_forms(word).is_empty()
    }

    /// The entries `word` can be a form of, with how it's inflected; the
    /// word as written first.
    fn base_forms(&self, word: &str) -> Vec<(u32, Inflection)> {
        let word = normalise(word);
        let lower = word.to_lowercase();
        let mut out: Vec<(u32, Inflection)> = Vec::new();
        let add = |out: &mut Vec<(u32, Inflection)>, e: u32, how| {
            if !out.iter().any(|(x, _)| *x == e) {
                out.push((e, how));
            }
        };
        for &e in self.by_word.get(&lower).into_iter().flatten() {
            add(&mut out, e, Inflection::Base);
        }
        for &e in self.by_form.get(&lower).into_iter().flatten() {
            let how = irregular_kind(&lower, self.entries[e as usize].pos);
            add(&mut out, e, how);
        }
        for (pos, how, base) in candidates(&lower) {
            for &e in self.by_word.get(&base).into_iter().flatten() {
                if self.entries[e as usize].pos == pos {
                    add(&mut out, e, how);
                }
            }
        }
        out
    }

    /// Everything about `word`: definitions, examples, synonyms, antonyms,
    /// broader and narrower terms, per part of speech.
    pub fn lookup(&self, word: &str) -> Lookup {
        let entries = self
            .base_forms(word)
            .into_iter()
            .map(|(e, inflection)| {
                let entry = &self.entries[e as usize];
                let key = entry.word.to_lowercase();
                let words_of = |ids: &[u32], limit: usize| -> Vec<String> {
                    let mut out: Vec<String> = Vec::new();
                    for &i in ids {
                        for w in &self.synsets[i as usize].words {
                            if w.to_lowercase() != key && !out.contains(w) {
                                out.push(w.clone());
                            }
                        }
                        if out.len() >= limit {
                            break;
                        }
                    }
                    out.truncate(limit);
                    out
                };
                let meanings = entry
                    .senses
                    .iter()
                    .map(|(s, antonyms)| {
                        let syn = &self.synsets[*s as usize];
                        Meaning {
                            pos: entry.pos,
                            definition: syn.definition.clone(),
                            examples: syn.examples.clone(),
                            synonyms: syn.words.iter().filter(|w| w.to_lowercase() != key).cloned().collect(),
                            antonyms: antonyms.clone(),
                            similar: words_of(&syn.similar, 12),
                            broader: words_of(&syn.broader, 6),
                            narrower: words_of(&self.narrower[*s as usize], 12),
                        }
                    })
                    .collect();
                Found {
                    word: entry.word.clone(),
                    pos: entry.pos,
                    pronunciation: entry.pronunciation.clone(),
                    inflection,
                    meanings,
                }
            })
            .collect();
        Lookup { entries }
    }

    /// Replacements for `word` as written, grouped by meaning (most common
    /// first): each a short definition and the words, already in the
    /// clicked word's form and capitalisation.
    pub fn synonyms(&self, word: &str) -> Vec<(PartOfSpeech, String, Vec<String>)> {
        let mut out = Vec::new();
        for found in self.lookup(word).entries {
            for m in found.meanings {
                // WordNet has few true synonyms for verbs: add a few narrower
                // ones ("walk" → "stroll", "march").
                let extra = if found.pos == PartOfSpeech::Verb { 6usize.saturating_sub(m.synonyms.len()) } else { 0 };
                let mut words: Vec<String> = Vec::new();
                for w in m.synonyms.iter().chain(&m.similar).chain(m.narrower.iter().take(extra)) {
                    if !words.contains(w) {
                        words.push(w.clone());
                    }
                }
                if words.is_empty() {
                    continue;
                }
                let words = words
                    .iter()
                    .map(|w| match_case(word.trim(), &self.inflect(w, found.pos, found.inflection)))
                    .collect();
                out.push((found.pos, m.definition.clone(), words));
            }
        }
        out
    }

    /// `base` put into the form `how` (as a `pos`), using its irregular
    /// forms where it has them.
    pub fn inflect(&self, base: &str, pos: PartOfSpeech, how: Inflection) -> String {
        if matches!(how, Inflection::Base | Inflection::Irregular) {
            return base.to_string();
        }
        // Phrases: verbs inflect their first word ("look into" → "looked
        // into"), nouns and adjectives their last.
        if let Some((first, rest)) = base.split_once(' ') {
            return match pos {
                PartOfSpeech::Verb => format!("{} {rest}", self.inflect(first, pos, how)),
                PartOfSpeech::Adjective if matches!(how, Inflection::Comparative) => format!("more {base}"),
                PartOfSpeech::Adjective if matches!(how, Inflection::Superlative) => format!("most {base}"),
                _ => {
                    let (head, last) = base.rsplit_once(' ').unwrap_or(("", base));
                    format!("{head} {}", self.inflect(last, pos, how))
                }
            };
        }
        let irregular: Vec<&str> = self
            .by_word
            .get(&base.to_lowercase())
            .into_iter()
            .flatten()
            .map(|&e| &self.entries[e as usize])
            .filter(|e| e.pos == pos)
            .flat_map(|e| e.forms.iter().map(String::as_str))
            .collect();
        if let Some(f) = irregular.iter().find(|f| irregular_kind(f, pos) == how) {
            return f.to_string();
        }
        // An irregular verb's past: one form ("bought") is both; two
        // ("went", "gone") can't be told apart, so leave the base form.
        if pos == PartOfSpeech::Verb && how == Inflection::Past {
            let past: Vec<&&str> =
                irregular.iter().filter(|f| irregular_kind(f, pos) == Inflection::Irregular).collect();
            match past[..] {
                [] => {}
                [one] => return one.to_string(),
                _ => return base.to_string(),
            }
        }
        regular(base, pos, how)
    }
}

/// Strips possessives and surrounding punctuation; curly apostrophes become
/// straight.
fn normalise(word: &str) -> String {
    let w = word.trim().replace('\u{2019}', "'");
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    let w = w.strip_suffix("'s").unwrap_or(w);
    w.to_string()
}

/// What an irregular form is, from its ending (best guess).
fn irregular_kind(form: &str, pos: PartOfSpeech) -> Inflection {
    match pos {
        PartOfSpeech::Adjective | PartOfSpeech::Adverb
            if form.ends_with("est") || form == "best" || form == "worst" =>
        {
            Inflection::Superlative
        }
        PartOfSpeech::Adjective | PartOfSpeech::Adverb
            if form.ends_with("er") || matches!(form, "better" | "worse" | "more" | "less") =>
        {
            Inflection::Comparative
        }
        PartOfSpeech::Noun => Inflection::S,
        PartOfSpeech::Verb if form.ends_with("ing") => Inflection::Ing,
        PartOfSpeech::Verb if form.ends_with("ed") => Inflection::Past,
        PartOfSpeech::Verb if form.ends_with('s') => Inflection::S,
        _ => Inflection::Irregular,
    }
}

/// Base forms a regular inflection could come from (WordNet's "morphy"
/// rules, plus undoubling: "stopped" → "stop").
fn candidates(w: &str) -> Vec<(PartOfSpeech, Inflection, String)> {
    use Inflection::*;
    use PartOfSpeech::*;
    const RULES: &[(PartOfSpeech, Inflection, &str, &str)] = &[
        (Noun, S, "s", ""),
        (Noun, S, "ses", "s"),
        (Noun, S, "xes", "x"),
        (Noun, S, "zes", "z"),
        (Noun, S, "ches", "ch"),
        (Noun, S, "shes", "sh"),
        (Noun, S, "men", "man"),
        (Noun, S, "ies", "y"),
        (Verb, S, "s", ""),
        (Verb, S, "ies", "y"),
        (Verb, S, "es", "e"),
        (Verb, S, "es", ""),
        (Verb, Past, "ed", "e"),
        (Verb, Past, "ed", ""),
        (Verb, Past, "ied", "y"),
        (Verb, Ing, "ing", "e"),
        (Verb, Ing, "ing", ""),
        (Verb, Ing, "ying", "ie"),
        (Adjective, Comparative, "er", ""),
        (Adjective, Comparative, "er", "e"),
        (Adjective, Comparative, "ier", "y"),
        (Adjective, Superlative, "est", ""),
        (Adjective, Superlative, "est", "e"),
        (Adjective, Superlative, "iest", "y"),
    ];
    let mut out = Vec::new();
    for &(pos, how, suffix, ending) in RULES {
        if let Some(stem) = w.strip_suffix(suffix).filter(|s| s.len() >= 2) {
            out.push((pos, how, format!("{stem}{ending}")));
            // "stopped", "running", "bigger": undouble the last consonant.
            let b = stem.as_bytes();
            if ending.is_empty() && b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2] && !is_vowel(b[b.len() - 1]) {
                out.push((pos, how, stem[..stem.len() - 1].to_string()));
            }
        }
    }
    out
}

fn is_vowel(b: u8) -> bool {
    matches!(b, b'a' | b'e' | b'i' | b'o' | b'u')
}

/// Short words ending consonant–vowel–consonant double the consonant
/// ("stop" → "stopped"); longer ones usually don't ("visit" → "visited").
fn doubles(w: &str) -> bool {
    let b = w.as_bytes();
    let n = b.len();
    let groups = b.windows(2).filter(|p| !is_vowel(p[0]) && is_vowel(p[1])).count() + usize::from(is_vowel(b[0]));
    n >= 3
        && groups == 1
        && !is_vowel(b[n - 1])
        && is_vowel(b[n - 2])
        && !is_vowel(b[n - 3])
        && !matches!(b[n - 1], b'w' | b'x' | b'y')
}

fn regular(w: &str, pos: PartOfSpeech, how: Inflection) -> String {
    let lower = w.to_lowercase();
    let b = lower.as_bytes();
    if b.is_empty() || !lower.is_ascii() {
        return w.to_string();
    }
    let last = b[b.len() - 1];
    let consonant_y = last == b'y' && b.len() >= 2 && !is_vowel(b[b.len() - 2]);
    let stem = &w[..w.len() - 1];
    match how {
        Inflection::S => {
            if consonant_y {
                format!("{stem}ies")
            } else if ["s", "x", "z", "ch", "sh"].iter().any(|e| lower.ends_with(e)) {
                format!("{w}es")
            } else {
                format!("{w}s")
            }
        }
        Inflection::Past => {
            if last == b'e' {
                format!("{w}d")
            } else if consonant_y {
                format!("{stem}ied")
            } else if doubles(&lower) {
                format!("{w}{}ed", last as char)
            } else {
                format!("{w}ed")
            }
        }
        Inflection::Ing => {
            if lower.ends_with("ie") {
                format!("{}ying", &w[..w.len() - 2])
            } else if last == b'e' && !lower.ends_with("ee") && b.len() > 2 {
                format!("{stem}ing")
            } else if doubles(&lower) {
                format!("{w}{}ing", last as char)
            } else {
                format!("{w}ing")
            }
        }
        Inflection::Comparative | Inflection::Superlative => {
            let (suffix, word) = if how == Inflection::Comparative { ("er", "more") } else { ("est", "most") };
            let syllables =
                b.windows(2).filter(|p| !is_vowel(p[0]) && is_vowel(p[1])).count() + usize::from(is_vowel(b[0]));
            if pos != PartOfSpeech::Adjective || syllables > 2 || (syllables == 2 && !consonant_y) {
                format!("{word} {w}")
            } else if consonant_y {
                format!("{stem}i{suffix}")
            } else if last == b'e' {
                format!("{w}{}", &suffix[1..])
            } else if doubles(&lower) {
                format!("{w}{}{suffix}", last as char)
            } else {
                format!("{w}{suffix}")
            }
        }
        Inflection::Base | Inflection::Irregular => w.to_string(),
    }
}

/// `replacement` capitalised like `original`: "Happy" → "Glad", "HAPPY" →
/// "GLAD"; proper nouns in the dictionary keep their own capitals.
pub fn match_case(original: &str, replacement: &str) -> String {
    let letters: Vec<char> = original.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return replacement.to_uppercase();
    }
    if letters.first().is_some_and(|c| c.is_uppercase()) {
        let mut c = replacement.chars();
        return match c.next() {
            Some(f) => f.to_uppercase().chain(c).collect(),
            None => String::new(),
        };
    }
    replacement.to_string()
}

/// The word around byte `offset` in `text`: letters, digits, and
/// apostrophes or hyphens inside a word.
pub fn word_at(text: &str, offset: usize) -> Option<std::ops::Range<usize>> {
    let is_word = |c: char| c.is_alphanumeric();
    let joins = |c: char| matches!(c, '\'' | '\u{2019}' | '-');
    let offset = offset.min(text.len());
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = chars.iter().position(|(i, _)| *i >= offset).unwrap_or(chars.len());
    // The character under the caret, or the one before it.
    let mut i = if chars.get(at).is_some_and(|(_, c)| is_word(*c)) {
        at
    } else if at > 0 && is_word(chars[at - 1].1) {
        at - 1
    } else {
        return None;
    };
    let mut j = i;
    while i > 0 && (is_word(chars[i - 1].1) || (joins(chars[i - 1].1) && i >= 2 && is_word(chars[i - 2].1))) {
        i -= 1;
    }
    while j + 1 < chars.len()
        && (is_word(chars[j + 1].1) || (joins(chars[j + 1].1) && chars.get(j + 2).is_some_and(|(_, c)| is_word(*c))))
    {
        j += 1;
    }
    let start = chars[i].0;
    let end = chars[j].0 + chars[j].1.len_utf8();
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_and_inflects() {
        let lx = lexicon();
        let (synsets, words) = lx.size();
        assert!(synsets > 100_000 && words > 100_000);

        let happy = lx.lookup("happy");
        let adj = &happy.entries[0];
        assert_eq!((adj.word.as_str(), adj.pos), ("happy", PartOfSpeech::Adjective));
        assert!(adj.meanings[0].definition.contains("pleasure"));
        assert!(adj.meanings[0].antonyms.contains(&"unhappy".to_string()));

        // Inflected words find their base and replacements come back inflected.
        let found = lx.lookup("happier");
        assert_eq!(found.entries[0].word, "happy");
        assert_eq!(found.entries[0].inflection, Inflection::Comparative);
        assert_eq!(lx.lookup("went").entries.iter().find(|f| f.pos == PartOfSpeech::Verb).unwrap().word, "go");
        assert!(lx.lookup("stopped").entries.iter().any(|f| f.word == "stop" && f.inflection == Inflection::Past));
        assert_eq!(lx.lookup("children").entries[0].word, "child");

        let all = |w: &str| lx.synonyms(w).into_iter().flat_map(|(_, _, w)| w).collect::<Vec<_>>();
        assert!(all("Quick").contains(&"Fast".to_string()), "{:?}", all("Quick"));
        assert!(all("cars").contains(&"automobiles".to_string()), "{:?}", all("cars"));
        assert!(all("purchased").contains(&"bought".to_string()), "{:?}", all("purchased"));
        assert!(all("walking").contains(&"marching".to_string()), "{:?}", all("walking"));
        assert!(lx.lookup("xyzzyq").is_empty());
    }

    #[test]
    fn regular_forms() {
        use Inflection::*;
        use PartOfSpeech::*;
        assert_eq!(regular("box", Noun, S), "boxes");
        assert_eq!(regular("city", Noun, S), "cities");
        assert_eq!(regular("stop", Verb, Past), "stopped");
        assert_eq!(regular("visit", Verb, Past), "visited");
        assert_eq!(regular("hope", Verb, Ing), "hoping");
        assert_eq!(regular("die", Verb, Ing), "dying");
        assert_eq!(regular("glad", Adjective, Comparative), "gladder");
        assert_eq!(regular("joyful", Adjective, Comparative), "more joyful");
        assert_eq!(regular("lucky", Adjective, Superlative), "luckiest");
        assert_eq!(match_case("HAPPY", "glad"), "GLAD");
        assert_eq!(match_case("Happy", "glad"), "Glad");
    }

    #[test]
    fn words_in_text() {
        let t = "It's a well-known fact, isn't it?";
        assert_eq!(&t[word_at(t, 1).unwrap()], "It's");
        assert_eq!(&t[word_at(t, 9).unwrap()], "well-known");
        assert_eq!(&t[word_at(t, 22).unwrap()], "fact");
        assert_eq!(&t[word_at(t, 22).unwrap()], "fact");
        assert_eq!(word_at(t, 23), None);
        assert_eq!(word_at("  ", 1), None);
    }
}
