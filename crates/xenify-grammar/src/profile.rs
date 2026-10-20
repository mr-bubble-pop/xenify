//! Writing profiles: what the checker should point out for a kind of
//! writing (an academic paper, a work email, a social post…).
//!
//! A profile sets, for each informal [`Feature`] of writing, whether it's
//! allowed, gently noted, or flagged; which rules are off; and the style
//! checks. Built-in profiles can be duplicated and edited; custom profiles
//! are TOML files (shareable, and read by the command line too).

use crate::Category;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A feature of informal or register-specific writing. These are correct in
/// their own terms; a profile decides whether to point them out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Slang,
    Texting,
    SpokenSpelling,
    AaeGrammar,
    NonstandardVerbs,
    RegionalDialect,
    Contractions,
    FirstPerson,
    EmojiExclaim,
    HedgingFiller,
    Profanity,
}

impl Feature {
    pub const ALL: [Feature; 11] = [
        Feature::Slang,
        Feature::Texting,
        Feature::SpokenSpelling,
        Feature::AaeGrammar,
        Feature::NonstandardVerbs,
        Feature::RegionalDialect,
        Feature::Contractions,
        Feature::FirstPerson,
        Feature::EmojiExclaim,
        Feature::HedgingFiller,
        Feature::Profanity,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Feature::Slang => "Slang",
            Feature::Texting => "Texting style",
            Feature::SpokenSpelling => "Spoken spellings",
            Feature::AaeGrammar => "African American English grammar",
            Feature::NonstandardVerbs => "Nonstandard verb forms",
            Feature::RegionalDialect => "Regional dialect",
            Feature::Contractions => "Contractions",
            Feature::FirstPerson => "First person",
            Feature::EmojiExclaim => "Emoji and exclamations",
            Feature::HedgingFiller => "Hedges and fillers",
            Feature::Profanity => "Swearing",
        }
    }

    /// What it covers, with examples (for the profile editor).
    pub fn describe(self) -> &'static str {
        match self {
            Feature::Slang => "Casual words like lit, sus, salty, bet, no cap",
            Feature::Texting => "Shorthand like u, rn, idk, tbh, lol, and all-lowercase writing",
            Feature::SpokenSpelling => "Spellings that follow speech: gonna, wanna, kinda, finna, dat",
            Feature::AaeGrammar => {
                "Habitual be (he be working), dropped is/are (she working), completive done (I done told you)"
            }
            Feature::NonstandardVerbs => {
                "Forms common in speech and many dialects: I seen it, he don't, we was, ain't, double negatives"
            }
            Feature::RegionalDialect => "Regional words and grammar: y'all, might could, wee, innit, I'm after doing",
            Feature::Contractions => "don't, it's, we'll — often avoided in formal writing",
            Feature::FirstPerson => "I, me, my, we — avoided in some formal writing",
            Feature::EmojiExclaim => "Emoji, smileys, kisses (xx) and !!",
            Feature::HedgingFiller => "basically, kind of, sort of, I guess, literally",
            Feature::Profanity => "Swear words",
        }
    }
}

/// How a profile treats a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Not pointed out.
    Allow,
    /// A gentle, neutral note ("Informal: … In standard English: …").
    Note,
    /// Pointed out as not suited to this kind of writing.
    Flag,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Allow, Level::Note, Level::Flag];

    pub fn name(self) -> &'static str {
        match self {
            Level::Allow => "Allow",
            Level::Note => "Note",
            Level::Flag => "Flag",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Built-in profiles can't be edited in place (duplicate them first).
    #[serde(skip)]
    pub built_in: bool,
    pub features: BTreeMap<Feature, Level>,
    /// Rules turned off in this profile (by name).
    pub disabled_rules: BTreeSet<String>,
    /// Categories to show.
    pub categories: BTreeSet<Category>,
    /// A sentence with more words than this is "long".
    pub long_sentence: usize,
    /// …and more than this "very long".
    pub very_long_sentence: usize,
    pub passive_voice: bool,
    pub adverbs: bool,
    pub wordy_phrases: bool,
    /// Don't point out informal features inside quotation marks (dialogue).
    pub ignore_in_quotes: bool,
    /// Clarity notes: noun stacks, long lead-ins, sentences doing too much.
    pub clarity: bool,
    /// Note a request that comes late in an email.
    pub main_point_first: bool,
    /// Note acronyms used before they're spelled out.
    pub define_acronyms: bool,
    /// Note words written two ways in one document ("organise"/"organize",
    /// "email"/"e-mail", "Internet"/"internet", "3"/"three").
    pub consistency: bool,
    /// Show sentences that may not make sense (from the mistake spotter:
    /// edit leftovers, a lost verb, jumbled words). Off by default: there's
    /// no suggested fix.
    pub sentence_sense: bool,
    /// Note word choice: clichés, "very" + a plain word, several sentences
    /// in a row starting the same way.
    pub word_choice: bool,
    /// Also suggest commas that style guides prefer but that don't change the
    /// meaning (after "However", after a greeting, the Oxford comma). Off:
    /// only commas that are needed.
    pub optional_commas: bool,
}

impl Default for Profile {
    fn default() -> Self {
        standard()
    }
}

impl Profile {
    pub fn level(&self, f: Feature) -> Level {
        self.features.get(&f).copied().unwrap_or(Level::Allow)
    }

    /// A copy that can be edited, with a new id and name.
    pub fn duplicate(&self, taken: &[&str]) -> Profile {
        let mut p = self.clone();
        p.built_in = false;
        let base = format!("{} (copy)", self.name);
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&slug(&name).as_str()) {
            name = format!("{base} {n}");
            n += 1;
        }
        p.id = slug(&name);
        p.name = name;
        p
    }
}

/// "My Profile (copy)" → "my-profile-copy".
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    s.trim_end_matches('-').to_string()
}

fn levels(l: [Level; 11]) -> BTreeMap<Feature, Level> {
    Feature::ALL.into_iter().zip(l).collect()
}

use Level::{Allow as A, Flag as F, Note as N};

fn base(id: &str, name: &str, description: &str, features: [Level; 11]) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        built_in: true,
        features: levels(features),
        disabled_rules: BTreeSet::new(),
        categories: Category::ALL.into_iter().collect(),
        long_sentence: 25,
        very_long_sentence: 40,
        passive_voice: true,
        adverbs: false,
        wordy_phrases: true,
        ignore_in_quotes: false,
        clarity: true,
        main_point_first: false,
        define_acronyms: false,
        consistency: true,
        word_choice: true,
        sentence_sense: false,
        optional_commas: true,
    }
}

//              Slang Text Spoken AAE NonStd Region Contr 1st Emoji Hedge Swear
fn standard() -> Profile {
    base(
        "standard",
        "Standard",
        "Everyday standard English. Mistakes are flagged; informal features get a gentle note.",
        [N, F, N, N, F, N, A, A, A, A, N],
    )
}

/// The profiles that come with Xenify Write.
pub fn built_in() -> Vec<Profile> {
    let mut v = vec![standard()];
    let mut p = base(
        "academic",
        "Academic paper",
        "Research papers and theses: formal, impersonal, precise.",
        [F, F, F, F, F, F, N, N, F, N, F],
    );
    (p.long_sentence, p.very_long_sentence, p.passive_voice) = (35, 50, false);
    v.push(p);
    v.push(base(
        "school-essay",
        "School essay",
        "Essays for school or college: formal, but your own voice is fine.",
        [F, F, F, N, F, N, N, A, F, N, F],
    ));
    v.push(base(
        "workplace-email",
        "Workplace email",
        "Email and messages to colleagues and clients: your request up front, short sentences.",
        [N, F, F, N, F, N, A, A, N, N, F],
    ));
    let mut p = base(
        "formal-letter",
        "Formal or legal letter",
        "Formal letters, complaints and legal correspondence.",
        [F, F, F, F, F, F, N, A, F, N, F],
    );
    p.long_sentence = 30;
    v.push(p);
    let mut p = base(
        "technical",
        "Technical documentation",
        "Architecture, infrastructure, workflows and guides: short, direct sentences; acronyms spelled out on first use.",
        [F, F, F, F, F, F, A, N, F, N, F],
    );
    (p.long_sentence, p.very_long_sentence) = (22, 35);
    v.push(p);
    v.push(base(
        "news",
        "News article",
        "Journalism: neutral, factual, third person.",
        [F, F, F, N, F, N, N, N, F, N, F],
    ));
    v.push(base(
        "marketing",
        "Marketing copy",
        "Websites, ads and flyers: lively and direct.",
        [N, N, N, N, F, N, A, A, A, N, N],
    ));
    v.push(base(
        "cover-letter",
        "Cover letter and CV",
        "Job applications: professional and confident.",
        [F, F, F, N, F, N, A, A, F, F, F],
    ));
    v.push(base(
        "professional-social",
        "Professional social post",
        "Posts on professional networks: friendly but polished.",
        [N, N, N, N, N, N, A, A, A, N, F],
    ));
    let mut p = base(
        "social",
        "Social post",
        "Personal social media: your own voice. Dialect features are noted, not flagged.",
        [A, A, A, N, N, N, A, A, A, A, A],
    );
    (p.passive_voice, p.wordy_phrases) = (false, false);
    v.push(p);
    let mut p = base(
        "workplace-chat",
        "Work chat",
        "Chat with colleagues: short and friendly. Real mistakes are flagged; casual punctuation isn't.",
        [N, N, N, N, F, N, A, A, A, N, F],
    );
    (p.passive_voice, p.wordy_phrases, p.optional_commas) = (false, false, false);
    p.categories.remove(&Category::Readability);
    v.push(p);
    let mut p = base(
        "casual",
        "Casual chat",
        "Messages to friends: anything goes except real slips.",
        [A, A, A, A, A, A, A, A, A, A, A],
    );
    (p.passive_voice, p.wordy_phrases) = (false, false);
    p.categories.remove(&Category::Readability);
    v.push(p);
    let mut p = base(
        "creative",
        "Creative writing",
        "Fiction and poetry: informal features are left alone inside dialogue.",
        [N, N, N, N, N, N, A, A, A, A, A],
    );
    (p.ignore_in_quotes, p.passive_voice) = (true, false);
    v.push(p);
    for p in &mut v {
        match p.id.as_str() {
            "workplace-email" => p.main_point_first = true,
            "technical" | "academic" | "formal-letter" | "cover-letter" => p.define_acronyms = true,
            _ => {}
        }
        match p.id.as_str() {
            "social" | "casual" | "creative" | "marketing" | "workplace-chat" => p.clarity = false,
            _ => {}
        }
        // Chat and personal posts aren't edited for consistency; fiction and
        // poetry repeat words and openers on purpose.
        match p.id.as_str() {
            "social" | "casual" | "workplace-chat" => (p.consistency, p.word_choice) = (false, false),
            "creative" => p.word_choice = false,
            _ => {}
        }
    }
    v
}

/// Built-in and custom profiles. Custom ones are `<id>.toml` files in a folder.
#[derive(Clone, Debug, Default)]
pub struct Profiles {
    pub dir: Option<PathBuf>,
    pub custom: Vec<Profile>,
}

impl Profiles {
    /// Loads custom profiles from `dir` (missing folder: none).
    pub fn load(dir: Option<PathBuf>) -> Self {
        let mut custom = Vec::new();
        if let Some(d) = &dir
            && let Ok(entries) = std::fs::read_dir(d)
        {
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_some_and(|x| x == "toml")
                    && let Ok(p) = read_profile(&path)
                {
                    custom.push(p);
                }
            }
        }
        custom.sort_by_key(|a| a.name.to_lowercase());
        Self { dir, custom }
    }

    /// Every profile: built-in first, then custom.
    pub fn all(&self) -> Vec<Profile> {
        built_in().into_iter().chain(self.custom.iter().cloned()).collect()
    }

    /// The profile with `id` (custom ones shadow nothing: ids are unique), or Standard.
    pub fn get(&self, id: &str) -> Profile {
        self.custom
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .or_else(|| built_in().into_iter().find(|p| p.id == id))
            .unwrap_or_else(standard)
    }

    pub fn ids(&self) -> Vec<String> {
        self.all().into_iter().map(|p| p.id).collect()
    }

    /// Saves (adds or replaces) a custom profile.
    pub fn save(&mut self, p: &Profile) -> Result<(), String> {
        if p.built_in || built_in().iter().any(|b| b.id == p.id) {
            return Err("built-in profiles can't be changed; duplicate it first".into());
        }
        if let Some(d) = &self.dir {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            write_profile(&d.join(format!("{}.toml", p.id)), p)?;
        }
        match self.custom.iter_mut().find(|c| c.id == p.id) {
            Some(c) => *c = p.clone(),
            None => self.custom.push(p.clone()),
        }
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        if let Some(d) = &self.dir {
            let f = d.join(format!("{id}.toml"));
            if f.exists() {
                std::fs::remove_file(f).map_err(|e| e.to_string())?;
            }
        }
        self.custom.retain(|p| p.id != id);
        Ok(())
    }

    /// Imports a profile file (a new id if it clashes).
    pub fn import(&mut self, path: &Path) -> Result<Profile, String> {
        let mut p = read_profile(path)?;
        let ids = self.ids();
        if ids.contains(&p.id) || p.id.is_empty() {
            let taken: Vec<&str> = ids.iter().map(String::as_str).collect();
            p = p.duplicate(&taken);
            p.name = p.name.replace(" (copy)", " (imported)");
        }
        self.save(&p)?;
        Ok(p)
    }
}

pub fn read_profile(path: &Path) -> Result<Profile, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut p: Profile = toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))?;
    if p.id.is_empty() {
        p.id = path.file_stem().map(|s| slug(&s.to_string_lossy())).unwrap_or_default();
    }
    p.built_in = false;
    Ok(p)
}

pub fn write_profile(path: &Path, p: &Profile) -> Result<(), String> {
    let s = toml::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(path, format!("# Xenify Write proofing profile\n{s}"))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_ins_are_unique_and_complete() {
        let all = built_in();
        let mut ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for p in &all {
            assert_eq!(p.features.len(), Feature::ALL.len(), "{}", p.id);
        }
    }

    #[test]
    fn save_load_duplicate_import() {
        let dir = std::env::temp_dir().join(format!("xenify-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ps = Profiles::load(Some(dir.clone()));
        assert!(ps.custom.is_empty());
        let ids = ps.ids();
        let taken: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut mine = ps.get("social").duplicate(&taken);
        assert_eq!(mine.name, "Social post (copy)");
        mine.features.insert(Feature::AaeGrammar, Level::Flag);
        ps.save(&mine).unwrap();
        assert!(ps.save(&ps.get("academic")).is_err(), "built-ins are read-only");
        let again = Profiles::load(Some(dir.clone()));
        assert_eq!(again.get(&mine.id), mine);
        // Importing the same file again gets a fresh id.
        let mut ps = again;
        let imported = ps.import(&dir.join(format!("{}.toml", mine.id))).unwrap();
        assert_ne!(imported.id, mine.id);
        ps.delete(&mine.id).unwrap();
        assert!(!dir.join(format!("{}.toml", mine.id)).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
