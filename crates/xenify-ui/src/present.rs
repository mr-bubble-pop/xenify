//! Presenting: the slides full screen, like PowerPoint. Each slide plays up
//! to its next pause marker (or its end) and waits; → / Space / click go on,
//! ← goes back, B blanks the screen, Esc ends. Shown full screen in this
//! window, or on another screen (a projector) while this window shows the
//! presenter view (notes, next slide, timer; see `presenter_view`).

use crate::app::XenifyApp;
use xenify_anim::deck::Deck;
use std::time::Instant;

/// Where the presentation shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// This window, full screen.
    ThisWindow,
    /// A window of its own, full screen on this monitor.
    Monitor(xenify_platform::desktop::Monitor),
}

/// Playback through a deck: pure logic, no drawing.
pub struct Playback {
    pub deck: Deck,
    /// Deck time shown.
    pub t: f64,
    /// The slide on screen. Tracked (not derived from `t`) because a slide's
    /// end and the next one's start can be the same moment: waiting at the
    /// end must still show the finished slide.
    slide: usize,
    /// Playing towards this stop (`None`: waiting for the presenter).
    pub until: Option<f64>,
    /// Past the last slide: a black "end" screen.
    pub ended: bool,
    pub black: bool,
}

impl Playback {
    /// Starts at slide `slide`, playing to its first stop.
    pub fn new(deck: Deck, slide: usize) -> Self {
        let slide = slide.min(deck.len().saturating_sub(1));
        let t = deck.slide_start(slide);
        let until = deck.next_stop(t);
        Self { deck, t, slide, until, ended: false, black: false }
    }

    /// Moves on to the next slide once time has passed this one's end.
    fn settle(&mut self) {
        while self.slide + 1 < self.deck.len() && self.t > self.deck.slide_end(self.slide) + 1e-9 {
            self.slide += 1;
        }
    }

    /// Advances the clock by `dt` seconds. Returns true while moving.
    pub fn tick(&mut self, dt: f64) -> bool {
        let Some(u) = self.until else { return false };
        self.t += dt;
        if self.t >= u {
            self.t = u;
            self.until = None;
        }
        self.settle();
        true
    }

    /// Next (→, Space, click): finish what's playing, else play to the next stop.
    /// Returns false once past the end (the presentation should close).
    pub fn next(&mut self) -> bool {
        self.black = false;
        if self.ended {
            return false;
        }
        if let Some(u) = self.until.take() {
            self.t = u;
            self.settle();
            return true;
        }
        match self.deck.next_stop(self.t) {
            Some(s) => self.until = Some(s),
            None => self.ended = true,
        }
        true
    }

    /// Previous (←): back to the previous slide, finished.
    pub fn previous(&mut self) {
        self.black = false;
        if self.ended {
            self.ended = false;
            return;
        }
        self.until = None;
        if self.slide == 0 {
            self.t = 0.0;
        } else {
            self.slide -= 1;
            self.t = self.deck.slide_end(self.slide);
        }
    }

    pub fn first(&mut self) {
        *self = Self::new(self.deck.clone(), 0);
    }

    pub fn last(&mut self) {
        self.until = None;
        self.ended = false;
        self.slide = self.deck.len().saturating_sub(1);
        self.t = self.deck.duration();
    }

    pub fn slide(&self) -> usize {
        self.slide
    }

    /// What's on screen.
    pub fn frame(&self) -> Option<xenify_anim::deck::DeckFrame> {
        let (start, end) = (self.deck.slide_start(self.slide), self.deck.slide_end(self.slide));
        if self.t >= start - 1e-9 && self.t <= end + 1e-9 {
            self.deck.plain(self.slide, self.t - start)
        } else {
            self.deck.at(self.t)
        }
    }
}

pub struct Presenter {
    pub playback: Playback,
    pub screen: Screen,
    last_tick: Option<Instant>,
    pub(crate) canvas: crate::gpu_canvas::GpuCanvas,
    /// The presenter view's current and next slides.
    pub(crate) view_canvas: crate::gpu_canvas::GpuCanvas,
    pub(crate) next_canvas: crate::gpu_canvas::GpuCanvas,
    pub(crate) text: xenify_render::TextEngine,
    started: Instant,
    /// When the talk's timer started (Reset restarts it).
    pub(crate) timer: Instant,
    /// Presenting in this window: showing the presenter view instead of the
    /// slides (V switches, for rehearsing).
    pub presenter_view: bool,
}

/// Starts presenting from slide `slide` on `screen`.
pub fn start(app: &mut XenifyApp, slide: usize, screen: Screen, ctx: &egui::Context) {
    app.settle_ai();
    app.playing = false;
    let deck = Deck::new(app.project());
    if deck.is_empty() {
        app.status = "Nothing to present.".into();
        return;
    }
    if screen == Screen::ThisWindow {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }
    app.presenter = Some(Presenter {
        playback: Playback::new(deck, slide),
        screen,
        last_tick: None,
        canvas: Default::default(),
        view_canvas: Default::default(),
        next_canvas: Default::default(),
        text: xenify_render::TextEngine::new(),
        started: Instant::now(),
        timer: Instant::now(),
        presenter_view: false,
    });
}

pub(crate) fn stop(app: &mut XenifyApp, ctx: &egui::Context) {
    if let Some(p) = app.presenter.take() {
        if p.screen == Screen::ThisWindow {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        // Leave the editor on the slide that was showing.
        if let Some(comp) = p.playback.deck.slide_comp(p.playback.slide()) {
            app.open_comp(comp);
        }
    }
}

/// The screens to offer: this window, then each monitor (projectors first).
pub fn screens() -> Vec<Screen> {
    let mut monitors = xenify_platform::desktop::native::monitors();
    monitors.sort_by_key(|m| m.primary);
    std::iter::once(Screen::ThisWindow).chain(monitors.into_iter().map(Screen::Monitor)).collect()
}

/// The default screen: another monitor if there is one (the audience's),
/// else this window.
pub fn default_screen() -> Screen {
    screens().into_iter().find(|s| matches!(s, Screen::Monitor(m) if !m.primary)).unwrap_or(Screen::ThisWindow)
}

pub fn screen_label(s: &Screen) -> String {
    match s {
        Screen::ThisWindow => "This window (full screen)".into(),
        Screen::Monitor(m) => {
            format!("{} — {}×{}{}", m.name, m.width, m.height, if m.primary { " (main)" } else { "" })
        }
    }
}

/// Draws the presentation, if one is running. Returns true when it takes the
/// whole window (the editor is hidden).
pub fn show(app: &mut XenifyApp, ui: &mut egui::Ui, frame: &eframe::Frame) -> bool {
    let Some(p) = &app.presenter else { return false };
    // Sound: the deck's mix while it plays; while it waits at a pause, the
    // soundtrack carries on if it's set to keep playing.
    let waiting = p.playback.until.is_none() || p.playback.ended;
    let keep = app.editor.project().soundtrack.keep_playing && !p.playback.ended;
    if !(waiting && keep && app.sound.is_playing()) {
        let source = crate::sound_ui::Source::Deck(p.playback.deck.duration());
        let t = p.playback.t;
        let enabled = app.prefs.play_sound;
        if let Some(msg) = app.sound.sync(app.editor.project(), source, t, !waiting, enabled) {
            app.status = msg;
        }
    }
    let Some(p) = &app.presenter else { return false };
    let ctx = ui.ctx().clone();
    let rs = frame.wgpu_render_state().cloned();
    match p.screen.clone() {
        Screen::ThisWindow => {
            let rect = ui.max_rect();
            let alive = if p.presenter_view {
                crate::presenter_view::show(app, ui, rect, rs.as_ref())
            } else {
                present_ui(app, ui, rect, rs.as_ref())
            };
            if !alive {
                stop(app, &ctx);
            }
            true
        }
        Screen::Monitor(m) => {
            let id = egui::ViewportId::from_hash_of("xenify-presentation");
            let builder = egui::ViewportBuilder::default()
                .with_title("Xenify Vector — presentation")
                .with_position(egui::pos2(m.x as f32, m.y as f32) / ctx.pixels_per_point())
                .with_inner_size(egui::vec2(m.width as f32, m.height as f32) / ctx.pixels_per_point())
                .with_decorations(false)
                .with_fullscreen(true);
            let mut alive = true;
            ctx.show_viewport_immediate(id, builder, |ui, _class| {
                if ui.input(|i| i.viewport().close_requested()) {
                    alive = false;
                }
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(ui, |ui| {
                    let rect = ui.max_rect();
                    alive &= present_ui(app, ui, rect, rs.as_ref());
                });
            });
            // This window: the presenter's own view.
            if alive && app.presenter.is_some() {
                let rect = ui.max_rect();
                alive = crate::presenter_view::show(app, ui, rect, rs.as_ref());
            }
            if !alive {
                stop(app, &ctx);
            }
            true
        }
    }
}

/// The slide itself and the presenter's keys. Returns false to end.
fn present_ui(
    app: &mut XenifyApp,
    ui: &mut egui::Ui,
    rect: egui::Rect,
    rs: Option<&eframe::egui_wgpu::RenderState>,
) -> bool {
    let project = app.editor.project().clone();
    let Some(p) = app.presenter.as_mut() else { return false };
    let response = ui.allocate_rect(rect, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            "Slide show: click or press Space for the next step, Esc to end",
        )
    });

    let alive = handle_keys(p, ui, response.clicked(), response.secondary_clicked());
    tick(p, ui);

    // Picture.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
    if p.playback.ended {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "End of presentation — click or press → to exit",
            egui::FontId::proportional(20.0),
            egui::Color32::from_gray(150),
        );
    } else if !p.playback.black {
        let frame = p.playback.frame();
        let label = format!("Slide {}", p.playback.slide() + 1);
        paint_frame(&mut p.canvas, &mut p.text, &project, frame, &painter, rect, rs, &label);
    }
    // A hint for the first seconds.
    if p.started.elapsed().as_secs_f32() < 3.0 {
        painter.text(
            rect.center_bottom() - egui::vec2(0.0, 24.0),
            egui::Align2::CENTER_BOTTOM,
            "→ / Space / click: next    ←: back    B: black screen    V: presenter view    Esc: end",
            egui::FontId::proportional(14.0),
            egui::Color32::from_white_alpha(140),
        );
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    alive
}

/// The presenter's keys (and clicks). Returns false to end.
pub(crate) fn handle_keys(p: &mut Presenter, ui: &egui::Ui, clicked: bool, right_clicked: bool) -> bool {
    let mut alive = true;
    let (next, prev, first, last, black, esc, view) = ui.input(|i| {
        let k = |key| i.key_pressed(key);
        (
            k(egui::Key::ArrowRight)
                || k(egui::Key::ArrowDown)
                || k(egui::Key::Space)
                || k(egui::Key::PageDown)
                || k(egui::Key::Enter)
                || k(egui::Key::N),
            k(egui::Key::ArrowLeft)
                || k(egui::Key::ArrowUp)
                || k(egui::Key::PageUp)
                || k(egui::Key::Backspace)
                || k(egui::Key::P),
            k(egui::Key::Home),
            k(egui::Key::End),
            k(egui::Key::B) || k(egui::Key::Period),
            k(egui::Key::Escape),
            k(egui::Key::V),
        )
    });
    if next || clicked {
        alive &= p.playback.next();
    }
    if prev || right_clicked {
        p.playback.previous();
    }
    if first {
        p.playback.first();
    }
    if last {
        p.playback.last();
    }
    if black {
        p.playback.black = !p.playback.black;
    }
    if view && p.screen == Screen::ThisWindow {
        p.presenter_view = !p.presenter_view;
    }
    if esc {
        alive = false;
    }
    alive
}

/// Advances the clock (both views call it; the second call in a frame adds nothing).
pub(crate) fn tick(p: &mut Presenter, ui: &egui::Ui) {
    let now = Instant::now();
    let dt = p.last_tick.map_or(0.0, |t| (now - t).as_secs_f64());
    p.last_tick = Some(now);
    if p.playback.tick(dt) {
        ui.ctx().request_repaint();
    }
}

/// Draws `frame` of the deck into `rect` (or `label` without a GPU).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_frame(
    canvas: &mut crate::gpu_canvas::GpuCanvas,
    text: &mut xenify_render::TextEngine,
    project: &xenify_doc::Project,
    frame: Option<xenify_anim::deck::DeckFrame>,
    painter: &egui::Painter,
    rect: egui::Rect,
    rs: Option<&eframe::egui_wgpu::RenderState>,
    label: &str,
) {
    let Some(frame) = frame else { return };
    match rs {
        Some(rs) => {
            let ppp = painter.ctx().pixels_per_point();
            let size = (((rect.width() * ppp) as u32).max(1), ((rect.height() * ppp) as u32).max(1));
            match canvas.render(rs, size, |scene| {
                xenify_render::encode_deck(project, &frame, scene, size.0, size.1, text);
            }) {
                Ok(id) => {
                    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                    painter.image(id, rect, uv, egui::Color32::WHITE);
                }
                Err(e) => {
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        e,
                        egui::FontId::proportional(16.0),
                        egui::Color32::RED,
                    );
                }
            }
        }
        None => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::{CompSettings, Composition, Project, Slide};

    /// Slide A (2 s, pause at 0.5 s), slide B (1 s), cut between them.
    fn playback() -> Playback {
        let mut p = Project::new();
        let a = p.compositions[0].id;
        p.compositions[0].settings.duration = 2.0;
        p.compositions[0].settings.pauses = vec![0.5];
        let b = p.alloc_id();
        p.compositions.push(Composition {
            id: b,
            settings: CompSettings { duration: 1.0, ..Default::default() },
            layers: vec![],
        });
        p.slides = vec![Slide::new(a), Slide::new(b)];
        Playback::new(Deck::new(&p), 0)
    }

    #[test]
    fn plays_to_each_stop_then_waits() {
        let mut pb = playback();
        assert_eq!(pb.until, Some(0.5));
        pb.tick(0.3);
        pb.tick(0.3);
        assert_eq!((pb.t, pb.until), (0.5, None), "stopped at the pause");
        assert!(!pb.tick(1.0), "waits for the presenter");
        assert!(pb.next());
        pb.tick(10.0);
        // Slide A has a pause, so it doesn't wait again at its end: it plays
        // out and carries on into slide B, which waits at its end.
        assert_eq!((pb.t, pb.slide()), (3.0, 1), "slide B played to its end");
        assert!(pb.next());
        assert!(pb.ended, "past the last slide: the end screen");
        assert!(!pb.next(), "then it closes");
    }

    #[test]
    fn the_showcase_needs_one_click_per_beat() {
        // Letting each part play out, a click per pause plus one per slide
        // gets through it: no stop just before a slide's end.
        let p = crate::showcase::project().expect("the showcase");
        // A click per pause, or at the end of a slide without pauses; the
        // last slide also waits at its end.
        let n = p.slides.len();
        let expected: usize = p
            .slides
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let pauses = p.comp(s.comp).unwrap().settings.pauses.len();
                if pauses == 0 || i + 1 == n { pauses + 1 } else { pauses }
            })
            .sum();
        let mut pb = Playback::new(Deck::new(&p), 0);
        let mut clicks = 0;
        loop {
            while pb.tick(0.05) {}
            clicks += 1;
            assert!(clicks < 500);
            pb.next();
            if pb.ended {
                break;
            }
        }
        assert_eq!(clicks, expected);
        // Each stop shows the finished beat for more than a moment: no stop
        // lands within a second after the one before it.
        let mut pb = Playback::new(Deck::new(&p), 0);
        let mut last = 0.0;
        loop {
            while pb.tick(0.05) {}
            let slide_start = pb.deck.slide_start(pb.slide());
            assert!(pb.t - last > 0.5 || (pb.t - slide_start) < 1e-6 || last == 0.0, "stops at {last} and {}", pb.t);
            last = pb.t;
            pb.next();
            if pb.ended {
                break;
            }
        }
    }

    #[test]
    fn next_while_playing_skips_to_the_stop_and_back_goes_to_the_previous_slide() {
        let mut pb = playback();
        pb.tick(0.1);
        pb.next();
        assert_eq!((pb.t, pb.until), (0.5, None));
        // Slide A had a pause: the next stop is slide B's end, not A's.
        pb.next();
        pb.next();
        assert_eq!((pb.t, pb.slide()), (3.0, 1));
        pb.previous();
        assert_eq!((pb.slide(), pb.t, pb.until), (0, 2.0, None), "slide A, finished");
        pb.previous();
        assert_eq!(pb.t, 0.0);
    }

    #[test]
    fn starting_mid_deck() {
        let pb = Playback::new(playback().deck, 1);
        assert_eq!((pb.t, pb.until), (2.0, Some(3.0)));
    }
}
