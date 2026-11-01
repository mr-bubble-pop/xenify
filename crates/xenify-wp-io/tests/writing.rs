//! Checklists and first-line / hanging indents survive DOCX, Markdown and
//! HTML.
use xenify_wp_doc::{Document, Editor, ListDef, ParaProps, Pos, Selection, lists, ops};
use xenify_wp_io::{Format, load, save};

fn build() -> Document {
    let mut ed = Editor::new(Document::from_plain_text("Buy milk\nCall Sam\nIndented first line\nHanging indent"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection { anchor: Pos::new(ids[0], 0), focus: Pos::new(ids[1], 0) };
    ops::toggle_list(&mut ed, ListDef::CHECKLIST).unwrap();
    ops::toggle_checked(&mut ed, ids[1]).unwrap();
    ed.sel = Selection::caret(Pos::new(ids[2], 0));
    ops::apply_para_props(&mut ed, &ParaProps { first_line: Some(36.0), ..Default::default() }, "Indent").unwrap();
    ed.sel = Selection::caret(Pos::new(ids[3], 0));
    ops::apply_para_props(
        &mut ed,
        &ParaProps { indent_left: Some(36.0), first_line: Some(-18.0), ..Default::default() },
        "Indent",
    )
    .unwrap();
    ed.doc
}

/// Text, checklist state and indents of each paragraph.
fn shape(d: &Document) -> Vec<String> {
    d.paragraphs()
        .map(|p| {
            let check = p.list.as_ref().map(|l| {
                let kind = d.list_def(&l.list).map(|def| def.level(l.level).kind);
                format!("{:?} checked={}", kind, l.checked)
            });
            format!("{} | {:?} | {:?} {:?}", p.text, check, p.props.indent_left, p.props.first_line)
        })
        .collect()
}

#[test]
fn checklists_and_indents_round_trip() {
    let doc = build();
    let want = shape(&doc);
    assert!(want[0].contains("Some(Check) checked=false"), "{want:?}");
    assert!(want[1].contains("checked=true"), "{want:?}");
    for f in [Format::Docx, Format::Markdown] {
        let back = load(&save(&doc, f).unwrap(), f).unwrap().doc;
        assert_eq!(shape(&back), want, "{f:?}");
        let labels = lists::labels(&back);
        let ids: Vec<_> = back.paragraphs().map(|p| p.id).collect();
        assert_eq!(labels[&ids[0]], lists::UNCHECKED, "{f:?}");
        assert_eq!(labels[&ids[1]], lists::CHECKED, "{f:?}");
    }
    let md = String::from_utf8(save(&doc, Format::Markdown).unwrap()).unwrap();
    assert!(md.contains("- [ ] Buy milk") && md.contains("- [x] Call Sam"), "{md}");
    let html = String::from_utf8(save(&doc, Format::Html).unwrap()).unwrap();
    assert!(html.contains("<input type=\"checkbox\" checked disabled>"), "{html}");
    assert!(html.contains("text-indent:36pt"), "{html}");
}

#[test]
fn reads_task_lists_written_elsewhere() {
    let doc = load(b"- [ ] one\n- [x] two\n  - [ ] nested\n", Format::Markdown).unwrap().doc;
    let s = shape(&doc);
    assert_eq!(s.len(), 3);
    assert!(s.iter().all(|l| l.contains("Some(Check)")), "{s:?}");
    assert!(s[1].contains("checked=true"));
    assert_eq!(doc.paragraphs().nth(2).unwrap().list.as_ref().unwrap().level, 1);
}

#[test]
fn tab_stops_round_trip() {
    use xenify_wp_doc::{TabKind, TabLeader, TabStop};
    let mut ed = Editor::new(Document::from_plain_text("Name\tPrice\tTotal"));
    let stops = vec![
        TabStop { pos: 144.0, kind: TabKind::Center, leader: TabLeader::None },
        TabStop { pos: 288.0, kind: TabKind::Decimal, leader: TabLeader::Dots },
        TabStop { pos: 400.0, kind: TabKind::Right, leader: TabLeader::Line },
    ];
    ops::apply_para_props(&mut ed, &ParaProps { tabs: Some(stops.clone()), ..Default::default() }, "Tabs").unwrap();
    for f in [Format::Docx, Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        let p = back.paragraphs().next().unwrap();
        assert_eq!(p.text, "Name\tPrice\tTotal", "{f:?}");
        assert_eq!(p.props.tabs.as_ref(), Some(&stops), "{f:?}");
    }
}

#[test]
fn columns_and_column_breaks_round_trip() {
    let mut ed = Editor::new(Document::from_plain_text("One\nTwo"));
    let mut page = ed.doc.sections[0].page.clone();
    page.columns = 3;
    page.column_gap = 24.0;
    page.column_line = true;
    let start = ed.doc.sections[0].start;
    ops::set_section(&mut ed, 0, start, page.clone()).unwrap();
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    ops::apply_para_props(&mut ed, &ParaProps { column_break_before: Some(true), ..Default::default() }, "Break")
        .unwrap();
    for f in [Format::Docx, Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        let p = &back.sections[0].page;
        assert_eq!((p.columns, p.column_gap, p.column_line), (3, 24.0, true), "{f:?}");
        let second = back.paragraphs().nth(1).unwrap();
        assert_eq!((second.text.as_str(), second.props.column_break_before), ("Two", Some(true)), "{f:?}");
    }
    let html = String::from_utf8(save(&ed.doc, Format::Html).unwrap()).unwrap();
    assert!(html.contains("column-count:3") && html.contains("column-rule") && html.contains("break-before:column"));
}

#[test]
fn shading_borders_and_page_colour_round_trip() {
    use xenify_wp_doc::{Border, ParaBorders, Rgba};
    let mut ed = Editor::new(Document::from_plain_text("Boxed note\nUnderlined heading"));
    let ids: Vec<_> = ed.doc.paragraphs().map(|p| p.id).collect();
    let line = Border { width: 1.5, color: Rgba::rgb(0x1f, 0x6f, 0xd1) };
    ed.sel = Selection::caret(Pos::new(ids[0], 0));
    let boxed = ParaProps {
        shading: Some(Rgba::rgb(0xfd, 0xf6, 0xe3)),
        borders: Some(ParaBorders::all(line)),
        ..Default::default()
    };
    ops::apply_para_props(&mut ed, &boxed, "Box").unwrap();
    ed.sel = Selection::caret(Pos::new(ids[1], 0));
    let under =
        ParaProps { borders: Some(ParaBorders { bottom: Some(line), ..Default::default() }), ..Default::default() };
    ops::apply_para_props(&mut ed, &under, "Line").unwrap();
    let mut meta = ed.doc.meta.clone();
    meta.page_color = Some(Rgba::rgb(0xf4, 0xf1, 0xea));
    ops::set_meta(&mut ed, meta, "Page colour").unwrap();
    for f in [Format::Docx, Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        let ps: Vec<_> = back.paragraphs().map(|p| (p.props.shading, p.props.borders)).collect();
        assert_eq!(ps[0], (boxed.shading, boxed.borders), "{f:?}");
        assert_eq!(ps[1], (None, under.borders), "{f:?}");
        assert_eq!(back.meta.page_color, Some(Rgba::rgb(0xf4, 0xf1, 0xea)), "{f:?}");
    }
    let html = String::from_utf8(save(&ed.doc, Format::Html).unwrap()).unwrap();
    assert!(html.contains("border-bottom:1.5pt solid") && html.contains("background:#fdf6e3"), "{html}");
}

#[test]
fn picture_borders_round_trip() {
    use xenify_wp_doc::{Block, Border, Rgba, tables};
    let png = {
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]));
        let mut v = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
        v
    };
    let mut ed = Editor::new(Document::from_plain_text("Before"));
    ed.doc.assets.insert("media/dot.png".into(), std::sync::Arc::new(png));
    tables::insert_image(&mut ed, "media/dot.png", 120.0, 60.0).unwrap();
    let id = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Image(_))).unwrap().id();
    let Some(Block::Image(mut img)) = ed.doc.block(id).cloned() else { panic!() };
    img.border = Some(Border { width: 2.0, color: Rgba::rgb(0x40, 0x40, 0x40) });
    tables::replace_block(&mut ed, id, Block::Image(img), "Border").unwrap();
    for f in [Format::Docx, Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        let border = back.sections[0].blocks.iter().find_map(|b| match b {
            Block::Image(i) => Some(i.border),
            _ => None,
        });
        assert_eq!(border, Some(Some(Border { width: 2.0, color: Rgba::rgb(0x40, 0x40, 0x40) })), "{f:?}");
    }
}

#[test]
fn rich_tables_round_trip() {
    use xenify_wp_doc::{Block, Border, ParaBorders, Rgba, VAlign, tables};
    let mut ed = Editor::new(Document::from_plain_text(""));
    tables::insert_table(&mut ed, 4, 3).unwrap();
    let id = ed.doc.sections[0].blocks.iter().find(|b| matches!(b, Block::Table(_))).unwrap().id();
    let line = Border { width: 1.5, color: Rgba::rgb(0xc6, 0x28, 0x28) };
    let Some(Block::Table(mut t)) = ed.doc.block(id).cloned() else { panic!() };
    t.rows[0].header = true;
    t.header_col = true;
    t.header_fill = Some(Rgba::rgb(0xf1, 0xf3, 0xf5));
    t.band = Some(Rgba::rgb(0xf4, 0xf7, 0xfb));
    t.rows[1].cells[0].col_span = 2;
    t.rows[2].cells[2].row_span = 2;
    t.rows[1].cells[2].shading = Some(Rgba::rgb(0xff, 0xe0, 0x82));
    t.rows[1].cells[2].v_align = VAlign::Middle;
    t.rows[3].cells[0].borders = Some(ParaBorders { bottom: Some(line), ..Default::default() });
    t.rows[2].min_height = Some(36.0);
    let mut next = 10_000;
    t.normalize(&mut || {
        next += 1;
        xenify_wp_doc::NodeId(next)
    });
    tables::replace_block(&mut ed, id, Block::Table(t), "Rich").unwrap();
    let want = ed.doc.table(id).unwrap().clone();
    let shape = |t: &xenify_wp_doc::Table| {
        t.rows
            .iter()
            .map(|r| {
                let cells: Vec<String> = r
                    .cells
                    .iter()
                    .map(|c| {
                        format!(
                            "{}{}x{} {:?} {:?} {:?}",
                            if c.covered { "-" } else { "" },
                            c.col_span,
                            c.row_span,
                            c.shading,
                            c.v_align,
                            c.borders
                        )
                    })
                    .collect();
                format!("h={} min={:?} {}", r.header, r.min_height, cells.join(" | "))
            })
            .collect::<Vec<_>>()
    };
    for f in [Format::Markdown, Format::Igdoc] {
        let back = load(&save(&ed.doc, f).unwrap(), f).unwrap().doc;
        let t = back
            .sections
            .iter()
            .flat_map(|s| &s.blocks)
            .find_map(|b| if let Block::Table(t) = b { Some(t) } else { None })
            .unwrap();
        assert_eq!(shape(t), shape(&want), "{f:?}");
        assert_eq!((t.header_col, t.header_fill, t.band), (want.header_col, want.header_fill, want.band), "{f:?}");
    }
    // DOCX: the same structure.
    let back = load(&save(&ed.doc, Format::Docx).unwrap(), Format::Docx).unwrap().doc;
    let t = back
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| if let Block::Table(t) = b { Some(t) } else { None })
        .unwrap();
    assert!(t.header_col && t.rows[0].header);
    assert_eq!((t.rows[1].cells[0].col_span, t.rows[2].cells[2].row_span), (2, 2));
    assert_eq!(t.rows[1].cells[2].v_align, VAlign::Middle);
    assert_eq!(t.rows[3].cells[0].borders.and_then(|b| b.bottom), Some(line));
    assert_eq!(t.rows[2].min_height, Some(36.0));
    // Word holds them as the cells' colours; they come back as the table's settings.
    assert_eq!(t.band, Some(Rgba::rgb(0xf4, 0xf7, 0xfb)), "second body row is banded");
    assert_eq!(t.header_fill, Some(Rgba::rgb(0xf1, 0xf3, 0xf5)), "header colour");
    assert_eq!((t.rows[2].cells[1].shading, t.rows[0].cells[1].shading), (None, None));
}

#[test]
fn superscripts_and_subscripts_round_trip() {
    use xenify_wp_doc::{CharFmt, VertAlign};
    let mut ed = Editor::new(Document::from_plain_text("x"));
    let id = ed.doc.paragraphs().next().unwrap().id;
    ed.sel = Selection::caret(Pos::new(id, 1));
    ops::insert_text(&mut ed, "th note", Some(&CharFmt { vert: Some(VertAlign::Superscript), ..Default::default() }))
        .unwrap();
    ops::insert_text(&mut ed, " CO", Some(&CharFmt::default())).unwrap();
    ops::insert_text(&mut ed, "2", Some(&CharFmt { vert: Some(VertAlign::Subscript), ..Default::default() })).unwrap();
    let shape = |d: &Document| {
        let p = d.paragraphs().next().unwrap();
        p.run_ranges().map(|(r, f)| (p.text[r].to_string(), f.vert)).collect::<Vec<_>>()
    };
    let want = shape(&ed.doc);
    for format in [Format::Markdown, Format::Docx, Format::Igdoc] {
        let bytes = save(&ed.doc, format).unwrap();
        assert_eq!(shape(&load(&bytes, format).unwrap().doc), want, "{format:?} {}", String::from_utf8_lossy(&bytes));
    }
    let html = String::from_utf8(save(&ed.doc, Format::Html).unwrap()).unwrap();
    assert!(html.contains("<sup>th note</sup>") && html.contains("<sub>2</sub>"));
}
