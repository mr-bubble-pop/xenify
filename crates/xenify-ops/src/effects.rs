//! Every ready-made effect in one catalogue (particles, fire and smoke,
//! explosions, shatter; water stays addable for old canvases' sake), and one way to add any of them: the effect
//! gallery, the menus and the AI all go through [`add_effect`].

use crate::blast::BlastKind;
use crate::{Command, OpError, invalid};
use kurbo::Rect;
use xenify_doc::{
    Color, Easing, Id, Keyframe, Layer, LayerKind, Modifier, ModifierKind, Project, Property, ShapeItem, Vec2,
};

/// The gallery's sections and what's in them.
pub const CATALOG: &[(&str, &[&str])] = &[
    ("Celebrate", &["Confetti Burst", "Confetti Rain", "Fireworks", "Sparkle"]),
    ("Weather", &["Snow", "Rain", "Bubbles", "Fog"]),
    ("Fire", &["Campfire", "Torch", "Wall of Fire", "Burning Shape", "Fire Trail"]),
    ("Smoke", &["Smoke Puffs", "Smoke Trail", "Rising Column"]),
    ("Explosions", &["Explosion", "Big Blast"]),
    ("Break apart", &["Shatter Out", "Shatter In", "Assemble into Shape"]),
];

/// Every effect's name, in gallery order.
pub fn names() -> impl Iterator<Item = &'static str> {
    CATALOG.iter().flat_map(|(_, names)| names.iter().copied())
}

/// Whether effect `name` works on another layer (the selection).
pub fn needs_target(name: &str) -> bool {
    matches!(name, "Burning Shape" | "Water Fill Layer" | "Shatter Out" | "Shatter In" | "Assemble into Shape")
}

pub fn tip(name: &str) -> &'static str {
    match name {
        "Confetti Burst" => "A burst of paper confetti",
        "Confetti Rain" => "Confetti drifting down across the frame",
        "Fireworks" => "Bursts of sparks, one after another",
        "Sparkle" => "Twinkling stars",
        "Snow" => "Snow falling (already falling at the start)",
        "Rain" => "Rain streaks",
        "Bubbles" => "Bubbles rising and wobbling",
        "Fog" => "Slow drifting mist",
        "Campfire" => "Flames with smoke, glowing",
        "Torch" => "A small flame",
        "Wall of Fire" => "Flames all along the bottom",
        "Burning Shape" => "Flames coming off the selected layer's outline",
        "Fire Trail" => "Move the layer: it leaves flames behind",
        "Smoke Puffs" => "A puff of smoke",
        "Smoke Trail" => "Move the layer: it leaves smoke behind",
        "Rising Column" => "A column of smoke",
        "Explosion" => "Fire, sparks, smoke and a shockwave ring, with camera shake",
        "Big Blast" => "A mushroom cloud: flash, shockwave, fireball, stem, cap, dust, debris and camera shake",
        "Water Fill Screen" => "Water rises until it fills the canvas",
        "Water Fill Layer" => "Water rises inside the selected layer's shape",
        "Shatter Out" => "The selected layer breaks into pieces that fly apart",
        "Shatter In" => "Pieces fly together into the selected layer",
        "Assemble into Shape" => "Particles fly in and form the selected layer's outline",
        _ => "",
    }
}

/// A Shatter modifier for a layer whose box (its own space) is `bounds`:
/// breaking from `time` (out), or coming together by `time` (in).
pub fn shatter(bounds: Rect, time: f64, out: bool) -> ModifierKind {
    let key = |t: f64, v: f64| Keyframe { time: t, value: v, easing: Easing::Linear };
    let flight = 2.0;
    let keys = if out {
        vec![key(time, 0.0), key(time + flight, flight)]
    } else {
        vec![key((time - flight).max(0.0), flight), key(time.max(flight), 0.0)]
    };
    let size = bounds.width().max(bounds.height()).max(50.0);
    ModifierKind::Shatter {
        count: 24.0.into(),
        origin: bounds.center().to_vec2().into(),
        force: (size * 0.8).into(),
        amount: 0.0.into(),
        rotation: (-90.0).into(),
        gravity: (size * 1.2).into(),
        spin: 180.0.into(),
        progress: Property::from_keyframes(keys).unwrap_or_else(|| 0.0.into()),
        seed: 1.0.into(),
    }
}

/// Where and on what to add an effect.
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
    /// Composition point (default: the preset's own spot).
    pub at: Option<Vec2>,
    /// When it starts.
    pub time: f64,
    /// The layer it works on, and its box: composition space (for water)
    /// and its own space (for shatter).
    pub target: Option<Id>,
    pub target_box: Option<Rect>,
    pub target_local_box: Option<Rect>,
}

/// The command that adds effect `name` (new layers take ids from `first`
/// up), and the layer to select afterwards.
pub fn add_effect(project: &Project, comp: Id, first: Id, name: &str, p: Placement) -> Result<(Command, Id), OpError> {
    let need = || p.target.ok_or_else(|| invalid(format!("{name} works on a layer: select one first")));
    let adjust = |cmd: Command| -> Command {
        match cmd {
            Command::AddLayer { comp, index, mut layer } => {
                if let Some(at) = p.at {
                    layer.transform.position = at.into();
                }
                if let LayerKind::Particles(e) = &mut layer.kind {
                    e.start = p.time;
                }
                Command::AddLayer { comp, index, layer }
            }
            other => other,
        }
    };
    Ok(match name {
        "Explosion" | "Big Blast" => {
            let kind = if name == "Explosion" { BlastKind::Explosion } else { BlastKind::BigBlast };
            let c = project.comp(comp)?;
            let (w, h) = (c.settings.width as f64, c.settings.height as f64);
            let at = p.at.unwrap_or(match kind {
                BlastKind::BigBlast => Vec2::new(w / 2.0, h * 0.85),
                BlastKind::Explosion => Vec2::new(w / 2.0, h / 2.0),
            });
            crate::blast::add_blast(project, comp, first, kind, at, p.time, (w.min(h) / 1080.0).max(0.1))?
        }
        "Water Fill Screen" => (crate::liquid::fill_screen(project, comp, first, p.time)?, first),
        "Water Fill Layer" => {
            let target = need()?;
            let b = p.target_box.ok_or_else(|| invalid("that layer isn't visible, so there's nothing to fill"))?;
            (crate::liquid::fill_layer(project, comp, first, target, b, p.time)?, first)
        }
        "Shatter Out" | "Shatter In" => {
            let target = need()?;
            let b =
                p.target_local_box.ok_or_else(|| invalid("that layer isn't visible, so there's nothing to break"))?;
            let mut meta = project.layer(target)?.meta.clone();
            meta.modifiers.push(Modifier::new(shatter(b, p.time, name == "Shatter Out")));
            (Command::SetLayerMeta { layer: target, meta }, target)
        }
        _ => {
            let target = if matches!(name, "Burning Shape" | "Assemble into Shape") { Some(need()?) } else { None };
            let cmd = crate::particles::add_preset(project, comp, first, name, target)?;
            (adjust(cmd), first)
        }
    })
}

/// A small scene showing effect `name`: a 480×270 canvas, with a title or a
/// star to act on where the effect needs a layer. Returns the project and
/// its canvas.
pub fn preview(name: &str) -> Option<(Project, Id)> {
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    {
        let c = p.comp_mut(comp).ok()?;
        c.settings.width = 480;
        c.settings.height = 270;
        c.settings.duration = 4.0;
        c.settings.background = Color::rgb(0.07, 0.08, 0.11);
    }
    let mut target = None;
    let mut target_box = None;
    let mut centre = Vec2::ZERO;
    if needs_target(name) {
        centre = Vec2::new(240.0, 140.0);
        let (layer, b) = if name == "Water Fill Layer" {
            let text = xenify_doc::TextLayer::new("FX", "Inter", 150.0, Color::WHITE).centered();
            let l = Layer::new(Id(1), "Title", LayerKind::Text(text)).with_position(centre);
            // About the box Inter draws "FX" in at 150 px.
            (l, Rect::new(175.0, 85.0, 305.0, 195.0))
        } else {
            let params = crate::draw::FigureParams { star_inner: 0.45, ..Default::default() };
            let star =
                crate::draw::figure_path(crate::draw::Figure::Star, &params, Rect::new(-70.0, -70.0, 70.0, 70.0));
            let shape = ShapeItem::path(star, Some(Color::rgb(1.0, 0.8, 0.25)), None);
            let l = Layer::shape(Id(1), "Star", vec![shape]).with_position(centre);
            (l, Rect::new(170.0, 70.0, 310.0, 210.0))
        };
        p.comp_mut(comp).ok()?.layers.push(layer);
        p.reserve_id(Id(1));
        target = Some(Id(1));
        target_box = Some(b);
    }
    let placement =
        Placement { at: None, time: 0.3, target, target_box, target_local_box: target_box.map(|b| b - centre) };
    let (cmd, _) = add_effect(&p, comp, Id(p.next_id), name, placement).ok()?;
    crate::apply(&mut p, cmd).ok()?;
    Some((p, comp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_adds_and_previews() {
        for name in names() {
            let (p, comp) = preview(name).unwrap_or_else(|| panic!("{name} previews"));
            let drawn =
                (0..8).any(|i| !xenify_anim::evaluate_project(&p, comp, 0.3 + i as f64 * 0.4).layers.is_empty());
            assert!(drawn, "{name} draws something");
            assert!(!tip(name).is_empty(), "{name} has a tip");
        }
    }

    #[test]
    fn effects_on_a_layer_need_one() {
        let p = Project::new();
        let comp = p.compositions[0].id;
        for name in names().filter(|n| needs_target(n)) {
            assert!(add_effect(&p, comp, Id(p.next_id), name, Placement::default()).is_err(), "{name}");
        }
    }
}
