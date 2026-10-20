//! Untrusted HTML (email) in, a small safe subset out.
//!
//! - Parsed with html5ever, the same parsing rules browsers use, so broken
//!   or tricky markup is read the way a browser would read it.
//! - Only an allowlist of elements and attributes survives. Scripts, frames,
//!   forms, embedded objects, SVG/MathML and event handlers are removed, and
//!   unknown elements keep only their content.
//! - `<style>` rules with simple selectors are applied as inline styles;
//!   only harmless CSS properties survive (no `url()`, no positioning).
//! - Content hidden from the reader (preheaders, `display:none`) is dropped.
//! - Remote images load only when allowed; tracking pixels never do.
//! - Links: only web, mail and phone links, each with warnings when it tries
//!   to fool you.
//!
//! Nothing here fetches anything: images come in through a resolver.

pub mod css;
pub mod links;

use css::{Info, Sheet};
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::fmt::Write;

pub use links::Warning;

#[derive(Clone, Debug)]
pub struct Options {
    /// Load pictures from the internet (else they're left out and counted).
    pub remote_images: bool,
    /// Largest picture embedded in the HTML itself (`data:`), in bytes of text.
    pub max_inline_image: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { remote_images: false, max_inline_image: 4 * 1024 * 1024 }
    }
}

/// A link in the cleaned HTML.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub href: String,
    pub text: String,
    pub warnings: Vec<Warning>,
}

/// The cleaned HTML and what was done to it.
#[derive(Clone, Debug, Default)]
pub struct Clean {
    pub html: String,
    /// Pictures from the internet that weren't loaded (remote images off).
    pub blocked_images: usize,
    /// Internet pictures wanted but not yet given by the resolver (fetch
    /// these, then clean again).
    pub to_fetch: Vec<String>,
    /// Tracking pixels removed.
    pub trackers: usize,
    pub links: Vec<Link>,
}

/// Elements dropped with everything inside them.
const DROP: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "embed", "applet", "noscript", "template", "head",
    "title", "meta", "link", "base", "svg", "math", "input", "select", "textarea", "option", "button", "audio",
    "video", "source", "track", "canvas", "map", "area", "param", "dialog", "portal", "noembed", "noframes",
];

/// Elements kept (with only their allowed attributes).
const KEEP: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "b",
    "big",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "tt",
    "u",
    "ul",
];

const VOID: &[&str] = &["br", "hr", "col", "img"];

/// Attributes kept on any element (besides `style`, which is rebuilt).
const ATTRS: &[&str] = &[
    "align",
    "valign",
    "bgcolor",
    "dir",
    "width",
    "height",
    "colspan",
    "rowspan",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "size",
    "face",
    "start",
    "type",
    "span",
    "alt",
    "title",
];

/// Deeper than this, only text is kept (a hostile message can't exhaust the stack).
const MAX_DEPTH: usize = 128;

/// Cleans `html`. `image` turns a picture's `src` (`cid:…` from the message
/// itself, or an internet address already fetched) into a `data:` URL, or
/// None if it isn't available.
pub fn clean(html: &str, opts: &Options, image: &dyn Fn(&str) -> Option<String>) -> Clean {
    let html = limit_nesting(html, MAX_NESTING);
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(&*html);
    let mut styles = String::new();
    collect_styles(&dom.document, &mut styles);
    let mut w = Writer { opts, image, sheet: Sheet::parse(&styles), out: Clean::default(), ancestors: Vec::new() };
    w.children(&dom.document);
    w.out.html = w.out.html.trim().to_string();
    w.out
}

/// Parsing time grows with the square of nesting depth (50,000 nested
/// `<div>`s take seconds), so tags deeper than this are dropped before
/// parsing. Real email nests a few dozen deep.
const MAX_NESTING: usize = 512;

/// `html` with start and end tags nested deeper than `max` removed (their
/// text stays). A rough scan is enough: the result is parsed and cleaned
/// like any other input, so this can change formatting but never safety.
fn limit_nesting(html: &str, max: usize) -> std::borrow::Cow<'_, str> {
    if html.matches('<').count() <= max {
        return html.into();
    }
    const VOIDS: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
    ];
    let mut out = String::with_capacity(html.len());
    let mut depth = 0usize;
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
        let tag = &rest[..end];
        let closing = tag.starts_with("</");
        let name: String = tag
            .trim_start_matches(['<', '/'])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let keep = if name.is_empty() {
            true // text, a comment or a doctype
        } else if closing {
            let deep = depth > max;
            depth = depth.saturating_sub(1);
            !deep
        } else if VOIDS.contains(&name.as_str()) || tag.ends_with("/>") {
            depth < max
        } else {
            depth += 1;
            depth <= max
        };
        if keep {
            out.push_str(tag);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into()
}

/// Plain text to safe HTML (a text-only message): paragraphs, line breaks
/// and web addresses as links.
pub fn from_text(text: &str) -> String {
    let mut out = String::new();
    for para in text.replace("\r\n", "\n").split("\n\n") {
        out.push_str("<p>");
        for (i, line) in para.lines().enumerate() {
            if i > 0 {
                out.push_str("<br>");
            }
            for (j, word) in line.split(' ').enumerate() {
                if j > 0 {
                    out.push(' ');
                }
                let bare = word.trim_end_matches(['.', ',', ')', ';', ':', '!', '?']);
                match links::allowed(bare).filter(|_| bare.starts_with("http")) {
                    Some(url) => {
                        let _ = write!(
                            out,
                            "<a href=\"{}\">{}</a>{}",
                            attr(url.as_str()),
                            text_esc(bare),
                            text_esc(&word[bare.len()..])
                        );
                    }
                    None => out.push_str(&text_esc(word)),
                }
            }
        }
        out.push_str("</p>");
    }
    out
}

fn collect_styles(node: &Handle, out: &mut String) {
    let mut stack = vec![node.clone()];
    while let Some(n) = stack.pop() {
        if let NodeData::Element { name, .. } = &n.data
            && &*name.local == "style"
        {
            for c in n.children.borrow().iter() {
                if let NodeData::Text { contents } = &c.data {
                    out.push_str(&contents.borrow());
                    out.push('\n');
                }
            }
            continue;
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
}

struct Writer<'a> {
    opts: &'a Options,
    image: &'a dyn Fn(&str) -> Option<String>,
    sheet: Sheet,
    out: Clean,
    ancestors: Vec<Info>,
}

impl Writer<'_> {
    fn children(&mut self, node: &Handle) {
        for c in node.children.borrow().iter() {
            self.node(c);
        }
    }

    fn node(&mut self, node: &Handle) {
        match &node.data {
            NodeData::Text { contents } => self.out.html.push_str(&text_esc(&contents.borrow())),
            NodeData::Element { name, attrs, .. } => {
                if name.ns != html5ever::ns!(html) {
                    return; // SVG, MathML
                }
                let tag = name.local.to_string().to_ascii_lowercase();
                if DROP.contains(&tag.as_str()) {
                    return;
                }
                if self.ancestors.len() >= MAX_DEPTH {
                    let mut t = String::new();
                    text_of(node, &mut t);
                    self.out.html.push_str(&text_esc(&t));
                    return;
                }
                let attrs: Vec<(String, String)> = attrs
                    .borrow()
                    .iter()
                    .map(|a| (a.name.local.to_string().to_ascii_lowercase(), a.value.to_string()))
                    .collect();
                self.element(node, &tag, &attrs);
            }
            NodeData::Document => self.children(node),
            _ => {} // comments (and Outlook's conditional ones), doctypes
        }
    }

    fn element(&mut self, node: &Handle, tag: &str, attrs: &[(String, String)]) {
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let info = Info {
            tag: tag.to_string(),
            classes: get("class")
                .map(|c| c.split_whitespace().map(|s| s.to_ascii_lowercase()).collect())
                .unwrap_or_default(),
            id: get("id").map(|s| s.trim().to_ascii_lowercase()),
        };
        let mut decls = if self.sheet.is_empty() { Vec::new() } else { self.sheet.matching(&self.ancestors, &info) };
        if let Some(inline) = get("style") {
            decls.extend(css::parse_decls(inline));
        }
        if css::hides(&decls) || get("hidden").is_some() {
            return;
        }
        let mut style = String::new();
        for (k, v) in &decls {
            if let Some((k, v)) = css::safe_decl(k, v) {
                let _ = write!(style, "{k}:{v};");
            }
        }
        if tag == "img" {
            self.img(attrs, &decls, &style);
            return;
        }
        let (out_tag, extra_style) = match tag {
            "center" => ("div", "text-align:center;"),
            "form" | "body" | "html" | "nav" | "label" | "fieldset" | "legend" | "details" | "summary" => ("div", ""),
            t if KEEP.contains(&t) => (t, ""),
            _ => ("", ""), // unknown: keep what's inside
        };
        let mut href = None;
        if out_tag == "a" {
            href = get("href").and_then(links::allowed);
            if href.is_none() {
                // Not a link we open: just its text.
                self.with_ancestor(info, |w| w.children(node));
                return;
            }
        }
        if out_tag.is_empty() || (matches!(tag, "body" | "html") && style.is_empty() && get("bgcolor").is_none()) {
            self.with_ancestor(info, |w| w.children(node));
            return;
        }
        let _ = write!(self.out.html, "<{out_tag}");
        if let Some(url) = &href {
            let _ = write!(self.out.html, " href=\"{}\"", attr(url.as_str()));
        }
        for (k, v) in attrs {
            if ATTRS.contains(&k.as_str()) && v.len() < 200 && !v.to_ascii_lowercase().contains("url(") {
                let _ = write!(self.out.html, " {k}=\"{}\"", attr(v));
            }
        }
        let style = format!("{extra_style}{style}");
        if !style.is_empty() {
            let _ = write!(self.out.html, " style=\"{}\"", attr(&style));
        }
        self.out.html.push('>');
        if VOID.contains(&out_tag) {
            return;
        }
        self.with_ancestor(info, |w| w.children(node));
        let _ = write!(self.out.html, "</{out_tag}>");
        if let Some(url) = href {
            let mut text = String::new();
            text_of(node, &mut text);
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let warnings = links::warnings(&url, &text);
            self.out.links.push(Link { href: url.to_string(), text, warnings });
        }
    }

    fn with_ancestor(&mut self, info: Info, f: impl FnOnce(&mut Self)) {
        self.ancestors.push(info);
        f(self);
        self.ancestors.pop();
    }

    fn img(&mut self, attrs: &[(String, String)], decls: &[(String, String)], style: &str) {
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim());
        let size = |k: &str| {
            decls.iter().rev().find(|(p, _)| p == k).and_then(|(_, v)| css::px(v)).or_else(|| get(k).and_then(css::px))
        };
        // Tracking pixels: tiny images, whoever they come from.
        if let (Some(w), Some(h)) = (size("width"), size("height"))
            && w <= 3.0
            && h <= 3.0
        {
            self.out.trackers += 1;
            return;
        }
        let src = get("src").unwrap_or("");
        let lower = src.to_ascii_lowercase();
        let data = if lower.starts_with("data:image/") {
            let ok = [
                "data:image/png",
                "data:image/jpeg",
                "data:image/jpg",
                "data:image/gif",
                "data:image/webp",
                "data:image/bmp",
            ]
            .iter()
            .any(|p| lower.starts_with(p))
                && src.len() <= self.opts.max_inline_image;
            ok.then(|| src.to_string())
        } else if lower.starts_with("cid:") {
            (self.image)(src)
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            if self.opts.remote_images {
                let got = (self.image)(src);
                if got.is_none() && !self.out.to_fetch.iter().any(|u| u == src) {
                    self.out.to_fetch.push(src.to_string());
                }
                got
            } else {
                self.out.blocked_images += 1;
                None
            }
        } else {
            None
        };
        let alt = get("alt").unwrap_or("");
        match data {
            Some(data) => {
                let _ = write!(self.out.html, "<img src=\"{}\"", attr(&data));
                for k in ["width", "height", "alt", "title", "align", "border"] {
                    if let Some(v) = get(k).filter(|v| v.len() < 100) {
                        let _ = write!(self.out.html, " {k}=\"{}\"", attr(v));
                    }
                }
                if !style.is_empty() {
                    let _ = write!(self.out.html, " style=\"{}\"", attr(style));
                }
                self.out.html.push('>');
            }
            // A picture that isn't shown: its description, if it has one.
            None if !alt.is_empty() => {
                let _ = write!(self.out.html, "<span style=\"color:#8a8f98\">[{}]</span>", text_esc(alt));
            }
            None => {}
        }
    }
}

/// All the text inside `node` (without recursion, for any depth).
fn text_of(node: &Handle, out: &mut String) {
    let mut stack = vec![node.clone()];
    while let Some(n) = stack.pop() {
        match &n.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { name, .. } if DROP.contains(&&*name.local) => continue,
            _ => {}
        }
        stack.extend(n.children.borrow().iter().rev().cloned());
    }
}

fn text_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn attr(s: &str) -> String {
    text_esc(s).replace('"', "&quot;")
}
