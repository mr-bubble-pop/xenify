//! The sentence breaker: damages correct sentences in known ways, so the
//! mistake and its fix are known without asking a model.
//!
//! Sources are the corrected passages and clean standard passages already in
//! the corpus (generated text, CC0), so no outside text is needed.

use crate::{Item, Mistake, append_items, read_items};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

/// Small seeded generator (xorshift64*), so runs are repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Byte ranges of the words (letters, digits, apostrophes, inner hyphens).
fn words(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let wordy = c.is_alphanumeric() || c == '\'' || c == '’' || (c == '-' && start.is_some());
        match (wordy, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

const PREPOSITIONS: &[&str] = &["in", "on", "at", "to", "from", "with", "of", "for", "by", "into", "under", "over"];

/// (base, past, past participle) for irregular verbs whose forms differ.
const IRREGULAR: &[(&str, &str, &str)] = &[
    ("go", "went", "gone"),
    ("see", "saw", "seen"),
    ("do", "did", "done"),
    ("take", "took", "taken"),
    ("give", "gave", "given"),
    ("write", "wrote", "written"),
    ("eat", "ate", "eaten"),
    ("begin", "began", "begun"),
    ("know", "knew", "known"),
    ("speak", "spoke", "spoken"),
    ("choose", "chose", "chosen"),
    ("break", "broke", "broken"),
    ("drive", "drove", "driven"),
    ("forget", "forgot", "forgotten"),
    ("ring", "rang", "rung"),
    ("swim", "swam", "swum"),
    ("sing", "sang", "sung"),
    ("drink", "drank", "drunk"),
    ("fly", "flew", "flown"),
    ("grow", "grew", "grown"),
    ("throw", "threw", "thrown"),
    ("draw", "drew", "drawn"),
    ("show", "showed", "shown"),
    ("fall", "fell", "fallen"),
    ("hide", "hid", "hidden"),
    ("ride", "rode", "ridden"),
    ("rise", "rose", "risen"),
    ("steal", "stole", "stolen"),
    ("wake", "woke", "woken"),
    ("wear", "wore", "worn"),
    ("run", "ran", "run"),
    ("come", "came", "come"),
    ("become", "became", "become"),
    ("send", "sent", "sent"),
    ("buy", "bought", "bought"),
    ("bring", "brought", "brought"),
    ("think", "thought", "thought"),
    ("make", "made", "made"),
    ("find", "found", "found"),
    ("tell", "told", "told"),
    ("leave", "left", "left"),
    ("meet", "met", "met"),
    ("pay", "paid", "paid"),
    ("keep", "kept", "kept"),
    ("hold", "held", "held"),
    ("build", "built", "built"),
    ("spend", "spent", "spent"),
    ("feel", "felt", "felt"),
    ("lose", "lost", "lost"),
];

/// Same-sentence verb swaps that break agreement.
const AGREEMENT: &[(&str, &str)] = &[
    ("is", "are"),
    ("are", "is"),
    ("was", "were"),
    ("were", "was"),
    ("has", "have"),
    ("have", "has"),
    ("does", "do"),
    ("doesn't", "don't"),
    ("don't", "doesn't"),
];

const SUBJECT_PRONOUNS: &[&str] = &["i", "you", "he", "she", "it", "we", "they"];

/// One way of breaking a sentence. Returns (new text, byte range of the
/// damage in the new text, byte range of the same words in the original).
type Breaker = fn(&str, &[Range<usize>], &mut Rng) -> Option<(String, Range<usize>, Range<usize>)>;

/// The breakages, named by the standard grammatical-error checklist (ERRANT
/// codes: M = missing, U = unnecessary, R = replaced, WO = word order), so the
/// scoreboard doubles as a coverage chart.
const KINDS: &[(&str, Breaker)] = &[
    ("M:DET missing a/the", missing_article),
    ("U:DET extra a/an", extra_article),
    ("R:DET a/an swapped", a_an_swap),
    ("M:PREP missing preposition", missing_preposition),
    ("U:PREP extra preposition", extra_preposition),
    ("R:PREP wrong preposition", wrong_preposition),
    ("M:PRON missing it/there", missing_dummy_subject),
    ("R:PRON wrong pronoun case", pronoun_case),
    ("M:VERB missing is/are", missing_auxiliary),
    ("M:VERB missing do", missing_do),
    ("U:VERB extra be", extra_be),
    ("M:PART missing to", missing_to),
    ("U:PART extra to", extra_to),
    ("R:VERB:SVA agreement", agreement_swap),
    ("R:VERB:SVA verb -s", verb_s),
    ("R:VERB:SVA what were left", what_were),
    ("R:VERB:FORM wrong verb form", wrong_verb_form),
    ("R:VERB:FORM be + base verb", be_base),
    ("R:VERB:TENSE tense shift", tense_shift),
    ("R:VERB:INFL made-up past", made_up_past),
    ("R:NOUN:NUM singular/plural", noun_number),
    ("R:NOUN:INFL made-up plural", made_up_plural),
    ("R:ADJ:FORM double comparative", double_comparative),
    ("R:ADV adjective for adverb", adjective_for_adverb),
    ("R:ORTH sound-alike word", sound_alike),
    ("U:PUNCT unneeded hyphen", extra_hyphen),
    ("M:PUNCT missing hyphen", missing_hyphen),
    ("GARBLED leftover words", leftover_words),
    ("GARBLED dropped verb", dropped_verb),
    ("GARBLED jumbled words", jumbled_words),
    ("GARBLED cut-off ending", cut_off_ending),
    ("WO swapped words", swapped_words),
    ("WO question word order", question_order),
    ("U:OTHER repeated word", repeated_word),
];

fn lower(text: &str, r: &Range<usize>) -> String {
    text[r.clone()].to_lowercase()
}

/// Picks one candidate word index at random.
fn pick(cands: Vec<usize>, rng: &mut Rng) -> Option<usize> {
    if cands.is_empty() { None } else { Some(cands[rng.below(cands.len())]) }
}

/// Deletes word `i` (and one neighbouring space). The damage is reported as
/// the words either side of the gap.
fn delete_word(text: &str, w: &[Range<usize>], i: usize) -> Option<(String, Range<usize>, Range<usize>)> {
    if i == 0 || i + 1 >= w.len() {
        return None;
    }
    let (prev, next) = (&w[i - 1], &w[i + 1]);
    // Only plain "a b c" runs: no punctuation between the three words.
    if text[prev.end..w[i].start] != *" " || text[w[i].end..next.start] != *" " {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..w[i].start]);
    out.push_str(&text[next.start..]);
    let shift = next.start - w[i].start;
    Some((out, prev.start..next.end - shift, prev.start..next.end))
}

/// Replaces word `i` with `new`, keeping a leading capital.
fn replace_word(text: &str, w: &[Range<usize>], i: usize, new: &str) -> (String, Range<usize>, Range<usize>) {
    let old = &text[w[i].clone()];
    let new = if old.starts_with(char::is_uppercase) {
        let mut c = new.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    } else {
        new.to_string()
    };
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..w[i].start]);
    out.push_str(&new);
    out.push_str(&text[w[i].end..]);
    (out, w[i].start..w[i].start + new.len(), w[i].clone())
}

/// Nouns used without an article after a preposition: "go to school", "from
/// church", "at table", "by train", "in bed".
const ZERO_ARTICLE: &[&str] = &[
    "school",
    "church",
    "college",
    "university",
    "bed",
    "table",
    "home",
    "work",
    "town",
    "hospital",
    "prison",
    "jail",
    "court",
    "sea",
    "class",
    "breakfast",
    "lunch",
    "dinner",
    "supper",
    "tea",
    "night",
    "noon",
    "midnight",
    "dawn",
    "dusk",
    "sunset",
    "sunrise",
    "train",
    "car",
    "bus",
    "foot",
    "air",
    "boat",
    "ship",
    "land",
    "hand",
    "heart",
    "office",
    "market",
    "chapel",
    "mass",
    "service",
    "camp",
    "bay",
    "board",
    "deck",
    "shore",
    "time",
    "length",
    "doors",
    "fire",
    "sight",
    "view",
    "order",
    "use",
    "fact",
    "general",
    "particular",
    "part",
    "course",
    "chance",
    "turn",
    "place",
    "case",
    "line",
    "touch",
    "charge",
    "danger",
    "trouble",
    "debt",
    "love",
    "war",
    "peace",
    "silence",
    "secret",
    "public",
    "private",
    "person",
    "mind",
    "question",
    "earth",
];

/// "in the sun" → "in sun"; "a report" → "report".
fn missing_article(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| {
            let (word, prev, next) = (lower(text, &w[i]), lower(text, &w[i - 1]), lower(text, &w[i + 1]));
            // Not before a name ("the Thames" → "Thames" is often fine), or a noun
            // that is normal without an article ("from church", "at table", "in bed").
            if text[w[i + 1].clone()].starts_with(char::is_uppercase) || ZERO_ARTICLE.contains(&next.as_str()) {
                return false;
            }
            match word.as_str() {
                "a" | "an" => !matches!(next.as_str(), "few" | "little" | "lot" | "number"),
                // "the" only after a preposition and before a singular-looking noun,
                // where English needs some determiner.
                "the" => PREPOSITIONS.contains(&prev.as_str()) && !next.ends_with('s') && next.len() > 2,
                _ => false,
            }
        })
        .collect();
    delete_word(text, w, pick(cands, rng)?)
}

/// "She is running late" → "She running late".
fn missing_auxiliary(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| {
            matches!(lower(text, &w[i]).as_str(), "is" | "are" | "am" | "was" | "were")
                && lower(text, &w[i + 1]).ends_with("ing")
                && w[i + 1].len() > 5
        })
        .collect();
    delete_word(text, w, pick(cands, rng)?)
}

/// "We do not agree" / "We don't agree" → "We not agree".
fn missing_do(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| {
            let word = lower(text, &w[i]).replace('’', "'");
            (matches!(word.as_str(), "do" | "does" | "did") && lower(text, &w[i + 1]) == "not")
                || matches!(word.as_str(), "don't" | "doesn't" | "didn't")
        })
        .collect();
    let i = pick(cands, rng)?;
    if lower(text, &w[i]).starts_with("d") && lower(text, &w[i]).len() <= 4 {
        return delete_word(text, w, i);
    }
    let (t, d, o) = replace_word(text, w, i, "not");
    Some((t, w[i - 1].start..d.end, w[i - 1].start..o.end))
}

/// "We need to check" → "We need check".
fn missing_to(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    const BEFORE: &[&str] = &[
        "need", "needs", "needed", "want", "wants", "wanted", "plan", "plans", "planned", "going", "trying", "able",
        "have", "has", "had", "hope", "decided", "agreed",
    ];
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| lower(text, &w[i]) == "to" && BEFORE.contains(&lower(text, &w[i - 1]).as_str()))
        .collect();
    delete_word(text, w, pick(cands, rng)?)
}

/// "listen to the radio" → "listen the radio"; "one of the" → "one the".
fn missing_preposition(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    // Only where the preposition is required: dropping "of" from "smelled of
    // the sea" leaves a correct sentence, so free verbs are not used.
    const REQUIRED: &[(&str, &str)] = &[
        ("listen", "to"),
        ("listened", "to"),
        ("listening", "to"),
        ("belong", "to"),
        ("belongs", "to"),
        ("belonged", "to"),
        ("refer", "to"),
        ("referred", "to"),
        ("according", "to"),
        ("due", "to"),
        ("thanks", "to"),
        ("depend", "on"),
        ("depends", "on"),
        ("depended", "on"),
        ("rely", "on"),
        ("relied", "on"),
        ("focus", "on"),
        ("focused", "on"),
        ("insist", "on"),
        ("insisted", "on"),
        ("wait", "for"),
        ("waited", "for"),
        ("waiting", "for"),
        ("look", "for"),
        ("looking", "for"),
        ("looked", "for"),
        ("apply", "for"),
        ("applied", "for"),
        ("pay", "for"),
        ("paid", "for"),
        ("search", "for"),
        ("searched", "for"),
        ("ask", "for"),
        ("asked", "for"),
        ("look", "at"),
        ("looked", "at"),
        ("looking", "at"),
        ("laugh", "at"),
        ("laughed", "at"),
        ("stare", "at"),
        ("stared", "at"),
        ("arrive", "at"),
        ("arrived", "at"),
        ("one", "of"),
        ("some", "of"),
        ("most", "of"),
        ("many", "of"),
        ("each", "of"),
        ("all", "of"),
        ("part", "of"),
        ("because", "of"),
        ("instead", "of"),
        ("consist", "of"),
        ("consists", "of"),
        ("approve", "of"),
        ("afraid", "of"),
        ("proud", "of"),
        ("aware", "of"),
        ("full", "of"),
        ("think", "about"),
        ("thought", "about"),
        ("talk", "about"),
        ("talked", "about"),
        ("worry", "about"),
        ("worried", "about"),
        ("care", "about"),
        ("deal", "with"),
        ("dealt", "with"),
        ("agree", "with"),
        ("agreed", "with"),
        ("cope", "with"),
        ("comply", "with"),
    ];
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| {
            let (prev, word) = (lower(text, &w[i - 1]), lower(text, &w[i]));
            REQUIRED.contains(&(prev.as_str(), word.as_str()))
                && matches!(
                    lower(text, &w[i + 1]).as_str(),
                    "the" | "a" | "an" | "my" | "our" | "your" | "their" | "this" | "these" | "his" | "her" | "its"
                )
        })
        .collect();
    delete_word(text, w, pick(cands, rng)?)
}

/// "They were late" → "They was late". Only right after a subject pronoun,
/// so the subject is known.
fn agreement_swap(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let cands = (1..w.len())
        .filter(|&i| {
            let prev = lower(text, &w[i - 1]);
            SUBJECT_PRONOUNS.contains(&prev.as_str())
                && AGREEMENT.iter().any(|(a, _)| *a == lower(text, &w[i]))
                // "she was" → "she were" can be a correct subjunctive ("as though she were").
                && !(matches!(prev.as_str(), "i" | "he" | "she" | "it") && lower(text, &w[i]) == "was")
        })
        .collect();
    let i = pick(cands, rng)?;
    let word = lower(text, &w[i]);
    let new = AGREEMENT.iter().find(|(a, _)| *a == word)?.1;
    let (t, d, o) = replace_word(text, w, i, new);
    // Report the subject and verb together.
    Some((t, w[i - 1].start..d.end, w[i - 1].start..o.end))
}

/// "has gone" → "has went"; "did go" → "did went"; "will see" → "will saw".
fn wrong_verb_form(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let mut cands = Vec::new();
    for i in 1..w.len() {
        let (prev, word) = (lower(text, &w[i - 1]), lower(text, &w[i]));
        for &(base, past, part) in IRREGULAR {
            let perfect = matches!(prev.as_str(), "has" | "have" | "had") && word == part && past != part;
            let modal = matches!(prev.as_str(), "did" | "will" | "can" | "could" | "should" | "would" | "must" | "to")
                && word == base
                && past != base;
            if perfect || modal {
                cands.push((i, past));
            }
        }
    }
    let (i, past) = cands[if cands.is_empty() { return None } else { rng.below(cands.len()) }];
    let (t, d, o) = replace_word(text, w, i, past);
    Some((t, w[i - 1].start..d.end, w[i - 1].start..o.end))
}

/// "the new report" → "new the report". Two lowercase neighbours, one of
/// them a short function word, so the result is almost never still correct.
fn swapped_words(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    const FUNCTION: &[&str] =
        &["the", "a", "an", "to", "of", "is", "are", "was", "will", "can", "my", "our", "their", "this"];
    let cands = (1..w.len().saturating_sub(1))
        .filter(|&i| {
            let (a, b) = (&w[i], &w[i + 1]);
            let (la, lb) = (lower(text, a), lower(text, b));
            text[a.end..b.start] == *" "
                && text[a.clone()].starts_with(char::is_lowercase)
                && text[b.clone()].starts_with(char::is_lowercase)
                && la != lb
                && FUNCTION.contains(&la.as_str()) != FUNCTION.contains(&lb.as_str())
                && la.len().max(lb.len()) > 3
        })
        .collect();
    let i = pick(cands, rng)?;
    let (a, b) = (w[i].clone(), w[i + 1].clone());
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..a.start]);
    out.push_str(&text[b.clone()]);
    out.push(' ');
    out.push_str(&text[a.clone()]);
    out.push_str(&text[b.end..]);
    Some((out, a.start..b.end, a.start..b.end))
}

/// "the report" → "the the report".
fn repeated_word(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Option<(String, Range<usize>, Range<usize>)> {
    let cands = (0..w.len())
        .filter(|&i| {
            let word = lower(text, &w[i]);
            word.len() > 1
                && !matches!(word.as_str(), "had" | "that" | "is" | "do")
                && text[w[i].clone()].starts_with(char::is_lowercase)
        })
        .collect();
    let i = pick(cands, rng)?;
    let r = w[i].clone();
    let mut out = String::with_capacity(text.len() + r.len() + 1);
    out.push_str(&text[..r.end]);
    out.push(' ');
    out.push_str(&text[r.clone()]);
    out.push_str(&text[r.end..]);
    Some((out, r.start..r.end + 1 + r.len(), r.clone()))
}

// ---- More of the checklist ------------------------------------------------

type Broken = Option<(String, Range<usize>, Range<usize>)>;

/// Replaces the bytes `r` with `new` (keeping a leading capital).
fn replace_span(text: &str, r: Range<usize>, new: &str) -> (String, Range<usize>, Range<usize>) {
    let new = if text[r.clone()].starts_with(char::is_uppercase) { capitalise(new) } else { new.to_string() };
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..r.start]);
    out.push_str(&new);
    out.push_str(&text[r.end..]);
    (out, r.start..r.start + new.len(), r)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Words `i` and `i + 1` with one plain space between them.
fn pair(text: &str, w: &[Range<usize>], i: usize) -> Option<Range<usize>> {
    (i + 1 < w.len() && text[w[i].end..w[i + 1].start] == *" ").then(|| w[i].start..w[i + 1].end)
}

/// Picks one (word index, replacement for the pair i..=i+1) at random.
fn pick_pair(text: &str, w: &[Range<usize>], cands: Vec<(usize, String)>, rng: &mut Rng) -> Broken {
    let cands: Vec<_> = cands.into_iter().filter(|(i, _)| pair(text, w, *i).is_some()).collect();
    if cands.is_empty() {
        return None;
    }
    let (i, new) = &cands[rng.below(cands.len())];
    Some(replace_span(text, pair(text, w, *i)?, new))
}

/// Nouns that take no a/an and no plural.
const UNCOUNTABLE: &[&str] = &[
    "advice",
    "information",
    "furniture",
    "equipment",
    "research",
    "news",
    "feedback",
    "luggage",
    "baggage",
    "homework",
    "knowledge",
    "evidence",
    "progress",
    "software",
    "traffic",
    "weather",
    "luck",
    "music",
];

/// Common regular verbs (base forms), for agreement and "be + verb" breaks.
const VERBS: &[&str] = &[
    "agree",
    "want",
    "need",
    "like",
    "know",
    "think",
    "work",
    "live",
    "help",
    "ask",
    "call",
    "try",
    "use",
    "seem",
    "feel",
    "look",
    "play",
    "walk",
    "talk",
    "start",
    "finish",
    "open",
    "close",
    "move",
    "love",
    "hope",
    "believe",
    "remember",
    "understand",
    "expect",
    "plan",
    "wait",
    "watch",
    "listen",
    "stay",
    "keep",
    "carry",
    "visit",
    "enjoy",
    "prefer",
    "suggest",
    "send",
    "write",
    "read",
    "come",
    "go",
    "make",
    "take",
    "see",
    "say",
    "tell",
    "give",
    "find",
    "bring",
    "leave",
    "run",
    "sit",
    "stand",
    "speak",
    "hear",
];

/// "some advice" → "an advice".
fn extra_article(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (1..w.len())
        .filter_map(|i| {
            let (prev, word) = (lower(text, &w[i - 1]), lower(text, &w[i]));
            let a = if word.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
            (UNCOUNTABLE.contains(&word.as_str())
                && matches!(
                    prev.as_str(),
                    "some" | "any" | "much" | "more" | "of" | "for" | "need" | "give" | "gave" | "get" | "got" | "have"
                ))
            .then(|| (i - 1, format!("{prev} {a} {}", &text[w[i].clone()])))
        })
        .collect();
    // "some advice" → "an advice", but "of advice" → "of an advice".
    let mut b = pick_pair(text, w, cands, rng)?;
    if let Some(stripped) = ["some ", "any ", "much ", "more "].iter().find_map(|p| b.0[b.1.clone()].strip_prefix(p)) {
        let new = stripped.to_string();
        b = replace_span(text, b.2.clone(), &new);
    }
    Some(b)
}

/// "an hour" → "a hour"; "a car" → "an car".
fn a_an_swap(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| match lower(text, &w[i]).as_str() {
            "a" => Some((i, format!("an {}", &text[w[i + 1].clone()]))),
            "an" => Some((i, format!("a {}", &text[w[i + 1].clone()]))),
            _ => None,
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "discuss the plan" → "discuss about the plan".
fn extra_preposition(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const VERB_PREP: &[(&str, &str)] = &[
        ("discuss", "about"),
        ("discussed", "about"),
        ("mention", "about"),
        ("mentioned", "about"),
        ("emphasize", "on"),
        ("emphasise", "on"),
        ("contact", "to"),
        ("contacted", "to"),
        ("attend", "to"),
        ("attended", "to"),
        ("enter", "into"),
        ("entered", "into"),
        ("reach", "to"),
        ("reached", "to"),
        ("marry", "with"),
        ("married", "with"),
        ("lack", "of"),
        ("lacks", "of"),
        ("answer", "to"),
        ("answered", "to"),
        ("request", "for"),
        ("requested", "for"),
        ("approach", "to"),
        ("approached", "to"),
    ];
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let word = lower(text, &w[i]);
            let next = lower(text, &w[i + 1]);
            let (_, prep) = VERB_PREP.iter().find(|(v, _)| *v == word)?;
            matches!(
                next.as_str(),
                "the"
                    | "a"
                    | "an"
                    | "my"
                    | "our"
                    | "your"
                    | "his"
                    | "her"
                    | "their"
                    | "this"
                    | "it"
                    | "him"
                    | "them"
                    | "me"
                    | "us"
            )
            .then(|| (i, format!("{} {prep} {}", &text[w[i].clone()], &text[w[i + 1].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "on Monday" → "in Monday"; "interested in" → "interested on".
fn wrong_preposition(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const DAYS: &[&str] = &["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    const FIXED: &[(&str, &str, &str)] = &[
        ("interested", "in", "on"),
        ("depend", "on", "of"),
        ("depends", "on", "of"),
        ("rely", "on", "in"),
        ("different", "from", "of"),
        ("afraid", "of", "from"),
        ("good", "at", "in"),
        ("married", "to", "with"),
        ("arrived", "at", "to"),
        ("arrive", "at", "to"),
        ("responsible", "for", "of"),
        ("proud", "of", "on"),
        ("aware", "of", "about"),
        ("listen", "to", "at"),
        ("listened", "to", "at"),
        ("wait", "for", "on"),
        ("waiting", "for", "on"),
        ("waited", "for", "on"),
        ("capable", "of", "to"),
        ("consist", "of", "in"),
        ("consists", "of", "in"),
        ("focus", "on", "in"),
        ("insist", "on", "in"),
        ("insisted", "on", "in"),
    ];
    let mut cands = Vec::new();
    for i in 0..w.len().saturating_sub(1) {
        let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
        if a == "on" && DAYS.contains(&b.as_str()) {
            cands.push((i, format!("in {}", &text[w[i + 1].clone()])));
        }
        if a == "at" && matches!(b.as_str(), "night" | "noon" | "midnight") {
            cands.push((i, format!("in {}", &text[w[i + 1].clone()])));
        }
        if let Some((_, _, bad)) = FIXED.iter().find(|(v, p, _)| *v == a && *p == b) {
            cands.push((i, format!("{} {bad}", &text[w[i].clone()])));
        }
    }
    pick_pair(text, w, cands, rng)
}

/// "It is important to…" → "Is important to…".
fn missing_dummy_subject(text: &str, w: &[Range<usize>], _rng: &mut Rng) -> Broken {
    let (first, second) = (lower(text, w.first()?), lower(text, w.get(1)?));
    if !matches!(first.as_str(), "it" | "there")
        || !matches!(second.as_str(), "is" | "was" | "are" | "were" | "will" | "has" | "seems")
        || w[0].start != 0
    {
        return None;
    }
    let r = pair(text, w, 0)?;
    Some(replace_span(text, r, &capitalise(&second)))
}

/// "with him" → "with he"; "for them" → "for they".
fn pronoun_case(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const CASE: &[(&str, &str)] = &[("him", "he"), ("her", "she"), ("them", "they"), ("me", "I"), ("us", "we")];
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            let (_, subj) = CASE.iter().find(|(o, _)| *o == b)?;
            // "her" is also possessive; only before punctuation or the sentence end.
            let ends = w.get(i + 2).is_none_or(|n| !text[w[i + 1].end..n.start].trim().is_empty());
            (PREPOSITIONS.contains(&a.as_str()) && (b != "her" || ends))
                .then(|| (i, format!("{} {subj}", &text[w[i].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "I agree" → "I am agree"; "they want" → "they are want".
fn extra_be(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            let be = match a.as_str() {
                "i" => "am",
                "we" | "they" | "you" => "are",
                _ => return None,
            };
            VERBS
                .contains(&b.as_str())
                .then(|| (i, format!("{} {be} {}", &text[w[i].clone()], &text[w[i + 1].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "must go" → "must to go"; "let me know" → "let me to know".
fn extra_to(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            (matches!(a.as_str(), "must" | "can" | "could" | "should" | "will" | "would" | "might" | "may")
                && (VERBS.contains(&b.as_str()) || matches!(b.as_str(), "be" | "have" | "do" | "get")))
            .then(|| (i, format!("{} to {}", &text[w[i].clone()], &text[w[i + 1].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "what was left" → "what were left" ("what" as "the thing that" is singular).
fn what_were(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const DONE: &[&str] = &[
        "left", "given", "needed", "said", "done", "required", "missing", "wanted", "offered", "expected", "decided",
        "agreed", "planned", "promised", "lost", "found", "used",
    ];
    let cands = (0..w.len().saturating_sub(2))
        .filter_map(|i| {
            let (a, b, c) = (lower(text, &w[i]), lower(text, &w[i + 1]), lower(text, &w[i + 2]));
            let new = match b.as_str() {
                "was" => "were",
                "is" => "are",
                _ => return None,
            };
            (a == "what" && DONE.contains(&c.as_str())).then(|| (i, format!("{} {new}", &text[w[i].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "she works" → "she work"; "they work" → "they works".
fn verb_s(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            let subj = &text[w[i].clone()];
            match a.as_str() {
                "he" | "she" | "it" => {
                    let base = VERBS.iter().find(|v| third_person(v) == b)?;
                    Some((i, format!("{subj} {base}")))
                }
                "they" | "we" | "i" | "you" => {
                    VERBS.contains(&b.as_str()).then(|| (i, format!("{subj} {}", third_person(&b))))
                }
                _ => None,
            }
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

fn third_person(v: &str) -> String {
    match v {
        "go" => "goes".into(),
        "carry" => "carries".into(),
        "try" => "tries".into(),
        v if v.ends_with("sh") || v.ends_with("ch") || v.ends_with('s') || v.ends_with('x') => format!("{v}es"),
        v => format!("{v}s"),
    }
}

/// "is going" → "is go"; "was waiting" → "was wait".
fn be_base(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            if !matches!(a.as_str(), "is" | "are" | "am" | "was" | "were") {
                return None;
            }
            let base = VERBS.iter().find(|v| ing(v) == b)?;
            Some((i, format!("{} {base}", &text[w[i].clone()])))
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

fn ing(v: &str) -> String {
    match v {
        "run" => "running".into(),
        "sit" => "sitting".into(),
        "plan" => "planning".into(),
        "stand" | "go" | "see" | "agree" => format!("{v}ing"),
        v if v.ends_with('e') => format!("{}ing", &v[..v.len() - 1]),
        v => format!("{v}ing"),
    }
}

/// "Yesterday we went home" → "Yesterday we go home".
fn tense_shift(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let lowered = text.to_lowercase();
    if !["yesterday", "last week", "last year", "last night", " ago"].iter().any(|m| lowered.contains(m)) {
        return None;
    }
    let cands: Vec<(usize, &str)> = (1..w.len())
        .filter_map(|i| {
            let word = lower(text, &w[i]);
            let (base, _, _) = IRREGULAR.iter().find(|(b, past, _)| *past == word && b != past)?;
            // Not after "had" (then it is a participle position).
            (!matches!(lower(text, &w[i - 1]).as_str(), "had" | "have" | "has")).then_some((i, *base))
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, base) = cands[rng.below(cands.len())];
    let r = w[i - 1].start..w[i].end;
    let new = format!("{}{}{base}", &text[w[i - 1].clone()], &text[w[i - 1].end..w[i].start]);
    Some(replace_span(text, r, &new))
}

/// "went" → "goed"; "bought" → "buyed".
fn made_up_past(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands: Vec<(usize, String)> = (0..w.len())
        .filter_map(|i| {
            let word = lower(text, &w[i]);
            let (base, _, _) = IRREGULAR.iter().find(|(b, past, _)| *past == word && b != past)?;
            let made = if base.ends_with('e') { format!("{base}d") } else { format!("{base}ed") };
            Some((i, made))
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, made) = &cands[rng.below(cands.len())];
    Some(replace_span(text, w[*i].clone(), made))
}

/// "these reports" → "these report"; "a report" → "a reports".
fn noun_number(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands = (0..w.len().saturating_sub(1))
        .filter_map(|i| {
            let (a, b) = (lower(text, &w[i]), lower(text, &w[i + 1]));
            let first = &text[w[i].clone()];
            let plural_det = matches!(
                a.as_str(),
                "these" | "those" | "many" | "several" | "two" | "three" | "four" | "five" | "few" | "both"
            );
            let single_det = matches!(a.as_str(), "this" | "each" | "every" | "one" | "another");
            if plural_det && b.len() > 4 && b.ends_with('s') && !b.ends_with("ss") && !b.ends_with("ies") {
                Some((i, format!("{first} {}", &b[..b.len() - 1])))
            } else if single_det
                && b.len() > 3
                && !b.ends_with('s')
                && !b.ends_with("ing")
                && b.chars().all(|c| c.is_ascii_lowercase())
                && !VERBS.contains(&b.as_str())
            {
                Some((i, format!("{first} {b}s")))
            } else {
                None
            }
        })
        .collect();
    pick_pair(text, w, cands, rng)
}

/// "children" → "childs"; "advice" → "advices".
fn made_up_plural(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const PLURALS: &[(&str, &str)] = &[
        ("children", "childs"),
        ("men", "mans"),
        ("women", "womans"),
        ("feet", "foots"),
        ("teeth", "tooths"),
        ("mice", "mouses"),
        ("people", "peoples"),
        ("information", "informations"),
        ("advice", "advices"),
        ("furniture", "furnitures"),
        ("equipment", "equipments"),
        ("feedback", "feedbacks"),
        ("homework", "homeworks"),
        ("luggage", "luggages"),
        ("knowledge", "knowledges"),
        ("research", "researches"),
    ];
    let cands: Vec<(usize, &str)> = (0..w.len())
        .filter_map(|i| PLURALS.iter().find(|(p, _)| *p == lower(text, &w[i])).map(|(_, bad)| (i, *bad)))
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, bad) = cands[rng.below(cands.len())];
    Some(replace_span(text, w[i].clone(), bad))
}

/// "better" → "more better"; "easiest" → "most easiest".
fn double_comparative(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const COMPARATIVES: &[&str] = &[
        "better", "worse", "bigger", "smaller", "easier", "faster", "larger", "older", "younger", "cheaper", "harder",
        "higher", "lower", "longer", "shorter",
    ];
    const SUPERLATIVES: &[&str] = &[
        "best", "worst", "biggest", "smallest", "easiest", "fastest", "largest", "oldest", "youngest", "cheapest",
        "hardest", "highest", "lowest", "longest", "shortest",
    ];
    let cands: Vec<(usize, String)> = (0..w.len())
        .filter_map(|i| {
            let word = lower(text, &w[i]);
            let prev = if i > 0 { lower(text, &w[i - 1]) } else { String::new() };
            if COMPARATIVES.contains(&word.as_str()) && prev != "much" && prev != "no" {
                Some((i, format!("more {}", &text[w[i].clone()])))
            } else if SUPERLATIVES.contains(&word.as_str()) && prev == "the" {
                Some((i, format!("most {}", &text[w[i].clone()])))
            } else {
                None
            }
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, new) = &cands[rng.below(cands.len())];
    Some(replace_span(text, w[*i].clone(), new))
}

/// "spoke carefully" → "spoke careful" (adverb after a verb).
fn adjective_for_adverb(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    // Not quick/slow/clear/easy/safe: "go slow", "take it easy" are fine informally.
    const ADVERBS: &[(&str, &str)] = &[
        ("carefully", "careful"),
        ("badly", "bad"),
        ("quietly", "quiet"),
        ("properly", "proper"),
        ("correctly", "correct"),
        ("perfectly", "perfect"),
        ("seriously", "serious"),
        ("honestly", "honest"),
        ("beautifully", "beautiful"),
        ("smoothly", "smooth"),
        ("regularly", "regular"),
        ("efficiently", "efficient"),
    ];
    let cands: Vec<(usize, &str)> = (1..w.len())
        .filter_map(|i| {
            let (_, adj) = ADVERBS.iter().find(|(adv, _)| *adv == lower(text, &w[i]))?;
            // After a verb-looking word ("-ed", "-s", or a known verb), not before an adjective.
            let prev = lower(text, &w[i - 1]);
            // Linking verbs take adjectives: "went quiet", "felt bad", "looked careful".
            const LINKING: &[&str] = &[
                "went", "go", "goes", "got", "get", "gets", "became", "become", "seemed", "seem", "seems", "looked",
                "look", "looks", "felt", "feel", "feels", "stayed", "stay", "kept", "keep", "turned", "fell", "grew",
                "sounded", "remained", "appeared",
            ];
            if LINKING.contains(&prev.as_str()) {
                return None;
            }
            (prev.ends_with("ed")
                || VERBS.contains(&prev.as_str())
                || VERBS.iter().any(|v| third_person(v) == prev)
                || IRREGULAR.iter().any(|(_, past, _)| *past == prev))
            .then_some((i, *adj))
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, adj) = cands[rng.below(cands.len())];
    let r = w[i - 1].start..w[i].end;
    let new = format!("{}{}{adj}", &text[w[i - 1].clone()], &text[w[i - 1].end..w[i].start]);
    Some(replace_span(text, r, &new))
}

/// "it's" → "its", "your" → "you're", "than" → "then"…
fn sound_alike(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    const SWAPS: &[(&str, &str)] = &[
        ("it's", "its"),
        ("its", "it's"),
        ("you're", "your"),
        ("your", "you're"),
        ("they're", "their"),
        ("their", "there"),
        ("there", "their"),
        ("than", "then"),
        ("too", "to"),
        ("lose", "loose"),
        ("affect", "effect"),
        ("effect", "affect"),
        ("whose", "who's"),
        ("we're", "were"),
        ("accept", "except"),
        ("quite", "quiet"),
        ("whether", "weather"),
        ("advice", "advise"),
        ("passed", "past"),
        ("principal", "principle"),
    ];
    // And every word of the sound-alike checker's sets (beer/bear/bare…).
    let sets = xenify_wp_check::sound_alike::sets();
    let cands: Vec<(usize, String)> = (1..w.len())
        .filter_map(|i| {
            let word = lower(text, &w[i]).replace('’', "'");
            if let Some((_, b)) = SWAPS.iter().find(|(a, _)| *a == word) {
                return Some((i, b.to_string()));
            }
            let set = sets.iter().find(|s| s.iter().any(|m| m.word == word))?;
            let others: Vec<&str> = set.iter().map(|m| m.word.as_str()).filter(|o| *o != word).collect();
            (!others.is_empty()).then(|| (i, others[(i * 7 + word.len()) % others.len()].to_string()))
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, new) = cands[rng.below(cands.len())].clone();
    let r = w[i - 1].start..w[i].end;
    let new = format!("{}{}{new}", &text[w[i - 1].clone()], &text[w[i - 1].end..w[i].start]);
    Some(replace_span(text, r, &new))
}

/// Adverbs in -ly that describe the next word (never hyphenated to it).
const LY_ADVERBS: &[&str] = &[
    "highly",
    "newly",
    "widely",
    "poorly",
    "recently",
    "freshly",
    "carefully",
    "beautifully",
    "badly",
    "nicely",
    "fully",
    "clearly",
    "openly",
    "deeply",
    "brightly",
    "richly",
    "finely",
    "heavily",
    "lightly",
    "properly",
    "perfectly",
    "completely",
    "barely",
    "slowly",
    "quickly",
    "frequently",
    "commonly",
    "publicly",
    "privately",
    "internationally",
    "locally",
    "specially",
    "specifically",
    "strongly",
    "tightly",
    "loosely",
    "neatly",
];

const LINKING: &[&str] = &["is", "are", "was", "were", "be", "been", "seems", "seemed", "became", "remains", "looks"];

/// "a highly skilled team" → "a highly-skilled team"; "it is well known." →
/// "it is well-known." (no hyphen in either).
fn extra_hyphen(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let mut cands: Vec<(Range<usize>, String)> = Vec::new();
    for i in 0..w.len().saturating_sub(2) {
        let adv = lower(text, &w[i]);
        let next = lower(text, &w[i + 1]);
        // An -ly adverb + a participle or adjective + a word (the noun).
        if LY_ADVERBS.contains(&adv.as_str())
            && (next.ends_with("ed") || next.ends_with("ing") || next.ends_with("ful") || next.ends_with("ive"))
            && next.len() > 3
            && let Some(r) = pair(text, w, i)
        {
            cands.push((r, format!("{}-{}", &text[w[i].clone()], &text[w[i + 1].clone()])));
        }
    }
    for i in 1..w.len() {
        // "is well known" at the end of a clause.
        let end = |k: usize| w.get(k).is_none_or(|n| !text[w[k - 1].end..n.start].trim().is_empty());
        if LINKING.contains(&lower(text, &w[i - 1]).as_str()) && i + 1 < w.len() {
            if lower(text, &w[i]) == "well"
                && (lower(text, &w[i + 1]).ends_with("ed") || lower(text, &w[i + 1]).ends_with("n"))
                && end(i + 2)
                && let Some(r) = pair(text, w, i)
            {
                cands.push((r, format!("{}-{}", &text[w[i].clone()], &text[w[i + 1].clone()])));
            }
            if i + 2 < w.len()
                && ["up to date", "out of date"].contains(&text[w[i].start..w[i + 2].end].to_lowercase().as_str())
                && end(i + 3)
            {
                let r = w[i].start..w[i + 2].end;
                cands.push((r.clone(), text[r].replace(' ', "-")));
            }
        }
    }
    if cands.is_empty() {
        return None;
    }
    let (r, new) = cands.swap_remove(rng.below(cands.len()));
    Some(replace_span(text, r, &new))
}

/// First words of compound adjectives: "well-known", "long-term", "self-made".
const MODIFIERS: &[&str] = &[
    "well", "ill", "long", "short", "high", "low", "full", "part", "self", "all", "cross", "real", "one", "two",
    "three", "four", "five", "six", "ten", "last", "first", "second", "third", "fast", "slow", "open", "world",
    "state", "best", "worst", "much", "hard", "easy", "free", "old", "small", "large", "mid", "user", "time", "top",
    "low", "large", "full", "data", "cost", "life", "child", "high", "eco",
];
const NUMBERS: &[&str] = &["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
/// Phrasal verbs as adjectives before a noun ("a follow-up meeting").
const JOINED: &[&str] = &[
    "follow-up",
    "check-in",
    "sign-up",
    "log-in",
    "set-up",
    "start-up",
    "warm-up",
    "kick-off",
    "drop-off",
    "pick-up",
    "one-on-one",
    "face-to-face",
    "up-to-date",
    "out-of-date",
    "day-to-day",
    "end-to-end",
    "state-of-the-art",
    "step-by-step",
    "on-site",
    "in-house",
    "off-site",
    "on-call",
];

/// "a well-known author" → "a well known author": a compound before a noun
/// loses its hyphen.
fn missing_hyphen(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands: Vec<usize> = (0..w.len().saturating_sub(1))
        .filter(|&i| {
            let word = &text[w[i].clone()];
            let parts: Vec<&str> = word.split('-').collect();
            // A modern compound adjective ("well-known", "long-term", "sandy-haired",
            // "twenty-eight", "follow-up"), not an old noun ("dining-room", "to-day")
            // or a prefix ("re-enter"), and not a name.
            let compound = (2..=3).contains(&parts.len())
                && parts.iter().all(|p| p.len() >= 2 && p.chars().all(|c| c.is_ascii_lowercase()))
                && parts[0].len() >= 3
                && (MODIFIERS.contains(&parts[0])
                    || NUMBERS.contains(&parts[0])
                    || parts[parts.len() - 1].ends_with("ed")
                    || parts[parts.len() - 1].ends_with("ing")
                    || JOINED.contains(&word));
            // Before a noun (a lower-case word right after), not after a linking verb.
            let before_noun =
                text[w[i].end..w[i + 1].start] == *" " && text[w[i + 1].clone()].chars().all(|c| c.is_lowercase());
            let predicate = i > 0 && LINKING.contains(&lower(text, &w[i - 1]).as_str());
            compound && before_noun && !predicate
        })
        .collect();
    let i = pick(cands, rng)?;
    let r = w[i].clone();
    let new = text[r.clone()].replace('-', " ");
    Some(replace_span(text, r, &new))
}

/// Words that can't follow a subject pronoun ("we the", "she to").
const DANGLING: &[&str] = &[
    "the", "a", "an", "to", "of", "and", "with", "for", "in", "on", "at", "by", "from", "that", "this", "my", "your",
    "our", "their", "his", "her", "its",
];

/// Words a sentence can't end on ("to the.", "of." — but "for this." and "with her." are fine).
const NO_END: &[&str] = &["the", "a", "an", "my", "your", "our", "their", "its", "of", "and"];

/// Pronouns that are only ever subjects ("it" and "you" can be objects: "made it the best").
const SUBJECTS_ONLY: &[&str] = &["i", "he", "she", "we", "they"];

/// Edit leftovers: a copy of 2–3 words from elsewhere in the sentence lands
/// mid-sentence ("We moved the meeting to the meeting Friday").
fn leftover_words(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    (0..8).find_map(|_| leftover_words_once(text, w, rng))
}

fn leftover_words_once(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    if w.len() < 7 {
        return None;
    }
    let len = 2 + rng.below(2);
    let from = rng.below(w.len() - len);
    let to = 1 + rng.below(w.len() - 2);
    // Not right next to its own copy (a repeated phrase is a different check).
    if to + 1 >= from && to <= from + len + 1 {
        return None;
    }
    let chunk = &text[w[from].start..w[from + len - 1].end];
    if chunk.contains(|c: char| ",.;:!?\"“”()".contains(c) || c.is_ascii_digit()) {
        return None;
    }
    // Inserted before word `to`, after a plain space.
    if &text[w[to - 1].end..w[to].start] != " " {
        return None;
    }
    // The damage is the copy plus the word it lands before (so it's unique in the text).
    let at = w[to].start;
    // Lower-case only a sentence's own first word (names keep their capitals).
    let chunk = if from == 0 { chunk.to_lowercase() } else { chunk.to_string() };
    let broken = format!("{}{chunk} {}", &text[..at], &text[at..]);
    Some((broken, at..at + chunk.len() + 1 + w[to].len(), w[to].clone()))
}

/// A rewrite that lost its verb: "We moved the meeting" → "We the meeting".
fn dropped_verb(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    let cands: Vec<(usize, usize)> = (0..w.len().saturating_sub(3))
        .flat_map(|i| [(i, 1), (i, 2)])
        .filter(|&(i, n)| {
            SUBJECTS_ONLY.contains(&lower(text, &w[i]).as_str())
                && i + n + 1 < w.len()
                && (1..=n).all(|k| !DANGLING.contains(&lower(text, &w[i + k]).as_str()))
                && DANGLING.contains(&lower(text, &w[i + n + 1]).as_str())
                && text[w[i].end..w[i + n + 1].start].chars().all(|c| c == ' ' || c.is_alphanumeric() || c == '\'')
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    let (i, n) = cands[rng.below(cands.len())];
    let r = w[i].start..w[i + n + 1].end;
    let new = format!("{} {}", &text[w[i].clone()], &text[w[i + n + 1].clone()]);
    Some(replace_span(text, r, &new))
}

/// Three or four neighbouring words in a jumbled order.
fn jumbled_words(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    (0..8).find_map(|_| jumbled_words_once(text, w, rng))
}

fn jumbled_words_once(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    if w.len() < 6 {
        return None;
    }
    let n = 3 + rng.below(2);
    let i = 1 + rng.below(w.len() - n);
    if i + n > w.len()
        || (i..i + n - 1).any(|k| &text[w[k].end..w[k + 1].start] != " ")
        || text[w[i].start..w[i + n - 1].end].contains(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    let orig: Vec<&str> = (i..i + n).map(|k| &text[w[k].clone()]).collect();
    // Rotate and swap so no word keeps its place.
    let mut new: Vec<&str> = orig[1..].to_vec();
    new.push(orig[0]);
    new.swap(0, n - 2);
    if (0..n).any(|k| new[k].eq_ignore_ascii_case(orig[k])) {
        return None;
    }
    let r = w[i].start..w[i + n - 1].end;
    Some(replace_span(text, r, &new.join(" ")))
}

/// A sentence cut off mid-clause: "…and sent it to the." (words lost before
/// the full stop).
fn cut_off_ending(text: &str, w: &[Range<usize>], rng: &mut Rng) -> Broken {
    // Sentence ends: a word right before ". " or "." at the end.
    let cands: Vec<(usize, usize)> = (4..w.len())
        .filter(|&i| {
            let after = &text[w[i].end..];
            after.starts_with('.') && !after.starts_with("..")
        })
        .flat_map(|i| (1..=3).map(move |n| (i, n)))
        .filter(|&(i, n)| {
            i >= n + 3
                && NO_END.contains(&lower(text, &w[i - n]).as_str())
                && (i - n + 1..=i).all(|k| !DANGLING.contains(&lower(text, &w[k]).as_str()))
                && text[w[i - n].start..w[i].end].chars().all(|c| c == ' ' || c.is_alphanumeric() || c == '\'')
        })
        .collect();
    if cands.is_empty() {
        return None;
    }
    // The damage is the last word left and the one before it ("to the").
    let (i, n) = cands[rng.below(cands.len())];
    let r = w[i - n - 1].start..w[i].end;
    Some(replace_span(text, r, &text[w[i - n - 1].start..w[i - n].end]))
}

/// "Where is the station?" → "Where the station is?"
fn question_order(text: &str, w: &[Range<usize>], _rng: &mut Rng) -> Broken {
    if !text.trim_end().ends_with('?') || w.len() < 4 {
        return None;
    }
    let wh = lower(text, &w[0]);
    let aux = lower(text, &w[1]);
    if w[0].start != 0
        || !matches!(wh.as_str(), "where" | "why" | "when" | "what" | "how" | "who")
        || !matches!(
            aux.as_str(),
            "is" | "are" | "was" | "were" | "do" | "does" | "did" | "can" | "will" | "would" | "should" | "could"
        )
    {
        return None;
    }
    // Move the helper verb after the subject: one pronoun, or "the/a/my" + noun.
    let subj_end = match lower(text, &w[2]).as_str() {
        "i" | "you" | "he" | "she" | "it" | "we" | "they" => 2,
        "the" | "a" | "an" | "my" | "your" | "our" | "his" | "her" | "their" | "this" | "that" => 3,
        _ => return None,
    };
    if subj_end >= w.len()
        || !text[w[1].start..w[subj_end].end]
            .split(' ')
            .all(|t| t.chars().all(|c| c.is_alphanumeric() || c == '\'' || c == '’'))
    {
        return None;
    }
    let subject = &text[w[2].start..w[subj_end].end];
    let r = w[1].start..w[subj_end].end;
    let (out, d, o) = replace_span(text, r, &format!("{subject} {}", &text[w[1].clone()]));
    Some((out, 0..d.end, 0..o.end))
}

/// Correct standard passages from a corpus file: corrected mistakes and clean text.
fn sources(path: &Path) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for it in read_items(path)? {
        let text = match (&it.error_type, it.corrected) {
            (Some(_), Some(c)) => c,
            (None, _) if it.register == "standard" && it.features.is_empty() => it.text,
            _ => continue,
        };
        if seen.insert(text.clone()) {
            out.push(text);
        }
    }
    Ok(out)
}

/// Breaks each source passage in up to `per_source` different ways.
pub fn run(inputs: &[std::path::PathBuf], out: &Path, per_source: usize, seed: u64) -> Result<(), String> {
    let mut rng = Rng(seed.max(1));
    let mut items = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for input in inputs {
        let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("corpus");
        for (n, text) in sources(input)?.into_iter().enumerate() {
            let w = words(&text);
            // Try the kinds made least so far first (random among ties), so
            // common breakages don't crowd out rare ones.
            let mut order: Vec<(usize, u64, usize)> =
                (0..KINDS.len()).map(|k| (counts.get(KINDS[k].0).copied().unwrap_or(0), rng.next(), k)).collect();
            order.sort();
            let order = order.into_iter().map(|(_, _, k)| k);
            let mut made = 0;
            for k in order {
                if made == per_source {
                    break;
                }
                let (kind, f) = KINDS[k];
                let Some((broken, damage, orig)) = f(&text, &w, &mut rng) else { continue };
                made += 1;
                *counts.entry(kind).or_default() += 1;
                items.push(Item {
                    id: format!("{kind}:{stem}:{n}"),
                    job: format!("{kind}:{stem}"),
                    setting: stem.to_string(),
                    register: "standard".into(),
                    error_type: Some(kind.to_string()),
                    voice: None,
                    mistakes: vec![Mistake { wrong: broken[damage].to_string(), fix: text[orig].to_string() }],
                    corrected: Some(text.clone()),
                    text: broken,
                    features: Vec::new(),
                    generator: "breaker".into(),
                    verified_by: None,
                });
            }
        }
    }
    if out.exists() {
        std::fs::remove_file(out).map_err(|e| e.to_string())?;
    }
    append_items(out, &items)?;
    for (k, n) in &counts {
        println!("{k:32} {n}");
    }
    println!("{} broken passages → {}", items.len(), out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn once(f: Breaker, text: &str) -> Option<(String, String, String)> {
        let (t, d, o) = f(text, &words(text), &mut Rng(7))?;
        Some((t.clone(), t[d].to_string(), text[o].to_string()))
    }

    #[test]
    fn breaks_as_described() {
        assert_eq!(
            once(missing_article, "We sat in the sun all day."),
            Some(("We sat in sun all day.".into(), "in sun".into(), "in the sun".into()))
        );
        assert_eq!(
            once(missing_do, "Crazy words do not catch grammar."),
            Some(("Crazy words not catch grammar.".into(), "words not".into(), "words do not".into()))
        );
        assert_eq!(
            once(agreement_swap, "Then they were late."),
            Some(("Then they was late.".into(), "they was".into(), "they were".into()))
        );
        assert_eq!(
            once(wrong_verb_form, "She has gone home."),
            Some(("She has went home.".into(), "has went".into(), "has gone".into()))
        );
        assert_eq!(
            once(repeated_word, "Hi there"),
            Some(("Hi there there".into(), "there there".into(), "there".into()))
        );
        assert_eq!(once(missing_auxiliary, "No words here."), None);
        // Garbled: edit leftovers, a lost verb, jumbled words, a cut-off ending.
        assert_eq!(
            once(dropped_verb, "Then we moved the meeting to Friday."),
            Some(("Then we the meeting to Friday.".into(), "we the".into(), "we moved the".into()))
        );
        assert_eq!(
            once(cut_off_ending, "She wrote the report and sent it to the manager."),
            Some(("She wrote the report and sent it to the.".into(), "to the".into(), "to the manager".into()))
        );
        assert_eq!(once(cut_off_ending, "He was ready for this."), None);
        assert_eq!(once(dropped_verb, "The rain made it the best day of the year."), None);
        let (t, d, _) = once(jumbled_words, "The committee will review every proposal next month.").unwrap();
        assert_ne!(t, "The committee will review every proposal next month.");
        assert_eq!(d.split(' ').count(), t.split(' ').count() - 4);
        let (t, d, o) = once(leftover_words, "The new library opens on Monday with a small party.").unwrap();
        assert_eq!(t.replacen(&d, &o, 1), "The new library opens on Monday with a small party.");
        // Hyphens: one the sentence doesn't need, and one it does.
        assert_eq!(
            once(extra_hyphen, "They hired a highly skilled team."),
            Some(("They hired a highly-skilled team.".into(), "highly-skilled".into(), "highly skilled".into()))
        );
        assert_eq!(
            once(extra_hyphen, "The author is well known."),
            Some(("The author is well-known.".into(), "well-known".into(), "well known".into()))
        );
        assert_eq!(once(extra_hyphen, "The author is well known for poems."), None);
        assert_eq!(
            once(missing_hyphen, "She is a well-known author."),
            Some(("She is a well known author.".into(), "well known".into(), "well-known".into()))
        );
        assert_eq!(once(missing_hyphen, "The author is well-known."), None);
        assert_eq!(once(missing_hyphen, "They re-entered the room."), None);
        // Sound-alikes from the checker's sets.
        assert_eq!(
            once(sound_alike, "He waved the fee."),
            Some(("He waived the fee.".into(), "He waived".into(), "He waved".into()))
        );
        // Correct without the article or preposition, so not broken that way.
        assert_eq!(once(missing_article, "She came back from the church early."), None);
        assert_eq!(once(missing_article, "They sailed on the Thames."), None);
        assert_eq!(once(missing_preposition, "It smelled of the sea."), None);
        assert_eq!(
            once(missing_preposition, "We listened to the radio."),
            Some(("We listened the radio.".into(), "listened the".into(), "listened to the".into()))
        );
        let cases: &[(Breaker, &str, &str)] = &[
            (missing_dummy_subject, "It is important to rest.", "Is important to rest."),
            (pronoun_case, "She came with him.", "She came with he."),
            (extra_be, "Yes, I agree with you.", "Yes, I am agree with you."),
            (extra_to, "You must go now.", "You must to go now."),
            (verb_s, "Then she works late.", "Then she work late."),
            (be_base, "He is going home.", "He is go home."),
            (tense_shift, "Yesterday we went home.", "Yesterday we go home."),
            (made_up_past, "We went home.", "We goed home."),
            (noun_number, "Read these reports first.", "Read these report first."),
            (made_up_plural, "The children laughed.", "The childs laughed."),
            (double_comparative, "This is better.", "This is more better."),
            (adjective_for_adverb, "She spoke carefully.", "She spoke careful."),
            (sound_alike, "It is bigger than mine.", "It is bigger then mine."),
            (question_order, "Where is the station?", "Where the station is?"),
            (wrong_preposition, "See you on Monday.", "See you in Monday."),
            (extra_preposition, "We discussed the plan.", "We discussed about the plan."),
            (extra_article, "I need some advice.", "I need an advice."),
            (a_an_swap, "Wait an hour.", "Wait a hour."),
            (what_were, "Take what was left.", "Take what were left."),
        ];
        for (f, from, to) in cases {
            assert_eq!(once(*f, from).map(|b| b.0).as_deref(), Some(*to), "{from}");
        }
    }
}
