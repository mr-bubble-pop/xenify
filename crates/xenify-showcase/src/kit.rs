//! The builder's toolkit: a deck being built through ordinary editor
//! commands (each recorded, so the whole showcase can be replayed), layer
//! constructors, keyframe helpers and the house style.

use xenify_audio::synth::{MusicRecipe, Recipe, Sfx, SfxRecipe};
use xenify_doc::{
    Color, CompSettings, Easing, Fill, GradientKind, GradientPaint, GradientStop, Id, Keyframe, Layer, LayerKind,
    MotionBlur, Project, Property, ShapeItem, Slide, Stroke, TextAnimator, TextLayer, Transition, TransitionKind, Vec2,
};
use xenify_ops::Command;
use kurbo::BezPath;

pub const W: f64 = 1920.0;
pub const H: f64 = 1080.0;
pub const FPS: f64 = 60.0;

// House style: near-black, one pink → violet accent gradient.
pub const BG: Color = Color::rgb(0.027, 0.027, 0.059);
pub const INK: Color = Color::rgb(0.96, 0.96, 1.0);
pub const MUTED: Color = Color::rgb(0.6, 0.63, 0.74);
pub const PINK: Color = Color::rgb(1.0, 0.24, 0.5);
pub const VIOLET: Color = Color::rgb(0.48, 0.36, 1.0);
pub const CYAN: Color = Color::rgb(0.2, 0.85, 1.0);
pub const GOLD: Color = Color::rgb(1.0, 0.78, 0.25);
pub const PANEL: Color = Color::rgb(0.09, 0.09, 0.15);
pub const LINE: Color = Color::rgb(0.22, 0.22, 0.32);
pub const FONT: &str = "Inter";

/// Expo-out: fast, then a long settle.
pub const EXPO: Easing = Easing::Bezier { x1: 0.16, y1: 1.0, x2: 0.3, y2: 1.0 };
/// Smooth in and out.
pub const SMOOTH: Easing = Easing::Bezier { x1: 0.65, y1: 0.0, x2: 0.35, y2: 1.0 };
/// A little overshoot.
pub const BACK: Easing = Easing::Bezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 };
pub const SPRING: Easing = Easing::Spring { stiffness: 170.0, damping: 11.0 };
pub const EASE_IN: Easing = Easing::Bezier { x1: 0.7, y1: 0.0, x2: 0.84, y2: 0.0 };

/// Keyframes `(time, value)`, each easing into the next with `easing`.
pub fn keys<T: xenify_doc::Animatable>(k: &[(f64, T)], easing: Easing) -> Property<T> {
    Property::from_keyframes(k.iter().cloned().map(|(time, value)| Keyframe { time, value, easing }).collect())
        .unwrap_or_else(|| k[0].1.clone().into())
}

/// Keyframes with an easing each.
pub fn keys_e<T: xenify_doc::Animatable>(k: &[(f64, T, Easing)]) -> Property<T> {
    Property::from_keyframes(k.iter().cloned().map(|(time, value, easing)| Keyframe { time, value, easing }).collect())
        .unwrap_or_else(|| k[0].1.clone().into())
}

pub fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

/// The accent gradient across `w` (left to right, in item space).
pub fn accent_gradient(w: f64) -> GradientPaint {
    GradientPaint {
        kind: GradientKind::Linear,
        start: v(-w / 2.0, 0.0).into(),
        end: v(w / 2.0, 0.0).into(),
        stops: vec![GradientStop { offset: 0.0, color: PINK }, GradientStop { offset: 1.0, color: VIOLET }].into(),
    }
}

pub fn gradient_fill(w: f64) -> Fill {
    Fill { color: PINK.into(), gradient: Some(accent_gradient(w)), pattern: None }
}

pub fn rect(name: &str, size: Vec2, radius: f64, color: Color, at: Vec2) -> Layer {
    let mut item = ShapeItem::rect(size, color);
    if let xenify_doc::Geometry::Rect { corner_radius, .. } = &mut item.geometry {
        *corner_radius = radius.into();
    }
    item.name = name.into();
    Layer::shape(Id(0), name, vec![item]).with_position(at)
}

pub fn ellipse(name: &str, size: Vec2, color: Color, at: Vec2) -> Layer {
    let mut item = ShapeItem::ellipse(size, color);
    item.name = name.into();
    Layer::shape(Id(0), name, vec![item]).with_position(at)
}

pub fn path(name: &str, path: BezPath, fill: Option<Color>, stroke: Option<(Color, f64)>, at: Vec2) -> Layer {
    let mut item = ShapeItem::path(path, fill, stroke);
    item.name = name.into();
    if let Some(s) = &mut item.stroke {
        s.cap = xenify_doc::LineCap::Round;
        s.join = xenify_doc::LineJoin::Round;
    }
    Layer::shape(Id(0), name, vec![item]).with_position(at)
}

/// Text centred on `at`.
pub fn text(name: &str, t: &str, size: f64, color: Color, at: Vec2) -> Layer {
    Layer::new(Id(0), name, LayerKind::Text(TextLayer::new(t, FONT, size, color).centered())).with_position(at)
}

/// Text starting at `at` (left, on the baseline).
pub fn text_left(name: &str, t: &str, size: f64, color: Color, at: Vec2) -> Layer {
    Layer::new(Id(0), name, LayerKind::Text(TextLayer::new(t, FONT, size, color))).with_position(at)
}

/// A text animator preset starting at `start` (seconds after the layer's in point).
pub fn animate(l: &mut Layer, preset: &str, start: f64) {
    if let LayerKind::Text(t) = &mut l.kind
        && let Some(mut a) = TextAnimator::preset(preset)
    {
        a.start += start;
        t.animators.push(a);
    }
}

/// Shows from `t0` to `t1` (None: to the end).
pub fn live(l: &mut Layer, t0: f64, t1: Option<f64>) {
    l.meta.in_point = t0;
    l.meta.out_point = t1;
}

/// Pops in from nothing at `t` (with overshoot).
pub fn pop(l: &mut Layer, t: f64) {
    let s = l.transform.scale.value_at(0.0);
    l.transform.scale = keys(&[(t, v(0.0, 0.0)), (t + 0.5, s)], BACK);
    l.opacity = keys(&[(t, 0.0), (t + 0.15, 1.0)], Easing::Linear);
}

/// Fades in at `t` over `d`.
pub fn fade_in(l: &mut Layer, t: f64, d: f64) {
    l.opacity = keys(&[(t, 0.0), (t + d, 1.0)], SMOOTH);
}

/// Fades in at `t0` and out at `t1`.
pub fn fade_in_out(l: &mut Layer, t0: f64, t1: f64) {
    l.opacity = keys(&[(t0, 0.0), (t0 + 0.3, 1.0), (t1, 1.0), (t1 + 0.3, 0.0)], SMOOTH);
}

/// Slides in from `from` (an offset) at `t`, expo-out.
pub fn slide_in(l: &mut Layer, t: f64, from: Vec2, d: f64) {
    let p = l.transform.position.value_at(0.0);
    l.transform.position = keys(&[(t, p + from), (t + d, p)], EXPO);
    l.opacity = keys(&[(t, 0.0), (t + d * 0.4, 1.0)], Easing::Linear);
}

/// Draws the item's outline on from `t` over `d` (trim end 0 → 1).
pub fn draw_on(l: &mut Layer, t: f64, d: f64) {
    if let LayerKind::Shape { shapes } = &mut l.kind {
        for s in shapes {
            s.trim = Some(xenify_doc::Trim {
                start: 0.0.into(),
                end: keys(&[(t, 0.0), (t + d, 1.0)], SMOOTH),
                offset: 0.0.into(),
            });
        }
    }
}

pub fn stroke(color: Color, width: f64) -> Stroke {
    Stroke {
        color: color.into(),
        width: width.into(),
        cap: xenify_doc::LineCap::Round,
        join: xenify_doc::LineJoin::Round,
        ..Default::default()
    }
}

/// The logo: an abstract four-point spark, `size` across, with the accent gradient.
pub fn spark_path(size: f64) -> BezPath {
    let r = size / 2.0;
    let i = r * 0.16;
    let mut p = BezPath::new();
    p.move_to((0.0, -r));
    p.quad_to((i, -i), (r, 0.0));
    p.quad_to((i, i), (0.0, r));
    p.quad_to((-i, i), (-r, 0.0));
    p.quad_to((-i, -i), (0.0, -r));
    p.close_path();
    p
}

pub fn logo(size: f64, at: Vec2) -> Layer {
    let mut item = ShapeItem::path(spark_path(size), Some(PINK), None);
    item.name = "Spark".into();
    let mut f = gradient_fill(size);
    if let Some(g) = &mut f.gradient {
        g.start = v(-size / 2.0, -size / 2.0).into();
        g.end = v(size / 2.0, size / 2.0).into();
    }
    item.fill = Some(f);
    Layer::shape(Id(0), "Logo mark", vec![item]).with_position(at)
}

/// A deck being built: every change is an editor command, applied and kept.
pub struct Deck {
    pub p: Project,
    pub log: Vec<Command>,
    /// The slide being built.
    pub comp: Id,
    pub name: String,
    started: bool,
}

impl Deck {
    pub fn new(name: &str) -> Self {
        let p = Project::new();
        let comp = p.compositions[0].id;
        Deck { p, log: Vec::new(), comp, name: name.to_string(), started: false }
    }

    /// Applies `cmd` (it must work: the builder only makes valid edits).
    pub fn exec(&mut self, cmd: Command) {
        let _ = xenify_ops::apply(&mut self.p, cmd.clone()).unwrap_or_else(|e| panic!("showcase step failed: {e}"));
        self.log.push(cmd);
    }

    pub fn next_id(&self) -> Id {
        Id(self.p.next_id)
    }

    /// Starts a new slide: `duration` seconds, waiting at `pauses`, arriving
    /// with `transition`. Returns its composition.
    pub fn slide(&mut self, name: &str, duration: f64, pauses: &[f64], transition: Transition, notes: &str) -> Id {
        self.finish_slide();
        let comp = if self.started {
            let (cmd, id) = xenify_ops::slides::add_slide(&self.p, None).expect("a slide");
            self.exec(cmd);
            id
        } else {
            self.started = true;
            let id = self.p.compositions[0].id;
            self.exec(Command::SetSlides { slides: vec![Slide::new(id)] });
            id
        };
        self.comp = comp;
        let settings = CompSettings {
            name: name.into(),
            width: W as u32,
            height: H as u32,
            fps: FPS,
            duration,
            background: BG,
            pauses: pauses.to_vec(),
            ..Default::default()
        };
        self.exec(Command::SetCompSettings { comp, settings });
        let index = self.p.slides.iter().position(|s| s.comp == comp).expect("the slide");
        let cmd = xenify_ops::slides::edit_slide(&self.p, index, |s| {
            s.transition_in = transition;
            s.notes = notes.to_string();
        })
        .expect("slide details");
        self.exec(cmd);
        comp
    }

    /// Turns on motion blur for the current slide (for camera moves).
    pub fn motion_blur(&mut self) {
        let mut settings = self.p.comp(self.comp).unwrap().settings.clone();
        settings.motion_blur = Some(MotionBlur { shutter_angle: 180.0, samples: 6 });
        let comp = self.comp;
        self.exec(Command::SetCompSettings { comp, settings });
    }

    /// Ends the current slide at its last pause when little follows it:
    /// presenting stops at pauses and again at a slide's end, so a pause
    /// just before the end would cost the presenter an extra click.
    pub fn finish_slide(&mut self) {
        if !self.started {
            return;
        }
        let settings = self.p.comp(self.comp).expect("the slide").settings.clone();
        let Some(&last) = settings.pauses.last() else { return };
        if settings.duration - last > 1.3 {
            return;
        }
        let mut s = settings;
        s.pauses.pop();
        s.duration = last + 0.05;
        let comp = self.comp;
        self.exec(Command::SetCompSettings { comp, settings: s });
    }

    /// Adds `layer` (with a fresh id) on top of the current slide.
    pub fn add(&mut self, mut layer: Layer) -> Id {
        let id = self.next_id();
        layer.id = id;
        let comp = self.comp;
        self.exec(Command::AddLayer { comp, index: None, layer });
        id
    }

    /// Adds `layer` to composition `comp` (e.g. a symbol).
    pub fn add_to(&mut self, comp: Id, mut layer: Layer) -> Id {
        let id = self.next_id();
        layer.id = id;
        self.exec(Command::AddLayer { comp, index: None, layer });
        id
    }

    /// Replaces a layer with an edited copy.
    pub fn edit(&mut self, id: Id, f: impl FnOnce(&mut Layer)) {
        let mut l = self.p.layer(id).expect("layer").clone();
        f(&mut l);
        self.exec(Command::ReplaceLayer { layer: Box::new(l) });
    }

    /// Ends (at `t`) every layer of the current slide added since `since`.
    pub fn end_since(&mut self, since: Id, t: f64) {
        let ids: Vec<Id> =
            self.p.comp(self.comp).unwrap().layers.iter().filter(|l| l.id.0 >= since.0).map(|l| l.id).collect();
        for id in ids {
            self.edit(id, |l| {
                if l.meta.out_point.is_none_or(|o| o > t) {
                    l.meta.out_point = Some(t);
                }
            });
        }
    }

    /// Adds emoji `glyph` (the bundled Fluent set) as a group of shapes.
    pub fn emoji(&mut self, glyph: &str, at: Vec2, size: f64) -> Id {
        let e = xenify_export::emoji::find(glyph).unwrap_or_else(|| panic!("no emoji {glyph}"));
        let layers = xenify_export::emoji::layers(e, self.p.next_id, at, size).expect("emoji");
        let ids: Vec<Id> = layers.iter().map(|l| l.id).collect();
        let comp = self.comp;
        self.exec(Command::batch(
            layers.into_iter().map(|layer| Command::AddLayer { comp, index: None, layer }).collect(),
        ));
        let (cmd, group) =
            xenify_ops::group::group(&self.p, comp, &ids, &e.name, 0.0, &mut xenify_anim::ApproxText).expect("group");
        self.exec(cmd);
        group
    }

    /// Parents `child` to `parent`, keeping `child`'s coordinates as given (relative to the parent).
    pub fn parent(&mut self, child: Id, parent: Id) {
        self.edit(child, |l| l.meta.parent = Some(parent));
    }

    /// A new symbol (a composition not shown as a slide) of `w`×`h`.
    pub fn symbol(&mut self, name: &str, w: f64, h: f64, duration: f64, background: Color) -> Id {
        let id = self.next_id();
        let settings = CompSettings {
            name: name.into(),
            width: w as u32,
            height: h as u32,
            fps: FPS,
            duration,
            background,
            ..Default::default()
        };
        self.exec(Command::AddComposition {
            index: None,
            comp: xenify_doc::Composition { id, settings, layers: Vec::new() },
        });
        // Placed symbols draw their layers only, so the background is a layer.
        if background.a > 0.0 {
            let mut bg = rect("Background", v(w, h), 0.0, background, v(w / 2.0, h / 2.0));
            bg.meta.locked = true;
            self.add_to(id, bg);
        }
        id
    }

    /// Places symbol `comp` centred on `at`, `scale` times its size, from `start`.
    pub fn place(&mut self, comp: Id, name: &str, at: Vec2, scale: f64, start: f64) -> Id {
        let c = self.p.comp(comp).expect("symbol").clone();
        let mut l = Layer::precomp(Id(0), name, &c, at);
        if let LayerKind::Precomp { start: s, .. } = &mut l.kind {
            *s = start;
        }
        l.transform.scale = v(scale, scale).into();
        self.add(l)
    }

    /// A synthesized sound effect at `t` on the current slide.
    /// Each effect is made once; `pitch` (semitones) plays it faster or
    /// slower, tape-style, so repeats share one sound.
    pub fn sfx(&mut self, kind: Sfx, t: f64, pitch: f64, volume: f64) -> Id {
        let recipe = Recipe::Sfx(SfxRecipe { seed: 7, ..SfxRecipe::of(kind) });
        let id = self.sound(recipe, t);
        let speed = 2f64.powf(pitch / 12.0);
        let length = kind.length();
        self.edit(id, |l| {
            if let LayerKind::Audio(a) = &mut l.kind {
                a.speed = speed;
                a.volume = volume.clamp(0.0, 1.0).into();
            }
            l.meta.out_point = l.meta.out_point.map(|_| t + length / speed);
        });
        id
    }

    pub fn sound(&mut self, recipe: Recipe, t: f64) -> Id {
        let media = xenify_audio::synth::media_asset(&recipe, self.next_id());
        let layer = Id(self.p.next_id + 1);
        let cmd = xenify_ops::media::add_clip(&self.p, self.comp, media, layer, t).expect("a sound");
        self.exec(cmd);
        layer
    }

    /// The music bed: a seamless loop under the whole presentation from `start`.
    pub fn music(&mut self, recipe: MusicRecipe, start: f64) {
        let media = xenify_audio::synth::media_asset(&Recipe::Music(recipe), self.next_id());
        let cmd = xenify_ops::media::add_to_soundtrack(&self.p, media, start);
        self.exec(cmd);
        let cmd = xenify_ops::media::edit_soundtrack(&self.p, |s| {
            s.keep_playing = true;
            for c in &mut s.clips {
                c.audio.looped = true;
                c.audio.volume = 0.55.into();
            }
        });
        self.exec(cmd);
    }

    /// A particle preset at `at` from `t` (`target`: a layer to form or burn).
    pub fn particles(
        &mut self,
        preset: &str,
        at: Option<Vec2>,
        t: f64,
        target: Option<Id>,
        f: impl FnOnce(&mut xenify_doc::Emitter),
    ) -> Id {
        let id = self.next_id();
        let mut cmd = xenify_ops::particles::add_preset(&self.p, self.comp, id, preset, target).expect("particles");
        if let Command::AddLayer { layer, .. } = &mut cmd {
            if let Some(at) = at {
                layer.transform.position = at.into();
            }
            if let LayerKind::Particles(e) = &mut layer.kind {
                e.start = t;
                f(e);
            }
        }
        self.exec(cmd);
        id
    }

    /// An effect from the gallery; returns the layer it selects.
    pub fn effect(&mut self, name: &str, place: xenify_ops::effects::Placement) -> Id {
        let (cmd, id) =
            xenify_ops::effects::add_effect(&self.p, self.comp, self.next_id(), name, place).expect("effect");
        self.exec(cmd);
        id
    }
}

/// A transition of `kind` lasting `d` seconds.
pub fn tr(kind: TransitionKind, d: f64) -> Transition {
    Transition { color: BG, ..Transition::new(kind, d) }
}
