//! A presentation as one timeline: each slide plays, then hands over to the
//! next with their transitions (the outgoing slide holds its last frame, the
//! incoming one waits on its first). Presenting stops at the slide's pause
//! markers and at its end; exporting plays straight through.

use xenify_doc::{Id, Project, Slide, TransitionKind};

pub use crate::transition::{Look, Piece};

/// One slide as drawn: which composition, at what time, looking how.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub slide: usize,
    pub comp: Id,
    pub time: f64,
    pub look: Look,
}

/// What's on screen: a slide, and during a transition the next one over it.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckFrame {
    pub under: Shot,
    pub over: Option<Shot>,
    /// Draw `under` on top of `over` (the old slide moves or breaks away).
    pub swap: bool,
    /// Smart Morph this far (0…1) from `under` to `over`: draw
    /// [`crate::transition::morph_scene`] instead of the two slides.
    pub morph: Option<f64>,
}

#[derive(Clone, Debug)]
struct Part {
    slide: Slide,
    /// Deck time the slide starts playing.
    start: f64,
    duration: f64,
    /// Handover to the next slide (after `start + duration`).
    handover: f64,
    pauses: Vec<f64>,
    /// Width per unit of height.
    aspect: f64,
}

#[derive(Clone, Debug)]
pub struct Deck {
    parts: Vec<Part>,
}

impl Deck {
    /// The project's slides (or its main composition alone).
    pub fn new(project: &Project) -> Self {
        let slides: Vec<Slide> = if project.slides.is_empty() {
            project.main_comp().map(|c| vec![Slide::new(c.id)]).unwrap_or_default()
        } else {
            project.slides.clone()
        };
        let mut parts = Vec::new();
        let mut t = 0.0;
        for (i, slide) in slides.iter().enumerate() {
            let Ok(c) = project.comp(slide.comp) else { continue };
            let next = slides.get(i + 1);
            let handover = next.map_or(0.0, |n| slide.transition_out.length().max(n.transition_in.length()));
            let mut pauses: Vec<f64> =
                c.settings.pauses.iter().copied().filter(|p| *p > 0.0 && *p < c.settings.duration).collect();
            pauses.sort_by(f64::total_cmp);
            parts.push(Part {
                slide: slide.clone(),
                start: t,
                duration: c.settings.duration.max(0.0),
                handover,
                pauses,
                aspect: c.settings.width.max(1) as f64 / c.settings.height.max(1) as f64,
            });
            t += c.settings.duration.max(0.0) + handover;
        }
        Self { parts }
    }

    pub fn len(&self) -> usize {
        self.parts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Total length, playing straight through.
    pub fn duration(&self) -> f64 {
        self.parts.last().map_or(0.0, |p| p.start + p.duration)
    }

    /// When slide `i` starts.
    pub fn slide_start(&self, i: usize) -> f64 {
        self.parts.get(i).map_or(0.0, |p| p.start)
    }

    /// When slide `i` has finished playing (before its handover).
    pub fn slide_end(&self, i: usize) -> f64 {
        self.parts.get(i).map_or(0.0, |p| p.start + p.duration)
    }

    pub fn slide_comp(&self, i: usize) -> Option<Id> {
        self.parts.get(i).map(|p| p.slide.comp)
    }

    pub fn notes(&self, i: usize) -> &str {
        self.parts.get(i).map_or("", |p| p.slide.notes.as_str())
    }

    /// The slide showing at `t` (the incoming one once a handover starts).
    pub fn slide_at(&self, t: f64) -> usize {
        self.parts.iter().rposition(|p| t >= p.start - 1e-9).unwrap_or(0)
    }

    /// Where presenting waits for the presenter, in deck time: each pause
    /// marker, else the end of the slide. A slide with pauses doesn't wait
    /// again at its end: after its last pause it plays out and carries on
    /// into the next slide. The last slide always waits at its end.
    pub fn stops(&self) -> Vec<f64> {
        let last = self.parts.len().saturating_sub(1);
        let mut v: Vec<f64> = self
            .parts
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                let pauses = p.pauses.iter().filter(|x| **x < p.duration - 1e-9).map(move |x| p.start + x);
                let has_pauses = p.pauses.iter().any(|x| *x < p.duration - 1e-9);
                let end = (!has_pauses || i == last).then_some(p.start + p.duration);
                pauses.chain(end)
            })
            .collect();
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        v
    }

    /// The first stop after `t`.
    pub fn next_stop(&self, t: f64) -> Option<f64> {
        self.stops().into_iter().find(|s| *s > t + 1e-9)
    }

    /// Slide `i` alone, `local` seconds in (no transition).
    pub fn plain(&self, i: usize, local: f64) -> Option<DeckFrame> {
        let part = self.parts.get(i)?;
        let shot = Shot { slide: i, comp: part.slide.comp, time: local.clamp(0.0, part.duration), look: Look::PLAIN };
        Some(DeckFrame { under: shot, over: None, swap: false, morph: None })
    }

    /// What to draw at deck time `t` (clamped to the deck).
    pub fn at(&self, t: f64) -> Option<DeckFrame> {
        let t = t.clamp(0.0, self.duration());
        let i = self.slide_at(t);
        let part = self.parts.get(i)?;
        let local = t - part.start;
        // In the handover after the slide before this one?
        let shot = |i: usize, time: f64, look: Look| Shot { slide: i, comp: self.parts[i].slide.comp, time, look };
        if local <= part.duration || i + 1 >= self.parts.len() {
            let under = shot(i, local.min(part.duration), Look::PLAIN);
            return Some(DeckFrame { under, over: None, swap: false, morph: None });
        }
        let elapsed = local - part.duration;
        let next = &self.parts[i + 1];
        // The incoming slide's transition wins; else how the outgoing one leaves.
        let tr = if next.slide.transition_in.kind != TransitionKind::Cut {
            next.slide.transition_in
        } else {
            part.slide.transition_out
        };
        let p = if tr.kind == TransitionKind::Cut {
            // A cut changes over at the end of the handover (the other side may be longer).
            if elapsed < part.handover { 0.0 } else { 1.0 }
        } else if tr.length() <= 0.0 {
            1.0
        } else {
            tr.easing.apply((elapsed / tr.length()).clamp(0.0, 1.0))
        };
        let (out, inn, swap) = crate::transition::looks(&tr, p, part.aspect);
        let morph = (tr.kind == TransitionKind::SmartMorph).then_some(p);
        Some(DeckFrame { under: shot(i, part.duration, out), over: Some(shot(i + 1, 0.0, inn)), swap, morph })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{CompSettings, Composition, Transition};

    /// Two 2 s slides; the second fades in over 1 s; the first has a pause at 0.5 s.
    fn deck() -> (Project, Id, Id) {
        let mut p = Project::new();
        let a = p.compositions[0].id;
        p.compositions[0].settings.duration = 2.0;
        p.compositions[0].settings.pauses = vec![0.5];
        let b = p.alloc_id();
        p.compositions.push(Composition {
            id: b,
            settings: CompSettings { duration: 2.0, ..Default::default() },
            layers: vec![],
        });
        let mut second = Slide::new(b);
        second.transition_in = Transition::new(TransitionKind::Fade, 1.0);
        p.slides = vec![Slide::new(a), second];
        (p, a, b)
    }

    #[test]
    fn slides_play_one_after_another_with_a_handover() {
        let (p, a, b) = deck();
        let d = Deck::new(&p);
        assert_eq!(d.duration(), 5.0, "2 + 1 handover + 2");
        assert_eq!(d.slide_start(1), 3.0);
        let f = d.at(1.0).unwrap();
        assert_eq!((f.under.comp, f.under.time, f.over.clone()), (a, 1.0, None));
        // Half way through the fade: A holds its last frame, B is half in.
        let f = d.at(2.5).unwrap();
        assert_eq!((f.under.comp, f.under.time), (a, 2.0));
        let over = f.over.unwrap();
        assert_eq!((over.comp, over.time), (b, 0.0));
        assert!((over.look.opacity - 0.5).abs() < 1e-9);
        assert_eq!(f.under.look, Look::PLAIN, "a cut out stays until covered");
        let f = d.at(4.0).unwrap();
        assert_eq!((f.under.comp, f.under.time, f.over.clone()), (b, 1.0, None));
    }

    #[test]
    fn presenting_stops_at_pauses_and_slide_ends() {
        let (p, _, _) = deck();
        let d = Deck::new(&p);
        // Slide A (ends at 2.0) has a pause at 0.5, so it doesn't wait again
        // at its end; the last slide always does.
        assert_eq!(d.stops(), [0.5, 5.0]);
        assert_eq!(d.next_stop(0.0), Some(0.5));
        assert_eq!(d.next_stop(0.5), Some(5.0));
        assert_eq!(d.next_stop(2.0), Some(5.0));
        assert_eq!(d.next_stop(5.0), None);
        assert_eq!(d.slide_at(2.9), 0);
        assert_eq!(d.slide_at(3.0), 1);
    }

    #[test]
    fn pushes_move_both_slides() {
        let (mut p, _, _) = deck();
        p.slides[1].transition_in =
            Transition { easing: xenify_doc::Easing::Linear, ..Transition::new(TransitionKind::Push, 1.0) };
        let f = Deck::new(&p).at(2.5).unwrap();
        assert!((f.under.look.offset().x + 0.5).abs() < 1e-9);
        assert!((f.over.unwrap().look.offset().x - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_plain_project_is_a_one_slide_deck() {
        let p = Project::new();
        let d = Deck::new(&p);
        assert_eq!(d.len(), 1);
        assert_eq!(d.duration(), p.compositions[0].settings.duration);
    }
}
