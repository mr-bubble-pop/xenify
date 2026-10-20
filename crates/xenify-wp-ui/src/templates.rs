//! File › New from template: ready-made documents (Markdown, built in).

pub const TEMPLATES: [(&str, &str, &str); 7] = [
    (
        "Letter",
        "Addresses placed for a window envelope, with mail-merge fields",
        include_str!("../templates/letter.md"),
    ),
    (
        "Invoice",
        "Header boxes, a line-items table that repeats per item, totals",
        include_str!("../templates/invoice.md"),
    ),
    ("Form", "Fill-in tables and a signature line", include_str!("../templates/form.md")),
    ("Report", "Title page, contents, headings, page numbers", include_str!("../templates/report.md")),
    (
        "Newsletter",
        "A banner and a story threaded through two linked boxes",
        include_str!("../templates/newsletter.md"),
    ),
    (
        "School report",
        "A centred title page, contents, then introduction, method, results and conclusion",
        include_str!("../templates/school-report.md"),
    ),
    (
        "Scientific paper",
        "The classic LaTeX article look: Computer Modern type, abstract, numbered sections, equations, references",
        include_str!("../templates/paper.md"),
    ),
];

/// Template `i` as a new, untitled document.
pub fn document(i: usize) -> Option<xenify_wp_doc::Document> {
    let (_, _, src) = TEMPLATES.get(i)?;
    xenify_wp_io::load(src.as_bytes(), xenify_wp_io::Format::Markdown).ok().map(|l| l.doc)
}

#[cfg(test)]
mod tests {
    #[test]
    fn templates_load_cleanly() {
        for (i, (name, _, src)) in super::TEMPLATES.iter().enumerate() {
            let loaded = xenify_wp_io::load(src.as_bytes(), xenify_wp_io::Format::Markdown).unwrap();
            assert!(loaded.warnings.is_empty(), "{name}: {:?}", loaded.warnings);
            assert!(super::document(i).is_some());
        }
        // The frames came through.
        assert_eq!(super::document(0).unwrap().sections[0].frames.len(), 2);
        assert_eq!(super::document(1).unwrap().sections[0].frames.len(), 4);
        let news = super::document(4).unwrap();
        assert_eq!(news.sections[0].frames[1].next, Some(news.sections[0].frames[2].id));
        // The school report's title page is a centred section of its own.
        let school = super::document(5).unwrap();
        assert_eq!(school.sections.len(), 2);
        assert_eq!(school.sections[0].page.v_align, xenify_wp_doc::PageVAlign::Center);
        assert_eq!(school.sections[1].page.start_number, Some(1));
        // The paper: Computer Modern, references, an equation.
        let paper = super::document(6).unwrap();
        assert_eq!(paper.styles.default_font, xenify_wp_layout::SERIF_FAMILY);
        assert_eq!(paper.meta.sources.len(), 2);
        assert!(
            paper
                .paragraphs()
                .any(|p| p.runs.iter().any(|r| matches!(r.fmt.field, Some(xenify_wp_doc::Field::Equation { .. }))))
        );
    }

    /// Renders the first pages of every template to target/template-*.png (needs a GPU; run with --ignored).
    #[test]
    #[ignore]
    fn screenshot_templates() {
        use xenify_wp_layout::{Engine, Fonts, offscreen::Offscreen, paint};
        let mut gpu = Offscreen::new().expect("a GPU");
        let mut engine = Engine::new(Fonts::bundled_only());
        for (i, (name, _, _)) in super::TEMPLATES.iter().enumerate() {
            let doc = super::document(i).unwrap();
            let layout = engine.layout(&doc);
            for (n, page) in layout.pages.iter().take(3).enumerate() {
                let scale = 1.5;
                let r = page.rect;
                let (w, h) = ((r.width() * scale) as u32, (r.height() * scale) as u32);
                let mut scene = vello::Scene::new();
                let t = kurbo::Affine::scale(scale) * kurbo::Affine::translate((-r.x0, -r.y0));
                paint::paint(&mut scene, &layout, t, r);
                let img = gpu.render(&scene, w, h).unwrap();
                let file = format!(
                    "{}/../../target/template-{}-{}.png",
                    env!("CARGO_MANIFEST_DIR"),
                    name.replace(' ', "-"),
                    n + 1
                );
                std::fs::write(file, img.to_png()).unwrap();
            }
        }
    }
}
