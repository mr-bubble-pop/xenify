//! Insert › Equation (Alt+=), or double-click an equation: a window with a
//! visual editor. Type into the equation itself: `/` makes a fraction, `^` a
//! power, `_` a subscript, `\` starts a command (`\sqrt`, `\alpha`); click
//! into any box (a root, a power) or move with the arrows and Tab. Palettes
//! of structures and symbols put them at the caret. The LaTeX is under
//! Advanced, for typing or pasting it directly.

use crate::app::WriteApp;
use crate::canvas::Canvas;
use egui::{Color32, Pos2, Rect, Vec2};
use xenify_math::edit::MathEdit;
use xenify_wp_doc::{Field, Pos, ops};

pub struct EquationEditor {
    /// The equation being edited (the position before its character).
    pub at: Pos,
    /// The LaTeX (kept in step with `math`).
    pub latex: String,
    pub display: bool,
    /// Inserted just now: cancelling removes it again.
    pub fresh: bool,
    /// The visual editor's tree and caret.
    math: MathEdit,
    /// A `\command` being typed in the visual editor.
    command: Option<String>,
    /// Put the keyboard in the visual editor next frame.
    focus: bool,
    canvas: Canvas,
    rendered: Option<(String, bool, u32, [i32; 2])>,
}

/// Palette: (button, what it does, hover). The action is a command name
/// (`frac` → `\frac`), or `^`, `_`, `(`, `[`, `{` as typed.
const STRUCTURES: &[(&str, &str, &str)] = &[
    ("a/b", "frac", "Fraction (or type /)"),
    ("xⁿ", "^", "Power (or type ^)"),
    ("xₙ", "_", "Subscript (or type _)"),
    ("√", "sqrt", "Square root (\\sqrt)"),
    ("ⁿ√", "root", "Root with an index"),
    ("∑", "sum", "Sum (\\sum)"),
    ("∫", "int", "Integral (\\int)"),
    ("lim", "lim", "Limit (\\lim)"),
    ("( )", "(", "Brackets that grow (or type ( )"),
    ("[ ]", "[", "Square brackets that grow"),
    ("{ }", "{", "Braces that grow"),
    ("x̂", "hat", "Hat (\\hat)"),
    ("vec", "vec", "Vector arrow (\\vec)"),
    ("x̄", "overline", "Bar (\\overline)"),
    ("(⋮)", "pmatrix", "2×2 matrix (\\pmatrix)"),
    ("{⋮", "cases", "Cases (\\cases)"),
    ("sin", "sin", "Sine (and \\cos, \\log…: type \\ and the name)"),
];

const SYMBOLS: &[(&str, &str)] = &[
    ("α", "\\alpha"),
    ("β", "\\beta"),
    ("γ", "\\gamma"),
    ("δ", "\\delta"),
    ("ε", "\\epsilon"),
    ("θ", "\\theta"),
    ("λ", "\\lambda"),
    ("μ", "\\mu"),
    ("π", "\\pi"),
    ("σ", "\\sigma"),
    ("φ", "\\phi"),
    ("ω", "\\omega"),
    ("Δ", "\\Delta"),
    ("Σ", "\\Sigma"),
    ("Ω", "\\Omega"),
    ("±", "\\pm"),
    ("×", "\\times"),
    ("÷", "\\div"),
    ("·", "\\cdot"),
    ("≤", "\\le"),
    ("≥", "\\ge"),
    ("≠", "\\ne"),
    ("≈", "\\approx"),
    ("≡", "\\equiv"),
    ("∝", "\\propto"),
    ("→", "\\to"),
    ("⇒", "\\Rightarrow"),
    ("⇔", "\\iff"),
    ("∞", "\\infty"),
    ("∂", "\\partial"),
    ("∇", "\\nabla"),
    ("∈", "\\in"),
    ("∉", "\\notin"),
    ("⊂", "\\subset"),
    ("∪", "\\cup"),
    ("∩", "\\cap"),
    ("∀", "\\forall"),
    ("∃", "\\exists"),
    ("∅", "\\emptyset"),
    ("…", "\\ldots"),
];

impl WriteApp {
    /// Insert › Equation: a new (empty) equation at the caret, and the editor on it.
    pub fn insert_equation(&mut self) {
        let display = self.editor.doc.para(self.editor.sel.focus.para).is_some_and(|p| p.text.is_empty());
        let field = Field::Equation { latex: String::new(), display, omml: String::new() };
        if ops::insert_field(&mut self.editor, field, "Insert equation").is_ok() {
            let at = self.editor.sel.focus;
            let at = Pos::new(at.para, at.offset - Field::CHAR.len_utf8());
            self.edit_equation(at, true);
        }
    }

    /// Opens the editor on the equation at `at`.
    pub fn edit_equation(&mut self, at: Pos, fresh: bool) {
        let Some((latex, display)) = ops::equation_at(&self.editor.doc, at) else { return };
        self.equation = Some(EquationEditor {
            at,
            math: MathEdit::new(&latex),
            latex,
            display,
            fresh,
            command: None,
            focus: true,
            canvas: Canvas::default(),
            rendered: None,
        });
    }
}

/// A palette button's action in the visual editor.
fn palette(math: &mut MathEdit, action: &str) {
    match action {
        "^" | "_" => math.insert_latex(action),
        "(" | "[" => math.type_char(action.chars().next().unwrap_or('(')),
        "{" => math.insert_latex("\\left\\{ \\right\\}"),
        a if a.starts_with('\\') => math.insert_latex(a),
        a => math.command(a),
    }
}

/// Commands offered while a `\name` is being typed.
const COMMANDS: &[&str] = &[
    "frac",
    "sqrt",
    "root",
    "sum",
    "int",
    "prod",
    "lim",
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "theta",
    "lambda",
    "mu",
    "pi",
    "sigma",
    "phi",
    "omega",
    "Delta",
    "Sigma",
    "Omega",
    "infty",
    "partial",
    "nabla",
    "pm",
    "times",
    "div",
    "cdot",
    "le",
    "ge",
    "ne",
    "approx",
    "equiv",
    "to",
    "Rightarrow",
    "in",
    "subset",
    "cup",
    "cap",
    "forall",
    "exists",
    "hat",
    "vec",
    "bar",
    "overline",
    "sin",
    "cos",
    "tan",
    "log",
    "ln",
    "exp",
    "binom",
    "pmatrix",
    "bmatrix",
    "cases",
];

/// Keys typed into the visual editor (while it has the keyboard). Returns
/// whether the equation changed.
fn keys(ed: &mut EquationEditor, ui: &egui::Ui) -> bool {
    use egui::{Event, Key};
    let events = ui.input(|i| i.events.clone());
    let mut changed = false;
    let finish = |ed: &mut EquationEditor| {
        if let Some(name) = ed.command.take()
            && !name.is_empty()
        {
            ed.math.command(&name);
        }
    };
    for e in events {
        match e {
            Event::Text(t) => {
                for c in t.chars() {
                    changed = true;
                    if let Some(cmd) = ed.command.as_mut() {
                        if c.is_ascii_alphabetic() {
                            cmd.push(c);
                            continue;
                        }
                        finish(ed);
                        if c == ' ' {
                            continue;
                        }
                    }
                    if c == '\\' {
                        ed.command = Some(String::new());
                    } else {
                        ed.math.type_char(c);
                    }
                }
            }
            Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command => {
                changed = true;
                if let Some(cmd) = ed.command.as_mut() {
                    match key {
                        Key::Backspace => {
                            if cmd.pop().is_none() {
                                ed.command = None;
                            }
                        }
                        Key::Escape => ed.command = None,
                        Key::Enter | Key::Tab | Key::ArrowRight => finish(ed),
                        _ => {}
                    }
                    continue;
                }
                match key {
                    Key::Backspace => ed.math.backspace(),
                    Key::Delete => ed.math.delete(),
                    Key::ArrowLeft => ed.math.left(),
                    Key::ArrowRight => ed.math.right(),
                    Key::ArrowUp => ed.math.vertical(true),
                    Key::ArrowDown => ed.math.vertical(false),
                    Key::Home => ed.math.home(),
                    Key::End => ed.math.end(),
                    Key::Tab => ed.math.next_box(modifiers.shift),
                    _ => changed = false,
                }
            }
            _ => {}
        }
    }
    changed
}

pub fn window(app: &mut WriteApp, ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(mut ed) = app.equation.take() else { return };
    let mut keep = true;
    let mut done = false;
    let mut remove = false;
    let mut open = true;
    egui::Window::new("Equation")
        .open(&mut open)
        .default_size([560.0, 420.0])
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-590.0, 110.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut ed.display, "On its own line");
                });
            });
            // The equation, edited where it's drawn.
            visual(&mut ed, ui, frame);
            ui.add(egui::Label::new(
                egui::RichText::new(
                    "Type into the equation: / makes a fraction, ^ a power, _ a subscript, \\ a command (\\sqrt, \\alpha). \
                     Click into any box, or move with the arrows and Tab.",
                )
                .weak(),
            ).wrap());
            if let Some(cmd) = ed.command.clone() {
                // Commands that start with what's typed: click one, or keep typing.
                ui.horizontal_wrapped(|ui| {
                    ui.weak(format!("\\{cmd}"));
                    for name in COMMANDS.iter().filter(|n| n.starts_with(cmd.as_str())).take(16) {
                        if ui.small_button(*name).clicked() {
                            ed.command = None;
                            ed.math.command(name);
                            ed.focus = true;
                        }
                    }
                    ui.weak("— Space or Enter to put it in");
                });
            }
            ui.add_space(4.0);
            let mut action = None;
            egui::CollapsingHeader::new("Structures").default_open(true).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, act, tip) in STRUCTURES {
                        if ui.button(*label).on_hover_text(*tip).clicked() {
                            action = Some(act.to_string());
                        }
                    }
                });
            });
            egui::CollapsingHeader::new("Symbols").default_open(true).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, cmd) in SYMBOLS {
                        if ui.button(*label).on_hover_text(*cmd).clicked() {
                            action = Some(cmd.to_string());
                        }
                    }
                });
            });
            if let Some(a) = action {
                ed.command = None;
                palette(&mut ed.math, &a);
                ed.latex = ed.math.latex();
                ed.focus = true;
            }
            egui::CollapsingHeader::new("Advanced: LaTeX").id_salt("equation-latex-advanced").show(ui, |ui| {
                ui.weak("The equation as LaTeX — type or paste here (\\frac{a}{b}, x^2, \\sqrt{x}, \\alpha…).");
                let out = egui::TextEdit::multiline(&mut ed.latex)
                    .id(egui::Id::new("equation-latex"))
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                if out.response.changed() {
                    ed.math = MathEdit::new(&ed.latex);
                    ed.command = None;
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Done").strong())).on_hover_text("Ctrl+Enter").clicked()
                {
                    done = true;
                }
                if ui.button("Cancel").on_hover_text("Esc").clicked() {
                    keep = false;
                }
                if ui.button("Delete equation").clicked() {
                    remove = true;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
                done = true;
            }
            if ed.command.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                keep = false;
            }
        });
    if !open {
        keep = false;
    }
    let empty = ed.latex.trim().is_empty();
    if remove || (!keep && ed.fresh) || (done && empty) {
        // Cancelled a new one, emptied it, or deleted it: it goes.
        let _ = ops::remove_equation(&mut app.editor, ed.at);
        app.refocus = true;
        return;
    }
    if done {
        let _ = ops::set_equation(&mut app.editor, ed.at, ed.latex.trim(), ed.display);
        // The caret after it.
        app.editor.sel = xenify_wp_doc::Selection::caret(Pos::new(ed.at.para, ed.at.offset + Field::CHAR.len_utf8()));
        app.refocus = true;
        return;
    }
    if keep {
        app.equation = Some(ed);
    } else {
        app.refocus = true;
    }
}

/// The visual editor: the equation drawn by the page renderer, with its
/// empty boxes and the caret drawn over it. Click to put the caret; type
/// when it has the keyboard.
fn visual(ed: &mut EquationEditor, ui: &mut egui::Ui, frame: &eframe::Frame) {
    let size = 26.0;
    let text = ui.visuals().text_color();
    let color = xenify_wp_doc::Rgba::rgb(text.r(), text.g(), text.b());
    let id = egui::Id::new("equation-visual");
    let avail = ui.available_width();
    // Sized from last frame's equation (it changes as you type).
    let t = xenify_wp_layout::equation::typeset_edit(&ed.math, ed.display, size, color);
    let (w, h) = t.as_ref().map_or((0.0, 0.0), |(t, _)| (t.width, t.ascent + t.descent));
    let area = Vec2::new(avail, (h + 32.0).clamp(80.0, 260.0));
    let (rect, resp) = ui.allocate_exact_size(area, egui::Sense::click());
    let resp = ui.interact(rect, id, egui::Sense::click()).union(resp);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Equation editor"));
    let has_focus = ui.memory(|m| m.has_focus(id));
    if std::mem::take(&mut ed.focus) || resp.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    // Arrows and Tab move the caret here, not the keyboard focus.
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
        )
    });
    let frame_color =
        if has_focus { crate::theme::accent(ui.visuals()) } else { ui.visuals().widgets.inactive.bg_stroke.color };
    ui.painter().rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(1.0, frame_color), egui::StrokeKind::Inside);
    let Some((t, b)) = t else { return };
    // Where the equation sits: centred, shrunk to fit when it's wider.
    let k = ((rect.width() - 24.0) / w.max(1.0)).clamp(0.1, 1.0);
    let x0 = ((rect.width() - w * k) / 2.0).max(12.0);
    let y0 = (rect.height() - h * k) / 2.0 + t.ascent * k;
    let to_screen = |x: f32, y: f32| rect.min + Vec2::new(x0 + x * k, y0 + y * k);
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        ed.command = None;
        ed.math.click(&b, (p.x - rect.min.x - x0) / k, (p.y - rect.min.y - y0) / k);
    }
    if has_focus && keys(ed, ui) {
        ed.latex = ed.math.latex();
    }
    // The drawing (made again only when the equation changes).
    if let Some(rs) = frame.wgpu_render_state() {
        let ppp = ui.ctx().pixels_per_point();
        let px = ((rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32);
        let key =
            (format!("{:?}", ed.math.root), ed.display, ppp.to_bits(), [rect.width() as i32, rect.height() as i32]);
        let bg = ui.visuals().extreme_bg_color;
        let texture = if ed.rendered.as_ref() == Some(&key) {
            ed.canvas.texture()
        } else {
            let tr = kurbo::Affine::scale(ppp as f64)
                * kurbo::Affine::translate((x0 as f64, y0 as f64))
                * kurbo::Affine::scale(k as f64);
            let [r, g, bb, _] = bg.to_array();
            match ed.canvas.render(rs, px, vello::peniko::Color::from_rgb8(r, g, bb), |scene| {
                for it in &t.items {
                    xenify_wp_layout::paint::paint_item(scene, it, tr);
                }
            }) {
                Ok(id) => {
                    ed.rendered = Some(key);
                    Some(id)
                }
                Err(_) => None,
            }
        };
        if let Some(tex) = texture {
            ui.painter().image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
    }
    // Empty boxes: dashed outlines to fill.
    let weak = ui.visuals().weak_text_color();
    for p in &b.pieces {
        if let xenify_math::Piece::Slot { x, y, w, h } = *p {
            let r = Rect::from_min_max(to_screen(x, y), to_screen(x + w, y + h)).shrink(1.0);
            let dash = egui::Shape::dashed_line(
                &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                egui::Stroke::new(1.0, weak),
                3.0,
                2.0,
            );
            ui.painter().extend(dash);
        }
    }
    // The caret (and a command being typed beside it).
    if has_focus && let Some((cx, cy, ch)) = ed.math.caret(&b) {
        let top = to_screen(cx, cy);
        let accent = crate::theme::accent(ui.visuals());
        ui.painter().line_segment([top, top + Vec2::new(0.0, ch * k)], egui::Stroke::new(1.5, accent));
        if let Some(cmd) = &ed.command {
            ui.painter().text(
                top + Vec2::new(3.0, ch * k),
                egui::Align2::LEFT_BOTTOM,
                format!("\\{cmd}"),
                egui::FontId::monospace(14.0),
                accent,
            );
        }
    }
    if ed.math.is_empty() && !has_focus {
        ui.painter().text(
            rect.center() + Vec2::new(0.0, h * k / 2.0 + 14.0),
            egui::Align2::CENTER_CENTER,
            "Click and type, or pick from below",
            egui::FontId::proportional(13.0),
            weak,
        );
    }
}
