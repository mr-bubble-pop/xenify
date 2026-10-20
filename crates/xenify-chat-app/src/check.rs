//! Spelling and grammar for what you write: Xenify Write's checker with the
//! profile that fits (a chat message is checked as chat, an email as email),
//! the dictionary shared with Write, and, when switched on, the AI mistake
//! spotter shared by all Xenify apps. Everything runs on this computer.
//!
//! The checker lives on its own thread (it isn't meant to be shared between
//! threads); commands send it requests and wait for the answer.

use xenify_wp_check::{
    Category, Checker, DEFAULT_THRESHOLD, Ignores, Issue, Profiles, Settings, SharedSpotter, SpotOptions, SpotterRun,
};
use xenify_wp_doc::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use tauri::State;
use tokio::sync::oneshot;

/// One thing to look at, with its place given in UTF-16 units (as the
/// screens count text).
#[derive(Clone, Debug, Serialize)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    /// "spelling", "grammar", "style", "readability" or "register".
    pub kind: &'static str,
    pub message: String,
    pub suggestions: Vec<String>,
    /// For "Ignore": hides this one mistake in this text.
    pub key: String,
    /// The word itself, for "Add to dictionary" (spelling only).
    pub word: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Options {
    /// "chat" or "email".
    #[serde(default)]
    pub kind: String,
    #[serde(default = "yes")]
    pub spelling: bool,
    #[serde(default = "yes")]
    pub grammar: bool,
    #[serde(default)]
    pub spotter: bool,
    /// Keys of mistakes you chose to ignore.
    #[serde(default)]
    pub ignored: Vec<String>,
}

fn yes() -> bool {
    true
}

enum Job {
    Check(String, Options, oneshot::Sender<Vec<Found>>),
    Words(oneshot::Sender<Vec<String>>),
    SetWords(Vec<String>),
    Spotter(oneshot::Sender<&'static str>),
}

pub struct Check {
    tx: Mutex<Sender<Job>>,
}

fn dictionary_path() -> PathBuf {
    xenify_wp_check::config_dir().join("dictionary.txt")
}

/// The words you've added (shared with Xenify Write).
fn read_words() -> Vec<String> {
    std::fs::read_to_string(dictionary_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn write_words(words: &[String]) {
    let p = dictionary_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, words.join("\n") + "\n");
}

fn kind_of(c: Category) -> &'static str {
    match c {
        Category::Spelling => "spelling",
        Category::Grammar => "grammar",
        Category::Style => "style",
        Category::Readability => "readability",
        Category::Register => "register",
    }
}

/// UTF-16 position of byte offset `b` in `text`.
fn utf16_at(text: &str, b: usize) -> usize {
    text.get(..b).map(|s| s.encode_utf16().count()).unwrap_or(0)
}

struct Worker {
    profiles: Profiles,
    words: Vec<String>,
    /// One checker per profile, rebuilt when the dictionary changes.
    checkers: HashMap<&'static str, Checker>,
    run: SpotterRun,
    spotter: Option<SharedSpotter>,
    /// Why the spotter isn't in use ("off", "not installed", "unavailable").
    spotter_state: &'static str,
}

impl Worker {
    fn profile_id(kind: &str) -> &'static str {
        if kind == "email" { "workplace-email" } else { "workplace-chat" }
    }

    fn check(&mut self, text: &str, o: &Options) -> Vec<Found> {
        let id = Self::profile_id(&o.kind);
        let profile = self.profiles.get(id);
        let (optional_commas, sentence_sense) = (profile.optional_commas, profile.sentence_sense);
        let words = &self.words;
        let checker =
            self.checkers.entry(id).or_insert_with(|| Checker::new(Settings { profile, ..Default::default() }, words));
        let ignore = Ignores { issues: o.ignored.iter().cloned().collect(), ..Ignores::default() };
        let mut issues: Vec<Issue> = Vec::new();
        // Each line is a paragraph.
        let mut offset = 0;
        for (n, line) in text.split('\n').enumerate() {
            for mut i in checker.check_paragraph(NodeId(n as u64), line, &ignore) {
                i.range = i.range.start + offset..i.range.end + offset;
                issues.push(i);
            }
            offset += line.len() + 1;
        }
        if o.spotter && o.grammar {
            self.spot(text, &mut issues, &ignore, optional_commas, sentence_sense);
        }
        issues.sort_by_key(|i| i.range.start);
        issues
            .into_iter()
            .filter(|i| if i.category == Category::Spelling { o.spelling } else { o.grammar })
            .filter(|i| !o.ignored.contains(&i.ignore_key(text)))
            .map(|i| Found {
                start: utf16_at(text, i.range.start),
                end: utf16_at(text, i.range.end),
                kind: kind_of(i.category),
                key: i.ignore_key(text),
                word: text.get(i.range.clone()).unwrap_or_default().to_string(),
                message: i.message,
                suggestions: i.suggestions.into_iter().take(5).collect(),
            })
            .collect()
    }

    fn spot(
        &mut self,
        text: &str,
        issues: &mut Vec<Issue>,
        ignore: &Ignores,
        optional_commas: bool,
        sentence_sense: bool,
    ) {
        if self.spotter.is_none() && self.spotter_state != "unavailable" {
            let dir = xenify_wp_check::default_spotter_dir();
            if !dir.join("config.json").exists() {
                self.spotter_state = "not installed";
                return;
            }
            match SharedSpotter::open(&dir) {
                Ok((s, _)) => {
                    self.spotter = Some(s);
                    self.spotter_state = "on";
                }
                Err(_) => self.spotter_state = "unavailable",
            }
        }
        let Some(s) = &self.spotter else { return };
        let opts = SpotOptions { threshold: DEFAULT_THRESHOLD, optional_commas, sentence_sense };
        // The spotter reads whole paragraphs too.
        let mut offset = 0;
        let mut found = Vec::new();
        for (n, line) in text.split('\n').enumerate() {
            let here: Vec<Issue> = issues
                .iter()
                .filter(|i| i.range.start >= offset && i.range.end <= offset + line.len())
                .map(|i| Issue { range: i.range.start - offset..i.range.end - offset, ..i.clone() })
                .collect();
            for mut i in self.run.paragraph(s, opts, NodeId(n as u64), line, &here, ignore) {
                i.range = i.range.start + offset..i.range.end + offset;
                found.push(i);
            }
            offset += line.len() + 1;
        }
        issues.extend(found);
    }
}

impl Check {
    pub fn new() -> Check {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("xenify-checking".into())
            .spawn(move || {
                let mut w = Worker {
                    profiles: Profiles::load(Some(xenify_wp_check::config_dir().join("proofing-profiles"))),
                    words: read_words(),
                    checkers: HashMap::new(),
                    run: SpotterRun::default(),
                    spotter: None,
                    spotter_state: "off",
                };
                // Get the chat checker ready now, not on the first message.
                w.check(
                    "Hello.",
                    &Options { kind: "chat".into(), spelling: true, grammar: true, ..Options::default() },
                );
                while let Ok(job) = rx.recv() {
                    match job {
                        Job::Check(text, o, reply) => {
                            let _ = reply.send(w.check(&text, &o));
                        }
                        Job::Words(reply) => {
                            // Xenify Write may have added some meanwhile.
                            let now = read_words();
                            if now != w.words {
                                w.words = now;
                                w.checkers.clear();
                            }
                            let _ = reply.send(w.words.clone());
                        }
                        Job::SetWords(words) => {
                            write_words(&words);
                            w.words = words;
                            w.checkers.clear();
                        }
                        Job::Spotter(reply) => {
                            let _ = reply.send(w.spotter_state);
                        }
                    }
                }
            })
            .expect("the checking thread starts");
        Check { tx: Mutex::new(tx) }
    }

    fn send(&self, job: Job) {
        let _ = self.tx.lock().unwrap().send(job);
    }
}

/// The things to look at in `text`.
#[tauri::command]
pub async fn check(check: State<'_, Check>, text: String, options: Options) -> Result<Vec<Found>, String> {
    let (tx, rx) = oneshot::channel();
    check.send(Job::Check(text, options, tx));
    rx.await.map_err(|_| "checking stopped".to_string())
}

/// Your dictionary (shared with Xenify Write).
#[tauri::command]
pub async fn words(check: State<'_, Check>) -> Result<Vec<String>, String> {
    let (tx, rx) = oneshot::channel();
    check.send(Job::Words(tx));
    rx.await.map_err(|_| "checking stopped".to_string())
}

#[tauri::command]
pub async fn add_word(check: State<'_, Check>, word: String) -> Result<Vec<String>, String> {
    let mut list = words(check.clone()).await?;
    let word = word.trim().to_string();
    if !word.is_empty() && !list.contains(&word) {
        list.push(word);
        check.send(Job::SetWords(list.clone()));
    }
    Ok(list)
}

#[tauri::command]
pub async fn remove_word(check: State<'_, Check>, word: String) -> Result<Vec<String>, String> {
    let mut list = words(check.clone()).await?;
    list.retain(|w| *w != word);
    check.send(Job::SetWords(list.clone()));
    Ok(list)
}

/// "on", "off", "not installed" or "unavailable".
#[tauri::command]
pub async fn spotter_state(check: State<'_, Check>) -> Result<&'static str, String> {
    let (tx, rx) = oneshot::channel();
    check.send(Job::Spotter(tx));
    rx.await.map_err(|_| "checking stopped".to_string())
}

/// Credit for the word list behind some explanations.
#[tauri::command]
pub fn checker_credits() -> &'static str {
    xenify_wp_check::lexicon::ATTRIBUTION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_spelling_with_screen_positions() {
        let mut w = Worker {
            profiles: Profiles::load(None),
            words: vec![],
            checkers: HashMap::new(),
            run: SpotterRun::default(),
            spotter: None,
            spotter_state: "off",
        };
        let o = Options { kind: "chat".into(), spelling: true, grammar: true, ..Options::default() };
        // "é" is one UTF-16 unit but two bytes: positions must be in UTF-16.
        let text = "Café first.\nI recieved it";
        let found = w.check(text, &o);
        let f = found.iter().find(|f| f.kind == "spelling").expect("a spelling mistake");
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[f.start..f.end]).unwrap(), "recieved");
        assert!(f.suggestions.iter().any(|s| s == "received"), "{:?}", f.suggestions);
        // Ignoring it, or spelling off, hides it.
        let o2 = Options { ignored: vec![f.key.clone()], ..o.clone() };
        assert!(!w.check(text, &o2).iter().any(|x| x.key == f.key));
        let o3 = Options { spelling: false, ..o };
        assert!(!w.check(text, &o3).iter().any(|x| x.kind == "spelling"));
    }
}
