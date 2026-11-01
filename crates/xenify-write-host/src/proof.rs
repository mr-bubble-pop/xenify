//! Spelling and grammar: Xenify Write's checker on a thread of its own (it
//! isn't meant to be shared between threads), with the dictionary shared
//! with the other Xenify apps. Paragraphs whose text hasn't changed are
//! answered from a cache, so checking after each pause in typing is cheap.
//! On the desktop the mistake spotter (an offline model, when installed)
//! checks after the rules: it finds what rules can't,
//! like missing words ("pass me salt").

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use xenify_wp_check::{Checker, Dialect, Ignores, Issue, Profiles, Settings, SpotterPrefs};
use xenify_wp_doc::NodeId;

/// How checking is set up (part of the settings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProofPrefs {
    pub enabled: bool,
    /// "auto" (this computer's) or a dialect name ("British"…).
    pub dialect: String,
    pub ignore_names: bool,
    /// The profile for documents that haven't chosen one.
    pub default_profile: String,
    /// The mistake spotter.
    pub spotter: SpotterPrefs,
}

impl Default for ProofPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            dialect: "auto".into(),
            ignore_names: true,
            default_profile: "standard".into(),
            spotter: SpotterPrefs::default(),
        }
    }
}

impl ProofPrefs {
    pub fn dialect(&self) -> Dialect {
        Dialect::ALL.into_iter().find(|d| format!("{d:?}") == self.dialect).unwrap_or_else(Dialect::from_locale)
    }
}

/// Bumped whenever a kind of writing (profile) is saved or deleted: the
/// checker reloads them before its next check.
pub static PROFILES_CHANGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Where the person's own kinds of writing are kept.
pub fn profiles_dir() -> PathBuf {
    xenify_wp_check::config_dir().join("proofing-profiles")
}

pub fn dictionary_path() -> PathBuf {
    xenify_wp_check::config_dir().join("dictionary.txt")
}

/// The words you've added (shared with the other Xenify apps).
pub fn read_words() -> Vec<String> {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub prefs: ProofPrefs,
    pub profile: String,
    pub paras: Vec<(NodeId, String)>,
    pub ignore_words: Vec<String>,
    pub ignore_issues: Vec<String>,
}

enum Job {
    Check(Request, Sender<Vec<Issue>>),
    Words(Sender<Vec<String>>),
    SetWords(Vec<String>),
    SpotterStatus(Sender<String>),
    /// A model loaded from bytes (the browser, which has no model folder).
    #[cfg(target_arch = "wasm32")]
    SetSpotter(Result<xenify_wp_check::Spotter, String>),
}

pub struct Proof {
    #[cfg(not(target_arch = "wasm32"))]
    tx: Mutex<Sender<Job>>,
    /// The browser has no threads: the checker runs in the call.
    #[cfg(target_arch = "wasm32")]
    worker: std::cell::RefCell<Worker>,
}

struct Worker {
    profiles: Profiles,
    profiles_seen: u64,
    words: Vec<String>,
    checkers: HashMap<String, Checker>,
    /// (checker key, ignore lists, text) → issues in that paragraph text.
    cache: HashMap<u64, Vec<Issue>>,
    /// The loaded spotter and the folder it came from (`None` inside: it didn't load).
    spotter: Option<(PathBuf, Option<Box<dyn xenify_wp_check::Spots + Send>>)>,
    spotter_run: xenify_wp_check::SpotterRun,
    /// What happened when the spotter last loaded (for Settings).
    spotter_status: String,
}

fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

impl Worker {
    fn check(&mut self, r: Request) -> Vec<Issue> {
        if !r.prefs.enabled {
            return Vec::new();
        }
        let now = PROFILES_CHANGED.load(std::sync::atomic::Ordering::Relaxed);
        if now != self.profiles_seen {
            self.profiles = Profiles::load(Some(profiles_dir()));
            self.profiles_seen = now;
            self.checkers.clear();
            self.cache.clear();
        }
        let profile = self.profiles.get(&r.profile);
        let dialect = r.prefs.dialect();
        let key = format!("{}|{dialect:?}|{}", profile.id, r.prefs.ignore_names);
        let words = &self.words;
        let checker = self.checkers.entry(key.clone()).or_insert_with(|| {
            Checker::new(
                Settings { profile, dialect, ignore_names: r.prefs.ignore_names, enabled: true, ..Default::default() },
                words,
            )
        });
        let ignore = Ignores {
            words: r.ignore_words.iter().cloned().collect(),
            issues: r.ignore_issues.iter().cloned().collect(),
        };
        let ignore_key = hash(&(&key, &r.ignore_words, &r.ignore_issues));
        if self.cache.len() > 20_000 {
            self.cache.clear();
        }
        let mut out = Vec::new();
        for (id, text) in &r.paras {
            let k = hash(&(ignore_key, text));
            let found = self.cache.entry(k).or_insert_with(|| checker.check_paragraph(NodeId(0), text, &ignore));
            out.extend(found.iter().cloned().map(|i| Issue { para: *id, ..i }));
        }
        out.extend(checker.check_whole(&r.paras, &ignore));
        self.spot(&r, &profile_opts(&self.profiles, &r.profile), &ignore, &mut out);
        out
    }

    /// The spotter's findings where no rule already marks the words.
    fn spot(
        &mut self,
        r: &Request,
        (optional_commas, sentence_sense): &(bool, bool),
        ignore: &Ignores,
        out: &mut Vec<Issue>,
    ) {
        let prefs = &r.prefs.spotter;
        if !prefs.enabled {
            // Let go of the model (and the shared checker, which exits once no app uses it).
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.spotter = None;
            }
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.load_spotter(&prefs.model_dir());
        let Some((_, Some(model))) = &self.spotter else { return };
        let opts = xenify_wp_check::SpotOptions {
            threshold: prefs.threshold,
            optional_commas: *optional_commas,
            sentence_sense: *sentence_sense,
        };
        let found: Vec<Issue> = r
            .paras
            .iter()
            .flat_map(|(id, t)| self.spotter_run.paragraph(model.as_ref(), opts, *id, t, out, ignore))
            .collect();
        out.extend(found);
    }

    /// Loads the model in `dir` unless it's the one already loaded.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_spotter(&mut self, dir: &std::path::Path) {
        use xenify_wp_check::{SharedSpotter, Spots, Spotter};
        if self.spotter.as_ref().is_some_and(|(p, _)| p == dir) {
            return;
        }
        // The shared checker holds one copy for every Xenify app; without it, load it here.
        let loaded: Result<(Box<dyn Spots + Send>, String), String> = match SharedSpotter::open(dir) {
            Ok((s, n)) => {
                Ok((Box::new(s), format!("Loaded ({n} kinds of mistake), shared with the other Xenify apps")))
            }
            Err(Some(e)) => Err(e),
            Err(None) => Spotter::load(dir).map(|s| {
                let n = s.codes().len();
                (Box::new(s) as Box<dyn Spots + Send>, format!("Loaded ({n} kinds of mistake)"))
            }),
        };
        self.spotter_status = match &loaded {
            Ok((_, status)) => status.clone(),
            Err(_) if !dir.join("config.json").exists() => {
                format!("Not installed. The model files go in {}", dir.display())
            }
            Err(e) => format!("Couldn't load the model from {}: {e}", dir.display()),
        };
        self.spotter = Some((dir.to_path_buf(), loaded.ok().map(|(m, _)| m)));
    }
}

/// The profile's spotter switches: (optional commas, sentences that may not make sense).
fn profile_opts(profiles: &Profiles, id: &str) -> (bool, bool) {
    let p = profiles.get(id);
    (p.optional_commas, p.sentence_sense)
}

impl Worker {
    fn new() -> Self {
        Worker {
            profiles: Profiles::load(Some(profiles_dir())),
            profiles_seen: PROFILES_CHANGED.load(std::sync::atomic::Ordering::Relaxed),
            words: read_words(),
            checkers: HashMap::new(),
            cache: HashMap::new(),
            spotter: None,
            spotter_run: Default::default(),
            spotter_status: String::new(),
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Check(r, reply) => {
                let _ = reply.send(self.check(r));
            }
            Job::Words(reply) => {
                let _ = reply.send(self.words.clone());
            }
            Job::SpotterStatus(reply) => {
                let _ = reply.send(self.spotter_status.clone());
            }
            #[cfg(target_arch = "wasm32")]
            Job::SetSpotter(loaded) => {
                self.spotter_status = match &loaded {
                    Ok(s) => format!("Loaded ({} kinds of mistake)", s.codes().len()),
                    Err(e) => format!("Couldn't load the model: {e}"),
                };
                self.spotter =
                    Some((PathBuf::new(), loaded.ok().map(|s| Box::new(s) as Box<dyn xenify_wp_check::Spots + Send>)));
                self.cache.clear();
            }
            Job::SetWords(words) => {
                write_words(&words);
                self.words = words;
                // Checkers know the words they were made with.
                self.checkers.clear();
                self.cache.clear();
            }
        }
    }
}

impl Default for Proof {
    fn default() -> Self {
        Self::new()
    }
}

impl Proof {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Self {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("proofing".into())
            .spawn(move || {
                let mut w = Worker::new();
                for job in rx {
                    w.handle(job);
                }
            })
            .expect("couldn't start the spelling checker");
        Self { tx: Mutex::new(tx) }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Self {
        Self { worker: std::cell::RefCell::new(Worker::new()) }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, job: Job) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(job);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, job: Job) {
        self.worker.borrow_mut().handle(job);
    }

    /// Checks the paragraphs (blocks until done; call off the main thread).
    pub fn check(&self, r: Request) -> Vec<Issue> {
        let (tx, rx) = channel();
        self.send(Job::Check(r, tx));
        rx.recv().unwrap_or_default()
    }

    /// What happened when the mistake spotter last loaded ("" before the first check).
    pub fn spotter_status(&self) -> String {
        let (tx, rx) = channel();
        self.send(Job::SpotterStatus(tx));
        rx.recv().unwrap_or_default()
    }

    /// Uses a model loaded from the bytes of its files (the browser
    /// downloads it; see [`xenify_wp_check::Spotter::from_bytes`]).
    #[cfg(target_arch = "wasm32")]
    pub fn set_spotter(&self, config: &[u8], tokenizer: &[u8], weights: Vec<u8>) {
        self.send(Job::SetSpotter(xenify_wp_check::Spotter::from_bytes(config, tokenizer, weights)));
    }

    pub fn words(&self) -> Vec<String> {
        let (tx, rx) = channel();
        self.send(Job::Words(tx));
        rx.recv().unwrap_or_default()
    }

    pub fn set_words(&self, mut words: Vec<String>) {
        words.sort_by_key(|w| w.to_lowercase());
        words.dedup();
        self.send(Job::SetWords(words));
    }
}

/// A word's meanings, other words already in its form, and opposites, from
/// the built-in dictionary: `{ base, pronunciation, meanings }`.
pub fn look_up(word: &str) -> serde_json::Value {
    let lx = xenify_wp_check::lexicon::lexicon();
    let groups = lx.synonyms(word);
    let found = lx.lookup(word);
    let mut meanings = Vec::new();
    for f in &found.entries {
        for m in &f.meanings {
            let swaps = groups
                .iter()
                .find(|(p, d, _)| *p == f.pos && *d == m.definition)
                .map(|g| g.2.clone())
                .unwrap_or_default();
            meanings.push(serde_json::json!({
                "pos": f.pos.name(), "definition": m.definition,
                "example": m.examples.first(), "swaps": swaps.into_iter().take(8).collect::<Vec<_>>(),
                "opposites": m.antonyms.iter().take(4).collect::<Vec<_>>(),
            }));
        }
    }
    serde_json::json!({
        "base": found.entries.first().map(|f| f.word.clone()).unwrap_or_else(|| word.to_string()),
        "pronunciation": found.entries.first().map(|f| f.pronunciation.clone()).unwrap_or_default(),
        "meanings": meanings,
    })
}
