//! Particle layers in the UI: Layer › New › Particles (presets) and
//! Properties › Particles (how many and when, where from, which way, the
//! forces, the look and how they finish). The canvas handles are in
//! `viewport/emitter_handle.rs`.

use crate::app::XenifyApp;
use egui::RichText;
use xenify_doc::{
    Color, EmitMode, Emitter, EmitterField, EmitterShape, EndLook, Id, Layer, LayerKind, PARTICLE_PRESETS,
    ParticleShape, PropPath,
};

/// Layer › New › Particles.
pub fn menu(app: &mut XenifyApp, ui: &mut egui::Ui) {
    for name in PARTICLE_PRESETS.iter().chain([&"—"]).chain(xenify_doc::FIRE_PRESETS) {
        if *name == "—" {
            ui.separator();
            continue;
        }
        let tip = match *name {
            "Assemble into Shape" => "Particles fly in and form the selected layer's outline",
            "Confetti Burst" => "A burst of paper confetti from the bottom",
            "Fireworks" => "Bursts of sparks, one after another",
            "Burning Shape" => "Flames coming off the selected layer",
            "Fire Trail" | "Smoke Trail" => "Move the layer: it leaves a trail",
            _ => "",
        };
        if ui.button(*name).on_hover_text(tip).clicked() {
            add(app, name);
            ui.close();
        }
    }
    ui.separator();
    for kind in [xenify_ops::blast::BlastKind::Explosion, xenify_ops::blast::BlastKind::BigBlast] {
        let tip = match kind {
            xenify_ops::blast::BlastKind::Explosion => "Fire, sparks, smoke and a shockwave ring, as a group of layers",
            xenify_ops::blast::BlastKind::BigBlast => {
                "A mushroom cloud: flash, shockwave, fireball, stem, rolling cap, dust, debris and camera shake"
            }
        };
        if ui.button(kind.label()).on_hover_text(tip).clicked() {
            add_blast(app, kind);
            ui.close();
        }
    }
}

/// Adds an explosion at the playhead, on the lower middle of the canvas.
pub fn add_blast(app: &mut XenifyApp, kind: xenify_ops::blast::BlastKind) {
    let s = &app.comp().settings;
    let (w, h) = (s.width as f64, s.height as f64);
    let at = match kind {
        xenify_ops::blast::BlastKind::BigBlast => kurbo::Vec2::new(w / 2.0, h * 0.85),
        xenify_ops::blast::BlastKind::Explosion => kurbo::Vec2::new(w / 2.0, h / 2.0),
    };
    let scale = (w.min(h) / 1080.0).max(0.1);
    let first = Id(app.project().next_id);
    match xenify_ops::blast::add_blast(app.project(), app.comp, first, kind, at, app.time, scale) {
        Ok((cmd, group)) => {
            if app.exec(&format!("Add {}", kind.label()), cmd) {
                app.select_only(Some(group));
                app.status = format!(
                    "Added a {} group: every part (flash, shockwave, fire, smoke…) is its own layer to change.",
                    kind.label()
                );
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

pub fn add(app: &mut XenifyApp, name: &str) {
    let id = Id(app.project().next_id);
    // "Assemble into Shape" forms the selected layer; "Burning Shape" burns it.
    let target =
        app.selected.filter(|s| app.project().layer(*s).is_ok_and(|l| !matches!(l.kind, LayerKind::Particles(_))));
    match xenify_ops::particles::add_preset(app.project(), app.comp, id, name, target) {
        Ok(cmd) => {
            if app.exec(&format!("Add {name}"), cmd) {
                app.select_only(Some(id));
                app.status =
                    format!("Added {name}. Drag the arrow to aim it; everything else is in Properties › Particles.");
            }
        }
        Err(e) => app.fail(e.to_string()),
    }
}

/// Properties › Particles.
pub fn section(app: &mut XenifyApp, ui: &mut egui::Ui, layer: &Layer) {
    let LayerKind::Particles(e) = &layer.kind else { return };
    let e: Emitter = (**e).clone();
    let mut b = e.clone();
    let mut begin = false;
    let others: Vec<(Id, String)> = app
        .comp()
        .layers
        .iter()
        .filter(|l| l.id != layer.id && !matches!(l.kind, LayerKind::Particles(_) | LayerKind::Camera(_)))
        .map(|l| (l.id, l.meta.name.clone()))
        .collect();
    let name_of = |id: Id| others.iter().find(|(o, _)| *o == id).map_or("(gone)".to_string(), |(_, n)| n.clone());
    let pick_layer = |ui: &mut egui::Ui, salt: &str, current: Option<Id>| -> Option<Id> {
        let mut out = None;
        egui::ComboBox::from_id_salt((salt, layer.id))
            .selected_text(current.map_or("Choose a layer…".into(), name_of))
            .show_ui(ui, |ui| {
                for (id, name) in &others {
                    if ui.selectable_label(current == Some(*id), name).clicked() {
                        out = Some(*id);
                    }
                }
            });
        out
    };

    egui::CollapsingHeader::new(RichText::new("Particles").strong()).default_open(true).show(ui, |ui| {
        // How many, and when.
        ui.horizontal(|ui| {
            ui.label("Emit");
            ui.selectable_value(&mut b.mode, EmitMode::Burst, "Burst").on_hover_text("All at once");
            ui.selectable_value(&mut b.mode, EmitMode::Stream, "Stream").on_hover_text("A steady flow");
        });
        egui::Grid::new(("emit", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            let path = if b.mode == EmitMode::Burst { EmitterField::Count } else { EmitterField::Rate };
            if b.mode == e.mode {
                crate::panels::prop_row(app, ui, layer, PropPath::Emitter(path));
            }
            crate::panels::prop_row_named(app, ui, layer, PropPath::Emitter(EmitterField::Lifetime), "Lifetime (s)");
        });
        egui::Grid::new(("emit-2", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            let num = |ui: &mut egui::Ui, label: &str, tip: &str, v: &mut f64, range, speed, suffix: &str| {
                ui.label(label).on_hover_text(tip);
                let r = ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix));
                ui.end_row();
                r.drag_started()
            };
            begin |= num(ui, "Starts at", "When the first particles leave", &mut b.start, 0.0..=3600.0, 0.01, " s");
            if b.mode == EmitMode::Burst {
                let mut repeat = b.repeat as f64;
                begin |= num(ui, "Repeats", "More bursts after the first", &mut repeat, 0.0..=100.0, 0.05, "");
                b.repeat = repeat.round() as u32;
                if b.repeat > 0 {
                    begin |= num(ui, "Every", "Time between bursts", &mut b.interval, 0.01..=60.0, 0.01, " s");
                }
            } else {
                ui.label("Prewarm");
                ui.checkbox(&mut b.prewarm, "Already going at the start")
                    .on_hover_text("Snow already falling, rain already pouring");
                ui.end_row();
                let mut stops = b.stop.is_some();
                ui.label("Stops");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut stops, "");
                    if stops {
                        let mut t = b.stop.unwrap_or(b.start + 2.0);
                        begin |= ui
                            .add(egui::DragValue::new(&mut t).range(0.0..=3600.0).speed(0.01).suffix(" s"))
                            .drag_started();
                        b.stop = Some(t);
                    } else {
                        b.stop = None;
                        ui.weak("keeps going");
                    }
                });
                ui.end_row();
            }
            let mut lv = b.lifetime_variation * 100.0;
            begin |= num(ui, "Lifetime ±", "How much lifetimes differ", &mut lv, 0.0..=100.0, 0.5, " %");
            b.lifetime_variation = lv / 100.0;
        });

        // Where from.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("From");
            egui::ComboBox::from_id_salt(("source", layer.id)).selected_text(b.shape.label()).show_ui(ui, |ui| {
                let (w, h) = (app.comp().settings.width as f64, app.comp().settings.height as f64);
                let choices = [
                    EmitterShape::Point,
                    EmitterShape::Line { length: w * 0.5 },
                    EmitterShape::Rect { size: kurbo::Vec2::new(w * 0.3, h * 0.3) },
                    EmitterShape::Ellipse { size: kurbo::Vec2::new(w * 0.3, w * 0.3) },
                ];
                for c in choices {
                    if ui
                        .selectable_label(std::mem::discriminant(&b.shape) == std::mem::discriminant(&c), c.label())
                        .clicked()
                    {
                        b.shape = c;
                    }
                }
                if let Some((id, _)) = others.first()
                    && ui.selectable_label(matches!(b.shape, EmitterShape::Layer { .. }), "Layer outline").clicked()
                {
                    b.shape = EmitterShape::Layer { layer: *id };
                }
            });
        });
        match &mut b.shape {
            EmitterShape::Line { length } => {
                ui.horizontal(|ui| {
                    ui.label("Length");
                    begin |= ui.add(egui::DragValue::new(length).range(0.0..=100_000.0).suffix(" px")).drag_started();
                });
            }
            EmitterShape::Rect { size } | EmitterShape::Ellipse { size } => {
                ui.horizontal(|ui| {
                    ui.label("Size");
                    begin |=
                        ui.add(egui::DragValue::new(&mut size.x).range(0.0..=100_000.0).suffix(" px")).drag_started();
                    ui.label("×");
                    begin |=
                        ui.add(egui::DragValue::new(&mut size.y).range(0.0..=100_000.0).suffix(" px")).drag_started();
                });
            }
            EmitterShape::Layer { layer: source } => {
                ui.horizontal(|ui| {
                    ui.label("Layer");
                    if let Some(id) = pick_layer(ui, "source-layer", Some(*source)) {
                        *source = id;
                    }
                });
            }
            EmitterShape::Point => {}
        }

        // Which way, and the forces on them.
        ui.add_space(4.0);
        ui.weak("Direction: drag the arrow on the canvas (Shift: 15° steps). Roll turns smoke round an Ellipse source, like a mushroom cloud's cap.");
        egui::Grid::new(("motion", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            use EmitterField as F;
            for (f, label) in [
                (F::Direction, "Direction (°)"),
                (F::Spread, "Spread (°)"),
                (F::Speed, "Speed (px/s)"),
                (F::Spin, "Spin (°/s)"),
                (F::Gravity, "Gravity (px/s²)"),
                (F::Drag, "Drag"),
                (F::Turbulence, "Turbulence (px)"),
                (F::Vortex, "Roll (°/s)"),
            ] {
                crate::panels::prop_row_named(app, ui, layer, PropPath::Emitter(f), label);
            }
        });
        egui::Grid::new(("motion-2", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            let mut pct = |ui: &mut egui::Ui, label: &str, tip: &str, v: &mut f64| {
                ui.label(label).on_hover_text(tip);
                let mut shown = *v * 100.0;
                let r = ui.add(egui::DragValue::new(&mut shown).range(0.0..=100.0).speed(0.5).suffix(" %"));
                begin |= r.drag_started();
                *v = shown / 100.0;
                ui.end_row();
            };
            pct(ui, "Speed ±", "How much speeds differ", &mut b.speed_variation);
            ui.label("Swirl speed").on_hover_text("How fast the turbulence changes");
            begin |= ui
                .add(egui::DragValue::new(&mut b.turbulence_speed).range(0.0..=20.0).speed(0.02).suffix(" /s"))
                .drag_started();
            ui.end_row();
        });

        // The look.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Each is a");
            egui::ComboBox::from_id_salt(("particle", layer.id)).selected_text(b.particle.label()).show_ui(ui, |ui| {
                for p in ParticleShape::SIMPLE {
                    ui.selectable_value(&mut b.particle, p.clone(), p.label());
                }
                if let Some((id, _)) = others.first()
                    && ui
                        .selectable_label(matches!(b.particle, ParticleShape::Layer { .. }), "A layer (sprite)")
                        .clicked()
                {
                    b.particle = ParticleShape::Layer { layer: *id };
                }
            });
            if let ParticleShape::Layer { layer: sprite } = &mut b.particle
                && let Some(id) = pick_layer(ui, "sprite", Some(*sprite))
            {
                *sprite = id;
            }
        });
        egui::Grid::new(("look", layer.id)).num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
            crate::panels::prop_row_named(app, ui, layer, PropPath::Emitter(EmitterField::Size), "Size (px)");
        });
        egui::Grid::new(("look-2", layer.id)).num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
            ui.label("Size ±");
            let mut sv = b.size_variation * 100.0;
            begin |= ui.add(egui::DragValue::new(&mut sv).range(0.0..=100.0).speed(0.5).suffix(" %")).drag_started();
            b.size_variation = sv / 100.0;
            ui.end_row();
            ui.label("Size at the end").on_hover_text("Relative to the start: 300% grows like smoke");
            let mut se = b.size_end * 100.0;
            begin |= ui.add(egui::DragValue::new(&mut se).range(0.0..=5000.0).speed(1.0).suffix(" %")).drag_started();
            b.size_end = se / 100.0;
            ui.end_row();
        });
        // Colours: a random pick each, or a gradient over their life.
        let mut over_life = b.color_over_life.is_some();
        ui.horizontal(|ui| {
            ui.label("Colours");
            if ui
                .checkbox(&mut over_life, "Change over their life")
                .on_hover_text("Fire: white → yellow → red → smoke")
                .changed()
            {
                b.color_over_life = over_life.then(|| {
                    let n = b.colors.len().max(1);
                    b.colors
                        .iter()
                        .enumerate()
                        .map(|(i, c)| xenify_doc::GradientStop { offset: i as f64 / (n - 1).max(1) as f64, color: *c })
                        .collect()
                });
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(stops) = &mut b.color_over_life {
                for (i, s) in stops.iter_mut().enumerate() {
                    crate::color_picker::button(ui, ("life-stop", layer.id.0, i), &mut s.color);
                }
                if ui.small_button("+").on_hover_text("Another colour at the end").clicked() {
                    let last = stops.last().map_or(Color::WHITE, |s| s.color);
                    stops.push(xenify_doc::GradientStop { offset: 1.0, color: last });
                    let n = stops.len();
                    for (i, s) in stops.iter_mut().enumerate() {
                        s.offset = i as f64 / (n - 1).max(1) as f64;
                    }
                }
            } else {
                let mut remove = None;
                for (i, c) in b.colors.iter_mut().enumerate() {
                    let r = crate::color_picker::button(ui, ("particle-colour", layer.id.0, i), c);
                    let _ = r;
                    if ui.small_button("−").on_hover_text("Remove this colour").clicked() {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove.filter(|_| b.colors.len() > 1) {
                    b.colors.remove(i);
                }
                if ui.small_button("+").on_hover_text("Add a colour").clicked() {
                    b.colors.push(b.colors.last().copied().unwrap_or(Color::WHITE));
                }
            }
        });

        // How they finish.
        ui.add_space(4.0);
        ui.label("Finally");
        ui.horizontal_wrapped(|ui| {
            let floor = app.comp().settings.height as f64 - 20.0;
            let mut choices = vec![EndLook::Fade, EndLook::Shrink, EndLook::Settle { floor }];
            if let Some((id, _)) = others.first() {
                choices.push(EndLook::Form { target: *id });
            }
            for c in choices {
                let on = std::mem::discriminant(&b.end) == std::mem::discriminant(&c);
                if ui.selectable_label(on, c.label()).clicked() && !on {
                    b.end = c;
                }
            }
        });
        match &mut b.end {
            EndLook::Settle { floor } => {
                ui.horizontal(|ui| {
                    ui.label("Floor at").on_hover_text("Canvas y where they come to rest");
                    begin |= ui.add(egui::DragValue::new(floor).speed(1.0).suffix(" px")).drag_started();
                });
            }
            EndLook::Form { target } => {
                ui.horizontal(|ui| {
                    ui.label("Shape of");
                    if let Some(id) = pick_layer(ui, "form-target", Some(*target)) {
                        *target = id;
                    }
                });
                ui.weak("They fly into its outline and stay there.");
            }
            _ => {}
        }
        ui.horizontal(|ui| {
            ui.label("Variation").on_hover_text("A different random pattern");
            let mut seed = b.seed as f64;
            begin |= ui.add(egui::DragValue::new(&mut seed).range(0.0..=1e9).speed(0.2)).drag_started();
            b.seed = seed.round() as u64;
        });
    });
    if begin {
        app.new_gesture();
    }
    if b != e
        && let Ok(cmd) = xenify_ops::particles::edit(app.project(), layer.id, |x| *x = b)
    {
        app.exec_merged("Change particles", cmd);
    }
}
