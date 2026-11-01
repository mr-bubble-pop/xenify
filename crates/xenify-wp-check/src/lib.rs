//! Proofing for Xenify Write: [`xenify_grammar`] (Harper plus Xenify's own
//! rules, profiles and explanations) applied to Write documents—paragraph
//! ids, the document's ignore lists, and code paragraphs skipped.
//! Everything runs locally; nothing leaves the machine.

pub use xenify_grammar::{
    Category, Dialect, Explanation, Feature, Level, Profile, Profiles, RULE_GROUPS, Readability, Settings,
    built_in_profiles, clarity, explain, is_useful_description, own_rules, read_profile, readability, rule_group,
    rule_title, slug, sound_alike, write_profile,
};
pub mod lexicon;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ops::Range;
use xenify_wp_doc::{Document, NodeId};

/// One problem found, in a paragraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub para: NodeId,
    /// Byte range in the paragraph's text.
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
    fn new(para: NodeId, i: xenify_grammar::Issue) -> Self {
        Issue {
            para,
            range: i.range,
            category: i.category,
            rule: i.rule,
            message: i.message,
            suggestions: i.suggestions,
            feature: i.feature,
            strong: i.strong,
        }
    }

    /// A key that identifies this issue in this sentence context (for "ignore once").
    pub fn ignore_key(&self, text: &str) -> String {
        xenify_grammar::Issue {
            range: self.range.clone(),
            category: self.category,
            rule: self.rule.clone(),
            message: String::new(),
            suggestions: Vec::new(),
            feature: None,
            strong: false,
        }
        .ignore_key(text)
    }
}

/// The checker, for Write documents.
pub struct Checker {
    inner: xenify_grammar::Checker,
}

impl Checker {
    /// A checker with `settings`, accepting the words in `user_words`.
    pub fn new(settings: Settings, user_words: &[String]) -> Self {
        Self { inner: xenify_grammar::Checker::new(settings, user_words) }
    }

    pub fn settings(&self) -> &Settings {
        self.inner.settings()
    }

    /// Every rule with its description (for the settings UI).
    pub fn rules(&self) -> Vec<(String, String)> {
        self.inner.rules()
    }

    /// Issues in paragraph `id` with text `text`, filtered by the settings
    /// and the document's ignore lists.
    pub fn check_paragraph(&mut self, id: NodeId, text: &str, ignore: &Ignores) -> Vec<Issue> {
        self.inner
            .check(text)
            .into_iter()
            .map(|i| Issue::new(id, i))
            .filter(|i| {
                let word = text.get(i.range.clone()).unwrap_or("");
                !(i.category == Category::Spelling && ignore.words.contains(word))
                    && !ignore.issues.contains(&i.ignore_key(text))
            })
            .collect()
    }

    /// Every issue in `doc` (code paragraphs are skipped).
    pub fn check_document(&mut self, doc: &Document) -> Vec<Issue> {
        let ignore = Ignores::of(doc);
        let paras: Vec<(NodeId, String)> = doc
            .paragraphs()
            .filter(|p| p.style != xenify_wp_doc::StyleSheet::CODE)
            .map(|p| (p.id, p.text.clone()))
            .collect();
        let mut out: Vec<Issue> = paras.iter().flat_map(|(id, t)| self.check_paragraph(*id, t, &ignore)).collect();
        out.extend(self.check_whole(&paras, &ignore));
        out
    }

    /// The whole-document checks (a buried request, acronyms used before
    /// they're spelled out) over the paragraphs in order.
    pub fn check_whole(&self, paras: &[(NodeId, String)], ignore: &Ignores) -> Vec<Issue> {
        let texts: Vec<String> = paras.iter().map(|(_, t)| t.clone()).collect();
        self.inner
            .check_whole(&texts)
            .into_iter()
            .map(|(i, x)| Issue::new(paras[i].0, x))
            .filter(|i| {
                paras
                    .iter()
                    .find(|(id, _)| *id == i.para)
                    .is_none_or(|(_, t)| !ignore.issues.contains(&i.ignore_key(t)))
            })
            .collect()
    }
}

pub use xenify_grammar_spotter::{Finding, Fix, Spotter};

/// The optional mistake spotter (an offline model), as the user set it up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpotterPrefs {
    pub enabled: bool,
    /// The model folder.
    pub path: String,
    /// How sure the model must be (0.5–0.99) before a spot is shown.
    pub threshold: f32,
}

/// How sure the model must be by default: 97% suits the B7 model (it
/// catches more real mistakes than at 90% with fewer false alarms on books).
pub const DEFAULT_THRESHOLD: f32 = 0.97;

impl Default for SpotterPrefs {
    fn default() -> Self {
        // On by default: it only runs when a model is installed.
        Self { enabled: true, path: String::new(), threshold: DEFAULT_THRESHOLD }
    }
}

impl SpotterPrefs {
    /// The model folder to load: [`SpotterPrefs::path`]; or when that's
    /// empty the standard place, `~/.local/share/xenify/spotter`, if a model
    /// is there; else the one Xenify Hub installed for all the apps
    /// ([`hub_spotter_dir`]); else the one installed with the app
    /// ([`set_bundled_spotter_dir`]).
    pub fn model_dir(&self) -> std::path::PathBuf {
        if !self.path.trim().is_empty() {
            return self.path.trim().into();
        }
        let own = default_spotter_dir();
        if own.join("config.json").exists() {
            return own;
        }
        if let Some(hub) = hub_spotter_dir() {
            return hub;
        }
        BUNDLED_SPOTTER.get().cloned().unwrap_or(own)
    }
}

static BUNDLED_SPOTTER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Where the app's installer put the spotter model (its resources folder);
/// used when there's none in the standard place. Only the first call counts.
pub fn set_bundled_spotter_dir(dir: std::path::PathBuf) {
    if dir.join("config.json").exists() {
        let _ = BUNDLED_SPOTTER.set(dir);
    }
}

/// The model Xenify Hub installed once for every app (its "checker" part):
/// `<data>/xenify/hub/apps/checker/<version>/spotter`, the version in use
/// being the one in `current.json` there. None when the Hub hasn't.
pub fn hub_spotter_dir() -> Option<std::path::PathBuf> {
    let hub = match std::env::var_os("XENIFY_DATA_DIR") {
        Some(d) => std::path::PathBuf::from(d).join("hub"),
        None => data_dir().join("hub"),
    };
    let part = hub.join("apps").join("checker");
    let current = std::fs::read_to_string(part.join("current.json")).ok()?;
    // {"version": "7.0.0", ...}: the first "version" is the one in use.
    let after = &current[current.find("\"version\"")? + 9..];
    let start = after.find('"')? + 1;
    let version = &after[start..start + after[start..].find('"')?];
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) {
        return None;
    }
    let dir = part.join(version).join("spotter");
    dir.join("config.json").exists().then_some(dir)
}

/// Where the spotter model goes unless chosen otherwise: `xenify/spotter`
/// in [`data_dir`].
pub fn default_spotter_dir() -> std::path::PathBuf {
    data_dir().join("spotter")
}

/// Xenify's own data on this computer (the model, autosaves): on Linux
/// `$XDG_DATA_HOME/xenify` (`~/.local/share/xenify`), on a Mac
/// `~/Library/Application Support/xenify`, on Windows `%LOCALAPPDATA%\xenify`.
pub fn data_dir() -> std::path::PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| ".".into()).join("xenify")
}

/// The user's settings files (dictionary, writing profiles): on Linux
/// `$XDG_CONFIG_HOME/xenify` (`~/.config/xenify`), on a Mac
/// `~/Library/Application Support/xenify`, on Windows `%APPDATA%\xenify`.
pub fn config_dir() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(|| ".".into()).join("xenify")
}

/// The user's home folder.
pub fn home_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| ".".into())
}

/// A spotter model to ask: one loaded in this app ([`Spotter`]) or the one
/// the shared background checker holds ([`SharedSpotter`]).
pub trait Spots {
    /// The spots in `text`, each with up to `fixes` suggestions, without the
    /// kinds of mistake in `hidden`.
    fn spots(
        &self,
        text: &str,
        threshold: f32,
        fixes: usize,
        hidden: &[String],
    ) -> Result<Vec<(Finding, Vec<Fix>)>, String>;
}

impl Spots for Spotter {
    fn spots(
        &self,
        text: &str,
        threshold: f32,
        fixes: usize,
        hidden: &[String],
    ) -> Result<Vec<(Finding, Vec<Fix>)>, String> {
        xenify_checker::spot_with_fixes(self, text, threshold, fixes, hidden)
    }
}

/// The model in `dir`, held by the shared background checker (`xenify-checker`)
/// so all Xenify windows and apps use one copy.
pub struct SharedSpotter {
    client: xenify_checker::Client,
    dir: std::path::PathBuf,
}

impl SharedSpotter {
    /// Connects to the shared checker (starting it if needed) and has it load
    /// `dir`; also the number of kinds of mistake the model knows. `Err(None)`
    /// when there's no shared checker (load the model in-app instead), and
    /// `Err(Some(e))` when the model itself didn't load.
    pub fn open(dir: &std::path::Path) -> Result<(Self, usize), Option<String>> {
        let client = xenify_checker::Client::connect().map_err(|_| None)?;
        let codes = client.load(dir).map_err(Some)?;
        Ok((Self { client, dir: dir.to_path_buf() }, codes))
    }
}

impl Spots for SharedSpotter {
    fn spots(
        &self,
        text: &str,
        threshold: f32,
        fixes: usize,
        hidden: &[String],
    ) -> Result<Vec<(Finding, Vec<Fix>)>, String> {
        self.client.spot(&self.dir, text, threshold, fixes, hidden)
    }
}

/// How [`SpotterRun`] filters what the spotter finds.
#[derive(Clone, Copy, Debug)]
pub struct SpotOptions {
    /// How sure the model must be (0–1).
    pub threshold: f32,
    /// Also show optional commas (the profile's setting).
    pub optional_commas: bool,
    /// Also show sentences that may not make sense (the profile's setting;
    /// these have no fix, so they're off by default).
    pub sentence_sense: bool,
}

/// The model's code for a sentence that may not make sense.
pub const GARBLED: &str = "garbled";

/// How sure the model must be before saying a sentence may not make sense
/// (stricter than other kinds: there's no fix to check against).
pub const GARBLED_THRESHOLD: f32 = 0.99;

/// Runs the spotter over paragraphs (cached by text), adding its findings
/// where no rule already marks the words. Optional commas are left out
/// unless the profile asks for them.
#[derive(Default)]
pub struct SpotterRun {
    cache: std::collections::HashMap<(String, u32, bool), Vec<xenify_grammar::Issue>>,
}

impl SpotterRun {
    pub fn paragraph(
        &mut self,
        spotter: &dyn Spots,
        opts: SpotOptions,
        id: NodeId,
        text: &str,
        existing: &[Issue],
        ignore: &Ignores,
    ) -> Vec<Issue> {
        let SpotOptions { threshold, optional_commas, sentence_sense } = opts;
        let key = (text.to_string(), threshold.to_bits(), sentence_sense);
        if !self.cache.contains_key(&key) {
            let mut found = Vec::new();
            let hidden = if sentence_sense { Vec::new() } else { vec![GARBLED.to_string()] };
            for (f, fixes) in spotter.spots(text, threshold, 3, &hidden).unwrap_or_default() {
                if spotter_false_alarm(&f.code, text, f.range.clone())
                    || f.code == GARBLED && f.confidence < GARBLED_THRESHOLD
                {
                    continue;
                }
                let ex = xenify_grammar::explain_code(&f.code);
                let title = ex.as_ref().map(|e| e.title.clone()).unwrap_or_else(|| f.code.replace('_', " "));
                // Suggestions replace the whole spot, so show each as the spot rewritten.
                let suggestions: Vec<String> = fixes
                    .iter()
                    .filter(|x| xenify_grammar::sensible_suggestion(&text[x.range.clone()], &x.text))
                    .filter(|x| x.range.start >= f.range.start.saturating_sub(1) && x.range.end <= f.range.end + 1)
                    .map(|x| {
                        let lo = f.range.start.min(x.range.start);
                        let hi = f.range.end.max(x.range.end);
                        let mut t = text[lo..hi].to_string();
                        t.replace_range(x.range.start - lo..x.range.end - lo, &x.text);
                        t
                    })
                    .collect();
                let range = {
                    let lo = fixes
                        .iter()
                        .map(|x| x.range.start)
                        .chain([f.range.start])
                        .min()
                        .unwrap_or(f.range.start)
                        .max(f.range.start.saturating_sub(1));
                    lo.min(f.range.start)..f.range.end
                };
                let message = if f.code == GARBLED {
                    format!(
                        "This part of the sentence may not make sense: words left from an edit, a missing word or words out of order? ({:.0}% sure)",
                        f.confidence * 100.0
                    )
                } else if suggestions.is_empty() {
                    format!("This may be a mistake: {}. ({:.0}% sure)", title.to_lowercase(), f.confidence * 100.0)
                } else {
                    format!(
                        "This may be a mistake: {}. One of the suggestions may fit. ({:.0}% sure)",
                        title.to_lowercase(),
                        f.confidence * 100.0
                    )
                };
                found.push(xenify_grammar::Issue {
                    range,
                    category: Category::Grammar,
                    rule: format!("Spotter:{}", f.code),
                    message,
                    suggestions,
                    feature: None,
                    strong: false,
                });
            }
            if self.cache.len() > 5000 {
                self.cache.clear();
            }
            self.cache.insert(key.clone(), found);
        }
        self.cache[&key]
            .iter()
            .filter(|i| optional_commas || !xenify_grammar::is_optional_punctuation(&i.rule))
            .filter(|i| {
                !existing.iter().any(|e| e.para == id && e.range.start < i.range.end && i.range.start < e.range.end)
            })
            .map(|i| Issue::new(id, i.clone()))
            .filter(|i| !ignore.issues.contains(&i.ignore_key(text)))
            .collect()
    }
}

/// Spotter findings known to be wrong: a spot starting or ending inside a
/// word (the model marked part of a word like "sexton"), and "a"/"an" before an
/// abbreviation or number, which depends on how it's said ("an MA",
/// "an 1177 battle"); the a/an rule handles those.
fn spotter_false_alarm(code: &str, text: &str, range: std::ops::Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let first = text[range.start..].chars().next();
    let last = text[..range.end].chars().next_back();
    let after = text[range.end..].chars().next();
    let joined =
        |a: Option<char>, b: Option<char>| a.is_some_and(char::is_alphanumeric) && b.is_some_and(char::is_alphanumeric);
    // Starts or ends inside a word ("⟦A⟧lamanni", "⟦duck-⟧billed").
    if joined(before, first) || joined(last, after) || (last == Some('-') && after.is_some_and(char::is_alphanumeric)) {
        return true;
    }
    if code == "article_wrong" {
        let mut words = text[range.start..].split_whitespace();
        if let (Some(a), Some(next)) = (words.next(), words.next())
            && matches!(a.to_lowercase().as_str(), "a" | "an")
        {
            let next = next.trim_matches(|c: char| !c.is_alphanumeric());
            let abbreviation = next.chars().filter(|c| c.is_alphabetic()).count() >= 2
                && next.chars().all(|c| c.is_uppercase() || c.is_ascii_digit());
            return abbreviation || next.starts_with(|c: char| c.is_ascii_digit());
        }
    }
    false
}

/// What the document says to ignore.
#[derive(Clone, Debug, Default)]
pub struct Ignores {
    pub words: BTreeSet<String>,
    pub issues: BTreeSet<String>,
}

impl Ignores {
    pub fn of(doc: &Document) -> Self {
        Self {
            words: doc.meta.ignore_words.iter().cloned().collect(),
            issues: doc.meta.ignore_issues.iter().cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hubs_shared_model_is_found() {
        let root = std::env::temp_dir().join(format!("xenify-hub-spotter-{}", std::process::id()));
        let part = root.join("hub/apps/checker");
        std::fs::create_dir_all(part.join("7.0.0/spotter")).unwrap();
        std::fs::write(part.join("7.0.0/spotter/config.json"), "{}").unwrap();
        std::fs::write(
            part.join("current.json"),
            r#"{"version": "7.0.0", "run": "spotter/config.json", "previous": {"version": "6.0.0"}}"#,
        )
        .unwrap();
        // SAFETY: no other test in this crate reads or sets XENIFY_DATA_DIR.
        unsafe { std::env::set_var("XENIFY_DATA_DIR", &root) };
        assert_eq!(hub_spotter_dir(), Some(part.join("7.0.0/spotter")));
        std::fs::write(part.join("current.json"), r#"{"version": "../x"}"#).unwrap();
        assert_eq!(hub_spotter_dir(), None);
        unsafe { std::env::remove_var("XENIFY_DATA_DIR") };
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn spotter_false_alarms_are_dropped() {
        let t = "Thomas Chatterton was born where the office of sexton was held.";
        let at = t.find("ton was").unwrap();
        assert!(spotter_false_alarm("article_missing", t, at..at + 3));
        let t = "It was against the Alamanni and the duck-billed ones.";
        let at = t.find("Alamanni").unwrap();
        assert!(spotter_false_alarm("article_wrong", t, at..at + 1));
        let at = t.find("duck-").unwrap();
        assert!(spotter_false_alarm("article_wrong", t, at..at + 5));
        for (t, w) in [("He was an MA student.", "an MA"), ("It was an 1177 conflict.", "an 1177")] {
            let at = t.find(w).unwrap();
            assert!(spotter_false_alarm("article_wrong", t, at..at + w.len()), "{t}");
        }
        let t = "He played on beach.";
        let at = t.find("on beach").unwrap();
        assert!(!spotter_false_alarm("article_missing", t, at..at + 8));
        let t = "It was a apple.";
        assert!(!spotter_false_alarm("article_wrong", t, 7..14));
    }

    #[test]
    fn spotter_commas_follow_the_profile() {
        // Set XENIFY_SPOTTER to a model folder to run this.
        let Some(dir) = std::env::var_os("XENIFY_SPOTTER") else { return };
        let spotter = Spotter::load(std::path::Path::new(&dir)).expect("model");
        let text = "If you can come let me know by Friday.";
        let commas = |optional| {
            SpotterRun::default()
                .paragraph(
                    &spotter,
                    SpotOptions { threshold: 0.5, optional_commas: optional, sentence_sense: false },
                    NodeId(0),
                    text,
                    &[],
                    &Ignores::default(),
                )
                .iter()
                .any(|i| i.rule == "Spotter:comma_intro")
        };
        assert!(commas(true), "the model should spot the optional comma");
        assert!(!commas(false));
    }

    #[test]
    fn documents_ignores_and_paragraph_ids() {
        let doc = Document::from_plain_text("This is an test of the speling checker.\nThe parleyvello is fine.");
        let mut c = Checker::new(Settings::default(), &[]);
        let issues = c.check_document(&doc);
        let p1 = doc.paragraphs().nth(1).unwrap();
        assert!(issues.iter().any(|i| i.para == p1.id && i.category == Category::Spelling));
        let mut doc2 = doc.clone();
        doc2.meta.ignore_words.push("parleyvello".into());
        assert!(c.check_document(&doc2).iter().all(|i| !(i.para == p1.id && i.category == Category::Spelling)));
    }
}
