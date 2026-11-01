//! Document → .docx (Office Open XML, ECMA-376).

use crate::zip::ZipWriter;
use std::collections::HashMap;
use std::fmt::Write;
use xenify_wp_doc::{
    Align, Block, CharFmt, Document, HfLine, NumberKind, PageSetup, ParaProps, ParaStyle, Paragraph, Rgba,
    SectionStart, StyleSheet, Table,
};

/// Twentieths of a point (twips).
fn tw(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

/// Half points (font sizes).
fn hp(pt: f32) -> i64 {
    (pt * 2.0).round() as i64
}

/// EMUs (images): 12700 per point.
fn emu(pt: f32) -> i64 {
    (pt * 12700.0).round() as i64
}

pub(super) fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn hex(c: Rgba) -> String {
    c.to_hex().trim_start_matches('#')[..6].to_uppercase()
}

fn jc(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "both",
    }
}

/// `<w:rPr>` for character formatting.
fn rpr(f: &CharFmt) -> String {
    let mut s = String::new();
    if let Some(font) = &f.font {
        let font = if font == "monospace" {
            "Consolas"
        } else if font == "serif" {
            "Times New Roman"
        } else {
            font
        };
        let _ = write!(s, "<w:rFonts w:ascii=\"{0}\" w:hAnsi=\"{0}\" w:cs=\"{0}\"/>", esc(font));
    }
    let flag = |s: &mut String, tag: &str, v: Option<bool>| {
        if let Some(v) = v {
            let _ = write!(s, "<w:{tag}{}/>", if v { "" } else { " w:val=\"0\"" });
        }
    };
    flag(&mut s, "b", f.bold);
    flag(&mut s, "i", f.italic);
    flag(&mut s, "strike", f.strike);
    if let Some(c) = f.color {
        let _ = write!(s, "<w:color w:val=\"{}\"/>", hex(c));
    }
    if let Some(sz) = f.size {
        let _ = write!(s, "<w:sz w:val=\"{0}\"/><w:szCs w:val=\"{0}\"/>", hp(sz));
    }
    // Schema order: sz, szCs, highlight, u, …, shd.
    if let Some(u) = f.underline {
        let _ = write!(s, "<w:u w:val=\"{}\"/>", if u { "single" } else { "none" });
    }
    if let Some(h) = f.highlight {
        let _ = write!(s, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(h));
    }
    match f.vert {
        Some(xenify_wp_doc::VertAlign::Superscript) => s.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        Some(xenify_wp_doc::VertAlign::Subscript) => s.push_str("<w:vertAlign w:val=\"subscript\"/>"),
        _ => {}
    }
    if s.is_empty() { s } else { format!("<w:rPr>{s}</w:rPr>") }
}

/// `<w:pPr>` content (without the wrapper) for paragraph properties, in
/// schema order, with `num` (the list numbering) in its place.
/// `frame`, `num` and `boxed` (frame `w:framePr`, list `w:numPr`, and frame
/// border and shading) go in their schema positions.
fn ppr_props(p: &ParaProps, frame: &str, num: &str, boxed: &str) -> String {
    let mut s = String::new();
    if p.keep_with_next == Some(true) {
        s.push_str("<w:keepNext/>");
    }
    if p.keep_together == Some(true) {
        s.push_str("<w:keepLines/>");
    }
    if p.page_break_before == Some(true) {
        s.push_str("<w:pageBreakBefore/>");
    }
    s.push_str(frame);
    if let Some(w) = p.widow_control {
        let _ = write!(s, "<w:widowControl{}/>", if w { "" } else { " w:val=\"0\"" });
    }
    s.push_str(num);
    s.push_str(boxed);
    if boxed.is_empty() {
        // The paragraph's own lines and shading (a frame's come in `boxed`).
        if let Some(b) = p.borders.filter(|b| !b.is_empty()) {
            let e = |side: &str, line: Option<xenify_wp_doc::Border>| {
                line.filter(|l| l.width > 0.0).map_or(String::new(), |l| {
                    format!(
                        "<w:{side} w:val=\"single\" w:sz=\"{}\" w:space=\"4\" w:color=\"{}\"/>",
                        ((l.width * 8.0).round() as i64).max(2),
                        hex(l.color)
                    )
                })
            };
            let _ = write!(
                s,
                "<w:pBdr>{}{}{}{}</w:pBdr>",
                e("top", b.top),
                e("left", b.left),
                e("bottom", b.bottom),
                e("right", b.right)
            );
        }
        if let Some(c) = p.shading.filter(|c| c.0[3] > 0) {
            let _ = write!(s, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(c));
        }
    }
    if let Some(tabs) = p.tabs.as_ref().filter(|t| !t.is_empty()) {
        s.push_str("<w:tabs>");
        for t in tabs {
            let val = match t.kind {
                xenify_wp_doc::TabKind::Left => "left",
                xenify_wp_doc::TabKind::Center => "center",
                xenify_wp_doc::TabKind::Right => "right",
                xenify_wp_doc::TabKind::Decimal => "decimal",
            };
            let leader = match t.leader {
                xenify_wp_doc::TabLeader::None => "",
                xenify_wp_doc::TabLeader::Dots => " w:leader=\"dot\"",
                xenify_wp_doc::TabLeader::Dashes => " w:leader=\"hyphen\"",
                xenify_wp_doc::TabLeader::Line => " w:leader=\"underscore\"",
            };
            let _ = write!(s, "<w:tab w:val=\"{val}\" w:pos=\"{}\"{leader}/>", tw(t.pos));
        }
        s.push_str("</w:tabs>");
    }
    if p.space_before.is_some() || p.space_after.is_some() || p.line_height.is_some() || p.line_exact.is_some() {
        s.push_str("<w:spacing");
        if let Some(v) = p.space_before {
            let _ = write!(s, " w:before=\"{}\"", tw(v));
        }
        if let Some(v) = p.space_after {
            let _ = write!(s, " w:after=\"{}\"", tw(v));
        }
        if let Some(v) = p.line_exact {
            let _ = write!(s, " w:line=\"{}\" w:lineRule=\"exact\"", tw(v));
        } else if let Some(v) = p.line_height {
            let _ = write!(s, " w:line=\"{}\" w:lineRule=\"auto\"", (v * 240.0).round() as i64);
        }
        s.push_str("/>");
    }
    if p.indent_left.is_some() || p.indent_right.is_some() || p.first_line.is_some() {
        s.push_str("<w:ind");
        if let Some(v) = p.indent_left {
            let _ = write!(s, " w:left=\"{}\"", tw(v));
        }
        if let Some(v) = p.indent_right {
            let _ = write!(s, " w:right=\"{}\"", tw(v));
        }
        match p.first_line {
            Some(v) if v < 0.0 => {
                let _ = write!(s, " w:hanging=\"{}\"", tw(-v));
            }
            Some(v) => {
                let _ = write!(s, " w:firstLine=\"{}\"", tw(v));
            }
            None => {}
        }
        s.push_str("/>");
    }
    if let Some(a) = p.align {
        let _ = write!(s, "<w:jc w:val=\"{}\"/>", jc(a));
    }
    s
}

fn style_xml(st: &ParaStyle) -> String {
    let mut s = format!("<w:style w:type=\"paragraph\" w:styleId=\"{}\">", esc(&st.id));
    let _ = write!(s, "<w:name w:val=\"{}\"/>", esc(&docx_style_name(st)));
    if let Some(b) = &st.based_on {
        let _ = write!(s, "<w:basedOn w:val=\"{}\"/>", esc(b));
    }
    if let Some(n) = &st.next {
        let _ = write!(s, "<w:next w:val=\"{}\"/>", esc(n));
    }
    s.push_str("<w:qFormat/>");
    let mut ppr = ppr_props(&st.para, "", "", "");
    if let Some(l) = st.outline_level {
        let _ = write!(ppr, "<w:outlineLvl w:val=\"{}\"/>", l.saturating_sub(1));
    }
    if !ppr.is_empty() {
        let _ = write!(s, "<w:pPr>{ppr}</w:pPr>");
    }
    s.push_str(&rpr(&st.char));
    s.push_str("</w:style>");
    s
}

/// Word's own names for the built-in styles, so they map onto Word's.
fn docx_style_name(st: &ParaStyle) -> String {
    match st.id.as_str() {
        "heading1" | "heading2" | "heading3" | "heading4" | "heading5" | "heading6" => {
            format!("heading {}", &st.id[7..])
        }
        "toc1" | "toc2" | "toc3" => format!("toc {}", &st.id[3..]),
        "normal" => "Normal".into(),
        "footnote-text" => "footnote text".into(),
        _ => st.name.clone(),
    }
}

/// Writes the whole package.
pub fn write(doc: &Document) -> Vec<u8> {
    write_with_pages(doc, &HashMap::new())
}

/// Like [`write`], with each heading's page (from a layout) shown in
/// contents tables, so they look right before Word refreshes them.
pub fn write_with_pages(doc: &Document, pages: &HashMap<xenify_wp_doc::NodeId, u32>) -> Vec<u8> {
    let mut w = Writer { toc_pages: pages.clone(), ..Writer::default() };
    // Word's note ids, in the order the text refers to them; and the paragraphs referred to.
    let (mut foot, mut end) = (0u32, 0u32);
    for p in doc.paragraphs() {
        for run in &p.runs {
            match &run.fmt.field {
                Some(xenify_wp_doc::Field::Note { id }) if !w.note_ids.contains_key(id) => {
                    if let Some(n) = doc.note(*id) {
                        let k = match n.kind {
                            xenify_wp_doc::NoteKind::Foot => {
                                foot += 1;
                                foot
                            }
                            xenify_wp_doc::NoteKind::End => {
                                end += 1;
                                end
                            }
                        };
                        w.note_ids.insert(*id, (n.kind, k));
                    }
                }
                Some(xenify_wp_doc::Field::Ref { target }) => {
                    w.ref_targets.insert(*target);
                }
                _ => {}
            }
            // Links within the document (#p12) point at a bookmark too.
            if let Some(id) = run.fmt.link.as_deref().and_then(internal_target) {
                w.ref_targets.insert(id);
            }
        }
    }
    // Comments: their parts, and where each range starts and ends in the text.
    let comment_xml = comment_parts(doc);
    if let Some((_, _, ids)) = &comment_xml {
        for (cid, start, end) in xenify_wp_doc::review::anchors(doc) {
            let Some((_, word_ids)) = ids.iter().find(|(c, _)| *c == cid) else { continue };
            for wid in word_ids {
                w.comment_marks.entry(start.para).or_default().push((start.offset, true, *wid));
                w.comment_marks.entry(end.para).or_default().push((end.offset, false, *wid));
            }
        }
    }
    let body = w.body(doc);
    // Footnotes and endnotes parts (with the separators Word expects).
    let width = doc.sections.first().map_or(451.0, |s| s.page.content_width());
    let mut note_parts: Vec<(&str, String)> = Vec::new();
    for (kind, tag, part) in [
        (xenify_wp_doc::NoteKind::Foot, "footnote", "footnotes.xml"),
        (xenify_wp_doc::NoteKind::End, "endnote", "endnotes.xml"),
    ] {
        let mut ids: Vec<(u32, xenify_wp_doc::NodeId)> =
            w.note_ids.iter().filter(|(_, (k, _))| *k == kind).map(|(id, (_, n))| (*n, *id)).collect();
        if ids.is_empty() {
            continue;
        }
        ids.sort();
        let mut xml = format!(
            "{HEAD}<w:{tag}s {NS}><w:{tag} w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:{tag}><w:{tag} w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{tag}>"
        );
        for (n, id) in ids {
            if let Some(note) = doc.note(id) {
                let inner = w.note_xml(doc, note, width);
                let _ = write!(xml, "<w:{tag} w:id=\"{n}\">{inner}</w:{tag}>");
            }
        }
        let _ = write!(xml, "</w:{tag}s>");
        note_parts.push((part, xml));
    }
    let mut z = ZipWriter::new();
    let mut ct = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"gif\" ContentType=\"image/gif\"/><Default Extension=\"webp\" ContentType=\"image/webp\"/><Default Extension=\"bmp\" ContentType=\"image/bmp\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    );
    for (name, _) in &w.parts {
        let kind = if name.starts_with("header") { "header" } else { "footer" };
        let _ = write!(
            ct,
            "<Override PartName=\"/word/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/>"
        );
    }
    for (part, _) in &note_parts {
        let kind = part.trim_end_matches(".xml");
        let _ = write!(
            ct,
            "<Override PartName=\"/word/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/>"
        );
    }
    if !doc.meta.changelog.is_empty() {
        ct.push_str("<Override PartName=\"/docProps/custom.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.custom-properties+xml\"/>");
    }
    if comment_xml.is_some() {
        ct.push_str("<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/><Override PartName=\"/word/commentsExtended.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml\"/>");
    }
    if !doc.meta.sources.is_empty() {
        ct.push_str("<Override PartName=\"/customXml/itemProps1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.customXmlProperties+xml\"/>");
    }
    ct.push_str("</Types>");
    z.add("[Content_Types].xml", ct.as_bytes(), true);
    // The change log rides along as a custom document property (JSON).
    let custom = (!doc.meta.changelog.is_empty()).then(|| {
        let json = serde_json::to_string(&doc.meta.changelog).unwrap_or_default();
        format!(
            "{HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><property fmtid=\"{{D5CDD505-2E9C-101B-9397-08002B2CF9AE}}\" pid=\"2\" name=\"XenifyChangeLog\"><vt:lpwstr>{}</vt:lpwstr></property></Properties>",
            esc(&json)
        )
    });
    z.add(
        "_rels/.rels",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>{}</Relationships>",
            if custom.is_some() {
                "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties\" Target=\"docProps/custom.xml\"/>"
            } else {
                ""
            }
        )
        .as_bytes(),
        true,
    );
    if let Some(c) = &custom {
        z.add("docProps/custom.xml", c.as_bytes(), true);
    }
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>{}</dc:creator></cp:coreProperties>",
        esc(&doc.meta.title),
        esc(&doc.meta.author)
    );
    z.add("docProps/core.xml", core.as_bytes(), true);
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rIdNumbering\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>",
    );
    for (rid, target, kind) in &w.rels {
        let _ = write!(
            rels,
            "<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{}\"{}/>",
            esc(target),
            if *kind == "hyperlink" { " TargetMode=\"External\"" } else { "" }
        );
    }
    for (part, _) in &note_parts {
        let kind = part.trim_end_matches(".xml");
        let _ = write!(
            rels,
            "<Relationship Id=\"rId{kind}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{part}\"/>"
        );
    }
    if comment_xml.is_some() {
        rels.push_str("<Relationship Id=\"rIdComments\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"comments.xml\"/><Relationship Id=\"rIdCommentsEx\" Type=\"http://schemas.microsoft.com/office/2011/relationships/commentsExtended\" Target=\"commentsExtended.xml\"/>");
    }
    if !doc.meta.sources.is_empty() {
        rels.push_str("<Relationship Id=\"rIdSources\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../customXml/item1.xml\"/>");
    }
    rels.push_str("</Relationships>");
    z.add("word/_rels/document.xml.rels", rels.as_bytes(), true);
    for (part, xml) in &note_parts {
        z.add(&format!("word/{part}"), xml.as_bytes(), true);
    }
    if let Some((comments, ext, _)) = &comment_xml {
        z.add("word/comments.xml", comments.as_bytes(), true);
        z.add("word/commentsExtended.xml", ext.as_bytes(), true);
    }
    if !doc.meta.sources.is_empty() {
        z.add("customXml/item1.xml", sources_xml(doc).as_bytes(), true);
        z.add(
            "customXml/itemProps1.xml",
            format!("{HEAD}<ds:datastoreItem ds:itemID=\"{{6E0E1B5A-0F55-4F0B-9C39-5A0F3E1F2C11}}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"><ds:schemaRefs><ds:schemaRef ds:uri=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\"/></ds:schemaRefs></ds:datastoreItem>").as_bytes(),
            true,
        );
        z.add(
            "customXml/_rels/item1.xml.rels",
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps\" Target=\"itemProps1.xml\"/></Relationships>",
            true,
        );
    }
    let background = doc
        .meta
        .page_color
        .filter(|c| c.0[3] > 0)
        .map_or(String::new(), |c| format!("<w:background w:color=\"{}\"/>", hex(c)));
    z.add(
        "word/document.xml",
        format!("{HEAD}<w:document {NS}>{background}<w:body>{body}</w:body></w:document>").as_bytes(),
        true,
    );
    z.add("word/styles.xml", styles(doc).as_bytes(), true);
    z.add("word/numbering.xml", numbering(doc).as_bytes(), true);
    z.add(
        "word/settings.xml",
        format!(
            "{HEAD}<w:settings {NS}>{}{}<w:defaultTabStop w:val=\"720\"/>{}<w:updateFields w:val=\"true\"/>{}</w:settings>",
            if doc.meta.page_color.is_some() { "<w:displayBackgroundShape/>" } else { "" },
            if doc.meta.track_changes { "<w:trackRevisions/>" } else { "" },
            if doc.sections.iter().any(|s| s.page.different_odd_even) { "<w:evenAndOddHeaders/>" } else { "" },
            note_parts
                .iter()
                .map(|(part, _)| {
                    let tag = part.trim_end_matches("s.xml");
                    format!("<w:{tag}Pr><w:{tag} w:id=\"-1\"/><w:{tag} w:id=\"0\"/></w:{tag}Pr>")
                })
                .collect::<String>()
        )
        .as_bytes(),
        true,
    );
    for (name, xml) in &w.parts {
        z.add(&format!("word/{name}"), xml.as_bytes(), true);
    }
    for (name, rels) in &w.part_rels {
        z.add(&format!("word/_rels/{name}.rels"), rels.as_bytes(), true);
    }
    for (name, src) in &w.media {
        if let Some(bytes) = doc.assets.get(src) {
            z.add(&format!("word/{name}"), bytes, false);
        }
    }
    z.finish()
}

const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// The paragraph a `#p12` link points at.
fn internal_target(url: &str) -> Option<xenify_wp_doc::NodeId> {
    url.strip_prefix("#p")?.parse().ok().map(xenify_wp_doc::NodeId)
}

/// The document's sources in Word's bibliography format (References › Manage Sources in Word).
fn sources_xml(doc: &Document) -> String {
    use xenify_wp_doc::cite::{CiteStyle, SourceKind};
    let style = match doc.meta.cite_style {
        CiteStyle::Apa => "APASixthEditionOfficeOnline.xsl",
        CiteStyle::Harvard => "HarvardAnglia2008OfficeOnline.xsl",
        CiteStyle::Ieee => "IEEE2006OfficeOnline.xsl",
        CiteStyle::Chicago => "Chicago.xsl",
    };
    let mut s = format!(
        "{HEAD}<b:Sources SelectedStyle=\"\\{style}\" StyleName=\"{}\" xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\" xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\">",
        doc.meta.cite_style.name()
    );
    for (i, src) in doc.meta.sources.iter().enumerate() {
        let kind = match src.kind {
            SourceKind::Book => "Book",
            SourceKind::Article => "JournalArticle",
            SourceKind::Web => "InternetSite",
            SourceKind::Report => "Report",
            SourceKind::Chapter => "BookSection",
            SourceKind::Other => "Misc",
        };
        let _ = write!(
            s,
            "<b:Source><b:Tag>{}</b:Tag><b:SourceType>{kind}</b:SourceType><b:RefOrder>{}</b:RefOrder>",
            esc(&src.key),
            i + 1
        );
        if !src.authors.is_empty() {
            s.push_str("<b:Author><b:Author><b:NameList>");
            for a in &src.authors {
                let _ = write!(
                    s,
                    "<b:Person><b:Last>{}</b:Last><b:First>{}</b:First></b:Person>",
                    esc(&a.family),
                    esc(&a.given)
                );
            }
            s.push_str("</b:NameList></b:Author></b:Author>");
        }
        let container_tag = match src.kind {
            SourceKind::Article => "JournalName",
            SourceKind::Chapter => "BookTitle",
            SourceKind::Web => "InternetSiteTitle",
            _ => "Comments",
        };
        for (tag, v) in [
            ("Title", &src.title),
            ("Year", &src.year),
            ("Publisher", &src.publisher),
            (container_tag, &src.container),
            ("Volume", &src.volume),
            ("Issue", &src.issue),
            ("Pages", &src.pages),
            ("URL", &src.url),
            ("DOI", &src.doi),
        ] {
            if !v.is_empty() {
                let _ = write!(s, "<b:{tag}>{}</b:{tag}>", esc(v));
            }
        }
        s.push_str("</b:Source>");
    }
    s.push_str("</b:Sources>");
    s
}
const NS: &str = "xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\" xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

fn styles(doc: &Document) -> String {
    let base = &doc.styles;
    let mut s = format!(
        "{HEAD}<w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{0}\" w:hAnsi=\"{0}\" w:cs=\"{0}\"/><w:sz w:val=\"{1}\"/><w:szCs w:val=\"{1}\"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>",
        esc(&base.default_font),
        hp(base.default_size)
    );
    for st in &base.paragraph {
        s.push_str(&style_xml(st));
    }
    s.push_str("</w:styles>");
    s
}

fn fmt_for(kind: NumberKind) -> &'static str {
    match kind {
        NumberKind::Bullet => "bullet",
        NumberKind::Decimal => "decimal",
        NumberKind::LowerAlpha => "lowerLetter",
        NumberKind::UpperAlpha => "upperLetter",
        NumberKind::LowerRoman => "lowerRoman",
        NumberKind::UpperRoman => "upperRoman",
        NumberKind::None | NumberKind::Check => "none",
    }
}

/// Numbering definitions: one abstract numbering per list definition, and
/// one instance per list definition (numId = index + 1). Restarts get their
/// own instances with a start override.
fn numbering(doc: &Document) -> String {
    let mut s = format!("{HEAD}<w:numbering {NS}>");
    for (i, def) in doc.lists.iter().enumerate() {
        let _ = write!(s, "<w:abstractNum w:abstractNumId=\"{i}\"><w:multiLevelType w:val=\"multilevel\"/>");
        for lvl in 0..9u8 {
            let l = def.level(lvl);
            let text = l.format.clone();
            let _ = write!(
                s,
                "<w:lvl w:ilvl=\"{lvl}\"><w:start w:val=\"{}\"/><w:numFmt w:val=\"{}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"{}\"/></w:pPr></w:lvl>",
                l.start,
                fmt_for(l.kind),
                esc(&text),
                tw(l.indent),
                tw(l.hanging)
            );
        }
        s.push_str("</w:abstractNum>");
    }
    for (i, _) in doc.lists.iter().enumerate() {
        let _ = write!(s, "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{i}\"/></w:num>", i + 1);
    }
    // Restarted lists: numIds from 1000, one per restart point.
    let mut next = 1000;
    for p in doc.paragraphs() {
        let Some(item) = &p.list else { continue };
        let Some(r) = item.restart else { continue };
        let Some(i) = doc.lists.iter().position(|d| d.id == item.list) else { continue };
        let _ = write!(
            s,
            "<w:num w:numId=\"{next}\"><w:abstractNumId w:val=\"{i}\"/><w:lvlOverride w:ilvl=\"{}\"><w:startOverride w:val=\"{r}\"/></w:lvlOverride></w:num>",
            item.level
        );
        next += 1;
    }
    s.push_str("</w:numbering>");
    s
}

#[derive(Default)]
struct Writer {
    /// Heading pages for contents tables (empty: no numbers).
    toc_pages: HashMap<xenify_wp_doc::NodeId, u32>,
    /// Extra relationships: (id, target, type).
    rels: Vec<(String, String, &'static str)>,
    /// Header/footer parts: (file name, xml).
    parts: Vec<(String, String)>,
    /// Pictures: (part name under word/, asset key).
    media: Vec<(String, String)>,
    media_ids: HashMap<String, String>,
    /// numId currently used for each list (changes at restarts).
    num_ids: HashMap<String, usize>,
    next_restart: usize,
    drawing_id: u32,
    /// While writing a frame's paragraphs: its `w:framePr`, and its border and shading.
    frame: Option<(String, String)>,
    /// Footnote/endnote ids in Word (from 1), by note.
    note_ids: HashMap<xenify_wp_doc::NodeId, (xenify_wp_doc::NoteKind, u32)>,
    /// Paragraphs other text refers to (get a bookmark).
    ref_targets: std::collections::HashSet<xenify_wp_doc::NodeId>,
    /// Where comment ranges start and end: paragraph → (offset, start?, Word comment id).
    comment_marks: HashMap<xenify_wp_doc::NodeId, Vec<(usize, bool, usize)>>,
    /// Tracked change ids written so far.
    change_id: usize,
    /// Relationships of header and footer parts: (part name, rels xml).
    part_rels: Vec<(String, String)>,
}

/// Word's comment parts: comments.xml and commentsExtended.xml (threads and
/// resolved state), with each thread's Word ids (the comment, then its
/// replies).
/// comments.xml, commentsExtended.xml, and each thread's Word comment ids.
type CommentParts = (String, String, Vec<(xenify_wp_doc::NodeId, Vec<usize>)>);

fn comment_parts(doc: &Document) -> Option<CommentParts> {
    if doc.meta.comments.is_empty() {
        return None;
    }
    let mut comments = format!(
        "{HEAD}<w:comments {NS} xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w14\">"
    );
    let mut ext = format!(
        "{HEAD}<w15:commentsEx xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w15\">"
    );
    let mut ids = Vec::new();
    let mut next = 0usize;
    for c in &doc.meta.comments {
        let mut thread = Vec::new();
        let parent_para = format!("{:08X}", 0x1000_0000 + next);
        let entries = std::iter::once((c.author.as_str(), c.date.as_str(), c.text.as_str()))
            .chain(c.replies.iter().map(|r| (r.author.as_str(), r.date.as_str(), r.text.as_str())));
        for (i, (author, date, text)) in entries.enumerate() {
            let para_id = format!("{:08X}", 0x1000_0000 + next);
            let initials: String = author.split_whitespace().filter_map(|w| w.chars().next()).collect();
            let date = if date.is_empty() { String::new() } else { format!(" w:date=\"{}\"", esc(date)) };
            let _ = write!(
                comments,
                "<w:comment w:id=\"{next}\" w:author=\"{}\"{date} w:initials=\"{}\">",
                esc(author),
                esc(&initials)
            );
            let lines: Vec<&str> = text.split('\n').collect();
            for (j, line) in lines.iter().enumerate() {
                // Word links threads by the comment's last paragraph.
                let pid = if j + 1 == lines.len() { format!(" w14:paraId=\"{para_id}\"") } else { String::new() };
                let mark = if j == 0 {
                    "<w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:annotationRef/></w:r>"
                } else {
                    ""
                };
                let _ = write!(
                    comments,
                    "<w:p{pid}><w:pPr><w:pStyle w:val=\"CommentText\"/></w:pPr>{mark}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    esc(line)
                );
            }
            comments.push_str("</w:comment>");
            let parent = if i == 0 { String::new() } else { format!(" w15:paraIdParent=\"{parent_para}\"") };
            let _ = write!(
                ext,
                "<w15:commentEx w15:paraId=\"{para_id}\"{parent} w15:done=\"{}\"/>",
                if c.resolved { 1 } else { 0 }
            );
            thread.push(next);
            next += 1;
        }
        ids.push((c.id, thread));
    }
    comments.push_str("</w:comments>");
    ext.push_str("</w15:commentsEx>");
    Some((comments, ext, ids))
}

impl Writer {
    fn rel(&mut self, target: String, kind: &'static str) -> String {
        if let Some((id, _, _)) = self.rels.iter().find(|(_, t, k)| *t == target && *k == kind) {
            return id.clone();
        }
        let id = format!("rId{}", self.rels.len() + 10);
        self.rels.push((id.clone(), target, kind));
        id
    }

    fn runs(&mut self, doc: &Document, p: &Paragraph) -> String {
        let mut s = String::new();
        let marks = self.comment_marks.get(&p.id).cloned().unwrap_or_default();
        let emit_marks = |s: &mut String, at: usize| {
            for (off, start, id) in &marks {
                if *off != at {
                    continue;
                }
                if *start {
                    let _ = write!(s, "<w:commentRangeStart w:id=\"{id}\"/>");
                } else {
                    let _ = write!(
                        s,
                        "<w:commentRangeEnd w:id=\"{id}\"/><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:commentReference w:id=\"{id}\"/></w:r>"
                    );
                }
            }
        };
        // The form control the runs so far are inside.
        let mut open: Option<xenify_wp_doc::NodeId> = None;
        for (range, fmt) in p.run_ranges() {
            emit_marks(&mut s, range.start);
            if fmt.control != open {
                if open.is_some() {
                    s.push_str("</w:sdtContent></w:sdt>");
                }
                open = fmt.control;
                if let Some(c) = open.and_then(|id| doc.control(id)) {
                    s.push_str(&control_start(c));
                } else {
                    open = None;
                }
            }
            let text = &p.text[range];
            if let Some(field) = &fmt.field {
                let plain = xenify_wp_doc::CharFmt { field: None, ..fmt.clone() };
                for _ in text.matches(xenify_wp_doc::Field::CHAR) {
                    s.push_str(&self.field_xml(doc, field, &plain));
                }
                continue;
            }
            // Deleted text (tracked) is `w:delText`.
            let deleted = fmt.change.as_ref().is_some_and(|c| c.kind == xenify_wp_doc::review::ChangeKind::Deleted);
            let t = if deleted { "delText" } else { "t" };
            // Tabs are their own element.
            let body: String = text
                .split('\t')
                .map(|x| {
                    if x.is_empty() {
                        String::new()
                    } else {
                        format!("<w:{t} xml:space=\"preserve\">{}</w:{t}>", esc(x))
                    }
                })
                .collect::<Vec<_>>()
                .join("<w:tab/>");
            let mut run = format!("<w:r>{}{body}</w:r>", rpr(fmt));
            if let Some(c) = &fmt.change {
                self.change_id += 1;
                let tag = if deleted { "del" } else { "ins" };
                let date = if c.date.is_empty() { String::new() } else { format!(" w:date=\"{}\"", esc(&c.date)) };
                run = format!(
                    "<w:{tag} w:id=\"{}\" w:author=\"{}\"{date}>{run}</w:{tag}>",
                    self.change_id,
                    esc(&c.author)
                );
            }
            match &fmt.link {
                // Inside the document: a bookmark, not a web address.
                Some(url) if url.starts_with('#') => {
                    let anchor =
                        internal_target(url).map_or_else(|| url[1..].to_string(), |id| format!("_Ref{}", id.0));
                    let _ = write!(s, "<w:hyperlink w:anchor=\"{}\">{run}</w:hyperlink>", esc(&anchor));
                }
                Some(url) => {
                    let id = self.rel(url.clone(), "hyperlink");
                    let _ = write!(s, "<w:hyperlink r:id=\"{id}\">{run}</w:hyperlink>");
                }
                None => s.push_str(&run),
            }
        }
        if open.is_some() {
            s.push_str("</w:sdtContent></w:sdt>");
        }
        emit_marks(&mut s, p.text.len());
        s
    }

    /// A field as Word writes it: a note reference, or a field code with
    /// its current result (Word updates it).
    fn field_xml(&mut self, doc: &Document, field: &xenify_wp_doc::Field, fmt: &xenify_wp_doc::CharFmt) -> String {
        use xenify_wp_doc::Field;
        let simple = |instr: &str, result: &str| {
            format!(
                "<w:fldSimple w:instr=\" {} \"><w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:fldSimple>",
                esc(instr),
                rpr(fmt),
                esc(result)
            )
        };
        match field {
            Field::Note { id } => match self.note_ids.get(id) {
                Some((kind, n)) => {
                    let (el, style) = match kind {
                        xenify_wp_doc::NoteKind::Foot => ("footnoteReference", "FootnoteReference"),
                        xenify_wp_doc::NoteKind::End => ("endnoteReference", "EndnoteReference"),
                    };
                    format!(
                        "<w:r><w:rPr><w:rStyle w:val=\"{style}\"/><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{el} w:id=\"{n}\"/></w:r>"
                    )
                }
                None => String::new(),
            },
            Field::Page => simple("PAGE", "1"),
            Field::Pages => simple("NUMPAGES", "1"),
            Field::Date => simple("DATE \\@ \"yyyy-MM-dd\"", ""),
            // A badge: bold capitals on its colour (Word has no badges).
            Field::Badge { text, color } => {
                let (bg, fg) = xenify_wp_doc::badge_colors(color);
                format!(
                    "<w:r><w:rPr><w:b/><w:color w:val=\"{}\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/></w:rPr><w:t xml:space=\"preserve\"> {} </w:t></w:r>",
                    fg.to_hex().trim_start_matches('#'),
                    bg.to_hex().trim_start_matches('#'),
                    esc(&text.to_uppercase())
                )
            }
            // A date that doesn't update is plain text in Word (its DATE field always does).
            Field::DateTime { fixed: Some(text), .. } => {
                format!("<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>", rpr(fmt), esc(text))
            }
            Field::DateTime { format, fixed: None } => simple(
                &format!("DATE \\@ \"{format}\""),
                &xenify_wp_doc::datetime::format(format, &xenify_wp_layout::now()),
            ),
            Field::Title => simple("TITLE", &doc.meta.title),
            Field::Author => simple("AUTHOR", &doc.meta.author),
            Field::Citation { keys, locator } => {
                // Word's own citation field: CITATION key \m key2 \p "page".
                let mut instr = format!("CITATION {}", keys.first().cloned().unwrap_or_default());
                for k in keys.iter().skip(1) {
                    instr.push_str(&format!(" \\m {k}"));
                }
                if !locator.is_empty() {
                    instr.push_str(&format!(
                        " \\p \"{}\"",
                        locator.trim_start_matches("p. ").trim_start_matches("pp. ")
                    ));
                }
                simple(&instr, &xenify_wp_doc::cite::citation(doc, keys, locator))
            }
            Field::Ref { target } => {
                let text = doc.para(*target).map_or(String::new(), |p| p.text.clone());
                simple(&format!("REF _Ref{} \\h", target.0), &text)
            }
            // Word's own equation markup as it came in; else written from the LaTeX.
            // A checkbox content control (Word 2010 and later).
            Field::Check { checked } => {
                let font = "<w:rFonts w:ascii=\"Segoe UI Symbol\" w:hAnsi=\"Segoe UI Symbol\"/>";
                format!(
                    "<w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val=\"{}\"/><w14:checkedState w14:val=\"2612\" w14:font=\"Segoe UI Symbol\"/><w14:uncheckedState w14:val=\"2610\" w14:font=\"Segoe UI Symbol\"/></w14:checkbox></w:sdtPr><w:sdtContent><w:r><w:rPr>{font}</w:rPr><w:t>{}</w:t></w:r></w:sdtContent></w:sdt>",
                    u8::from(*checked),
                    if *checked { "☒" } else { "☐" }
                )
            }
            Field::Equation { latex, display, omml } => {
                let math =
                    if omml.is_empty() { xenify_math::to_omml(&xenify_math::parse(latex)) } else { omml.clone() };
                if *display && !math.starts_with("<m:oMathPara") {
                    format!("<m:oMathPara>{math}</m:oMathPara>")
                } else {
                    math
                }
            }
        }
    }

    /// A note's paragraphs, the first starting with the note's number mark.
    fn note_xml(&mut self, doc: &Document, note: &xenify_wp_doc::Note, width: f32) -> String {
        let (el, style) = match note.kind {
            xenify_wp_doc::NoteKind::Foot => ("footnoteRef", "FootnoteReference"),
            xenify_wp_doc::NoteKind::End => ("endnoteRef", "EndnoteReference"),
        };
        let mark = format!(
            "<w:r><w:rPr><w:rStyle w:val=\"{style}\"/><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{el}/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r>"
        );
        let mut xml = self.blocks(doc, &note.blocks, width);
        let at = xml.find("</w:pPr>").map(|i| i + "</w:pPr>".len()).or_else(|| xml.find("<w:p>").map(|i| i + 5));
        if let Some(at) = at {
            xml.insert_str(at, &mark);
        }
        xml
    }

    fn paragraph(&mut self, doc: &Document, p: &Paragraph, sect: Option<&str>) -> String {
        // Checklist items: Word has no checklist numbering, so the box is a
        // character (☐/☒) and a tab, with the list's indent (read back as a checklist).
        let checklist = p.list.as_ref().filter(|i| {
            doc.list_def(&i.list).is_some_and(|d| d.level(i.level).kind == xenify_wp_doc::NumberKind::Check)
        });
        if let Some(item) = checklist {
            let mut q = p.clone();
            let lv = doc.list_def(&item.list).map(|d| d.level(item.level)).unwrap_or_default();
            let fmt = q.runs.first().map(|r| r.fmt.clone()).unwrap_or_default();
            let mark = if item.checked { "\u{2612}\t" } else { "\u{2610}\t" };
            q.replace_start(0, xenify_wp_doc::Span::plain(mark, fmt));
            q.list = None;
            q.props.indent_left = Some(lv.indent);
            q.props.first_line = Some(-lv.hanging);
            return self.paragraph(doc, &q, sect);
        }
        let mut ppr = format!("<w:pStyle w:val=\"{}\"/>", esc(&p.style));
        let mut num_pr = String::new();
        if let Some(item) = &p.list {
            let base = doc.lists.iter().position(|d| d.id == item.list).map_or(1, |i| i + 1);
            let num = if item.restart.is_some() {
                let id = 1000 + self.next_restart;
                self.next_restart += 1;
                self.num_ids.insert(item.list.clone(), id);
                id
            } else {
                *self.num_ids.entry(item.list.clone()).or_insert(base)
            };
            num_pr = format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{num}\"/></w:numPr>", item.level);
        }
        let (frame, boxed) = self.frame.clone().unwrap_or_default();
        ppr.push_str(&ppr_props(&p.props, &frame, &num_pr, &boxed));
        // A tracked inserted or deleted paragraph break: on the paragraph mark.
        if let Some(c) = &p.props.break_change {
            self.change_id += 1;
            let tag = match c.kind {
                xenify_wp_doc::review::ChangeKind::Inserted => "ins",
                xenify_wp_doc::review::ChangeKind::Deleted => "del",
            };
            let date = if c.date.is_empty() { String::new() } else { format!(" w:date=\"{}\"", esc(&c.date)) };
            let _ = write!(
                ppr,
                "<w:rPr><w:{tag} w:id=\"{}\" w:author=\"{}\"{date}/></w:rPr>",
                self.change_id,
                esc(&c.author)
            );
        }
        if let Some(sect) = sect {
            ppr.push_str(sect);
        }
        let column_break =
            if p.props.column_break_before == Some(true) { "<w:r><w:br w:type=\"column\"/></w:r>" } else { "" };
        // A paragraph other text refers to: a bookmark around it.
        let (bm_start, bm_end) = if self.ref_targets.contains(&p.id) {
            (
                format!("<w:bookmarkStart w:id=\"{0}\" w:name=\"_Ref{0}\"/>", p.id.0),
                format!("<w:bookmarkEnd w:id=\"{}\"/>", p.id.0),
            )
        } else {
            (String::new(), String::new())
        };
        format!("<w:p><w:pPr>{ppr}</w:pPr>{column_break}{bm_start}{}{bm_end}</w:p>", self.runs(doc, p))
    }

    fn hf(&mut self, kind: &str, line: &HfLine, n: usize) -> String {
        let name = format!("{kind}{n}.xml");
        let tag = if kind == "header" { "hdr" } else { "ftr" };
        let style = if kind == "header" { "header" } else { "footer" };
        // One paragraph with tab stops: left, centre and right parts.
        let mut runs = String::new();
        for (i, part) in [&line.left, &line.center, &line.right].into_iter().enumerate() {
            if i > 0 {
                runs.push_str("<w:r><w:tab/></w:r>");
            }
            runs.push_str(&field_runs(part));
        }
        let xml = format!(
            "{HEAD}<w:{tag} {NS}><w:p><w:pPr><w:pStyle w:val=\"{style}\"/><w:tabs><w:tab w:val=\"center\" w:pos=\"4680\"/><w:tab w:val=\"right\" w:pos=\"9360\"/></w:tabs></w:pPr>{runs}</w:p></w:{tag}>"
        );
        self.parts.push((name.clone(), xml));
        self.rel(name, if kind == "header" { "header" } else { "footer" })
    }

    /// A rich header or footer part (its blocks), with its own relationships.
    fn hf_rich(&mut self, doc: &Document, kind: &str, blocks: &[Block], width: f32, n: usize) -> String {
        let name = format!("{kind}{n}.xml");
        let tag = if kind == "header" { "hdr" } else { "ftr" };
        // The part's pictures and links are its own relationships.
        let body_rels = std::mem::take(&mut self.rels);
        let inner = self.blocks(doc, blocks, width);
        let own = std::mem::replace(&mut self.rels, body_rels);
        let xml = format!("{HEAD}<w:{tag} {NS}>{inner}</w:{tag}>");
        if !own.is_empty() {
            let mut rels = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
            );
            for (rid, target, kind) in &own {
                let _ = write!(
                    rels,
                    "<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{}\"{}/>",
                    esc(target),
                    if *kind == "hyperlink" { " TargetMode=\"External\"" } else { "" }
                );
            }
            rels.push_str("</Relationships>");
            self.part_rels.push((name.clone(), rels));
        }
        self.parts.push((name.clone(), xml));
        self.rel(name, if kind == "header" { "header" } else { "footer" })
    }

    fn sect_pr(&mut self, doc: &Document, n: usize, odd_even: bool) -> String {
        let sec = &doc.sections[n];
        let (page, start) = (&sec.page, sec.start);
        let mut s = String::from("<w:sectPr>");
        // Linked to the previous section: no references of its own (Word carries them on).
        let linked = n > 0 && page.link_to_previous;
        // Default; even (odd/even is document-wide in Word: sections without
        // their own even header repeat the default one); first page.
        let mut slots = vec![(xenify_wp_doc::HfSlot::Default, "default", 0)];
        if odd_even {
            slots.push((xenify_wp_doc::HfSlot::Even, "even", 200));
        }
        if page.different_first {
            slots.push((xenify_wp_doc::HfSlot::First, "first", 100));
        }
        let width = page.content_width();
        for (slot, typ, base) in slots {
            if linked {
                break;
            }
            let slot = if slot == xenify_wp_doc::HfSlot::Even && !page.different_odd_even {
                xenify_wp_doc::HfSlot::Default
            } else {
                slot
            };
            for footer in [false, true] {
                let kind = if footer { "footer" } else { "header" };
                let idx = base + n * 2 + usize::from(footer);
                let rich = sec.headers.iter().find(|h| h.footer == footer && h.slot == slot);
                let line = match (footer, slot) {
                    (false, xenify_wp_doc::HfSlot::Default) => &page.header,
                    (true, xenify_wp_doc::HfSlot::Default) => &page.footer,
                    (false, xenify_wp_doc::HfSlot::Even) => &page.even_header,
                    (true, xenify_wp_doc::HfSlot::Even) => &page.even_footer,
                    (false, xenify_wp_doc::HfSlot::First) => &page.first_header,
                    (true, xenify_wp_doc::HfSlot::First) => &page.first_footer,
                };
                let id = match rich {
                    Some(part) => self.hf_rich(doc, kind, &part.blocks, width, idx),
                    // A first page with nothing on it still needs its (empty) part.
                    None if !line.is_empty() || typ == "first" => self.hf(kind, line, idx),
                    None => continue,
                };
                let _ = write!(s, "<w:{kind}Reference w:type=\"{typ}\" r:id=\"{id}\"/>");
            }
        }
        if start == SectionStart::Continuous {
            s.push_str("<w:type w:val=\"continuous\"/>");
        }
        let orient = if page.width > page.height { " w:orient=\"landscape\"" } else { "" };
        let m = &page.margins;
        let _ = write!(
            s,
            "<w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/><w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{}\" w:footer=\"{}\" w:gutter=\"0\"/>",
            tw(page.width),
            tw(page.height),
            tw(m.top),
            tw(m.right),
            tw(m.bottom),
            tw(m.left),
            tw(m.top / 2.0),
            tw(m.bottom / 2.0)
        );
        let fmt = match page.number_format {
            xenify_wp_doc::PageNumberFormat::Decimal => "",
            xenify_wp_doc::PageNumberFormat::LowerRoman => " w:fmt=\"lowerRoman\"",
            xenify_wp_doc::PageNumberFormat::UpperRoman => " w:fmt=\"upperRoman\"",
            xenify_wp_doc::PageNumberFormat::LowerLetter => " w:fmt=\"lowerLetter\"",
            xenify_wp_doc::PageNumberFormat::UpperLetter => " w:fmt=\"upperLetter\"",
        };
        let start = page.start_number.map_or(String::new(), |n| format!(" w:start=\"{n}\""));
        if !fmt.is_empty() || !start.is_empty() {
            let _ = write!(s, "<w:pgNumType{start}{fmt}/>");
        }
        if page.columns > 1 {
            let sep = if page.column_line { " w:sep=\"1\"" } else { "" };
            let _ = write!(s, "<w:cols w:num=\"{}\" w:space=\"{}\"{sep}/>", page.columns, tw(page.column_gap));
        }
        match page.v_align {
            xenify_wp_doc::PageVAlign::Top => {}
            xenify_wp_doc::PageVAlign::Center => s.push_str("<w:vAlign w:val=\"center\"/>"),
            xenify_wp_doc::PageVAlign::Bottom => s.push_str("<w:vAlign w:val=\"bottom\"/>"),
        }
        if page.different_first {
            s.push_str("<w:titlePg/>");
        }
        s.push_str("</w:sectPr>");
        s
    }

    fn table(&mut self, doc: &Document, t: &Table, width: f32) -> String {
        let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
        let tw_total = width * t.width;
        let mut s = String::from("<w:tbl><w:tblPr>");
        let _ = write!(s, "<w:tblW w:w=\"{}\" w:type=\"dxa\"/>", tw(tw_total));
        if t.align != Align::Left {
            let _ = write!(s, "<w:jc w:val=\"{}\"/>", jc(t.align));
        }
        match t.border {
            Some(b) if b.width > 0.0 => {
                let e = |side: &str| format!("<w:{side} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{}\"/>", ((b.width * 8.0).round() as i64).max(2), hex(b.color));
                let _ = write!(s, "<w:tblBorders>{}{}{}{}{}{}</w:tblBorders>", e("top"), e("left"), e("bottom"), e("right"), e("insideH"), e("insideV"));
            }
            _ => s.push_str("<w:tblBorders><w:top w:val=\"nil\"/><w:left w:val=\"nil\"/><w:bottom w:val=\"nil\"/><w:right w:val=\"nil\"/><w:insideH w:val=\"nil\"/><w:insideV w:val=\"nil\"/></w:tblBorders>"),
        }
        let pad = tw(t.padding);
        let _ = write!(
            s,
            "<w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"{pad}\" w:type=\"dxa\"/><w:left w:w=\"{pad}\" w:type=\"dxa\"/><w:bottom w:w=\"{pad}\" w:type=\"dxa\"/><w:right w:w=\"{pad}\" w:type=\"dxa\"/></w:tblCellMar>{}</w:tblPr><w:tblGrid>",
            format_args!(
                "<w:tblLook w:firstRow=\"{}\" w:lastRow=\"{}\" w:firstColumn=\"{}\" w:noHBand=\"{}\"/>",
                u8::from(t.rows.first().is_some_and(|r| r.header)),
                u8::from(t.rows.last().is_some_and(|r| r.footer)),
                u8::from(t.header_col),
                u8::from(t.band.is_none())
            )
        );
        let col_w: Vec<f32> = t.widths.iter().map(|w| w / total * tw_total).collect();
        for w in &col_w {
            let _ = write!(s, "<w:gridCol w:w=\"{}\"/>", tw(*w));
        }
        s.push_str("</w:tblGrid>");
        let mut body_row = 0;
        for (r, row) in t.rows.iter().enumerate() {
            s.push_str("<w:tr>");
            // Word's "header row" repeats on each page: only when that's asked for.
            let repeats = row.header && t.repeat_header;
            if repeats || row.min_height.is_some() {
                s.push_str("<w:trPr>");
                if let Some(h) = row.min_height {
                    let _ = write!(s, "<w:trHeight w:val=\"{}\" w:hRule=\"atLeast\"/>", tw(h));
                }
                if repeats {
                    s.push_str("<w:tblHeader/>");
                }
                s.push_str("</w:trPr>");
            }
            let banded = !row.header && body_row % 2 == 1;
            if !row.header {
                body_row += 1;
            }
            let mut c = 0;
            while c < row.cells.len() {
                let cell = &row.cells[c];
                // Cells under a vertical merge continue it; others under merges are skipped.
                let (ar, ac) = t.anchor_of(r, c);
                let anchor = &t.rows[ar].cells[ac];
                if cell.covered && ar == r {
                    c += 1;
                    continue;
                }
                let span = anchor.col_span as usize;
                let w: f32 = col_w[c..(c + span).min(col_w.len())].iter().sum();
                let _ = write!(s, "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>", tw(w));
                if span > 1 {
                    let _ = write!(s, "<w:gridSpan w:val=\"{span}\"/>");
                }
                if anchor.row_span > 1 {
                    s.push_str(if ar == r { "<w:vMerge w:val=\"restart\"/>" } else { "<w:vMerge/>" });
                }
                if let Some(b) = anchor.borders.filter(|b| !b.is_empty()) {
                    let e = |side: &str, l: Option<xenify_wp_doc::Border>| {
                        l.filter(|l| l.width > 0.0).map_or(String::new(), |l| {
                            format!(
                                "<w:{side} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{}\"/>",
                                ((l.width * 8.0).round() as i64).max(2),
                                hex(l.color)
                            )
                        })
                    };
                    let _ = write!(
                        s,
                        "<w:tcBorders>{}{}{}{}</w:tcBorders>",
                        e("top", b.top),
                        e("left", b.left),
                        e("bottom", b.bottom),
                        e("right", b.right)
                    );
                }
                // Header and band colours are written as the cells' own (Word keeps them in table styles).
                let header = row.header || t.header_col && c == 0;
                let fill =
                    anchor.shading.or(if header { t.header_fill } else { None }).or(if banded { t.band } else { None });
                if let Some(sh) = fill {
                    let _ = write!(s, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(sh));
                }
                match anchor.v_align {
                    xenify_wp_doc::VAlign::Top => {}
                    xenify_wp_doc::VAlign::Middle => s.push_str("<w:vAlign w:val=\"center\"/>"),
                    xenify_wp_doc::VAlign::Bottom => s.push_str("<w:vAlign w:val=\"bottom\"/>"),
                }
                s.push_str("</w:tcPr>");
                if ar == r {
                    let inner = self.blocks(doc, &cell.blocks, w);
                    s.push_str(if inner.is_empty() { "<w:p/>" } else { &inner });
                } else {
                    s.push_str("<w:p/>");
                }
                s.push_str("</w:tc>");
                c += span;
            }
            s.push_str("</w:tr>");
        }
        s.push_str("</w:tbl>");
        s
    }

    fn image(&mut self, img: &xenify_wp_doc::Image) -> String {
        let ext = img.src.rsplit('.').next().unwrap_or("png").to_ascii_lowercase();
        let name = match self.media_ids.get(&img.src) {
            Some(n) => n.clone(),
            None => {
                let n = format!("media/image{}.{ext}", self.media.len() + 1);
                self.media.push((n.clone(), img.src.clone()));
                self.media_ids.insert(img.src.clone(), n.clone());
                n
            }
        };
        let rid = self.rel(name, "image");
        self.drawing_id += 1;
        let (w, h) = if img.width > 0.0 { (img.width, img.height) } else { (200.0, 150.0) };
        let (cx, cy) = (emu(w), emu(h));
        let id = self.drawing_id;
        let line = img.border.filter(|b| b.width > 0.0).map_or(String::new(), |b| {
            format!(
                "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:ln>",
                emu(b.width),
                hex(b.color)
            )
        });
        format!(
            "<w:p><w:pPr>{}<w:jc w:val=\"{}\"/></w:pPr><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\" descr=\"{}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"Picture {id}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{line}</pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
            self.frame.as_ref().map_or(String::new(), |(f, b)| format!("{f}{b}")),
            jc(img.align),
            esc(&img.alt)
        )
    }

    fn toc(&mut self, doc: &Document, t: &xenify_wp_doc::Toc) -> String {
        use xenify_wp_doc::TocKind;
        let field = |instr: &str, style: &str| {
            format!(
                "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> {instr} </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>"
            )
        };
        match t.kind {
            TocKind::Bibliography => {
                // Word's bibliography field, with the entries as its result.
                let mut s = field("BIBLIOGRAPHY", "normal");
                for runs in xenify_wp_doc::cite::bibliography(doc) {
                    s.push_str("<w:p><w:pPr><w:ind w:left=\"720\" w:hanging=\"720\"/></w:pPr>");
                    for (text, italic) in runs {
                        let _ = write!(
                            s,
                            "<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                            if italic { "<w:rPr><w:i/></w:rPr>" } else { "" },
                            esc(&text)
                        );
                    }
                    s.push_str("</w:p>");
                }
                s.push_str("<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>");
                return s;
            }
            TocKind::Figures => {
                // A contents field over the Caption style.
                let mut s = field("TOC \\h \\z \\t \"Caption,1\"", "toc1");
                for p in doc.paragraphs().filter(|p| p.style == StyleSheet::CAPTION && !p.text.trim().is_empty()) {
                    let _ = write!(
                        s,
                        "<w:p><w:pPr><w:pStyle w:val=\"toc1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                        esc(&p.text)
                    );
                }
                s.push_str("<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>");
                return s;
            }
            TocKind::ChangeLog => {
                // A real table Word shows, marked so it reads back as the live log.
                let width = doc.sections.first().map_or(451.0, |s| s.page.content_width());
                let table = xenify_wp_doc::changelog::table(doc);
                let inner = self.table(doc, &table, width);
                return format!(
                    "<w:sdt><w:sdtPr><w:alias w:val=\"Change log\"/><w:tag w:val=\"xenify-changelog\"/></w:sdtPr><w:sdtContent>{inner}<w:p/></w:sdtContent></w:sdt>"
                );
            }
            TocKind::Contents => {}
        }
        // A TOC field with the current entries as its cached result (Word updates it on open).
        let mut s = format!(
            "<w:p><w:pPr><w:pStyle w:val=\"toc1\"/></w:pPr><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-{}\" \\h \\z \\u </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>",
            t.levels
        );
        // Page numbers at a right tab with dot leaders, as Word draws them.
        let right = doc.sections.first().map_or(451.0, |s| s.page.content_width());
        let tab =
            format!("<w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"{}\"/></w:tabs>", (right * 20.0).round());
        for (id, level, text) in xenify_wp_layout::toc_entries(doc, t.id, t.levels) {
            let page = self.toc_pages.get(&id).filter(|_| t.page_numbers);
            let _ = write!(
                s,
                "<w:p><w:pPr><w:pStyle w:val=\"toc{}\"/>{}</w:pPr><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>{}</w:p>",
                level.min(3),
                if page.is_some() { tab.as_str() } else { "" },
                esc(&text),
                page.map_or(String::new(), |n| format!("<w:r><w:tab/></w:r><w:r><w:t>{n}</w:t></w:r>"))
            );
        }
        s.push_str("<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>");
        s
    }

    fn blocks(&mut self, doc: &Document, blocks: &[Block], width: f32) -> String {
        let mut s = String::new();
        for b in blocks {
            match b {
                Block::Paragraph(p) => s.push_str(&self.paragraph(doc, p, None)),
                Block::Table(t) => {
                    s.push_str(&self.table(doc, t, width));
                }
                Block::Image(img) => s.push_str(&self.image(img)),
                Block::Toc(t) => s.push_str(&self.toc(doc, t)),
            }
        }
        s
    }

    fn body(&mut self, doc: &Document) -> String {
        let mut s = String::new();
        let n = doc.sections.len();
        for (i, sec) in doc.sections.iter().enumerate() {
            let width = sec.page.content_width();
            let blocks = &sec.blocks;
            // A section's properties go in its last paragraph (or, for the
            // last section, at the end of the body).
            let odd_even = doc.sections.iter().any(|s| s.page.different_odd_even);
            let sect = self.sect_pr(doc, i, odd_even);
            // Frames go first: Word anchors a frame to the paragraph that holds it.
            for f in &sec.frames {
                self.frame = Some(frame_pr(f, &sec.page));
                let inner = (f.width - 2.0 * f.padding).max(12.0);
                s.push_str(&self.blocks(doc, &f.blocks, inner));
                self.frame = None;
            }
            if i + 1 == n {
                s.push_str(&self.blocks(doc, blocks, width));
                s.push_str(&sect);
            } else {
                let (last, rest) = match blocks.split_last() {
                    Some((Block::Paragraph(p), rest)) => (Some(p), rest),
                    _ => (None, &blocks[..]),
                };
                s.push_str(&self.blocks(doc, rest, width));
                match last {
                    Some(p) => s.push_str(&self.paragraph(doc, p, Some(&sect))),
                    None => {
                        let _ = write!(s, "<w:p><w:pPr>{sect}</w:pPr></w:p>");
                    }
                }
            }
            // The next section's start type belongs to it; Word reads it from its own sectPr.
        }
        s
    }
}

/// Runs for header/footer text with `{page}` / `{pages}` as Word fields.
fn field_runs(text: &str) -> String {
    let mut s = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["{page}", "{pages}", "{section-pages}", "{date}", "{title}", "{author}"]
            .iter()
            .filter_map(|f| rest.find(f).map(|i| (i, *f)))
            .min();
        let Some((i, f)) = next else {
            let _ = write!(s, "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(rest));
            break;
        };
        if i > 0 {
            let _ = write!(s, "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(&rest[..i]));
        }
        let instr = match f {
            "{page}" => "PAGE",
            "{pages}" => "NUMPAGES",
            "{section-pages}" => "SECTIONPAGES",
            "{date}" => "DATE \\@ \"yyyy-MM-dd\"",
            "{title}" => "TITLE",
            _ => "AUTHOR",
        };
        let _ = write!(s, "<w:fldSimple w:instr=\" {} \"><w:r><w:t>1</w:t></w:r></w:fldSimple>", esc(instr));
        rest = &rest[i + f.len()..];
    }
    s
}

/// Word frame properties for `f` (positioned from the page corner) and its
/// paragraphs' border and shading.
fn frame_pr(f: &xenify_wp_doc::Frame, _page: &PageSetup) -> (String, String) {
    use xenify_wp_doc::Wrap;
    let wrap = match f.wrap {
        Wrap::Around => "around",
        Wrap::TopBottom => "notBeside",
        Wrap::InFront | Wrap::Behind => "none",
    };
    let pad = f.padding;
    // Word's frame size is the text area; our frame's box includes the padding.
    let pr = format!(
        "<w:framePr w:w=\"{}\" w:h=\"{}\" w:hRule=\"exact\" w:hSpace=\"{}\" w:vSpace=\"{}\" w:wrap=\"{wrap}\" w:vAnchor=\"page\" w:hAnchor=\"page\" w:x=\"{}\" w:y=\"{}\"/>",
        tw(f.width - 2.0 * pad),
        tw(f.height - 2.0 * pad),
        tw(f.gap),
        tw(f.gap),
        tw(f.x + pad),
        tw(f.y + pad)
    );
    let mut boxed = String::new();
    if let Some(b) = f.border.filter(|b| b.width > 0.0) {
        let e = |side: &str| {
            format!(
                "<w:{side} w:val=\"single\" w:sz=\"{}\" w:space=\"{}\" w:color=\"{}\"/>",
                ((b.width * 8.0).round() as i64).max(2),
                pad.round() as i64,
                hex(b.color)
            )
        };
        let _ = write!(boxed, "<w:pBdr>{}{}{}{}</w:pBdr>", e("top"), e("left"), e("bottom"), e("right"));
    }
    if let Some(c) = f.fill {
        let _ = write!(boxed, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(c));
    }
    (pr, boxed)
}

/// The opening of a content control: its settings, then `w:sdtContent`.
fn control_start(c: &xenify_wp_doc::controls::Control) -> String {
    use xenify_wp_doc::controls::ControlKind;
    let mut pr = String::new();
    if !c.title.is_empty() {
        let _ = write!(pr, "<w:alias w:val=\"{}\"/>", esc(&c.title));
    }
    if !c.tag.is_empty() {
        let _ = write!(pr, "<w:tag w:val=\"{}\"/>", esc(&c.tag));
    }
    if c.placeholder {
        pr.push_str("<w:showingPlcHdr/>");
    }
    let list = |items: &[String]| {
        items.iter().map(|i| format!("<w:listItem w:displayText=\"{0}\" w:value=\"{0}\"/>", esc(i))).collect::<String>()
    };
    match &c.kind {
        ControlKind::Text { rich: false } => pr.push_str("<w:text/>"),
        ControlKind::Text { rich: true } => {}
        ControlKind::Date { format } => {
            let _ = write!(
                pr,
                "<w:date><w:dateFormat w:val=\"{}\"/><w:calendar w:val=\"gregorian\"/></w:date>",
                esc(format)
            );
        }
        ControlKind::Dropdown { items } => {
            let _ = write!(pr, "<w:dropDownList>{}</w:dropDownList>", list(items));
        }
        ControlKind::ComboBox { items } => {
            let _ = write!(pr, "<w:comboBox>{}</w:comboBox>", list(items));
        }
    }
    format!("<w:sdt><w:sdtPr>{pr}</w:sdtPr><w:sdtContent>")
}
