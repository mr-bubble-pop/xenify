//! Calendars you subscribe to by link: Google's "secret address in iCal
//! format", Outlook's published .ics link, school and team calendars.
//! Read-only; refreshed now and then, downloading only when it changed.

use crate::event::Event;
use std::time::Duration;

/// What a refresh found.
#[derive(Debug, PartialEq)]
pub enum Fetched {
    /// New contents, with what to send next time to skip unchanged ones.
    Changed { events: Vec<Event>, name: Option<String>, validator: String },
    /// Same as last time.
    NotModified,
}

/// Biggest calendar file accepted.
const MAX: u64 = 30 * 1024 * 1024;

/// `webcal://` links are plain web links underneath.
pub fn normalize(url: &str) -> String {
    let u = url.trim();
    if let Some(rest) = u.strip_prefix("webcals://").or_else(|| u.strip_prefix("webcal://")) {
        format!("https://{rest}")
    } else {
        u.to_string()
    }
}

/// Downloads the calendar unless it's unchanged since `validator` (an ETag
/// or Last-Modified from before, as returned here).
pub fn fetch(url: &str, validator: &str) -> Result<Fetched, String> {
    let url = normalize(url);
    if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")) {
        return Err("Use an https:// (or webcal://) link.".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent.get(&url).header("Accept", "text/calendar, */*;q=0.5");
    if let Some(etag) = validator.strip_prefix("etag:") {
        req = req.header("If-None-Match", etag);
    } else if let Some(date) = validator.strip_prefix("date:") {
        req = req.header("If-Modified-Since", date);
    }
    let mut resp = req.call().map_err(|e| format!("Couldn't reach the calendar: {e}"))?;
    let status = resp.status().as_u16();
    if status == 304 {
        return Ok(Fetched::NotModified);
    }
    if status == 401 || status == 403 {
        return Err("The calendar's address needs a password or is private: use its secret/published link.".into());
    }
    if status == 404 || status == 410 {
        return Err("Nothing is at that address any more (it may have been reset).".into());
    }
    if status >= 400 {
        return Err(format!("The calendar's server said {status}."));
    }
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let validator = header("ETag")
        .map(|e| format!("etag:{e}"))
        .or_else(|| header("Last-Modified").map(|d| format!("date:{d}")))
        .unwrap_or_default();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX)
        .read_to_string()
        .map_err(|e| format!("Couldn't read the calendar: {e}"))?;
    let cals = crate::ics::parse(&text).map_err(|_| "That address isn't an iCalendar (.ics) file.".to_string())?;
    let name = cals.iter().find_map(|c| c.text("X-WR-CALNAME")).filter(|n| !n.trim().is_empty());
    let events = cals.iter().flat_map(|c| c.children_named("VEVENT")).filter_map(Event::from_component).collect();
    Ok(Fetched::Changed { events, name, validator })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Serves one calendar; answers 304 when the ETag matches.
    fn serve() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut out = c.try_clone().unwrap();
                let mut rd = BufReader::new(c);
                let mut etag_match = false;
                let mut line = String::new();
                while rd.read_line(&mut line).unwrap_or(0) > 2 {
                    if line.to_ascii_lowercase().starts_with("if-none-match: \"v1\"") {
                        etag_match = true;
                    }
                    line.clear();
                }
                if etag_match {
                    let _ = write!(out, "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n");
                } else {
                    let body = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:School terms\r\nBEGIN:VEVENT\r\nUID:t1\r\nSUMMARY:Half-term\r\nDTSTART;VALUE=DATE:20261026\r\nDTEND;VALUE=DATE:20261031\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
                    let _ = write!(
                        out,
                        "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                }
            }
        });
        port
    }

    #[test]
    fn downloads_then_skips_unchanged() {
        let url = format!("http://127.0.0.1:{}/basic.ics", serve());
        let Fetched::Changed { events, name, validator } = fetch(&url, "").unwrap() else { panic!() };
        assert_eq!(events.len(), 1);
        assert_eq!(name.as_deref(), Some("School terms"));
        assert_eq!(validator, "etag:\"v1\"");
        assert_eq!(fetch(&url, &validator).unwrap(), Fetched::NotModified);
    }

    #[test]
    fn links_are_checked() {
        assert_eq!(normalize(" webcal://calendar.example.com/a.ics"), "https://calendar.example.com/a.ics");
        assert!(fetch("http://example.com/a.ics", "").unwrap_err().contains("https"));
        assert!(fetch("ftp://x", "").is_err());
    }
}
