//! Pictures and charts in Excel files' drawing layers.
//!
//! Opening: pictures are shown where they were (and kept as they are);
//! charts of the common kinds (bars, lines, areas, pies, scatter) become
//! Calc charts. Saving: Calc's charts are written as Excel charts (with
//! their numbers cached, so Excel shows them at once), beside whatever else
//! the drawing layer held.

use crate::book::Book;
use crate::charts::{Chart, ChartKind, Source};
use crate::edit::Range;
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::{Value as Json, json};

/// English Metric Units in a pixel (at 96 per inch).
const EMU: f64 = 9525.0;

/// A cell position with an offset into it (zero-based, as drawings store it).
#[derive(Clone, Copy, Debug, Default)]
pub struct Marker {
    pub col: i32,
    pub col_off: i64,
    pub row: i32,
    pub row_off: i64,
}

/// One thing in a drawing layer.
#[derive(Clone, Debug, Default)]
pub struct Anchor {
    /// The anchor element as written.
    pub raw: String,
    pub from: Marker,
    pub to: Option<Marker>,
    /// Size (EMU), for anchors with one corner.
    pub ext: Option<(i64, i64)>,
    /// A picture's relationship id.
    pub picture: Option<String>,
    /// A chart's relationship id.
    pub chart: Option<String>,
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The anchors of a drawing part.
pub fn anchors(xml: &str) -> Vec<Anchor> {
    let mut out = Vec::new();
    let mut r = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut cur: Option<Anchor> = None;
    let mut start = 0usize;
    let mut which = ""; // "from" or "to"
    let mut field = String::new();
    loop {
        let before = r.buffer_position() as usize;
        let ev = r.read_event();
        let after = r.buffer_position() as usize;
        match ev {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref().as_bytes()).into_owned();
                let l = local(&name).to_string();
                depth += 1;
                if depth == 2 && l.ends_with("Anchor") {
                    cur = Some(Anchor::default());
                    start = before;
                }
                match l.as_str() {
                    "from" => which = "from",
                    "to" => which = "to",
                    "col" | "colOff" | "row" | "rowOff" => field = l,
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref().as_bytes()).into_owned();
                let l = local(&name);
                if let Some(a) = cur.as_mut() {
                    let at: Vec<(String, String)> = e
                        .attributes()
                        .flatten()
                        .map(|x| {
                            (
                                x.key.as_ref().to_string(),
                                x.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .map(|v| v.into_owned())
                                    .unwrap_or_default(),
                            )
                        })
                        .collect();
                    let get = |k: &str| {
                        at.iter().find(|(n, _)| n == k || n.ends_with(&format!(":{k}"))).map(|(_, v)| v.clone())
                    };
                    match l {
                        "ext" if a.ext.is_none() && depth == 2 => {
                            a.ext = Some((
                                get("cx").and_then(|v| v.parse().ok()).unwrap_or(0),
                                get("cy").and_then(|v| v.parse().ok()).unwrap_or(0),
                            ));
                        }
                        "blip" => a.picture = get("embed"),
                        "chart" => a.chart = get("id"),
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(a) = cur.as_mut() {
                    let v: i64 = t.as_ref().trim().parse().unwrap_or(0);
                    let m = if which == "to" { a.to.get_or_insert_with(Marker::default) } else { &mut a.from };
                    match field.as_str() {
                        "col" => m.col = v as i32,
                        "colOff" => m.col_off = v,
                        "row" => m.row = v as i32,
                        "rowOff" => m.row_off = v,
                        _ => {}
                    }
                }
                field.clear();
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref().as_bytes()).into_owned();
                if depth == 2
                    && local(&name).ends_with("Anchor")
                    && let Some(mut a) = cur.take()
                {
                    a.raw = xml[start..after].to_string();
                    out.push(a);
                }
                if matches!(local(&name), "from" | "to") {
                    which = "";
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Where a marker is on the sheet, in pixels from its top left.
fn px(book: &Book, index: u32, m: Marker) -> (f64, f64) {
    let mut x = 0.0;
    for c in 1..=m.col {
        x += book.model.get_column_width(index, c).unwrap_or(90.0);
    }
    let mut y = 0.0;
    for r in 1..=m.row {
        y += book.model.get_row_height(index, r).unwrap_or(25.0);
    }
    (x + m.col_off as f64 / EMU, y + m.row_off as f64 / EMU)
}

/// The cell (1-based) and offset (pixels) a sheet position falls in.
fn cell_at(book: &Book, index: u32, x: f64, y: f64) -> (i32, i32, f64, f64) {
    let (mut col, mut left) = (1, 0.0);
    loop {
        let w = book.model.get_column_width(index, col).unwrap_or(90.0);
        if left + w > x || col >= 16384 {
            break;
        }
        left += w;
        col += 1;
    }
    let (mut row, mut top) = (1, 0.0);
    loop {
        let h = book.model.get_row_height(index, row).unwrap_or(25.0);
        if top + h > y || row >= 1_048_576 {
            break;
        }
        top += h;
        row += 1;
    }
    (row, col, x - left, y - top)
}

/// An anchor's top left (row, col, dx, dy) and size in pixels.
pub fn place(book: &Book, index: u32, a: &Anchor) -> (i32, i32, f64, f64, f64, f64) {
    let (x1, y1) = px(book, index, a.from);
    let (w, h) = match (a.to, a.ext) {
        (Some(to), _) => {
            let (x2, y2) = px(book, index, to);
            (x2 - x1, y2 - y1)
        }
        (None, Some((cx, cy))) => (cx as f64 / EMU, cy as f64 / EMU),
        _ => (320.0, 200.0),
    };
    (a.from.row + 1, a.from.col + 1, a.from.col_off as f64 / EMU, a.from.row_off as f64 / EMU, w.max(20.0), h.max(20.0))
}

// ----- Charts in -----

/// A cell range written in a chart: Sheet1!$B$2:$B$11 → (sheet name, top, left, bottom, right).
fn parse_ref(f: &str) -> Option<(String, i32, i32, i32, i32)> {
    let f = f.trim().trim_start_matches('=').trim_start_matches('(').trim_end_matches(')');
    let (sheet, cells) = f.rsplit_once('!')?;
    let sheet = sheet.trim_matches('\'').replace("''", "'");
    let (a, b) = cells.split_once(':').unwrap_or((cells, cells));
    let (r1, c1) = crate::tables::parse_cell_pub(a)?;
    let (r2, c2) = crate::tables::parse_cell_pub(b)?;
    Some((sheet, r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

/// What a chart part says: its kind, title and each series' (name, categories, values) references.
struct Parsed {
    kind: Option<ChartKind>,
    title: String,
    series: Vec<(Option<String>, Option<String>, Option<String>)>,
}

fn parse_chart(xml: &str) -> Parsed {
    let mut p = Parsed { kind: None, title: String::new(), series: Vec::new() };
    let mut r = Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut in_title = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref().as_bytes()).into_owned();
                let l = local(&name).to_string();
                if p.kind.is_none() && path.iter().any(|x| x == "plotArea") {
                    p.kind = match l.as_str() {
                        "barChart" | "bar3DChart" => Some(ChartKind::Bar),
                        "lineChart" | "line3DChart" | "stockChart" => Some(ChartKind::Line),
                        "areaChart" | "area3DChart" => Some(ChartKind::Area),
                        "pieChart" | "pie3DChart" | "doughnutChart" | "ofPieChart" => Some(ChartKind::Pie),
                        "scatterChart" | "bubbleChart" => Some(ChartKind::Scatter),
                        _ => None,
                    };
                }
                if l == "title" && path.last().is_some_and(|x| x == "chart") {
                    in_title = true;
                }
                if l == "ser" {
                    p.series.push((None, None, None));
                }
                path.push(l);
            }
            // Bars across, or stacked.
            Ok(Event::Empty(e)) if p.kind == Some(ChartKind::Bar) => {
                let name = String::from_utf8_lossy(e.name().as_ref().as_bytes()).into_owned();
                let val = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == "val")
                    .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
                    .unwrap_or_default();
                match (local(&name), val.as_str()) {
                    ("barDir", "bar") => p.kind = Some(ChartKind::Sideways),
                    ("grouping", "stacked" | "percentStacked") => p.kind = Some(ChartKind::Stacked),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.as_ref().to_string();
                let last = path.last().map(String::as_str).unwrap_or("");
                if in_title && last == "t" {
                    p.title.push_str(&text);
                } else if last == "f"
                    && path.iter().any(|x| x == "ser")
                    && let Some(s) = p.series.last_mut()
                {
                    if path.iter().any(|x| x == "tx") {
                        s.0 = Some(text);
                    } else if path.iter().any(|x| x == "cat" || x == "xVal") {
                        s.1 = Some(text);
                    } else if path.iter().any(|x| x == "val" || x == "yVal") {
                        s.2 = Some(text);
                    }
                }
            }
            Ok(Event::End(_)) => {
                if path.last().is_some_and(|x| x == "title") {
                    in_title = false;
                }
                path.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    p
}

/// A Calc chart for an Excel chart, when it reads columns of one table or block.
pub fn import_chart(book: &Book, xml: &str, id: u32, sheet: u32, at: (i32, i32, f64, f64, f64, f64)) -> Option<Chart> {
    let p = parse_chart(xml);
    let kind = p.kind?;
    let vals: Vec<(String, i32, i32, i32, i32)> =
        p.series.iter().filter_map(|s| s.2.as_deref().and_then(parse_ref)).collect();
    if vals.is_empty() || vals.len() != p.series.len() {
        return None;
    }
    let (sheet_name, top, _, bottom, _) = vals[0].clone();
    // One column each, the same rows, on one sheet.
    if vals.iter().any(|v| v.0 != sheet_name || v.1 != top || v.3 != bottom || v.2 != v.4) {
        return None;
    }
    let source_sheet = book.model.workbook.worksheets.iter().find(|w| w.get_name() == sheet_name)?.sheet_id;
    let cat = p.series[0].1.as_deref().and_then(parse_ref);
    let x_col = cat.as_ref().filter(|c| c.0 == sheet_name && c.1 == top && c.3 == bottom && c.2 == c.4).map(|c| c.2);
    let cols: Vec<i32> = vals.iter().map(|v| v.2).collect();
    // Inside a table: by its column names (it grows with the table).
    if let Some(t) = crate::tables::at(book, source_sheet, top, cols[0]) {
        let name = |c: i32| t.columns.get((c - t.left) as usize).map(|x| x.name.clone());
        if cols.iter().all(|c| *c >= t.left && *c <= t.right) {
            let series: Vec<String> = cols.iter().filter_map(|c| name(*c)).collect();
            let source = Source {
                sheet: source_sheet,
                table: Some(t.name.clone()),
                x: x_col.and_then(name),
                series,
                header: true,
                ..Default::default()
            };
            return Some(Chart {
                id,
                sheet,
                row: at.0,
                col: at.1,
                dx: at.2,
                dy: at.3,
                w: at.4,
                h: at.5,
                kind,
                title: p.title,
                source,
            });
        }
    }
    // A block: names in the row above, point names in the column to the left of the numbers.
    let mut sorted = cols.clone();
    sorted.sort();
    let contiguous = sorted.windows(2).all(|w| w[1] == w[0] + 1);
    let labels = x_col.is_some_and(|x| x + 1 == sorted[0]);
    if !contiguous || (x_col.is_some() && !labels) || top < 2 {
        return None;
    }
    let left = if labels { sorted[0] - 1 } else { sorted[0] };
    let range = Range { top: top - 1, left, bottom, right: *sorted.last()? };
    let source = Source { sheet: source_sheet, range: Some(range), labels, header: true, ..Default::default() };
    Some(Chart { id, sheet, row: at.0, col: at.1, dx: at.2, dy: at.3, w: at.4, h: at.5, kind, title: p.title, source })
}

// ----- Charts out -----

use xenify_util::esc;

fn quote(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

fn col_letters(c: i32) -> String {
    crate::tables::column_letters(c)
}

/// Series colours, as Calc draws them.
const COLOURS: [&str; 6] = ["C2410C", "2A78D6", "1BAF7A", "6250D6", "EDA100", "E87BA4"];

/// An Excel chart part for a Calc chart, or none for kinds Excel charts can't
/// show the same way (histograms, box charts).
pub fn chart_xml(book: &Book, c: &Chart) -> Option<String> {
    let data: Json = crate::charts::data(book, c).ok()?;
    let sheet_name = quote(&book.sheet_name(c.source.sheet).ok()?);
    let grouped = c.source.grouped() && !c.kind.each_row();
    // The cells each series reads (none for added-up charts: their numbers go in as they are).
    let (rows, x_col, cols): ((i32, i32), Option<i32>, crate::charts::SeriesColumns) = if grouped {
        ((0, 0), None, Vec::new())
    } else {
        let (top, bottom, x, cols) = crate::charts::columns(book, &c.source).ok()?;
        ((top, bottom), x, cols)
    };
    // Table columns and headed cells have a cell naming each series.
    let named_by_cell = !grouped && (c.source.table.is_some() || c.source.header);
    let range = |k: i32| format!("{sheet_name}!${}${}:${}${}", col_letters(k), rows.0, col_letters(k), rows.1);
    let header_row = rows.0 - 1;
    let labels: Vec<String> = data["labels"].as_array()?.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();
    let pts_str = |vals: &[String]| -> String {
        vals.iter().enumerate().map(|(k, v)| format!("<c:pt idx=\"{k}\"><c:v>{}</c:v></c:pt>", esc(v))).collect()
    };
    let pts_num = |vals: &[Json]| -> String {
        vals.iter()
            .enumerate()
            .filter_map(|(k, v)| v.as_f64().map(|n| format!("<c:pt idx=\"{k}\"><c:v>{n}</c:v></c:pt>")))
            .collect()
    };
    let str_cache =
        |vals: &[String]| format!("<c:strCache><c:ptCount val=\"{}\"/>{}</c:strCache>", vals.len(), pts_str(vals));
    let num_cache = |vals: &[Json]| {
        format!(
            "<c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{}</c:numCache>",
            vals.len(),
            pts_num(vals)
        )
    };
    let all_series: Vec<(String, Vec<Json>)> = data["series"]
        .as_array()?
        .iter()
        .map(|s| (s["name"].as_str().unwrap_or("").to_string(), s["values"].as_array().cloned().unwrap_or_default()))
        .collect();
    let scatter = c.kind == ChartKind::Scatter;
    let mut series = String::new();
    for (k, (name, values)) in all_series.iter().enumerate() {
        let col = cols.get(k).map(|x| x.1);
        if !grouped && col.is_none() {
            continue;
        }
        let colour = COLOURS[k % COLOURS.len()];
        let fill = match c.kind {
            ChartKind::Line => format!(
                "<c:spPr><a:ln w=\"25400\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"{colour}\"/></a:solidFill></a:ln></c:spPr>"
            ),
            ChartKind::Scatter => "<c:spPr><a:ln w=\"25400\"><a:noFill/></a:ln></c:spPr>".to_string(),
            ChartKind::Pie => String::new(),
            _ => format!("<c:spPr><a:solidFill><a:srgbClr val=\"{colour}\"/></a:solidFill></c:spPr>"),
        };
        let tx = match col {
            Some(col) if named_by_cell => format!(
                "<c:tx><c:strRef><c:f>{sheet_name}!${}${header_row}</c:f>{}</c:strRef></c:tx>",
                col_letters(col),
                str_cache(std::slice::from_ref(name))
            ),
            _ => format!("<c:tx><c:v>{}</c:v></c:tx>", esc(name)),
        };
        let cat = match (x_col, scatter, grouped) {
            (_, _, true) => {
                format!("<c:cat><c:strLit><c:ptCount val=\"{}\"/>{}</c:strLit></c:cat>", labels.len(), pts_str(&labels))
            }
            (Some(x), true, _) => format!(
                "<c:xVal><c:numRef><c:f>{}</c:f>{}</c:numRef></c:xVal>",
                range(x),
                num_cache(data["x"].as_array().map(|v| v.as_slice()).unwrap_or(&[]))
            ),
            (Some(x), false, _) => {
                format!("<c:cat><c:strRef><c:f>{}</c:f>{}</c:strRef></c:cat>", range(x), str_cache(&labels))
            }
            (None, _, _) => String::new(),
        };
        let val_tag = if scatter { "yVal" } else { "val" };
        let val = match col {
            Some(col) => format!("<c:numRef><c:f>{}</c:f>{}</c:numRef>", range(col), num_cache(values)),
            None => format!(
                "<c:numLit><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{}</c:numLit>",
                values.len(),
                pts_num(values)
            ),
        };
        let extra = match c.kind {
            ChartKind::Line => "<c:marker><c:symbol val=\"none\"/></c:marker>",
            ChartKind::Scatter => "<c:marker><c:symbol val=\"circle\"/><c:size val=\"6\"/></c:marker>",
            _ => "",
        };
        let pie_points: String = if c.kind == ChartKind::Pie {
            (0..labels.len())
                .map(|p| format!("<c:dPt><c:idx val=\"{p}\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr></c:dPt>", COLOURS[p % COLOURS.len()]))
                .collect()
        } else {
            String::new()
        };
        series.push_str(&format!(
            "<c:ser><c:idx val=\"{k}\"/><c:order val=\"{k}\"/>{tx}{fill}{extra}{pie_points}{cat}<c:{val_tag}>{val}</c:{val_tag}>{}</c:ser>",
            if c.kind == ChartKind::Line || scatter { "<c:smooth val=\"0\"/>" } else { "" }
        ));
    }
    let n_series = all_series.len();
    let axes = "<c:axId val=\"500100\"/><c:axId val=\"500200\"/>";
    let grid = "<c:majorGridlines><c:spPr><a:ln w=\"6350\"><a:solidFill><a:srgbClr val=\"EFECE8\"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>";
    let (plot, axes_xml) = match c.kind {
        ChartKind::Bar | ChartKind::Sideways | ChartKind::Stacked => {
            let dir = if c.kind == ChartKind::Sideways { "bar" } else { "col" };
            let (grouping, overlap) = if c.kind == ChartKind::Stacked { ("stacked", 100) } else { ("clustered", -6) };
            (
                format!(
                    "<c:barChart><c:barDir val=\"{dir}\"/><c:grouping val=\"{grouping}\"/><c:varyColors val=\"0\"/>{series}<c:gapWidth val=\"60\"/><c:overlap val=\"{overlap}\"/>{axes}</c:barChart>"
                ),
                true,
            )
        }
        ChartKind::Line => (
            format!(
                "<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{series}<c:marker val=\"1\"/>{axes}</c:lineChart>"
            ),
            true,
        ),
        ChartKind::Area => (
            format!("<c:areaChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{series}{axes}</c:areaChart>"),
            true,
        ),
        ChartKind::Pie => (
            format!(
                "<c:doughnutChart><c:varyColors val=\"1\"/>{series}<c:firstSliceAng val=\"0\"/><c:holeSize val=\"58\"/></c:doughnutChart>"
            ),
            false,
        ),
        ChartKind::Scatter => (
            format!(
                "<c:scatterChart><c:scatterStyle val=\"lineMarker\"/><c:varyColors val=\"0\"/>{series}{axes}</c:scatterChart>"
            ),
            true,
        ),
        ChartKind::Histogram | ChartKind::Box | ChartKind::Heatmap => return None,
    };
    let axes_xml = if axes_xml {
        let first = if c.kind == ChartKind::Scatter {
            "<c:valAx><c:axId val=\"500100\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"500200\"/><c:crosses val=\"autoZero\"/></c:valAx>".to_string()
        } else {
            "<c:catAx><c:axId val=\"500100\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"500200\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/></c:catAx>".to_string()
        };
        format!(
            "{first}<c:valAx><c:axId val=\"500200\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/>{grid}<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:crossAx val=\"500100\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx>"
        )
    } else {
        String::new()
    };
    let title = if c.title.is_empty() {
        "<c:autoTitleDeleted val=\"1\"/>".to_string()
    } else {
        format!(
            "<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1300\" b=\"1\"/></a:pPr><a:r><a:rPr lang=\"en-GB\" sz=\"1300\" b=\"1\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
            esc(&c.title)
        )
    };
    let legend = if n_series > 1 || c.kind == ChartKind::Pie {
        "<c:legend><c:legendPos val=\"t\"/><c:overlay val=\"0\"/></c:legend>"
    } else {
        ""
    };
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><c:roundedCorners val=\"0\"/><c:chart>{title}<c:plotArea><c:layout/>{plot}{axes_xml}</c:plotArea>{legend}<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart><c:spPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:ln><a:noFill/></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1000\"><a:solidFill><a:srgbClr val=\"57534E\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"en-GB\"/></a:p></c:txPr></c:chartSpace>"
    ))
}

/// The anchor that places chart `c` (relationship `rid`, drawing object `n`).
pub fn chart_anchor(book: &Book, c: &Chart, rid: &str, n: usize) -> Option<String> {
    let i = book.index(c.sheet).ok()?;
    let start = px(book, i, Marker { col: c.col - 1, col_off: 0, row: c.row - 1, row_off: 0 });
    let (x1, y1) = (start.0 + c.dx, start.1 + c.dy);
    let (r2, c2, dx2, dy2) = cell_at(book, i, x1 + c.w, y1 + c.h);
    let m = |col: i32, cdx: f64, row: i32, rdy: f64| {
        format!(
            "<xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff>",
            col - 1,
            (cdx * EMU) as i64,
            row - 1,
            (rdy * EMU) as i64
        )
    };
    Some(format!(
        "<xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from>{}</xdr:from><xdr:to>{}</xdr:to><xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{}\" name=\"{}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"{rid}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>",
        m(c.col, c.dx, c.row, c.dy),
        m(c2, dx2, r2, dy2),
        n + 100,
        esc(if c.title.is_empty() { "Chart" } else { &c.title }),
    ))
}

/// The anchor that places picture `p` (relationship `rid` to its bytes, drawing object `n`).
pub fn picture_anchor(book: &Book, p: &crate::pictures::Picture, rid: &str, n: usize) -> Option<String> {
    let i = book.index(p.sheet).ok()?;
    let start = px(book, i, Marker { col: p.col - 1, col_off: 0, row: p.row - 1, row_off: 0 });
    let (x1, y1) = (start.0 + p.dx, start.1 + p.dy);
    let (r2, c2, dx2, dy2) = cell_at(book, i, x1 + p.w, y1 + p.h);
    let m = |col: i32, cdx: f64, row: i32, rdy: f64| {
        format!(
            "<xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff>",
            col - 1,
            (cdx * EMU) as i64,
            row - 1,
            (rdy * EMU) as i64
        )
    };
    Some(format!(
        "<xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from>{}</xdr:from><xdr:to>{}</xdr:to><xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{}\" name=\"{}\" descr=\"{}\"/><xdr:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:twoCellAnchor>",
        m(p.col, p.dx, p.row, p.dy),
        m(c2, dx2, r2, dy2),
        n + 300,
        esc(if p.name.is_empty() { "Picture" } else { &p.name }),
        esc(&p.alt),
        (p.w * EMU) as i64,
        (p.h * EMU) as i64,
    ))
}

pub const DRAWING_START: &str = "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">";

/// A picture for the screens: where it goes and its bytes.
pub fn picture_json(book: &Book, index: u32, a: &Anchor, bytes: &[u8], name: &str) -> Json {
    use base64::Engine;
    let (row, col, dx, dy, w, h) = place(book, index, a);
    let mime = match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "emf" | "wmf" => "",
        _ => "",
    };
    json!({ "row": row, "col": col, "dx": dx, "dy": dy, "w": w, "h": h, "mime": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes), "name": name })
}
