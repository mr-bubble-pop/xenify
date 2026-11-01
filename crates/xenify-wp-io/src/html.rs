//! HTML export: one self-contained file (styles as CSS classes, pictures
//! inlined), readable in any browser and printable with the page setup.

use base64::Engine as _;
use std::fmt::Write;
use xenify_wp_doc::{Align, Block, CharFmt, Document, NumberKind, Paragraph, ResolvedChar, StyleSheet, Table, lists};

use xenify_util::esc;

fn align(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "justify",
    }
}

fn font_css(font: &str) -> String {
    match font {
        "serif" | "sans-serif" | "monospace" => font.into(),
        f => format!("\"{}\", sans-serif", f.replace('"', "")),
    }
}

/// CSS for the difference between character looks `c` and `base`.
fn char_css(c: &ResolvedChar, base: Option<&ResolvedChar>) -> String {
    let mut s = String::new();
    let d = |f: &dyn Fn(&ResolvedChar) -> bool| base.is_none_or(f);
    if d(&|b| b.font != c.font) {
        let _ = write!(s, "font-family:{};", font_css(&c.font));
    }
    if d(&|b| b.size != c.size) {
        let _ = write!(s, "font-size:{}pt;", c.size);
    }
    if d(&|b| b.bold != c.bold) {
        s.push_str(if c.bold { "font-weight:700;" } else { "font-weight:400;" });
    }
    if d(&|b| b.italic != c.italic) {
        s.push_str(if c.italic { "font-style:italic;" } else { "font-style:normal;" });
    }
    if d(&|b| (b.underline, b.strike) != (c.underline, c.strike)) {
        let v = match (c.underline, c.strike) {
            (true, true) => "underline line-through",
            (true, false) => "underline",
            (false, true) => "line-through",
            _ => "none",
        };
        let _ = write!(s, "text-decoration:{v};");
    }
    if d(&|b| b.color != c.color) {
        let _ = write!(s, "color:{};", c.color.to_hex());
    }
    if let Some(h) = c.highlight
        && d(&|b| b.highlight != c.highlight)
    {
        let _ = write!(s, "background:{};", h.to_hex());
    }
    s
}

fn para_css(r: &xenify_wp_doc::ResolvedPara) -> String {
    format!(
        "text-align:{};margin:{}pt {}pt {}pt {}pt;line-height:{};{}",
        align(r.align),
        r.space_before,
        r.indent_right,
        r.space_after,
        r.indent_left,
        r.line_exact.map_or(r.line_height.to_string(), |pt| format!("{pt}pt")),
        if r.first_line != 0.0 { format!("text-indent:{}pt;", r.first_line) } else { String::new() }
    ) + &boxed_css(r)
}

/// Paragraph shading and borders as CSS.
fn boxed_css(r: &xenify_wp_doc::ResolvedPara) -> String {
    let mut s = String::new();
    if let Some(c) = r.shading {
        let _ = write!(s, "background:{};", c.to_hex());
    }
    let b = &r.borders;
    for (side, line) in [("top", b.top), ("bottom", b.bottom), ("left", b.left), ("right", b.right)] {
        if let Some(l) = line.filter(|l| l.width > 0.0) {
            let _ = write!(s, "border-{side}:{}pt solid {};", l.width, l.color.to_hex());
        }
    }
    if !s.is_empty() {
        s.push_str("padding:4pt;");
    }
    s
}

fn class(id: &str) -> String {
    format!("s-{}", id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
}

/// A paragraph's text as HTML inline markup.
/// A field's current text: note numbers link to their note, cross-references to their heading.
fn field_html(doc: &Document, field: &xenify_wp_doc::Field) -> String {
    use xenify_wp_doc::Field;
    match field {
        Field::Note { id } => {
            let n = xenify_wp_doc::notes::numbers(doc).get(id).cloned().unwrap_or_default();
            format!("<sup class=\"note-ref\"><a id=\"ref{0}\" href=\"#note{0}\">{1}</a></sup>", id.0, esc(&n))
        }
        Field::Citation { keys, locator } => {
            format!("<cite>{}</cite>", esc(&xenify_wp_doc::cite::citation(doc, keys, locator)))
        }
        Field::Page | Field::Pages => String::new(),
        Field::Date => esc(&chrono_free_today()),
        Field::Badge { text, color } => {
            let (bg, fg) = xenify_wp_doc::badge_colors(color);
            format!(
                "<span class=\"badge\" style=\"background:{};color:{};font-weight:700;font-size:.78em;padding:0 .35em;border-radius:3px\">{}</span>",
                bg.to_hex(),
                fg.to_hex(),
                esc(&text.to_uppercase())
            )
        }
        Field::DateTime { format, fixed } => {
            esc(&fixed.clone().unwrap_or_else(|| xenify_wp_doc::datetime::format(format, &xenify_wp_layout::now())))
        }
        Field::Title => esc(&doc.meta.title),
        Field::Author => esc(&doc.meta.author),
        Field::Ref { target } => match doc.para(*target) {
            Some(p) => format!("<a href=\"#p{}\">{}</a>", target.0, esc(&p.text)),
            None => String::new(),
        },
        // LaTeX in MathJax/KaTeX's delimiters.
        Field::Equation { latex, display: false, .. } => {
            format!("<span class=\"math inline\">\\({}\\)</span>", esc(latex))
        }
        Field::Equation { latex, display: true, .. } => {
            format!("<span class=\"math display\">\\[{}\\]</span>", esc(latex))
        }
        Field::Check { checked } => {
            format!("<input type=\"checkbox\" disabled{}>", if *checked { " checked" } else { "" })
        }
    }
}

/// Today as YYYY-MM-DD (UTC).
fn chrono_free_today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let days = secs.div_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Comment threads, after the text.
fn comments_html(doc: &Document) -> String {
    let open: Vec<_> = doc.meta.comments.iter().filter(|c| !c.resolved).collect();
    if open.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"comments\">\n<hr>\n<h2>Comments</h2>\n");
    for c in open {
        let _ = write!(
            out,
            "<div class=\"thread\" id=\"comment{}\"><p><b>{}</b> {}</p>",
            c.id.0,
            esc(&c.author),
            esc(&c.text)
        );
        for r in &c.replies {
            let _ = write!(out, "<p class=\"reply\"><b>{}</b> {}</p>", esc(&r.author), esc(&r.text));
        }
        out.push_str("</div>\n");
    }
    out.push_str("</section>\n");
    out
}

/// Footnotes and endnotes, numbered, at the end of the page.
fn notes_html(doc: &Document) -> String {
    let numbers = xenify_wp_doc::notes::numbers(doc);
    let mut seen = std::collections::HashSet::new();
    let mut out = String::new();
    for kind in [xenify_wp_doc::NoteKind::Foot, xenify_wp_doc::NoteKind::End] {
        let mut items = String::new();
        for p in doc.paragraphs() {
            for (_, id) in xenify_wp_doc::notes::references(p) {
                let Some(note) = doc.note(id).filter(|n| n.kind == kind) else { continue };
                if !seen.insert(id) {
                    continue;
                }
                let body: Vec<String> = note
                    .blocks
                    .iter()
                    .filter_map(|b| if let Block::Paragraph(p) = b { Some(inline(doc, p)) } else { None })
                    .collect();
                let _ = writeln!(
                    items,
                    "<li id=\"note{0}\" value=\"{1}\"><span class=\"note-num\">{2}</span> {3} <a href=\"#ref{0}\" aria-label=\"Back to the text\">↩</a></li>",
                    id.0,
                    items.matches("<li").count() + 1,
                    esc(numbers.get(&id).map_or("", |s| s.as_str())),
                    body.join("<br>")
                );
            }
        }
        if !items.is_empty() {
            let class = match kind {
                xenify_wp_doc::NoteKind::Foot => "footnotes",
                xenify_wp_doc::NoteKind::End => "endnotes",
            };
            let _ = write!(out, "<section class=\"{class}\">\n<hr>\n");
            let _ = write!(out, "<ol>\n{items}</ol>\n</section>\n");
        }
    }
    out
}

fn inline(doc: &Document, p: &Paragraph) -> String {
    let style = doc.styles.resolve(&p.style).char;
    let mut out = String::new();
    for (range, fmt) in p.run_ranges() {
        if let Some(field) = &fmt.field {
            for _ in p.text[range].matches(xenify_wp_doc::Field::CHAR) {
                out.push_str(&field_html(doc, field));
            }
            continue;
        }
        let text = esc(&p.text[range]);
        let c = style.with(fmt);
        let code = fmt.font.as_deref() == Some("monospace");
        let mut inner = if code { format!("<code>{text}</code>") } else { text };
        let rest = CharFmt { link: None, change: None, comments: Vec::new(), vert: None, ..fmt.clone() };
        if !rest.is_empty() && !code {
            let css = char_css(&c, Some(&style));
            if !css.is_empty() {
                inner = format!("<span style=\"{css}\">{inner}</span>");
            }
        }
        match fmt.vert {
            Some(xenify_wp_doc::VertAlign::Superscript) => inner = format!("<sup>{inner}</sup>"),
            Some(xenify_wp_doc::VertAlign::Subscript) => inner = format!("<sub>{inner}</sub>"),
            _ => {}
        }
        if let Some(url) = &fmt.link {
            inner = format!("<a href=\"{}\">{inner}</a>", esc(url));
        }
        // Tracked changes, as HTML's own <ins> and <del>.
        if let Some(ch) = &fmt.change {
            let tag = match ch.kind {
                xenify_wp_doc::review::ChangeKind::Inserted => "ins",
                xenify_wp_doc::review::ChangeKind::Deleted => "del",
            };
            let when = if ch.date.is_empty() { String::new() } else { format!(" datetime=\"{}\"", esc(&ch.date)) };
            inner = format!("<{tag} title=\"{}\"{when}>{inner}</{tag}>", esc(&ch.author));
        }
        // Open comments: the text marked, linking to the thread.
        for id in &fmt.comments {
            if let Some(c) = doc.meta.comments.iter().find(|c| c.id == *id && !c.resolved) {
                inner = format!(
                    "<mark class=\"comment\" title=\"{}: {}\"><a href=\"#comment{}\">{inner}</a></mark>",
                    esc(&c.author),
                    esc(&c.text),
                    id.0
                );
            }
        }
        out.push_str(&inner);
    }
    if out.is_empty() { "<br>".into() } else { out }
}

fn list_type(k: NumberKind) -> &'static str {
    match k {
        NumberKind::Bullet => "disc",
        NumberKind::Decimal => "decimal",
        NumberKind::LowerAlpha => "lower-alpha",
        NumberKind::UpperAlpha => "upper-alpha",
        NumberKind::LowerRoman => "lower-roman",
        NumberKind::UpperRoman => "upper-roman",
        NumberKind::None | NumberKind::Check => "none",
    }
}

fn paragraph(doc: &Document, p: &Paragraph) -> String {
    let r = doc.resolve_para(p);
    let tag = match r.outline_level {
        Some(l) if (1..=6).contains(&l) && p.style.starts_with("heading") => format!("h{l}"),
        _ if p.style == StyleSheet::QUOTE => "blockquote".into(),
        _ => "p".into(),
    };
    let mut style = String::new();
    if !p.props.is_empty() {
        style = para_css(&r);
    }
    if r.page_break_before {
        style.push_str("break-before:page;");
    }
    if r.column_break_before {
        style.push_str("break-before:column;");
    }
    let style_attr = if style.is_empty() { String::new() } else { format!(" style=\"{style}\"") };
    format!("<{tag} id=\"p{}\" class=\"{}\"{style_attr}>{}</{tag}>\n", p.id.0, class(&p.style), inline(doc, p))
}

fn table(doc: &Document, t: &Table) -> String {
    let mut s = String::new();
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let border = t.border.map_or("none".to_string(), |b| format!("{}pt solid {}", b.width, b.color.to_hex()));
    let margin = match t.align {
        Align::Center => "auto",
        Align::Right => "0 0 0 auto",
        _ => "0",
    };
    let _ = writeln!(s, "<table style=\"width:{}%;margin:{margin};--b:{border}\">", t.width * 100.0);
    s.push_str("<colgroup>");
    for w in &t.widths {
        let _ = write!(s, "<col style=\"width:{:.2}%\">", w / total * 100.0);
    }
    s.push_str("</colgroup>\n");
    let heads = t.rows.iter().take_while(|r| r.header).count();
    let foot = t.rows.len() - t.rows.iter().rev().take_while(|r| r.footer).count().min(t.rows.len() - heads);
    let mut body_row = 0;
    for (ri, row) in t.rows.iter().enumerate() {
        if ri == 0 && heads > 0 {
            s.push_str("<thead>\n");
        }
        if ri == heads && ri < foot {
            s.push_str("<tbody>\n");
        }
        if ri == foot {
            if foot > heads {
                s.push_str("</tbody>\n");
            }
            s.push_str("<tfoot>\n");
        }
        match row.min_height {
            Some(h) => {
                let _ = write!(s, "<tr style=\"height:{h}pt\">");
            }
            None => s.push_str("<tr>"),
        }
        let banded = !row.header && body_row % 2 == 1;
        if !row.header {
            body_row += 1;
        }
        for (ci, cell) in row.cells.iter().enumerate().filter(|(_, c)| !c.covered) {
            let header = row.header || row.footer || t.header_col && ci == 0;
            let tag = if header { "th" } else { "td" };
            let mut attrs = String::new();
            if cell.col_span > 1 {
                let _ = write!(attrs, " colspan=\"{}\"", cell.col_span);
            }
            if cell.row_span > 1 {
                let _ = write!(attrs, " rowspan=\"{}\"", cell.row_span);
            }
            let fill =
                cell.shading.or(if header { t.header_fill } else { None }).or(if banded { t.band } else { None });
            let mut bg = fill.map_or(String::new(), |c| format!("background:{};", c.to_hex()));
            match cell.v_align {
                xenify_wp_doc::VAlign::Top => bg.push_str("vertical-align:top;"),
                xenify_wp_doc::VAlign::Middle => bg.push_str("vertical-align:middle;"),
                xenify_wp_doc::VAlign::Bottom => bg.push_str("vertical-align:bottom;"),
            }
            if let Some(b) = cell.borders {
                for (side, l) in [("top", b.top), ("bottom", b.bottom), ("left", b.left), ("right", b.right)] {
                    if let Some(l) = l.filter(|l| l.width > 0.0) {
                        let _ = write!(bg, "border-{side}:{}pt solid {};", l.width, l.color.to_hex());
                    }
                }
            }
            let _ = write!(s, "<{tag}{attrs} style=\"padding:{}pt;{bg}\">", t.padding);
            s.push_str(&blocks(doc, &cell.blocks));
            let _ = write!(s, "</{tag}>");
        }
        s.push_str("</tr>\n");
        if ri + 1 == heads {
            s.push_str("</thead>\n");
        }
    }
    if foot < t.rows.len() {
        s.push_str("</tfoot>\n");
    } else if t.rows.len() > heads {
        s.push_str("</tbody>\n");
    }
    s.push_str("</table>\n");
    s
}

fn blocks(doc: &Document, blocks: &[Block]) -> String {
    let labels = lists::labels(doc);
    let mut s = String::new();
    // Open lists: (tag, level).
    let mut open: Vec<(&'static str, u8)> = Vec::new();
    let close_to = |s: &mut String, open: &mut Vec<(&'static str, u8)>, depth: usize| {
        while open.len() > depth {
            let (tag, _) = open.pop().expect("non-empty");
            let _ = writeln!(s, "</li></{tag}>");
        }
    };
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::Paragraph(p) if p.list.is_some() => {
                let item = p.list.as_ref().expect("checked");
                let def = doc.list_def(&item.list);
                let kind = def.map_or(NumberKind::Bullet, |d| d.level(item.level).kind);
                let tag = if kind == NumberKind::Bullet { "ul" } else { "ol" };
                let depth = item.level as usize + 1;
                if open.len() >= depth && open[depth - 1].0 != tag {
                    close_to(&mut s, &mut open, depth - 1);
                }
                if open.len() >= depth {
                    close_to(&mut s, &mut open, depth);
                    s.push_str("</li>");
                }
                while open.len() < depth {
                    let lt = list_type(def.map_or(kind, |d| d.level(open.len() as u8).kind));
                    let start = if tag == "ol" {
                        labels
                            .get(&p.id)
                            .and_then(|l| {
                                l.trim_end_matches(['.', ')']).rsplit('.').next().and_then(|n| n.parse::<u32>().ok())
                            })
                            .map_or(String::new(), |n| format!(" start=\"{n}\""))
                    } else {
                        String::new()
                    };
                    let _ = write!(s, "<{tag}{start} style=\"list-style-type:{lt}\">");
                    open.push((tag, open.len() as u8));
                }
                let tick = match p.list.as_ref() {
                    Some(i) if doc.list_def(&i.list).is_some_and(|d| d.level(i.level).kind == NumberKind::Check) => {
                        if i.checked {
                            "<input type=\"checkbox\" checked disabled> "
                        } else {
                            "<input type=\"checkbox\" disabled> "
                        }
                    }
                    _ => "",
                };
                let _ = write!(s, "<li id=\"p{}\" class=\"{}\">{tick}{}", p.id.0, class(&p.style), inline(doc, p));
            }
            other => {
                close_to(&mut s, &mut open, 0);
                match other {
                    Block::Paragraph(p) if p.style == StyleSheet::CODE => {
                        s.push_str("<pre class=\"s-code\">");
                        let mut lines = Vec::new();
                        while let Some(Block::Paragraph(q)) = blocks.get(i) {
                            if q.style != StyleSheet::CODE || q.list.is_some() {
                                break;
                            }
                            lines.push(esc(&q.text));
                            i += 1;
                        }
                        s.push_str(&lines.join("\n"));
                        s.push_str("</pre>\n");
                        continue;
                    }
                    Block::Paragraph(p) => s.push_str(&paragraph(doc, p)),
                    Block::Table(t) => s.push_str(&table(doc, t)),
                    Block::Image(img) => {
                        let src = match doc.assets.get(&img.src) {
                            Some(bytes) => {
                                let mime = if img.src.ends_with(".png") {
                                    "image/png"
                                } else if img.src.ends_with(".webp") {
                                    "image/webp"
                                } else if img.src.ends_with(".gif") {
                                    "image/gif"
                                } else {
                                    "image/jpeg"
                                };
                                format!(
                                    "data:{mime};base64,{}",
                                    base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())
                                )
                            }
                            None => esc(&img.src),
                        };
                        // Narrower than the page: it shrinks, keeping its shape.
                        let mut size = if img.width > 0.0 && img.height > 0.0 {
                            format!("width:{}pt;height:auto;aspect-ratio:{} / {};", img.width, img.width, img.height)
                        } else {
                            String::new()
                        };
                        if let Some(b) = img.border.filter(|b| b.width > 0.0) {
                            let _ = write!(size, "border:{}pt solid {};", b.width, b.color.to_hex());
                        }
                        let _ = writeln!(
                            s,
                            "<figure style=\"text-align:{}\"><img src=\"{src}\" alt=\"{}\" style=\"{size}max-width:100%\"></figure>",
                            align(img.align),
                            esc(&img.alt)
                        );
                    }
                    Block::Toc(t) if t.kind == xenify_wp_doc::TocKind::Bibliography => {
                        s.push_str("<section class=\"bibliography\">\n");
                        for runs in xenify_wp_doc::cite::bibliography(doc) {
                            let text: String = runs
                                .iter()
                                .map(|(t, i)| if *i { format!("<i>{}</i>", esc(t)) } else { esc(t) })
                                .collect();
                            let _ = writeln!(s, "<p style=\"padding-left:36pt;text-indent:-36pt\">{text}</p>");
                        }
                        s.push_str("</section>\n");
                    }
                    Block::Toc(t) if t.kind == xenify_wp_doc::TocKind::ChangeLog => {
                        if !doc.meta.changelog.is_empty() {
                            s.push_str(&table(doc, &xenify_wp_doc::changelog::table(doc)));
                        }
                    }
                    Block::Toc(t) if t.kind == xenify_wp_doc::TocKind::Figures => {
                        s.push_str("<nav class=\"figures\">\n");
                        for p in
                            doc.paragraphs().filter(|p| p.style == StyleSheet::CAPTION && !p.text.trim().is_empty())
                        {
                            let _ =
                                writeln!(s, "<p class=\"s-toc1\"><a href=\"#p{}\">{}</a></p>", p.id.0, esc(&p.text));
                        }
                        s.push_str("</nav>\n");
                    }
                    Block::Toc(t) => {
                        s.push_str("<nav class=\"toc\">\n");
                        for (id, level, text) in xenify_wp_layout::toc_entries(doc, t.id, t.levels) {
                            let _ = writeln!(
                                s,
                                "<p class=\"s-toc{}\"><a href=\"#p{}\">{}</a></p>",
                                level.min(3),
                                id.0,
                                esc(&text)
                            );
                        }
                        s.push_str("</nav>\n");
                    }
                }
            }
        }
        i += 1;
    }
    close_to(&mut s, &mut open, 0);
    s
}

pub fn write(doc: &Document) -> String {
    let mut css = String::new();
    let base = doc.styles.resolve(StyleSheet::NORMAL);
    let ps = doc.sections.first().map(|s| s.page.clone()).unwrap_or_default();
    let m = &ps.margins;
    let _ = writeln!(
        css,
        "@page {{ size: {}pt {}pt; margin: {}pt {}pt {}pt {}pt; }}",
        ps.width, ps.height, m.top, m.right, m.bottom, m.left
    );
    let _ = writeln!(
        css,
        "body {{ max-width: {}pt; margin: 2em auto; padding: 0 1em; {}{} {} }}",
        ps.content_width(),
        doc.meta.page_color.map_or(String::new(), |c| format!("background:{};", c.to_hex())),
        char_css(&base.char, None),
        format_args!("line-height:{};", base.line_height)
    );
    css.push_str("table { border-collapse: collapse; } td, th { border: var(--b); vertical-align: top; text-align: left; } td > *:first-child, th > *:first-child { margin-top: 0; } nav.toc a { color: inherit; text-decoration: none; } pre { white-space: pre-wrap; } figure { margin: 8pt 0; }\n");
    for st in &doc.styles.paragraph {
        let r = doc.styles.resolve(&st.id);
        let _ = writeln!(css, ".{} {{ {}{} }}", class(&st.id), para_css(&r), char_css(&r.char, Some(&base.char)));
    }
    let title = if doc.meta.title.is_empty() { "Document".to_string() } else { esc(&doc.meta.title) };
    let mut body = String::new();
    for s in &doc.sections {
        // Frames float beside the text (a web page has no fixed page to place them on).
        for f in &s.frames {
            let side = if f.x + f.width / 2.0 > s.page.width / 2.0 { "right" } else { "left" };
            let float = match f.wrap {
                xenify_wp_doc::Wrap::TopBottom => "clear:both;".to_string(),
                _ => format!("float:{side};margin-{}:{}pt;", if side == "right" { "left" } else { "right" }, f.gap),
            };
            let border =
                f.border.map_or(String::new(), |b| format!("border:{}pt solid {};", b.width, b.color.to_hex()));
            let fill = f.fill.map_or(String::new(), |c| format!("background:{};", c.to_hex()));
            let _ = writeln!(
                body,
                "<aside class=\"frame\" style=\"{float}width:{}pt;min-height:{}pt;padding:{}pt;box-sizing:border-box;{border}{fill}\">",
                f.width, f.height, f.padding
            );
            body.push_str(&blocks(doc, &f.blocks));
            body.push_str("</aside>\n");
        }
        if s.page.columns > 1 {
            let rule = if s.page.column_line { "column-rule:1px solid #808080;" } else { "" };
            let _ = writeln!(
                body,
                "<div class=\"columns\" style=\"column-count:{};column-gap:{}pt;{rule}\">",
                s.page.columns, s.page.column_gap
            );
            body.push_str(&blocks(doc, &s.blocks));
            body.push_str("</div>\n");
        } else {
            body.push_str(&blocks(doc, &s.blocks));
        }
    }
    body.push_str(&notes_html(doc));
    body.push_str(&comments_html(doc));
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"generator\" content=\"Xenify Write\">\n<title>{title}</title>\n<style>\n{css}</style>\n</head>\n<body>\n{body}</body>\n</html>\n"
    )
}
