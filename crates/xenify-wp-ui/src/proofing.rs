//! Spelling, grammar and readability checks in the app: a background
//! worker, squiggles, the right-click menu, the Proofing panel, writing
//! profiles (and their editor) and "Why?" explanations.

use crate::app::WriteApp;
use egui::{Color32, RichText};
use xenify_wp_check::{
    Category, Checker, Feature, Ignores, Issue, Level, Profile, Profiles, RULE_GROUPS, Settings, SharedSpotter, Spots,
    Spotter, SpotterPrefs, SpotterRun, is_useful_description, rule_group, rule_title,
};
use xenify_wp_doc::{NodeId, ops};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub fn color(c: Category) -> Color32 {
    match c {
        Category::Spelling => Color32::from_rgb(0xd9, 0x30, 0x25),
        Category::Grammar => Color32::from_rgb(0x1a, 0x73, 0xe8),
        Category::Style => Color32::from_rgb(0x8e, 0x44, 0xad),
        Category::Readability => Color32::from_rgb(0xe3, 0x8b, 0x00),
        Category::Register => Color32::from_rgb(0x00, 0x89, 0x7b),
    }
}

struct Request {
    /// Which document (its tab) and which version of it.
    stamp: (u64, u64),
    paras: Vec<(NodeId, String)>,
    ignores: Ignores,
    settings: Settings,
    words: Vec<String>,
    spotter: SpotterPrefs,
}

/// A document's checking results while another document is in front.
#[derive(Default)]
pub struct DocChecks {
    issues: Vec<Issue>,
    sent: Option<(u64, u64)>,
    checked: Option<(u64, u64)>,
    changed_at: Option<(u64, Instant)>,
    doc: u64,
}

impl DocChecks {
    /// For a newly opened document `doc` (checked when it comes to the front).
    pub fn new(doc: u64) -> Self {
        Self { doc, ..Default::default() }
    }

    /// What was found the last time it was checked.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// True when the results are for revision `revision` of the document.
    pub fn is_checked(&self, revision: u64) -> bool {
        self.checked == Some((self.doc, revision))
    }
}

/// The checker, on its own thread.
pub struct Proofing {
    tx: Sender<Request>,
    rx: Receiver<((u64, u64), Vec<Issue>)>,
    pub issues: Vec<Issue>,
    pub settings: Settings,
    /// The user's dictionary (shared by all documents).
    pub words: Vec<String>,
    sent: Option<(u64, u64)>,
    /// The document and revision the shown issues are for.
    checked: Option<(u64, u64)>,
    changed_at: Option<(u64, Instant)>,
    /// The document being checked (its tab's key).
    pub doc: u64,
    /// Results that came for a document not in front (another window's): kept for it.
    parked: Vec<((u64, u64), Vec<Issue>)>,
    /// Rules Harper offers (filled by the worker on first run).
    pub rules: Vec<(String, String)>,
    rules_rx: Receiver<Vec<(String, String)>>,
    pub panel: PanelState,
    /// Built-in and custom writing profiles.
    pub profiles: Profiles,
    pub editor: ProfileEditor,
    /// The issue whose "Why?" explanation is open.
    pub why: Option<Issue>,
    /// The optional mistake spotter, and what the worker last said about it.
    pub spotter: SpotterPrefs,
    pub spotter_status: String,
    spotter_rx: Receiver<String>,
    /// The language follows the computer's (not saved here; see Prefs).
    pub dialect_auto: bool,
    /// Words known for now but not saved to the dictionary (contact and
    /// chat member names in Xenify Connect).
    pub extra_words: Vec<String>,
    /// Paragraph styles left unchecked besides code (quoted replies and
    /// signatures in Xenify Connect).
    pub skip_styles: Vec<String>,
}

#[derive(Default)]
pub struct PanelState {
    /// The issue the Writing tab is showing (Next / Previous).
    pub current: Option<Issue>,
    pub show_rules: bool,
    pub filter: Option<Category>,
    /// Text to find rules by (name or description).
    pub rule_search: String,
}

/// The "Writing profiles" window.
#[derive(Default)]
pub struct ProfileEditor {
    pub open: bool,
    /// The profile shown (by id).
    pub selected: String,
    /// The profile being edited (unsaved changes).
    pub draft: Option<Profile>,
    pub path: String,
    pub status: String,
}

/// `proofing-profiles/` in the settings folder: custom writing profiles.
pub fn profiles_dir() -> PathBuf {
    dictionary_path().with_file_name("proofing-profiles")
}

/// `dictionary.txt` in the settings folder (`~/.config/xenify` on Linux):
/// one word per line.
pub fn dictionary_path() -> PathBuf {
    xenify_wp_check::config_dir().join("dictionary.txt")
}

/// The user's dictionary words.
pub fn user_words() -> Vec<String> {
    load_words()
}

fn load_words() -> Vec<String> {
    std::fs::read_to_string(dictionary_path())
        .map(|s| {
            s.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect()
        })
        .unwrap_or_default()
}

impl Proofing {
    pub fn new(settings: Settings, spotter: SpotterPrefs) -> Self {
        let (tx, req_rx) = channel::<Request>();
        let (res_tx, rx) = channel();
        let (rules_tx, rules_rx) = channel();
        let (status_tx, spotter_rx) = channel::<String>();
        std::thread::Builder::new()
            .name("xenify-write-proofing".into())
            .spawn(move || {
                let mut checker: Option<(Settings, Vec<String>, Checker)> = None;
                // The loaded model and the folder it came from.
                let mut model: Option<(std::path::PathBuf, Option<Box<dyn Spots + Send>>)> = None;
                let mut run = SpotterRun::default();
                while let Ok(mut req) = req_rx.recv() {
                    // Only the newest request matters.
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    let fresh = !matches!(&checker, Some((s, w, _)) if *s == req.settings && *w == req.words);
                    if fresh {
                        let c = Checker::new(req.settings.clone(), &req.words);
                        let _ = rules_tx.send(c.rules());
                        checker = Some((req.settings.clone(), req.words.clone(), c));
                    }
                    let c = &mut checker.as_mut().expect("just made").2;
                    let mut issues: Vec<Issue> =
                        req.paras.iter().flat_map(|(id, t)| c.check_paragraph(*id, t, &req.ignores)).collect();
                    issues.extend(c.check_whole(&req.paras, &req.ignores));
                    if req.spotter.enabled {
                        let dir = req.spotter.model_dir();
                        if model.as_ref().is_none_or(|(p, _)| *p != dir) {
                            // The shared checker holds one copy for every window and app;
                            // without it, load the model here.
                            let loaded: Result<(Box<dyn Spots + Send>, String), String> =
                                match SharedSpotter::open(&dir) {
                                    Ok((s, n)) => Ok((Box::new(s), format!("Loaded ({n} kinds of mistake), shared"))),
                                    Err(Some(e)) => Err(e),
                                    Err(None) => Spotter::load(&dir).map(|s| {
                                        let n = s.codes().len();
                                        (Box::new(s) as Box<dyn Spots + Send>, format!("Loaded ({n} kinds of mistake)"))
                                    }),
                                };
                            let _ = status_tx.send(match &loaded {
                                Ok((_, status)) => status.clone(),
                                Err(_) if !dir.join("config.json").exists() => {
                                    format!("No model installed. Put the model files in {}", dir.display())
                                }
                                Err(e) => format!("Couldn't load the model from {}: {e}", dir.display()),
                            });
                            model = Some((dir, loaded.ok().map(|(m, _)| m)));
                        }
                        if let Some((_, Some(m))) = &model {
                            let found: Vec<Issue> = req
                                .paras
                                .iter()
                                .flat_map(|(id, t)| {
                                    run.paragraph(
                                        m.as_ref(),
                                        xenify_wp_check::SpotOptions {
                                            threshold: req.spotter.threshold,
                                            optional_commas: req.settings.profile.optional_commas,
                                            sentence_sense: req.settings.profile.sentence_sense,
                                        },
                                        *id,
                                        t,
                                        &issues,
                                        &req.ignores,
                                    )
                                })
                                .collect();
                            issues.extend(found);
                        }
                    } else {
                        // Let go of the model (and the shared checker, which exits
                        // once no app uses it).
                        model = None;
                    }
                    if res_tx.send((req.stamp, issues)).is_err() {
                        break;
                    }
                }
            })
            .expect("thread");
        Self {
            tx,
            rx,
            issues: Vec::new(),
            settings,
            words: load_words(),
            sent: None,
            checked: None,
            changed_at: None,
            doc: 0,
            parked: Vec::new(),
            rules: Vec::new(),
            rules_rx,
            panel: PanelState::default(),
            profiles: Profiles::load(Some(profiles_dir())),
            editor: ProfileEditor::default(),
            why: None,
            spotter,
            spotter_status: String::new(),
            spotter_rx,
            dialect_auto: true,
            extra_words: Vec::new(),
            skip_styles: Vec::new(),
        }
    }

    /// The id of the profile `doc` uses.
    pub fn profile_id(&self, doc: &xenify_wp_doc::Document) -> String {
        doc.meta.proofing_profile.clone().unwrap_or_else(|| self.settings.default_profile.clone())
    }

    /// Keeps the checker's profile in step with the document's choice.
    fn sync_profile(&mut self, doc: &xenify_wp_doc::Document) {
        let p = self.profiles.get(&self.profile_id(doc));
        if p != self.settings.profile {
            self.settings.profile = p;
            self.recheck();
        }
    }

    /// Asks for a fresh check (settings or dictionary changed).
    pub fn recheck(&mut self) {
        self.sent = None;
    }

    /// Sends the document for checking a moment after typing stops; picks up results.
    pub fn poll(&mut self, ctx: &egui::Context, doc: &xenify_wp_doc::Document) {
        while let Ok((stamp, issues)) = self.rx.try_recv() {
            if Some(stamp) == self.sent {
                self.issues = issues;
                self.checked = Some(stamp);
                ctx.request_repaint();
            } else {
                // Another document's: kept until it's in front again (one per document).
                self.parked.retain(|(s, _)| s.0 != stamp.0);
                self.parked.push((stamp, issues));
            }
        }
        if let Ok(r) = self.rules_rx.try_recv() {
            self.rules = r;
        }
        while let Ok(st) = self.spotter_rx.try_recv() {
            self.spotter_status = st;
        }
        self.sync_profile(doc);
        if !self.settings.enabled {
            self.issues.clear();
            self.checked = Some((self.doc, doc.revision));
            return;
        }
        let rev = doc.revision;
        if self.sent == Some((self.doc, rev)) {
            return;
        }
        let now = Instant::now();
        let at = match self.changed_at {
            Some((r, t)) if r == rev => t,
            _ => {
                self.changed_at = Some((rev, now));
                now
            }
        };
        // First check at once; later ones once typing pauses.
        let wait = if self.sent.is_none() { Duration::ZERO } else { Duration::from_millis(500) };
        if now.duration_since(at) < wait {
            ctx.request_repaint_after(wait - now.duration_since(at));
            return;
        }
        let paras = doc
            .paragraphs()
            .filter(|p| p.style != xenify_wp_doc::StyleSheet::CODE && !self.skip_styles.contains(&p.style))
            .map(|p| (p.id, p.text.clone()))
            .collect();
        let _ = self.tx.send(Request {
            stamp: (self.doc, rev),
            paras,
            ignores: Ignores::of(doc),
            settings: self.settings.clone(),
            words: self.words.iter().chain(&self.extra_words).cloned().collect(),
            spotter: self.spotter.clone(),
        });
        self.sent = Some((self.doc, rev));
    }

    /// Swaps in another document's checking state (switching tabs).
    pub fn swap_doc(&mut self, doc: u64, checks: &mut DocChecks) {
        std::mem::swap(&mut self.issues, &mut checks.issues);
        std::mem::swap(&mut self.sent, &mut checks.sent);
        std::mem::swap(&mut self.checked, &mut checks.checked);
        std::mem::swap(&mut self.changed_at, &mut checks.changed_at);
        std::mem::swap(&mut self.doc, &mut checks.doc);
        debug_assert_eq!(self.doc, doc);
        // Results that came while it was away.
        if let Some(i) = self.parked.iter().position(|(s, _)| Some(*s) == self.sent) {
            self.issues = self.parked.remove(i).1;
            self.checked = self.sent;
        }
    }

    /// True when the issues shown are for this revision of the front document.
    pub fn is_checked(&self, doc: &xenify_wp_doc::Document) -> bool {
        self.checked == Some((self.doc, doc.revision))
    }

    /// Moves results that came for a document not in front into its saved
    /// state, so it can show them without coming to the front.
    pub fn deliver_parked(&mut self, checks: &mut DocChecks) {
        if let Some(i) = self.parked.iter().position(|(s, _)| Some(*s) == checks.sent) {
            checks.issues = self.parked.remove(i).1;
            checks.checked = checks.sent;
        }
    }

    pub fn add_word(&mut self, word: &str) {
        let w = word.trim().to_string();
        if w.is_empty() || self.words.contains(&w) {
            return;
        }
        self.words.push(w.clone());
        let path = dictionary_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, self.words.join("\n") + "\n");
        self.issues.retain(|i| i.category != Category::Spelling);
        self.recheck();
    }
}

/// The issue at byte `offset` of paragraph `para`, if any.
pub fn issue_at(issues: &[Issue], para: NodeId, offset: usize) -> Option<&Issue> {
    issues.iter().find(|i| i.para == para && i.range.start <= offset && offset <= i.range.end)
}

/// Applies `change` to the document's profile. A built-in profile is first
/// copied to a custom one ("‹name› (mine)"), which the document then uses.
pub fn edit_profile(app: &mut WriteApp, change: impl FnOnce(&mut Profile)) {
    edit_profile_for(&mut app.proofing, &mut app.editor.doc, change);
}

/// [`edit_profile`] for any document and checker (not just Write's window).
pub fn edit_profile_for(proofing: &mut Proofing, doc: &mut xenify_wp_doc::Document, change: impl FnOnce(&mut Profile)) {
    let mut p = proofing.settings.profile.clone();
    if p.built_in {
        let ids = proofing.profiles.ids();
        let taken: Vec<&str> = ids.iter().map(String::as_str).collect();
        p = p.duplicate(&taken);
        p.name = p.name.replace(" (copy)", " (mine)");
        p.id = xenify_wp_check::slug(&p.name);
        if ids.contains(&p.id) {
            p.id = format!("{}-{}", p.id, ids.len());
        }
        doc.meta.proofing_profile = Some(p.id.clone());
        doc.revision += 1;
    }
    change(&mut p);
    if let Err(e) = proofing.profiles.save(&p) {
        proofing.editor.status = e;
    }
    proofing.settings.profile = Profile::default(); // forces a resync
    proofing.recheck();
}

/// The right-click menu for `issue`; true once an action was taken.
pub fn issue_menu(app: &mut WriteApp, ui: &mut egui::Ui, issue: &Issue) -> bool {
    issue_menu_for(&mut app.proofing, &mut app.editor, ui, issue)
}

/// [`issue_menu`] for any editor and checker (Xenify Connect's text boxes too).
pub fn issue_menu_for(
    proofing: &mut Proofing,
    editor: &mut xenify_wp_doc::Editor,
    ui: &mut egui::Ui,
    issue: &Issue,
) -> bool {
    let mut done = false;
    let text = editor.doc.para(issue.para).map(|p| p.text.clone()).unwrap_or_default();
    let word = text.get(issue.range.clone()).unwrap_or("").to_string();
    let label = match issue.feature {
        Some(f) => format!("{} · {}", issue.category.name(), f.name()),
        None => issue.category.name().to_string(),
    };
    ui.label(RichText::new(label).small().color(color(issue.category)));
    ui.label(&issue.message);
    ui.separator();
    for s in issue.suggestions.iter().take(6) {
        let label = if s.is_empty() { "Remove it".to_string() } else { s.clone() };
        if ui.button(RichText::new(label).strong()).clicked() {
            let _ = ops::replace_text(editor, issue.para, issue.range.clone(), s, "Correction");
            done = true;
            ui.close();
        }
    }
    if !issue.suggestions.is_empty() {
        ui.separator();
    }
    if ui.button("Ignore").clicked() {
        editor.doc.meta.ignore_issues.push(issue.ignore_key(&text));
        editor.doc.revision += 1;
        done = true;
        ui.close();
    }
    if issue.category == Category::Spelling && ui.button(format!("Add \"{word}\" to dictionary")).clicked() {
        proofing.add_word(&word);
        done = true;
        ui.close();
    }
    // The rest (explanations, turning checks off) one level down.
    crate::icons::submenu(ui, crate::icons::Icon::Settings, "More options", |ui| {
        if ui.button("Why? Explain this").clicked() {
            proofing.why = Some(issue.clone());
            done = true;
            ui.close();
        }
        let profile_name = proofing.settings.profile.name.clone();
        if let Some(f) = issue.feature {
            if ui.button(format!("Allow {} in \"{profile_name}\"", f.name().to_lowercase())).clicked() {
                edit_profile_for(proofing, &mut editor.doc, |p| {
                    p.features.insert(f, Level::Allow);
                });
                done = true;
                ui.close();
            }
        } else if issue.category == Category::Spelling {
            if ui.button(format!("Ignore \"{word}\" in this document")).clicked() {
                editor.doc.meta.ignore_words.push(word.clone());
                editor.doc.revision += 1;
                done = true;
                ui.close();
            }
        } else if matches!(
            issue.rule.as_str(),
            "PassiveVoice"
                | "WordyPhrase"
                | "Adverb"
                | "LongSentence"
                | "VeryLongSentence"
                | "ConsistentSpelling"
                | "ConsistentHyphens"
                | "ConsistentCapitals"
                | "ConsistentNumbers"
                | "Cliche"
                | "WeakIntensifier"
                | "RepeatedOpener"
                | "Spotter:garbled"
        ) {
            if ui.button(format!("Turn this check off in \"{profile_name}\"")).clicked() {
                let rule = issue.rule.clone();
                edit_profile_for(proofing, &mut editor.doc, |p| match rule.as_str() {
                    "PassiveVoice" => p.passive_voice = false,
                    "WordyPhrase" => p.wordy_phrases = false,
                    "Adverb" => p.adverbs = false,
                    "NounStack" | "LongLeadIn" | "TooManyIdeas" | "Filler" | "ListColon" | "VagueReference" => {
                        p.clarity = false
                    }
                    "MainPointLate" => p.main_point_first = false,
                    "GreetingComma" | "DiscourseMarkers" | "HelloGreeting" | "Spotter:comma_intro" => {
                        p.optional_commas = false
                    }
                    "UndefinedAcronym" => p.define_acronyms = false,
                    "Spotter:garbled" => p.sentence_sense = false,
                    r if r.starts_with("Consistent") => p.consistency = false,
                    "Cliche" | "WeakIntensifier" | "RepeatedOpener" => p.word_choice = false,
                    _ => {
                        p.categories.remove(&Category::Readability);
                    }
                });
                done = true;
                ui.close();
            }
        } else {
            if ui.button(format!("Turn this rule off in \"{profile_name}\"")).clicked() {
                let rule = issue.rule.clone();
                edit_profile_for(proofing, &mut editor.doc, |p| {
                    p.disabled_rules.insert(rule);
                });
                done = true;
                ui.close();
            }
            if ui.button("Turn this rule off everywhere").clicked() {
                proofing.settings.disabled_rules.insert(issue.rule.clone());
                proofing.recheck();
                done = true;
                ui.close();
            }
        }
    });
    done
}

/// The "Why?" window for the issue chosen from its menu.
pub fn why_window(app: &mut WriteApp, ctx: &egui::Context) {
    why_window_for(&mut app.proofing, &app.editor.doc, ctx);
}

/// [`why_window`] for any document and checker.
pub fn why_window_for(proofing: &mut Proofing, doc: &xenify_wp_doc::Document, ctx: &egui::Context) {
    let Some(issue) = proofing.why.clone() else { return };
    let text = doc.para(issue.para).map(|p| p.text.clone()).unwrap_or_default();
    let found = text.get(issue.range.clone()).unwrap_or("").to_string();
    let rule = issue.feature.map(|f| format!("Register:{f:?}")).unwrap_or_else(|| issue.rule.clone());
    let expl = xenify_wp_check::explain(&rule).unwrap_or_else(|| xenify_wp_check::Explanation {
        title: issue.category.name().to_string(),
        text: proofing
            .rules
            .iter()
            .find(|(r, _)| *r == issue.rule)
            .map(|(_, d)| d.replace('`', ""))
            .unwrap_or_default(),
        examples: Vec::new(),
    });
    let mut open = true;
    let mut close = false;
    crate::dialog::window(ctx, format!("Why? — {}", expl.title), &mut open).default_width(380.0).show(ctx, |ui| {
        ui.label(RichText::new(format!("\u{201c}{found}\u{201d}")).strong().color(color(issue.category)));
        ui.label(&issue.message);
        ui.separator();
        ui.label(&expl.text);
        if !expl.examples.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new("Examples").strong());
            egui::Grid::new("why-examples").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for (wrong, right) in &expl.examples {
                    ui.label(RichText::new(format!("✗ {wrong}")).color(Color32::from_rgb(0xc0, 0x39, 0x2b)));
                    ui.label(RichText::new(format!("✓ {right}")).color(Color32::from_rgb(0x1e, 0x84, 0x49)));
                    ui.end_row();
                }
            });
        }
        if let Some(f) = issue.feature {
            ui.add_space(6.0);
            ui.weak(format!(
                "\"{}\" sets {} to {}.",
                proofing.settings.profile.name,
                f.name().to_lowercase(),
                proofing.settings.profile.level(f).name()
            ));
        }
        ui.weak(format!("Rule: {}", issue.rule));
        close = crate::dialog::buttons(ui, &[("Close", crate::dialog::Kind::Cancel)]).is_some();
    });
    if !open || close {
        proofing.why = None;
    }
}

/// The Proofing side panel.
/// The Spelling and grammar tab: this document's writing profile and what
/// was found. How checking works is in Settings › Spelling and grammar.
/// "This document is …": the writing profile (what to point out for this
/// kind of writing), saved in the document.
pub fn document_type(app: &mut WriteApp, ui: &mut egui::Ui) {
    let all = app.proofing.profiles.all();
    let current = app.proofing.profile_id(&app.editor.doc);
    let name_of = |id: &str| all.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
    let mut chosen = current.clone();
    ui.horizontal(|ui| {
        ui.label("This document is");
        let r = egui::ComboBox::from_id_salt("doc-profile").selected_text(name_of(&current)).show_ui(ui, |ui| {
            for p in &all {
                ui.selectable_value(&mut chosen, p.id.clone(), &p.name).on_hover_text(&p.description);
            }
        });
        r.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Writing profile"));
    });
    if chosen != current {
        app.editor.doc.meta.proofing_profile = Some(chosen);
        app.editor.doc.revision += 1;
    }
    ui.weak(&app.proofing.settings.profile.description);
}

/// Settings › Spelling and grammar.
pub fn settings(app: &mut WriteApp, ui: &mut egui::Ui) {
    use crate::settings::group;
    let mut s = app.proofing.settings.clone();
    let all = app.proofing.profiles.all();
    let name_of = |id: &str| all.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
    group(ui, "Checking", |ui| {
        ui.checkbox(&mut s.enabled, "Check spelling and grammar as I type");
        let computer = xenify_wp_check::Dialect::from_locale();
        let auto_text = format!("Same as this computer ({})", computer.name());
        let mut auto = app.proofing.dialect_auto;
        ui.horizontal(|ui| {
            let r = egui::ComboBox::from_label("Language")
                .selected_text(if auto { auto_text.clone() } else { s.dialect.name().to_string() })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(auto, &auto_text).clicked() {
                        auto = true;
                        s.dialect = computer;
                    }
                    for d in xenify_wp_check::Dialect::ALL {
                        if ui.selectable_label(!auto && s.dialect == d, d.name()).clicked() {
                            auto = false;
                            s.dialect = d;
                        }
                    }
                });
            let _ = r;
        });
        app.proofing.dialect_auto = auto;
        ui.checkbox(&mut s.ignore_names, "Don't flag names (unknown capitalised words mid-sentence)");
    });
    group(ui, "Profiles", |ui| {
        ui.weak("A profile says what to point out for a kind of writing (a report, an email, a story…).");
        ui.horizontal(|ui| {
            ui.label("New documents use");
            egui::ComboBox::from_id_salt("default-profile").selected_text(name_of(&s.default_profile)).show_ui(
                ui,
                |ui| {
                    for p in &all {
                        ui.selectable_value(&mut s.default_profile, p.id.clone(), &p.name);
                    }
                },
            );
        });
        if ui.button("Edit profiles…").clicked() {
            app.proofing.editor.open = true;
            app.proofing.editor.selected = app.proofing.settings.profile.id.clone();
            app.proofing.editor.draft = None;
        }
    });
    if s != app.proofing.settings {
        app.proofing.settings = s;
        app.proofing.recheck();
    }
    ui.add_space(12.0);
    egui::CollapsingHeader::new("Advanced").id_salt("proof-advanced").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.collapsing("Mistake finder (runs on this computer)", |ui| {
            let mut sp = app.proofing.spotter.clone();
            ui.checkbox(&mut sp.enabled, "Also look for mistakes the rules miss").on_hover_text(
                "A small model on this computer that finds missing words, word order and comma mistakes. Nothing is sent anywhere.",
            );
            ui.horizontal(|ui| {
                ui.label("Only show what it's at least");
                ui.add(egui::Slider::new(&mut sp.threshold, 0.5..=0.99).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                ui.label("sure of");
            });
            ui.horizontal(|ui| {
                ui.label("Model files in");
                ui.add(egui::TextEdit::singleline(&mut sp.path).hint_text(xenify_wp_check::default_spotter_dir().display().to_string()));
            });
            if sp.enabled && !app.proofing.spotter_status.is_empty() {
                ui.weak(&app.proofing.spotter_status);
            }
            if sp != app.proofing.spotter {
                app.proofing.spotter = sp;
                app.proofing.recheck();
            }
        });
        ui.collapsing("Rules (all profiles)", |ui| rules_section(app, ui));
        ui.collapsing(format!("Your dictionary ({} words)", app.proofing.words.len()), |ui| {
            ui.weak(dictionary_path().display().to_string());
            let mut remove = None;
            for (i, w) in app.proofing.words.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(w);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                app.proofing.words.remove(i);
                let _ = std::fs::write(dictionary_path(), app.proofing.words.join("\n") + "\n");
                app.proofing.recheck();
            }
        });
    });
}

/// Every rule, in groups, with a search box and the ones turned off first.
fn rules_section(app: &mut WriteApp, ui: &mut egui::Ui) {
    if app.proofing.rules.is_empty() {
        ui.weak("Loading…");
        return;
    }
    ui.weak("Writing profiles choose what to check; turn single rules off here, or right-click a mistake.");
    ui.add(
        egui::TextEdit::singleline(&mut app.proofing.panel.rule_search)
            .hint_text(format!("Find among {} rules…", app.proofing.rules.len())),
    );
    let needle = app.proofing.panel.rule_search.trim().to_lowercase();
    let rules = app.proofing.rules.clone();
    let shown: Vec<&(String, String)> = rules
        .iter()
        .filter(|(name, desc)| {
            needle.is_empty()
                || name.to_lowercase().contains(&needle)
                || rule_title(name).to_lowercase().contains(&needle)
                || desc.to_lowercase().contains(&needle)
        })
        .collect();
    let mut changed = false;
    let mut disabled = app.proofing.settings.rules_off();
    // The Settings page scrolls: no scroll area of its own.
    {
        let off: Vec<_> = shown.iter().filter(|(n, _)| disabled.contains(n)).collect();
        if !off.is_empty() {
            ui.label(RichText::new(format!("Turned off ({})", off.len())).strong());
            for (name, desc) in off {
                changed |= rule_row(ui, name, desc, &mut disabled);
            }
            ui.separator();
        }
        for group in RULE_GROUPS {
            let members: Vec<_> = shown.iter().filter(|(_, d)| rule_group(d) == group).collect();
            if members.is_empty() {
                continue;
            }
            let on = members.iter().filter(|(n, _)| !disabled.contains(n)).count();
            let header = format!("{group} ({on} of {} on)", members.len());
            egui::CollapsingHeader::new(header)
                .id_salt(group)
                .open(if needle.is_empty() { None } else { Some(true) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("All on").clicked() {
                            members.iter().for_each(|(n, _)| {
                                disabled.remove(n);
                            });
                            changed = true;
                        }
                        if ui.small_button("All off").clicked() {
                            disabled.extend(members.iter().map(|(n, _)| n.clone()));
                            changed = true;
                        }
                    });
                    for (name, desc) in &members {
                        changed |= rule_row(ui, name, desc, &mut disabled);
                    }
                });
        }
        if shown.is_empty() {
            ui.weak("No rule matches.");
        }
    }
    if changed {
        app.proofing.settings.set_rules_off(disabled);
        app.proofing.recheck();
    }
}

/// One rule's checkbox (its description on hover); true when toggled.
fn rule_row(ui: &mut egui::Ui, name: &str, desc: &str, disabled: &mut BTreeSet<String>) -> bool {
    let mut on = !disabled.contains(name);
    let mut r = ui.checkbox(&mut on, rule_title(name));
    if is_useful_description(desc) {
        r = r.on_hover_text(desc);
    }
    if r.changed() {
        if on {
            disabled.remove(name);
        } else {
            disabled.insert(name.to_string());
        }
    }
    r.changed()
}

/// The "Writing profiles" window: pick, duplicate, edit, delete, import and
/// export profiles.
pub fn profile_window(app: &mut WriteApp, ctx: &egui::Context) {
    if !app.proofing.editor.open {
        return;
    }
    let mut open = true;
    let all = app.proofing.profiles.all();
    if app.proofing.editor.draft.as_ref().is_none_or(|d| d.id != app.proofing.editor.selected) {
        app.proofing.editor.draft = Some(app.proofing.profiles.get(&app.proofing.editor.selected));
    }
    let mut action: Option<&str> = None;
    let mut close = false;
    crate::dialog::window(ctx, "Writing profiles", &mut open).default_size([720.0, 520.0]).show(ctx, |ui| {
        ui.horizontal_top(|ui| {
            // The list.
            ui.vertical(|ui| {
                ui.set_width(200.0);
                egui::ScrollArea::vertical().id_salt("profile-list").max_height(420.0).show(ui, |ui| {
                    for p in &all {
                        let label = if p.built_in { format!("🔒 {}", p.name) } else { p.name.clone() };
                        if ui
                            .selectable_label(app.proofing.editor.selected == p.id, label)
                            .on_hover_text(&p.description)
                            .clicked()
                        {
                            app.proofing.editor.selected = p.id.clone();
                        }
                    }
                });
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Duplicate").clicked() {
                        action = Some("duplicate");
                    }
                    let custom = app.proofing.editor.draft.as_ref().is_some_and(|d| !d.built_in);
                    if ui.add_enabled(custom, egui::Button::new("Delete")).clicked() {
                        action = Some("delete");
                    }
                    if ui.button("Use for this document").clicked() {
                        action = Some("use");
                    }
                });
                ui.separator();
                ui.label("File (.toml)");
                if app.proofing.editor.path.is_empty() {
                    app.proofing.editor.path = dirs_home().join("profile.toml").display().to_string();
                }
                ui.text_edit_singleline(&mut app.proofing.editor.path);
                ui.horizontal(|ui| {
                    if ui.button("Import").clicked() {
                        action = Some("import");
                    }
                    if ui.button("Export").clicked() {
                        action = Some("export");
                    }
                });
                if !app.proofing.editor.status.is_empty() {
                    ui.weak(&app.proofing.editor.status);
                }
            });
            ui.separator();
            // The selected profile.
            let Some(d) = app.proofing.editor.draft.as_mut() else { return };
            let editable = !d.built_in;
            ui.vertical(|ui| {
                if !editable {
                    ui.colored_label(
                        Color32::from_rgb(0xb0, 0x7a, 0x00),
                        "Built-in profiles can't be changed. Duplicate this one to make your own.",
                    );
                }
                ui.add_enabled_ui(editable, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut d.name);
                    });
                    ui.horizontal(|ui| {
                        ui.label("About");
                        ui.add(egui::TextEdit::singleline(&mut d.description).desired_width(380.0));
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Informal writing").strong());
                    egui::ScrollArea::vertical().id_salt("profile-features").max_height(260.0).show(ui, |ui| {
                        egui::Grid::new("features").num_columns(4).striped(true).show(ui, |ui| {
                            for f in Feature::ALL {
                                ui.label(f.name()).on_hover_text(f.describe());
                                let mut level = d.level(f);
                                for l in Level::ALL {
                                    ui.radio_value(&mut level, l, l.name());
                                }
                                d.features.insert(f, level);
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Checks").strong());
                    ui.horizontal_wrapped(|ui| {
                        for c in Category::ALL {
                            let mut on = d.categories.contains(&c);
                            if ui.checkbox(&mut on, RichText::new(c.name()).color(color(c))).changed() {
                                if on {
                                    d.categories.insert(c);
                                } else {
                                    d.categories.remove(&c);
                                }
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut d.passive_voice, "Passive voice");
                        ui.checkbox(&mut d.wordy_phrases, "Wordy phrases");
                        ui.checkbox(&mut d.adverbs, "Adverbs");
                        ui.checkbox(&mut d.ignore_in_quotes, "Leave dialogue in quotes alone");
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut d.clarity, "Clarity notes").on_hover_text(
                            "Noun stacks, long lead-ins before the main point, sentences doing too much",
                        );
                        ui.checkbox(&mut d.main_point_first, "Request up front")
                            .on_hover_text("Note when an email's request or question comes late");
                        ui.checkbox(&mut d.define_acronyms, "Spell out acronyms")
                            .on_hover_text("Note acronyms used before they're written in full");
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut d.consistency, "Consistency").on_hover_text(
                            "Note words written two ways in one document: organise/organize, email/e-mail, Internet/internet, 3/three",
                        );
                        ui.checkbox(&mut d.sentence_sense, "Sentences that may not make sense").on_hover_text(
                            "Point out sentences that don't hold together: words left over from an edit, a missing verb, jumbled words, a sentence that stops too soon. No fix is suggested",
                        );
                        ui.checkbox(&mut d.word_choice, "Word choice").on_hover_text(
                            "Note clichés, \"very\" + a plain word, and several sentences in a row starting the same way",
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Commas:");
                        ui.radio_value(&mut d.optional_commas, false, "Only where needed");
                        ui.radio_value(&mut d.optional_commas, true, "Also optional ones").on_hover_text(
                            "Also suggest commas style guides prefer but that don't change the meaning: after However, after a greeting, the Oxford comma",
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Long sentences over");
                        ui.add(egui::DragValue::new(&mut d.long_sentence).range(8..=80).suffix(" words"));
                    });
                    d.very_long_sentence = d.very_long_sentence.max(d.long_sentence + 5);
                    if !d.disabled_rules.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("Rules off: {}", d.disabled_rules.len()));
                            let mut remove = None;
                            for r in &d.disabled_rules {
                                if ui.small_button(format!("{r} ✖")).on_hover_text("Turn back on").clicked() {
                                    remove = Some(r.clone());
                                }
                            }
                            if let Some(r) = remove {
                                d.disabled_rules.remove(&r);
                            }
                        });
                    }
                });
            });
        });
        // Changes to your own profiles are saved as you make them.
        close = crate::dialog::buttons(ui, &[("Close", crate::dialog::Kind::Cancel)]).is_some();
    });
    // Save edits to custom profiles as they're made.
    if let Some(d) = app.proofing.editor.draft.clone()
        && !d.built_in
        && d != app.proofing.profiles.get(&d.id)
    {
        match app.proofing.profiles.save(&d) {
            Ok(()) => app.proofing.editor.status = format!("Saved \"{}\"", d.name),
            Err(e) => app.proofing.editor.status = e,
        }
        app.proofing.settings.profile = Profile::default();
        app.proofing.recheck();
    }
    let ed = &mut app.proofing.editor;
    match action {
        Some("duplicate") => {
            let ids = app.proofing.profiles.ids();
            let taken: Vec<&str> = ids.iter().map(String::as_str).collect();
            let copy = app.proofing.profiles.get(&ed.selected).duplicate(&taken);
            ed.status = match app.proofing.profiles.save(&copy) {
                Ok(()) => format!("Made \"{}\"", copy.name),
                Err(e) => e,
            };
            ed.selected = copy.id;
            ed.draft = None;
        }
        Some("delete") => {
            let id = ed.selected.clone();
            ed.status = match app.proofing.profiles.delete(&id) {
                Ok(()) => "Deleted".into(),
                Err(e) => e,
            };
            if app.editor.doc.meta.proofing_profile.as_deref() == Some(id.as_str()) {
                app.editor.doc.meta.proofing_profile = None;
                app.editor.doc.revision += 1;
            }
            if app.proofing.settings.default_profile == id {
                app.proofing.settings.default_profile = "standard".into();
            }
            ed.selected = "standard".into();
            ed.draft = None;
        }
        Some("use") => {
            app.editor.doc.meta.proofing_profile = Some(ed.selected.clone());
            app.editor.doc.revision += 1;
            ed.status = "This document now uses it".into();
        }
        Some("import") => {
            let path = PathBuf::from(ed.path.trim());
            ed.status = match app.proofing.profiles.import(&path) {
                Ok(p) => {
                    ed.selected = p.id.clone();
                    ed.draft = None;
                    format!("Imported \"{}\"", p.name)
                }
                Err(e) => e,
            };
        }
        Some("export") => {
            let path = PathBuf::from(ed.path.trim());
            let p = app.proofing.profiles.get(&ed.selected);
            ed.status = match xenify_wp_check::write_profile(&path, &p) {
                Ok(()) => format!("Exported to {}", path.display()),
                Err(e) => e,
            };
        }
        _ => {}
    }
    if !open || close {
        app.proofing.editor.open = false;
    }
}

fn dirs_home() -> PathBuf {
    xenify_wp_check::home_dir()
}
