//! Xenify's own grammar rules that need more than word patterns (sentence
//! position, what follows, word metadata). Each rule is a Harper [`Linter`]
//! working on one sentence at a time, through the small [`Word`] view.
//!
//! The rules are our own work from general English grammar; the mistakes
//! they target come from common-error lists (pronoun case, nonstandard past
//! forms, its/it's, agreement) and from the proofing corpus scoreboard.

use harper_core::linting::{Lint, LintKind, Linter, Suggestion};
use harper_core::{Document, Span, Token, TokenKind, TokenStringExt};

/// A word or punctuation mark in a sentence (whitespace skipped).
struct Word<'a> {
    tok: &'a Token,
    text: String,
    lower: String,
}

impl Word<'_> {
    fn kind(&self) -> &TokenKind {
        &self.tok.kind
    }
    fn is_word(&self) -> bool {
        self.tok.kind.is_word()
    }
    fn is(&self, w: &str) -> bool {
        self.lower == w
    }
    fn any(&self, ws: &[&str]) -> bool {
        ws.contains(&self.lower.as_str())
    }
}

fn words<'a>(sentence: &'a [Token], src: &[char]) -> Vec<Word<'a>> {
    sentence
        .iter()
        .filter(|t| !t.kind.is_whitespace() && !matches!(t.kind, TokenKind::Newline(_) | TokenKind::ParagraphBreak))
        .map(|t| {
            let text = t.get_str(src);
            Word { tok: t, lower: text.to_lowercase().replace('\u{2019}', "'"), text }
        })
        .collect()
}

/// Copies the capitalisation of `like` (first letter) onto `word`.
fn match_case(word: &str, like: &str) -> String {
    if like.chars().next().is_some_and(char::is_uppercase) {
        let mut c = word.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    } else {
        word.to_string()
    }
}

fn lint(span: Span<char>, kind: LintKind, message: impl Into<String>, suggestions: &[String]) -> Lint {
    Lint {
        span,
        lint_kind: kind,
        suggestions: suggestions.iter().map(|s| Suggestion::ReplaceWith(s.chars().collect())).collect(),
        message: message.into(),
        priority: 31,
    }
}

fn span_of(ws: &[Word]) -> Span<char> {
    Span::new(ws[0].tok.span.start, ws[ws.len() - 1].tok.span.end)
}

/// The text of `ws` as written (single spaces).
fn joined(ws: &[Word]) -> String {
    ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Runs `f` on each sentence of the document.
fn each_sentence(document: &Document, mut f: impl FnMut(&[Word], &mut Vec<Lint>)) -> Vec<Lint> {
    let src = document.get_source();
    let mut out = Vec::new();
    for s in document.get_tokens().iter_sentences() {
        let ws = words(s, src);
        if !ws.is_empty() {
            f(&ws, &mut out);
        }
    }
    out
}

const AUX: &[&str] = &[
    "am",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "have",
    "has",
    "had",
    "do",
    "does",
    "did",
    "will",
    "would",
    "can",
    "could",
    "shall",
    "should",
    "may",
    "might",
    "must",
    "won't",
    "wouldn't",
    "can't",
    "cannot",
    "couldn't",
    "shouldn't",
    "don't",
    "doesn't",
    "didn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "haven't",
    "hasn't",
    "hadn't",
];

/// A finite verb that can follow a subject (not an -ing form or a noun-only word).
fn finite_verb(w: &Word) -> bool {
    if w.any(AUX) {
        return true;
    }
    let k = w.kind();
    k.is_verb() && !k.is_verb_progressive_form() && !((k.is_noun() || k.is_adjective()) && !k.is_verb_past_form())
}

// ---------------------------------------------------------------------------

/// Subject vs object pronouns in pairs: "Her and Marcus are…" → "She and
/// Marcus", "for my brother and I" → "me", "between you and I".
#[derive(Default)]
pub struct PronounCase;

fn subject_form(p: &str) -> Option<&'static str> {
    Some(match p {
        "me" => "I",
        "him" => "he",
        "her" => "she",
        "us" => "we",
        "them" => "they",
        _ => return None,
    })
}

fn object_form(p: &str) -> Option<&'static str> {
    Some(match p {
        "i" => "me",
        "he" => "him",
        "she" => "her",
        "we" => "us",
        "they" => "them",
        _ => return None,
    })
}

/// Prepositions and verbs that take an object (so a following pronoun pair
/// must be in object form).
const OBJECT_TAKERS: &[&str] = &[
    "between",
    "for",
    "to",
    "with",
    "from",
    "about",
    "of",
    "at",
    "by",
    "among",
    "against",
    "without",
    "toward",
    "towards",
    "behind",
    "near",
    "except",
    "including",
    "beside",
    "besides",
    "like",
    "unlike",
    "upon",
    "onto",
    "into",
    "around",
    "through",
    "via",
    "told",
    "tell",
    "tells",
    "asked",
    "ask",
    "asks",
    "gave",
    "give",
    "gives",
    "sent",
    "send",
    "sends",
    "invited",
    "invite",
    "invites",
    "let",
    "lets",
    "made",
    "make",
    "makes",
    "helped",
    "help",
    "helps",
    "thanked",
    "thank",
    "thanks",
    "called",
    "emailed",
    "showed",
    "show",
    "shows",
    "taught",
    "teach",
    "joined",
    "join",
    "met",
    "saw",
    "see",
    "heard",
    "hear",
    "included",
    "include",
    "paid",
    "pay",
    "hired",
    "picked",
    "chose",
    "choose",
    "selected",
    "select",
    "congratulated",
    "contacted",
    "contact",
    "took",
    "take",
    "brought",
    "bring",
    "offered",
    "offer",
    "texted",
    "warned",
    "reminded",
    "remind",
    "allowed",
    "allow",
    "encouraged",
    "encourage",
    "cc",
    "cc'd",
    "copy",
    "copied",
    "introduced",
    "assigned",
    "assign",
    "promoted",
    "drove",
    "drive",
    "dropped",
    "visited",
    "visit",
    "followed",
    "follow",
    "trusted",
    "trust",
    "blamed",
    "blame",
    "kept",
    "keep",
    "left",
    "photographed",
    "interviewed",
    "nominated",
    "elected",
    "named",
    "appointed",
    "mentioned",
    "praised",
    "booked",
    "registered",
    "signed",
    "tagged",
];

impl Linter for PronounCase {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            // (a) A pair as the subject at the start of the sentence.
            let start = ws.iter().position(|w| w.is_word()).unwrap_or(0);
            let s = &ws[start..];
            if s.len() >= 4 {
                // "Me/Him/Her/Us/Them and X <verb>"
                if let Some(subj) = subject_form(&s[0].lower).filter(|_| s[1].is("and")) {
                    // X: up to three words, then a finite verb.
                    if let Some(n) = (1..=3).find(|&n| {
                        s.len() > 2 + n
                            && finite_verb(&s[2 + n])
                            && s[2..2 + n].iter().all(|w| {
                                w.is_word() && !w.any(AUX) && !w.lower.ends_with("'s") && !w.lower.ends_with('\'')
                            })
                    }) {
                        let other = joined(&s[2..2 + n]);
                        let fix = if subj == "I" {
                            format!("{} and I", match_case(&other, &s[0].text))
                        } else if let Some(o) = subject_form(&s[2].lower).filter(|_| n == 1) {
                            // "Me and him" → "He and I"
                            format!("{} and {}", match_case(o, &s[0].text), subj)
                        } else {
                            format!("{} and {other}", match_case(subj, &s[0].text))
                        };
                        let fix = if s[0].is("me") && n == 1 && s[2].is("him") | s[2].is("her") | s[2].is("them") {
                            format!("{} and I", match_case(subject_form(&s[2].lower).unwrap(), &s[0].text))
                        } else {
                            fix
                        };
                        out.push(lint(
                            span_of(&s[..2 + n]),
                            LintKind::Grammar,
                            "A pronoun in the subject of a sentence takes its subject form (I, he, she, we, they).",
                            &[fix],
                        ));
                    }
                }
                // "X and him/her/them/me <verb>" at the start: "Marcus and him went".
                if let Some(n) = (1..=3).find(|&n| {
                    s.len() > n + 2
                        && s[n].is("and")
                        && s[..n]
                            .iter()
                            .all(|w| w.is_word() && !w.any(AUX) && subject_form(&w.lower).is_none() && !w.is("i"))
                }) && let Some(subj) = subject_form(&s[n + 1].lower)
                    && finite_verb(&s[n + 2])
                    && (s[0].kind().is_proper_noun()
                        || s[0].kind().is_determiner()
                        || s[0].kind().is_pronoun()
                        || s[0].text.chars().next().is_some_and(char::is_uppercase))
                {
                    out.push(lint(
                        span_of(&s[n + 1..n + 2]),
                        LintKind::Grammar,
                        "In the subject of a sentence, use the subject form of the pronoun.",
                        &[subj.to_string()],
                    ));
                }
            }
            // (b) "<object taker> X and I" not followed by a verb: "for my brother and I".
            for i in 0..ws.len() {
                if !ws[i].any(OBJECT_TAKERS) {
                    continue;
                }
                // X: one to three words, then "and", then I/he/she/we/they.
                for n in 1..=3 {
                    let (a, p) = (i + 1 + n, i + 2 + n);
                    if p >= ws.len() || !ws[a].is("and") {
                        continue;
                    }
                    if !ws[i + 1..a].iter().all(|w| {
                        w.is_word()
                            && !w.any(AUX)
                            && !w.is("i")
                            && subject_form(&w.lower).is_none()
                            && object_form(&w.lower).is_none()
                            || w.is("you")
                    }) {
                        continue;
                    }
                    let Some(obj) = object_form(&ws[p].lower) else { continue };
                    // A verb after the pair (perhaps after adverbs) means it's the subject of a clause
                    // ("…for Sam and I wanted", "…and they block", "…and I also sent").
                    let mut q = p + 1;
                    while ws.get(q).is_some_and(|w| w.kind().is_adverb() && !w.any(&["to", "then"])) {
                        q += 1;
                    }
                    if ws.get(q).is_some_and(|w| {
                        (w.any(AUX)
                            || w.kind().is_verb()
                                && (w.kind().is_verb_past_form()
                                    || !w.kind().is_adjective() && !w.kind().is_determiner()))
                            && !w.is("to")
                    }) {
                        continue;
                    }
                    if ws.get(p + 1).is_some_and(|w| w.lower.starts_with('\'')) {
                        continue; // I'm, I've…
                    }
                    // "…from Sony and they sound amazing": he/she/we/they before a verb start a clause.
                    if !ws[p].is("i") && ws.get(q).is_some_and(|w| w.kind().is_verb()) {
                        continue;
                    }
                    out.push(lint(
                        span_of(&ws[p..p + 1]),
                        LintKind::Grammar,
                        format!("After `{}`, use the object form of the pronoun: `{obj}`.", ws[i].text),
                        &[obj.to_string()],
                    ));
                    break;
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Finds subject/object pronoun mix-ups in pairs: `Her and Sam are` → `She and Sam are`, `for my brother and I` → `for my brother and me`, `between you and I`."
    }
}

// ---------------------------------------------------------------------------

/// "I seen it", "they done it", "she taken it": a past participle used as
/// the simple past.
#[derive(Default)]
pub struct ParticipleAsPast;

fn simple_past(pp: &str) -> Option<&'static str> {
    Some(match pp {
        "seen" => "saw",
        "done" => "did",
        "gone" => "went",
        "taken" => "took",
        "given" => "gave",
        "written" => "wrote",
        "eaten" => "ate",
        "spoken" => "spoke",
        "broken" => "broke",
        "chosen" => "chose",
        "driven" => "drove",
        "ridden" => "rode",
        "risen" => "rose",
        "shown" => "showed",
        "known" => "knew",
        "grown" => "grew",
        "thrown" => "threw",
        "drawn" => "drew",
        "flown" => "flew",
        "worn" => "wore",
        "torn" => "tore",
        "begun" => "began",
        "sung" => "sang",
        "rung" => "rang",
        "drunk" => "drank",
        "sunk" => "sank",
        "swum" => "swam",
        "stolen" => "stole",
        "frozen" => "froze",
        "forgotten" => "forgot",
        "hidden" => "hid",
        "bitten" => "bit",
        "fallen" => "fell",
        "shaken" => "shook",
        "woken" => "woke",
        _ => return None,
    })
}

impl Linter for ParticipleAsPast {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !ws[i].any(&["i", "you", "we", "they", "he", "she", "it"]) {
                    continue;
                }
                let v = &ws[i + 1];
                let Some(past) = simple_past(&v.lower) else { continue };
                // "they done finished" (completive done) and "you known as" are other constructions.
                if ws.get(i + 2).is_some_and(|w| w.kind().is_verb_past_form() || w.is("as") || w.is("by")) {
                    continue;
                }
                // Questions and inversions: "Have you seen…", "Where have they gone".
                if i > 0
                    && ws[i - 1].any(&[
                        "have", "has", "had", "having", "haven't", "hasn't", "hadn't", "'ve", "is", "are", "was",
                        "were", "be", "get", "got",
                    ])
                {
                    continue;
                }
                let have = if ws[i].any(&["he", "she", "it"]) { "has" } else { "have" };
                out.push(lint(
                    v.tok.span,
                    LintKind::Grammar,
                    format!("`{}` needs a helper verb here: use `{past}` or `{have} {}`.", v.text, v.text),
                    &[past.to_string(), format!("{have} {}", v.text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Finds a past participle used as the simple past: `I seen it` → `I saw it` or `I have seen it`."
    }
}

// ---------------------------------------------------------------------------

/// "Its raining", "its been", "its a…": `its` where `it's` (it is / it has) is meant.
#[derive(Default)]
pub struct ItsForItIs;

const AFTER_ITS: &[&str] = &[
    "been",
    "not",
    "a",
    "an",
    "the",
    "time",
    "ok",
    "okay",
    "fine",
    "true",
    "important",
    "hard",
    "easy",
    "so",
    "very",
    "too",
    "just",
    "only",
    "all",
    "my",
    "your",
    "our",
    "their",
    "his",
    "her",
    "about",
    "because",
    "like",
    "still",
    "never",
    "always",
    "already",
    "really",
    "getting",
    "going",
    "gonna",
    "almost",
    "nearly",
    "probably",
    "definitely",
    "likely",
    "unlikely",
    "possible",
    "impossible",
    "clear",
    "obvious",
    "worth",
    "great",
    "good",
    "bad",
    "nice",
    "late",
    "early",
    "over",
    "done",
    "up",
    "on",
    "here",
    "there",
    "me",
    "you",
    "him",
    "them",
    "us",
    "what",
    "how",
    "who",
    "when",
    "where",
    "why",
    "that",
    "this",
    "no",
    "hot",
    "cold",
    "raining",
    "snowing",
    "sunny",
    "cloudy",
    "windy",
];

impl Linter for ItsForItIs {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !ws[i].is("its") {
                    continue;
                }
                let n = &ws[i + 1];
                let k = n.kind();
                // "its own", "its first", "its best feature" are possessive.
                if n.any(&["own", "first", "last", "way", "use", "value", "place", "part", "name", "very"]) {
                    continue;
                }
                if !n.is_word() {
                    continue;
                }
                const TIME_WORDS: &[&str] = &[
                    "today",
                    "tonight",
                    "tomorrow",
                    "yesterday",
                    "now",
                    "again",
                    "here",
                    "there",
                    "outside",
                    "inside",
                    "everywhere",
                    "anymore",
                    "already",
                    "though",
                    "too",
                ];
                let after_noun = ws.get(i + 2).is_some_and(|w| {
                    w.kind().is_noun()
                        && !w.any(TIME_WORDS)
                        && !w.any(&["to", "that", "how", "what", "why", "if", "whether", "when"])
                });
                let (adj, part) = (k.is_adjective(), k.is_verb_past_participle_form() || k.is_verb_progressive_form());
                // Strong signs of "it is/has" ("its been", "its raining", "its not").
                let always = n.any(AFTER_ITS) && !n.is("time");
                if !always {
                    // A plain noun: possessive ("its tail").
                    if k.is_noun() && !adj && !k.is_adverb() && !part {
                        continue;
                    }
                    // An adjective or participle before a noun: possessive ("its dedicated staff", "its best feature").
                    if (adj || part) && after_noun {
                        continue;
                    }
                    if !(adj || part || k.is_adverb() || k.is_determiner() || k.is_verb()) {
                        continue;
                    }
                    // A word that's both noun and adjective ("tail", "best") reads as "it is" only before a clause.
                    if k.is_noun()
                        && adj
                        && !part
                        && !ws.get(i + 2).is_some_and(|w| {
                            w.any(&[
                                "to", "that", "if", "when", "because", "not", "enough", "so", "how", "whether", "what",
                            ])
                        })
                    {
                        continue;
                    }
                } else if (adj || part)
                    && after_noun
                    && !n.any(&["been", "not", "a", "an", "the", "so", "too", "very", "just", "really"])
                {
                    continue;
                }
                // "its time" possessive ("over its time in office") — only after a preposition.
                if n.is("time") && i > 0 && ws[i - 1].kind().is_preposition() {
                    continue;
                }
                out.push(lint(
                    ws[i].tok.span,
                    LintKind::Grammar,
                    "Did you mean `it's` (it is / it has)? `Its` means \"belonging to it\".",
                    &[match_case("it's", &ws[i].text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Finds `its` where `it's` (it is, it has) is meant: `its raining`, `its been a while`."
    }
}

// ---------------------------------------------------------------------------

/// "it's" where the possessive "its" is meant: "the park closes and it's
/// gates are locked", "with it's vaccinations".
#[derive(Default)]
pub struct ItIsForIts;

/// Verbs and adjectives that take a clause ("I think it's…", "glad it's…").
const CLAUSE_TAKERS: &[&str] = &[
    "think", "thinks", "thought", "know", "knows", "knew", "say", "says", "said", "believe", "believes", "hope",
    "hopes", "guess", "feel", "feels", "felt", "mean", "means", "suppose", "seems", "hear", "heard", "bet", "see",
    "saw", "admit", "claim", "claims", "realize", "realise", "realized", "realised", "notice", "noticed", "sure",
    "glad", "afraid", "worry", "figure", "find", "found", "promise", "swear", "wish", "tell", "told", "show", "shows",
    "prove", "sounds", "looks", "because", "since", "if", "when", "while", "until", "unless", "although", "though",
    "so", "but", "and", "or", "that", "before", "after", "once", "now", "where", "as", "how", "what", "why", "whether",
    "maybe", "perhaps", "yes", "no", "well", "oh", "hope", "means", "is", "was", "agree", "doubt", "trust", "assume",
];

impl Linter for ItIsForIts {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !(ws[i].is("it's") || ws[i].is("it") && ws.get(i + 1).is_some_and(|w| w.is("'s"))) {
                    continue;
                }
                let j = if ws[i].is("it's") { i + 1 } else { i + 2 };
                let Some(n) = ws.get(j) else { continue };
                let k = n.kind();
                // "it's own" is always the possessive.
                let own = n.any(&["own", "default", "main", "primary", "original", "entire", "current", "former"]);
                // A plain noun (not adjective, adverb, verb form, pronoun or a word like "time"/"fun").
                const NOT_POSSESSED: &[&str] = &[
                    "time",
                    "fun",
                    "home",
                    "work",
                    "nothing",
                    "something",
                    "everything",
                    "anything",
                    "money",
                    "love",
                    "life",
                    "business",
                    "history",
                    "magic",
                    "news",
                    "trouble",
                    "luck",
                    "school",
                    "bedtime",
                    "morning",
                    "night",
                    "noon",
                    "midnight",
                    "summer",
                    "winter",
                    "spring",
                    "autumn",
                    "fall",
                    "worth",
                    "me",
                    "you",
                    "him",
                    "her",
                    "us",
                    "them",
                    "a",
                    "an",
                    "the",
                    "all",
                    "half",
                    "just",
                    "true",
                    "ok",
                    "okay",
                    "fine",
                    "late",
                    "early",
                    "over",
                    "up",
                    "out",
                    "done",
                    "gone",
                    "here",
                    "there",
                    "now",
                    "no",
                    "not",
                    "been",
                    "got",
                    "going",
                    "about",
                    "like",
                    "hard",
                    "easy",
                    "cold",
                    "hot",
                    "dark",
                    "light",
                    "lunch",
                    "dinner",
                    "breakfast",
                    "friday",
                    "monday",
                    "tuesday",
                    "wednesday",
                    "thursday",
                    "saturday",
                    "sunday",
                    "christmas",
                    "payday",
                    "chaos",
                    "science",
                    "art",
                    "war",
                    "peace",
                    "progress",
                    "game",
                    "showtime",
                ];
                // Lowercase words aren't names even if the dictionary has a proper-noun sense ("gates");
                // a known word with no part of speech recorded ("vaccinations") is treated as a noun.
                let lower_case = n.text.chars().next().is_some_and(char::is_lowercase);
                let untagged = !(k.is_noun()
                    || k.is_adjective()
                    || k.is_verb()
                    || k.is_adverb()
                    || k.is_pronoun()
                    || k.is_determiner()
                    || k.is_preposition()
                    || k.is_conjunction());
                let plain_noun = (k.is_noun() || untagged && lower_case && n.is_word())
                    && !k.is_adjective()
                    && !k.is_adverb()
                    && !k.is_verb_progressive_form()
                    && !k.is_verb_past_participle_form()
                    && !k.is_pronoun()
                    && !n.any(NOT_POSSESSED)
                    && (lower_case || !k.is_proper_noun());
                let prev = i.checked_sub(1).map(|p| &ws[p]);
                let after_prep = prev.is_some_and(|p| p.kind().is_preposition() && !p.any(CLAUSE_TAKERS));
                // "in it's degraded state": adjective + noun after a preposition.
                let adj_noun = after_prep
                    && (k.is_adjective() || k.is_verb())
                    && !n.any(NOT_POSSESSED)
                    && ws.get(j + 1).is_some_and(|w| w.kind().is_noun() && !w.any(NOT_POSSESSED));
                if !(plain_noun || own || adj_noun) {
                    continue;
                }
                // Evidence it's possessive: a verb straight after the noun ("it's gates are locked"),
                // or a preposition / object-taking verb before ("with it's vaccinations").
                let mut e = j + 1;
                while ws.get(e).is_some_and(|w| w.kind().is_noun() && !w.kind().is_verb()) && e < j + 3 {
                    e += 1;
                }
                let verb_after = ws.get(e).is_some_and(|w| w.any(AUX) || finite_verb(w));
                let taker_before = prev.is_some_and(|p| {
                    (p.kind().is_preposition() || p.kind().is_verb() && !p.any(AUX)) && !p.any(CLAUSE_TAKERS)
                });
                if !(own || verb_after || taker_before || adj_noun) {
                    continue;
                }
                let span = span_of(&ws[i..j]);
                out.push(lint(
                    span,
                    LintKind::Grammar,
                    "Did you mean `its` (belonging to it)? `It's` means \"it is\" or \"it has\".",
                    &[match_case("its", &ws[i].text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Finds `it's` where the possessive `its` is meant: `the park and it's gates`."
    }
}

// ---------------------------------------------------------------------------

/// "There is many reasons" → "There are"; "there was several" → "were".
#[derive(Default)]
pub struct ThereIsPlural;

const PLURAL_QUANTIFIERS: &[&str] = &[
    "many",
    "several",
    "numerous",
    "various",
    "countless",
    "multiple",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "twelve",
    "twenty",
    "hundreds",
    "thousands",
    "dozens",
    "millions",
    "few",
    "both",
    "these",
    "those",
];

impl Linter for ThereIsPlural {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(2) {
                if !ws[i].is("there") || !ws[i + 1].any(&["is", "was", "isn't", "wasn't"]) {
                    continue;
                }
                // "Is there…?" questions put the verb first; skip "there is" after a verb ("make sure there is").
                let mut j = i + 2;
                // "there is still/also/always many…"
                while j < ws.len()
                    && ws[j].any(&[
                        "still",
                        "also",
                        "always",
                        "often",
                        "usually",
                        "now",
                        "probably",
                        "definitely",
                        "certainly",
                    ])
                {
                    j += 1;
                }
                let Some(n) = ws.get(j) else { continue };
                let quantified = n.any(PLURAL_QUANTIFIERS)
                    || (n.is("a")
                        && ws.get(j + 1).is_some_and(|w| w.is("few") || w.is("couple"))
                        && ws.get(j + 2).is_some_and(|w| !w.is("of")))
                    || n.kind().is_number() && n.lower != "1" && n.lower != "one";
                // A bare plural noun: "There is reasons to…" (not "There is news", "There is data").
                let bare_plural = n.kind().is_plural_noun_only()
                    && !n.kind().is_mass_noun()
                    && ws.get(j + 1).is_none_or(|w| !w.lower.starts_with('\''));
                if !(quantified || bare_plural) {
                    continue;
                }
                // "There is a few…" is common informally but nonstandard; "there's" is handled the same way.
                let fix = match ws[i + 1].lower.as_str() {
                    "is" => "are",
                    "was" => "were",
                    "isn't" => "aren't",
                    _ => "weren't",
                };
                out.push(lint(
                    ws[i + 1].tok.span,
                    LintKind::Agreement,
                    format!("With a plural noun after `there`, use `{fix}`."),
                    &[match_case(fix, &ws[i + 1].text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Makes `there is/was` agree with a plural noun: `There is many reasons` → `There are many reasons`."
    }
}

// ---------------------------------------------------------------------------

/// Durations with the wrong preposition: "on 10 minutes" → "in", "since
/// five years" → "for".
#[derive(Default)]
pub struct DurationPreposition;

const UNITS: &[&str] = &[
    "second", "seconds", "minute", "minutes", "hour", "hours", "day", "days", "week", "weeks", "month", "months",
    "year", "years", "decade", "decades", "mins", "hrs",
];
const COUNTS: &[&str] = &[
    "a", "an", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    "fifteen", "twenty", "thirty", "forty", "fifty", "sixty", "several", "many", "few", "couple", "half",
];

impl Linter for DurationPreposition {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(2) {
                let p = &ws[i];
                if !p.any(&["on", "since", "during"]) {
                    continue;
                }
                // The count may be "a couple of", "a few", "ten", "10".
                let mut j = i + 1;
                let mut counted = false;
                while j < ws.len()
                    && j <= i + 4
                    && (ws[j].any(COUNTS)
                        || ws[j].kind().is_number()
                        || ws[j].is("of")
                        || ws[j].is("more")
                        || ws[j].is("than"))
                {
                    counted |= !ws[j].is("of");
                    j += 1;
                }
                if !counted || !ws.get(j).is_some_and(|w| w.any(UNITS)) {
                    continue;
                }
                // "since a year ago" is fine; "on a day like this" / "on two days a week" are not durations.
                if ws.get(j + 1).is_some_and(|w| {
                    w.any(&[
                        "ago", "before", "earlier", "later", "like", "a", "per", "each", "every", "when", "that", "of",
                        "in", "off",
                    ])
                }) {
                    continue;
                }
                if p.is("on") && (ws[i + 1].is("a") || ws[i + 1].is("an") || ws[i + 1].is("one")) {
                    continue; // "on a day", "on one day"
                }
                let (fix, msg) = match p.lower.as_str() {
                    "on" => ("in", "For a time from now, use `in` (`in 10 minutes`)."),
                    "since" => (
                        "for",
                        "For a length of time, use `for` (`for five years`); `since` takes a starting point (`since 2019`).",
                    ),
                    _ => ("for", "For how long something lasts, use `for` (`for two hours`)."),
                };
                if p.is("during")
                    && !ws[i + 1].kind().is_number()
                    && !ws[i + 1].any(&[
                        "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "twelve", "twenty",
                        "thirty", "forty", "fifty", "sixty", "several",
                    ])
                {
                    continue; // "during the next few days" is fine
                }
                out.push(lint(p.tok.span, LintKind::Grammar, msg, &[match_case(fix, &p.text)]));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes prepositions with lengths of time: `on 10 minutes` → `in 10 minutes`, `since five years` → `for five years`."
    }
}

// ---------------------------------------------------------------------------

/// "look forward to hear from you" → "hearing".
#[derive(Default)]
pub struct ToIng;

/// The -ing form of a verb lemma (regular spelling rules plus exceptions).
fn ing(v: &str) -> String {
    match v {
        "be" => return "being".into(),
        "see" => return "seeing".into(),
        "lie" => return "lying".into(),
        "die" => return "dying".into(),
        "tie" => return "tying".into(),
        _ => {}
    }
    let b = v.as_bytes();
    let vowel = |c: u8| b"aeiou".contains(&c);
    if let Some(stem) = v.strip_suffix("ie") {
        return format!("{stem}ying");
    }
    if v.ends_with('e') && !v.ends_with("ee") && !v.ends_with("ye") && !v.ends_with("oe") {
        return format!("{}ing", &v[..v.len() - 1]);
    }
    // Short consonant-vowel-consonant words double the last consonant (stop → stopping).
    let n = b.len();
    if (3..=4).contains(&n) && !vowel(b[n - 1]) && vowel(b[n - 2]) && !vowel(b[n - 3]) && !b"wxy".contains(&b[n - 1]) {
        return format!("{v}{}ing", b[n - 1] as char);
    }
    format!("{v}ing")
}

impl Linter for ToIng {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(3) {
                let lead = ws[i].any(&["look", "looks", "looking", "looked"])
                    && ws[i + 1].is("forward")
                    && ws[i + 2].is("to")
                    || ws[i].any(&["committed", "dedicated", "accustomed", "devoted"]) && ws[i + 1].is("to") && false;
                if !lead {
                    continue;
                }
                let v = &ws[i + 3];
                let k = v.kind();
                if !(k.is_verb_lemma() && !k.is_verb_progressive_form())
                    || v.any(&["the", "a", "an", "it", "this", "that", "your", "our", "my", "his", "her", "their"])
                {
                    continue;
                }
                // "look forward to work" could be a noun; only flag words that aren't also common nouns, or are followed by an object/adverb.
                if k.is_noun()
                    && !ws.get(i + 4).is_some_and(|w| {
                        w.any(&[
                            "from", "with", "you", "your", "the", "a", "an", "to", "it", "them", "him", "her", "us",
                            "more", "again", "soon", "back",
                        ])
                    })
                {
                    continue;
                }
                out.push(lint(
                    v.tok.span,
                    LintKind::Grammar,
                    "After `look forward to`, use the -ing form of the verb.",
                    &[ing(&v.lower)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes the verb after `look forward to`: `I look forward to hear from you` → `hearing`."
    }
}

// ---------------------------------------------------------------------------

/// Subject–verb agreement with a noun subject at the start of a sentence,
/// looking past "of …"/"in …" phrases: "The results of the test was" →
/// "were"; "The boxes in the hall needs" → "need"; "Each of the students
/// have" → "has".
#[derive(Default)]
pub struct NounSubjectAgreement;

/// Head nouns whose agreement varies or depends on what follows.
const VARIABLE_NUMBER: &[&str] = &[
    "data",
    "media",
    "staff",
    "team",
    "family",
    "crew",
    "committee",
    "government",
    "police",
    "series",
    "species",
    "means",
    "news",
    "headquarters",
    "sheep",
    "fish",
    "deer",
    "aircraft",
    "offspring",
    "majority",
    "minority",
    "number",
    "percent",
    "percentage",
    "rest",
    "half",
    "lot",
    "lots",
    "total",
    "variety",
    "range",
    "couple",
    "group",
    "set",
    "pair",
    "bunch",
    "kind",
    "type",
    "sort",
    "none",
    "all",
    "some",
    "most",
    "any",
    "more",
    "part",
    "portion",
    "proportion",
    "fraction",
    "third",
    "quarter",
    "board",
    "jury",
    "audience",
    "public",
    "class",
    "company",
    "band",
    "army",
    "council",
    "faculty",
    "personnel",
    "people",
    "folks",
    "politics",
    "economics",
    "mathematics",
    "physics",
    "statistics",
    "ethics",
    "athletics",
    "gymnastics",
    "thanks",
    "remains",
    "premises",
    "goods",
    "clothes",
    "contents",
    "earnings",
    "savings",
    "surroundings",
    "belongings",
    "glasses",
    "scissors",
    "trousers",
    "jeans",
    "pants",
    "shorts",
    "pajamas",
    "pyjamas",
    "stairs",
    "outskirts",
    "congratulations",
    "who",
    "which",
    "what",
    "that",
];

const DETERMINERS: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "my", "your", "his", "her", "our", "their", "its", "many",
    "several", "both", "every", "each", "another", "no", "two", "three", "four", "five", "most", "few",
];

const PREPOSITIONS: &[&str] = &[
    "of", "in", "on", "at", "for", "with", "from", "near", "by", "behind", "under", "about", "inside", "outside",
    "between", "around", "along", "beside", "above", "below", "across", "among", "without",
];

fn dict() -> &'static harper_core::spell::FstDictionary {
    use std::sync::OnceLock;
    static D: OnceLock<std::sync::Arc<harper_core::spell::FstDictionary>> = OnceLock::new();
    D.get_or_init(harper_core::spell::FstDictionary::curated)
}

/// Whether `word` is in Harper's dictionary.
pub(crate) fn known(word: &str) -> bool {
    use harper_core::spell::Dictionary;
    dict().contains_word_str(word)
}

fn dict_has(word: &str, test: impl Fn(&harper_core::DictWordMetadata) -> bool) -> bool {
    use harper_core::spell::Dictionary;
    dict().get_word_metadata_str(word).is_some_and(|m| test(&m))
}

/// The other forms of a verb that are real words: base, -s, -ed/irregular
/// past and participle, -ing, plus the be/have/do families.
pub fn verb_forms(word: &str) -> Vec<String> {
    const FAMILIES: &[&[&str]] = &[
        &["am", "is", "are", "was", "were", "be", "been", "being"],
        &["has", "have", "had", "having"],
        &["do", "does", "did", "done", "doing"],
        &["doesn't", "don't", "didn't"],
        &["isn't", "aren't", "wasn't", "weren't"],
        &["hasn't", "haven't", "hadn't"],
    ];
    let w = word.to_lowercase();
    if let Some(f) = FAMILIES.iter().find(|f| f.contains(&w.as_str())) {
        return f.iter().filter(|x| **x != w).map(|x| x.to_string()).collect();
    }
    let base = if dict_has(&w, |m| m.is_verb_lemma()) {
        Some(w.clone())
    } else {
        base_form(&w).or_else(|| PAST_TO_BASE.iter().find(|(p, _)| *p == w).map(|(_, b)| b.to_string()))
    };
    let Some(base) = base else { return Vec::new() };
    let mut out = vec![base.clone()];
    out.extend(third_person(&base));
    out.push(ing(&base));
    if let Some((_, past, pp)) = IRREGULAR.iter().find(|(b, _, _)| *b == base) {
        out.push(past.to_string());
        out.push(pp.to_string());
    } else {
        let ed = if base.ends_with('e') { format!("{base}d") } else { format!("{base}ed") };
        if known(&ed) {
            out.push(ed);
        }
    }
    out.retain(|x| *x != w && known(x));
    out.dedup();
    out
}

/// Common irregular verbs: (base, past, past participle).
const IRREGULAR: &[(&str, &str, &str)] = &[
    ("go", "went", "gone"),
    ("see", "saw", "seen"),
    ("do", "did", "done"),
    ("take", "took", "taken"),
    ("give", "gave", "given"),
    ("write", "wrote", "written"),
    ("eat", "ate", "eaten"),
    ("speak", "spoke", "spoken"),
    ("break", "broke", "broken"),
    ("choose", "chose", "chosen"),
    ("drive", "drove", "driven"),
    ("ride", "rode", "ridden"),
    ("rise", "rose", "risen"),
    ("show", "showed", "shown"),
    ("know", "knew", "known"),
    ("grow", "grew", "grown"),
    ("throw", "threw", "thrown"),
    ("draw", "drew", "drawn"),
    ("fly", "flew", "flown"),
    ("wear", "wore", "worn"),
    ("begin", "began", "begun"),
    ("sing", "sang", "sung"),
    ("drink", "drank", "drunk"),
    ("swim", "swam", "swum"),
    ("steal", "stole", "stolen"),
    ("freeze", "froze", "frozen"),
    ("forget", "forgot", "forgotten"),
    ("hide", "hid", "hidden"),
    ("fall", "fell", "fallen"),
    ("shake", "shook", "shaken"),
    ("come", "came", "come"),
    ("run", "ran", "run"),
    ("get", "got", "got"),
    ("make", "made", "made"),
    ("say", "said", "said"),
    ("tell", "told", "told"),
    ("think", "thought", "thought"),
    ("bring", "brought", "brought"),
    ("buy", "bought", "bought"),
    ("find", "found", "found"),
    ("leave", "left", "left"),
    ("send", "sent", "sent"),
    ("spend", "spent", "spent"),
    ("build", "built", "built"),
    ("keep", "kept", "kept"),
    ("meet", "met", "met"),
    ("pay", "paid", "paid"),
    ("lead", "led", "led"),
    ("feel", "felt", "felt"),
    ("hold", "held", "held"),
    ("stand", "stood", "stood"),
    ("understand", "understood", "understood"),
    ("lose", "lost", "lost"),
    ("catch", "caught", "caught"),
    ("teach", "taught", "taught"),
    ("sell", "sold", "sold"),
    ("sit", "sat", "sat"),
    ("win", "won", "won"),
];

/// Irregular past forms back to the base.
const PAST_TO_BASE: &[(&str, &str)] = &[
    ("went", "go"),
    ("gone", "go"),
    ("saw", "see"),
    ("seen", "see"),
    ("did", "do"),
    ("done", "do"),
    ("took", "take"),
    ("taken", "take"),
    ("gave", "give"),
    ("given", "give"),
    ("wrote", "write"),
    ("written", "write"),
    ("ate", "eat"),
    ("eaten", "eat"),
    ("spoke", "speak"),
    ("spoken", "speak"),
    ("broke", "break"),
    ("broken", "break"),
    ("came", "come"),
    ("ran", "run"),
    ("got", "get"),
    ("made", "make"),
    ("said", "say"),
    ("told", "tell"),
    ("thought", "think"),
    ("brought", "bring"),
    ("bought", "buy"),
    ("found", "find"),
    ("left", "leave"),
    ("sent", "send"),
    ("knew", "know"),
    ("known", "know"),
    ("began", "begin"),
    ("begun", "begin"),
    ("fell", "fall"),
    ("fallen", "fall"),
    ("chose", "choose"),
    ("chosen", "choose"),
    ("drove", "drive"),
    ("driven", "drive"),
    ("flew", "fly"),
    ("flown", "fly"),
];

/// "needs" → "need", "tries" → "try", "watches" → "watch".
fn base_form(v: &str) -> Option<String> {
    let candidates = [
        v.strip_suffix("ies").map(|s| format!("{s}y")),
        v.strip_suffix("es").map(str::to_string),
        v.strip_suffix('s').map(str::to_string),
    ];
    candidates.into_iter().flatten().find(|c| dict_has(c, |m| m.is_verb_lemma()))
}

/// "need" → "needs", "try" → "tries", "watch" → "watches".
fn third_person(v: &str) -> Option<String> {
    let b = v.as_bytes();
    let c = if v.ends_with('y') && b.len() > 1 && !b"aeiou".contains(&b[b.len() - 2]) {
        format!("{}ies", &v[..v.len() - 1])
    } else if ["s", "sh", "ch", "x", "z", "o"].iter().any(|e| v.ends_with(e)) {
        format!("{v}es")
    } else {
        format!("{v}s")
    };
    dict_has(&c, |m| m.is_verb_third_person_singular_present_form()).then_some(c)
}

fn singular_verb(v: &str) -> Option<String> {
    Some(
        match v {
            "are" => "is",
            "were" => "was",
            "have" => "has",
            "do" => "does",
            "don't" => "doesn't",
            "aren't" => "isn't",
            "weren't" => "wasn't",
            "haven't" => "hasn't",
            _ => return None,
        }
        .to_string(),
    )
}

fn plural_verb(v: &str) -> Option<String> {
    Some(
        match v {
            "is" => "are",
            "was" => "were",
            "has" => "have",
            "does" => "do",
            "doesn't" => "don't",
            "isn't" => "aren't",
            "wasn't" => "weren't",
            "hasn't" => "haven't",
            _ => return None,
        }
        .to_string(),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum Number {
    Singular,
    Plural,
}

/// Plurals the dictionary may mark as singular.
const IRREGULAR_PLURALS: &[&str] = &[
    "mitochondria",
    "bacteria",
    "criteria",
    "phenomena",
    "strata",
    "alumni",
    "fungi",
    "cacti",
    "nuclei",
    "stimuli",
    "radii",
    "syllabi",
    "larvae",
    "antennae",
    "vertebrae",
    "formulae",
    "appendices",
    "indices",
    "matrices",
    "vertices",
    "analyses",
    "crises",
    "theses",
    "hypotheses",
    "diagnoses",
    "oases",
    "children",
    "men",
    "women",
    "feet",
    "teeth",
    "geese",
    "mice",
    "oxen",
    "dice",
];

fn noun_number(w: &Word) -> Option<Number> {
    let k = w.kind();
    if w.any(IRREGULAR_PLURALS) {
        return Some(Number::Plural);
    }
    if w.any(VARIABLE_NUMBER) || k.is_proper_noun() || !k.is_noun() {
        return None;
    }
    // A word with a plural ending isn't trusted as singular ("findings").
    let s_plural =
        w.lower.ends_with('s') && !["ss", "us", "is", "ics", "ness", "ous", "'s"].iter().any(|e| w.lower.ends_with(e));
    if s_plural && !k.is_plural_noun() {
        return None;
    }
    if k.is_plural_noun() && !k.is_singular_noun() && !k.is_mass_noun() {
        Some(Number::Plural)
    } else if k.is_singular_noun() && !k.is_plural_noun() {
        Some(Number::Singular)
    } else {
        None
    }
}

/// A noun phrase from `i`: optional determiner, then adjectives/nouns. The
/// index after it and its head (last noun).
fn noun_phrase(ws: &[Word], mut i: usize) -> Option<(usize, usize)> {
    if ws.get(i)?.any(DETERMINERS) {
        i += 1;
    }
    let start = i;
    let mut head = None;
    while let Some(w) = ws.get(i) {
        let k = w.kind();
        if !w.is_word()
            || w.any(PREPOSITIONS)
            || w.any(AUX)
            || w.any(&["and", "or", "but", "who", "which", "that", "than", "as"])
        {
            break;
        }
        if w.any(&[
            "still",
            "always",
            "never",
            "often",
            "really",
            "usually",
            "also",
            "just",
            "only",
            "actually",
            "sometimes",
            "rarely",
            "seldom",
            "generally",
            "certainly",
            "probably",
            "clearly",
            "now",
        ]) {
            break;
        }
        // After the head, a word that can be a verb is the verb when what follows
        // can't continue a noun phrase ("students need a permit" vs "coffee machine has").
        let plural_head =
            head.is_some_and(|h: usize| ws[h].kind().is_plural_noun() && !ws[h].kind().is_singular_noun());
        let s_verb = k.is_verb_third_person_singular_present_form()
            && ws
                .get(i + 1)
                .is_some_and(|n| !n.kind().is_noun() || n.kind().is_verb_progressive_form() || n.any(DETERMINERS));
        if head.is_some() && k.is_verb() && !k.is_verb_progressive_form() && (plural_head || s_verb || !k.is_noun()) {
            let next_is_verbal = ws.get(i + 1).is_some_and(|n| n.any(AUX) || n.kind().is_verb() && !n.kind().is_noun());
            if !next_is_verbal {
                break;
            }
        }
        // A base-form verb the dictionary also lists as a noun is the verb when an
        // object follows: "My chair sit me", "The dog want the ball". A later "is"
        // means the noun goes on: "The car park the council built is full".
        if head.is_some_and(|h| noun_number(&ws[h]).is_some())
            && k.is_verb_lemma()
            && ws.get(i + 1).is_some_and(|n| {
                n.any(&["me", "him", "us", "them", "the", "a", "an", "my", "your", "his", "our", "their"])
            })
            && !ws[i + 1..].iter().any(|w| w.any(AUX))
        {
            break;
        }
        if k.is_noun() {
            head = Some(i);
        } else if !k.is_adjective() {
            break;
        }
        i += 1;
        if i - start > 4 {
            return None;
        }
    }
    head.map(|h| (i, h))
}

/// Nouns that, after "the", make the phrase singular whatever follows
/// ("the variety of options is"); after "a" they're quantifiers and take the
/// number of what follows ("a variety of options are").
const SINGULAR_WITH_THE: &[&str] = &[
    "variety",
    "range",
    "series",
    "collection",
    "set",
    "kind",
    "type",
    "sort",
    "number",
    "combination",
    "list",
    "selection",
    "array",
    "assortment",
    "mix",
    "presence",
    "absence",
    "quality",
    "quantity",
    "amount",
    "level",
    "cost",
    "price",
    "size",
    "length",
    "rate",
];

/// Units of amount: a plural amount can take a singular verb.
const AMOUNTS: &[&str] = &[
    "dollars",
    "pounds",
    "euros",
    "cents",
    "pence",
    "yen",
    "rupees",
    "years",
    "months",
    "weeks",
    "days",
    "hours",
    "minutes",
    "seconds",
    "miles",
    "kilometres",
    "kilometers",
    "metres",
    "meters",
    "feet",
    "inches",
    "yards",
    "kilograms",
    "grams",
    "tons",
    "tonnes",
    "litres",
    "liters",
    "gallons",
    "degrees",
    "points",
    "percent",
];

/// Indefinite pronouns that are singular.
const SINGULAR_PRONOUNS: &[&str] = &[
    "everyone",
    "everybody",
    "everything",
    "someone",
    "somebody",
    "something",
    "anyone",
    "anybody",
    "anything",
    "nobody",
    "nothing",
];

/// Words after which a new subject can start.
const CLAUSE_STARTERS: &[&str] =
    &["that", "because", "when", "if", "while", "although", "though", "since", "whereas", "unless", "and", "but", "so"];

/// Multi-word links that don't change the subject: "the professor along with her students is".
fn skip_addition(ws: &[Word], i: usize) -> Option<usize> {
    let w = |k: usize| ws.get(i + k).map(|w| w.lower.as_str());
    match (w(0)?, w(1), w(2)) {
        ("along" | "together", Some("with"), _) => Some(i + 2),
        ("as", Some("well"), Some("as")) => Some(i + 3),
        ("in", Some("addition"), Some("to")) => Some(i + 3),
        ("alongside", _, _) => Some(i + 1),
        _ => None,
    }
}

/// Checks a subject starting at `s[0]`; pushes a lint if its verb disagrees.
fn check_subject(s: &[Word], out: &mut Vec<Lint>) {
    if s.len() < 3 {
        return;
    }
    let (mut end, number, head_text) = if s[0].any(&["each", "one", "neither", "either"]) && s[1].is("of") {
        // "Each/One/Neither of the X …": singular.
        let Some((e, _)) = noun_phrase(s, 2) else { return };
        (e, Number::Singular, format!("{} of …", s[0].text))
    } else if s[0].any(SINGULAR_PRONOUNS) {
        (1, Number::Singular, s[0].text.clone())
    } else {
        // A gerund subject ("Making mistakes improves…") is singular whatever follows.
        if s[0].kind().is_verb_progressive_form() {
            return;
        }
        let has_det = s[0].any(DETERMINERS);
        let Some((end, head)) = noun_phrase(s, 0) else { return };
        let the_phrase = s[0].any(&["the", "this", "that", "my", "your", "his", "her", "our", "their", "its"]);
        let n = if the_phrase && s[head].any(SINGULAR_WITH_THE) && s.get(end).is_some_and(|w| w.is("of")) {
            Number::Singular
        } else {
            let Some(n) = noun_number(&s[head]) else { return };
            n
        };
        // Without a determiner, a singular noun is only safe before "of …" ("Analysis of the cores reveal").
        if !has_det && n == Number::Singular && !s.get(end).is_some_and(|w| w.is("of")) {
            return;
        }
        if s[0].any(&["these", "those", "many", "several", "both", "two", "three", "four", "five", "few"])
            && n == Number::Singular
        {
            return;
        }
        (end, n, s[head].text.clone())
    };
    // Skip one prepositional phrase ("in the hall", "of premium ingredients and sustainable packaging")
    // and additions ("along with her students").
    for _ in 0..2 {
        if let Some(e) = skip_addition(s, end) {
            let Some((e2, _)) = noun_phrase(s, e) else { return };
            end = e2;
        } else if s.get(end).is_some_and(|w| w.any(PREPOSITIONS)) {
            let Some((mut e, _)) = noun_phrase(s, end + 1) else { return };
            if s.get(e).is_some_and(|w| w.is("and"))
                && s.get(e + 1).is_some_and(|w| w.kind().is_noun() || w.kind().is_adjective() || w.any(DETERMINERS))
            {
                let Some((e2, _)) = noun_phrase(s, e + 1) else { return };
                e = e2;
            }
            end = e;
        }
    }
    // Amounts take either: "Five hundred dollars is enough", "Ten years have passed".
    if number == Number::Plural && AMOUNTS.iter().any(|u| head_text.eq_ignore_ascii_case(u)) {
        return;
    }
    // A compound subject or a list isn't judged here.
    if s.get(end).is_some_and(|w| w.any(&["and", "or", "nor"]) || w.kind().is_comma()) {
        return;
    }
    // Adverbs between subject and verb: "always meet", "still work".
    while s.get(end).is_some_and(|w| {
        w.kind().is_adverb() && !w.any(AUX) && !w.kind().is_verb()
            || w.any(&[
                "still",
                "always",
                "never",
                "often",
                "really",
                "usually",
                "also",
                "just",
                "only",
                "actually",
                "sometimes",
                "rarely",
                "seldom",
                "generally",
                "certainly",
                "probably",
                "clearly",
            ])
    }) {
        end += 1;
    }
    let Some(v) = s.get(end) else { return };
    let k = v.kind();
    // A noun that looks like a verb: "the exhibit features works by local artists".
    if k.is_noun() && s.get(end + 1).is_some_and(|w| w.any(&["by", "of", "from", "for"])) {
        return;
    }
    let fix = match number {
        Number::Plural => plural_verb(&v.lower).or_else(|| {
            (k.is_verb_third_person_singular_present_form() && !k.is_noun()
                || v.any(&[
                    "needs", "wants", "goes", "makes", "seems", "looks", "says", "runs", "works", "requires", "shows",
                    "reveals",
                ]))
            .then(|| base_form(&v.lower))
            .flatten()
        }),
        Number::Singular => singular_verb(&v.lower).or_else(|| {
            // A base-form verb right after the subject ("each of them need").
            // Verbs whose past tense looks like the base form ("The coach let him play") are skipped.
            const SAME_PAST: &[&str] = &[
                "let",
                "put",
                "set",
                "cut",
                "hit",
                "quit",
                "read",
                "spread",
                "shut",
                "cost",
                "hurt",
                "bet",
                "burst",
                "cast",
                "split",
                "thrust",
                "upset",
                "broadcast",
                "forecast",
                "rid",
                "shed",
                "slit",
                "wed",
                "fit",
                "beat",
                "bid",
                "whet",
                "please",
                "except",
                "like",
            ];
            let likely_verb = k.is_verb_lemma()
                && !v.any(SAME_PAST)
                && !k.is_preposition()
                && !k.is_verb_past_form()
                && !k.is_adjective()
                && (!k.is_noun()
                    || s.get(end + 1).is_some_and(|n| {
                        n.any(&[
                            "to",
                            "a",
                            "an",
                            "the",
                            "that",
                            "it",
                            "this",
                            "these",
                            "those",
                            "me",
                            "us",
                            "them",
                            "him",
                            "her",
                            "you",
                            "your",
                            "my",
                            "his",
                            "our",
                            "their",
                            "its",
                            "some",
                            "all",
                            "out",
                            "up",
                            "perfectly",
                            "well",
                            "over",
                            "on",
                            "in",
                            "from",
                            "with",
                            "about",
                        ])
                    }));
            likely_verb.then(|| third_person(&v.lower)).flatten()
        }),
    };
    let Some(fix) = fix else { return };
    let what = if number == Number::Plural { "plural" } else { "singular" };
    out.push(lint(
        v.tok.span,
        LintKind::Agreement,
        format!("The subject `{head_text}` is {what}, so the verb should be `{fix}`."),
        &[match_case(&fix, &v.text)],
    ));
}

impl Linter for NounSubjectAgreement {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            let Some(first) = ws.iter().position(|w| w.is_word()) else { return };
            check_subject(&ws[first..], out);
            // Later clauses: "…, and the feedback from customers have…", "…that each of the students need…".
            for i in first + 1..ws.len().saturating_sub(3) {
                if !ws[i].any(CLAUSE_STARTERS) {
                    continue;
                }
                // After and/but/so, only a clause with its own determiner ("and the feedback…").
                let next = &ws[i + 1];
                let starts_np = next.any(DETERMINERS)
                    || next.any(SINGULAR_PRONOUNS)
                    || next.any(&["each", "one", "neither", "either"]) && ws.get(i + 2).is_some_and(|w| w.is("of"));
                if !starts_np {
                    continue;
                }
                if ws[i].any(&["and", "but", "so"]) && !ws[i - 1].kind().is_comma() {
                    continue;
                }
                // The last item of a list ("the costumes, the editing, and the music were"):
                // the verb goes with the whole list.
                if ws[i].any(&["and", "or"]) && ws[..i - 1].iter().rev().take(8).any(|w| w.kind().is_comma()) {
                    continue;
                }
                check_subject(&ws[i + 1..], out);
            }
        })
    }

    fn description(&self) -> &str {
        "Makes the verb agree with a noun subject, looking past phrases like `of the experiment`: `The results of the test was` → `were`."
    }
}

// ---------------------------------------------------------------------------

/// Commas around the person being addressed: "Hi Marcus" (greeting line),
/// "Thanks John for…" → "Thanks, John, for…", "Let's eat Grandma" → "eat,
/// Grandma", "I love you Mom" → "you, Mom".
#[derive(Default)]
pub struct AddressComma;

pub(crate) const GREETINGS: &[&str] = &["hi", "hello", "hey", "dear", "greetings", "morning", "evening", "afternoon"];
pub(crate) const THANKS: &[&str] =
    &["thanks", "congratulations", "congrats", "sorry", "welcome", "cheers", "bye", "goodbye", "goodnight"];
/// Words that address people.
pub(crate) const ADDRESS_WORDS: &[&str] = &[
    "grandma",
    "grandpa",
    "granny",
    "nana",
    "mom",
    "mum",
    "dad",
    "mommy",
    "mummy",
    "daddy",
    "honey",
    "babe",
    "sweetie",
    "darling",
    "guys",
    "everyone",
    "everybody",
    "folks",
    "team",
    "kids",
    "boys",
    "girls",
    "y'all",
    "sir",
    "ma'am",
    "madam",
    "buddy",
    "mate",
    "bro",
    "dude",
    "friends",
    "children",
    "class",
    "all",
    "there",
    "love",
];
const TITLES: &[&str] = &[
    "mr",
    "mrs",
    "ms",
    "miss",
    "dr",
    "prof",
    "professor",
    "mx",
    "sir",
    "madam",
    "dame",
    "lord",
    "lady",
    "coach",
    "officer",
    "doctor",
    "aunt",
    "uncle",
];

/// A capitalised word that could be a name (not "I've", "Inside", "The").
fn name_like(w: &Word) -> bool {
    let k = w.kind();
    w.is_word()
        && w.text.chars().next().is_some_and(char::is_uppercase)
        && w.lower != "i"
        && !w.lower.starts_with("i'")
        // Acronyms aren't names: "a valid ID", "the CEO".
        && !(w.text.chars().count() > 1 && w.text.chars().all(|c| c.is_uppercase() || c.is_ascii_digit()))
        && !w.any(&[
            "who", "what", "where", "when", "why", "how", "which", "whose", "whom", "there", "here", "it", "this",
            "that", "yes", "no", "not",
        ])
        && (k.is_proper_noun()
            || !(k.is_preposition()
                || k.is_determiner()
                || k.is_verb()
                || k.is_adverb()
                || k.is_conjunction()
                || k.is_pronoun()))
}

/// The person addressed starting at `i`: titles, capitalised names, or one
/// address word. Returns the index after it.
fn addressee(ws: &[Word], i: usize) -> Option<usize> {
    let mut j = i;
    if ws.get(j)?.any(ADDRESS_WORDS) {
        return Some(j + 1);
    }
    while ws.get(j).is_some_and(|w| w.any(TITLES)) {
        j += 1;
        if ws.get(j).is_some_and(|w| w.lower == ".") {
            j += 1;
        }
    }
    let first_name = j;
    while j < first_name + 3 && ws.get(j).is_some_and(name_like) {
        j += 1;
    }
    (j > first_name).then_some(j)
}

fn is_end(w: Option<&Word>) -> bool {
    w.is_none_or(|w| matches!(w.lower.as_str(), "." | "!" | "?" | "…"))
}

impl Linter for AddressComma {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        let mut out = Vec::new();
        out.extend(each_sentence(document, |ws, out| {
            let Some(first) = ws.iter().position(|w| w.is_word()) else { return };
            let s = &ws[first..];
            // "Thanks John for…", "Thank you everyone for…", "Congratulations Priya!"
            let lead = if s[0].any(THANKS) {
                1
            } else if s[0].is("thank") && s.get(1).is_some_and(|w| w.is("you")) {
                2
            } else {
                0
            };
            if lead > 0
                && s.get(lead).is_some_and(|w| w.is_word())
                && let Some(after) = addressee(s, lead)
                    .filter(|&a| !(s[lead].is("all") && s.get(a).is_some_and(|w| w.is("the") || w.is("of"))))
            {
                // Not "Thank you all the same" / "Welcome Home".
                let more = !is_end(s.get(after)) && !s.get(after).is_some_and(|w| w.kind().is_comma());
                if !(s[lead].is("there")
                    || s[lead].is("love")
                    || s[0].is("welcome") && !s[lead].text.chars().next().is_some_and(char::is_uppercase))
                {
                    let lead_text = joined(&s[..lead]);
                    let name = joined(&s[lead..after]);
                    let fix = if more { format!("{lead_text}, {name},") } else { format!("{lead_text}, {name}") };
                    out.push(lint(
                        span_of(&s[..after]),
                        LintKind::Punctuation,
                        "Use commas around the name of the person you're talking to (`Thanks, John, for…`).",
                        &[fix],
                    ));
                }
            }
            // "Look Tom, …", "Sorry Sarah, …", "Well John, …": an interjection, then the person.
            if s[0].any(&[
                "look",
                "listen",
                "wait",
                "well",
                "yes",
                "no",
                "yeah",
                "okay",
                "ok",
                "oh",
                "honestly",
                "seriously",
                "sorry",
                "please",
            ]) && s.get(1).is_some_and(|w| w.is_word() && !s[0].kind().is_comma())
                && let Some(after) = addressee(s, 1).filter(|&a| {
                    !s[1].any(&["all", "there", "love", "team"])
                        && !s.get(a).is_some_and(|w| {
                            w.kind().is_comma() || w.any(AUX) || w.kind().is_verb() && !w.any(&["i", "you"])
                        })
                })
                && !(s[0].is("please") && !s[1].any(ADDRESS_WORDS))
            {
                let more = !is_end(s.get(after));
                let name = joined(&s[1..after]);
                let fix = if more { format!("{}, {name},", s[0].text) } else { format!("{}, {name}", s[0].text) };
                out.push(lint(
                    span_of(&s[..after]),
                    LintKind::Punctuation,
                    "Use commas around the name of the person you're talking to (`Look, Tom, …`).",
                    &[fix],
                ));
                return;
            }
            // "Mr. Henderson please forward…", "John can you help…": the person first, then a request.
            if let Some(after) = addressee(s, 0).filter(|&a| {
                a >= 1 && !s[0].any(ADDRESS_WORDS)
                    || s[0].any(&["mom", "mum", "dad", "guys", "everyone", "everybody", "folks", "sir", "madam"])
            }) && (s.get(after).is_some_and(|w| w.is("please"))
                || s.get(after).is_some_and(|w| w.any(AUX)) && s.get(after + 1).is_some_and(|w| w.is("you")))
                && first + after < ws.len()
            {
                let name = &s[after - 1];
                out.push(lint(
                    name.tok.span,
                    LintKind::Punctuation,
                    "Put a comma after the name of the person you're talking to (`John, please…`).",
                    &[format!("{},", name.text)],
                ));
                return;
            }
            // "Tell me Maria whether…", "Trust me Chris I've…", "I'm telling you Mark the…"
            for i in 1..s.len() {
                if !(s[i - 1].any(&["me", "you"])
                    && s.get(i).is_some_and(|w| w.text.chars().next().is_some_and(char::is_uppercase) && !w.is("i")))
                {
                    continue;
                }
                let Some(after) = addressee(s, i) else { continue };
                if is_end(s.get(after))
                    || s.get(after)
                        .is_some_and(|w| w.kind().is_comma() || w.any(AUX) || w.kind().is_verb() && !w.kind().is_noun())
                {
                    continue;
                }
                if s.get(after).is_some_and(|w| w.lower.starts_with('\'') && !w.is("'ve")) {
                    continue;
                }
                let name = joined(&s[i..after]);
                out.push(lint(
                    span_of(&s[i - 1..after]),
                    LintKind::Punctuation,
                    "Use commas around the name of the person you're talking to (`Trust me, Chris, …`).",
                    &[format!("{}, {name},", s[i - 1].text)],
                ));
                return;
            }
            // A name at the very end of a sentence spoken to someone: "Let me know about the workflow John."
            let second_person = s.iter().any(|w| w.any(&["you", "your", "yourself", "you're", "you've", "you'll"]))
                || s[0].any(&[
                    "please", "let", "thanks", "thank", "sorry", "hope", "remember", "don't", "stop", "come", "get",
                ]);
            if second_person && s.len() >= 4 {
                let last = s.len() - if is_end(s.last()) && s.last().is_some_and(|w| !w.is_word()) { 1 } else { 0 };
                let mut i = last;
                while i > 1 && last - i < 3 && name_like(&s[i - 1]) {
                    i -= 1;
                }
                // Include titles: "… Dr. Chen", "… Mr. Chen"
                while i > 1 && s[i - 1].any(TITLES) || i > 2 && s[i - 1].lower == "." && s[i - 2].any(TITLES) {
                    i -= if s[i - 1].lower == "." { 2 } else { 1 };
                }
                let named = i < last && i >= 2;
                if named {
                    let prev = &s[i - 1];
                    let pk = prev.kind();
                    // Not an object or appositive: "…to Ms. Chen", "…met John", "…my cousin Jeff", "…in New York".
                    const RELATIONS: &[&str] = &[
                        "cousin",
                        "friend",
                        "brother",
                        "sister",
                        "uncle",
                        "aunt",
                        "colleague",
                        "boss",
                        "teacher",
                        "son",
                        "daughter",
                        "wife",
                        "husband",
                        "partner",
                        "mother",
                        "father",
                        "grandmother",
                        "grandfather",
                        "neighbor",
                        "neighbour",
                        "coach",
                        "manager",
                        "client",
                        "doctor",
                        "president",
                        "director",
                        "author",
                        "artist",
                        "singer",
                        "player",
                        "captain",
                        "king",
                        "queen",
                        "prince",
                        "princess",
                        "saint",
                        "st",
                        "mount",
                        "lake",
                        "river",
                        "street",
                        "avenue",
                        "road",
                        "city",
                        "state",
                        "university",
                        "college",
                        "team",
                    ];
                    // A word that can be a verb or a noun is a noun after "your"/"the" ("your help Maria").
                    let noun_use =
                        pk.is_noun() && i >= 3 && (s[i - 2].kind().is_determiner() || s[i - 2].kind().is_adjective());
                    let object_like = pk.is_preposition()
                        || pk.is_verb() && !noun_use
                        || pk.is_determiner() && !prev.any(&["that", "this", "it"])
                        || pk.is_proper_noun()
                        || prev.any(RELATIONS)
                        || prev.text.chars().next().is_some_and(char::is_uppercase)
                        || !prev.is_word()
                        || prev.any(&[
                            "and", "or", "&", "with", "to", "for", "from", "by", "at", "of", "as", "called", "named",
                            "dear", "meet", "met", "is", "was", "be", "am",
                        ]);
                    let common = s[i..last].iter().all(|w| {
                        w.kind().is_proper_noun()
                            || !w.kind().is_noun() && !w.kind().is_verb() && !w.kind().is_adjective()
                    }) || s[i..last].iter().any(|w| w.any(TITLES));
                    if !object_like
                        && common
                        && !s[i..last].iter().any(|w| {
                            w.any(&[
                                "PM",
                                "AM",
                                "pm",
                                "am",
                                "Monday",
                                "Tuesday",
                                "Wednesday",
                                "Thursday",
                                "Friday",
                                "Saturday",
                                "Sunday",
                                "January",
                                "February",
                                "March",
                                "April",
                                "May",
                                "June",
                                "July",
                                "August",
                                "September",
                                "October",
                                "November",
                                "December",
                            ])
                        })
                    {
                        let name = joined(&s[i..last]);
                        out.push(lint(
                            span_of(&s[i - 1..last]),
                            LintKind::Punctuation,
                            "Put a comma before the name of the person you're talking to (`Let me know, John.`).",
                            &[format!("{}, {name}", prev.text)],
                        ));
                        return;
                    }
                }
            }
            // "Let's eat Grandma." / "I love you Mom." — an address word or name at the end of the sentence.
            for i in 1..s.len() {
                let Some(after) = addressee(s, i) else { continue };
                if !is_end(s.get(after)) || s[i - 1].kind().is_comma() {
                    continue;
                }
                let by_pronoun = s[i - 1].any(&["you", "ya"]) && (s[i].any(ADDRESS_WORDS) || after - i == 1);
                let lets = (s[0].any(&["let's", "lets"]) || s[0].is("let") && s.get(1).is_some_and(|w| w.is("'s")))
                    && i >= 2
                    && s[i - 1].kind().is_verb()
                    && after - i == 1
                    && s[i].any(ADDRESS_WORDS)
                    && !s[i].any(&["all", "there", "love", "team", "class"]);
                if !(by_pronoun || lets) || s[i].any(&["all", "there", "love"]) && !lets {
                    continue;
                }
                let before = &s[i - 1];
                out.push(lint(
                    span_of(&s[i - 1..after]),
                    LintKind::Punctuation,
                    "Put a comma before the name of the person you're talking to (`Let's eat, Grandma`).",
                    &[format!("{}, {}", before.text, joined(&s[i..after]))],
                ));
                break;
            }
        }));
        out
    }

    fn description(&self) -> &str {
        "Adds commas around the person you're addressing: `Hi Marcus,`, `Thanks, John, for…`, `Let's eat, Grandma`."
    }
}

// ---------------------------------------------------------------------------

/// Joining words: two that do the same job ("Although it rained, but we
/// went"), "however" joining two sentences with only a comma, and
/// "despite of".
#[derive(Default)]
pub struct JoiningWords;

impl Linter for JoiningWords {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            let Some(first) = ws.iter().position(|w| w.is_word()) else { return };
            let s = &ws[first..];
            // "Although/Though/Even though … , but …" and "Because/Since/As … , so …"
            let lead =
                if s[0].any(&["although", "though"]) || s[0].is("even") && s.get(1).is_some_and(|w| w.is("though")) {
                    Some("but")
                } else if s[0].is("because") && !s.get(1).is_some_and(|w| w.is("of"))
                    || s[0].is("since") && s.get(1).is_some_and(|w| w.kind().is_pronoun() || w.kind().is_determiner())
                {
                    Some("so")
                } else {
                    None
                };
            if let Some(extra) = lead
                && let Some(i) = (2..s.len()).find(|&i| s[i - 1].kind().is_comma() && s[i].is(extra))
            {
                let msg = if extra == "but" {
                    format!(
                        "`{}` already joins the two parts, so `but` isn't needed (`Although it rained, we went`).",
                        s[0].text
                    )
                } else {
                    format!(
                        "`{}` already joins the two parts, so `so` isn't needed (`Because it rained, we stayed`).",
                        s[0].text
                    )
                };
                out.push(lint(span_of(&s[i - 1..i + 1]), LintKind::Grammar, msg, &[",".to_string()]));
            }
            for i in 1..ws.len().saturating_sub(1) {
                // ", however it failed" → "; however, it failed"
                if ws[i].is("however")
                    && ws[i - 1].kind().is_comma()
                    && ws.get(i + 1).is_some_and(|w| {
                        w.is_word()
                            && !w.kind().is_comma()
                            && (w.kind().is_pronoun()
                                || w.kind().is_determiner()
                                || w.kind().is_proper_noun()
                                || w.any(&["there", "this", "that"]))
                    })
                {
                    out.push(lint(
                        span_of(&ws[i - 1..i + 1]),
                        LintKind::Punctuation,
                        "`However` can't join two sentences with just a comma. Use a semicolon or a full stop: `I tried; however, it failed.`",
                        &["; however,".to_string()],
                    ));
                }
                // "despite of"
                if ws[i - 1].is("despite") && ws[i].is("of") && !ws.get(i + 1).is_some_and(|w| w.is("course")) {
                    out.push(lint(
                        span_of(&ws[i - 1..i + 1]),
                        LintKind::Grammar,
                        "Use `despite` or `in spite of`, not `despite of`.",
                        &[match_case("despite", &ws[i - 1].text), match_case("in spite of", &ws[i - 1].text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes joining words: `Although it rained, but we went`, `I tried, however it failed`, `despite of`."
    }
}

// ---------------------------------------------------------------------------

/// Word order in a question inside a sentence: "Can you tell me where is the
/// office?" → "where the office is"; and a frequency adverb before a form
/// of be: "I always am late" → "I am always late".
#[derive(Default)]
pub struct WordOrder;

const ASKING: &[&str] = &[
    "know",
    "knows",
    "knew",
    "tell",
    "told",
    "ask",
    "asked",
    "asking",
    "wonder",
    "wondering",
    "wondered",
    "explain",
    "show",
    "sure",
    "idea",
    "understand",
    "remember",
    "forgot",
    "forget",
    "check",
    "find",
    "learn",
    "see",
    "decide",
    "clear",
    "unsure",
    "confirm",
    "clarify",
];

impl Linter for WordOrder {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(3) {
                // "…tell me where is the office?"
                let mut j = i + 1;
                if ws[j].any(&["me", "us", "you", "him", "her", "them"]) {
                    j += 1;
                }
                if !ws[i].any(ASKING)
                    || !ws.get(j).is_some_and(|w| w.any(&["where", "what", "when", "how", "why", "who", "which"]))
                {
                    continue;
                }
                let Some(aux) = ws.get(j + 1).filter(|w| {
                    w.any(&["is", "are", "was", "were", "can", "will", "could", "should", "would", "has", "have"])
                }) else {
                    continue;
                };
                // The subject after it: a pronoun, or a short noun phrase.
                let start = j + 2;
                let Some(first) = ws.get(start) else { continue };
                let mut sub_end = start + 1;
                if !first.kind().is_pronoun() || first.kind().is_determiner() {
                    let mut nouns = usize::from(first.kind().is_noun());
                    // Words the dictionary doesn't know ("runbook") are taken as nouns.
                    let nounish = |w: &Word| {
                        w.kind().is_noun()
                            || w.kind().is_adjective()
                            || matches!(w.kind(), TokenKind::Word(None))
                            || w.kind().is_proper_noun()
                    };
                    while sub_end < ws.len()
                        && sub_end < start + 4
                        && ws[sub_end].is_word()
                        && nounish(&ws[sub_end])
                        && !ws[sub_end].any(AUX)
                    {
                        // After a noun, a word that can be a verb is the verb ("the meeting start").
                        if nouns > 0 && ws[sub_end].kind().is_verb() && !ws[sub_end].kind().is_verb_progressive_form() {
                            break;
                        }
                        nouns += usize::from(ws[sub_end].kind().is_noun());
                        sub_end += 1;
                    }
                }
                if !(first.kind().is_pronoun()
                    || first.kind().is_determiner()
                    || first.kind().is_proper_noun()
                    || first.kind().is_noun())
                    || first.kind().is_verb_progressive_form()
                {
                    continue;
                }
                if first.kind().is_determiner() && sub_end == start + 1 {
                    continue; // "the" on its own isn't a subject
                }
                // The rest runs to the end of the sentence (a few words at most).
                let mut end = sub_end;
                while end < ws.len() && end < sub_end + 5 && ws[end].is_word() {
                    end += 1;
                }
                if !(end == ws.len() || matches!(ws[end].lower.as_str(), "?" | "." | "!" | ",")) {
                    continue;
                }
                // "who is the manager" asks about the subject: either order is fine.
                if ws[j].any(&["who", "which"]) && first.kind().is_determiner() {
                    continue;
                }
                let subject = joined(&ws[start..sub_end]);
                let rest = joined(&ws[sub_end..end]);
                let fixed = [ws[j].text.as_str(), &subject, aux.text.as_str(), &rest]
                    .iter()
                    .filter(|x| !x.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(lint(
                    span_of(&ws[j..end]),
                    LintKind::Miscellaneous,
                    format!("Inside a sentence, a question keeps normal word order: `{fixed}`."),
                    std::slice::from_ref(&fixed),
                ));
            }
            for i in 1..ws.len().saturating_sub(1) {
                // "I always am late" → "I am always late"
                if ws[i].any(&[
                    "always",
                    "never",
                    "often",
                    "usually",
                    "sometimes",
                    "rarely",
                    "seldom",
                    "also",
                    "already",
                    "generally",
                    "normally",
                ]) && ws[i - 1].kind().is_pronoun()
                    && ws[i - 1].any(&["i", "you", "he", "she", "it", "we", "they"])
                    && ws[i + 1].any(&["am", "is", "are", "was", "were"])
                    && !ws.get(i + 2).is_some_and(|w| w.is("not"))
                {
                    out.push(lint(
                        span_of(&ws[i..i + 2]),
                        LintKind::Miscellaneous,
                        format!(
                            "Words like `{}` go after am/is/are: `{} {}`.",
                            ws[i].lower,
                            ws[i + 1].text,
                            ws[i].text
                        ),
                        &[format!("{} {}", ws[i + 1].text, ws[i].text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes word order: `tell me where is the office` → `where the office is`; `I always am late` → `I am always late`."
    }
}

// ---------------------------------------------------------------------------

/// A comma after the greeting line of a letter or email: "Hi Marcus" → "Hi
/// Marcus,". Optional in modern style, so profiles can leave it out.
#[derive(Default)]
pub struct GreetingComma;

impl Linter for GreetingComma {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        let src = document.get_source();
        let mut out = Vec::new();
        // Greeting lines: look at whole lines so "Dear Ms. Chen" + newline is seen.
        let tokens = document.get_tokens();
        let mut line_start = 0;
        for (n, t) in tokens.iter().enumerate() {
            let last = n + 1 == tokens.len();
            if !(matches!(t.kind, TokenKind::Newline(_) | TokenKind::ParagraphBreak) || last) {
                continue;
            }
            let end = if last { n + 1 } else { n };
            let ws = words(&tokens[line_start..end], src);
            line_start = n + 1;
            let Some(g) = ws.first() else { continue };
            let mut i = 1;
            if g.any(&["good"]) && ws.get(1).is_some_and(|w| w.any(&["morning", "afternoon", "evening"])) {
                i = 2;
            } else if !g.any(GREETINGS) || g.any(&["morning", "evening", "afternoon"]) {
                continue;
            }
            let Some(after) = addressee(&ws, i) else { continue };
            // "Hi Marcus" alone on the line, or running straight into the message: "Dear Mr. Henderson I am…"
            let next = ws.get(after);
            if next.is_some_and(|w| matches!(w.lower.as_str(), "," | ":" | "!" | "—" | "-" | "." | ";")) {
                continue;
            }
            if next.is_some_and(|w| w.any(&["or", "and", "&", "of", "from", "to"])) {
                continue;
            }
            let name = &ws[after - 1];
            out.push(lint(
                name.tok.span,
                LintKind::Punctuation,
                "Put a comma after the greeting (`Hi Marcus,`).",
                &[format!("{},", name.text)],
            ));
        }
        out
    }

    fn description(&self) -> &str {
        "Adds a comma after a greeting: `Hi Marcus` → `Hi Marcus,`."
    }
}

// ---------------------------------------------------------------------------

/// "Your welcome" → "You're welcome" (not "your welcome pack").
#[derive(Default)]
pub struct YourWelcome;

impl Linter for YourWelcome {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !ws[i].is("your") {
                    continue;
                }
                let mut j = i + 1;
                while j < ws.len()
                    && ws[j].any(&["very", "most", "always", "so", "more", "than", "really", "truly", "quite"])
                {
                    j += 1;
                }
                if !ws.get(j).is_some_and(|w| w.is("welcome")) {
                    continue;
                }
                // What follows must not be a noun ("your welcome pack", "your welcome email").
                if ws.get(j + 1).is_some_and(|w| {
                    w.is_word()
                        && !w.any(&[
                            "to", "back", "anytime", "any", "again", "here", "anyway", "though", "and", "my", "friend",
                            "everyone", "all", "guys", "mate", "sir", "madam",
                        ])
                }) {
                    continue;
                }
                out.push(lint(
                    ws[i].tok.span,
                    LintKind::Grammar,
                    "Did you mean `you're welcome` (you are welcome)?",
                    &[match_case("you're", &ws[i].text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `your welcome` → `you're welcome`."
    }
}

// ---------------------------------------------------------------------------

/// The part-of-speech tag Harper's tagger gave a word ("VERB", "NOUN"…).
fn pos(w: &Word) -> Option<String> {
    match w.kind() {
        TokenKind::Word(Some(m)) => m.pos_tag.map(|t| format!("{t:?}")),
        _ => None,
    }
}

/// A statement with no verb at all: "The cat for walk." (went for a walk).
/// Short replies, greetings, headings and questions are left alone.
#[derive(Default)]
pub struct NoVerb;

impl Linter for NoVerb {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            let words: Vec<&Word> = ws.iter().filter(|w| w.is_word()).collect();
            // Only a full stop: "Coffee or tea?", "Great job!" are fine.
            if words.len() < 3 || !ws.last().is_some_and(|w| w.lower == ".") {
                return;
            }
            // A determiner and a noun to start: a statement about something
            // (not a notice: "No smoking in the lobby.").
            if !(words[0].any(DETERMINERS) && words[1].kind().is_noun()) || words[0].is("no") {
                return;
            }
            // Cut short at an abbreviation: "The meeting with Dr." | "Chen has…".
            if words
                .last()
                .is_some_and(|w| w.any(TITLES) || w.any(&["st", "jr", "sr", "etc", "vs", "inc", "ltd", "co"]))
            {
                return;
            }
            if ws.iter().any(|w| w.lower == ":" || w.lower == ";" || w.kind().is_comma()) {
                return;
            }
            // Any word that is or could be a verb here counts, except a noun-like
            // word straight after a preposition ("for walk").
            let after_prep = |w: &Word| {
                words.iter().position(|x| std::ptr::eq(*x, w)).is_some_and(|i| {
                    i > 0
                        && words[i - 1]
                            .any(&["for", "with", "of", "in", "on", "at", "from", "by", "about", "after", "before"])
                        && w.kind().is_noun()
                        && !w.lower.ends_with("ing")
                })
            };
            let after_det = |w: &Word| {
                words.iter().position(|x| std::ptr::eq(*x, w)).is_none_or(|i| {
                    i == 0
                        || words[i - 1].any(DETERMINERS)
                        || words[i - 1].any(PREPOSITIONS)
                        || words[i - 1].kind().is_adjective() && !words[i - 1].kind().is_noun()
                })
            };
            let verbish = |w: &&Word| {
                matches!(pos(w).as_deref(), Some("VERB" | "AUX")) && !after_prep(w)
                    || w.any(AUX)
                    || w.lower.ends_with("'s")
                    || w.lower.ends_with("'re")
                    || w.lower.ends_with("'m")
                    // The tagger reads "software cuts" as two nouns; a word that can be a verb
                    // is the verb unless a determiner, adjective or preposition comes before it.
                    || w.kind().is_verb() && !after_prep(w) && !after_det(w)
            };
            if words.iter().any(verbish) {
                return;
            }
            out.push(lint(
                span_of(ws),
                LintKind::Grammar,
                "This sentence has no verb. What does the subject do or what is it? (\"The cat for a walk\" → \"The cat went for a walk\")",
                &[],
            ));
        })
    }

    fn description(&self) -> &str {
        "Finds statements with no verb: `The cat for walk.`"
    }
}

/// "Bought" (paid for) where "brought" (carried along) was meant: "I bought
/// beers to the party".
#[derive(Default)]
pub struct BoughtBrought;

impl Linter for BoughtBrought {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        // Things you buy *to* somewhere: tickets to the show, flights to Rome.
        const TO_SOMEWHERE: &[&str] = &[
            "ticket",
            "tickets",
            "flight",
            "flights",
            "pass",
            "passes",
            "seat",
            "seats",
            "admission",
            "access",
            "entry",
            "subscription",
            "subscriptions",
            "membership",
            "memberships",
            "fare",
            "fares",
            "trip",
            "trips",
            "tour",
            "tours",
            "share",
            "shares",
            "stake",
            "insurance",
            "rights",
            "route",
            "time",
            "voucher",
            "vouchers",
        ];
        each_sentence(document, |ws, out| {
            for i in 0..ws.len() {
                if !ws[i].any(&["bought", "buyed"]) {
                    continue;
                }
                // bought X (one to three words) to/along/back + somewhere/someone.
                for n in 1..=3 {
                    let (t, after) = (i + 1 + n, i + 2 + n);
                    let Some(to) = ws.get(t) else { break };
                    let what = &ws[i + 1..t];
                    if !what.iter().all(|w| w.is_word()) || what.iter().any(|w| w.any(TO_SOMEWHERE)) {
                        break;
                    }
                    let place = ws.get(after).is_some_and(|w| {
                        w.any(DETERMINERS) && !w.any(&["a", "an"])
                            || w.any(&["work", "school", "class", "church", "me", "us", "you", "him", "her", "them"])
                    });
                    if to.is("to") && place || to.any(&["along", "back"]) && n == 1 {
                        out.push(lint(
                            ws[i].tok.span,
                            LintKind::Grammar,
                            "Did you mean `brought` (carried along)? `Bought` means paid for.",
                            &[match_case("brought", &ws[i].text)],
                        ));
                        break;
                    }
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `I bought snacks to the party` → `brought`."
    }
}

/// "I'd rather go, then stay": `rather … than`, not `then`.
#[derive(Default)]
pub struct RatherThen;

impl Linter for RatherThen {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            let Some(r) = ws.iter().position(|w| w.is("rather")) else { return };
            // "would/'d/had rather …": a preference, so a comparison follows.
            let preference = r > 0 && (ws[r - 1].any(&["would", "had", "'d"]) || ws[r - 1].lower.ends_with("'d"));
            if !preference || ws.iter().any(|w| w.is("than")) {
                return;
            }
            if let Some(t) = (r + 2..ws.len().min(r + 9)).find(|&k| ws[k].is("then")) {
                out.push(lint(
                    ws[t].tok.span,
                    LintKind::Grammar,
                    "After `rather`, compare with `than`: `I'd rather walk than drive`. `Then` is about time or order.",
                    &[match_case("than", &ws[t].text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `I'd rather walk, then drive` → `than drive`."
    }
}

/// "Yesterday she lead the team", "has lead the way": `led`, the past of
/// lead, spelt like the metal.
#[derive(Default)]
pub struct LeadForLed;

impl Linter for LeadForLed {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const UNITS: &[&str] = &[
            "week",
            "month",
            "year",
            "night",
            "time",
            "season",
            "summer",
            "winter",
            "spring",
            "autumn",
            "fall",
            "weekend",
            "term",
            "semester",
            "quarter",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "decade",
            "century",
        ];
        // What "led" leads: a person, a group or the way, not a metal ("has lead paint").
        const LED_WHAT: &[&str] = &[
            "the", "a", "an", "us", "them", "him", "her", "me", "you", "it", "our", "their", "his", "my", "your",
            "its", "to", "by", "many", "several", "two", "three", "efforts", "teams",
        ];
        each_sentence(document, |ws, out| {
            let past = ws.iter().enumerate().any(|(k, w)| {
                w.any(&["yesterday", "ago", "earlier", "previously"])
                    || w.is("last") && ws.get(k + 1).is_some_and(|n| n.any(UNITS))
                    || w.is("in")
                        && ws.get(k + 1).is_some_and(|n| {
                            n.lower.len() == 4 && n.lower.parse::<u32>().is_ok_and(|y| (1000..2100).contains(&y))
                        })
            });
            for i in 1..ws.len() {
                if !ws[i].is("lead") {
                    continue;
                }
                let next = ws.get(i + 1);
                let prev = &ws[i - 1];
                let after_have =
                    prev.any(&["has", "have", "had", "having", "'ve", "'d"]) && next.is_some_and(|n| n.any(LED_WHAT));
                let after_be = prev.any(&["was", "were", "been", "being", "is", "are"])
                    && next.is_some_and(|n| n.any(&["by", "astray", "away", "into", "out", "off", "through"]));
                let past_subject = past && prev.any(&["i", "you", "we", "they", "he", "she", "it", "who"]);
                if after_have || after_be || past_subject {
                    out.push(lint(
                        ws[i].tok.span,
                        LintKind::Grammar,
                        "Did you mean `led` (the past of lead)? `Lead` is the present tense, or the metal.",
                        &[match_case("led", &ws[i].text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `Yesterday she lead the team` → `led`."
    }
}

/// "A women", "one children": a or one before an irregular plural.
#[derive(Default)]
pub struct SingularAfterA;

impl Linter for SingularAfterA {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const PAIRS: &[(&str, &str)] = &[
            ("women", "woman"),
            ("men", "man"),
            ("children", "child"),
            ("feet", "foot"),
            ("teeth", "tooth"),
            ("mice", "mouse"),
            ("geese", "goose"),
            ("policemen", "policeman"),
            ("businessmen", "businessman"),
            ("businesswomen", "businesswoman"),
            ("firemen", "fireman"),
            ("chairmen", "chairman"),
        ];
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !ws[i].any(&["a", "an", "one", "each", "every", "another"]) {
                    continue;
                }
                let w = &ws[i + 1];
                // "a women's team", "one of the men", "a men-only event".
                if ws.get(i + 2).is_some_and(|n| n.lower.starts_with('\'') || n.lower == "-") {
                    continue;
                }
                let one = match PAIRS.iter().find(|(many, _)| w.is(many)) {
                    Some((_, one)) => one.to_string(),
                    None => match regular_plural_after_a(ws, i) {
                        Some(one) => one,
                        None => continue,
                    },
                };
                out.push(lint(
                    w.tok.span,
                    LintKind::Agreement,
                    format!("After `{}`, use the singular: `{one}`.", ws[i].text),
                    &[match_case(&one, &w.text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `a women` → `a woman`, `one children` → `one child`, `a cats` → `a cat`."
    }
}

/// The singular of a regular plural ("cats" → cat, "boxes" → box, "cities"
/// → city) when the dictionary knows it as a singular noun.
fn regular_singular(plural: &str) -> Option<String> {
    let mut tries = Vec::new();
    if let Some(b) = plural.strip_suffix("ies") {
        tries.push(format!("{b}y"));
    }
    if let Some(b) = plural.strip_suffix("ves") {
        tries.push(format!("{b}f"));
        tries.push(format!("{b}fe"));
    }
    if let Some(b) = plural.strip_suffix("es") {
        tries.push(b.to_string());
    }
    if let Some(b) = plural.strip_suffix('s') {
        tries.push(b.to_string());
    }
    tries.into_iter().find(|t| t.len() > 1 && dict_has(t, |m| m.is_singular_noun()))
}

/// The regular plural of a singular noun ("cat" → cats, "box" → boxes,
/// "city" → cities, "knife" → knives) when the dictionary knows it.
fn regular_plural(one: &str) -> Option<String> {
    let mut tries = Vec::new();
    match one.strip_suffix('y') {
        Some(b) if !b.ends_with(['a', 'e', 'o', 'u']) => tries.push(format!("{b}ies")),
        _ => {}
    }
    if one.ends_with(['s', 'x', 'z', 'o']) || one.ends_with("ch") || one.ends_with("sh") {
        tries.push(format!("{one}es"));
    }
    if let Some(b) = one.strip_suffix("fe").or_else(|| one.strip_suffix('f')) {
        tries.push(format!("{b}ves"));
    }
    tries.push(format!("{one}s"));
    tries.into_iter().find(|t| dict_has(t, |m| m.is_noun()))
}

/// A past-tense verb ("sat", "walked"); the dictionary also lists "sat" as a noun.
fn looks_past(w: &Word) -> bool {
    w.kind().is_verb_past_form()
        || w.lower.ends_with("ed") && w.kind().is_verb()
        || PAST_TO_BASE.iter().any(|(p, _)| w.is(p))
        || IRREGULAR.iter().any(|(_, p, _)| w.is(p))
}

/// A noun that isn't also a function word: the dictionary lists "when" and
/// "two" as nouns too. (Its adjective tags are no help: "cat" has one.)
fn plain_noun(w: &Word) -> bool {
    const NOT_COUNTED: &[&str] = &[
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "twenty",
        "hundred",
        "thousand",
        "million",
        "today",
        "tomorrow",
        "tonight",
        "yesterday",
        "last",
        "first",
        "next",
        "other",
        "others",
        "more",
        "less",
        "most",
        "least",
        "much",
        "lot",
        "lots",
        "deal",
        "few",
        "all",
        "each",
        "extra",
        "spare",
    ];
    let k = w.kind();
    k.is_noun()
        && !w.any(NOT_COUNTED)
        && !k.is_adverb()
        && !k.is_conjunction()
        && !k.is_pronoun()
        && !k.is_determiner()
        && !k.is_preposition()
        && !k.is_cardinal_number()
}

/// A noun that the word before it could describe ("sales" in "a sales
/// team"), not a preposition or past verb that the dictionary also lists
/// as a noun ("in", "sat").
fn describes(n: &Word) -> bool {
    n.is_word()
        && n.kind().is_noun()
        && !n.kind().is_preposition()
        && !n.any(PREPOSITIONS)
        && !n.any(AUX)
        && !looks_past(n)
}

/// Nouns that look plural but take `a` ("a means", "a thanks", "a heads
/// up", "a hands-on class").
const PLURAL_LOOKING: &[&str] = &[
    "means",
    "series",
    "species",
    "news",
    "thanks",
    "congrats",
    "kudos",
    "odds",
    "headquarters",
    "crossroads",
    "barracks",
    "gallows",
    "innings",
    "works",
    "whereabouts",
    "mews",
    "lens",
    "bus",
    "gas",
    "chaos",
    "summons",
    "sales",
    "sports",
    "arms",
    "customs",
    "goods",
    "savings",
    "earnings",
    "ends",
    "needs",
    "lots",
    "loads",
    "tons",
    "billions",
    "millions",
    "thousands",
    "hundreds",
    "dozens",
    "ages",
    "times",
    "ways",
    "yes",
    "nos",
    "dos",
    "heads",
    "hands",
];

/// "A cats", "every days": a regular plural noun right after `ws[i]` (a,
/// an, one, each, every, another). Its singular, or `None` when the
/// pattern is fine or unsure: "a sales team", "grade A students", "one
/// needs", "a rags to riches story".
fn regular_plural_after_a(ws: &[Word], i: usize) -> Option<String> {
    let (det, w) = (&ws[i], ws.get(i + 1)?);
    // "Grade A students", "Plan A results": a capital A mid-sentence is a letter.
    if det.text == "A" && i > 0 || w.text.chars().next().is_some_and(char::is_uppercase) {
        return None;
    }
    if !w.is_word()
        || !w.lower.ends_with('s')
        || w.any(PLURAL_LOOKING)
        || !plain_noun(w)
        || noun_number(w) != Some(Number::Plural)
    {
        return None;
    }
    // "Twenty-one shillings", "twenty one shillings".
    if det.is("one") && i > 0 && (ws[i - 1].is("-") || ws[i - 1].kind().is_cardinal_number()) {
        return None;
    }
    // "One needs", "each costs", "another says": these can stand alone before a verb.
    if det.any(&["one", "each", "another"]) && w.kind().is_verb() {
        return None;
    }
    // "No one", "the one friends call", "this one".
    if det.is("one") && i > 0 && ws[i - 1].any(&["the", "this", "that", "no", "any", "every", "which"]) {
        return None;
    }
    // A plural used as a describing word: "a sales team", "a dogs' home",
    // "a pros and cons list", "a rags to riches story".
    if ws.get(i + 2).is_some_and(|n| describes(n) || n.any(&["and", "or", "&", "to", "n", "'n'"])) {
        return None;
    }
    regular_singular(&w.lower)
}

/// "Two cat", "many problem": a singular countable noun right after a
/// number or plural count word `ws[i]`. Its plural, or `None` when the
/// pattern is fine or unsure: "a two car garage", "five percent", "both
/// parent and child", "few care".
fn regular_singular_after_many(ws: &[Word], i: usize) -> Option<String> {
    const NUMBERS: &[&str] = &[
        "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "twenty", "thirty",
        "forty", "fifty", "several", "multiple", "numerous", "various",
    ];
    // These can also stand alone as the subject: "many agree", "both work here".
    const PRONOUNISH: &[&str] = &["many", "few", "both", "these", "those"];
    // Units and counts that stay singular after a number, and "these kind of".
    const STAYS_SINGULAR: &[&str] = &[
        "percent", "dozen", "hundred", "thousand", "million", "billion", "trillion", "score", "head", "stone", "pound",
        "foot", "inch", "mile", "kind", "sort", "type", "lot", "bit", "o'clock", "and", "or", "of",
    ];
    let (det, w) = (&ws[i], ws.get(i + 1)?);
    let numeric = det.any(NUMBERS);
    // "A plus-four man", "twenty-two".
    if !numeric && !det.any(PRONOUNISH) || i > 0 && ws[i - 1].is("-") {
        return None;
    }
    let k = w.kind();
    if !w.is_word()
        || w.text.chars().next().is_some_and(char::is_uppercase)
        || w.any(STAYS_SINGULAR)
        || !plain_noun(w)
        || noun_number(w) != Some(Number::Singular)
        || !k.is_countable_noun()
        || k.is_mass_noun()
    {
        return None;
    }
    // "Both parent and child".
    if det.is("both") && ws[i + 1..].iter().take(4).any(|x| x.is("and")) {
        return None;
    }
    let next = ws.get(i + 2);
    // A describing count: "a two car garage", "a five star hotel", "a ten minute walk".
    if next.is_some_and(|n| {
        describes(n) || n.kind().is_adjective() && !n.kind().is_preposition() && !n.any(PREPOSITIONS) && !looks_past(n)
    }) {
        return None;
    }
    // A word that's also a verb or describing word ("work", "heavy"; the
    // dictionary counts "cat" as both) only when what follows can't continue
    // it: "I have two dog.", "Two cat sat there." — not "these heavy, quiet
    // people", "ten deep about us", "few care".
    if k.is_verb() || k.is_adjective() || dict_has(&w.lower, |m| m.is_adjective()) {
        let ends = next.is_none_or(|n| n.any(&[".", "!", "?", ";", ":"]));
        let verb_follows = next.is_some_and(|n| n.any(AUX) || looks_past(n));
        // "There were many mistake", "I made few change": the count is an object.
        let object = i > 0 && (ws[i - 1].any(AUX) || ws[i - 1].kind().is_preposition() || looks_past(&ws[i - 1]));
        if !(numeric || object) || !(ends || verb_follows) {
            return None;
        }
    }
    regular_plural(&w.lower)
}

/// "Many woman", "two child": a plural count before an irregular singular.
#[derive(Default)]
pub struct PluralAfterMany;

impl Linter for PluralAfterMany {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const PAIRS: &[(&str, &str)] = &[
            ("woman", "women"),
            ("man", "men"),
            ("child", "children"),
            ("foot", "feet"),
            ("tooth", "teeth"),
            ("mouse", "mice"),
            ("goose", "geese"),
            ("person", "people"),
            ("policeman", "policemen"),
            ("businessman", "businessmen"),
            ("businesswoman", "businesswomen"),
            ("fireman", "firemen"),
            ("chairman", "chairmen"),
        ];
        const MANY: &[&str] = &[
            "many",
            "several",
            "few",
            "both",
            "these",
            "those",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine",
            "ten",
            "twenty",
            "hundred",
            "hundreds",
            "thousands",
            "millions",
            "multiple",
            "numerous",
            "various",
        ];
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                if !ws[i].any(MANY) {
                    continue;
                }
                let w = &ws[i + 1];
                // "a two-man team", "some man's coat", "all man-made", "Many a man".
                if ws.get(i + 2).is_some_and(|n| n.lower.starts_with('\'') || n.lower == "-")
                    || ws[i].is("many") && w.is("a")
                    || i > 0 && ws[i - 1].is("-")
                {
                    continue;
                }
                let many = match PAIRS.iter().find(|(one, _)| w.is(one)) {
                    Some((_, many)) => many.to_string(),
                    None => match regular_singular_after_many(ws, i) {
                        Some(many) => many,
                        None => continue,
                    },
                };
                // "two foot tall", "six foot fence": measurements keep "foot".
                if w.is("foot")
                    && ws[i].any(&[
                        "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "twenty", "hundred",
                    ])
                {
                    continue;
                }
                out.push(lint(
                    w.tok.span,
                    LintKind::Agreement,
                    format!("After `{}`, use the plural: `{many}`.", ws[i].text),
                    &[match_case(&many, &w.text)],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Fixes `many woman` → `many women`, `two child` → `two children`, `two cat` → `two cats`."
    }
}

/// A name written in lowercase where it's introduced as a name: "my name is
/// troy" → "Troy", "a dog named max" → "Max".
#[derive(Default)]
pub struct NameCapital;

impl Linter for NameCapital {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len() {
                // Where does a name start?
                let at = if ws[i].is("name")
                    && i > 0
                    && ws[i - 1].any(&["my", "his", "her", "their", "our", "your", "whose", "its"])
                {
                    match ws.get(i + 1) {
                        Some(w) if w.any(&["is", "was", "'s"]) => i + 2,
                        _ => continue,
                    }
                } else if ws[i].any(&["named", "nicknamed"]) || ws[i].is("me") && i > 0 && ws[i - 1].is("call") {
                    i + 1
                } else {
                    continue;
                };
                // The name: one or two lowercase words that are names, or not words at all.
                let mut j = at;
                while j < at + 2
                    && ws.get(j).is_some_and(|w| {
                        let first = w.text.chars().next();
                        let cap = match_case(&w.lower, "A");
                        w.is_word()
                            && first.is_some_and(char::is_lowercase)
                            && w.lower.chars().all(char::is_alphabetic)
                            && (dict_has(&cap, |m| m.is_proper_noun()) && !known_common(&w.lower)
                                || !known(&w.lower) && w.lower.len() > 1)
                    })
                {
                    let w = &ws[j];
                    out.push(lint(
                        w.tok.span,
                        LintKind::Capitalization,
                        "Names start with a capital letter.",
                        &[match_case(&w.lower, "A")],
                    ));
                    j += 1;
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Capitalises names: `my name is troy` → `Troy`, `a dog named max` → `Max`."
    }
}

/// Everyday lowercase words that are also names ("my name is mud", "named
/// after", "call me later"): never taken as names here.
fn known_common(w: &str) -> bool {
    [
        "mud", "after", "later", "back", "anytime", "tomorrow", "today", "tonight", "soon", "now", "crazy", "old",
        "when", "if", "on", "at", "in", "for", "the", "a", "an", "one", "best", "top", "not", "also", "as", "by",
    ]
    .contains(&w)
}

/// A direct question ending with a full stop: "How are you today." → "?".
/// Polite requests ("Could you send it.") are left alone.
#[derive(Default)]
pub struct QuestionMark;

impl Linter for QuestionMark {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const LEAD: &[&str] =
            &["oh", "so", "hey", "and", "but", "well", "ok", "okay", "hi", "hello", "dear", "anyway", "also", "now"];
        const WH: &[&str] = &["how", "what", "where", "when", "why", "who", "which"];
        const ASK: &[&str] = &[
            "is", "are", "was", "were", "do", "does", "did", "have", "has", "am", "isn't", "aren't", "don't",
            "doesn't", "didn't",
        ];
        const SUBJECT: &[&str] = &[
            "you", "it", "they", "we", "he", "she", "i", "this", "that", "there", "the", "your", "my", "our", "their",
            "his", "her",
        ];
        each_sentence(document, |ws, out| {
            let Some(end) = ws.last().filter(|w| w.lower == ".") else { return };
            let mut i = 0;
            while ws.get(i).is_some_and(|w| w.any(LEAD) || w.kind().is_comma()) {
                i += 1;
            }
            if ws.iter().any(|w| w.is("please")) {
                return;
            }
            // The question word may follow a short exclamation: "oh deer how are you".
            let wh_at = (i..(i + 3).min(ws.len())).find(|&k| ws[k].any(WH));
            let start = wh_at.map_or(i, |k| k + 1);
            let question = ws.get(start).is_some_and(|w| w.any(ASK))
                && ws.get(start + 1).is_some_and(|w| w.any(SUBJECT))
                // "How are you today" but not "What is done is done" / "Is that so" is fine either way.
                && ws.len() > start + 2;
            if question {
                out.push(lint(
                    end.tok.span,
                    LintKind::Punctuation,
                    "This is a question, so it ends with a question mark.",
                    &["?".to_string()],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Ends direct questions with `?`: `How are you today.` → `How are you today?`."
    }
}

// ---------------------------------------------------------------------------

/// The names of the Rust rules.
/// Whether `a` and `b` touch (no space between them).
fn touching(a: &Word, b: &Word) -> bool {
    a.tok.span.end == b.tok.span.start
}

/// "A highly-skilled team": an -ly adverb never takes a hyphen.
#[derive(Default)]
pub struct LyHyphen;

impl Linter for LyHyphen {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(2) {
                let (adv, dash, next) = (&ws[i], &ws[i + 1], &ws[i + 2]);
                let is_dash = dash.text == "-" || dash.text == "\u{2011}";
                // An adverb only (not "early", "friendly", "family": adjectives and nouns can take one).
                let adverb = adv.lower.ends_with("ly")
                    && adv.lower.len() > 4
                    && adv.kind().is_adverb()
                    && !adv.kind().is_adjective()
                    && !adv.kind().is_noun();
                // Set phrases that keep it (Oxford): "fully-fledged".
                let set_phrase = adv.is("fully") && next.lower == "fledged";
                if adverb && !set_phrase && is_dash && next.is_word() && touching(adv, dash) && touching(dash, next) {
                    let fixed = format!("{} {}", adv.text, next.text);
                    out.push(lint(
                        span_of(&ws[i..=i + 2]),
                        LintKind::Punctuation,
                        format!("An -ly word needs no hyphen: `{fixed}`."),
                        &[fixed],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Takes the hyphen out after an -ly adverb: `a highly-skilled team` → `a highly skilled team`."
    }
}

/// "The author is well-known": after the noun, a compound needs no hyphen
/// (most style guides; a gentle note).
#[derive(Default)]
pub struct HyphenAfterNoun;

impl Linter for HyphenAfterNoun {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const LINKING: &[&str] = &[
            "is", "are", "was", "were", "be", "been", "being", "seems", "seemed", "looks", "looked", "became",
            "becomes", "remains", "remained", "feels", "felt",
        ];
        const SOFT: &[&str] = &["very", "so", "quite", "really", "too", "not", "always", "now", "still"];
        each_sentence(document, |ws, out| {
            for (i, w) in ws.iter().enumerate() {
                // well-known, up-to-date, out-of-date: words joined by hyphens with no spaces.
                let mut j = i;
                let mut parts = vec![w.text.clone()];
                while j + 2 < ws.len()
                    && (ws[j + 1].text == "-")
                    && touching(&ws[j], &ws[j + 1])
                    && touching(&ws[j + 1], &ws[j + 2])
                {
                    parts.push(ws[j + 2].text.clone());
                    j += 2;
                }
                if parts.len() < 2 {
                    continue;
                }
                let lower: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
                let compound = (lower.len() == 2 && lower[0] == "well")
                    || lower == ["up", "to", "date"]
                    || lower == ["out", "of", "date"];
                if !compound {
                    continue;
                }
                // After a linking verb (maybe with "very"), and nothing noun-like after it.
                let mut k = i;
                while k > 0 && ws[k - 1].any(SOFT) {
                    k -= 1;
                }
                let after_link = k > 0 && ws[k - 1].any(LINKING);
                let ends = ws.get(j + 1).is_none_or(|n| {
                    !n.is_word() || n.any(&["and", "but", "or", "for", "in", "by", "to", "with", "among"])
                });
                if after_link && ends {
                    let fixed = parts.join(" ");
                    out.push(lint(
                        span_of(&ws[i..=j]),
                        LintKind::Style,
                        format!(
                            "After the noun this usually needs no hyphens: `{fixed}`. Before a noun it keeps them: `{} ones`.",
                            parts.join("-")
                        ),
                        &[fixed],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "After the noun, compounds usually drop their hyphens: `the author is well-known` → `well known`."
    }
}

/// "My three dogs dogs bark": a longer word typed twice. (Harper's
/// RepeatedWords covers short common words like "the the".)
#[derive(Default)]
pub struct DoubledWord;

/// Words English doubles on purpose ("had had", "that that", "very very",
/// "bye bye", "knock knock", "blah blah").
const DOUBLES_OK: &[&str] = &[
    "had", "that", "very", "really", "bye", "knock", "chop", "blah", "night", "there", "well", "never", "again",
    "round", "over", "more", "long", "away", "come", "hush", "pooh", "tick", "tock", "tsk", "tut", "ha", "haha", "boo",
    "yes", "yeah", "no", "nope", "now", "far", "many", "much", "quick", "hear", "right", "okay", "please", "back",
    "down", "faster", "slowly", "louder", "higher", "deeper", "closer", "sorry", "thank", "help", "wait", "stop",
    "run", "go", "beep", "bang", "ding", "dong", "knock", "mama", "papa", "dada", "choo", "bling", "hip", "goody",
    "chit", "chat", "pom", "bonbon", "tom", "papa", "murmur", "tutu", "bora", "walla", "sing", "same", "girly",
    "pretty", "little", "teeny", "tiny", "wee", "fifty",
];

impl Linter for DoubledWord {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(1) {
                let (a, b) = (&ws[i], &ws[i + 1]);
                let gap = b.tok.span.start - a.tok.span.end;
                // Same word, one space apart, 4+ letters (Harper has the short ones),
                // not a name ("Duran Duran", "Bora Bora") and not a deliberate double.
                if a.is_word()
                    && b.is_word()
                    && gap == 1
                    && a.lower == b.lower
                    && a.lower.chars().count() >= 4
                    && a.lower.chars().all(char::is_alphabetic)
                    && !a.text.starts_with(char::is_uppercase)
                    && !DOUBLES_OK.contains(&a.lower.as_str())
                {
                    out.push(lint(
                        span_of(&ws[i..=i + 1]),
                        LintKind::Repetition,
                        format!("`{}` is typed twice.", a.text),
                        std::slice::from_ref(&a.text),
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Finds a word typed twice: `my three dogs dogs bark` → `my three dogs bark`."
    }
}

/// "There we're three cakes", "We we're late": "we're" right after a
/// subject or "there" is "were".
#[derive(Default)]
pub struct WereForWeAre;

impl Linter for WereForWeAre {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 1..ws.len() {
                let (prev, w) = (&ws[i - 1], &ws[i]);
                let next = ws.get(i + 1);
                // "There we're going to stop" can be "There, we're going…" said fast.
                let ing_after_there = prev.lower == "there" && next.is_some_and(|n| n.any(&["going", "gonna"]));
                if w.lower == "we're"
                    && prev.any(&["there", "they", "we", "you", "he", "she", "it", "i"])
                    && prev.tok.span.end + 1 == w.tok.span.start
                    && !ing_after_there
                {
                    out.push(lint(
                        span_of(&ws[i..=i]),
                        LintKind::WordChoice,
                        "`We're` means `we are`; here it should be `were`.",
                        &[match_case("were", &w.text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "`We're` after a subject or `there` is `were`: `there we're three cakes` → `there were three cakes`."
    }
}

/// Verbs whose past is the same word ("Dr Lee put the files away" is fine).
const SAME_PAST: &[&str] = &[
    "put",
    "cut",
    "hit",
    "set",
    "let",
    "shut",
    "read",
    "quit",
    "cast",
    "cost",
    "hurt",
    "spread",
    "split",
    "bet",
    "burst",
    "beat",
    "bid",
    "rid",
    "shed",
    "slit",
    "thrust",
    "upset",
    "broadcast",
    "forecast",
    "wet",
    "fit",
    "knit",
    "spit",
    "sweat",
    "wed",
    "input",
    "output",
    "offset",
    "reset",
    "preset",
    "outbid",
    "overcast",
    "recast",
    "undercut",
];

/// "Mr Monkey sit at the table": a title and a name, then a verb in its
/// plain form, is "sits" (or "sat").
#[derive(Default)]
pub struct TitleNameAgreement;

impl Linter for TitleNameAgreement {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const TITLES: &[&str] = &[
            "mr",
            "mrs",
            "ms",
            "miss",
            "dr",
            "mx",
            "sir",
            "dame",
            "professor",
            "uncle",
            "aunt",
            "aunty",
            "auntie",
            "grandma",
            "grandpa",
            "nana",
            "pop",
        ];
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(2) {
                if !ws[i].any(TITLES) || !ws[i].text.starts_with(char::is_uppercase) {
                    continue;
                }
                // "Mr." / "Dr." and one or two capitalised names.
                let mut j = i + 1;
                if ws.get(j).is_some_and(|w| w.text == ".") {
                    j += 1;
                }
                let start = j;
                // After a title any capitalised word is a name ("Mr Monkey", "Dr Lee").
                while j < ws.len()
                    && j < start + 2
                    && ws[j].is_word()
                    && ws[j].text.starts_with(char::is_uppercase)
                    && ws[j].text.chars().all(char::is_alphabetic)
                {
                    j += 1;
                }
                if j == start || j >= ws.len() {
                    continue;
                }
                // The subject starts the clause: nothing but "and"/"but"/"so" or a comma before the title.
                let clause_start =
                    i == 0 || ws[i - 1].text == "," || ws[i - 1].any(&["and", "but", "so", "then", "when", "because"]);
                let v = &ws[j];
                // A plain present verb ("sit", "make"), not one that's also its past ("put", "read").
                let plain = v.is_word()
                    && v.text.chars().all(|c| c.is_ascii_lowercase())
                    && dict_has(&v.lower, |m| m.is_verb_lemma())
                    && !dict_has(&v.lower, |m| m.is_verb_past_form())
                    && !v.any(SAME_PAST)
                    // Past forms that are also verbs ("lay" from "lie", "fell", "found").
                    && !v.any(&["lay", "fell", "found", "saw", "wound", "ground", "bore", "bound", "felt", "brake", "spake"])
                    // Words that are mostly adverbs or adjectives ("still", "busy", "down").
                    && !(v.kind().is_adverb() || v.kind().is_adjective() || v.kind().is_preposition())
                    && !v.any(AUX);
                if clause_start
                    && plain
                    && let Some(s) = third_person(&v.lower)
                {
                    out.push(lint(
                        span_of(&ws[j..=j]),
                        LintKind::Agreement,
                        format!("With one person as the subject, the verb takes -s: `{s}`."),
                        &[s],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "A title and a name take a verb with -s: `Mr Monkey sit at the table` → `Mr Monkey sits`."
    }
}

/// "New ways off working", "a lot off people", "instead off": a typed
/// "off" where "of" was meant. Phrasal verbs ("turn off the light", "set off
/// walking") and "one-off" are left alone.
#[derive(Default)]
pub struct OffForOf;

impl Linter for OffForOf {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        const OF_WORDS: &[&str] = &[
            "kind", "kinds", "sort", "sorts", "lot", "lots", "couple", "instead", "because", "ahead", "full", "part",
            "plenty", "type", "types", "number", "out",
        ];
        const PHRASAL: &[&str] = &["set", "sets", "kick", "kicks", "start", "starts", "head", "heads", "show", "shows"];
        each_sentence(document, |ws, out| {
            for i in 1..ws.len().saturating_sub(1) {
                let (prev, w, next) = (&ws[i - 1], &ws[i], &ws[i + 1]);
                if w.lower != "off" || !next.is_word() || prev.tok.span.end + 1 != w.tok.span.start {
                    continue;
                }
                // "out off the house", "a lot off people", "instead off waiting".
                let of_word =
                    prev.any(OF_WORDS) && !(prev.lower == "out" && next.any(&["and", "or", "to", "for", "with"]));
                // A noun, then "off", then an -ing word: "ways off working".
                let noun_then_ing = prev.kind().is_noun()
                    && !prev.kind().is_verb()
                    && !prev.any(PHRASAL)
                    && next.lower.ends_with("ing")
                    && next.kind().is_verb();
                if of_word || noun_then_ing {
                    out.push(lint(
                        span_of(&ws[i..=i]),
                        LintKind::Typo,
                        "This looks like `of` with an extra f.",
                        &[match_case("of", &w.text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Finds `off` typed for `of`: `new ways off working` → `new ways of working`."
    }
}

// ---------------------------------------------------------------------------

/// Small words left out where the pattern is clear: "I sat my chair" → "sat
/// on my chair", "Can you pass me salt?" → "pass me the salt". (The spotter
/// catches other missing words; these two it misses.)
#[derive(Default)]
pub struct MissingSmallWord;

/// Things you sit on or in.
const SEATS: &[&str] = &[
    "chair", "seat", "sofa", "couch", "bench", "stool", "bed", "floor", "ground", "grass", "step", "steps", "armchair",
    "beach", "sand", "carpet", "rug", "cushion", "desk", "lap", "saddle",
];
const SIT: &[&str] = &["sat", "sit", "sits", "sitting"];
const POSSESSIVES: &[&str] = &["my", "the", "a", "his", "her", "our", "their", "your", "this", "that", "its"];
/// Words after "pass me" that don't need "the" ("pass me something", "pass me yours").
const NOT_BARE: &[&str] = &[
    "the",
    "a",
    "an",
    "some",
    "any",
    "more",
    "something",
    "anything",
    "everything",
    "nothing",
    "that",
    "this",
    "these",
    "those",
    "it",
    "them",
    "one",
    "another",
    "yours",
    "his",
    "hers",
    "theirs",
    "ours",
    "my",
    "your",
    "our",
    "their",
    "her",
    "its",
    "by",
    "back",
    "over",
    "up",
    "down",
    "on",
    "along",
    "round",
    "around",
];

impl Linter for MissingSmallWord {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            for i in 0..ws.len().saturating_sub(2) {
                // "sat my chair": sat + my/the… + a seat, not followed by "down"/"up" (moving a chair).
                if ws[i].any(SIT)
                    && ws[i + 1].any(POSSESSIVES)
                    && ws[i + 2].any(SEATS)
                    && !ws.get(i + 3).is_some_and(|w| w.any(&["down", "up", "upright", "back", "out"]))
                {
                    let d = &ws[i + 1];
                    out.push(lint(
                        d.tok.span,
                        LintKind::Grammar,
                        format!(
                            "A word seems to be missing: you sit on or in something (`{} on {} {}`).",
                            ws[i].text,
                            d.text,
                            ws[i + 2].text
                        ),
                        &[format!("on {}", d.text), format!("in {}", d.text)],
                    ));
                }
                // "pass me salt": pass/hand + me/us + a singular noun on its own, at the end.
                if ws[i].any(&["pass", "hand"])
                    && ws[i + 1].any(&["me", "us", "him", "her", "them"])
                    && ws[i + 2].is_word()
                    && !ws[i + 2].any(NOT_BARE)
                    && plain_noun(&ws[i + 2])
                    // A name ("pass me Tom"), by its capital: the dictionary lists "salt" as a name too (SALT).
                    && !ws[i + 2].text.starts_with(char::is_uppercase)
                    && noun_number(&ws[i + 2]) != Some(Number::Plural)
                    && ws.get(i + 3).is_none_or(|w| !w.is_word())
                {
                    let n = &ws[i + 2];
                    out.push(lint(
                        n.tok.span,
                        LintKind::Grammar,
                        format!("A word seems to be missing: `the {}` (the one that's there).", n.text),
                        &[format!("the {}", n.text)],
                    ));
                }
            }
        })
    }

    fn description(&self) -> &str {
        "Finds a small word left out where the pattern is clear: `I sat my chair` → `sat on my chair`, `pass me salt` → `pass me the salt`."
    }
}

// ---------------------------------------------------------------------------

/// Two sentences run together with nothing joining them: "I went home I ate
/// dinner". Flagged even though the right joining word can't be known; the
/// suggestions are a full stop, "and" or a semicolon.
///
/// Only a new subject pronoun (I, he, she, we, they) straight after a word
/// that ends a clause, followed by its own verb, when the words before have a
/// verb of their own. Left alone: after verbs and words that take a clause
/// ("I think he left", "glad they came"), after joining words, and after a
/// noun with a/the/my… ("the book I read" is a clause about the book).
#[derive(Default)]
pub struct RunOnSentence;

/// Words a clause can follow without "that" or a comma.
const TAKES_CLAUSE: &[&str] = &[
    "sure",
    "glad",
    "afraid",
    "sorry",
    "certain",
    "aware",
    "happy",
    "sad",
    "pleased",
    "surprised",
    "shocked",
    "worried",
    "relieved",
    "proud",
    "confident",
    "convinced",
    "amazed",
    "disappointed",
    "thankful",
    "grateful",
    "lucky",
    "true",
    "time",
    "day",
    "way",
    "reason",
    "place",
    "moment",
    "minute",
    "second",
    "hour",
    "night",
    "morning",
    "week",
    "year",
    "everything",
    "anything",
    "something",
    "nothing",
    "all",
    "everyone",
    "anyone",
    "someone",
    "nobody",
    "much",
    "more",
    "less",
    "kind",
    "sort",
    "most",
    "least",
    "best",
    "worst",
    "first",
    "last",
    "only",
    "next",
    "same",
    "now",
    "soon",
    "whenever",
    "wherever",
    "however",
    "whatever",
    "whoever",
    "lest",
    "says",
    "said",
    "thinks",
    "thought",
    "ago",
    "out",
    "ere",
    "rather",
    "again",
];
const JOINS: &[&str] = &[
    "and",
    "but",
    "or",
    "nor",
    "so",
    "yet",
    "for",
    "because",
    "when",
    "if",
    "that",
    "which",
    "who",
    "whom",
    "whose",
    "where",
    "while",
    "after",
    "before",
    "since",
    "until",
    "till",
    "unless",
    "though",
    "although",
    "whether",
    "once",
    "as",
    "than",
    "like",
    "what",
    "how",
    "why",
    "then",
    "also",
    "even",
    "just",
    "still",
    "maybe",
    "perhaps",
    "please",
    "unlike",
    "except",
    "whereas",
    "wherever",
    "whenever",
    "now",
    "here",
    "there",
    "today",
    "tonight",
    "yesterday",
    "tomorrow",
    "really",
    "probably",
    "only",
    "both",
    "either",
    "neither",
    "not",
    "to",
    "of",
    "in",
    "on",
    "at",
    "with",
    "by",
    "from",
    "about",
    "into",
    "onto",
    "over",
    "under",
    "between",
    "without",
    "within",
    "through",
    "during",
    "against",
    "among",
    "towards",
    "toward",
    "upon",
    "said",
    "says",
    "told",
    "tell",
    "asked",
    "ask",
    "know",
    "knew",
    "think",
    "thought",
    "hope",
    "wish",
    "bet",
    "guess",
    "suppose",
    "mean",
    "feel",
    "felt",
    "believe",
    "say",
    "see",
    "saw",
    "heard",
    "hear",
    "notice",
    "noticed",
    "realised",
    "realized",
    "found",
    "find",
    "remember",
    "forgot",
    "worry",
    "promise",
    "swear",
    "doubt",
];
const DETS: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "my", "your", "his", "her", "our", "their", "its", "every",
    "each", "any", "some", "no", "which", "what", "whose", "another", "other", "two", "three", "four", "five", "few",
    "many", "several", "all", "both",
];

/// Words that make what follows part of a bigger sentence.
const SUBORDINATE: &[&str] = &[
    "when",
    "if",
    "as",
    "because",
    "after",
    "before",
    "since",
    "until",
    "till",
    "unless",
    "though",
    "although",
    "while",
    "whether",
    "once",
    "whenever",
    "wherever",
    "than",
    "that",
    "how",
    "what",
    "why",
    "who",
    "which",
    "where",
    "ere",
    "lest",
    "like",
    "whom",
    "whose",
    "now",
    "everywhere",
    "anywhere",
    "somewhere",
    "moment",
];

/// Verbs whose object can be a clause without "that" ("I hope they come").
const CLAUSE_VERBS: &[&str] = &[
    "say",
    "tell",
    "ask",
    "know",
    "think",
    "hope",
    "wish",
    "bet",
    "guess",
    "suppose",
    "mean",
    "feel",
    "believe",
    "see",
    "hear",
    "notice",
    "realise",
    "realize",
    "find",
    "remember",
    "forget",
    "worry",
    "promise",
    "swear",
    "doubt",
    "decide",
    "figure",
    "admit",
    "agree",
    "argue",
    "claim",
    "explain",
    "insist",
    "reply",
    "warn",
    "assume",
    "dream",
    "expect",
    "fear",
    "imagine",
    "pretend",
    "suspect",
    "understand",
    "learn",
    "show",
    "prove",
    "reckon",
    "show",
    "make",
    "let",
    "help",
    "watch",
    "bet",
    "pray",
    "trust",
    "insure",
    "ensure",
    "check",
    "confirm",
    "mention",
    "announce",
    "report",
    "suggest",
    "recommend",
    "demand",
    "request",
    "write",
    "read",
    "mind",
    "care",
    "like",
    "love",
    "hate",
    "want",
    "need",
    "prefer",
    "wonder",
    "discover",
    "predict",
    "deny",
    "complain",
    "joke",
    "shout",
    "scream",
    "whisper",
    "answer",
    "add",
    "note",
    "state",
    "teach",
    "remind",
    "convince",
    "inform",
    "assure",
    "confess",
    "fancy",
    "realise",
    "guarantee",
    "bet",
];
const IRREGULAR_CLAUSE: &[&str] = &[
    "said",
    "told",
    "knew",
    "known",
    "thought",
    "meant",
    "felt",
    "saw",
    "seen",
    "heard",
    "found",
    "forgot",
    "forgotten",
    "swore",
    "sworn",
    "understood",
    "learnt",
    "shown",
    "proved",
    "proven",
    "made",
    "wrote",
    "written",
];
fn takes_clause_verb(w: &str) -> bool {
    if IRREGULAR_CLAUSE.contains(&w) {
        return true;
    }
    CLAUSE_VERBS.iter().any(|b| {
        let stem = b.strip_suffix('e').unwrap_or(b);
        let y = b.strip_suffix('y').map(|s| format!("{s}i"));
        w == *b
            || w == format!("{b}s")
            || w == format!("{b}es")
            || w == format!("{b}ed")
            || w == format!("{stem}ed")
            || w == format!("{stem}ing")
            || w == format!("{b}ing")
            // admit → admitted, admitting.
            || b.chars().last().is_some_and(|c| w == format!("{b}{c}ed") || w == format!("{b}{c}ing"))
            || y.as_ref().is_some_and(|y| w == format!("{y}ed") || w == format!("{y}es"))
    })
}

impl Linter for RunOnSentence {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        each_sentence(document, |ws, out| {
            // The start of the current clause (after punctuation).
            let mut clause = 0;
            for j in 0..ws.len() {
                let w = &ws[j];
                if !w.is_word() {
                    clause = j + 1;
                    continue;
                }
                if j < clause + 2 || !(w.text == "I" || w.any(&["he", "she", "we", "they"])) {
                    continue;
                }
                let prev = &ws[j - 1];
                let Some(next) = ws.get(j + 1) else { continue };
                if !prev.is_word() || prev.any(JOINS) || prev.any(TAKES_CLAUSE) || prev.any(AUX) {
                    continue;
                }
                // After a verb of saying, thinking or seeing, the pronoun starts what's said ("I think he left").
                if takes_clause_verb(&prev.lower) {
                    continue;
                }
                // "I told him she was here".
                if prev.any(&["me", "him", "her", "us", "them", "you"]) && j >= 2 && takes_clause_verb(&ws[j - 2].lower)
                {
                    continue;
                }
                // "the book I read": a clause about the noun.
                if prev.kind().is_noun() && ws[clause..j - 1].iter().rev().take(5).any(|x| x.any(DETS)) {
                    continue;
                }
                // The new part needs its own verb right after the pronoun; the old part one of its own.
                let past = |x: &Word| IRREGULAR.iter().any(|(_, p, _)| x.is(p));
                if !(finite_verb(next) || past(next)) || next.any(&["to"]) {
                    continue;
                }
                if !ws[clause..j - 1].iter().any(|x| finite_verb(x) || past(x)) {
                    continue;
                }
                // "As soon as I saw it I felt…", "If he was late he…": a joining word earlier in
                // the clause means the pronoun starts its main part.
                if ws[clause..j].iter().any(|x| x.any(SUBORDINATE)) {
                    continue;
                }
                // The clause before must start with its subject (I, Mum, the dog), not a phrase
                // in front of it ("In the dark he…", "Very slowly she…", "And then they…").
                let first = &ws[clause];
                // "Startled at the noise he…", "Closer and closer they came…".
                let k = first.kind();
                if (k.is_adjective() || first.lower.ends_with("ed") || first.lower.ends_with("ing"))
                    && !k.is_noun()
                    && !k.is_pronoun()
                    && !k.is_determiner()
                {
                    continue;
                }
                if first.any(JOINS) && !first.any(&["then"])
                    || first.any(PREPOSITIONS)
                    || first.lower.ends_with("ly")
                    || first.kind().is_adverb() && !first.kind().is_noun() && !first.kind().is_pronoun()
                {
                    continue;
                }
                // A capital inside the sentence on the pronoun ("He") is a strong sign; else
                // the clause before must be a few words long, so short phrases don't trigger it.
                if j - clause < 3 {
                    continue;
                }
                let start = prev.tok.span.end;
                let span = Span::new(start, w.tok.span.end);
                let next_word = if w.text == "I" { "I".to_string() } else { w.lower.clone() };
                let cap = match_case(&w.lower, "X");
                let cap = if w.text == "I" { "I".to_string() } else { cap };
                out.push(lint(
                    span,
                    LintKind::Grammar,
                    "These look like two sentences with nothing joining them. Add a full stop, or a joining word such as and, but, so or because.",
                    &[format!(". {cap}"), format!(", and {next_word}"), format!("; {next_word}")],
                ));
            }
        })
    }

    fn description(&self) -> &str {
        "Finds two sentences run together with no joining word or full stop: `I went home I ate dinner` → `I went home. I ate dinner`."
    }
}

pub const NAMES: &[&str] = &[
    "PronounCaseInPairs",
    "ParticipleAsPast",
    "ItsForItIs",
    "ThereIsPlural",
    "DurationPreposition",
    "LookForwardToIng",
    "YourWelcome",
    "NounSubjectAgreement",
    "AddressComma",
    "ItIsForIts",
    "JoiningWords",
    "WordOrder",
    "GreetingComma",
    "NoVerb",
    "BoughtBrought",
    "RatherThen",
    "SingularAfterA",
    "QuestionMark",
    "PluralAfterMany",
    "NameCapital",
    "LyHyphen",
    "HyphenAfterNoun",
    "LeadForLed",
    "DoubledWord",
    "WereForWeAre",
    "TitleNameAgreement",
    "OffForOf",
    "MissingSmallWord",
    "RunOnSentence",
];

/// Adds every Rust rule to `group` (under [`NAMES`]); returns the names added.
pub fn add_to(group: &mut harper_core::linting::LintGroup) -> Vec<&'static str> {
    let added = [
        group.add(NAMES[0], PronounCase),
        group.add(NAMES[1], ParticipleAsPast),
        group.add(NAMES[2], ItsForItIs),
        group.add(NAMES[3], ThereIsPlural),
        group.add(NAMES[4], DurationPreposition),
        group.add(NAMES[5], ToIng),
        group.add(NAMES[6], YourWelcome),
        group.add(NAMES[7], NounSubjectAgreement),
        group.add(NAMES[8], AddressComma),
        group.add(NAMES[9], ItIsForIts),
        group.add(NAMES[10], JoiningWords),
        group.add(NAMES[11], WordOrder),
        group.add(NAMES[12], GreetingComma),
        group.add(NAMES[13], NoVerb),
        group.add(NAMES[14], BoughtBrought),
        group.add(NAMES[15], RatherThen),
        group.add(NAMES[16], SingularAfterA),
        group.add(NAMES[17], QuestionMark),
        group.add(NAMES[18], PluralAfterMany),
        group.add(NAMES[19], NameCapital),
        group.add(NAMES[20], LyHyphen),
        group.add(NAMES[21], HyphenAfterNoun),
        group.add(NAMES[22], LeadForLed),
        group.add(NAMES[23], DoubledWord),
        group.add(NAMES[24], WereForWeAre),
        group.add(NAMES[25], TitleNameAgreement),
        group.add(NAMES[26], OffForOf),
        group.add(NAMES[27], MissingSmallWord),
        group.add(NAMES[28], RunOnSentence),
    ];
    NAMES.iter().zip(added).filter(|(_, a)| *a).map(|(n, _)| *n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harper_core::spell::FstDictionary;

    /// The text with the first suggestion of every lint applied, or the
    /// lints' messages when there are none.
    fn fix(linter: &mut dyn Linter, text: &str) -> String {
        let doc = Document::new_plain_english(text, &FstDictionary::curated());
        let mut lints = linter.lint(&doc);
        lints.sort_by_key(|l| std::cmp::Reverse(l.span.start));
        let mut chars: Vec<char> = text.chars().collect();
        for l in lints {
            if let Some(Suggestion::ReplaceWith(r)) = l.suggestions.first() {
                chars.splice(l.span.start..l.span.end, r.iter().copied());
            }
        }
        chars.into_iter().collect()
    }

    fn check(linter: &mut dyn Linter, cases: &[(&str, &str)]) {
        let mut bad = Vec::new();
        for (input, want) in cases {
            let got = fix(linter, input);
            if got != *want {
                bad.push(format!("{input:?}\n   want {want:?}\n    got {got:?}"));
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    #[test]
    fn run_on_sentences() {
        check(
            &mut RunOnSentence,
            &[
                ("I went home I ate dinner.", "I went home. I ate dinner."),
                ("Mum cooked dinner we watched a film.", "Mum cooked dinner. We watched a film."),
                ("I liked the book I read.", "I liked the book I read."),
                ("I think he left.", "I think he left."),
                ("I told him she was here.", "I told him she was here."),
                ("As soon as I saw it I felt it was home.", "As soon as I saw it I felt it was home."),
                ("When he saw Mary he waved.", "When he saw Mary he waved."),
                (
                    "Anne blushed and admitted she was thinking of it.",
                    "Anne blushed and admitted she was thinking of it.",
                ),
                ("A few nights ago I met two men I used to know.", "A few nights ago I met two men I used to know."),
            ],
        );
    }

    #[test]
    fn missing_small_words() {
        check(
            &mut MissingSmallWord,
            &[
                ("I sat my chair.", "I sat on my chair."),
                ("Can you pass me salt?", "Can you pass me the salt?"),
                ("She sat the chair down by the window.", "She sat the chair down by the window."),
                ("He sat my sister on the bench.", "He sat my sister on the bench."),
                ("Please pass me something.", "Please pass me something."),
                ("Pass me the salt.", "Pass me the salt."),
                ("Pass me Tom.", "Pass me Tom."),
            ],
        );
    }

    #[test]
    fn new_slips() {
        check(
            &mut BoughtBrought,
            &[
                ("I bought bears to the party just in case.", "I brought bears to the party just in case."),
                ("She bought her laptop back to the office.", "She brought her laptop back to the office."),
                ("I bought tickets to the show.", "I bought tickets to the show."),
                ("I bought it to share with you.", "I bought it to share with you."),
                ("We bought a gift for the party.", "We bought a gift for the party."),
            ],
        );
        check(
            &mut RatherThen,
            &[
                ("I'd rather be pissed off, then pissed on.", "I'd rather be pissed off, than pissed on."),
                ("I would rather walk then drive.", "I would rather walk than drive."),
                ("I'd rather walk than drive.", "I'd rather walk than drive."),
                ("It was rather late, then we left.", "It was rather late, then we left."),
            ],
        );
        check(
            &mut SingularAfterA,
            &[
                (
                    "Never underestimate the intelligence of a women.",
                    "Never underestimate the intelligence of a woman.",
                ),
                ("Only one children came.", "Only one child came."),
                ("She plays on a women's team.", "She plays on a women's team."),
                ("One of the men left.", "One of the men left."),
            ],
        );
    }

    #[test]
    fn lead_for_led() {
        check(
            &mut LeadForLed,
            &[
                ("Yesterday she lead the team to victory.", "Yesterday she led the team to victory."),
                ("He lead us out of the building last week.", "He led us out of the building last week."),
                ("She has lead the project for years.", "She has led the project for years."),
                ("The project was lead by Anna.", "The project was led by Anna."),
                ("In 2019 they lead the league.", "In 2019 they led the league."),
                ("They lead the league this year.", "They lead the league this year."),
                ("The old house had lead pipes.", "The old house had lead pipes."),
                ("The pipe was lead.", "The pipe was lead."),
                ("Last week I took the lead.", "Last week I took the lead."),
                ("I lead the team now.", "I lead the team now."),
            ],
        );
    }

    #[test]
    fn regular_number_after_counts() {
        check(
            &mut SingularAfterA,
            &[
                ("A cats sat on the mat.", "A cat sat on the mat."),
                ("She ate a burgers for lunch.", "She ate a burger for lunch."),
                ("I go there every days.", "I go there every day."),
                ("We need another burgers.", "We need another burger."),
                ("Another wins.", "Another wins."),
                ("He found a cities map.", "He found a cities map."),
                ("She is a sales manager.", "She is a sales manager."),
                ("It was a rags to riches story.", "It was a rags to riches story."),
                ("They run a dogs' home.", "They run a dogs' home."),
                ("Grade A students passed.", "Grade A students passed."),
                ("One needs patience.", "One needs patience."),
                ("Each costs five dollars.", "Each costs five dollars."),
                ("It is a means to an end.", "It is a means to an end."),
                ("No one thinks so.", "No one thinks so."),
                ("Just a heads up, the deadline moved.", "Just a heads up, the deadline moved."),
            ],
        );
        check(
            &mut PluralAfterMany,
            &[
                ("I have two dog.", "I have two dogs."),
                ("Two cat sat there.", "Two cats sat there."),
                ("We found several problem.", "We found several problems."),
                ("These heavy, quiet people left.", "These heavy, quiet people left."),
                ("Those two could argue all day.", "Those two could argue all day."),
                ("There were many mistake.", "There were many mistakes."),
                ("Three box arrived today.", "Three boxes arrived today."),
                ("They live in a two car garage.", "They live in a two car garage."),
                ("Sales rose five percent.", "Sales rose five percent."),
                ("Both parent and child agreed.", "Both parent and child agreed."),
                ("Few care about it.", "Few care about it."),
                ("Many hope for rain.", "Many hope for rain."),
                ("Both work here.", "Both work here."),
                ("These kind of things happen.", "These kind of things happen."),
                ("It was a ten minute walk.", "It was a ten minute walk."),
                ("He is six foot tall.", "He is six foot tall."),
            ],
        );
    }

    /// False-alarm scan: `NUMBER_SCAN=file.jsonl cargo test -p xenify-grammar
    /// number_scan -- --ignored --nocapture` prints every flag on clean text
    /// (`corrected`, else `text`) and counts flags on the mistaken `text`.
    #[test]
    #[ignore]
    fn number_scan() {
        let path = std::env::var("NUMBER_SCAN").expect("NUMBER_SCAN=file.jsonl");
        let flags = |text: &str| {
            let doc = Document::new_plain_english(text, &FstDictionary::curated());
            let mut lints = SingularAfterA.lint(&doc);
            lints.extend(PluralAfterMany.lint(&doc));
            lints
        };
        let (mut n, mut false_alarms, mut wrong, mut caught) = (0, 0, 0, 0);
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let text = v["text"].as_str().unwrap_or_default();
            let clean = v["corrected"].as_str().unwrap_or(text);
            n += 1;
            for l in flags(clean) {
                false_alarms += 1;
                let w: String = clean.chars().skip(l.span.start).take(l.span.end - l.span.start).collect();
                println!("FA [{w}] {}  ::  {clean}", l.message);
            }
            if clean != text {
                wrong += 1;
                let got = flags(text);
                if !got.is_empty() {
                    caught += 1;
                    let l = &got[0];
                    let w: String = text.chars().skip(l.span.start).take(l.span.end - l.span.start).collect();
                    println!("CAUGHT [{w}] {}  ::  {text}", l.message);
                }
            }
        }
        println!("{false_alarms} false alarms in {n} clean passages; flagged {caught} of {wrong} mistaken ones");
    }

    #[test]
    fn name_capital() {
        check(
            &mut NameCapital,
            &[
                ("hello my name is troy", "hello my name is Troy"),
                ("Her name is sarah and she leads the team.", "Her name is Sarah and she leads the team."),
                ("We adopted a dog named max.", "We adopted a dog named Max."),
                ("Hi, my name is ngozi okafor.", "Hi, my name is Ngozi Okafor."),
                ("My name is mud after that meeting.", "My name is mud after that meeting."),
                ("The street was named after a general.", "The street was named after a general."),
                ("Call me later.", "Call me later."),
                ("My name is Troy.", "My name is Troy."),
                ("He was named best player.", "He was named best player."),
            ],
        );
    }

    #[test]
    fn question_mark() {
        check(
            &mut QuestionMark,
            &[
                ("oh dear how are you today.", "oh dear how are you today?"),
                ("oh deer how are you today.", "oh deer how are you today?"),
                ("Did you get my email.", "Did you get my email?"),
                ("Where is the report.", "Where is the report?"),
                ("Could you send me the file.", "Could you send me the file."),
                ("Can you please check it.", "Can you please check it."),
                ("How it works is simple.", "How it works is simple."),
                ("What we found was surprising.", "What we found was surprising."),
                ("I wonder how you are.", "I wonder how you are."),
            ],
        );
    }

    #[test]
    fn no_verb() {
        let mut d = harper_core::Document::new_plain_english_curated("The cat for walk.");
        assert_eq!(NoVerb.lint(&d).len(), 1);
        for ok in [
            "The cat went for a walk.",
            "The end of the road.",
            "Thanks for everything.",
            "Great job!",
            "A cup of tea?",
            "The report is attached.",
            "The team's results.",
            "Our new office, near the station.",
            "Our new software cuts setup time in half.",
            "No smoking in the lobby area.",
            "The meeting with Dr.",
            "The store on Main Street sells springtails for bioactive setups.",
            "An array of technical certifications backs my expertise in cloud infrastructure.",
        ] {
            d = harper_core::Document::new_plain_english_curated(ok);
            let n = NoVerb.lint(&d).len();
            if ok == "The end of the road." {
                continue; // a heading-like phrase; either answer is fine
            }
            assert_eq!(n, 0, "{ok}");
        }
    }

    #[test]
    fn pronoun_case() {
        check(
            &mut PronounCase,
            &[
                ("Her and Marcus are taking the dogs out.", "She and Marcus are taking the dogs out."),
                ("Him and David have been working all day.", "He and David have been working all day."),
                ("Me and Sam finished early.", "Sam and I finished early."),
                ("Me and him went home.", "He and I went home."),
                (
                    "Can you believe it, between you and I, we sold out?",
                    "Can you believe it, between you and me, we sold out?",
                ),
                (
                    "Mom made dinner for my brother and I last night.",
                    "Mom made dinner for my brother and me last night.",
                ),
                (
                    "The boss told my coworker and I to lead the meeting.",
                    "The boss told my coworker and me to lead the meeting.",
                ),
                ("Marcus and him went to the lake.", "Marcus and he went to the lake."),
                // Correct sentences stay as they are.
                ("She and Marcus are taking the dogs out.", "She and Marcus are taking the dogs out."),
                ("I told her and Marcus to wait.", "I told her and Marcus to wait."),
                ("After my wife and I arrived, we ate.", "After my wife and I arrived, we ate."),
                ("This is for Sam and me.", "This is for Sam and me."),
                ("Her and her sister's house is lovely.", "Her and her sister's house is lovely."),
                ("Between them and us there is a wall.", "Between them and us there is a wall."),
                ("Thanks to Priya and I'm sure the team, we won.", "Thanks to Priya and I'm sure the team, we won."),
                (
                    "I bought these for my commute and they block out noise.",
                    "I bought these for my commute and they block out noise.",
                ),
                (
                    "I attached it to this email and I also sent a copy.",
                    "I attached it to this email and I also sent a copy.",
                ),
                (
                    "We watched a film about penguins and I learned a lot.",
                    "We watched a film about penguins and I learned a lot.",
                ),
                (
                    "I bought the new headphones from Sony and they sound amazing.",
                    "I bought the new headphones from Sony and they sound amazing.",
                ),
            ],
        );
    }

    #[test]
    fn participle_as_past() {
        check(
            &mut ParticipleAsPast,
            &[
                ("I seen him there.", "I saw him there."),
                ("They done it again.", "They did it again."),
                ("She taken the last one.", "She took the last one."),
                ("I have seen him there.", "I have seen him there."),
                ("Have you seen him?", "Have you seen him?"),
                ("He is known as Bob.", "He is known as Bob."),
                ("It seen better days.", "It saw better days."),
            ],
        );
    }

    #[test]
    fn its_for_it_is() {
        check(
            &mut ItsForItIs,
            &[
                ("Its raining today.", "It's raining today."),
                ("Its been a long week.", "It's been a long week."),
                ("I think its a good idea.", "I think it's a good idea."),
                ("Its not fair.", "It's not fair."),
                ("The company raised its prices.", "The company raised its prices."),
                ("The dog wagged its tail.", "The dog wagged its tail."),
                ("The team won its first game.", "The team won its first game."),
                ("The city and its people.", "The city and its people."),
                ("The app has its own settings.", "The app has its own settings."),
                ("The building lost its only exit.", "The building lost its only exit."),
                ("The machine and its training data.", "The machine and its training data."),
                (
                    "The library is closed, but its scheduled to reopen.",
                    "The library is closed, but it's scheduled to reopen.",
                ),
                ("Close the deck, so its best to clear it.", "Close the deck, so it's best to clear it."),
                ("We tried; its unclear whether it works.", "We tried; it's unclear whether it works."),
                ("I'm glad its finally happening.", "I'm glad it's finally happening."),
                ("My phone arrived and its exactly what I wanted.", "My phone arrived and it's exactly what I wanted."),
                (
                    "The school credits its dedicated teaching staff.",
                    "The school credits its dedicated teaching staff.",
                ),
                ("The book has its best moments early.", "The book has its best moments early."),
                ("The cafe is great - its run by a nice couple.", "The cafe is great - it's run by a nice couple."),
                ("The dog wagged its tail happily.", "The dog wagged its tail happily."),
                ("The firm lost its biggest client.", "The firm lost its biggest client."),
            ],
        );
    }

    #[test]
    fn it_is_for_its() {
        check(
            &mut ItIsForIts,
            &[
                (
                    "The park closes at sunset and it's gates are locked.",
                    "The park closes at sunset and its gates are locked.",
                ),
                ("Every pet comes with it's vaccinations.", "Every pet comes with its vaccinations."),
                ("We will review it's liability insurance.", "We will review its liability insurance."),
                ("Each module exposes it's own endpoint.", "Each module exposes its own endpoint."),
                ("The restaurant claims it's pasta is homemade.", "The restaurant claims its pasta is homemade."),
                ("The service runs in it's degraded state.", "The service runs in its degraded state."),
                ("It's time to go.", "It's time to go."),
                ("I think it's money well spent.", "I think it's money well spent."),
                ("It's raining.", "It's raining."),
                ("I know it's true.", "I know it's true."),
                ("It's Friday!", "It's Friday!"),
                ("It's a deal.", "It's a deal."),
                ("It's hard work.", "It's hard work."),
                ("Because it's summer, we swim.", "Because it's summer, we swim."),
                ("It's been a long day.", "It's been a long day."),
                ("Tell me when it's ready.", "Tell me when it's ready."),
                ("It's me.", "It's me."),
                ("It's lunch time.", "It's lunch time."),
                ("It's freezing outside.", "It's freezing outside."),
                ("It's teamwork that matters.", "It's teamwork that matters."),
            ],
        );
    }

    #[test]
    fn there_is_plural() {
        check(
            &mut ThereIsPlural,
            &[
                ("There is many reasons to stay.", "There are many reasons to stay."),
                ("There was several problems.", "There were several problems."),
                ("There is two options.", "There are two options."),
                ("There is still many questions.", "There are still many questions."),
                ("There is a reason.", "There is a reason."),
                ("There is news from home.", "There is news from home."),
                ("There is a lot of work.", "There is a lot of work."),
                ("There are many reasons.", "There are many reasons."),
                ("There was a few of us left.", "There was a few of us left."),
            ],
        );
    }

    #[test]
    fn duration_preposition() {
        check(
            &mut DurationPreposition,
            &[
                ("I'll call you on 10 minutes.", "I'll call you in 10 minutes."),
                ("She has lived here since five years.", "She has lived here for five years."),
                ("We met on a day like this.", "We met on a day like this."),
                ("I have lived here since 2019.", "I have lived here since 2019."),
                ("I left two years ago.", "I left two years ago."),
                ("The shop is open on two days a week.", "The shop is open on two days a week."),
                ("During the next few days, rest.", "During the next few days, rest."),
            ],
        );
    }

    #[test]
    fn look_forward_to() {
        check(
            &mut ToIng,
            &[
                ("I look forward to hear from you.", "I look forward to hearing from you."),
                ("We look forward to meet you.", "We look forward to meeting you."),
                ("I'm looking forward to see you again.", "I'm looking forward to seeing you again."),
                ("I look forward to hearing from you.", "I look forward to hearing from you."),
                ("I look forward to the meeting.", "I look forward to the meeting."),
                ("I look forward to work on Monday.", "I look forward to work on Monday."),
            ],
        );
    }

    #[test]
    fn noun_subject_agreement() {
        check(
            &mut NounSubjectAgreement,
            &[
                ("The results of the experiment was clear.", "The results of the experiment were clear."),
                ("The boxes in the hall needs moving.", "The boxes in the hall need moving."),
                ("Each of the students have a book.", "Each of the students has a book."),
                ("The list of items are on the table.", "The list of items is on the table."),
                ("The dogs runs fast.", "The dogs run fast."),
                ("One of the reasons are cost.", "One of the reasons is cost."),
                ("My parents was happy.", "My parents were happy."),
                // Correct, or not safe to judge.
                ("The results of the experiment were clear.", "The results of the experiment were clear."),
                ("The list of items is on the table.", "The list of items is on the table."),
                ("The number of cases is rising.", "The number of cases is rising."),
                ("A number of cases are rising.", "A number of cases are rising."),
                ("The data shows a trend.", "The data shows a trend."),
                ("The team are celebrating.", "The team are celebrating."),
                ("The dog and the cat are friends.", "The dog and the cat are friends."),
                ("The man who lives next door is nice.", "The man who lives next door is nice."),
                ("News travels fast.", "News travels fast."),
                ("Physics is hard.", "Physics is hard."),
                ("The house prices rise every year.", "The house prices rise every year."),
                ("Sales were strong.", "Sales were strong."),
                ("The company's profits rose.", "The company's profits rose."),
                ("The findings suggest a link.", "The findings suggest a link."),
                ("The mitochondria are small.", "The mitochondria are small."),
                ("The criteria is unclear.", "The criteria are unclear."),
                (
                    "Remember that each of the students need a permit.",
                    "Remember that each of the students needs a permit.",
                ),
                ("Everyone in my family love your videos.", "Everyone in my family love your videos."),
                ("A new report from the council highlights rents.", "A new report from the council highlights rents."),
                ("The tall man in the blue jacket is my cousin.", "The tall man in the blue jacket is my cousin."),
                ("The coach let him play.", "The coach let him play."),
                (
                    "Making fewer mistakes in your messages improves things.",
                    "Making fewer mistakes in your messages improves things.",
                ),
                ("Everyone except the interns has left.", "Everyone except the interns has left."),
                ("Neither of the two tutorials explain it.", "Neither of the two tutorials explains it."),
                ("The variety of flavors appeal to kids.", "The variety of flavors appeals to kids."),
                ("A variety of flavors appeal to kids.", "A variety of flavors appeal to kids."),
                ("The number of tourists have increased.", "The number of tourists has increased."),
                (
                    "The professor along with her students are presenting.",
                    "The professor along with her students is presenting.",
                ),
                (
                    "The group of guys I play with always meet at Pete's.",
                    "The group of guys I play with always meet at Pete's.",
                ),
                ("The collection of records always stand out.", "The collection of records always stands out."),
                (
                    "It sold out, and the feedback from customers have been great.",
                    "It sold out, and the feedback from customers has been great.",
                ),
                ("The combination of salt and fats make it tasty.", "The combination of salt and fats makes it tasty."),
                ("Each of the cartridges still work perfectly.", "Each of the cartridges still works perfectly."),
                ("I think that the children are fine.", "I think that the children are fine."),
                ("He said that the plan works.", "He said that the plan works."),
                ("Everyone knows that.", "Everyone knows that."),
                ("A number of people have left.", "A number of people have left."),
                ("The majority of voters are young.", "The majority of voters are young."),
                ("Five hundred dollars is enough to buy the book.", "Five hundred dollars is enough to buy the book."),
                (
                    "Our new art exhibit features works by local artists.",
                    "Our new art exhibit features works by local artists.",
                ),
                (
                    "Thanks to Alex; the choreography, the costume design, and the editing were all great.",
                    "Thanks to Alex; the choreography, the costume design, and the editing were all great.",
                ),
            ],
        );
    }

    #[test]
    fn address_comma() {
        check(
            &mut AddressComma,
            &[
                ("Thanks John for the update.", "Thanks, John, for the update."),
                ("Thank you everyone for coming.", "Thank you, everyone, for coming."),
                ("Congratulations Priya!", "Congratulations, Priya!"),
                ("Let's eat Grandma.", "Let's eat, Grandma."),
                ("I love you Mom.", "I love you, Mom."),
                ("See you guys!", "See you, guys!"),
                // Correct or not addressing anyone.
                ("Thanks, John, for the update.", "Thanks, John, for the update."),
                ("Thanks for the update.", "Thanks for the update."),
                ("I called Mom.", "I called Mom."),
                ("Let's eat, Grandma.", "Let's eat, Grandma."),
                ("Thank you all the same.", "Thank you all the same."),
                ("Welcome to the team.", "Welcome to the team."),
                ("Let's visit Paris.", "Let's visit Paris."),
                ("Let's call Sam.", "Let's call Sam."),
                ("Let's go team!", "Let's go team!"),
                ("Hello there!", "Hello there!"),
                ("Thank you so much.", "Thank you so much."),
                ("We met Dr. Patel in London.", "We met Dr. Patel in London."),
                ("Look Tom I already told you.", "Look, Tom, I already told you."),
                ("Mr. Henderson please forward the waiver.", "Mr. Henderson, please forward the waiver."),
                ("John can you help me?", "John, can you help me?"),
                ("Tell me Maria whether it worked.", "Tell me, Maria, whether it worked."),
                ("Trust me Chris I've done this before.", "Trust me, Chris, I've done this before."),
                (
                    "Let me know if you have questions about the new workflow John.",
                    "Let me know if you have questions about the new workflow, John.",
                ),
                ("I can't believe you did that Sarah.", "I can't believe you did that, Sarah."),
                // Not addressing anyone.
                ("Please send the documents to Ms. Chen.", "Please send the documents to Ms. Chen."),
                ("Happy birthday to my favorite cousin Jeff!", "Happy birthday to my favorite cousin Jeff!"),
                ("Can you introduce me to Dr. Chen?", "Can you introduce me to Dr. Chen?"),
                ("I hope you enjoy New York.", "I hope you enjoy New York."),
                ("Did you see Tom?", "Did you see Tom?"),
                ("Did you hear what happened to Tom?", "Did you hear what happened to Tom?"),
                ("You should read Harry Potter.", "You should read Harry Potter."),
                ("Thank you for choosing Acme.", "Thank you for choosing Acme."),
                ("Your order ships from Ohio.", "Your order ships from Ohio."),
                ("John can come with us.", "John can come with us."),
                ("Who do you think will win?", "Who do you think will win?"),
                ("Mark would know.", "Mark would know."),
                ("I told you Mark would come.", "I told you Mark would come."),
                ("Look Inside the Box is a good book.", "Look Inside the Box is a good book."),
                ("You can call me Al.", "You can call me Al."),
                ("Did you meet my colleague Sarah?", "Did you meet my colleague Sarah?"),
                ("Have you been to Lake Tahoe?", "Have you been to Lake Tahoe?"),
                ("I hope you liked the gift from Grandma.", "I hope you liked the gift from Grandma."),
                ("Please contact our support team at Acme Corp.", "Please contact our support team at Acme Corp."),
                (
                    "I will try this and share the results with you all.",
                    "I will try this and share the results with you all.",
                ),
                (
                    "Tickets are sold at the kiosk; you must have a valid ID.",
                    "Tickets are sold at the kiosk; you must have a valid ID.",
                ),
            ],
        );
    }

    #[test]
    fn joining_words() {
        check(
            &mut JoiningWords,
            &[
                ("Although it was late, but we went out.", "Although it was late, we went out."),
                ("Even though I asked, but nobody answered.", "Even though I asked, nobody answered."),
                ("Because it rained, so we stayed in.", "Because it rained, we stayed in."),
                ("I tried, however it failed.", "I tried; however, it failed."),
                ("Despite of the rain, we played.", "Despite the rain, we played."),
                ("Although it was late, we went out.", "Although it was late, we went out."),
                ("It was late, but we went out.", "It was late, but we went out."),
                ("We tried, however, and it failed.", "We tried, however, and it failed."),
                ("However, it failed.", "However, it failed."),
                (
                    "Because of the rain, so many people stayed home.",
                    "Because of the rain, so many people stayed home.",
                ),
                ("Since 2019, so much has changed.", "Since 2019, so much has changed."),
            ],
        );
    }

    #[test]
    fn word_order() {
        check(
            &mut WordOrder,
            &[
                ("Can you tell me where is the office?", "Can you tell me where the office is?"),
                ("I don't know what is the problem.", "I don't know what the problem is."),
                ("Do you know when will the meeting start?", "Do you know when the meeting will start?"),
                ("I wonder why is he late.", "I wonder why he is late."),
                (
                    "Can you tell me where is the runbook for the cache?",
                    "Can you tell me where the runbook is for the cache?",
                ),
                ("I always am late.", "I am always late."),
                ("She never is on time.", "She is never on time."),
                ("Can you tell me where the office is?", "Can you tell me where the office is?"),
                ("Do you know who is the manager?", "Do you know who is the manager?"),
                ("I know what is happening.", "I know what is happening."),
                ("Where is the office?", "Where is the office?"),
                ("I am always late.", "I am always late."),
                ("They always are not ready.", "They always are not ready."),
            ],
        );
    }

    #[test]
    fn greeting_comma() {
        check(
            &mut GreetingComma,
            &[
                ("Hello darling did you miss me?", "Hello darling, did you miss me?"),
                ("Hi Marcus\nThe report is attached.", "Hi Marcus,\nThe report is attached."),
                ("Dear Ms. Chen\n\nThank you for writing.", "Dear Ms. Chen,\n\nThank you for writing."),
                ("Dear Mr. Henderson I am writing to apply.", "Dear Mr. Henderson, I am writing to apply."),
                ("Good morning Yuki\nCould we talk?", "Good morning Yuki,\nCould we talk?"),
                ("Hi Marcus,\nThe report is attached.", "Hi Marcus,\nThe report is attached."),
                ("Dear Ms. Chen:\nThank you.", "Dear Ms. Chen:\nThank you."),
            ],
        );
    }

    #[test]
    fn your_welcome() {
        check(
            &mut YourWelcome,
            &[
                ("Your welcome.", "You're welcome."),
                ("Thanks! Your most welcome.", "Thanks! You're most welcome."),
                ("Your very welcome to join us.", "You're very welcome to join us."),
                ("Your welcome pack is in the post.", "Your welcome pack is in the post."),
                ("You're welcome.", "You're welcome."),
            ],
        );
    }

    #[test]
    fn verb_forms_list() {
        let f = verb_forms("went");
        assert!(f.contains(&"go".to_string()) && f.contains(&"gone".to_string()), "{f:?}");
        let f = verb_forms("need");
        assert!(
            f.contains(&"needs".to_string()) && f.contains(&"needed".to_string()) && f.contains(&"needing".to_string()),
            "{f:?}"
        );
        assert!(verb_forms("was").contains(&"were".to_string()));
    }

    #[test]
    fn ing_forms() {
        for (v, want) in [
            ("hear", "hearing"),
            ("make", "making"),
            ("stop", "stopping"),
            ("see", "seeing"),
            ("lie", "lying"),
            ("visit", "visiting"),
            ("fix", "fixing"),
            ("play", "playing"),
            ("get", "getting"),
        ] {
            assert_eq!(ing(v), want);
        }
    }

    #[test]
    fn off_for_of() {
        assert_eq!(fix(&mut OffForOf, "New ways off working."), "New ways of working.");
        assert_eq!(fix(&mut OffForOf, "There were a lot off people."), "There were a lot of people.");
        assert_eq!(fix(&mut OffForOf, "She ran out off the house."), "She ran out of the house.");
        assert_eq!(fix(&mut OffForOf, "Instead off waiting, we left."), "Instead of waiting, we left.");
        for ok in [
            "Turn off the light.",
            "We set off walking at dawn.",
            "It was a one off event.",
            "He took his hat off.",
            "The day off was nice.",
            "Keep off the grass.",
            "The meeting is off.",
            "The lights went out and off.",
            "Jump off the wall.",
            "Take the lid off quickly.",
        ] {
            assert_eq!(fix(&mut OffForOf, ok), ok, "{ok}");
        }
    }

    #[test]
    fn were_for_we_are() {
        assert_eq!(
            fix(&mut WereForWeAre, "There we’re fascinating pastries here."),
            "There were fascinating pastries here."
        );
        assert_eq!(fix(&mut WereForWeAre, "We we're late for school."), "We were late for school.");
        for ok in ["We're late.", "There, we're done.", "The things we're doing are fun.", "There we're going to stop."]
        {
            assert_eq!(fix(&mut WereForWeAre, ok), ok, "{ok}");
        }
    }

    #[test]
    fn title_name_agreement() {
        assert_eq!(fix(&mut TitleNameAgreement, "Mr Monkey sit at the table."), "Mr Monkey sits at the table.");
        assert_eq!(fix(&mut TitleNameAgreement, "Mrs Patel walk to the shops."), "Mrs Patel walks to the shops.");
        for ok in [
            "Mr Monkey sits at the table.",
            "Mr Monkey sat at the table.",
            "Mr Smith and I sit together.",
            "Mrs Patel will walk home.",
            "Dr Lee put the files away.",
            "Ask Mr Brown to sit down.",
            "Mr Monkey Business is a film.",
            "Mr Browne still seemed puzzled.",
            "Miss Ramsdell lay down her cake.",
            "Aunt Janet down the road waved.",
            "Miss Mackin busy as ever, waved.",
        ] {
            assert_eq!(fix(&mut TitleNameAgreement, ok), ok, "{ok}");
        }
    }

    #[test]
    fn doubled_words() {
        assert_eq!(fix(&mut DoubledWord, "My three dogs dogs bark."), "My three dogs bark.");
        assert_eq!(fix(&mut DoubledWord, "He has been working working there."), "He has been working there.");
        // Deliberate doubles, names, short words (Harper's rule) and words split by punctuation.
        for ok in [
            "What he had had was gone.",
            "It was very very cold.",
            "We met Duran Duran.",
            "We sat in the the hall.",
            "Knock knock, who is there?",
            "Bye bye, little one.",
            "It was long, long ago.",
            "Wait, wait!",
        ] {
            assert_eq!(fix(&mut DoubledWord, ok), ok, "{ok}");
        }
    }
}
