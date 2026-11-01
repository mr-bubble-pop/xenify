//! Autocorrect: small fixes made as you type. A short list the user can edit
//! (`teh` → `the`, `(c)` → `©`, `--` → `–`), curly quotes, emoticons as
//! emoji (`:)` → 🙂), and optionally a capital letter at the start of a sentence. Each correction is its own undo
//! step, so Ctrl+Z straight after gives back what was typed.

use crate::edit::{EditError, Editor, Step};
use crate::{Pos, Selection, Span};

/// The autocorrect settings (Settings › Format as you type).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Autocorrect {
    /// Replace the words and symbols in [`Autocorrect::entries`].
    pub replace: bool,
    /// What is typed → what it becomes.
    pub entries: Vec<(String, String)>,
    /// Straight quotes ' " become curly ‘ ’ “ ”.
    pub smart_quotes: bool,
    /// A capital letter at the start of a sentence (off by default: it
    /// guesses wrong after abbreviations).
    pub capitalize_sentences: bool,
    /// Emoticons typed on their own become emoji: `:)` → 🙂 ([`EMOTICONS`]).
    pub emoticons: bool,
}

impl Default for Autocorrect {
    fn default() -> Self {
        Self {
            replace: true,
            entries: default_entries(),
            smart_quotes: true,
            capitalize_sentences: false,
            emoticons: true,
        }
    }
}

/// Emoticons and the emoji they become (typed on their own, then a space).
pub const EMOTICONS: &[(&str, &str)] = &[
    (":)", "🙂"),
    (":-)", "🙂"),
    (":D", "😀"),
    (":-D", "😀"),
    (";)", "😉"),
    (";-)", "😉"),
    (":(", "🙁"),
    (":-(", "🙁"),
    (":'(", "😢"),
    (":P", "😛"),
    (":-P", "😛"),
    (":O", "😮"),
    (":-O", "😮"),
    (":|", "😐"),
    (":*", "😘"),
    ("XD", "😆"),
    ("<3", "❤\u{fe0f}"),
    ("</3", "💔"),
    ("(y)", "👍"),
    ("(n)", "👎"),
];

/// The emoticons that type `emoji` (for showing beside it in a picker).
pub fn emoticons_for(emoji: &str) -> Vec<&'static str> {
    let key = |s: &str| s.chars().filter(|&c| c != '\u{fe0f}').collect::<String>();
    EMOTICONS.iter().filter(|(_, e)| key(e) == key(emoji)).map(|(t, _)| *t).collect()
}

/// The starting list: common typos and symbols, kept short on purpose.
pub fn default_entries() -> Vec<(String, String)> {
    [
        ("teh", "the"),
        ("adn", "and"),
        ("taht", "that"),
        ("recieve", "receive"),
        ("seperate", "separate"),
        ("definately", "definitely"),
        ("occured", "occurred"),
        ("untill", "until"),
        ("(c)", "©"),
        ("(r)", "®"),
        ("(tm)", "™"),
        ("--", "–"),
        ("...", "…"),
        ("->", "→"),
        ("<-", "←"),
        ("=>", "⇒"),
        ("1/2", "½"),
        ("1/4", "¼"),
        ("3/4", "¾"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

/// Typing one of these ends a word and may trigger a replacement.
fn ends_word(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\u{201d}')
}

/// Before an opening quote: the start, a space, or an opening bracket/dash.
fn opens_after(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => {
            c.is_whitespace() || matches!(c, '(' | '[' | '{' | '\u{2014}' | '\u{2013}' | '\u{2018}' | '\u{201c}')
        }
    }
}

/// `to` in the case `typed` was written in: `Teh` → `The`, `TEH` → `THE`.
fn match_case(typed: &str, to: &str) -> String {
    let letters: Vec<char> = typed.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return to.to_uppercase();
    }
    if letters.first().is_some_and(|c| c.is_uppercase()) {
        let mut cs = to.chars();
        return cs.next().map(|f| f.to_uppercase().chain(cs).collect()).unwrap_or_default();
    }
    to.to_string()
}

/// The correction to make after `typed` was typed at the end of `before`
/// (the paragraph text up to and including it): the byte range of `before`
/// to replace and its new text.
pub fn correction(before: &str, typed: &str, opts: &Autocorrect) -> Option<(std::ops::Range<usize>, String)> {
    let mut chars = typed.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None; // pasted or several characters at once: leave alone
    }
    let end = before.len();
    let start = end - c.len_utf8();
    let prev = before[..start].chars().next_back();
    if opts.smart_quotes && (c == '\'' || c == '"') {
        let curly = match (c, opens_after(prev)) {
            ('\'', true) => '\u{2018}',
            ('\'', false) => '\u{2019}',
            (_, true) => '\u{201c}',
            (_, false) => '\u{201d}',
        };
        return Some((start..end, curly.to_string()));
    }
    if !ends_word(c) {
        return None;
    }
    let text = &before[..start];
    if opts.replace {
        // The longest entry the text ends with wins ("1/2" before "/2").
        let mut found: Option<(usize, &str)> = None;
        for (from, to) in &opts.entries {
            if from.is_empty() || from.len() > text.len() || found.is_some_and(|(n, _)| n >= from.len()) {
                continue;
            }
            let at = text.len() - from.len();
            let Some(tail) = text.get(at..) else { continue };
            if !tail.eq_ignore_ascii_case(from) {
                continue;
            }
            // Words and numbers only match whole: "steh" isn't "teh", "11/2" isn't "1/2".
            let wordy = from.starts_with(char::is_alphanumeric);
            if wordy && text[..at].chars().next_back().is_some_and(|p| p.is_alphanumeric() || p == '\'') {
                continue;
            }
            found = Some((from.len(), to.as_str()));
        }
        if let Some((n, to)) = found {
            let at = text.len() - n;
            return Some((at..start, match_case(&text[at..], to)));
        }
    }
    if opts.emoticons && c.is_whitespace() {
        // On its own: after a space or at the start ("f(x:)" stays).
        let longest = EMOTICONS
            .iter()
            .filter(|(from, _)| {
                text.len() >= from.len()
                    && text.is_char_boundary(text.len() - from.len())
                    && text[text.len() - from.len()..].eq_ignore_ascii_case(from)
                    && text[..text.len() - from.len()].chars().next_back().is_none_or(char::is_whitespace)
            })
            .max_by_key(|(from, _)| from.len());
        if let Some((from, to)) = longest {
            return Some((text.len() - from.len()..start, to.to_string()));
        }
    }
    if opts.capitalize_sentences && c.is_whitespace() {
        // The word just finished, if it starts a sentence and is lower case.
        let word_start = text.rfind(|ch: char| ch.is_whitespace()).map_or(0, |i| i + 1);
        let word = &text[word_start..];
        let first = word.chars().next()?;
        if !first.is_lowercase() || !word.chars().all(|ch| ch.is_alphabetic() || ch == '\'') {
            return None;
        }
        let before_word = text[..word_start].trim_end();
        let starts = before_word.is_empty() || before_word.ends_with(['.', '!', '?']);
        // "e.g. word", "Dr. word": an abbreviation, not the end of a sentence.
        let last_word = before_word.rsplit(char::is_whitespace).next().unwrap_or("");
        let abbreviation =
            last_word.trim_end_matches('.').contains('.') || last_word.len() <= 3 && last_word.ends_with('.');
        if starts && !(abbreviation && !before_word.is_empty()) {
            let upper: String = first.to_uppercase().collect();
            return Some((word_start..word_start + first.len_utf8(), upper));
        }
    }
    None
}

/// After `typed` was typed at the caret: makes the correction, if any, as its
/// own undo step. Returns whether it changed anything.
pub fn autocorrect(ed: &mut Editor, typed: &str, opts: &Autocorrect) -> Result<bool, EditError> {
    if !ed.sel.is_collapsed() {
        return Ok(false);
    }
    let at = ed.sel.focus;
    let Some(p) = ed.doc.para(at.para) else { return Ok(false) };
    let Some(before) = p.text.get(..at.offset) else { return Ok(false) };
    if !before.ends_with(typed) {
        return Ok(false);
    }
    let Some((range, new)) = correction(before, typed, opts) else { return Ok(false) };
    let fmt = p.fmt_at(range.start + usize::from(range.start < range.end));
    let para = at.para;
    let caret = at.offset - range.len() + new.len();
    ed.seal();
    ed.transact("Autocorrect", None, |tx| {
        tx.step(Step::Replace { para, offset: range.start, remove: range.len(), insert: Span::plain(new, fmt) })?;
        Ok(Selection::caret(Pos::new(para, caret)))
    })?;
    ed.seal();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(before: &str, typed: &str) -> Option<String> {
        let opts = Autocorrect { capitalize_sentences: true, ..Autocorrect::default() };
        let (r, new) = correction(before, typed, &opts)?;
        let mut s = before.to_string();
        s.replace_range(r, &new);
        Some(s)
    }

    #[test]
    fn corrects_words_and_symbols() {
        assert_eq!(fix("I saw teh ", " ").as_deref(), Some("I saw the "));
        assert_eq!(fix("I saw Teh,", ",").as_deref(), Some("I saw The,"));
        assert_eq!(fix("Copyright (c) ", " ").as_deref(), Some("Copyright © "));
        assert_eq!(fix("wait... ", " ").as_deref(), Some("wait… "));
        assert_eq!(fix("a -- b", " ").as_deref(), None); // "b" isn't a word end
        assert_eq!(fix("a -- ", " ").as_deref(), Some("a – "));
        assert_eq!(fix("add 1/2 ", " ").as_deref(), Some("add ½ "));
        // Only whole words and numbers.
        assert_eq!(fix("I saw steh ", " "), None);
        assert_eq!(fix("page 11/2 ", " "), None);
    }

    #[test]
    fn emoticons_become_emoji_on_their_own() {
        assert_eq!(fix("Great :) ", " ").as_deref(), Some("Great 🙂 "));
        assert_eq!(fix(";) ", " ").as_deref(), Some("😉 "));
        assert_eq!(fix("I :-( ", " ").as_deref(), Some("I 🙁 "));
        assert_eq!(fix("so :'( ", " ").as_deref(), Some("so 😢 "));
        assert_eq!(fix("love <3 ", " ").as_deref(), Some("love ❤\u{fe0f} "));
        // Not inside other text, and only when switched on.
        assert_eq!(fix("f(x:) ", " "), None);
        let off = Autocorrect { emoticons: false, ..Autocorrect::default() };
        assert_eq!(correction("Great :) ", " ", &off), None);
        assert_eq!(emoticons_for("😉"), vec![";)", ";-)"]);
    }

    #[test]
    fn curly_quotes() {
        assert_eq!(fix("She said \"", "\"").as_deref(), Some("She said \u{201c}"));
        assert_eq!(fix("She said \u{201c}hi\"", "\"").as_deref(), Some("She said \u{201c}hi\u{201d}"));
        assert_eq!(fix("don'", "'").as_deref(), Some("don\u{2019}"));
        assert_eq!(fix("'", "'").as_deref(), Some("\u{2018}"));
    }

    #[test]
    fn capitals_start_sentences_but_not_after_abbreviations() {
        assert_eq!(fix("the ", " ").as_deref(), Some("The "));
        assert_eq!(fix("It ended. then ", " ").as_deref(), Some("It ended. Then "));
        assert_eq!(fix("It is e.g. then ", " "), None);
        assert_eq!(fix("See Dr. smith ", " "), None);
        assert_eq!(fix("It went then ", " "), None);
        // Off by default.
        assert_eq!(correction("the ", " ", &Autocorrect::default()), None);
    }

    #[test]
    fn one_undo_gives_back_what_was_typed() {
        let mut ed = Editor::new(crate::Document::new());
        let para = ed.doc.paragraphs().next().unwrap().id;
        ed.sel = Selection::caret(Pos::new(para, 0));
        for c in "teh ".chars() {
            crate::ops::insert_text(&mut ed, &c.to_string(), None).unwrap();
            autocorrect(&mut ed, &c.to_string(), &Autocorrect::default()).unwrap();
        }
        assert_eq!(ed.doc.para(para).unwrap().text, "the ");
        assert_eq!(ed.sel.focus.offset, 4);
        ed.undo();
        assert_eq!(ed.doc.para(para).unwrap().text, "teh ");
    }
}
