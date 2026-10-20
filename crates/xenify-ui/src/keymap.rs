//! Keyboard shortcuts: every command the app can run from a key ([`Action`]),
//! the Browser/Figma-style defaults, the user's changes, and matching key
//! presses to actions. Menus, tooltips and the F1 reference all read their
//! shortcut text from here, so they can't drift apart.

use crate::app::Tool;
use egui::{Key, KeyboardShortcut, Modifiers};
use std::collections::HashMap;

/// eframe storage key for the user's changes to the default shortcuts.
pub const KEYMAP_KEY: &str = "keymap_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    // File
    New,
    Open,
    Save,
    SaveAs,
    Import,
    Export,
    Settings,
    Print,
    CloseTab,
    NextTab,
    PrevTab,
    Quit,
    ShortcutsHelp,
    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    SelectAll,
    Deselect,
    SelectNext,
    SelectPrev,
    RenameLayer,
    // Arrange / object
    Group,
    Ungroup,
    Precompose,
    ClipMask,
    ReleaseClipMask,
    ToFront,
    Forward,
    Backward,
    ToBack,
    RotateCw,
    RotateCcw,
    Rotate180,
    FlipH,
    FlipV,
    // View
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    ToggleRulers,
    ToggleGrid,
    ToggleGuides,
    PositionSize,
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,
    // Slides
    NewSlide,
    DuplicateSlide,
    DeleteSlide,
    AddPause,
    Present,
    PresentCurrent,
    // AI
    AiNew,
    AiNewSlide,
    AiModifySlide,
    AiImage,
    // Time
    PlayPause,
    FrameBack,
    FrameForward,
    GoStart,
    GoEnd,
    PrevKey,
    NextKey,
    // Tools and paint
    Tool(Tool),
    CommitTool,
    ToggleFillStroke,
    SwapFillStroke,
    ResetColors,
    BrushSmaller,
    BrushBigger,
}

/// When an action's keys work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    /// Always, even while typing in a text field (Save, Quit…).
    Global,
    /// Not while typing (text fields keep their own Ctrl+Z, Ctrl+A…).
    NotTyping,
    /// Only on the canvas: not while typing or with a dialog open.
    Canvas,
    /// Canvas, and only while a brush-sized tool (Brush, Pencil, Sculpt) is active.
    BrushTool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    File,
    Edit,
    Object,
    View,
    Slides,
    Ai,
    Time,
    Tools,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::File,
        Category::Edit,
        Category::Object,
        Category::View,
        Category::Slides,
        Category::Ai,
        Category::Time,
        Category::Tools,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Edit => "Edit",
            Category::Object => "Object",
            Category::View => "View",
            Category::Slides => "Slides",
            Category::Ai => "AI",
            Category::Time => "Time",
            Category::Tools => "Tools & Colours",
        }
    }
}

impl Action {
    /// Every action, in menu/reference order.
    pub fn all() -> Vec<Action> {
        use Action::*;
        let mut v = vec![
            New,
            Open,
            Save,
            SaveAs,
            Import,
            Export,
            Settings,
            Print,
            CloseTab,
            NextTab,
            PrevTab,
            Quit,
            ShortcutsHelp,
            Undo,
            Redo,
            Cut,
            Copy,
            Paste,
            Duplicate,
            Delete,
            SelectAll,
            Deselect,
            SelectNext,
            SelectPrev,
            RenameLayer,
            Group,
            Ungroup,
            Precompose,
            ClipMask,
            ReleaseClipMask,
            ToFront,
            Forward,
            Backward,
            ToBack,
            RotateCw,
            RotateCcw,
            Rotate180,
            FlipH,
            FlipV,
            ZoomIn,
            ZoomOut,
            ZoomFit,
            Zoom100,
            ToggleRulers,
            ToggleGrid,
            ToggleGuides,
            PositionSize,
            NudgeLeft,
            NudgeRight,
            NudgeUp,
            NudgeDown,
            NewSlide,
            DuplicateSlide,
            DeleteSlide,
            AddPause,
            Present,
            PresentCurrent,
            AiNew,
            AiNewSlide,
            AiModifySlide,
            AiImage,
            PlayPause,
            FrameBack,
            FrameForward,
            GoStart,
            GoEnd,
            PrevKey,
            NextKey,
        ];
        v.extend(crate::app::Tool::ALL.iter().map(|t| Tool(t.0)));
        v.extend([CommitTool, ToggleFillStroke, SwapFillStroke, ResetColors, BrushSmaller, BrushBigger]);
        v
    }

    /// Stable id used when saving the user's shortcuts (`"edit.duplicate"`).
    pub fn id(self) -> String {
        match self {
            Action::Tool(t) => format!("tool.{}", format!("{t:?}").to_lowercase()),
            a => format!(
                "{}.{}",
                self.category().label().to_lowercase().split(' ').next().unwrap(),
                snake(&format!("{a:?}"))
            ),
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::all().into_iter().find(|a| a.id() == id)
    }

    pub fn label(self) -> String {
        use Action::*;
        match self {
            New => "New…",
            Open => "Open…",
            Save => "Save",
            SaveAs => "Save As…",
            Import => "Import…",
            Export => "Export…",
            Settings => "Settings…",
            Print => "Print…",
            CloseTab => "Close Tab",
            NextTab => "Next Tab",
            PrevTab => "Previous Tab",
            Quit => "Quit",
            ShortcutsHelp => "Keyboard Shortcuts",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            Duplicate => "Duplicate",
            Delete => "Delete",
            SelectAll => "Select All",
            Deselect => "Deselect / Cancel Tool",
            SelectNext => "Select Next Object",
            SelectPrev => "Select Previous Object",
            RenameLayer => "Rename Layer",
            Group => "Group",
            Ungroup => "Ungroup",
            Precompose => "Convert to Symbol",
            ClipMask => "Set Mask",
            ReleaseClipMask => "Release Mask",
            ToFront => "Bring to Front",
            Forward => "Bring Forward",
            Backward => "Send Backward",
            ToBack => "Send to Back",
            RotateCw => "Rotate 90° Clockwise",
            RotateCcw => "Rotate 90° Anticlockwise",
            Rotate180 => "Rotate 180°",
            FlipH => "Flip Horizontal",
            FlipV => "Flip Vertical",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomFit => "Fit to Window",
            Zoom100 => "Actual Size (100%)",
            ToggleRulers => "Show Rulers",
            ToggleGrid => "Show Grid",
            ToggleGuides => "Show Guides",
            NudgeLeft => "Nudge Left",
            NudgeRight => "Nudge Right",
            NudgeUp => "Nudge Up",
            NudgeDown => "Nudge Down",
            PositionSize => "Position & Size…",
            NewSlide => "New Slide",
            DuplicateSlide => "Duplicate Slide",
            DeleteSlide => "Delete Slide",
            AddPause => "Add / Remove Pause at Playhead",
            Present => "Present From Start",
            PresentCurrent => "Present From This Slide",
            AiNew => "New From a Description…",
            AiNewSlide => "New Slide with AI…",
            AiModifySlide => "Modify This Slide…",
            AiImage => "New Image…",
            PlayPause => "Play / Pause (hold and drag to pan)",
            FrameBack => "Previous Frame",
            FrameForward => "Next Frame",
            GoStart => "Go to Start",
            GoEnd => "Go to End",
            PrevKey => "Previous Keyframe",
            NextKey => "Next Keyframe",
            Tool(t) => {
                return crate::app::Tool::ALL
                    .iter()
                    .find(|x| x.0 == t)
                    .map_or("Tool", |x| x.2)
                    .split(" (")
                    .next()
                    .unwrap_or("Tool")
                    .split(" —")
                    .next()
                    .unwrap_or("Tool")
                    .to_string();
            }
            CommitTool => "Finish the Current Tool (Enter)",
            ToggleFillStroke => "Switch Fill / Stroke",
            SwapFillStroke => "Swap Fill and Stroke Colours",
            ResetColors => "Reset Colours",
            BrushSmaller => "Smaller Brush",
            BrushBigger => "Bigger Brush",
        }
        .to_string()
    }

    pub fn category(self) -> Category {
        use Action::*;
        match self {
            New | Open | Save | SaveAs | Import | Export | Settings | Print | CloseTab | NextTab | PrevTab | Quit
            | ShortcutsHelp => Category::File,
            Undo | Redo | Cut | Copy | Paste | Duplicate | Delete | SelectAll | Deselect | SelectNext | SelectPrev
            | RenameLayer => Category::Edit,
            Group | Ungroup | Precompose | ClipMask | ReleaseClipMask | ToFront | Forward | Backward | ToBack
            | RotateCw | RotateCcw | Rotate180 | FlipH | FlipV => Category::Object,
            ZoomIn | ZoomOut | ZoomFit | Zoom100 | ToggleRulers | ToggleGrid | ToggleGuides => Category::View,
            PositionSize | NudgeLeft | NudgeRight | NudgeUp | NudgeDown => Category::Object,
            NewSlide | DuplicateSlide | DeleteSlide | AddPause | Present | PresentCurrent => Category::Slides,
            AiNew | AiNewSlide | AiModifySlide | AiImage => Category::Ai,
            PlayPause | FrameBack | FrameForward | GoStart | GoEnd | PrevKey | NextKey => Category::Time,
            Tool(_) | CommitTool | ToggleFillStroke | SwapFillStroke | ResetColors | BrushSmaller | BrushBigger => {
                Category::Tools
            }
        }
    }

    pub fn context(self) -> Ctx {
        use Action::*;
        match self {
            New | Open | Save | SaveAs | Import | Export | Settings | Print | CloseTab | NextTab | PrevTab | Quit
            | ShortcutsHelp => Ctx::Global,
            Undo | Redo | SelectAll | Duplicate | Group | Ungroup | ClipMask | ReleaseClipMask | ZoomIn | ZoomOut
            | ZoomFit | Zoom100 | RotateCw | RotateCcw | Rotate180 | Cut | Copy | Paste | Precompose | NewSlide
            | DuplicateSlide | Present | PresentCurrent | ToggleRulers | ToggleGrid | ToggleGuides | PositionSize
            | AiNew | AiNewSlide | AiModifySlide | AiImage => Ctx::NotTyping,
            BrushSmaller | BrushBigger => Ctx::BrushTool,
            _ => Ctx::Canvas,
        }
    }

    /// Delivered by the system as clipboard events, not key presses, so their
    /// keys can't be changed.
    pub fn fixed(self) -> bool {
        matches!(self, Action::Cut | Action::Copy | Action::Paste | Action::Precompose)
    }
}

fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// A key with modifiers. "Ctrl" means Cmd on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: Modifiers,
    pub key: Key,
}

impl Chord {
    pub const fn new(mods: Modifiers, key: Key) -> Self {
        Self { mods, key }
    }

    pub fn parse(s: &str) -> Option<Chord> {
        let mut mods = Modifiers::NONE;
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        // "Ctrl++" : the key is "+".
        let (key, rest) = if s.ends_with("++") {
            ("+", &parts[..parts.len().saturating_sub(2)])
        } else {
            (*parts.last()?, &parts[..parts.len() - 1])
        };
        for m in rest {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => mods.command = true,
                "shift" => mods.shift = true,
                "alt" | "option" => mods.alt = true,
                _ => return None,
            }
        }
        Some(Chord { mods, key: Key::from_name(key)? })
    }

    /// `"Ctrl+Shift+Z"`.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.mods.command || self.mods.ctrl {
            s.push_str(if cfg!(target_os = "macos") { "Cmd+" } else { "Ctrl+" });
        }
        if self.mods.alt {
            s.push_str("Alt+");
        }
        if self.mods.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.symbol_or_name());
        s
    }

    fn mod_count(&self) -> u8 {
        (self.mods.command || self.mods.ctrl) as u8 + self.mods.shift as u8 + self.mods.alt as u8
    }
}

fn c(mods: Modifiers, key: Key) -> Chord {
    Chord::new(mods, key)
}

/// The default (Browser/Figma-style) bindings.
pub fn defaults() -> HashMap<Action, Vec<Chord>> {
    use Action::*;
    const CTRL: Modifiers = Modifiers::COMMAND;
    const CTRL_SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::COMMAND };
    const CTRL_ALT: Modifiers = Modifiers { alt: true, ..Modifiers::COMMAND };
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const ALT: Modifiers = Modifiers::ALT;
    const NONE: Modifiers = Modifiers::NONE;
    let mut m: HashMap<Action, Vec<Chord>> = HashMap::new();
    let mut bind = |a: Action, chords: &[Chord]| {
        m.insert(a, chords.to_vec());
    };
    bind(New, &[c(CTRL, Key::N)]);
    bind(Open, &[c(CTRL, Key::O)]);
    bind(Save, &[c(CTRL, Key::S)]);
    bind(SaveAs, &[c(CTRL_SHIFT, Key::S)]);
    bind(Import, &[c(CTRL, Key::I)]);
    bind(Export, &[c(CTRL_SHIFT, Key::E)]);
    bind(Settings, &[c(CTRL, Key::Comma)]);
    bind(Print, &[c(CTRL, Key::P)]);
    bind(CloseTab, &[c(CTRL, Key::W)]);
    bind(NextTab, &[c(CTRL, Key::Tab)]);
    bind(PrevTab, &[c(CTRL_SHIFT, Key::Tab)]);
    bind(Quit, &[c(CTRL, Key::Q)]);
    bind(ShortcutsHelp, &[c(NONE, Key::F1)]);
    bind(Undo, &[c(CTRL, Key::Z)]);
    bind(Redo, &[c(CTRL_SHIFT, Key::Z), c(CTRL, Key::Y)]);
    bind(Cut, &[c(CTRL, Key::X)]);
    bind(Copy, &[c(CTRL, Key::C)]);
    bind(Paste, &[c(CTRL, Key::V)]);
    bind(Precompose, &[c(CTRL_SHIFT, Key::C)]);
    bind(Duplicate, &[c(CTRL, Key::D)]);
    bind(Delete, &[c(NONE, Key::Delete), c(NONE, Key::Backspace)]);
    bind(SelectAll, &[c(CTRL, Key::A)]);
    bind(Deselect, &[c(NONE, Key::Escape)]);
    // Photoshop's keys for the layer above/below (Tab moves between controls).
    bind(SelectNext, &[c(ALT, Key::CloseBracket)]);
    bind(SelectPrev, &[c(ALT, Key::OpenBracket)]);
    bind(RenameLayer, &[c(NONE, Key::F2)]);
    bind(Group, &[c(CTRL, Key::G)]);
    bind(Ungroup, &[c(CTRL_SHIFT, Key::G)]);
    bind(ClipMask, &[c(CTRL, Key::Num7)]);
    bind(ReleaseClipMask, &[c(CTRL_ALT, Key::Num7)]);
    bind(ToFront, &[c(NONE, Key::CloseBracket)]);
    bind(Forward, &[c(CTRL, Key::CloseBracket)]);
    bind(Backward, &[c(CTRL, Key::OpenBracket)]);
    bind(ToBack, &[c(NONE, Key::OpenBracket)]);
    bind(RotateCw, &[c(CTRL_SHIFT, Key::ArrowRight)]);
    bind(RotateCcw, &[c(CTRL_SHIFT, Key::ArrowLeft)]);
    bind(Rotate180, &[]);
    bind(FlipH, &[c(SHIFT, Key::H)]);
    bind(FlipV, &[c(SHIFT, Key::V)]);
    bind(ZoomIn, &[c(CTRL, Key::Equals), c(CTRL, Key::Plus)]);
    bind(ZoomOut, &[c(CTRL, Key::Minus)]);
    bind(ZoomFit, &[c(SHIFT, Key::Num1)]);
    bind(Zoom100, &[c(CTRL, Key::Num0), c(SHIFT, Key::Num0)]);
    bind(ToggleRulers, &[c(CTRL, Key::R)]);
    bind(ToggleGrid, &[c(CTRL, Key::Quote)]);
    bind(ToggleGuides, &[c(CTRL, Key::Semicolon)]);
    bind(PositionSize, &[c(CTRL, Key::T)]);
    bind(NewSlide, &[c(CTRL, Key::M)]);
    bind(AiNewSlide, &[c(CTRL_ALT, Key::M)]);
    bind(AiModifySlide, &[c(CTRL_ALT, Key::E)]);
    bind(DuplicateSlide, &[]);
    bind(DeleteSlide, &[]);
    bind(AddPause, &[c(CTRL_SHIFT, Key::P)]);
    bind(Present, &[c(NONE, Key::F5)]);
    bind(PresentCurrent, &[c(SHIFT, Key::F5)]);
    bind(PlayPause, &[c(NONE, Key::Space)]);
    // Arrows nudge the selection (and step frames when nothing is selected).
    bind(FrameBack, &[c(NONE, Key::PageUp), c(NONE, Key::Comma)]);
    bind(FrameForward, &[c(NONE, Key::PageDown), c(NONE, Key::Period)]);
    bind(NudgeLeft, &[c(NONE, Key::ArrowLeft), c(SHIFT, Key::ArrowLeft)]);
    bind(NudgeRight, &[c(NONE, Key::ArrowRight), c(SHIFT, Key::ArrowRight)]);
    bind(NudgeUp, &[c(NONE, Key::ArrowUp), c(SHIFT, Key::ArrowUp)]);
    bind(NudgeDown, &[c(NONE, Key::ArrowDown), c(SHIFT, Key::ArrowDown)]);
    bind(GoStart, &[c(NONE, Key::Home)]);
    bind(GoEnd, &[c(NONE, Key::End)]);
    bind(PrevKey, &[c(NONE, Key::J)]);
    bind(NextKey, &[c(NONE, Key::K)]);
    for (tool, _, _, key) in crate::app::Tool::ALL {
        bind(Tool(tool), &key.map(|k| vec![c(NONE, k)]).unwrap_or_default());
    }
    bind(Tool(crate::app::Tool::Slice), &[c(SHIFT, Key::C)]);
    bind(CommitTool, &[c(NONE, Key::Enter)]);
    bind(ToggleFillStroke, &[c(NONE, Key::X)]);
    bind(SwapFillStroke, &[c(SHIFT, Key::X)]);
    bind(ResetColors, &[c(NONE, Key::D)]);
    bind(BrushSmaller, &[c(NONE, Key::OpenBracket)]);
    bind(BrushBigger, &[c(NONE, Key::CloseBracket)]);
    m
}

/// The effective shortcuts: the defaults plus the user's changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    bindings: HashMap<Action, Vec<Chord>>,
    /// Actions the user rebound (only these are saved).
    changed: HashMap<Action, Vec<Chord>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self { bindings: defaults(), changed: HashMap::new() }
    }
}

/// Saved form: action id → chord texts.
type Saved = HashMap<String, Vec<String>>;

impl Keymap {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let saved: Saved = storage.and_then(|s| eframe::get_value(s, KEYMAP_KEY)).unwrap_or_default();
        Self::from_saved(&saved)
    }

    pub fn from_saved(saved: &Saved) -> Self {
        let mut km = Self::default();
        for (id, chords) in saved {
            let Some(a) = Action::from_id(id) else { continue };
            let chords: Vec<Chord> = chords.iter().filter_map(|t| Chord::parse(t)).collect();
            km.set(a, chords);
        }
        km
    }

    pub fn to_saved(&self) -> Saved {
        self.changed.iter().map(|(a, cs)| (a.id(), cs.iter().map(Chord::text).collect())).collect()
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, KEYMAP_KEY, &self.to_saved());
    }

    pub fn chords(&self, a: Action) -> &[Chord] {
        self.bindings.get(&a).map_or(&[], Vec::as_slice)
    }

    /// The first shortcut as text, for menus (`""` when unbound).
    pub fn text(&self, a: Action) -> String {
        self.chords(a).first().map_or_else(String::new, Chord::text)
    }

    /// A tooltip with the action's current shortcut: "Play / pause (Space)",
    /// or just `text` when it has none (so hints follow rebinding).
    pub fn tip(&self, a: Action, text: &str) -> String {
        match self.text(a) {
            k if k.is_empty() => text.to_string(),
            k => format!("{text} ({k})"),
        }
    }

    /// All shortcuts as text: `"Ctrl+Shift+Z, Ctrl+Y"`.
    pub fn all_text(&self, a: Action) -> String {
        self.chords(a).iter().map(Chord::text).collect::<Vec<_>>().join(", ")
    }

    pub fn set(&mut self, a: Action, chords: Vec<Chord>) {
        if a.fixed() {
            return;
        }
        if defaults().get(&a).is_some_and(|d| *d == chords) {
            self.changed.remove(&a);
        } else {
            self.changed.insert(a, chords.clone());
        }
        self.bindings.insert(a, chords);
    }

    pub fn reset(&mut self, a: Action) {
        self.set(a, defaults().remove(&a).unwrap_or_default());
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }

    pub fn is_default(&self, a: Action) -> bool {
        !self.changed.contains_key(&a)
    }

    /// Other actions that `chord` would also trigger where `a` works.
    pub fn conflicts_with(&self, a: Action, chord: Chord) -> Vec<Action> {
        self.bindings
            .iter()
            .filter(|(b, cs)| **b != a && cs.contains(&chord) && overlaps(a, **b))
            .map(|(b, _)| *b)
            .collect()
    }

    /// Every pair of actions sharing a chord where both can fire.
    pub fn conflicts(&self) -> Vec<(Chord, Action, Action)> {
        let mut out = Vec::new();
        let all = Action::all();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                for ch in self.chords(*a) {
                    if self.chords(*b).contains(ch) && overlaps(*a, *b) {
                        out.push((*ch, *a, *b));
                    }
                }
            }
        }
        out
    }

    /// The actions whose keys were pressed this frame (consuming the key
    /// events). Chords with more modifiers are checked first, so Ctrl+Shift+Z
    /// isn't taken by Ctrl+Z.
    pub fn poll(&self, ctx: &egui::Context, state: KeyState) -> Vec<Action> {
        let mut candidates: Vec<(Chord, Action)> = self
            .bindings
            .iter()
            .filter(|(a, _)| !a.fixed() && state.allows(**a))
            .flat_map(|(a, cs)| cs.iter().map(move |c| (*c, *a)))
            .collect();
        // Most specific first; brush sizing beats arranging on the same key.
        candidates.sort_by_key(|(c, a)| (std::cmp::Reverse(c.mod_count()), a.context() != Ctx::BrushTool));
        let mut fired = Vec::new();
        for (chord, action) in candidates {
            // Space plays on release, unless it was held to pan the canvas.
            if chord.key == Key::Space && chord.mods.is_none() {
                let released = ctx.input_mut(|i| {
                    let pos = i.events.iter().position(|e| {
                        matches!(e, egui::Event::Key { key: Key::Space, pressed: false, modifiers, .. } if modifiers.is_none())
                    });
                    pos.map(|p| i.events.remove(p)).is_some()
                });
                // Swallow the press so nothing else reacts to it.
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space));
                if released && !state.space_panned && !fired.contains(&action) {
                    fired.push(action);
                }
                continue;
            }
            let shortcut = KeyboardShortcut::new(chord.mods, chord.key);
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) && !fired.contains(&action) {
                fired.push(action);
            }
        }
        fired
    }
}

/// Whether two actions can fire in the same situation.
fn overlaps(a: Action, b: Action) -> bool {
    use Ctx::*;
    match (a.context(), b.context()) {
        // Brush sizing wins over canvas keys while a brush tool is active; that's intended.
        (BrushTool, Canvas) | (Canvas, BrushTool) => false,
        _ => true,
    }
}

/// What's going on when keys are checked.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyState {
    pub typing: bool,
    pub dialog_open: bool,
    pub brush_tool: bool,
    /// Space was held to pan the canvas since it was pressed.
    pub space_panned: bool,
}

impl KeyState {
    fn allows(&self, a: Action) -> bool {
        match a.context() {
            Ctx::Global => true,
            Ctx::NotTyping => !self.typing,
            Ctx::Canvas => !self.typing && !self.dialog_open,
            Ctx::BrushTool => !self.typing && !self.dialog_open && self.brush_tool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip() {
        for text in ["Ctrl+Shift+Z", "Ctrl+Alt+7", "F1", "Shift+1", "Ctrl+]", "Ctrl+,", "Space"] {
            let chord = Chord::parse(text).unwrap_or_else(|| panic!("{text}"));
            let back = chord.text();
            if !cfg!(target_os = "macos") {
                assert_eq!(back, text);
            }
            assert_eq!(Chord::parse(&back), Some(chord));
        }
        assert_eq!(Chord::parse("Ctrl++").map(|c| c.key), Some(Key::Plus));
        assert_eq!(Chord::parse("Hyper+Q"), None);
        assert_eq!(Chord::parse("Ctrl+NoSuchKey"), None);
    }

    #[test]
    fn defaults_have_no_conflicts_and_every_action_is_described() {
        let km = Keymap::default();
        assert_eq!(km.conflicts(), vec![], "default shortcuts clash");
        let mut ids = std::collections::HashSet::new();
        for a in Action::all() {
            assert!(!a.label().is_empty(), "{a:?} has no label");
            assert!(ids.insert(a.id()), "duplicate id {}", a.id());
            assert_eq!(Action::from_id(&a.id()), Some(a));
        }
        assert_eq!(Action::Duplicate.id(), "edit.duplicate");
        assert_eq!(Action::Tool(Tool::Pen).id(), "tool.pen");
        assert_eq!(km.text(Action::Duplicate), if cfg!(target_os = "macos") { "Cmd+D" } else { "Ctrl+D" });
        assert_eq!(Action::Tool(Tool::Pen).label(), "Pen");
    }

    #[test]
    fn only_changes_are_saved_and_unknown_entries_are_ignored() {
        let mut km = Keymap::default();
        assert!(km.to_saved().is_empty());
        let chord = Chord::parse("Ctrl+Shift+D").unwrap();
        km.set(Action::Duplicate, vec![chord]);
        let saved = km.to_saved();
        assert_eq!(saved.len(), 1);
        let mut saved_with_junk = saved.clone();
        saved_with_junk.insert("edit.no_such_action".into(), vec!["Ctrl+Q".into()]);
        let back = Keymap::from_saved(&saved_with_junk);
        assert_eq!(back.chords(Action::Duplicate), [chord]);
        assert!(!back.is_default(Action::Duplicate));
        // Setting it back to the default forgets the change.
        km.reset(Action::Duplicate);
        assert!(km.to_saved().is_empty());
        // Clipboard keys can't be rebound.
        km.set(Action::Copy, vec![chord]);
        assert_eq!(km.text(Action::Copy), Keymap::default().text(Action::Copy));
    }

    #[test]
    fn conflicts_name_the_other_action() {
        let km = Keymap::default();
        let ctrl_d = Chord::parse("Ctrl+D").unwrap();
        assert_eq!(km.conflicts_with(Action::Group, ctrl_d), vec![Action::Duplicate]);
        // Brush sizing and arranging share [ and ] on purpose.
        let bracket = Chord::parse("[").unwrap();
        assert_eq!(km.conflicts_with(Action::BrushSmaller, bracket), vec![]);
    }

    #[test]
    fn contexts() {
        let typing = KeyState { typing: true, ..Default::default() };
        assert!(typing.allows(Action::Save));
        assert!(!typing.allows(Action::Undo), "text fields keep their own undo");
        let dialog = KeyState { dialog_open: true, ..Default::default() };
        assert!(dialog.allows(Action::Undo) && !dialog.allows(Action::Delete));
        assert!(!KeyState::default().allows(Action::BrushBigger));
        assert!(KeyState { brush_tool: true, ..Default::default() }.allows(Action::BrushBigger));
    }
}
