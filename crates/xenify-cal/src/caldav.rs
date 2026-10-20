//! CalDAV (RFC 4791, plus WebDAV sync, RFC 6578): calendars kept on a
//! server and edited from here, as Fastmail, Nextcloud, iCloud, Radicale
//! and others offer. Sign-in is a user name and (app) password.

use std::time::Duration;
use ureq::http::{Method, Request};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DavError {
    #[error("couldn't reach the server: {0}")]
    Network(String),
    #[error("the server didn't accept the user name or password")]
    Auth,
    #[error("no calendars were found at that address")]
    NotFound,
    /// Someone changed it on the server since we last looked (If-Match failed).
    #[error("it was changed elsewhere")]
    Conflict,
    /// The sync token is too old: list everything again.
    #[error("the server asked for a full refresh")]
    TokenExpired,
    #[error("the server said {0}")]
    Status(u16),
    #[error("unexpected reply from the server: {0}")]
    Bad(String),
}

/// A calendar on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteCalendar {
    /// Full URL of the collection.
    pub url: String,
    pub name: String,
    pub color: Option<String>,
    pub sync_token: Option<String>,
    pub ctag: Option<String>,
}

/// An event resource on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub url: String,
    pub etag: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: Vec<Item>,
    pub removed: Vec<String>,
    pub token: String,
}

pub struct Dav {
    agent: ureq::Agent,
    auth: String,
}

const NS_DAV: &str = "DAV:";
const NS_CAL: &str = "urn:ietf:params:xml:ns:caldav";

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        use base64::Engine;
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .allow_non_standard_methods(true)
            // Redirects are followed here, keeping the method and sign-in.
            .max_redirects(0)
            .build()
            .into();
        let auth = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")));
        Dav { agent, auth }
    }

    /// One request, following up to 5 redirects. Returns (status, final URL, body, ETag).
    fn send(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<(u16, String, String, Option<String>), DavError> {
        let mut url = url.to_string();
        for _ in 0..5 {
            let mut b = Request::builder()
                .method(Method::from_bytes(method.as_bytes()).map_err(|e| DavError::Bad(e.to_string()))?)
                .uri(&url)
                .header("Authorization", &self.auth)
                .header("User-Agent", "Xenify Mail");
            if let Some(d) = depth {
                b = b.header("Depth", d);
            }
            if body.is_some() {
                let ct =
                    if method == "PUT" { "text/calendar; charset=utf-8" } else { "application/xml; charset=utf-8" };
                b = b.header("Content-Type", ct);
            }
            for (k, v) in headers {
                b = b.header(*k, *v);
            }
            let req = b.body(body.unwrap_or("").to_string()).map_err(|e| DavError::Bad(e.to_string()))?;
            let mut resp = self.agent.run(req).map_err(|e| DavError::Network(e.to_string()))?;
            let status = resp.status().as_u16();
            let header = |n: &str| resp.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
            if matches!(status, 301 | 302 | 307 | 308)
                && let Some(loc) = header("Location")
            {
                url = join(&url, &loc);
                continue;
            }
            let etag = header("ETag");
            let text = resp.body_mut().with_config().limit(50 * 1024 * 1024).read_to_string().unwrap_or_default();
            if status == 401 {
                return Err(DavError::Auth);
            }
            return Ok((status, url, text, etag));
        }
        Err(DavError::Bad("too many redirects".into()))
    }

    fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<(String, String), DavError> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:ic="http://apple.com/ns/ical/"><d:prop>{props}</d:prop></d:propfind>"#
        );
        let (status, url, text, _) = self.send("PROPFIND", url, Some(depth), Some(&body), &[])?;
        match status {
            207 => Ok((url, text)),
            404 => Err(DavError::NotFound),
            s => Err(DavError::Status(s)),
        }
    }

    /// Finds your calendars from the server's address (its root, its
    /// /.well-known/caldav, a principal or a calendar home all work).
    pub fn discover(&self, server: &str) -> Result<Vec<RemoteCalendar>, DavError> {
        let server = server.trim().trim_end_matches('/').to_string();
        let candidates = [format!("{server}/.well-known/caldav"), format!("{server}/")];
        let mut principal = None;
        for c in &candidates {
            match self.propfind(c, "0", "<d:current-user-principal/>") {
                Ok((url, xml)) => {
                    if let Some(p) = first_href(&xml, "current-user-principal") {
                        principal = Some(join(&url, &p));
                        break;
                    }
                }
                Err(DavError::Auth) => return Err(DavError::Auth),
                Err(_) => continue,
            }
        }
        let principal = principal.unwrap_or_else(|| format!("{server}/"));
        let (url, xml) = self.propfind(&principal, "0", "<c:calendar-home-set/>")?;
        let home = first_href(&xml, "calendar-home-set").map(|h| join(&url, &h)).unwrap_or(principal);
        let (url, xml) = self.propfind(
            &home,
            "1",
            "<d:resourcetype/><d:displayname/><ic:calendar-color/><c:supported-calendar-component-set/><cs:getctag/><d:sync-token/>",
        )?;
        let cals = parse_calendars(&url, &xml);
        if cals.is_empty() { Err(DavError::NotFound) } else { Ok(cals) }
    }

    /// Changes since `token` (everything when empty), with the new token.
    pub fn changes(&self, cal: &RemoteCalendar, token: &str) -> Result<Changes, DavError> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><d:sync-collection xmlns:d="DAV:"><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#,
            xml_escape(token)
        );
        let (status, url, text, _) = self.send("REPORT", &cal.url, Some("1"), Some(&body), &[])?;
        match status {
            207 => Ok(parse_changes(&url, &text)),
            403 | 409 if !token.is_empty() => Err(DavError::TokenExpired),
            // No sync support: compare ETags instead.
            400 | 403 | 405 | 501 => {
                let items = self.list(cal)?;
                Ok(Changes { changed: items, removed: Vec::new(), token: String::new() })
            }
            s => Err(DavError::Status(s)),
        }
    }

    /// Every event resource with its ETag.
    pub fn list(&self, cal: &RemoteCalendar) -> Result<Vec<Item>, DavError> {
        let (url, xml) = self.propfind(&cal.url, "1", "<d:getetag/><d:resourcetype/>")?;
        Ok(parse_changes(&url, &xml)
            .changed
            .into_iter()
            .filter(|i| i.url.trim_end_matches('/') != url.trim_end_matches('/'))
            .collect())
    }

    /// Downloads events: (url, etag, iCalendar text).
    pub fn get(&self, cal: &RemoteCalendar, urls: &[String]) -> Result<Vec<(String, String, String)>, DavError> {
        if urls.is_empty() {
            return Ok(Vec::new());
        }
        let hrefs: String = urls.iter().map(|u| format!("<d:href>{}</d:href>", xml_escape(&path_of(u)))).collect();
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop>{hrefs}</c:calendar-multiget>"#
        );
        let (status, url, text, _) = self.send("REPORT", &cal.url, Some("1"), Some(&body), &[])?;
        if status != 207 {
            return Err(DavError::Status(status));
        }
        Ok(parse_data(&url, &text))
    }

    /// Creates (no `etag`) or replaces (`etag` given: only if unchanged)
    /// an event. Returns its new ETag when the server says.
    pub fn put(&self, url: &str, ics: &str, etag: Option<&str>) -> Result<Option<String>, DavError> {
        let cond = match etag {
            Some(e) => ("If-Match", e),
            None => ("If-None-Match", "*"),
        };
        let (status, _, _, new_etag) = self.send("PUT", url, None, Some(ics), &[cond])?;
        match status {
            200 | 201 | 204 => Ok(new_etag),
            412 => Err(DavError::Conflict),
            s => Err(DavError::Status(s)),
        }
    }

    /// Deletes an event (only if unchanged since `etag`, when given).
    pub fn delete(&self, url: &str, etag: Option<&str>) -> Result<(), DavError> {
        let headers: Vec<(&str, &str)> = etag.map(|e| vec![("If-Match", e)]).unwrap_or_default();
        let (status, _, _, _) = self.send("DELETE", url, None, None, &headers)?;
        match status {
            200 | 204 | 404 => Ok(()),
            412 => Err(DavError::Conflict),
            s => Err(DavError::Status(s)),
        }
    }
}

/// `base` + `href` (absolute, or a path on the same server).
pub fn join(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let origin_end =
        base.find("://").map(|i| base[i + 3..].find('/').map(|j| i + 3 + j).unwrap_or(base.len())).unwrap_or(0);
    if href.starts_with('/') {
        format!("{}{href}", &base[..origin_end])
    } else {
        format!("{}/{href}", base.trim_end_matches('/'))
    }
}

fn path_of(url: &str) -> String {
    url.find("://")
        .and_then(|i| url[i + 3..].find('/').map(|j| url[i + 3 + j..].to_string()))
        .unwrap_or_else(|| url.to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The first `<href>` inside property `prop`.
fn first_href(xml: &str, prop: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let p = doc.descendants().find(|n| n.tag_name().name() == prop)?;
    p.descendants()
        .find(|n| n.tag_name().name() == "href" && n.tag_name().namespace() == Some(NS_DAV))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

fn text_of<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.descendants().find(|c| c.tag_name().name() == name).and_then(|c| c.text()).map(str::trim)
}

/// Each `<response>` that succeeded (status 200): (href, the node).
fn ok_responses<'a>(
    doc: &'a roxmltree::Document<'a>,
) -> impl Iterator<Item = (String, roxmltree::Node<'a, 'a>, bool)> + 'a {
    doc.descendants()
        .filter(|n| n.tag_name().name() == "response" && n.tag_name().namespace() == Some(NS_DAV))
        .filter_map(|r| {
            let href = r.children().find(|c| c.tag_name().name() == "href")?.text()?.trim().to_string();
            // A response-level 404 (sync: removed) has a <status> directly under <response>.
            let gone =
                r.children().any(|c| c.tag_name().name() == "status" && c.text().is_some_and(|t| t.contains(" 404")));
            Some((href, r, gone))
        })
}

fn parse_calendars(base: &str, xml: &str) -> Vec<RemoteCalendar> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    ok_responses(&doc)
        .filter(|(_, r, gone)| {
            !gone
                && r.descendants()
                    .any(|n| n.tag_name().name() == "calendar" && n.tag_name().namespace() == Some(NS_CAL))
        })
        .filter(|(_, r, _)| {
            // Only calendars that hold events (not just tasks).
            let comps: Vec<&str> =
                r.descendants().filter(|n| n.tag_name().name() == "comp").filter_map(|n| n.attribute("name")).collect();
            comps.is_empty() || comps.iter().any(|c| c.eq_ignore_ascii_case("VEVENT"))
        })
        .map(|(href, r, _)| RemoteCalendar {
            url: join(base, &href),
            name: text_of(r, "displayname")
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| href.trim_end_matches('/').rsplit('/').next().unwrap_or("Calendar").to_string()),
            color: text_of(r, "calendar-color").map(|c| c.chars().take(7).collect()),
            sync_token: text_of(r, "sync-token").map(str::to_string),
            ctag: text_of(r, "getctag").map(str::to_string),
        })
        .collect()
}

fn parse_changes(base: &str, xml: &str) -> Changes {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Changes::default() };
    let mut out = Changes {
        token: doc
            .descendants()
            .find(|n| {
                n.tag_name().name() == "sync-token" && n.parent().is_some_and(|p| p.tag_name().name() == "multistatus")
            })
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string(),
        ..Default::default()
    };
    for (href, r, gone) in ok_responses(&doc) {
        let url = join(base, &href);
        if gone {
            out.removed.push(url);
        } else if let Some(etag) = text_of(r, "getetag") {
            // Collections have no ETag in most servers; events do.
            if !r.descendants().any(|n| n.tag_name().name() == "collection") {
                out.changed.push(Item { url, etag: etag.to_string() });
            }
        }
    }
    out
}

fn parse_data(base: &str, xml: &str) -> Vec<(String, String, String)> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    ok_responses(&doc)
        .filter(|(_, _, gone)| !gone)
        .filter_map(|(href, r, _)| {
            let data = text_of(r, "calendar-data")?.to_string();
            Some((join(base, &href), text_of(r, "getetag").unwrap_or("").to_string(), data))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_join() {
        assert_eq!(join("https://dav.example.com/a/b/", "/c/d/"), "https://dav.example.com/c/d/");
        assert_eq!(join("https://dav.example.com/a/b/", "e.ics"), "https://dav.example.com/a/b/e.ics");
        assert_eq!(join("https://x.org/a", "https://y.org/z"), "https://y.org/z");
        assert_eq!(path_of("https://x.org/a/b.ics"), "/a/b.ics");
    }

    #[test]
    fn reads_calendar_lists_and_changes() {
        let home = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:ic="http://apple.com/ns/ical/">
<d:response><d:href>/dav/sam/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/dav/sam/work/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Work</d:displayname><ic:calendar-color>#ff8800ff</ic:calendar-color><d:sync-token>http://x/sync/5</d:sync-token><c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/dav/sam/tasks/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname><c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
</d:multistatus>"#;
        let cals = parse_calendars("https://dav.example.com/dav/sam/", home);
        assert_eq!(cals.len(), 1, "task lists left out");
        assert_eq!(cals[0].url, "https://dav.example.com/dav/sam/work/");
        assert_eq!(cals[0].color.as_deref(), Some("#ff8800"));
        assert_eq!(cals[0].sync_token.as_deref(), Some("http://x/sync/5"));

        let sync = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">
<d:response><d:href>/dav/sam/work/a.ics</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/dav/sam/work/b.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>
<d:sync-token>http://x/sync/6</d:sync-token></d:multistatus>"#;
        let ch = parse_changes("https://dav.example.com/dav/sam/work/", sync);
        assert_eq!(
            ch.changed,
            [Item { url: "https://dav.example.com/dav/sam/work/a.ics".into(), etag: "\"1\"".into() }]
        );
        assert_eq!(ch.removed, ["https://dav.example.com/dav/sam/work/b.ics"]);
        assert_eq!(ch.token, "http://x/sync/6");
    }
}
