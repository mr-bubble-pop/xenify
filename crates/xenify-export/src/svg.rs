//! SVG import (any SVG → editable path layers) and export (a frame → SVG).

use kurbo::{Affine, BezPath, Point, Shape, Vec2};
use xenify_doc::{Color, FillRule, Gradient, GradientKind, GradientStop, Id, Layer, LineCap, LineJoin, ShapeItem};

// ---------------------------------------------------------------- import

/// Parses SVG data into path layers (paint order), with ids starting at
/// `first_id`. Content is centred in a `comp_w`×`comp_h` composition and scaled
/// down to fit if it's larger. Groups are flattened and images left out; see
/// [`import_svg_document`] to keep them.
pub fn import_svg(data: &[u8], first_id: u64, comp_w: u32, comp_h: u32) -> Result<Vec<Layer>, String> {
    let opts = SvgImportOptions { keep_groups: false, keep_images: false, ..Default::default() };
    Ok(import_svg_document(data, first_id, comp_w, comp_h, opts)?.layers)
}

/// An SVG document's own size (its `width`/`height`, else its view box), in pixels.
pub fn svg_size(data: &[u8]) -> Option<(u32, u32)> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let s = tree.size();
    Some((s.width().round().max(1.0) as u32, s.height().round().max(1.0) as u32))
}

#[derive(Clone, Copy, Debug)]
pub struct SvgImportOptions {
    /// Named SVG groups (`<g id="…">`) become Xenify Draw groups.
    pub keep_groups: bool,
    /// Embedded and linked images become image layers.
    pub keep_images: bool,
    /// Longest side kept for images (larger ones are scaled down).
    pub max_image_side: u32,
    /// Where the SVG goes (its user units → composition), instead of being
    /// centred and fitted into the frame.
    pub place: Option<Affine>,
}

impl Default for SvgImportOptions {
    fn default() -> Self {
        Self { keep_groups: true, keep_images: true, max_image_side: crate::image::DEFAULT_MAX_SIDE, place: None }
    }
}

/// The layers of an imported SVG and the images they show.
#[derive(Debug)]
pub struct SvgImport {
    /// Paint order; a group comes right after (above) its members.
    pub layers: Vec<Layer>,
    pub assets: Vec<xenify_doc::ImageAsset>,
}

/// Parses SVG data into layers, ids starting at `first_id` (assets get ids
/// too). Content is centred in a `comp_w`×`comp_h` composition and scaled down
/// to fit if it's larger.
pub fn import_svg_document(
    data: &[u8],
    first_id: u64,
    comp_w: u32,
    comp_h: u32,
    opts: SvgImportOptions,
) -> Result<SvgImport, String> {
    let mut opt = usvg::Options { font_family: xenify_render::DEFAULT_FAMILY.into(), ..Default::default() };
    opt.fontdb_mut().load_system_fonts();
    // The bundled Inter, so text converts to outlines even without system fonts;
    // unknown fonts fall back to it (usvg's fallback is the generic serif family).
    let db = opt.fontdb_mut();
    db.load_font_data(xenify_render::INTER_FONT.to_vec());
    db.set_serif_family(xenify_render::DEFAULT_FAMILY);
    db.set_sans_serif_family(xenify_render::DEFAULT_FAMILY);
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| format!("couldn't read SVG: {e}"))?;

    let mut nodes = Vec::new();
    collect(tree.root(), 1.0, opts, &mut 0, &mut nodes);
    let Some(bounds) = nodes_bounds(&nodes) else {
        return Err("the SVG has no shapes Xenify Draw can import".into());
    };

    // Fit: centre in the composition; shrink to 80% if larger.
    let (w, h) = (comp_w as f64, comp_h as f64);
    let k = (0.8 * w / bounds.width().max(1.0)).min(0.8 * h / bounds.height().max(1.0)).min(1.0);
    let fit = Affine::translate(Vec2::new(w / 2.0, h / 2.0))
        * Affine::scale(k)
        * Affine::translate(-bounds.center().to_vec2());
    let (fit, k) = match opts.place {
        Some(p) => (p, p.determinant().abs().sqrt()),
        None => (fit, k),
    };

    let mut out = SvgImport { layers: Vec::new(), assets: Vec::new() };
    let mut next = first_id;
    build_layers(nodes, None, fit, k, opts, &mut next, &mut out);
    if out.layers.is_empty() {
        return Err("the SVG has no shapes Xenify Draw can import".into());
    }
    Ok(out)
}

/// A piece of the SVG, in canvas coordinates.
enum Node {
    /// `pattern`: a pattern fill's picture and its pixel → canvas placement.
    Shape {
        name: String,
        path: BezPath,
        shape: Box<ShapeItem>,
        pattern: Option<(Vec<u8>, Affine)>,
    },
    Image {
        name: String,
        transform: Affine,
        size: (f64, f64),
        data: Vec<u8>,
    },
    Group {
        name: String,
        children: Vec<Node>,
    },
}

impl Node {
    fn rename(&mut self, to: &str) {
        match self {
            Node::Shape { name, shape, .. } => {
                *name = to.to_string();
                shape.name = to.to_string();
            }
            Node::Image { name, .. } | Node::Group { name, .. } => *name = to.to_string(),
        }
    }
}

fn nodes_bounds(nodes: &[Node]) -> Option<kurbo::Rect> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Shape { path, .. } => Some(path.bounding_box()),
            Node::Image { transform, size, .. } => {
                Some(transform.transform_rect_bbox(kurbo::Rect::new(0.0, 0.0, size.0, size.1)))
            }
            Node::Group { children, .. } => nodes_bounds(children),
        })
        .reduce(|a, b| a.union(b))
}

fn build_layers(
    nodes: Vec<Node>,
    parent: Option<Id>,
    fit: Affine,
    k: f64,
    opts: SvgImportOptions,
    next: &mut u64,
    out: &mut SvgImport,
) {
    fn alloc(next: &mut u64) -> Id {
        *next += 1;
        Id(*next - 1)
    }
    for node in nodes {
        let layer = match node {
            Node::Shape { name, path, mut shape, pattern } => {
                let comp_path = fit * path;
                let c = comp_path.bounding_box().center().to_vec2();
                let to_local = Affine::translate(-c) * fit;
                shape.geometry = xenify_doc::Geometry::Path { path: (Affine::translate(-c) * comp_path).into() };
                let gradients = [
                    shape.fill.as_mut().and_then(|f| f.gradient.as_mut()),
                    shape.stroke.as_mut().and_then(|s| s.gradient.as_mut()),
                ];
                for g in gradients.into_iter().flatten() {
                    for p in [&mut g.start, &mut g.end] {
                        if let xenify_doc::Property::Static(v) = p {
                            *v = (to_local * v.to_point()).to_vec2();
                        }
                    }
                }
                if let Some(s) = &mut shape.stroke {
                    s.width = (s.width.value_at(0.0) * k).into();
                    for d in &mut s.dashes {
                        *d *= k;
                    }
                    s.dash_offset = (s.dash_offset.value_at(0.0) * k).into();
                }
                if let (Some((data, placement)), Some(fill)) = (pattern, shape.fill.as_mut())
                    && let Ok(img) = crate::image::prepare(&data, opts.max_image_side)
                {
                    let asset = match out.assets.iter().find(|a| a.is(&img.data)) {
                        Some(a) => a.clone(),
                        None => {
                            let a = xenify_doc::ImageAsset::new(
                                alloc(next),
                                format!("{name} pattern"),
                                img.mime,
                                img.width,
                                img.height,
                                img.data,
                            );
                            out.assets.push(a.clone());
                            a
                        }
                    };
                    // Prepared pixels may be fewer than the file's: keep the tile's size.
                    let px = kurbo::Affine::scale_non_uniform(
                        img_px_ratio(&data, asset.width),
                        img_px_ratio(&data, asset.width),
                    );
                    let m = to_local * placement * px;
                    let [a, b, c2, d, e, f] = m.as_coeffs();
                    fill.pattern = Some(xenify_doc::Pattern {
                        asset: asset.id,
                        scale: (a * d - b * c2).abs().sqrt(),
                        rotation: b.atan2(a).to_degrees(),
                        offset: Vec2::new(e, f),
                    });
                }
                Layer::shape(alloc(next), name, vec![*shape]).with_position(c)
            }
            Node::Image { name, transform, size, data } => {
                let Ok(img) = crate::image::prepare(&data, opts.max_image_side) else { continue };
                let asset = match out.assets.iter().find(|a| a.is(&img.data)) {
                    Some(a) => a.clone(),
                    None => {
                        let a =
                            xenify_doc::ImageAsset::new(alloc(next), &name, img.mime, img.width, img.height, img.data);
                        out.assets.push(a.clone());
                        a
                    }
                };
                // Pixel space → canvas: the SVG draws the image over `size` units.
                let m = fit
                    * transform
                    * Affine::scale_non_uniform(size.0 / img.width.max(1) as f64, size.1 / img.height.max(1) as f64);
                let [a, b, c, d, _, _] = m.as_coeffs();
                let sx = a.hypot(b);
                let sy = (a * d - b * c) / sx.max(1e-12);
                let centre = kurbo::Point::new(img.width as f64 / 2.0, img.height as f64 / 2.0);
                let mut layer = Layer::image(alloc(next), name, &asset, (m * centre).to_vec2());
                layer.transform.rotation = b.atan2(a).to_degrees().into();
                layer.transform.scale = Vec2::new(sx, sy).into();
                layer
            }
            Node::Group { name, children } => {
                let id = alloc(next);
                build_layers(children, Some(id), fit, k, opts, next, out);
                let mut g = Layer::new(id, name, xenify_doc::LayerKind::Group);
                g.meta.parent = parent;
                out.layers.push(g);
                continue;
            }
        };
        let mut layer = layer;
        layer.meta.parent = parent;
        out.layers.push(layer);
    }
}

fn kurbo_affine(t: usvg::Transform) -> Affine {
    Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

fn usvg_color(c: usvg::Color, opacity: f32) -> Color {
    Color::rgba(c.red as f64 / 255.0, c.green as f64 / 255.0, c.blue as f64 / 255.0, opacity as f64)
}

/// Walks the tree, collecting paths in canvas coordinates (paint order).
/// `count` numbers unnamed paths across the whole document.
fn collect(group: &usvg::Group, opacity: f32, opts: SvgImportOptions, count: &mut usize, out: &mut Vec<Node>) {
    let opacity = opacity * group.opacity().get();
    for node in group.children() {
        match node {
            usvg::Node::Group(g) if opts.keep_groups && !g.id().is_empty() => {
                let before = *count;
                let mut children = Vec::new();
                collect(g, opacity, opts, count, &mut children);
                match children.len() {
                    0 => {}
                    // usvg wraps elements with a transform (and images) in a group
                    // carrying their id: one child is that element, named after it.
                    1 => {
                        let mut child = children.pop().unwrap();
                        if *count == before + 1 && !matches!(child, Node::Group { .. }) {
                            child.rename(g.id());
                        }
                        out.push(child);
                    }
                    _ => out.push(Node::Group { name: g.id().to_string(), children }),
                }
            }
            usvg::Node::Group(g) => collect(g, opacity, opts, count, out),
            usvg::Node::Text(t) => collect(t.flattened(), opacity, opts, count, out),
            usvg::Node::Image(img) => {
                if !opts.keep_images || !img.is_visible() {
                    continue;
                }
                let data = match img.kind() {
                    usvg::ImageKind::PNG(d)
                    | usvg::ImageKind::JPEG(d)
                    | usvg::ImageKind::GIF(d)
                    | usvg::ImageKind::WEBP(d) => d.to_vec(),
                    // Nested SVG images: import their content in place.
                    usvg::ImageKind::SVG(tree) => {
                        let mut children = Vec::new();
                        collect(tree.root(), opacity, opts, count, &mut children);
                        let t = kurbo_affine(img.abs_transform());
                        out.extend(children.into_iter().map(|n| transform_node(n, t)));
                        continue;
                    }
                };
                *count += 1;
                let name = if img.id().is_empty() { format!("Image {count}") } else { img.id().to_string() };
                let size = img.size();
                out.push(Node::Image {
                    name,
                    transform: kurbo_affine(img.abs_transform()),
                    size: (size.width() as f64, size.height() as f64),
                    data,
                });
            }
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let abs = kurbo_affine(p.abs_transform());
                let mut path = BezPath::new();
                for seg in p.data().segments() {
                    use usvg::tiny_skia_path::PathSegment as S;
                    let pt = |q: usvg::tiny_skia_path::Point| Point::new(q.x as f64, q.y as f64);
                    match seg {
                        S::MoveTo(a) => path.move_to(pt(a)),
                        S::LineTo(a) => path.line_to(pt(a)),
                        S::QuadTo(a, b) => path.quad_to(pt(a), pt(b)),
                        S::CubicTo(a, b, c) => path.curve_to(pt(a), pt(b), pt(c)),
                        S::Close => path.close_path(),
                    }
                }
                let world = abs * path;
                *count += 1;
                let name = if p.id().is_empty() { format!("Path {count}") } else { p.id().to_string() };
                let mut shape = ShapeItem::path(BezPath::new(), None, None);
                shape.name = name.clone();
                let mut pattern = None;
                if let Some(f) = p.fill() {
                    let a = f.opacity().get() * opacity;
                    shape.fill_rule = match f.rule() {
                        usvg::FillRule::NonZero => FillRule::NonZero,
                        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
                    };
                    shape.fill = match f.paint() {
                        usvg::Paint::Color(c) => Some(xenify_doc::Fill::solid(usvg_color(*c, a))),
                        usvg::Paint::LinearGradient(g) => gradient_fill(
                            g.stops(),
                            a,
                            GradientKind::Linear,
                            abs * kurbo_affine(g.transform()),
                            (g.x1(), g.y1()),
                            (g.x2(), g.y2()),
                        ),
                        usvg::Paint::RadialGradient(g) => {
                            let (cx, cy, r) = (g.cx(), g.cy(), g.r().get());
                            gradient_fill(
                                g.stops(),
                                a,
                                GradientKind::Radial,
                                abs * kurbo_affine(g.transform()),
                                (cx, cy),
                                (cx + r, cy),
                            )
                        }
                        usvg::Paint::Pattern(pt) => {
                            // A picture tile becomes a pattern fill (below); otherwise a neutral colour.
                            if let Some((data, place)) = pattern_image(pt.root()) {
                                let r = pt.rect();
                                pattern = Some((
                                    data,
                                    abs * kurbo_affine(pt.transform())
                                        * Affine::translate((r.x() as f64, r.y() as f64))
                                        * place,
                                ));
                            }
                            Some(xenify_doc::Fill::solid(Color::rgba(0.5, 0.5, 0.5, a as f64)))
                        }
                    };
                }
                if let Some(s) = p.stroke() {
                    let a = s.opacity().get() * opacity;
                    // Gradient strokes keep their gradient (and the first stop as the colour).
                    let gradient = match s.paint() {
                        usvg::Paint::LinearGradient(g) => gradient_fill(
                            g.stops(),
                            a,
                            GradientKind::Linear,
                            abs * kurbo_affine(g.transform()),
                            (g.x1(), g.y1()),
                            (g.x2(), g.y2()),
                        ),
                        usvg::Paint::RadialGradient(g) => {
                            let (cx, cy, r) = (g.cx(), g.cy(), g.r().get());
                            gradient_fill(
                                g.stops(),
                                a,
                                GradientKind::Radial,
                                abs * kurbo_affine(g.transform()),
                                (cx, cy),
                                (cx + r, cy),
                            )
                        }
                        _ => None,
                    }
                    .and_then(|f| f.gradient);
                    let color = match s.paint() {
                        usvg::Paint::Color(c) => usvg_color(*c, a),
                        usvg::Paint::LinearGradient(g) => {
                            g.stops().first().map_or(Color::BLACK, |st| usvg_color(st.color(), st.opacity().get()))
                        }
                        usvg::Paint::RadialGradient(g) => {
                            g.stops().first().map_or(Color::BLACK, |st| usvg_color(st.color(), st.opacity().get()))
                        }
                        usvg::Paint::Pattern(_) => Color::BLACK,
                    };
                    let scale = abs.determinant().abs().sqrt();
                    let dashes: Vec<f64> =
                        s.dasharray().map(|d| d.iter().map(|v| *v as f64 * scale).collect()).unwrap_or_default();
                    shape.stroke = Some(xenify_doc::Stroke {
                        cap_roundness: 0.0,
                        style: Default::default(),
                        color: color.into(),
                        width: (s.width().get() as f64 * abs.determinant().abs().sqrt()).into(),
                        join: match s.linejoin() {
                            usvg::LineJoin::Round => LineJoin::Round,
                            usvg::LineJoin::Bevel => LineJoin::Bevel,
                            _ => LineJoin::Miter,
                        },
                        cap: match s.linecap() {
                            usvg::LineCap::Round => LineCap::Round,
                            usvg::LineCap::Square => LineCap::Square,
                            usvg::LineCap::Butt => LineCap::Butt,
                        },
                        dash_offset: (s.dashoffset() as f64 * scale).into(),
                        dashes,
                        gradient,
                        // usvg has already turned SVG markers into paths.
                        ..Default::default()
                    });
                }
                if shape.fill.is_some() || shape.stroke.is_some() {
                    out.push(Node::Shape { name, path: world, shape: Box::new(shape), pattern });
                }
            }
        }
    }
}

/// Original pixels per prepared pixel (images over the size limit are shrunk on import).
fn img_px_ratio(original: &[u8], prepared_width: u32) -> f64 {
    image::ImageReader::new(std::io::Cursor::new(original))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .map_or(1.0, |(w, _)| w as f64 / prepared_width.max(1) as f64)
}

/// The first raster picture in a pattern's tile and where it sits (pixels →
/// pattern space).
fn pattern_image(group: &usvg::Group) -> Option<(Vec<u8>, Affine)> {
    for node in group.children() {
        match node {
            usvg::Node::Image(img) => {
                let data = match img.kind() {
                    usvg::ImageKind::PNG(d)
                    | usvg::ImageKind::JPEG(d)
                    | usvg::ImageKind::GIF(d)
                    | usvg::ImageKind::WEBP(d) => d.to_vec(),
                    usvg::ImageKind::SVG(_) => continue,
                };
                let (w, h) = image::ImageReader::new(std::io::Cursor::new(&data))
                    .with_guessed_format()
                    .ok()?
                    .into_dimensions()
                    .ok()?;
                let size = img.size();
                let place = kurbo_affine(img.abs_transform())
                    * Affine::scale_non_uniform(
                        size.width() as f64 / w.max(1) as f64,
                        size.height() as f64 / h.max(1) as f64,
                    );
                return Some((data, place));
            }
            usvg::Node::Group(g) => {
                if let Some(found) = pattern_image(g) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// `node` moved by `t` (for SVG images nested in SVG).
fn transform_node(node: Node, t: Affine) -> Node {
    match node {
        Node::Shape { name, path, mut shape, pattern } => {
            let gradients = [
                shape.fill.as_mut().and_then(|f| f.gradient.as_mut()),
                shape.stroke.as_mut().and_then(|s| s.gradient.as_mut()),
            ];
            for g in gradients.into_iter().flatten() {
                for p in [&mut g.start, &mut g.end] {
                    if let xenify_doc::Property::Static(v) = p {
                        *v = (t * v.to_point()).to_vec2();
                    }
                }
            }
            Node::Shape { name, path: t * path, shape, pattern: pattern.map(|(d, m)| (d, t * m)) }
        }
        Node::Image { name, transform, size, data } => Node::Image { name, transform: t * transform, size, data },
        Node::Group { name, children } => {
            Node::Group { name, children: children.into_iter().map(|n| transform_node(n, t)).collect() }
        }
    }
}

fn gradient_fill(
    stops: &[usvg::Stop],
    opacity: f32,
    kind: GradientKind,
    to_canvas: Affine,
    a: (f32, f32),
    b: (f32, f32),
) -> Option<xenify_doc::Fill> {
    let stops: Vec<GradientStop> = stops
        .iter()
        .map(|s| GradientStop {
            offset: s.offset().get() as f64,
            color: usvg_color(s.color(), s.opacity().get() * opacity),
        })
        .collect();
    let first = stops.first()?.color;
    let p = |(x, y): (f32, f32)| (to_canvas * Point::new(x as f64, y as f64)).to_vec2();
    Some(xenify_doc::Fill {
        color: first.into(),
        gradient: Some(Gradient { kind, start: p(a), end: p(b), stops }.into()),
        pattern: None,
    })
}

// ---------------------------------------------------------------- export

pub use crate::svg_export::{AnimatedSvgOptions, export_animated_svg, export_svg};

#[cfg(test)]
mod tests {
    use super::*;
    use xenify_doc::LayerKind;

    const SAMPLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
      <defs><linearGradient id="lg" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
      <rect id="box" x="10" y="10" width="80" height="80" fill="url(#lg)"/>
      <g transform="translate(100 0)"><circle id="dot" cx="50" cy="50" r="40" fill="#00ff00" stroke="#000" stroke-width="4"/></g>
    </svg>"##;

    #[test]
    fn text_becomes_outlines_with_the_bundled_font() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 50"><text x="10" y="40" font-family="NoSuchFont" font-size="30" fill="#fff">Hi</text></svg>"##;
        let layers = import_svg(svg.as_bytes(), 1, 1000, 1000).unwrap();
        assert!(!layers.is_empty(), "text should import as outline paths");
    }

    #[test]
    fn imports_shapes_colours_gradients_and_transforms() {
        let layers = import_svg(SAMPLE.as_bytes(), 10, 1000, 1000).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].id, Id(10));
        assert_eq!(layers[0].meta.name, "box");
        // Centred in the composition, scale 1 (fits): the SVG's centre (100,50) → (500,500).
        let box_pos = layers[0].transform.position.value_at(0.0);
        let dot_pos = layers[1].transform.position.value_at(0.0);
        assert!((box_pos - Vec2::new(450.0, 500.0)).hypot() < 1e-3, "{box_pos:?}");
        assert!((dot_pos - Vec2::new(550.0, 500.0)).hypot() < 1e-3, "{dot_pos:?}");
        let g = layers[0].shapes()[0].fill.as_ref().unwrap().gradient.as_ref().unwrap().value_at(0.0);
        assert_eq!(g.stops.len(), 2);
        assert!(g.end.x > g.start.x, "left to right");
        let dot = &layers[1].shapes()[0];
        assert_eq!(dot.fill.as_ref().unwrap().color.value_at(0.0), Color::rgb(0.0, 1.0, 0.0));
        assert_eq!(dot.stroke.as_ref().unwrap().width.value_at(0.0), 4.0);
    }

    #[test]
    fn exports_mattes_as_masks() {
        let mut c = xenify_doc::Project::new().compositions.remove(0);
        let dot =
            Layer::shape(Id(1), "dot", vec![ShapeItem::ellipse(Vec2::new(50.0, 50.0), Color::rgb(1.0, 0.0, 0.0))]);
        let mut bg = Layer::shape(Id(2), "bg", vec![ShapeItem::rect(Vec2::new(300.0, 300.0), Color::BLACK)]);
        bg.meta.matte = Some(xenify_doc::Matte { layer: Id(1), mode: xenify_doc::MatteMode::AlphaInverted });
        c.layers = vec![dot, bg];
        let svg = export_svg(&xenify_anim::evaluate(&c, 0.0), &mut xenify_anim::ApproxText);
        assert!(svg.contains(r#"<mask id="m1""#) && svg.contains(r#"<g mask="url(#m1)">"#), "{svg}");
        // Inverted: white everywhere, the matte painted black; the matte isn't drawn outside the mask.
        assert!(svg.contains(r##"fill="#ffffff"/>"##) && svg.contains(r##"fill="#000000""##));
        assert!(!svg.contains("#ff0000"));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn export_then_import_round_trips_geometry() {
        let project = xenify_doc::demo_project();
        let comp = project.main_comp().unwrap();
        let scene = xenify_anim::evaluate(comp, 2.0);
        let svg = export_svg(&scene, &mut xenify_anim::ApproxText);
        assert!(svg.contains("<svg") && svg.contains("<path") && svg.contains("<text"));
        let layers = import_svg(svg.as_bytes(), 1, comp.settings.width, comp.settings.height).unwrap();
        // Background rect + ring + square + ball (+ text converted to outlines when fonts exist).
        assert!(layers.len() >= 4, "{}", layers.len());
    }

    fn tiny_png() -> Vec<u8> {
        xenify_render::RgbaImage { width: 4, height: 2, pixels: vec![255; 32] }.to_png()
    }

    #[test]
    fn keeps_named_groups_and_images() {
        use base64::Engine;
        let uri = base64::engine::general_purpose::STANDARD.encode(tiny_png());
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100">
              <g id="logo"><rect id="a" x="0" y="0" width="10" height="10" fill="#f00"/><circle id="b" cx="50" cy="50" r="5" fill="#0f0"/></g>
              <image id="photo" x="100" y="20" width="40" height="20" href="data:image/png;base64,{uri}"/>
            </svg>"##
        );
        let out = import_svg_document(svg.as_bytes(), 10, 1000, 1000, SvgImportOptions::default()).unwrap();
        let names: Vec<&str> = out.layers.iter().map(|l| l.meta.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "logo", "photo"], "members sit below their group");
        let logo = out.layers[2].id;
        assert!(matches!(out.layers[2].kind, LayerKind::Group));
        assert_eq!(out.layers[0].meta.parent, Some(logo));
        assert_eq!(out.layers[3].meta.parent, None);

        assert_eq!(out.assets.len(), 1);
        let asset = &out.assets[0];
        assert_eq!((asset.width, asset.height, asset.mime.as_str()), (4, 2, "image/png"));
        let photo = &out.layers[3];
        assert!(matches!(photo.kind, LayerKind::Image { asset: a, .. } if a == asset.id));
        // 4×2 pixels drawn over 40×20 SVG units: scale 10.
        assert!((photo.transform.scale.value_at(0.0) - Vec2::new(10.0, 10.0)).hypot() < 1e-6);
        // Content spans (0,0)–(140,55): its centre (70, 27.5) lands on the comp's (500, 500),
        // so the photo's centre (120, 30) is at (550, 502.5).
        assert!((photo.transform.position.value_at(0.0) - Vec2::new(550.0, 502.5)).hypot() < 1e-6);
        let ids: std::collections::HashSet<u64> =
            out.layers.iter().map(|l| l.id.0).chain(out.assets.iter().map(|a| a.id.0)).collect();
        assert_eq!(ids.len(), out.layers.len() + out.assets.len(), "unique ids");

        // The old entry point still flattens and skips images.
        let flat = import_svg(svg.as_bytes(), 1, 1000, 1000).unwrap();
        assert_eq!(flat.len(), 2);
        assert!(flat.iter().all(|l| matches!(l.kind, LayerKind::Shape { .. })));
    }

    #[test]
    fn exports_text_on_a_path_as_svg_text_path() {
        let mut c = xenify_doc::Project::new().compositions.remove(0);
        let circle = Layer::shape(Id(1), "c", vec![ShapeItem::ellipse(Vec2::new(200.0, 200.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 300.0));
        let mut t = xenify_doc::TextLayer::new("Round & round", "Inter", 24.0, Color::WHITE).centered();
        t.letter_spacing = 3.0;
        t.path = Some(xenify_doc::TextPath { layer: Id(1), offset: 0.0, flip: false });
        c.layers = vec![circle, Layer::new(Id(2), "t", LayerKind::Text(t))];
        let svg = export_svg(&xenify_anim::evaluate(&c, 0.0), &mut xenify_anim::ApproxText);
        assert!(svg.contains(r##"<textPath href="#p1" startOffset="50%">Round &amp; round</textPath>"##), "{svg}");
        assert!(svg.contains(r#"<path id="p1" d="M"#) && svg.contains(r#"letter-spacing="3""#));
        assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
    }

    #[test]
    fn exports_effects_as_filters_and_blend_modes() {
        use xenify_doc::{BlendMode, Effect, EffectKind};
        let mut c = xenify_doc::Project::new().compositions.remove(0);
        let mut sq = Layer::shape(Id(1), "sq", vec![ShapeItem::rect(Vec2::new(100.0, 100.0), Color::WHITE)])
            .with_position(Vec2::new(300.0, 300.0));
        sq.meta.effects = vec![
            Effect::new(EffectKind::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 0.5).into(),
                offset: Vec2::new(8.0, 10.0).into(),
                blur: 12.0.into(),
            }),
            Effect::new(EffectKind::Blur { radius: 4.0.into() }),
        ];
        sq.meta.blend = BlendMode::Multiply;
        c.layers = vec![sq];
        let svg = export_svg(&xenify_anim::evaluate(&c, 0.0), &mut xenify_anim::ApproxText);
        assert!(svg.contains(r#"<g class="blend-multiply" filter="url(#f1)">"#), "{svg}");
        assert!(svg.contains(".blend-multiply { mix-blend-mode: multiply }"));
        assert!(svg.contains(r#"<feOffset dx="8" dy="10" result="s0o"/>"#));
        assert!(svg.contains(r#"<feGaussianBlur in="SourceGraphic" stdDeviation="2" result="content"/>"#));
        assert!(svg.contains(r#"<feMergeNode in="s0"/>"#) && svg.contains(r#"<feMergeNode in="content"/>"#));
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        assert!(tree.root().has_children());
    }

    #[test]
    fn exports_image_layers() {
        let mut p = xenify_doc::Project::new();
        let asset = xenify_doc::ImageAsset::new(p.alloc_id(), "px", "image/png", 4, 2, tiny_png());
        let id = p.alloc_id();
        let layer = Layer::image(id, "px", &asset, Vec2::new(100.0, 100.0));
        let comp = p.compositions[0].id;
        p.comp_mut(comp).unwrap().layers.push(layer);
        p.assets.push(asset.clone());

        let scene = xenify_anim::evaluate_project(&p, comp, 0.0);
        let svg = export_svg(&scene, &mut xenify_anim::ApproxText);
        assert!(svg.contains(r#"<image id="i1" width="4" height="2""#), "{svg}");
        assert!(svg.contains(&format!(r#"href="{}""#, asset.data_uri())));
        assert!(svg.contains(r##"<use href="#i1"/>"##));
        let back = import_svg_document(svg.as_bytes(), 1, 1920, 1080, SvgImportOptions::default()).unwrap();
        assert_eq!(back.assets.len(), 1, "round trip keeps the image");

        let animated =
            crate::svg::export_animated_svg(&p, comp, &mut xenify_anim::ApproxText, Default::default()).unwrap();
        assert_eq!(animated.matches("base64,").count(), 1, "the image is stored once");

        let lottie = crate::lottie::export_lottie(&p, comp, &mut xenify_render::TextEngine::new()).unwrap();
        let a = &lottie["assets"][0];
        assert_eq!((a["e"].as_i64(), a["w"].as_i64()), (Some(1), Some(4)));
        assert!(a["p"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(lottie["layers"][0]["ty"], 2);
        assert_eq!(lottie["layers"][0]["refId"], a["id"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(import_svg(b"not svg", 1, 100, 100).is_err());
    }
}
