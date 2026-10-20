//! Starting up, keyboard shortcuts.

use super::common::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use xenify_ui::testing::FakeLog;

#[test]
fn app_starts_headless() {
    let h = harness(FakeLog::default());
    h.get_by_label("File");
}

#[test]
fn space_tap_plays_and_pauses() {
    let mut h = harness(FakeLog::default());
    assert!(!h.state().playing);
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(h.state().playing);
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(!h.state().playing);
}

#[test]
fn select_all_duplicate_and_undo() {
    let mut h = harness(FakeLog::default());
    let before = ids(&h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    let selected = h.state().selected_layers();
    assert_eq!(selected.len(), before.len(), "the demo's layers are all top-level objects");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), before.len() * 2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&h), before);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    h.run_steps(2);
    assert_eq!(ids(&h).len(), before.len() * 2, "Ctrl+Shift+Z redoes (not undo again)");
}

#[test]
fn brackets_arrange_the_selection() {
    let mut h = harness(FakeLog::default());
    let bottom = ids(&h)[0];
    h.state_mut().select_only(Some(bottom));
    h.key_press(Key::CloseBracket);
    h.run_steps(2);
    assert_eq!(*ids(&h).last().unwrap(), bottom, "] brings to front");
    h.key_press(Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(ids(&h)[0], bottom, "[ sends to back");
}

#[test]
fn flip_rotates_around_the_selection() {
    let mut h = harness(FakeLog::default());
    let id = ids(&h)[1];
    let pos = |h: &Harness| h.state().project().layer(id).unwrap().transform.scale.value_at(0.0);
    h.state_mut().select_only(Some(id));
    let before = pos(&h);
    h.key_press_modifiers(Modifiers::SHIFT, Key::H);
    h.run_steps(2);
    let after = pos(&h);
    assert!((after.x.abs() - before.x.abs()).abs() < 1e-9 && after != before, "{before:?} → {after:?}");
}

#[test]
fn holding_space_and_dragging_pans_instead_of_playing() {
    let mut h = harness(FakeLog::default());
    let id = ids(&h)[2];
    h.state_mut().select_only(Some(id));
    let before = h.state().project().clone();
    // Somewhere on the canvas (left of the side panels, below the menus).
    let (from, to) = (egui::pos2(300.0, 300.0), egui::pos2(420.0, 360.0));
    h.key_down(Key::Space);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=5 {
        h.hover_at(from + (to - from) * (i as f32 / 5.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(1);
    h.key_up(Key::Space);
    h.run_steps(2);
    assert!(!h.state().playing, "letting go of Space after panning doesn't play");
    assert_eq!(*h.state().project(), before, "panning doesn't move anything");
}

/// Characters in the UI's string literals that the installed fonts can't draw
/// (they'd show as empty boxes).
#[test]
fn every_ui_string_can_be_drawn_with_the_fonts() {
    let h = harness(FakeLog::default());
    // How `c` is drawn. (`has_glyph` can't tell a real glyph from the
    // fallback box when both come from the emoji font.)
    let drawn = |c: char| {
        let font = egui::FontId::proportional(14.0);
        let g = h.ctx.fonts_mut(|f| f.layout_no_wrap(c.to_string(), font, egui::Color32::WHITE));
        g.rows.first().and_then(|r| r.row.glyphs.first()).map(|g| g.uv_rect)
    };
    let fallback = drawn('\u{10FFFD}');
    assert_eq!(drawn('⤒'), fallback, "the check spots a glyph no font has");
    assert_ne!(drawn('🔒'), fallback, "and knows emoji-font glyphs");
    let mut missing = Vec::new();
    let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                // Characters between quotes (a simple scan; escapes skipped).
                let mut inside = false;
                let mut chars = line.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' if inside => {
                            chars.next();
                        }
                        '"' => inside = !inside,
                        c if inside && !c.is_ascii() && c != '\u{fe0f}' && drawn(c) == fallback => {
                            missing.push(format!("{c:?} at {}:{}", path.display(), n + 1));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    assert!(missing.is_empty(), "no glyph for: {missing:#?}");
}
