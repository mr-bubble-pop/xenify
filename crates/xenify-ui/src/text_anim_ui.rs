//! Text animators in the UI: Properties › Text › Animators (add a preset,
//! then change unit, order, timing, easing and the "from" state), and a row
//! per animator under the text layer in the Layers panel.

use crate::app::XenifyApp;
use crate::widgets::ACCENT;
use egui::{Color32, Pos2, RichText, Sense};
use xenify_doc::text_anim::PRESETS;
use xenify_doc::{Id, Layer, TextAnimator, TextLayer, TextOrder, TextUnit};

/// The "Add Animation" menu: the presets.
pub fn add_menu(app: &mut XenifyApp, ui: &mut egui::Ui, layer: Id) {
    for name in PRESETS {
        if ui.button(*name).clicked() {
            add(app, layer, name);
            ui.close();
        }
    }
}

/// Adds preset `name`, starting at the playhead.
pub fn add(app: &mut XenifyApp, layer: Id, name: &str) {
    let start = app.project().layer(layer).map_or(0.0, |l| (app.time - l.meta.in_point).max(0.0));
    match xenify_ops::text_anim::add_preset(app.project(), layer, name, start) {
        Ok(cmd) => {
            if app.exec(&format!("Add {name}"), cmd) {
                app.status = format!("Added “{name}” from the playhead. Press Space to watch it.");
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// Properties › Text › Animators.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, text: &TextLayer) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Animators").strong())
            .on_hover_text("Letters, words or lines animating in one after another");
        ui.menu_button("+ Add Animation", |ui| add_menu(app, ui, layer.id))
            .response
            .on_hover_text("Add a text animation");
    });
    if text.animators.is_empty() {
        ui.weak("None yet: + Add Animation picks one (Typewriter, Pop by Letter…).");
    }
    for (i, a) in text.animators.iter().enumerate() {
        egui::CollapsingHeader::new(RichText::new(if a.name.is_empty() { "Animation" } else { &a.name }))
            .id_salt(("text-anim", layer.id, i))
            .default_open(true)
            .show(ui, |ui| editor(app, ui, layer.id, i, a));
    }
}

fn editor(app: &mut XenifyApp, ui: &mut egui::Ui, layer: Id, index: usize, a: &TextAnimator) {
    let mut b = a.clone();
    let mut begin = false;
    ui.horizontal(|ui| {
        ui.checkbox(&mut b.enabled, "On");
        ui.checkbox(&mut b.out, "Animate out").on_hover_text("Leave for the “from” state instead of arriving");
        if ui.button("Remove").clicked()
            && let Ok(cmd) = xenify_ops::text_anim::remove(app.project(), layer, index)
        {
            app.exec("Remove text animation", cmd);
        }
    });
    egui::Grid::new(("text-anim-grid", layer, index)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("By");
        egui::ComboBox::from_id_salt(("unit", layer, index)).selected_text(b.unit.label()).show_ui(ui, |ui| {
            for u in TextUnit::ALL {
                ui.selectable_value(&mut b.unit, u, u.label());
            }
        });
        ui.end_row();
        ui.label("Order");
        egui::ComboBox::from_id_salt(("order", layer, index)).selected_text(b.order.label()).show_ui(ui, |ui| {
            for o in TextOrder::ALL {
                ui.selectable_value(&mut b.order, o, o.label());
            }
        });
        ui.end_row();
        let mut num = |ui: &mut egui::Ui, label: &str, tip: &str, v: &mut f64, range, speed, suffix: &str| {
            ui.label(label).on_hover_text(tip);
            let r = ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix));
            begin |= r.drag_started();
            ui.end_row();
        };
        num(ui, "Starts at", "When the first one starts (layer time)", &mut b.start, 0.0..=3600.0, 0.01, " s");
        num(ui, "Stagger", "Time between one starting and the next", &mut b.stagger, 0.0..=10.0, 0.005, " s");
        num(ui, "Each takes", "How long each one animates", &mut b.duration, 0.0..=30.0, 0.01, " s");
        ui.label("Easing");
        let current = xenify_doc::Easing::PRESETS.iter().find(|(_, e)| *e == b.easing).map(|(n, _)| *n);
        ui.menu_button(current.map_or("custom".to_string(), crate::curve::preset_label), |ui| {
            if let Some(e) = crate::curve::preset_picker(ui, Some(b.easing)) {
                b.easing = e;
                ui.close();
            }
        });
        ui.end_row();
        ui.label(RichText::new("From").color(ACCENT)).on_hover_text("How each one starts; it animates to rest");
        ui.end_row();
        let mut x = b.from.offset.x;
        let mut y = b.from.offset.y;
        num(ui, "Move X", "Starts this far to the side", &mut x, -5000.0..=5000.0, 1.0, " px");
        num(ui, "Move Y", "Starts this far down (negative: up)", &mut y, -5000.0..=5000.0, 1.0, " px");
        b.from.offset = kurbo::Vec2::new(x, y);
        let mut scale = b.from.scale * 100.0;
        num(ui, "Size", "Starts at this size", &mut scale, 0.0..=1000.0, 1.0, " %");
        b.from.scale = scale / 100.0;
        num(ui, "Turn", "Starts turned by", &mut b.from.rotation, -3600.0..=3600.0, 1.0, "°");
        let mut opacity = b.from.opacity * 100.0;
        num(ui, "Opacity", "Starts this visible", &mut opacity, 0.0..=100.0, 1.0, " %");
        b.from.opacity = opacity / 100.0;
        num(ui, "Blur", "Starts this blurred", &mut b.from.blur, 0.0..=200.0, 0.2, " px");
        num(
            ui,
            "Spacing",
            "Starts spread apart by this much per letter",
            &mut b.from.tracking,
            -200.0..=500.0,
            0.5,
            " px",
        );
        if b.order == TextOrder::Random {
            let mut seed = b.seed as f64;
            num(ui, "Variation", "A different random order", &mut seed, 0.0..=9999.0, 0.2, "");
            b.seed = seed.round() as u64;
        }
    });
    if begin {
        app.new_gesture();
    }
    if b != *a
        && let Ok(cmd) = xenify_ops::text_anim::edit(app.project(), layer, index, |x| *x = b)
    {
        app.exec_merged("Change text animation", cmd);
    }
}

/// A row per animator under a text layer in the Layers panel: 👁 turns it
/// on and off, a click selects the layer, the menu removes it.
pub fn layer_rows(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer, depth: usize) {
    let xenify_doc::LayerKind::Text(t) = &layer.kind else { return };
    let indent = 4.0 + depth as f32 * 16.0 + 18.0 + 48.0;
    for (i, a) in t.animators.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
        let r = ui.interact(rect, egui::Id::new(("text-anim-row", layer.id, i)), Sense::click());
        let label = format!("Aa {}", if a.name.is_empty() { "Animation" } else { &a.name });
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let painter = ui.painter_at(rect);
        if r.hovered() {
            painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let dim = Color32::from_gray(90);
        let colour = if a.enabled { ui.visuals().text_color() } else { dim };
        let y = rect.center().y;
        painter.text(
            Pos2::new(rect.left() + indent + 10.0, y),
            egui::Align2::CENTER_CENTER,
            "👁",
            egui::FontId::proportional(12.0),
            colour,
        );
        painter.text(
            Pos2::new(rect.left() + indent + 24.0, y),
            egui::Align2::LEFT_CENTER,
            "Aa",
            egui::FontId::proportional(11.0),
            if a.enabled { ACCENT } else { dim },
        );
        painter.text(
            Pos2::new(rect.left() + indent + 42.0, y),
            egui::Align2::LEFT_CENTER,
            if a.name.is_empty() { "Animation" } else { &a.name },
            egui::FontId::proportional(12.5),
            colour,
        );
        let eye = egui::Rect::from_min_size(Pos2::new(rect.left() + indent, rect.top()), egui::vec2(20.0, 18.0));
        let eye = ui
            .interact(eye, egui::Id::new(("text-anim-eye", layer.id, i)), Sense::click())
            .on_hover_text(if a.enabled { "Turn this animation off" } else { "Turn this animation on" });
        eye.widget_info(|| {
            let name = if a.name.is_empty() { "Animation" } else { &a.name };
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, a.enabled, format!("{name} on"))
        });
        let toggle = |app: &mut XenifyApp| {
            if let Ok(cmd) = xenify_ops::text_anim::edit(app.project(), layer.id, i, |x| x.enabled = !x.enabled) {
                app.exec("Toggle text animation", cmd);
            }
        };
        if eye.clicked() {
            toggle(app);
        } else if r.clicked() {
            app.select_only(Some(layer.id));
        }
        r.on_hover_text("Text animation: its settings are in Properties › Text").context_menu(|ui| {
            if ui.button(if a.enabled { "Turn Off" } else { "Turn On" }).clicked() {
                toggle(app);
                ui.close();
            }
            if ui.button("Remove").clicked() {
                if let Ok(cmd) = xenify_ops::text_anim::remove(app.project(), layer.id, i) {
                    app.exec("Remove text animation", cmd);
                }
                ui.close();
            }
        });
    }
}
