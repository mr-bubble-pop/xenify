//! YAML front matter: document settings Markdown has no syntax for (title,
//! page setup, style definitions, audience and tone).

use saphyr::{LoadableYamlNode, Scalar, Yaml};
use serde_json::{Map, Value, json};
use xenify_wp_doc::{Align, CharFmt, Document, ListDef, PageSetup, ParaProps, ParaStyle, Rgba, StyleSheet};

/// YAML text → JSON value (strings, numbers, booleans, lists, maps).
pub fn yaml_to_json(src: &str) -> Result<Value, String> {
    let docs = Yaml::load_from_str(src).map_err(|e| e.to_string())?;
    Ok(docs.first().map(convert).unwrap_or(Value::Null))
}

fn convert(y: &Yaml) -> Value {
    match y {
        Yaml::Value(s) => scalar(s),
        Yaml::Representation(s, ..) => Value::String(s.to_string()),
        Yaml::Sequence(v) => Value::Array(v.iter().map(convert).collect()),
        Yaml::Mapping(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                let key = match convert(k) {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                out.insert(key, convert(v));
            }
            Value::Object(out)
        }
        Yaml::Tagged(_, inner) => convert(inner),
        _ => Value::Null,
    }
}

fn scalar(s: &Scalar) -> Value {
    match s {
        Scalar::Null => Value::Null,
        Scalar::Boolean(b) => Value::Bool(*b),
        Scalar::Integer(i) => json!(i),
        Scalar::FloatingPoint(f) => json!(f.into_inner()),
        Scalar::String(s) => Value::String(s.to_string()),
    }
}

/// Writes a JSON value as block-style YAML (the subset front matter needs).
pub fn emit_yaml(v: &Value) -> String {
    let mut out = String::new();
    emit_map(v, 0, &mut out);
    out
}

fn emit_map(v: &Value, indent: usize, out: &mut String) {
    let Value::Object(m) = v else { return };
    for (k, v) in m {
        out.push_str(&" ".repeat(indent));
        out.push_str(&yaml_str(k));
        out.push(':');
        match v {
            Value::Object(o) if !o.is_empty() => {
                out.push('\n');
                emit_map(v, indent + 2, out);
            }
            Value::Array(a) if a.iter().any(|x| x.is_object()) => {
                out.push('\n');
                for item in a {
                    out.push_str(&" ".repeat(indent + 2));
                    out.push_str("- ");
                    let mut inner = String::new();
                    emit_map(item, indent + 4, &mut inner);
                    out.push_str(inner.trim_start());
                }
            }
            _ => {
                out.push(' ');
                out.push_str(&flow(v));
                out.push('\n');
            }
        }
    }
}

fn flow(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => yaml_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(flow).collect::<Vec<_>>().join(", ")),
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, v)| format!("{}: {}", yaml_str(k), flow(v))).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// A YAML scalar: plain when that reads back as the same string, else
/// double-quoted (JSON escaping is valid YAML).
fn yaml_str(s: &str) -> String {
    let special = s.is_empty()
        || s.trim() != s
        || s.chars().any(|c| ":#{}[],&*!|>'\"%@`\\".contains(c) || c.is_control())
        || s.starts_with(['-', '?', '.'])
        || matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~")
        || s.parse::<f64>().is_ok();
    if special { serde_json::to_string(s).expect("strings serialise") } else { s.to_string() }
}

/// A length in points from a number (points) or a string with a unit
/// (`pt`, `in`, `cm`, `mm`, `px`).
pub fn length(v: &Value) -> Option<f32> {
    if let Some(n) = v.as_f64() {
        return Some(n as f32);
    }
    let s = v.as_str()?.trim().to_ascii_lowercase();
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f32 = num.trim().parse().ok()?;
    Some(match unit.trim() {
        "" | "pt" => n,
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "px" => n * 0.75,
        _ => return None,
    })
}

const PAGE_SIZES: [(&str, (f32, f32)); 6] = [
    ("A4", PageSetup::A4),
    ("Letter", PageSetup::LETTER),
    ("Legal", (612.0, 1008.0)),
    ("A5", (419.53, 595.28)),
    ("A3", (841.89, 1190.55)),
    ("Tabloid", (792.0, 1224.0)),
];

/// A number with at most two decimals (f32 values would print as 1.2999999523).
fn round(v: f32) -> Value {
    let r = ((v as f64) * 100.0).round() / 100.0;
    if r.fract() == 0.0 { json!(r as i64) } else { json!(r) }
}

fn page_json(p: &PageSetup) -> Value {
    let (w, h) = (p.width, p.height);
    let named = PAGE_SIZES.iter().find(|(_, (a, b))| {
        ((a - w).abs() < 0.5 && (b - h).abs() < 0.5) || ((a - h).abs() < 0.5 && (b - w).abs() < 0.5)
    });
    let mut m = Map::new();
    match named {
        Some((name, _)) => {
            m.insert("size".into(), json!(name));
            if w > h {
                m.insert("orientation".into(), json!("landscape"));
            }
        }
        None => {
            m.insert("width".into(), json!(format!("{}pt", (w * 100.0).round() / 100.0)));
            m.insert("height".into(), json!(format!("{}pt", (h * 100.0).round() / 100.0)));
        }
    }
    let mg = &p.margins;
    m.insert(
        "margins".into(),
        json!({"top": round(mg.top), "bottom": round(mg.bottom), "left": round(mg.left), "right": round(mg.right)}),
    );
    let fresh = PageSetup::default();
    if p.columns != fresh.columns {
        m.insert("columns".into(), json!(p.columns));
    }
    if p.column_line {
        m.insert("column-line".into(), json!(true));
    }
    if p.column_gap != fresh.column_gap {
        m.insert("column-gap".into(), round(p.column_gap));
    }
    for (k, line) in [("header", &p.header), ("footer", &p.footer)] {
        if !line.is_empty() {
            m.insert(k.into(), hf_json(line));
        }
    }
    if p.different_first {
        m.insert("different-first-page".into(), json!(true));
        m.insert("first-header".into(), hf_json(&p.first_header));
        m.insert("first-footer".into(), hf_json(&p.first_footer));
    }
    if let Some(n) = p.start_number {
        m.insert("start-number".into(), json!(n));
    }
    if p.different_odd_even {
        m.insert("different-odd-even".into(), json!(true));
        m.insert("even-header".into(), hf_json(&p.even_header));
        m.insert("even-footer".into(), hf_json(&p.even_footer));
    }
    if !p.number_format.is_decimal() {
        m.insert("number-format".into(), serde_json::to_value(p.number_format).unwrap_or(Value::Null));
    }
    if !p.v_align.is_top() {
        m.insert("vertical-align".into(), serde_json::to_value(p.v_align).unwrap_or(Value::Null));
    }
    Value::Object(m)
}

fn hf_json(l: &xenify_wp_doc::HfLine) -> Value {
    let mut m = Map::new();
    for (k, v) in [("left", &l.left), ("center", &l.center), ("right", &l.right)] {
        if !v.is_empty() {
            m.insert(k.into(), json!(v));
        }
    }
    Value::Object(m)
}

fn hf_from(v: &Value) -> xenify_wp_doc::HfLine {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    // A plain string means a centred line.
    if let Some(center) = v.as_str() {
        return xenify_wp_doc::HfLine { center: center.into(), ..Default::default() };
    }
    xenify_wp_doc::HfLine { left: s("left"), center: s("center"), right: s("right") }
}

/// A later section's page setup, as JSON (a `::: {.section page=…}` attribute).
pub fn section_page_json(p: &PageSetup) -> String {
    page_json(p).to_string()
}

/// The page setup in a `::: {.section page=…}` attribute.
pub fn section_page_from(json: &str) -> PageSetup {
    let mut p = PageSetup::default();
    if let Ok(v) = serde_json::from_str::<Value>(json) {
        page_from(&v, &mut p);
    }
    p
}

fn page_from(v: &Value, p: &mut PageSetup) {
    if let Some(name) = v["size"].as_str()
        && let Some((_, (w, h))) = PAGE_SIZES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
    {
        (p.width, p.height) = (*w, *h);
    }
    if let Some(w) = length(&v["width"]) {
        p.width = w;
    }
    if let Some(h) = length(&v["height"]) {
        p.height = h;
    }
    let landscape = v["orientation"].as_str().is_some_and(|o| o.eq_ignore_ascii_case("landscape"));
    if landscape != (p.width > p.height) {
        std::mem::swap(&mut p.width, &mut p.height);
    }
    let m = &v["margins"];
    if let Some(all) = length(m) {
        p.margins = xenify_wp_doc::Margins { top: all, bottom: all, left: all, right: all };
    }
    if let Some(n) = v["columns"].as_u64() {
        p.columns = n.clamp(1, 12) as u8;
    }
    p.column_line = v["column-line"].as_bool().unwrap_or(false);
    if let Some(g) = length(&v["column-gap"]) {
        p.column_gap = g;
    }
    if !v["header"].is_null() {
        p.header = hf_from(&v["header"]);
    }
    if !v["footer"].is_null() {
        p.footer = hf_from(&v["footer"]);
    }
    p.different_first = boolish(&v["different-first-page"]).unwrap_or(false);
    if !v["first-header"].is_null() {
        p.first_header = hf_from(&v["first-header"]);
    }
    if !v["first-footer"].is_null() {
        p.first_footer = hf_from(&v["first-footer"]);
    }
    p.start_number = v["start-number"].as_u64().map(|n| n as u32);
    p.different_odd_even = boolish(&v["different-odd-even"]).unwrap_or(false);
    if !v["even-header"].is_null() {
        p.even_header = hf_from(&v["even-header"]);
    }
    if !v["even-footer"].is_null() {
        p.even_footer = hf_from(&v["even-footer"]);
    }
    p.number_format = serde_json::from_value(v["number-format"].clone()).unwrap_or_default();
    p.v_align = serde_json::from_value(v["vertical-align"].clone()).unwrap_or_default();
    for (key, slot) in [
        ("top", &mut p.margins.top),
        ("bottom", &mut p.margins.bottom),
        ("left", &mut p.margins.left),
        ("right", &mut p.margins.right),
    ] {
        if let Some(x) = length(&m[key]) {
            *slot = x;
        }
    }
}

fn align_name(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "justify",
    }
}

pub fn parse_align(s: &str) -> Option<Align> {
    Some(match s.to_ascii_lowercase().as_str() {
        "left" | "start" => Align::Left,
        "center" | "centre" => Align::Center,
        "right" | "end" => Align::Right,
        "justify" | "justified" | "both" => Align::Justify,
        _ => return None,
    })
}

/// Character and paragraph properties as flat `key: value` pairs (shared by
/// style definitions here and paragraph/span attributes in the body).
pub fn props_json(c: &CharFmt, p: &ParaProps, m: &mut Map<String, Value>) {
    // A tracked inserted or deleted paragraph break.
    if let Some(ch) = &p.break_change {
        let kind = match ch.kind {
            xenify_wp_doc::review::ChangeKind::Inserted => "inserted",
            xenify_wp_doc::review::ChangeKind::Deleted => "deleted",
        };
        m.insert("break-change".into(), json!(kind));
        m.insert("break-author".into(), json!(ch.author));
        if !ch.date.is_empty() {
            m.insert("break-date".into(), json!(ch.date));
        }
    }
    if let Some(v) = &c.font {
        m.insert("font".into(), json!(v));
    }
    if let Some(v) = c.size {
        m.insert("size".into(), round(v));
    }
    for (k, v) in [("bold", c.bold), ("italic", c.italic), ("underline", c.underline), ("strike", c.strike)] {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
    if let Some(v) = c.color {
        m.insert("color".into(), json!(v.to_hex()));
    }
    if let Some(v) = c.highlight {
        m.insert("highlight".into(), json!(v.to_hex()));
    }
    if let Some(v) = p.align {
        m.insert("align".into(), json!(align_name(v)));
    }
    for (k, v) in [
        ("space-before", p.space_before),
        ("space-after", p.space_after),
        ("line-height", p.line_height.filter(|_| p.line_exact.is_none())),
        ("indent-left", p.indent_left),
        ("indent-right", p.indent_right),
        ("first-line", p.first_line),
    ] {
        if let Some(v) = v {
            m.insert(k.into(), round(v));
        }
    }
    // Exact line height: "14pt" (a plain number is a multiple).
    if let Some(v) = p.line_exact {
        m.insert("line-height".into(), json!(format!("{}pt", (v * 100.0).round() / 100.0)));
    }
    if let Some(c) = p.shading {
        m.insert("shading".into(), json!(c.to_hex()));
    }
    if let Some(b) = &p.borders {
        let line = |l: &Option<xenify_wp_doc::Border>| l.map(|l| format!("{} {}", round_f(l.width), l.color.to_hex()));
        if b.top == b.bottom && b.top == b.left && b.top == b.right {
            if let Some(v) = line(&b.top) {
                m.insert("border".into(), json!(v));
            }
        } else {
            for (k, v) in [
                ("border-top", &b.top),
                ("border-bottom", &b.bottom),
                ("border-left", &b.left),
                ("border-right", &b.right),
            ] {
                if let Some(v) = line(v) {
                    m.insert(k.into(), json!(v));
                }
            }
        }
    }
    if let Some(tabs) = &p.tabs {
        m.insert("tabs".into(), json!(write_tabs(tabs)));
    }
    for (k, v) in [
        ("keep-with-next", p.keep_with_next),
        ("keep-together", p.keep_together),
        ("page-break-before", p.page_break_before),
        ("column-break-before", p.column_break_before),
        ("widow-control", p.widow_control),
    ] {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
}

fn boolish(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| match v.as_str()?.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" => Some(false),
        _ => None,
    })
}

/// Reads what [`props_json`] writes (also used for `{key=value}` attributes,
/// whose values are all strings).
pub fn props_from(get: &dyn Fn(&str) -> Option<Value>) -> (CharFmt, ParaProps) {
    let s = |k: &str| get(k).and_then(|v| v.as_str().map(str::to_string).or_else(|| Some(v.to_string())));
    let num = |k: &str| get(k).and_then(|v| length(&v).or_else(|| v.as_str().and_then(|x| x.parse().ok())));
    let b = |k: &str| get(k).and_then(|v| boolish(&v));
    let c = CharFmt {
        font: s("font"),
        size: num("size"),
        bold: b("bold"),
        italic: b("italic"),
        underline: b("underline"),
        strike: b("strike"),
        color: s("color").and_then(|x| Rgba::from_hex(&x)),
        highlight: s("highlight").and_then(|x| Rgba::from_hex(&x)),
        ..Default::default()
    };
    let p = ParaProps {
        break_change: s("break-change").map(|k| xenify_wp_doc::review::Change {
            kind: if k == "inserted" {
                xenify_wp_doc::review::ChangeKind::Inserted
            } else {
                xenify_wp_doc::review::ChangeKind::Deleted
            },
            author: s("break-author").unwrap_or_default(),
            date: s("break-date").unwrap_or_default(),
        }),
        align: s("align").and_then(|x| parse_align(&x)),
        space_before: num("space-before"),
        space_after: num("space-after"),
        line_height: get("line-height").and_then(|v| v.as_f64().map(|f| f as f32).or_else(|| v.as_str()?.parse().ok())),
        line_exact: get("line-height")
            .filter(|v| v.as_str().is_some_and(|s| s.trim_end().ends_with(|c: char| c.is_alphabetic())))
            .and_then(|v| length(&v)),
        indent_left: num("indent-left"),
        indent_right: num("indent-right"),
        first_line: num("first-line"),
        tabs: s("tabs").map(|t| read_tabs(&t)),
        keep_with_next: b("keep-with-next"),
        keep_together: b("keep-together"),
        page_break_before: b("page-break-before"),
        column_break_before: b("column-break-before"),
        shading: s("shading").and_then(|x| Rgba::from_hex(&x)),
        borders: {
            let line = |k: &str| {
                s(k).and_then(|v| {
                    let mut it = v.split_whitespace();
                    let width = it.next()?.parse().ok()?;
                    let color = it.next().and_then(Rgba::from_hex).unwrap_or(Rgba::rgb(0, 0, 0));
                    Some(xenify_wp_doc::Border { width, color })
                })
            };
            let all = line("border");
            let b = xenify_wp_doc::ParaBorders {
                top: line("border-top").or(all),
                bottom: line("border-bottom").or(all),
                left: line("border-left").or(all),
                right: line("border-right").or(all),
            };
            (!b.is_empty()).then_some(b)
        },
        widow_control: b("widow-control"),
    };
    (c, p)
}

fn style_json(s: &ParaStyle) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), json!(s.name));
    m.insert("based-on".into(), s.based_on.as_ref().map_or(Value::Null, |b| json!(b)));
    if let Some(n) = &s.next {
        m.insert("next".into(), json!(n));
    }
    if let Some(l) = s.outline_level {
        m.insert("outline-level".into(), json!(l));
    }
    if let Some(l) = &s.list {
        m.insert("list".into(), json!(l));
    }
    props_json(&s.char, &s.para, &mut m);
    Value::Object(m)
}

fn style_from(id: &str, v: &Value) -> ParaStyle {
    let get = |k: &str| v.get(k).cloned().filter(|x| !x.is_null());
    let (char, para) = props_from(&get);
    ParaStyle {
        id: id.into(),
        name: v["name"].as_str().unwrap_or(id).to_string(),
        based_on: if v.get("based-on").is_some() {
            v["based-on"].as_str().map(str::to_string)
        } else if id == StyleSheet::NORMAL {
            None
        } else {
            Some(StyleSheet::NORMAL.into())
        },
        next: v["next"].as_str().map(str::to_string),
        outline_level: v["outline-level"].as_u64().map(|l| l.clamp(1, 9) as u8),
        list: v["list"].as_str().map(str::to_string),
        para,
        char,
    }
}

/// The front matter for `doc` (only what differs from a new document), or
/// `None` when there is nothing to say.
pub fn write(doc: &Document) -> Option<String> {
    let fresh = Document::new();
    let mut m = Map::new();
    let meta = &doc.meta;
    // No title: a Markdown file's name is its title, and a `title:` line at the
    // top of every file was clutter people kept deleting. (One in a file that's
    // opened is still read, for {title} in headers.)
    for (k, v) in [("author", &meta.author), ("audience", &meta.audience), ("tone", &meta.tone)] {
        if !v.is_empty() {
            m.insert(k.into(), json!(v));
        }
    }
    if meta.pageless {
        m.insert("pageless".into(), json!(true));
    }
    if let Some(c) = meta.page_color {
        m.insert("page-color".into(), json!(c.to_hex()));
    }
    if meta.track_changes {
        m.insert("track-changes".into(), json!(true));
    }
    if !meta.changelog.is_empty() {
        m.insert("changelog".into(), serde_json::to_value(&meta.changelog).unwrap_or(Value::Null));
    }
    if meta.cite_style != xenify_wp_doc::cite::CiteStyle::default() {
        m.insert("cite-style".into(), json!(cite_style_key(meta.cite_style)));
    }
    if !meta.sources.is_empty() {
        // Pandoc's `references:` (CSL-JSON fields).
        m.insert("references".into(), Value::Array(meta.sources.iter().map(source_to_csl).collect()));
    }
    if let Some(mc) = &meta.merge {
        let mut mm = Map::new();
        for (k, v) in [("source", &mc.source), ("file-name", &mc.file_name), ("group-by", &mc.group_by)] {
            if !v.is_empty() {
                mm.insert(k.into(), json!(v));
            }
        }
        m.insert("merge".into(), Value::Object(mm));
    }
    if let Some(p) = &meta.proofing_profile {
        m.insert("proofing".into(), json!(p));
    }
    if let Some(t) = &meta.template {
        m.insert("template".into(), json!(t.name));
        m.insert("template-version".into(), json!(t.version));
    }
    if !meta.ignore_words.is_empty() {
        m.insert("ignore-words".into(), json!(meta.ignore_words));
    }
    if !meta.ignore_issues.is_empty() {
        m.insert("ignore-issues".into(), json!(meta.ignore_issues));
    }
    if let Some(s) = doc.sections.first()
        && s.page != fresh.sections[0].page
    {
        m.insert("page".into(), page_json(&s.page));
    }
    if doc.styles.default_font != fresh.styles.default_font {
        m.insert("default-font".into(), json!(doc.styles.default_font));
    }
    if doc.styles.default_size != fresh.styles.default_size {
        m.insert("default-size".into(), round(doc.styles.default_size));
    }
    let mut styles = Map::new();
    for s in &doc.styles.paragraph {
        if fresh.styles.get(&s.id) != Some(s) {
            styles.insert(s.id.clone(), style_json(s));
        }
    }
    if !styles.is_empty() {
        m.insert("styles".into(), Value::Object(styles));
    }
    let lists: Vec<Value> = doc
        .lists
        .iter()
        .filter(|l| fresh.list_def(&l.id) != Some(*l))
        .map(|l| serde_json::to_value(l).expect("lists serialise"))
        .collect();
    if !lists.is_empty() {
        m.insert("lists".into(), Value::Array(lists));
    }
    (!m.is_empty()).then(|| emit_yaml(&Value::Object(m)))
}

/// Applies front matter to a new document.
pub fn read(v: &Value, doc: &mut Document, warnings: &mut Vec<String>) {
    let meta = &mut doc.meta;
    for (k, slot) in [
        ("title", &mut meta.title),
        ("author", &mut meta.author),
        ("audience", &mut meta.audience),
        ("tone", &mut meta.tone),
    ] {
        if let Some(s) = v[k].as_str() {
            *slot = s.to_string();
        } else if let Some(a) = v[k].as_array() {
            *slot = a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ");
        }
    }
    meta.pageless = boolish(&v["pageless"]).unwrap_or(false);
    meta.page_color = v["page-color"].as_str().and_then(Rgba::from_hex);
    meta.track_changes = boolish(&v["track-changes"]).unwrap_or(false);
    meta.changelog = serde_json::from_value(v["changelog"].clone()).unwrap_or_default();
    if let Some(s) = v["cite-style"].as_str() {
        meta.cite_style =
            xenify_wp_doc::cite::CiteStyle::ALL.into_iter().find(|c| cite_style_key(*c) == s).unwrap_or_default();
    }
    if let Some(refs) = v["references"].as_array() {
        meta.sources = refs.iter().filter_map(source_from_csl).collect();
    }
    if v["merge"].is_object() {
        let s = |k: &str| v["merge"][k].as_str().unwrap_or("").to_string();
        meta.merge = Some(xenify_wp_doc::merge::MergeConfig {
            source: s("source"),
            file_name: s("file-name"),
            group_by: s("group-by"),
        });
    }
    meta.proofing_profile = v["proofing"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    meta.template = v["template"].as_str().filter(|s| !s.is_empty()).map(|name| xenify_wp_doc::linked::TemplateLink {
        name: name.to_string(),
        version: v["template-version"].as_str().unwrap_or("").to_string(),
        ..Default::default()
    });
    let strings = |k: &str| {
        v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    meta.ignore_words = strings("ignore-words");
    meta.ignore_issues = strings("ignore-issues");
    if v["page"].is_object()
        && let Some(s) = doc.sections.first_mut()
    {
        page_from(&v["page"], &mut s.page);
    }
    if let Some(f) = v["default-font"].as_str() {
        doc.styles.default_font = f.into();
    }
    if let Some(sz) = length(&v["default-size"]) {
        doc.styles.default_size = sz;
    }
    if let Some(styles) = v["styles"].as_object() {
        for (id, def) in styles {
            if !def.is_object() {
                warnings.push(format!("style {id:?} in the front matter isn't a mapping; ignored"));
                continue;
            }
            let s = style_from(id, def);
            match doc.styles.get_mut(id) {
                Some(slot) => *slot = s,
                None => doc.styles.paragraph.push(s),
            }
        }
    }
    if let Some(lists) = v["lists"].as_array() {
        for l in lists {
            match serde_json::from_value::<ListDef>(l.clone()) {
                Ok(def) => match doc.lists.iter_mut().find(|d| d.id == def.id) {
                    Some(slot) => *slot = def,
                    None => doc.lists.push(def),
                },
                Err(e) => warnings.push(format!("a list definition in the front matter was ignored: {e}")),
            }
        }
    }
}

/// A number written without needless decimals.
fn round_f(v: f32) -> String {
    format!("{}", (v * 100.0).round() / 100.0)
}

/// Tab stops as text: `100 right dots, 200` (points from the margin, then
/// the kind and leader when they aren't left and none).
pub(crate) fn write_tabs(tabs: &[xenify_wp_doc::TabStop]) -> String {
    use xenify_wp_doc::{TabKind, TabLeader};
    tabs.iter()
        .map(|t| {
            let mut s = format!("{}", (t.pos * 100.0).round() / 100.0);
            match t.kind {
                TabKind::Left => {}
                TabKind::Center => s.push_str(" center"),
                TabKind::Right => s.push_str(" right"),
                TabKind::Decimal => s.push_str(" decimal"),
            }
            match t.leader {
                TabLeader::None => {}
                TabLeader::Dots => s.push_str(" dots"),
                TabLeader::Dashes => s.push_str(" dashes"),
                TabLeader::Line => s.push_str(" line"),
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn read_tabs(s: &str) -> Vec<xenify_wp_doc::TabStop> {
    use xenify_wp_doc::{TabKind, TabLeader, TabStop};
    s.split(',')
        .filter_map(|part| {
            let mut words = part.split_whitespace();
            let pos: f32 = words.next()?.parse().ok()?;
            let mut t = TabStop { pos, ..Default::default() };
            for w in words {
                match w {
                    "center" => t.kind = TabKind::Center,
                    "right" => t.kind = TabKind::Right,
                    "decimal" => t.kind = TabKind::Decimal,
                    "dots" => t.leader = TabLeader::Dots,
                    "dashes" => t.leader = TabLeader::Dashes,
                    "line" => t.leader = TabLeader::Line,
                    _ => {}
                }
            }
            Some(t)
        })
        .collect()
}

fn cite_style_key(c: xenify_wp_doc::cite::CiteStyle) -> &'static str {
    use xenify_wp_doc::cite::CiteStyle;
    match c {
        CiteStyle::Apa => "apa",
        CiteStyle::Harvard => "harvard",
        CiteStyle::Ieee => "ieee",
        CiteStyle::Chicago => "chicago",
    }
}

const CSL_KINDS: [(xenify_wp_doc::cite::SourceKind, &str); 6] = {
    use xenify_wp_doc::cite::SourceKind;
    [
        (SourceKind::Book, "book"),
        (SourceKind::Article, "article-journal"),
        (SourceKind::Web, "webpage"),
        (SourceKind::Report, "report"),
        (SourceKind::Chapter, "chapter"),
        (SourceKind::Other, "document"),
    ]
};

/// A source as CSL-JSON (what Pandoc and reference managers use).
pub fn source_to_csl(s: &xenify_wp_doc::cite::Source) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(s.key));
    m.insert("type".into(), json!(CSL_KINDS.iter().find(|(k, _)| *k == s.kind).map_or("document", |(_, n)| n)));
    if !s.authors.is_empty() {
        let a: Vec<Value> = s
            .authors
            .iter()
            .map(|n| {
                let mut o = Map::new();
                o.insert("family".into(), json!(n.family));
                if !n.given.is_empty() {
                    o.insert("given".into(), json!(n.given));
                }
                Value::Object(o)
            })
            .collect();
        m.insert("author".into(), Value::Array(a));
    }
    for (k, v) in [
        ("title", &s.title),
        ("container-title", &s.container),
        ("publisher", &s.publisher),
        ("volume", &s.volume),
        ("issue", &s.issue),
        ("page", &s.pages),
        ("URL", &s.url),
        ("DOI", &s.doi),
    ] {
        if !v.is_empty() {
            m.insert(k.into(), json!(v));
        }
    }
    if let Ok(y) = s.year.trim().parse::<i64>() {
        m.insert("issued".into(), json!({"date-parts": [[y]]}));
    } else if !s.year.is_empty() {
        m.insert("issued".into(), json!({"literal": s.year}));
    }
    if !s.accessed.is_empty() {
        m.insert("accessed".into(), json!({"literal": s.accessed}));
    }
    Value::Object(m)
}

pub fn source_from_csl(v: &Value) -> Option<xenify_wp_doc::cite::Source> {
    use xenify_wp_doc::cite::{Name, Source, SourceKind};
    let st = |k: &str| match &v[k] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let date = |k: &str| {
        let d = &v[k];
        if let Some(y) = d["date-parts"][0][0].as_i64() {
            y.to_string()
        } else if let Some(l) = d["literal"].as_str().or(d["raw"].as_str()) {
            l.to_string()
        } else {
            String::new()
        }
    };
    let key = st("id");
    if key.is_empty() {
        return None;
    }
    let kind = st("type");
    Some(Source {
        key,
        kind: CSL_KINDS
            .iter()
            .find(|(_, n)| *n == kind)
            .map_or(if kind.starts_with("article") { SourceKind::Article } else { SourceKind::Other }, |(k, _)| *k),
        authors: v["author"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|n| Name {
                        family: n["family"].as_str().or(n["literal"].as_str()).unwrap_or("").into(),
                        given: n["given"].as_str().unwrap_or("").into(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        title: st("title"),
        container: st("container-title"),
        publisher: st("publisher"),
        year: date("issued"),
        volume: st("volume"),
        issue: st("issue"),
        pages: st("page"),
        url: st("URL"),
        doi: st("DOI"),
        accessed: date("accessed"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_round_trip() {
        let v = json!({"title": "A: title", "n": 3, "page": {"size": "A4", "margins": {"top": 72}}, "tags": ["a", "true"],
                       "styles": {"quote": {"name": "Quote", "italic": true, "color": "#595959"}}});
        let text = emit_yaml(&v);
        assert_eq!(yaml_to_json(&text).unwrap(), v, "{text}");
    }

    #[test]
    fn lengths_with_units() {
        assert_eq!(length(&json!("1in")), Some(72.0));
        assert_eq!(length(&json!("2.54cm")).map(|v| v.round()), Some(72.0));
        assert_eq!(length(&json!(12)), Some(12.0));
        assert_eq!(length(&json!("12 pt")), Some(12.0));
        assert_eq!(length(&json!("wide")), None);
    }
}
