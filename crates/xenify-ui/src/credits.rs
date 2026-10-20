//! Help › Credits & Licences: where every bundled asset and the built-in
//! artwork came from (assets/SOURCES.toml, compiled in), and the licences of
//! the libraries the program is built with.

use xenify_export::sources::{self, Origin};

const OPEN: &str = "credits-open";

pub fn open(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(OPEN), true));
}

/// Shows the window while it's open (call every frame).
pub fn window(ctx: &egui::Context) {
    let id = egui::Id::new(OPEN);
    if !ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let (_, closed) = crate::widgets::modal(ctx, OPEN, "Credits & Licences", 720.0, |ui| {
        ui.label(
            "This program is MIT-licensed. Here is where its fonts, artwork and other assets came from, \
             and the licence each one is used under:",
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
            let all = sources::bundled();
            // Licence files are shown under the asset that uses them.
            let shown =
                all.iter().filter(|e| !all.iter().any(|o| o.license_file.is_some() && o.license_file == e.path));
            let (made, other): (Vec<_>, Vec<_>) = shown.partition(|e| e.source == Origin::Generated);
            ui.strong("Bundled and downloaded");
            for e in other {
                entry(ui, e);
            }
            ui.add_space(8.0);
            ui.strong("Made by this program");
            ui.weak("Original work, drawn or generated in code. No third-party material.");
            for e in made {
                entry(ui, e);
            }
        });
        ui.add_space(6.0);
        ui.weak("The libraries' licences are in THIRD_PARTY_LICENSES.txt, shipped with the program.");
    });
    if closed {
        ctx.data_mut(|d| d.remove::<bool>(id));
    }
}

fn entry(ui: &mut egui::Ui, e: &sources::Source) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&e.name).strong());
            ui.weak(format!("· {}", e.license));
            if let Some(v) = &e.version {
                ui.weak(format!("· {v}"));
            }
        });
        if let Some(a) = &e.attribution {
            ui.label(a);
        }
        if let Some(url) = &e.source_url {
            ui.hyperlink(url);
        }
        if let Some(g) = &e.generator {
            ui.weak(format!("Made by {g}"));
        }
        if let Some(n) = &e.notes {
            ui.weak(n);
        }
        if let Some(text) = e.license_file.as_deref().and_then(sources::licence_text) {
            egui::CollapsingHeader::new("Licence text").id_salt(("licence", &e.name)).show(ui, |ui| {
                ui.label(egui::RichText::new(text).monospace().small());
            });
        }
    });
}
