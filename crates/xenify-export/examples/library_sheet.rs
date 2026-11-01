//! Writes an SVG contact sheet of every diagram library shape, joined by a
//! few connectors, for checking them by eye:
//! `cargo run -p xenify-export --example library_sheet -- out.svg`

use xenify_doc::{Color, ConnectorEnd, Id, PortRef, Project, Vec2};
use xenify_ops::{apply, diagram, library};

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "library.svg".into());
    let mut p = Project::new();
    let comp = p.compositions[0].id;
    let mut settings = diagram::diagram_settings("Library", 1900, 1500);
    settings.background = Color::WHITE;
    p.comp_mut(comp).unwrap().settings = settings;
    let mut ids = Vec::new();
    let mut row = 0;
    for cat in library::Category::ALL {
        let shapes: Vec<_> = library::all()
            .iter()
            .filter(|s| s.category == cat && !s.id.ends_with("pool") && !s.id.ends_with("lane"))
            .collect();
        for (i, s) in shapes.iter().enumerate() {
            let (col, r) = (i % 8, row + i / 8);
            let at = Vec2::new(120.0 + col as f64 * 225.0, 90.0 + r as f64 * 170.0);
            let (cmd, id) = library::insert(&p, comp, s.id, at, None, None).unwrap();
            apply(&mut p, cmd).unwrap();
            ids.push(id);
        }
        row += shapes.len().div_ceil(8);
    }
    // A few connectors, some crossing, to see routing and jumps.
    for (a, b) in [(0usize, 9usize), (1, 17), (8, 3), (2, 26), (20, 30)] {
        if let (Some(&a), Some(&b)) = (ids.get(a), ids.get(b)) {
            let (cmd, _) = diagram::add_connector(
                &p,
                comp,
                ConnectorEnd::glued(a, PortRef::Auto, Vec2::ZERO),
                ConnectorEnd::glued(b, PortRef::Auto, Vec2::ZERO),
            )
            .unwrap();
            apply(&mut p, cmd).unwrap();
        }
    }
    let _ = Id(0);
    let scene = xenify_anim::evaluate_project(&p, comp, 0.0);
    let svg = xenify_export::svg::export_svg(&scene, &mut xenify_render::TextEngine::new());
    std::fs::write(&out, svg).unwrap();
    eprintln!("wrote {out}");
}
