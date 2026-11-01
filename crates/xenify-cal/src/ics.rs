//! The iCalendar text format (RFC 5545) as a tree of components and
//! properties: unfolding and folding long lines, parameters, and escaping.
//! Lenient when reading (real invitations bend the rules), strict when
//! writing.

/// `NAME;PARAM=VALUE:value`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Property {
    /// Upper case (`DTSTART`).
    pub name: String,
    /// (upper-case name, value without quotes).
    pub params: Vec<(String, String)>,
    /// The raw value (text escapes still in; see [`unescape`]).
    pub value: String,
}

impl Property {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self { name: name.to_ascii_uppercase(), params: Vec::new(), value: value.into() }
    }

    pub fn with(mut self, param: &str, value: impl Into<String>) -> Self {
        self.params.push((param.to_ascii_uppercase(), value.into()));
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The value as text (escapes undone).
    pub fn text(&self) -> String {
        unescape(&self.value)
    }
}

/// `BEGIN:NAME … END:NAME`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub props: Vec<Property>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_ascii_uppercase(), ..Default::default() }
    }

    pub fn prop(&self, name: &str) -> Option<&Property> {
        self.props.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn props_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> + 'a {
        self.props.iter().filter(move |p| p.name.eq_ignore_ascii_case(name))
    }

    /// A property's text value.
    pub fn text(&self, name: &str) -> Option<String> {
        self.prop(name).map(Property::text)
    }

    pub fn push(&mut self, p: Property) {
        self.props.push(p);
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> + 'a {
        self.children.iter().filter(move |c| c.name.eq_ignore_ascii_case(name))
    }

    /// As iCalendar text (CRLF line ends, lines folded at 75 bytes).
    pub fn to_ics(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        fold_into(out, &format!("BEGIN:{}", self.name));
        for p in &self.props {
            let mut line = p.name.clone();
            for (k, v) in &p.params {
                // A line break would start a new property.
                let v = v.replace(['\r', '\n'], " ");
                line.push(';');
                line.push_str(k);
                line.push('=');
                if v.contains([':', ';', ',']) {
                    line.push('"');
                    line.push_str(&v.replace('"', "'"));
                    line.push('"');
                } else {
                    line.push_str(&v);
                }
            }
            line.push(':');
            line.push_str(&p.value.replace(['\r', '\n'], ""));
            fold_into(out, &line);
        }
        for c in &self.children {
            c.write(out);
        }
        fold_into(out, &format!("END:{}", self.name));
    }
}

/// Appends `line`, folded so no line is over 75 bytes (never splitting a
/// character).
fn fold_into(out: &mut String, line: &str) {
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("not an iCalendar file (no BEGIN:VCALENDAR)")]
    NotCalendar,
    #[error("the calendar ends early (no END:{0})")]
    Unclosed(String),
}

/// Reads iCalendar text. Returns the top-level components (usually one
/// VCALENDAR). Lines it can't read are skipped.
pub fn parse(text: &str) -> Result<Vec<Component>, ParseError> {
    let mut stack: Vec<Component> = Vec::new();
    let mut top = Vec::new();
    for line in unfold(text) {
        let Some(p) = parse_line(&line) else { continue };
        if p.name == "BEGIN" {
            stack.push(Component::new(p.value.trim()));
        } else if p.name == "END" {
            let Some(done) = stack.pop() else { continue };
            match stack.last_mut() {
                Some(parent) => parent.children.push(done),
                None => top.push(done),
            }
        } else if let Some(c) = stack.last_mut() {
            c.props.push(p);
        }
        // A depth this large is a broken or hostile file.
        if stack.len() > 32 {
            return Err(ParseError::Unclosed(stack.last().map(|c| c.name.clone()).unwrap_or_default()));
        }
    }
    if let Some(open) = stack.pop() {
        // Missing END lines: keep what was read (Outlook sometimes truncates).
        let mut open = open;
        while let Some(mut parent) = stack.pop() {
            parent.children.push(open);
            open = parent;
        }
        top.push(open);
    }
    if !top.iter().any(|c| c.name == "VCALENDAR") {
        return Err(ParseError::NotCalendar);
    }
    Ok(top)
}

/// Joins folded lines (a line starting with a space or tab continues the one before).
fn unfold(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

/// `NAME;P1=a;P2="b:c":value` → Property.
fn parse_line(line: &str) -> Option<Property> {
    // The name ends at the first ';' or ':'.
    let name_end = line.find([';', ':'])?;
    let name = line[..name_end].trim().to_ascii_uppercase();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let mut params = Vec::new();
    let mut rest = &line[name_end..];
    while let Some(r) = rest.strip_prefix(';') {
        let eq = r.find('=')?;
        let pname = r[..eq].trim().to_ascii_uppercase();
        let after = &r[eq + 1..];
        let (value, next) = if let Some(q) = after.strip_prefix('"') {
            let close = q.find('"')?;
            (q[..close].to_string(), &q[close + 1..])
        } else {
            let end = after.find([';', ':']).unwrap_or(after.len());
            (after[..end].to_string(), &after[end..])
        };
        params.push((pname, value));
        rest = next;
    }
    let value = rest.strip_prefix(':')?.to_string();
    Some(Property { name, params, value })
}

/// Text value escapes: `\n` `\,` `\;` `\\`.
pub fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_params_and_escapes() {
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Lunch\\, then a walk\\; maybe\r\nDESCRIPTION:Line one\\nLine \r\n two\r\nORGANIZER;CN=\"Lee, Sam\";SENT-BY=\"mailto:a@b\":mailto:sam@x.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let cal = &parse(text).unwrap()[0];
        let ev = cal.children_named("VEVENT").next().unwrap();
        assert_eq!(ev.text("SUMMARY").unwrap(), "Lunch, then a walk; maybe");
        assert_eq!(ev.text("DESCRIPTION").unwrap(), "Line one\nLine two");
        let org = ev.prop("ORGANIZER").unwrap();
        assert_eq!(org.param("CN"), Some("Lee, Sam"));
        assert_eq!(org.param("sent-by"), Some("mailto:a@b"));
        assert_eq!(org.value, "mailto:sam@x.org");
    }

    #[test]
    fn writes_and_folds_long_lines_without_splitting_characters() {
        let mut ev = Component::new("VEVENT");
        let long = "Café ".repeat(40);
        ev.push(Property::new("SUMMARY", escape(&long)));
        ev.push(Property::new("ATTENDEE", "mailto:a@x.org").with("CN", "Lee, Ana"));
        let mut cal = Component::new("VCALENDAR");
        cal.children.push(ev);
        let text = cal.to_ics();
        assert!(text.lines().all(|l| l.trim_end_matches('\r').len() <= 75), "{text}");
        assert!(text.contains("ATTENDEE;CN=\"Lee, Ana\":mailto:a@x.org"));
        let back = parse(&text).unwrap();
        let ev = back[0].children_named("VEVENT").next().unwrap();
        assert_eq!(ev.text("SUMMARY").unwrap(), long);
    }

    #[test]
    fn tolerates_bad_input() {
        assert_eq!(parse("hello"), Err(ParseError::NotCalendar));
        // No END lines (truncated): what was read is kept.
        let cal = parse("BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:x\n").unwrap();
        assert_eq!(cal[0].children[0].text("SUMMARY").unwrap(), "x");
        // Garbage lines are skipped.
        assert!(parse("BEGIN:VCALENDAR\n:::\n;;;\nX\nEND:VCALENDAR\n").is_ok());
        let deep = "BEGIN:VCALENDAR\n".to_string() + &"BEGIN:X\n".repeat(100);
        assert!(parse(&deep).is_err());
    }
}
