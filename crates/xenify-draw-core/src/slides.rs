//! Slides: a presentation is a list of canvases with transitions between
//! them, speaker notes and pause points. Presenting plays the deck as one
//! timeline (`Deck`), stopping at each slide's pauses and end.

use serde_json::{Value as Json, json};
use xenify_anim::deck::Deck;
use xenify_doc::Transition;

use crate::{Session, err, f};

fn index_arg(args: &Json) -> Result<usize, String> {
    f(args, "index").map(|i| i as usize).ok_or_else(|| "missing index".into())
}

impl Session {
    pub(crate) fn slides_call(&mut self, name: &str, args: &Json) -> Option<Result<Json, String>> {
        Some(match name {
            "slides" => Ok(self.slides()),
            "add_slide" => {
                let after = f(args, "after").map(|i| i as usize);
                xenify_ops::slides::add_slide(self.project(), after)
                    .map_err(err)
                    .and_then(|(cmd, id)| self.exec("Add slide", cmd).and_then(|_| self.open_comp(id)).map(|_| json!(id.0)))
            }
            "duplicate_slide" => index_arg(args).and_then(|i| {
                let (cmd, id) = xenify_ops::slides::duplicate_slide(self.project(), i).map_err(err)?;
                self.exec("Duplicate slide", cmd)?;
                self.open_comp(id)?;
                Ok(json!(id.0))
            }),
            "delete_slide" => index_arg(args).and_then(|i| {
                let list = xenify_ops::slides::slides_or_main(self.project());
                if list.len() < 2 {
                    return Err("A presentation needs at least one slide.".into());
                }
                let cmd = xenify_ops::slides::delete_slide(self.project(), i).map_err(err)?;
                let next = list.get(if i + 1 < list.len() { i + 1 } else { i.saturating_sub(1) }).map(|s| s.comp);
                if let Some(n) = next {
                    self.open_comp(n)?;
                }
                self.exec("Delete slide", cmd).map(|_| Json::Null)
            }),
            "move_slide" => {
                let (from, to) = (index_arg(args), f(args, "to").map(|t| t as usize).ok_or_else(|| "missing to".to_string()));
                match (from, to) {
                    (Ok(a), Ok(b)) => match xenify_ops::slides::move_slide(self.project(), a, b).map_err(err) {
                        Ok(Some(cmd)) => self.exec("Move slide", cmd).map(|_| Json::Null),
                        Ok(None) => Ok(Json::Null),
                        Err(e) => Err(e),
                    },
                    (Err(e), _) | (_, Err(e)) => Err(e),
                }
            }
            "open_slide" => index_arg(args).and_then(|i| {
                let s = xenify_ops::slides::slides_or_main(self.project());
                let comp = s.get(i).ok_or("no such slide")?.comp;
                self.open_comp(comp).map(|_| self.state())
            }),
            "set_slide" => self.set_slide(args),
            "deck" => Ok(self.deck_info()),
            "transition_kinds" => Ok(json!(xenify_doc::TransitionKind::ALL.iter().map(|k| json!({
                "id": k, "label": k.label(), "direction": k.has_direction(), "color": k.has_color(), "softness": k.has_softness(),
            })).collect::<Vec<_>>())),
            "morph_pairs" => index_arg(args).map(|i| self.morph_pairs(i)),
            _ => return None,
        })
    }

    /// Each slide: index, canvas id, name, whether it's open, transitions and notes.
    fn slides(&self) -> Json {
        let p = self.project();
        json!({
            "presentation": !p.slides.is_empty(),
            "list": xenify_ops::slides::slides_or_main(p).iter().enumerate().map(|(i, s)| {
                let c = p.comp(s.comp).ok();
                json!({
                    "index": i, "comp": s.comp.0, "name": c.map(|c| c.settings.name.clone()), "open": s.comp == self.comp,
                    "width": c.map(|c| c.settings.width), "height": c.map(|c| c.settings.height), "duration": c.map(|c| c.settings.duration),
                    "transition_in": s.transition_in, "transition_out": s.transition_out, "notes": s.notes,
                })
            }).collect::<Vec<_>>(),
        })
    }

    /// A slide's transitions (as stored) and notes.
    fn set_slide(&mut self, args: &Json) -> Result<Json, String> {
        let i = index_arg(args)?;
        let tin: Option<Transition> =
            args.get("transition_in").map(|v| serde_json::from_value(v.clone())).transpose().map_err(err)?;
        let tout: Option<Transition> =
            args.get("transition_out").map(|v| serde_json::from_value(v.clone())).transpose().map_err(err)?;
        let notes = args.get("notes").and_then(Json::as_str).map(str::to_string);
        let cmd = xenify_ops::slides::edit_slide(self.project(), i, |s| {
            if let Some(t) = tin {
                s.transition_in = t;
            }
            if let Some(t) = tout {
                s.transition_out = t;
            }
            if let Some(n) = notes {
                s.notes = n;
            }
        })
        .map_err(err)?;
        self.editor.execute_merged("Slide", format!("slide:{i}"), cmd).map_err(err)?;
        Ok(Json::Null)
    }

    /// Smart Morph into slide `i` from the one before: the layers that glide (same name), the
    /// ones that only fade in, and the names of the earlier slide's unpaired layers (to pair by renaming).
    fn morph_pairs(&self, i: usize) -> Json {
        let p = self.project();
        let (Some(prev), Some(this)) = (i.checked_sub(1).and_then(|k| p.slides.get(k)), p.slides.get(i)) else {
            return Json::Null;
        };
        let pairs = xenify_anim::transition::morph_pairs(p, prev.comp, this.comp);
        let (Ok(ca), Ok(cb)) = (p.comp(prev.comp), p.comp(this.comp)) else { return Json::Null };
        let (pa, pb): (Vec<_>, Vec<_>) = pairs.iter().map(|(a, b, _)| (*a, *b)).unzip();
        json!({
            "pairs": pairs.iter().map(|(_, _, n)| n).collect::<Vec<_>>(),
            "lone_in": cb.layers.iter().filter(|l| l.meta.parent.is_none() && !pb.contains(&l.id)).map(|l| json!({ "id": l.id.0, "name": l.meta.name })).collect::<Vec<_>>(),
            "lone_prev": ca.layers.iter().filter(|l| l.meta.parent.is_none() && !pa.contains(&l.id)).map(|l| l.meta.name.clone()).collect::<Vec<_>>(),
        })
    }

    /// The presentation as one timeline: its length, where each slide starts, and where presenting stops.
    fn deck_info(&self) -> Json {
        let d = Deck::new(self.project());
        json!({
            "duration": d.duration(),
            "starts": (0..d.len()).map(|i| d.slide_start(i)).collect::<Vec<_>>(),
            "stops": d.stops(),
            "notes": (0..d.len()).map(|i| d.notes(i)).collect::<Vec<_>>(),
            "ends": (0..d.len()).map(|i| d.slide_end(i)).collect::<Vec<_>>(),
            "keep_playing": self.project().soundtrack.keep_playing,
        })
    }

    /// Draws the presentation at deck time `t` into a `width`×`height` target (letterboxed in black).
    pub fn paint_deck(&mut self, out: &mut impl xenify_render::Painter, t: f64, width: u32, height: u32) {
        let d = Deck::new(self.project());
        if let Some(frame) = d.at(t) {
            let p = self.project().clone();
            xenify_render::encode_deck(&p, &frame, out, width, height, &mut self.text);
        }
    }

    /// Draws canvas `comp` at `time`, mapped by `view` (slide thumbnails).
    pub fn paint_comp(
        &mut self,
        out: &mut impl xenify_render::Painter,
        comp: xenify_doc::Id,
        time: f64,
        view: kurbo::Affine,
    ) {
        let scene = xenify_anim::evaluate_project(self.project(), comp, time);
        xenify_render::encode_seen(&scene, out, view, kurbo::Affine::IDENTITY, &mut self.text);
    }
}

/// Two sample slides (blue with a title and a yellow disc, then crimson with
/// the disc moved, so Smart Morph has something to glide) joined by `kind`,
/// for the transition picker's moving previews. The transition runs from 1 s for 1 s.
pub fn transition_sample(kind: xenify_doc::TransitionKind) -> xenify_doc::Project {
    use xenify_doc::{
        Color, CompSettings, Composition, Id, Layer, LayerKind, ShapeItem, Slide, TextLayer, Transition, Vec2,
    };
    let mut p = xenify_doc::Project::new();
    let a = p.compositions[0].id;
    let settings = CompSettings { width: 320, height: 180, duration: 1.0, ..Default::default() };
    let title = |id: u64, s: &str| {
        Layer::new(Id(id), "Title", LayerKind::Text(TextLayer::new(s, "Inter", 34.0, Color::WHITE).centered()))
            .with_position(Vec2::new(160.0, 50.0))
    };
    let disc = |id: u64, x: f64| {
        Layer::shape(Id(id), "Disc", vec![ShapeItem::ellipse(Vec2::new(60.0, 60.0), Color::rgb(1.0, 0.8, 0.2))])
            .with_position(Vec2::new(x, 120.0))
    };
    p.compositions[0].settings = CompSettings { background: Color::rgb(0.12, 0.22, 0.55), ..settings.clone() };
    p.compositions[0].layers = vec![title(1, "One"), disc(2, 80.0)];
    let b = p.alloc_id();
    p.compositions.push(Composition {
        id: b,
        settings: CompSettings { background: Color::rgb(0.55, 0.1, 0.3), ..settings },
        layers: vec![title(101, "Two"), disc(102, 240.0)],
    });
    let mut second = Slide::new(b);
    second.transition_in = Transition { color: Color::WHITE, ..Transition::new(kind, 1.0) };
    p.slides = vec![Slide::new(a), second];
    p
}
