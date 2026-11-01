//! Citations and bibliographies: the document's sources (books, articles,
//! web pages, reports), and how they're cited in the text and listed at
//! the end in APA 7, Harvard, IEEE or Chicago (author-date) style. The
//! formatting rules are written for Xenify from the styles' published
//! conventions; no style files are copied.

use crate::{Document, Field};
use serde::{Deserialize, Serialize};

/// A person's name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Name {
    pub family: String,
    pub given: String,
}

impl Name {
    /// "Smith, J. A."
    fn family_initials(&self) -> String {
        let initials: Vec<String> = self
            .given
            .split([' ', '-'])
            .filter(|w| !w.is_empty())
            .map(|w| format!("{}.", w.chars().next().unwrap_or(' ')))
            .collect();
        if initials.is_empty() { self.family.clone() } else { format!("{}, {}", self.family, initials.join(" ")) }
    }

    /// "J. A. Smith"
    fn initials_family(&self) -> String {
        let initials: Vec<String> = self
            .given
            .split([' ', '-'])
            .filter(|w| !w.is_empty())
            .map(|w| format!("{}.", w.chars().next().unwrap_or(' ')))
            .collect();
        if initials.is_empty() { self.family.clone() } else { format!("{} {}", initials.join(" "), self.family) }
    }

    /// "Smith, John"
    fn family_given(&self) -> String {
        if self.given.is_empty() { self.family.clone() } else { format!("{}, {}", self.family, self.given) }
    }

    /// "John Smith"
    fn given_family(&self) -> String {
        if self.given.is_empty() { self.family.clone() } else { format!("{} {}", self.given, self.family) }
    }
}

/// What kind of work a source is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    Book,
    Article,
    Web,
    Report,
    Chapter,
    Other,
}

impl SourceKind {
    pub const ALL: [SourceKind; 6] = [
        SourceKind::Book,
        SourceKind::Article,
        SourceKind::Chapter,
        SourceKind::Report,
        SourceKind::Web,
        SourceKind::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Book => "Book",
            SourceKind::Article => "Journal article",
            SourceKind::Web => "Web page",
            SourceKind::Report => "Report",
            SourceKind::Chapter => "Book chapter",
            SourceKind::Other => "Other",
        }
    }
}

/// A source the document cites (the fields of CSL-JSON that matter here).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    /// The key citations use (e.g. "smith2020").
    pub key: String,
    pub kind: SourceKind,
    pub authors: Vec<Name>,
    pub title: String,
    /// Journal, book (for a chapter), or website name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub container: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub year: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub volume: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub issue: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pages: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub doi: String,
    /// When a web page was read (YYYY-MM-DD).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub accessed: String,
}

/// How citations and the bibliography look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiteStyle {
    #[default]
    Apa,
    Harvard,
    Ieee,
    Chicago,
}

impl CiteStyle {
    pub const ALL: [CiteStyle; 4] = [CiteStyle::Apa, CiteStyle::Harvard, CiteStyle::Ieee, CiteStyle::Chicago];

    pub fn name(self) -> &'static str {
        match self {
            CiteStyle::Apa => "APA 7",
            CiteStyle::Harvard => "Harvard",
            CiteStyle::Ieee => "IEEE",
            CiteStyle::Chicago => "Chicago (author-date)",
        }
    }
}

/// The year, or "n.d." (no date).
fn year(s: &Source) -> &str {
    if s.year.trim().is_empty() { "n.d." } else { s.year.trim() }
}

/// Families of the authors as an in-text citation names them.
fn in_text_names(s: &Source, style: CiteStyle) -> String {
    let fam: Vec<&str> = s.authors.iter().map(|a| a.family.as_str()).filter(|f| !f.is_empty()).collect();
    let and = if style == CiteStyle::Apa { "&" } else { "and" };
    match fam.len() {
        0 => {
            // No author: a short title.
            let t: Vec<&str> = s.title.split_whitespace().take(4).collect();
            format!("“{}”", t.join(" "))
        }
        1 => fam[0].to_string(),
        2 => format!("{} {and} {}", fam[0], fam[1]),
        _ => format!("{} et al.", fam[0]),
    }
}

/// The order sources are first cited in (IEEE numbers).
pub fn cited_order(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in doc.paragraphs() {
        for run in &p.runs {
            if let Some(Field::Citation { keys, .. }) = &run.fmt.field {
                for k in keys {
                    if !out.contains(k) {
                        out.push(k.clone());
                    }
                }
            }
        }
    }
    out
}

/// A citation of `keys` in the text, in the document's style: "(Smith & Lee,
/// 2020, p. 4)", "[2], [5]". An unknown source shows as its key.
pub fn citation(doc: &Document, keys: &[String], locator: &str) -> String {
    let style = doc.meta.cite_style;
    let find = |k: &String| doc.meta.sources.iter().find(|s| &s.key == k);
    if style == CiteStyle::Ieee {
        let order = cited_order(doc);
        let nums: Vec<String> =
            keys.iter().map(|k| order.iter().position(|o| o == k).map_or(k.clone(), |i| (i + 1).to_string())).collect();
        let loc = if locator.is_empty() { String::new() } else { format!(", {locator}") };
        return format!("[{}{loc}]", nums.join(", "));
    }
    let parts: Vec<String> = keys
        .iter()
        .map(|k| match find(k) {
            None => k.clone(),
            Some(s) => {
                let names = in_text_names(s, style);
                match style {
                    CiteStyle::Apa => format!("{names}, {}", year(s)),
                    _ => format!("{names} {}", year(s)),
                }
            }
        })
        .collect();
    let loc = match (style, locator.trim()) {
        (_, "") => String::new(),
        (CiteStyle::Chicago, l) => format!(", {}", l.trim_start_matches("p. ").trim_start_matches("pp. ")),
        (_, l) => format!(", {l}"),
    };
    format!("({}{loc})", parts.join("; "))
}

/// One bibliography entry as runs of (text, italic).
pub fn entry(s: &Source, style: CiteStyle, number: usize) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut push = |t: String, i: bool| {
        if !t.is_empty() {
            out.push((t, i));
        }
    };
    let list = |names: Vec<String>, and: &str| -> String {
        match names.len() {
            0 => String::new(),
            1 => names[0].clone(),
            2 => format!("{}, {and} {}", names[0], names[1]),
            n => format!("{}, {and} {}", names[..n - 1].join(", "), names[n - 1]),
        }
    };
    let title = s.title.trim().trim_end_matches('.').to_string();
    let italic_title = matches!(s.kind, SourceKind::Book | SourceKind::Report | SourceKind::Web);
    let link = if !s.doi.is_empty() {
        format!("https://doi.org/{}", s.doi.trim_start_matches("https://doi.org/"))
    } else {
        s.url.clone()
    };
    let vol = || {
        let mut v = s.volume.clone();
        if !s.issue.is_empty() {
            v.push_str(&format!("({})", s.issue));
        }
        v
    };
    match style {
        CiteStyle::Apa => {
            let names = list(s.authors.iter().map(Name::family_initials).collect(), "&");
            let names = if names.is_empty() { String::new() } else { format!("{names} ") };
            push(format!("{names}({}). ", year(s)), false);
            if italic_title {
                push(title.clone(), true);
                push(". ".into(), false);
            } else {
                push(format!("{title}. "), false);
            }
            if !s.container.is_empty() {
                let in_ = if s.kind == SourceKind::Chapter { "In " } else { "" };
                push(in_.to_string(), false);
                push(s.container.clone(), true);
                if s.kind == SourceKind::Article && !s.volume.is_empty() {
                    push(", ".into(), false);
                    push(s.volume.clone(), true);
                    if !s.issue.is_empty() {
                        push(format!("({})", s.issue), false);
                    }
                }
                if !s.pages.is_empty() {
                    push(format!(", {}", s.pages), false);
                }
                push(". ".into(), false);
            }
            if !s.publisher.is_empty() && s.kind != SourceKind::Article {
                push(format!("{}. ", s.publisher), false);
            }
            push(link, false);
        }
        CiteStyle::Harvard | CiteStyle::Chicago => {
            let names = if style == CiteStyle::Harvard {
                list(s.authors.iter().map(Name::family_initials).collect(), "and")
            } else {
                let mut v: Vec<String> = Vec::new();
                for (i, a) in s.authors.iter().enumerate() {
                    v.push(if i == 0 { a.family_given() } else { a.given_family() });
                }
                list(v, "and")
            };
            let names = if names.is_empty() { String::new() } else { format!("{names} ") };
            if style == CiteStyle::Harvard {
                push(format!("{names}({}) ", year(s)), false);
            } else {
                push(format!("{}{}. ", names, year(s)), false);
            }
            if italic_title {
                push(title.clone(), true);
                push(". ".into(), false);
            } else {
                push(format!("‘{title}’. "), false);
            }
            if !s.container.is_empty() {
                push(s.container.clone(), true);
                let v = vol();
                if !v.is_empty() {
                    push(format!(", {v}"), false);
                }
                if !s.pages.is_empty() {
                    push(format!(", pp. {}", s.pages), false);
                }
                push(". ".into(), false);
            }
            if !s.publisher.is_empty() && s.kind != SourceKind::Article {
                push(format!("{}. ", s.publisher), false);
            }
            if !link.is_empty() {
                push(if style == CiteStyle::Harvard { format!("Available at: {link}") } else { link }, false);
                if !s.accessed.is_empty() && style == CiteStyle::Harvard {
                    push(format!(" (Accessed: {})", s.accessed), false);
                }
            }
        }
        CiteStyle::Ieee => {
            push(format!("[{number}] "), false);
            let names = list(s.authors.iter().map(Name::initials_family).collect(), "and");
            if !names.is_empty() {
                push(format!("{names}, "), false);
            }
            if italic_title {
                push(title.clone(), true);
                push(", ".into(), false);
            } else {
                push(format!("“{title},” "), false);
            }
            if !s.container.is_empty() {
                push(s.container.clone(), true);
                if !s.volume.is_empty() {
                    push(format!(", vol. {}", s.volume), false);
                }
                if !s.issue.is_empty() {
                    push(format!(", no. {}", s.issue), false);
                }
                if !s.pages.is_empty() {
                    push(format!(", pp. {}", s.pages), false);
                }
                push(", ".into(), false);
            }
            if !s.publisher.is_empty() && s.kind != SourceKind::Article {
                push(format!("{}, ", s.publisher), false);
            }
            push(format!("{}.", year(s)), false);
            if !link.is_empty() {
                push(format!(" [Online]. Available: {link}"), false);
            }
        }
    }
    // Tidy the end.
    if let Some(last) = out.last_mut() {
        let t = last.0.trim_end().to_string();
        last.0 = if t.ends_with('.') || t.starts_with("http") { t } else { format!("{t}.") };
    }
    out
}

/// The bibliography: every cited source (all sources when nothing is
/// cited yet), as entries in the style's order.
pub fn bibliography(doc: &Document) -> Vec<Vec<(String, bool)>> {
    let style = doc.meta.cite_style;
    let order = cited_order(doc);
    let mut sources: Vec<&Source> = if order.is_empty() {
        doc.meta.sources.iter().collect()
    } else {
        order.iter().filter_map(|k| doc.meta.sources.iter().find(|s| &s.key == k)).collect()
    };
    if style != CiteStyle::Ieee {
        sources.sort_by_key(|s| {
            let first = s.authors.first().map_or(s.title.to_lowercase(), |a| a.family.to_lowercase());
            (first, s.year.clone())
        });
    }
    sources.iter().enumerate().map(|(i, s)| entry(s, style, i + 1)).collect()
}

/// A key for a new source: family name and year, made unique.
pub fn new_key(doc: &Document, s: &Source) -> String {
    let base: String = s
        .authors
        .first()
        .map_or_else(|| s.title.split_whitespace().next().unwrap_or("source").to_string(), |a| a.family.clone())
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let base = format!("{}{}", if base.is_empty() { "source" } else { &base }, s.year.trim());
    let mut key = base.clone();
    let mut n = 1;
    while doc.meta.sources.iter().any(|x| x.key == key) {
        key = format!("{base}{}", (b'a' + (n % 26) as u8) as char);
        n += 1;
    }
    key
}

/// Reads BibTeX entries (@book{key, author = {…}, title = {…}, …}).
pub fn read_bibtex(text: &str) -> Vec<Source> {
    let mut out = Vec::new();
    for chunk in text.split('@').skip(1) {
        let Some(brace) = chunk.find('{') else { continue };
        let kind = chunk[..brace].trim().to_lowercase();
        let body = &chunk[brace + 1..];
        let Some(comma) = body.find(',') else { continue };
        let mut s = Source {
            key: body[..comma].trim().to_string(),
            kind: match kind.as_str() {
                "article" => SourceKind::Article,
                "inbook" | "incollection" => SourceKind::Chapter,
                "techreport" | "report" => SourceKind::Report,
                "online" | "misc" if body.contains("url") => SourceKind::Web,
                "book" => SourceKind::Book,
                _ => SourceKind::Other,
            },
            ..Default::default()
        };
        // field = {value} or "value", nested braces allowed.
        let mut rest = &body[comma + 1..];
        while let Some(eq) = rest.find('=') {
            let name = rest[..eq].trim().trim_start_matches(',').trim().to_lowercase();
            let after = rest[eq + 1..].trim_start();
            let (value, used) = if let Some(inner) = after.strip_prefix('{') {
                let mut depth = 1;
                let mut end = inner.len();
                for (i, c) in inner.char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                (inner[..end].replace(['{', '}'], ""), &inner[(end + 1).min(inner.len())..])
            } else if let Some(inner) = after.strip_prefix('"') {
                let end = inner.find('"').unwrap_or(inner.len());
                (inner[..end].to_string(), &inner[(end + 1).min(inner.len())..])
            } else {
                let end = after.find([',', '}']).unwrap_or(after.len());
                (after[..end].trim().to_string(), &after[end..])
            };
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            match name.as_str() {
                "author" | "editor" if s.authors.is_empty() => {
                    s.authors = value
                        .split(" and ")
                        .map(|n| match n.split_once(',') {
                            Some((f, g)) => Name { family: f.trim().into(), given: g.trim().into() },
                            None => {
                                let parts: Vec<&str> = n.split_whitespace().collect();
                                match parts.split_last() {
                                    Some((f, g)) => Name { family: f.to_string(), given: g.join(" ") },
                                    None => Name::default(),
                                }
                            }
                        })
                        .collect()
                }
                "title" => s.title = value,
                "journal" | "journaltitle" | "booktitle" => s.container = value,
                "publisher" | "institution" | "organization" => s.publisher = value,
                "year" | "date" => s.year = value.chars().take(4).collect(),
                "volume" => s.volume = value,
                "number" | "issue" => s.issue = value,
                "pages" => s.pages = value.replace("--", "–"),
                "url" => s.url = value,
                "doi" => s.doi = value,
                "urldate" => s.accessed = value,
                _ => {}
            }
            rest = used;
            if rest.trim_start().starts_with('}') && !rest.contains('=') {
                break;
            }
        }
        if !s.key.is_empty() {
            out.push(s);
        }
    }
    out
}

/// Reads RIS records (TY - JOUR … ER -).
pub fn read_ris(text: &str) -> Vec<Source> {
    let mut out = Vec::new();
    let mut cur: Option<Source> = None;
    for line in text.lines() {
        let Some((tag, value)) = line.split_once("  - ").or_else(|| line.split_once(" - ")) else { continue };
        let (tag, value) = (tag.trim(), value.trim().to_string());
        match tag {
            "TY" => {
                cur = Some(Source {
                    kind: match value.as_str() {
                        "JOUR" => SourceKind::Article,
                        "BOOK" => SourceKind::Book,
                        "CHAP" => SourceKind::Chapter,
                        "RPRT" => SourceKind::Report,
                        "ELEC" | "WEB" => SourceKind::Web,
                        _ => SourceKind::Other,
                    },
                    ..Default::default()
                })
            }
            "ER" => {
                if let Some(s) = cur.take() {
                    out.push(s);
                }
            }
            _ => {
                let Some(s) = cur.as_mut() else { continue };
                match tag {
                    "AU" | "A1" => {
                        let (f, g) = value.split_once(',').map_or((value.as_str(), ""), |(f, g)| (f, g));
                        s.authors.push(Name { family: f.trim().into(), given: g.trim().into() });
                    }
                    "TI" | "T1" => s.title = value,
                    "JO" | "JF" | "T2" | "BT" => s.container = value,
                    "PB" => s.publisher = value,
                    "PY" | "Y1" | "DA" => s.year = value.chars().take(4).collect(),
                    "VL" => s.volume = value,
                    "IS" => s.issue = value,
                    "SP" => s.pages = value,
                    "EP" if !s.pages.is_empty() => s.pages = format!("{}–{value}", s.pages),
                    "UR" => s.url = value,
                    "DO" => s.doi = value,
                    "ID" => s.key = value,
                    _ => {}
                }
            }
        }
    }
    out
}

/// References › Manage sources: replaces the document's sources.
pub fn set_sources(ed: &mut crate::Editor, sources: Vec<Source>) -> Result<(), crate::EditError> {
    let meta = crate::Meta { sources, ..ed.doc.meta.clone() };
    crate::ops::set_meta(ed, meta, "Edit sources")
}

/// References › Citation style.
pub fn set_style(ed: &mut crate::Editor, style: CiteStyle) -> Result<(), crate::EditError> {
    let meta = crate::Meta { cite_style: style, ..ed.doc.meta.clone() };
    crate::ops::set_meta(ed, meta, "Citation style")
}

/// References › Insert citation: cites `keys` at the caret.
pub fn insert_citation(ed: &mut crate::Editor, keys: Vec<String>, locator: String) -> Result<(), crate::EditError> {
    crate::ops::insert_field(ed, Field::Citation { keys, locator }, "Insert citation")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(style: CiteStyle) -> Document {
        let mut d = Document::new();
        d.meta.cite_style = style;
        d.meta.sources = vec![
            Source {
                key: "smith2020".into(),
                kind: SourceKind::Book,
                authors: vec![Name { family: "Smith".into(), given: "Jane Ann".into() }],
                title: "Writing well".into(),
                publisher: "Harbour Press".into(),
                year: "2020".into(),
                ..Default::default()
            },
            Source {
                key: "lee2018".into(),
                kind: SourceKind::Article,
                authors: vec![
                    Name { family: "Lee".into(), given: "Min".into() },
                    Name { family: "Okafor".into(), given: "Ngozi".into() },
                ],
                title: "Commas and clarity".into(),
                container: "Journal of Prose".into(),
                volume: "12".into(),
                issue: "3".into(),
                pages: "45–67".into(),
                year: "2018".into(),
                doi: "10.1000/jp.2018.3".into(),
                ..Default::default()
            },
        ];
        d
    }

    #[test]
    fn in_text_citations_per_style() {
        let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let d = doc_with(CiteStyle::Apa);
        assert_eq!(citation(&d, &keys(&["smith2020"]), "p. 4"), "(Smith, 2020, p. 4)");
        assert_eq!(citation(&d, &keys(&["lee2018", "smith2020"]), ""), "(Lee & Okafor, 2018; Smith, 2020)");
        let d = doc_with(CiteStyle::Harvard);
        assert_eq!(citation(&d, &keys(&["lee2018"]), ""), "(Lee and Okafor 2018)");
        let d = doc_with(CiteStyle::Chicago);
        assert_eq!(citation(&d, &keys(&["smith2020"]), "p. 4"), "(Smith 2020, 4)");
        let d = doc_with(CiteStyle::Ieee);
        assert_eq!(citation(&d, &keys(&["missing"]), ""), "[missing]");
    }

    #[test]
    fn bibliography_entries_per_style() {
        let flat = |e: Vec<(String, bool)>| e.into_iter().map(|(t, _)| t).collect::<String>();
        let d = doc_with(CiteStyle::Apa);
        let b: Vec<String> = bibliography(&d).into_iter().map(flat).collect();
        assert_eq!(
            b[0],
            "Lee, M., & Okafor, N. (2018). Commas and clarity. Journal of Prose, 12(3), 45–67. https://doi.org/10.1000/jp.2018.3"
        );
        assert_eq!(b[1], "Smith, J. A. (2020). Writing well. Harbour Press.");
        let d = doc_with(CiteStyle::Ieee);
        let b: Vec<String> = bibliography(&d).into_iter().map(flat).collect();
        assert_eq!(b[0], "[1] J. A. Smith, Writing well, Harbour Press, 2020.");
        // The title of a book is italic.
        let e = entry(&d.meta.sources[0], CiteStyle::Apa, 1);
        assert!(e.iter().any(|(t, i)| t == "Writing well" && *i));
    }

    #[test]
    fn bibtex_and_ris_are_read() {
        let b = read_bibtex(
            "@article{lee2018, author = {Lee, Min and Okafor, Ngozi}, title = {Commas and {Clarity}}, journal = \"Journal of Prose\", year = 2018, pages = {45--67}}",
        );
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].key, "lee2018");
        assert_eq!(b[0].authors[1].family, "Okafor");
        assert_eq!((b[0].title.as_str(), b[0].pages.as_str()), ("Commas and Clarity", "45–67"));
        let r =
            read_ris("TY  - BOOK\nAU  - Smith, Jane\nTI  - Writing well\nPY  - 2020\nPB  - Harbour Press\nER  - \n");
        assert_eq!(
            (r[0].kind, r[0].authors[0].family.as_str(), r[0].year.as_str()),
            (SourceKind::Book, "Smith", "2020")
        );
    }
}
