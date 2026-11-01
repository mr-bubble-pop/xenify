//! PDF export, drawn from the same layout as the screen.
//!
//! Text is real text: every font used is embedded as a subset (variable
//! fonts instanced at the weight/width used) with a ToUnicode map, so the
//! PDF can be searched and copied from. Headings become bookmarks.

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect as PRect, Ref, Str, TextStr};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};
use std::collections::HashMap;
use std::sync::Arc;
use xenify_wp_doc::{Document, NodeId, Rgba};
use xenify_wp_layout::{DocLayout, Fragment, Item, Page};

/// One embedded font: a font file at one set of variation coordinates.
struct FontUse {
    data: Arc<Vec<u8>>,
    index: u32,
    coords: Vec<i16>,
    remap: subsetter::GlyphRemapper,
    /// New glyph id → the text it stands for.
    text: HashMap<u16, String>,
    name: String,
}

struct Ids {
    next: i32,
}

impl Ids {
    fn next(&mut self) -> Ref {
        self.next += 1;
        Ref::new(self.next)
    }
}

fn rgb(c: Rgba) -> (f32, f32, f32) {
    (c.0[0] as f32 / 255.0, c.0[1] as f32 / 255.0, c.0[2] as f32 / 255.0)
}

/// Normalized variation coordinates → user-space axis values (undoing `avar`).
fn user_coords(font: &FontRef, coords: &[i16]) -> Vec<(skrifa::Tag, f32)> {
    let axes = font.axes();
    let avar = font.avar().ok();
    let mut out = Vec::new();
    for (i, axis) in axes.iter().enumerate() {
        let mut n = coords.get(i).copied().unwrap_or(0) as f32 / 16384.0;
        // Invert the axis's avar segment map (piecewise linear).
        if let Some(avar) = &avar
            && let Some(Ok(seg)) = avar.axis_segment_maps().get(i)
        {
            let maps: Vec<(f32, f32)> = seg
                .axis_value_maps()
                .iter()
                .map(|m| (m.from_coordinate().to_f32(), m.to_coordinate().to_f32()))
                .collect();
            if maps.len() >= 2 {
                for w in maps.windows(2) {
                    let ((f0, t0), (f1, t1)) = (w[0], w[1]);
                    if n >= t0 && n <= t1 {
                        n = if (t1 - t0).abs() < 1e-6 { f0 } else { f0 + (n - t0) / (t1 - t0) * (f1 - f0) };
                        break;
                    }
                }
            }
        }
        let (min, def, max) = (axis.min_value(), axis.default_value(), axis.max_value());
        let v = if n >= 0.0 { def + n * (max - def) } else { def + n * (def - min) };
        out.push((axis.tag(), v));
    }
    out
}

/// Draws the document into a PDF. `image` gives a picture's file bytes.
pub fn write(doc: &Document, layout: &DocLayout, image: &dyn Fn(&str) -> Option<Arc<Vec<u8>>>) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut ids = Ids { next: 0 };
    let catalog = ids.next();
    let tree = ids.next();
    let page_refs: Vec<Ref> = layout.pages.iter().map(|_| ids.next()).collect();
    let mut fonts: Vec<FontUse> = Vec::new();
    let mut font_index: HashMap<(u64, u32, Vec<i16>), usize> = HashMap::new();
    let mut images: HashMap<String, Option<(Ref, u32, u32)>> = HashMap::new();
    // (object, pixels, alpha mask, width, height)
    #[allow(clippy::type_complexity)]
    let mut image_objects: Vec<(Ref, Vec<u8>, Option<Vec<u8>>, u32, u32)> = Vec::new();
    // Heading id → (page index, y from the page top) for bookmarks and links.
    let mut heading_at: HashMap<NodeId, (usize, f64)> = HashMap::new();
    let mut page_contents = Vec::new();

    for (pi, page) in layout.pages.iter().enumerate() {
        let on = |r: kurbo::Rect| r.center().y >= page.rect.y0 && r.center().y <= page.rect.y1;
        let mut c = Content::new();
        let h = page.rect.height();
        let tx = |x: f64| (x - page.rect.x0) as f32;
        let ty = |y: f64| (h - (y - page.rect.y0)) as f32;
        let fill_rect = |c: &mut Content, r: kurbo::Rect, color: Rgba| {
            let (cr, cg, cb) = rgb(color);
            c.set_fill_rgb(cr, cg, cb);
            c.rect(tx(r.x0), ty(r.y1), r.width() as f32, r.height() as f32);
            c.fill_nonzero();
        };
        if let Some(paper) = layout.page_color {
            fill_rect(&mut c, page.rect, paper);
        }
        for s in layout.shapes.iter().filter(|s| !s.front && on(s.rect)) {
            fill_rect(&mut c, s.rect, s.color);
        }
        let mut xobjects: Vec<(String, Ref)> = Vec::new();
        for img in layout.images.iter().filter(|i| on(i.rect)) {
            let entry = images.entry(img.src.clone()).or_insert_with(|| {
                let bytes = image(&img.src)?;
                let decoded = ::image::load_from_memory(&bytes).ok()?.to_rgba8();
                let (w, hh) = decoded.dimensions();
                let mut rgb_bytes = Vec::with_capacity((w * hh * 3) as usize);
                let mut alpha = Vec::with_capacity((w * hh) as usize);
                for p in decoded.pixels() {
                    rgb_bytes.extend_from_slice(&p.0[..3]);
                    alpha.push(p.0[3]);
                }
                let r = ids.next();
                let mask = alpha.iter().any(|a| *a < 255).then_some(alpha);
                image_objects.push((r, rgb_bytes, mask, w, hh));
                Some((r, w, hh))
            });
            if let Some((r, _, _)) = entry {
                let name = format!("Im{}", xobjects.len());
                c.save_state();
                c.transform([
                    img.rect.width() as f32,
                    0.0,
                    0.0,
                    img.rect.height() as f32,
                    tx(img.rect.x0),
                    ty(img.rect.y1),
                ]);
                c.x_object(Name(name.as_bytes()));
                c.restore_state();
                xobjects.push((name, *r));
            }
        }
        for s in layout.shapes.iter().filter(|s| s.front && on(s.rect)) {
            fill_rect(&mut c, s.rect, s.color);
        }
        let frags: Vec<&Fragment> = layout.fragments.iter().chain(&layout.decor).filter(|f| on(f.rect())).collect();
        let mut used_fonts: Vec<usize> = Vec::new();
        for f in &frags {
            if f.para.id != NodeId(0) && f.lines.start == 0 {
                heading_at.entry(f.para.id).or_insert((pi, f.rect().y0 - page.rect.y0));
            }
            for item in f.para.items.iter().filter(|it| f.lines.contains(&it.line())) {
                match item {
                    Item::Rect { rect, color, .. } => fill_rect(&mut c, *rect + f.origin.to_vec2(), *color),
                    Item::Emoji { rect, glyph, .. } => {
                        // The same picture as on screen, sharp enough to print.
                        let entry = images.entry(format!("emoji:{glyph}")).or_insert_with(|| {
                            let e = xenify_wp_layout::emoji::find(glyph)?;
                            let px = 160;
                            let rgba = xenify_wp_layout::emoji::picture(e, px)?;
                            let mut rgb_bytes = Vec::with_capacity((px * px * 3) as usize);
                            let mut alpha = Vec::with_capacity((px * px) as usize);
                            for p in rgba.as_chunks::<4>().0 {
                                // Premultiplied → straight colour.
                                let a = p[3];
                                let un = |v: u8| {
                                    if a == 0 { 0 } else { ((v as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 }
                                };
                                rgb_bytes.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2])]);
                                alpha.push(a);
                            }
                            let r = ids.next();
                            image_objects.push((r, rgb_bytes, Some(alpha), px, px));
                            Some((r, px, px))
                        });
                        if let Some((r, _, _)) = entry {
                            let rect = *rect + f.origin.to_vec2();
                            let name = format!("Im{}", xobjects.len());
                            c.save_state();
                            c.transform([
                                rect.width() as f32,
                                0.0,
                                0.0,
                                rect.height() as f32,
                                tx(rect.x0),
                                ty(rect.y1),
                            ]);
                            c.x_object(Name(name.as_bytes()));
                            c.restore_state();
                            xobjects.push((name, *r));
                        }
                    }
                    Item::Glyphs { font, size, coords, glyphs, color, skew, chars, .. } => {
                        let key = (font.data.id(), font.index, coords.clone());
                        let fi = *font_index.entry(key).or_insert_with(|| {
                            fonts.push(FontUse {
                                data: Arc::new(font.data.data().to_vec()),
                                index: font.index,
                                coords: coords.clone(),
                                remap: subsetter::GlyphRemapper::new(),
                                text: HashMap::new(),
                                name: String::new(),
                            });
                            fonts.len() - 1
                        });
                        if !used_fonts.contains(&fi) {
                            used_fonts.push(fi);
                        }
                        let fu = &mut fonts[fi];
                        let (cr, cg, cb) = rgb(*color);
                        c.set_fill_rgb(cr, cg, cb);
                        c.begin_text();
                        c.set_font(Name(format!("F{fi}").as_bytes()), *size);
                        let shear = skew.map_or(0.0, |d| (d.to_radians()).tan());
                        // One positioned glyph at a time: exact placement from the layout.
                        for (g, text) in glyphs.iter().zip(chars.iter().chain(std::iter::repeat(&String::new()))) {
                            let gid = fu.remap.remap(g.id as u16);
                            if !text.is_empty() {
                                fu.text.entry(gid).or_insert_with(|| text.clone());
                            }
                            let x = tx(f.origin.x + g.x as f64);
                            let y = ty(f.origin.y + g.y as f64);
                            c.set_text_matrix([1.0, 0.0, shear, 1.0, x, y]);
                            c.show(Str(&gid.to_be_bytes()));
                        }
                        c.end_text();
                    }
                }
            }
        }
        page_contents.push((c.finish(), used_fonts, xobjects));
    }

    // Fonts: subset (and instance) each, with widths and a ToUnicode map.
    let mut font_refs = Vec::new();
    for (i, fu) in fonts.iter_mut().enumerate() {
        let type0 = ids.next();
        let cid = ids.next();
        let desc = ids.next();
        let file = ids.next();
        let cmap_ref = ids.next();
        font_refs.push(type0);
        let Ok(face) = FontRef::from_index(&fu.data, fu.index) else { continue };
        let vars = user_coords(&face, &fu.coords);
        let subset = if vars.is_empty() {
            subsetter::subset(&fu.data, fu.index, &fu.remap)
        } else {
            let vars: Vec<(subsetter::Tag, f32)> =
                vars.iter().map(|(t, v)| (subsetter::Tag::new(&t.to_be_bytes()), *v)).collect();
            subsetter::subset_with_variations(&fu.data, fu.index, &vars, &fu.remap)
        };
        let Ok(subset) = subset else { continue };
        let sub = FontRef::new(&subset).ok();
        let upem = sub.as_ref().and_then(|f| f.head().ok()).map_or(1000.0, |h| h.units_per_em() as f32);
        let scale = 1000.0 / upem;
        let base = face
            .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
            .english_or_first()
            .map_or_else(|| format!("Font{i}"), |s| s.chars().filter(|c| c.is_ascii_graphic()).collect());
        // A subset tag (six capital letters) keeps viewers from mixing up subsets.
        let tag: String = (0..6).map(|k| (b'A' + ((i * 7 + k * 3) % 26) as u8) as char).collect();
        fu.name = format!("{tag}+{base}");
        let count = fu.remap.remapped_gids().count() as u16;
        let widths: Vec<f32> = match &sub {
            Some(sf) => {
                let gm = sf.glyph_metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
                (0..count).map(|g| gm.advance_width(skrifa::GlyphId::new(g as u32)).unwrap_or(0.0) * scale).collect()
            }
            None => vec![0.0; count as usize],
        };
        let metrics = sub
            .as_ref()
            .map(|sf| sf.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default()));
        let (asc, desc_v, cap) = metrics.as_ref().map_or((800.0, -200.0, 700.0), |m| {
            (m.ascent * scale, m.descent * scale, m.cap_height.unwrap_or(m.ascent * 0.7) * scale)
        });
        let sys = SystemInfo { registry: Str(b"Adobe"), ordering: Str(b"Identity"), supplement: 0 };
        pdf.type0_font(type0)
            .base_font(Name(fu.name.as_bytes()))
            .encoding_predefined(Name(b"Identity-H"))
            .descendant_font(cid)
            .to_unicode(cmap_ref);
        // Outlines in a CFF table (OpenType .otf fonts) embed differently from TrueType ones.
        let cff = sub.as_ref().is_some_and(|f| {
            f.table_data(skrifa::raw::types::Tag::new(b"CFF ")).is_some()
                || f.table_data(skrifa::raw::types::Tag::new(b"CFF2")).is_some()
        });
        let mut cf = pdf.cid_font(cid);
        cf.subtype(if cff { CidFontType::Type0 } else { CidFontType::Type2 })
            .base_font(Name(fu.name.as_bytes()))
            .system_info(sys)
            .font_descriptor(desc)
            .default_width(0.0);
        if !cff {
            cf.cid_to_gid_map_predefined(Name(b"Identity"));
        }
        cf.widths().consecutive(0, widths.iter().copied());
        cf.finish();
        let mut fd = pdf.font_descriptor(desc);
        fd.name(Name(fu.name.as_bytes()))
            .flags(FontFlags::NON_SYMBOLIC)
            .bbox(PRect::new(-500.0, desc_v - 200.0, 1500.0, asc + 200.0))
            .italic_angle(0.0)
            .ascent(asc)
            .descent(desc_v)
            .cap_height(cap)
            .stem_v(80.0);
        if cff {
            fd.font_file3(file);
        } else {
            fd.font_file2(file);
        }
        fd.finish();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&subset, 6);
        if cff {
            pdf.stream(file, &packed).filter(Filter::FlateDecode).pair(Name(b"Subtype"), Name(b"OpenType"));
        } else {
            pdf.stream(file, &packed).filter(Filter::FlateDecode).pair(Name(b"Length1"), subset.len() as i32);
        }
        let mut cmap = UnicodeCmap::new(Name(b"Custom"), sys);
        let mut pairs: Vec<(&u16, &String)> = fu.text.iter().collect();
        pairs.sort();
        for (gid, text) in pairs {
            cmap.pair_with_multiple(*gid, text.chars());
        }
        pdf.cmap(cmap_ref, &cmap.finish());
    }

    for (r, rgb_bytes, mask, w, h) in &image_objects {
        let smask = mask.as_ref().map(|m| {
            let mr = ids.next();
            let packed = miniz_oxide::deflate::compress_to_vec_zlib(m, 6);
            let mut x = pdf.image_xobject(mr, &packed);
            x.width(*w as i32).height(*h as i32).color_space_name(Name(b"DeviceGray")).bits_per_component(8);
            x.filter(Filter::FlateDecode);
            x.finish();
            mr
        });
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(rgb_bytes, 6);
        let mut x = pdf.image_xobject(*r, &packed);
        x.width(*w as i32).height(*h as i32).color_space_name(Name(b"DeviceRGB")).bits_per_component(8);
        x.filter(Filter::FlateDecode);
        if let Some(m) = smask {
            x.s_mask(m);
        }
        x.finish();
    }

    // Pages, with links from contents entries to their headings.
    for (pi, (page, (content, used, xobjects))) in layout.pages.iter().zip(page_contents).enumerate() {
        let content_ref = ids.next();
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&content, 6);
        pdf.stream(content_ref, &packed).filter(Filter::FlateDecode);
        let links = contents_links(layout, page, &heading_at, &page_refs);
        let web = text_links(doc, layout, page, &page_refs);
        let annot_refs: Vec<Ref> = links.iter().map(|_| ids.next()).collect();
        let web_refs: Vec<Ref> = web.iter().map(|_| ids.next()).collect();
        let mut p = pdf.page(page_refs[pi]);
        p.media_box(PRect::new(0.0, 0.0, page.rect.width() as f32, page.rect.height() as f32))
            .parent(tree)
            .contents(content_ref);
        if !annot_refs.is_empty() || !web_refs.is_empty() {
            p.annotations(annot_refs.iter().chain(&web_refs).copied());
        }
        let mut res = p.resources();
        let mut fd = res.fonts();
        for fi in used {
            fd.pair(Name(format!("F{fi}").as_bytes()), font_refs[fi]);
        }
        fd.finish();
        let mut xd = res.x_objects();
        for (name, r) in &xobjects {
            xd.pair(Name(name.as_bytes()), *r);
        }
        xd.finish();
        res.finish();
        p.finish();
        for ((rect, target, top), r) in links.iter().zip(&annot_refs) {
            let mut a = pdf.annotation(*r);
            a.subtype(pdf_writer::types::AnnotationType::Link).rect(*rect).border(0.0, 0.0, 0.0, None);
            a.action()
                .action_type(pdf_writer::types::ActionType::GoTo)
                .destination()
                .page(*target)
                .xyz(0.0, *top, None);
        }
        for ((rect, target), r) in web.iter().zip(&web_refs) {
            let mut a = pdf.annotation(*r);
            a.subtype(pdf_writer::types::AnnotationType::Link).rect(*rect).border(0.0, 0.0, 0.0, None);
            let mut act = a.action();
            match target {
                LinkTarget::Uri(u) => {
                    act.action_type(pdf_writer::types::ActionType::Uri).uri(pdf_writer::Str(u.as_bytes()));
                }
                LinkTarget::Page(pg, top) => {
                    act.action_type(pdf_writer::types::ActionType::GoTo).destination().page(*pg).xyz(0.0, *top, None);
                }
            }
        }
    }
    pdf.pages(tree).kids(page_refs.iter().copied()).count(page_refs.len() as i32);

    // Bookmarks from the headings.
    let outline = write_outline(doc, &mut pdf, &mut ids, layout, &heading_at, &page_refs);
    let mut cat = pdf.catalog(catalog);
    cat.pages(tree);
    if let Some(o) = outline {
        cat.outlines(o);
    }
    cat.finish();
    let info = ids.next();
    let mut di = pdf.document_info(info);
    if !doc.meta.title.is_empty() {
        di.title(TextStr(&doc.meta.title));
    }
    if !doc.meta.author.is_empty() {
        di.author(TextStr(&doc.meta.author));
    }
    di.creator(TextStr("Xenify Write"));
    di.finish();
    pdf.finish()
}

/// Where a link in the text goes.
enum LinkTarget {
    Uri(String),
    /// A page, and the top of the target on it (PDF space).
    Page(Ref, f32),
}

/// Link areas of the text's own links on `page`.
fn text_links(doc: &Document, layout: &DocLayout, page: &Page, page_refs: &[Ref]) -> Vec<(PRect, LinkTarget)> {
    let h = page.rect.height();
    let mut out = Vec::new();
    for p in doc.paragraphs() {
        if !p.runs.iter().any(|r| r.fmt.link.is_some()) {
            continue;
        }
        for (range, fmt) in p.run_ranges() {
            let Some(url) = &fmt.link else { continue };
            let target = match url.strip_prefix("#p").and_then(|n| n.parse::<u64>().ok()) {
                Some(id) => {
                    let Some(&fi) = layout.by_para.get(&NodeId(id)).and_then(|v| v.first()) else { continue };
                    let top = layout.fragments[fi].rect().y0;
                    let Some(tp) = layout.pages.iter().position(|pg| top >= pg.rect.y0 && top <= pg.rect.y1) else {
                        continue;
                    };
                    LinkTarget::Page(
                        page_refs[tp],
                        (layout.pages[tp].rect.height() - (top - layout.pages[tp].rect.y0)) as f32,
                    )
                }
                None if url.starts_with('#') => continue,
                None => LinkTarget::Uri(url.clone()),
            };
            for r in layout.range_rects(p.id, range.clone()) {
                let c = r.center();
                if c.y < page.rect.y0 || c.y > page.rect.y1 {
                    continue;
                }
                let rect = PRect::new(
                    (r.x0 - page.rect.x0) as f32,
                    (h - (r.y1 - page.rect.y0)) as f32,
                    (r.x1 - page.rect.x0) as f32,
                    (h - (r.y0 - page.rect.y0)) as f32,
                );
                let t = match &target {
                    LinkTarget::Uri(u) => LinkTarget::Uri(u.clone()),
                    LinkTarget::Page(r, t) => LinkTarget::Page(*r, *t),
                };
                out.push((rect, t));
            }
        }
    }
    out
}

/// Link areas of contents entries on `page`: (rect in PDF space, target page, target top).
fn contents_links(
    layout: &DocLayout,
    page: &Page,
    heading_at: &HashMap<NodeId, (usize, f64)>,
    page_refs: &[Ref],
) -> Vec<(PRect, Ref, f32)> {
    let h = page.rect.height();
    layout
        .fragments
        .iter()
        .filter(|f| f.para.link.is_some())
        .filter(|f| {
            let c = f.rect().center();
            c.y >= page.rect.y0 && c.y <= page.rect.y1
        })
        .filter_map(|f| {
            let (tp, top) = *heading_at.get(&f.para.link?)?;
            let r = f.rect();
            let rect = PRect::new(
                (r.x0 - page.rect.x0) as f32,
                (h - (r.y1 - page.rect.y0)) as f32,
                (r.x1 - page.rect.x0) as f32,
                (h - (r.y0 - page.rect.y0)) as f32,
            );
            let target_h = layout.pages[tp].rect.height();
            Some((rect, page_refs[tp], (target_h - top) as f32))
        })
        .collect()
}

fn write_outline(
    doc: &Document,
    pdf: &mut Pdf,
    ids: &mut Ids,
    layout: &DocLayout,
    heading_at: &HashMap<NodeId, (usize, f64)>,
    page_refs: &[Ref],
) -> Option<Ref> {
    // (level, title, page, top) for every heading placed.
    let heads: Vec<(u8, String, usize, f64)> = doc
        .paragraphs()
        .filter_map(|p| {
            let level = doc.styles.resolve(&p.style).outline_level?;
            let (pg, top) = *heading_at.get(&p.id)?;
            (!p.text.trim().is_empty()).then(|| (level, p.text.clone(), pg, top))
        })
        .collect();
    if heads.is_empty() {
        return None;
    }
    let root = ids.next();
    let refs: Vec<Ref> = heads.iter().map(|_| ids.next()).collect();
    // Parent of each item: the nearest earlier item with a lower level.
    let parent: Vec<Option<usize>> =
        (0..heads.len()).map(|i| (0..i).rev().find(|&j| heads[j].0 < heads[i].0)).collect();
    let children = |p: Option<usize>| -> Vec<usize> { (0..heads.len()).filter(|&i| parent[i] == p).collect() };
    for (i, (_, title, pg, top)) in heads.iter().enumerate() {
        let sibs = children(parent[i]);
        let k = sibs.iter().position(|&s| s == i).unwrap_or(0);
        let kids = children(Some(i));
        let mut it = pdf.outline_item(refs[i]);
        it.title(TextStr(title)).parent(parent[i].map_or(root, |p| refs[p]));
        if k > 0 {
            it.prev(refs[sibs[k - 1]]);
        }
        if let Some(&n) = sibs.get(k + 1) {
            it.next(refs[n]);
        }
        if let (Some(&f), Some(&l)) = (kids.first(), kids.last()) {
            it.first(refs[f]).last(refs[l]).count(-(kids.len() as i32));
        }
        let h = layout.pages[*pg].rect.height();
        it.dest().page(page_refs[*pg]).xyz(0.0, (h - top) as f32, None);
        it.finish();
    }
    let top = children(None);
    pdf.outline(root).first(refs[top[0]]).last(refs[*top.last().expect("non-empty")]).count(top.len() as i32);
    Some(root)
}
