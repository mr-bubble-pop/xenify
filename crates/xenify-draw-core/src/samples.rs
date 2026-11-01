//! Examples to start from (the New canvas screen).

use kurbo::Vec2;
use xenify_doc::{Color, CompSettings, Easing, Keyframe, Layer, LayerKind, Project, Property, ShapeItem, TextLayer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Poster,
    Bounce,
    StudioPitch,
}

fn hex(s: &str) -> Color {
    Color::from_hex(s).unwrap_or(Color::BLACK)
}

fn keys<T: xenify_doc::Animatable>(k: &[(f64, T, Easing)]) -> Property<T> {
    Property::from_keyframes(
        k.iter().map(|(time, value, easing)| Keyframe { time: *time, value: value.clone(), easing: *easing }).collect(),
    )
    .unwrap_or(Property::Static(k[0].1.clone()))
}

impl Sample {
    pub const ALL: [Sample; 3] = [Sample::Poster, Sample::Bounce, Sample::StudioPitch];

    pub fn name(self) -> &'static str {
        match self {
            Sample::Poster => "poster",
            Sample::Bounce => "bounce",
            Sample::StudioPitch => "studio-pitch",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Sample::Poster => "Event poster",
            Sample::Bounce => "Bouncing shapes",
            Sample::StudioPitch => "Riverside Studio pitch",
        }
    }

    /// A moment when everything in the example is on screen.
    pub fn showcase_time(self) -> f64 {
        match self {
            Sample::Poster => 3.0,
            Sample::Bounce => 1.5,
            Sample::StudioPitch => 2.5,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn project(self) -> Project {
        match self {
            Sample::Poster => poster(),
            Sample::Bounce => xenify_doc::demo_project(),
            Sample::StudioPitch => studio_pitch(),
        }
    }
}

/// 1080×1350, 6 s: an autumn bake sale poster whose words slide in and cherry drops on the cake.
fn poster() -> Project {
    let mut p = Project::new();
    p.compositions[0].settings = CompSettings {
        name: "Autumn bake sale".into(),
        width: 1080,
        height: 1350,
        fps: 30.0,
        duration: 6.0,
        background: hex("#FBEBD9"),
        ..Default::default()
    };
    let mut layers = Vec::new();
    let ink = hex("#431407");
    let rust = hex("#7C2D12");

    let mut id = || p.alloc_id();
    // Plate, sponge and icing.
    let plate = Layer::shape(id(), "Plate", vec![ShapeItem::ellipse(Vec2::new(660.0, 84.0), hex("#E7C9A9"))])
        .with_position(Vec2::new(540.0, 1116.0));
    let sponge = Layer::shape(id(), "Sponge", vec![ShapeItem::rect(Vec2::new(504.0, 210.0), hex("#B45309"))])
        .with_position(Vec2::new(540.0, 1005.0));
    let icing = Layer::shape(id(), "Icing", vec![ShapeItem::rect(Vec2::new(504.0, 66.0), hex("#FFF7ED"))])
        .with_position(Vec2::new(540.0, 933.0));
    let mut cherry = Layer::shape(id(), "Cherry", vec![ShapeItem::ellipse(Vec2::new(54.0, 54.0), hex("#B91C1C"))])
        .with_position(Vec2::new(540.0, 876.0));
    cherry.transform.position = keys(&[
        (1.6, Vec2::new(540.0, 600.0), Easing::EASE_IN),
        (2.1, Vec2::new(540.0, 876.0), Easing::EASE_OUT),
        (2.3, Vec2::new(540.0, 850.0), Easing::EASE_IN),
        (2.5, Vec2::new(540.0, 876.0), Easing::Linear),
    ]);
    cherry.opacity = keys(&[(1.55, 0.0, Easing::Linear), (1.6, 1.0, Easing::Linear)]);
    layers.extend([plate, sponge, icing, cherry]);

    // Words.
    let text = |id, name: &str, s: &str, size: f64, color: Color, at: Vec2| {
        Layer::new(id, name, LayerKind::Text(TextLayer::new(s, xenify_render::DEFAULT_FAMILY, size, color).centered()))
            .with_position(at)
    };
    let mut heading = text(id(), "Heading", "Autumn\nBake Sale", 138.0, ink, Vec2::new(540.0, 430.0));
    heading.transform.position =
        keys(&[(0.0, Vec2::new(540.0, 300.0), Easing::EASE_OUT), (0.8, Vec2::new(540.0, 430.0), Easing::Linear)]);
    heading.opacity = keys(&[(0.0, 0.0, Easing::Linear), (0.6, 1.0, Easing::Linear)]);
    let mut date = text(id(), "Date and place", "Saturday 2 November · 10am", 50.0, rust, Vec2::new(540.0, 660.0));
    date.opacity = keys(&[(1.2, 0.0, Easing::Linear), (1.7, 1.0, Easing::Linear)]);
    let mut invite = text(id(), "Invite line", "Bring your friends!", 44.0, rust, Vec2::new(540.0, 740.0));
    invite.opacity = keys(&[(2.0, 0.0, Easing::Linear), (2.5, 1.0, Easing::Linear)]);
    layers.extend([heading, date, invite]);

    p.compositions[0].layers = layers;
    p
}

// ----- Riverside Studio: a five-slide pitch for a made-up design studio -----

const PAPER: &str = "#F6F3EF";
const INK: &str = "#1C1917";
const ACCENT: &str = "#C2410C";
const STONE: &str = "#57534E";
const SAND: &str = "#DCCFC2";
const FONT: &str = "Figtree";

/// Text in Figtree: left-aligned on its baseline at `at`, or centred on it.
fn words(id: xenify_doc::Id, name: &str, s: &str, size: f64, color: &str, at: Vec2, centred: bool) -> Layer {
    let mut t = TextLayer::new(s, FONT, size, hex(color));
    if centred {
        t = t.centered();
    }
    t.line_height = 1.25;
    Layer::new(id, name, LayerKind::Text(t)).with_position(at)
}

/// Rises `by` pixels into place and fades in, from `t0` for `len` seconds.
fn rise(mut l: Layer, t0: f64, len: f64, by: f64) -> Layer {
    let at = l.transform.position.value_at(0.0);
    l.transform.position = keys(&[(t0, at + Vec2::new(0.0, by), Easing::EASE_OUT), (t0 + len, at, Easing::Linear)]);
    l.opacity = keys(&[(t0, 0.0, Easing::EASE_OUT), (t0 + len, 1.0, Easing::Linear)]);
    l
}

/// Letters coming in word by word from `t0`.
fn by_word(mut l: Layer, t0: f64, stagger: f64) -> Layer {
    if let LayerKind::Text(t) = &mut l.kind
        && let Some(mut a) = xenify_doc::TextAnimator::preset("Fade Up by Word")
    {
        a.start = t0;
        a.stagger = stagger;
        t.animators.push(a);
    }
    l
}

fn rounded(size: Vec2, fill: &str, radius: f64) -> ShapeItem {
    let mut r = ShapeItem::rect(size, hex(fill));
    if let xenify_doc::Geometry::Rect { corner_radius, .. } = &mut r.geometry {
        *corner_radius = radius.into();
    }
    r
}

/// A gentle river line across the slide at height `y` (rising and falling by
/// `amp`, shifted left by `phase`, `width` thick), drawn on from `t0`.
fn wave(id: xenify_doc::Id, name: &str, (y, amp, phase, width): (f64, f64, f64, f64), color: &str, t0: f64) -> Layer {
    let mut path = kurbo::BezPath::new();
    let step = 240.0;
    let x0 = -120.0 - phase;
    path.move_to((x0, y));
    let mut x = x0;
    let mut up = true;
    while x < 2040.0 {
        let dy = if up { -amp } else { amp };
        path.curve_to((x + step * 0.36, y + dy), (x + step * 0.64, y + dy), (x + step, y));
        x += step;
        up = !up;
    }
    let mut line = ShapeItem::path(path, None, Some((hex(color), width)));
    if let Some(s) = &mut line.stroke {
        s.cap = xenify_doc::LineCap::Round;
    }
    line.trim = Some(xenify_doc::Trim {
        end: keys(&[(t0, 0.0, Easing::EASE_IN_OUT), (t0 + 1.6, 1.0, Easing::Linear)]),
        ..Default::default()
    });
    Layer::shape(id, name, vec![line])
}

fn slide_settings(name: &str, duration: f64, background: &str) -> CompSettings {
    CompSettings {
        name: name.into(),
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration,
        background: hex(background),
        ..Default::default()
    }
}

/// The three river lines along the bottom of the first and last slides, drawn on from `t0`.
fn river(p: &mut Project, t0: f64) -> [Layer; 3] {
    [
        wave(p.alloc_id(), "River (far)", (900.0, 22.0, 60.0, 5.0), SAND, t0),
        wave(p.alloc_id(), "River (middle)", (950.0, 26.0, 150.0, 6.0), "#E8A27F", t0 + 0.2),
        wave(p.alloc_id(), "River (near)", (1000.0, 30.0, 0.0, 7.0), ACCENT, t0 + 0.4),
    ]
}

/// 1920×1080, five slides joined by Push, Wipe, Dip to colour and Iris;
/// on each, the words and shapes come in and stop, waiting for the presenter.
pub(crate) fn studio_pitch() -> Project {
    use xenify_doc::{Composition, Slide, Transition, TransitionKind};
    let mut p = Project::new();
    let mut slides = Vec::new();
    let mut add = |p: &mut Project, settings: CompSettings, layers: Vec<Layer>, tin: Transition, notes: &str| {
        let id = if p.compositions[0].layers.is_empty() && slides.is_empty() {
            p.compositions[0].settings = settings;
            p.compositions[0].layers = layers;
            p.compositions[0].id
        } else {
            let id = p.alloc_id();
            p.compositions.push(Composition { id, settings, layers });
            id
        };
        let mut s = Slide::new(id);
        s.transition_in = tin;
        s.notes = notes.into();
        slides.push(s);
    };
    let left = 200.0;

    // 1. Title.
    let mut l = river(&mut p, 0.2).to_vec();
    let mut kicker =
        words(p.alloc_id(), "Kicker", "DESIGN STUDIO  ·  SINCE 2014", 30.0, ACCENT, Vec2::new(left, 330.0), false);
    if let LayerKind::Text(t) = &mut kicker.kind {
        t.letter_spacing = 4.0;
    }
    l.push(rise(kicker, 0.1, 0.6, 16.0));
    l.push(by_word(
        words(p.alloc_id(), "Title", "Riverside Studio", 168.0, INK, Vec2::new(left, 510.0), false),
        0.3,
        0.18,
    ));
    l.push(rise(
        words(
            p.alloc_id(),
            "Tagline",
            "Brand, print and motion design for people who make things.",
            46.0,
            STONE,
            Vec2::new(left, 610.0),
            false,
        ),
        1.0,
        0.7,
        20.0,
    ));
    add(
        &mut p,
        slide_settings("Riverside Studio", 2.6, PAPER),
        l,
        Transition::default(),
        "Hello! We're Riverside Studio, a small design studio by the river. Five slides: who we are, what we do, how we've grown, what a client says, and how to reach us.",
    );

    // 2. What we do: three cards.
    let mut l = vec![
        rise(words(p.alloc_id(), "Heading", "What we do", 96.0, INK, Vec2::new(left, 250.0), false), 0.1, 0.6, 20.0),
        rise(
            words(p.alloc_id(), "Subheading", "Three things, done well.", 42.0, STONE, Vec2::new(left, 325.0), false),
            0.3,
            0.6,
            20.0,
        ),
    ];
    let cards = [
        ("Brand identity", "Names, logos, colours\nand the rules that keep\nthem looking right."),
        ("Print & packaging", "Labels, boxes, posters\nand books, checked on\nreal paper."),
        ("Motion & film", "Animated logos, social\nclips and explainers\nthat move with care."),
    ];
    for (i, (title, text)) in cards.iter().enumerate() {
        let cx = 430.0 + 530.0 * i as f64;
        let (top, x) = (430.0, cx - 190.0);
        let t0 = 0.5 + 0.25 * i as f64;
        let icon = match i {
            0 => ShapeItem::ellipse(Vec2::new(88.0, 88.0), hex(ACCENT)),
            1 => rounded(Vec2::new(80.0, 80.0), ACCENT, 14.0),
            _ => {
                let mut tri = kurbo::BezPath::new();
                tri.move_to((-34.0, -44.0));
                tri.line_to((48.0, 0.0));
                tri.line_to((-34.0, 44.0));
                tri.close_path();
                ShapeItem::path(tri, Some(hex(ACCENT)), None)
            }
        };
        let n = i + 1;
        l.push(rise(
            Layer::shape(p.alloc_id(), format!("Card {n}"), vec![rounded(Vec2::new(460.0, 460.0), "#FFFFFF", 28.0)])
                .with_position(Vec2::new(cx, top + 230.0)),
            t0,
            0.6,
            40.0,
        ));
        l.push(rise(
            Layer::shape(p.alloc_id(), format!("Card {n} icon"), vec![icon])
                .with_position(Vec2::new(x + 44.0, top + 94.0)),
            t0,
            0.6,
            40.0,
        ));
        l.push(rise(
            words(p.alloc_id(), &format!("Card {n} title"), title, 46.0, INK, Vec2::new(x, top + 250.0), false),
            t0,
            0.6,
            40.0,
        ));
        l.push(rise(
            words(p.alloc_id(), &format!("Card {n} text"), text, 32.0, STONE, Vec2::new(x, top + 318.0), false),
            t0,
            0.6,
            40.0,
        ));
    }
    add(
        &mut p,
        slide_settings("What we do", 1.8, "#EFE9E2"),
        l,
        Transition::new(TransitionKind::Push, 0.8),
        "Brand identity is half our work. Print keeps us honest: we always proof on real paper. Motion is the newest and fastest-growing part.",
    );

    // 3. A bar chart: projects per year.
    let mut l = vec![
        rise(
            words(p.alloc_id(), "Heading", "Growing every year", 96.0, INK, Vec2::new(left, 250.0), false),
            0.1,
            0.6,
            20.0,
        ),
        rise(
            words(
                p.alloc_id(),
                "Subheading",
                "Projects delivered, 2021–2025",
                42.0,
                STONE,
                Vec2::new(left, 325.0),
                false,
            ),
            0.3,
            0.6,
            20.0,
        ),
        Layer::shape(p.alloc_id(), "Axis", vec![ShapeItem::rect(Vec2::new(1520.0, 4.0), hex(SAND))])
            .with_position(Vec2::new(960.0, 902.0)),
    ];
    let years = [("2021", 18.0), ("2022", 26.0), ("2023", 34.0), ("2024", 41.0), ("2025", 52.0)];
    for (i, (year, n)) in years.iter().enumerate() {
        let cx = 352.0 + 304.0 * i as f64;
        let h = n * 8.6;
        let last = i + 1 == years.len();
        let colour = if last { ACCENT } else { "#CDB9A6" };
        let t0 = 0.5 + 0.15 * i as f64;
        let mut bar = rounded(Vec2::new(150.0, h), colour, 10.0);
        if let xenify_doc::Geometry::Rect { position, .. } = &mut bar.geometry {
            *position = Vec2::new(0.0, -h / 2.0).into();
        }
        let mut bar = Layer::shape(p.alloc_id(), format!("Bar {year}"), vec![bar]).with_position(Vec2::new(cx, 900.0));
        bar.transform.scale =
            keys(&[(t0, Vec2::new(1.0, 0.0), Easing::EASE_OUT), (t0 + 0.7, Vec2::new(1.0, 1.0), Easing::Linear)]);
        l.push(bar);
        let value = words(
            p.alloc_id(),
            &format!("Value {year}"),
            &format!("{n}"),
            52.0,
            if last { ACCENT } else { INK },
            Vec2::new(cx, 900.0 - h - 44.0),
            true,
        );
        l.push(rise(value, t0 + 0.6, 0.4, 12.0));
        l.push(words(p.alloc_id(), &format!("Year {year}"), year, 34.0, STONE, Vec2::new(cx, 950.0), true));
    }
    let mut wipe = Transition::new(TransitionKind::Wipe, 1.0);
    wipe.softness = 0.4;
    add(
        &mut p,
        slide_settings("Growing every year", 2.4, PAPER),
        l,
        wipe,
        "From 18 projects in our first full year to 52 last year, almost all from people who were sent to us by earlier clients.",
    );

    // 4. A quote, on ink.
    // An opening quote mark drawn as two "6"s: a disc with a tail curling up and right.
    let six = |dx: f64| {
        let mut tail = kurbo::BezPath::new();
        tail.move_to((dx - 34.0, 4.0));
        tail.curve_to((dx - 38.0, -48.0), (dx - 8.0, -84.0), (dx + 30.0, -96.0));
        tail.line_to((dx + 34.0, -80.0));
        tail.curve_to((dx + 8.0, -68.0), (dx - 4.0, -48.0), (dx - 2.0, -30.0));
        tail.close_path();
        let mut disc = ShapeItem::ellipse(Vec2::new(70.0, 70.0), hex(ACCENT));
        if let xenify_doc::Geometry::Ellipse { position, .. } = &mut disc.geometry {
            *position = Vec2::new(dx, 0.0).into();
        }
        [ShapeItem::path(tail, Some(hex(ACCENT)), None), disc]
    };
    let mut mark = Layer::shape(p.alloc_id(), "Quote mark", [six(-50.0), six(50.0)].concat())
        .with_position(Vec2::new(left + 84.0, 250.0));
    mark.transform.scale = keys(&[
        (0.2, Vec2::new(0.6, 0.6), Easing::Spring { stiffness: 120.0, damping: 12.0 }),
        (1.0, Vec2::new(1.0, 1.0), Easing::Linear),
    ]);
    mark.opacity = keys(&[(0.2, 0.0, Easing::Linear), (0.45, 1.0, Easing::Linear)]);
    let mut rule = Layer::shape(p.alloc_id(), "Rule", vec![ShapeItem::rect(Vec2::new(96.0, 6.0), hex(ACCENT))])
        .with_position(Vec2::new(left, 760.0));
    if let Some(s) = rule.shapes_mut() {
        s[0].geometry = xenify_doc::Geometry::Rect {
            position: Vec2::new(48.0, 0.0).into(),
            size: Vec2::new(96.0, 6.0).into(),
            corner_radius: 3.0.into(),
        };
    }
    rule.transform.scale =
        keys(&[(1.6, Vec2::new(0.0, 1.0), Easing::EASE_OUT), (2.1, Vec2::new(1.0, 1.0), Easing::Linear)]);
    let l = vec![
        mark,
        by_word(
            words(
                p.alloc_id(),
                "Quote",
                "Riverside turned a tired logo into\na brand our customers actually\ntalk about.",
                80.0,
                PAPER,
                Vec2::new(left, 430.0),
                false,
            ),
            0.4,
            0.07,
        ),
        rule,
        rise(
            words(
                p.alloc_id(),
                "Who said it",
                "Mara Lindqvist, founder of Oak & Ember Bakery",
                38.0,
                "#A8A29E",
                Vec2::new(left, 830.0),
                false,
            ),
            1.8,
            0.6,
            12.0,
        ),
    ];
    let mut dip = Transition::new(TransitionKind::DipToColor, 1.0);
    dip.color = hex(ACCENT);
    add(
        &mut p,
        slide_settings("What clients say", 2.6, INK),
        l,
        dip,
        "Oak & Ember came to us with a logo drawn in a hurry ten years ago. Sales of their boxed cakes doubled in the year after the new look.",
    );

    // 5. Contact.
    let mut l = river(&mut p, 0.3).to_vec();
    l.push(by_word(
        words(p.alloc_id(), "Heading", "Let’s make something\ntogether.", 120.0, INK, Vec2::new(left, 300.0), false),
        0.2,
        0.12,
    ));
    let lines = [
        ("Email", "hello@riverside.example", ACCENT, 50.0),
        ("Phone", "+1 555 0142", INK, 42.0),
        ("Address", "12 Mill Lane, Riverside", STONE, 42.0),
    ];
    for (i, (name, text, colour, size)) in lines.iter().enumerate() {
        l.push(rise(
            words(p.alloc_id(), name, text, *size, colour, Vec2::new(left, 600.0 + 72.0 * i as f64), false),
            1.0 + 0.2 * i as f64,
            0.5,
            14.0,
        ));
    }
    let mut iris = Transition::new(TransitionKind::Iris, 1.0);
    iris.softness = 0.3;
    add(
        &mut p,
        slide_settings("Contact", 2.6, PAPER),
        l,
        iris,
        "Thank you! We reply to every email within a day. Questions?",
    );

    p.slides = slides;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_example_is_listed_and_opens() {
        let mut s = crate::Session::default();
        let list = s.call("samples", &json!({})).unwrap();
        let names: Vec<&str> = list.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["poster", "bounce", "studio-pitch"]);
        for n in names {
            s.call("sample", &json!({ "name": n })).unwrap();
            let again = Project::from_json(&s.project().to_json()).unwrap();
            assert_eq!(&again, s.project(), "{n} survives saving");
        }
        assert!(s.call("sample", &json!({ "name": "nope" })).is_err());
    }

    #[test]
    fn the_studio_pitch_is_five_slides_that_wait_after_their_animation() {
        let mut s = crate::Session::sample(Sample::StudioPitch);
        let slides = s.call("slides", &json!({})).unwrap();
        assert_eq!(slides["presentation"], true);
        let list = slides["list"].as_array().unwrap();
        let names: Vec<&str> = list.iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Riverside Studio", "What we do", "Growing every year", "What clients say", "Contact"]);
        // Each slide after the first arrives with a transition; each has notes.
        assert!(list.iter().skip(1).all(|x| x["transition_in"]["kind"] != "cut"));
        assert!(list.iter().all(|x| !x["notes"].as_str().unwrap().is_empty()));
        // Presenting waits once per slide, at its end.
        let deck = s.call("deck", &json!({})).unwrap();
        assert_eq!(deck["stops"].as_array().unwrap().len(), 5);
        // By then everything has come in: no layer is still see-through or squashed.
        let p = s.project().clone();
        for slide in &p.slides {
            let c = p.comp(slide.comp).unwrap();
            let end = c.settings.duration;
            for l in &c.layers {
                assert!(
                    (l.opacity.value_at(end) - 1.0).abs() < 1e-9,
                    "{}: {} is faded at the end",
                    c.settings.name,
                    l.meta.name
                );
                assert_eq!(
                    l.transform.scale.value_at(end),
                    Vec2::new(1.0, 1.0),
                    "{}: {}",
                    c.settings.name,
                    l.meta.name
                );
                if let LayerKind::Text(t) = &l.kind {
                    let units = |a: &xenify_doc::TextAnimator| match a.unit {
                        xenify_doc::TextUnit::Word => t.text.split_whitespace().count(),
                        xenify_doc::TextUnit::Line => t.text.lines().count(),
                        _ => t.text.chars().count(),
                    };
                    assert!(
                        t.animators.iter().all(|a| a.end(units(a)) <= end),
                        "{}: {} still animating",
                        c.settings.name,
                        l.meta.name
                    );
                    assert_eq!(t.font_family, "Figtree");
                }
            }
        }
        // Opened on the first slide, with its title on screen.
        let st = s.call("state", &json!({})).unwrap();
        assert_eq!(st["comp"]["name"], "Riverside Studio");
        assert!(Sample::StudioPitch.showcase_time() <= p.compositions[0].settings.duration);
    }
}
