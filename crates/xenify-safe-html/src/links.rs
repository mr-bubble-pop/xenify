//! Which links are allowed, and warnings for ones that try to fool you.

use url::{Host, Url};

/// Something suspicious about a link, shown before it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Warning {
    /// The text shows one web address but the link goes somewhere else.
    TextShowsOtherSite { shown: String, actual: String },
    /// The address uses look-alike letters (it's spelt in another alphabet,
    /// shown here as it really reads).
    LookAlikeLetters { host: String, reads_as: String },
    /// The address is a bare number (IP address) rather than a name.
    NumericAddress(String),
    /// `https://yourbank.com@other.site/`: the part before `@` isn't where
    /// it goes.
    HiddenDestination { actual: String },
}

impl Warning {
    pub fn describe(&self) -> String {
        match self {
            Warning::TextShowsOtherSite { shown, actual } => {
                format!("The link text shows {shown}, but it actually goes to {actual}.")
            }
            Warning::LookAlikeLetters { host, reads_as } => format!(
                "The address {reads_as} uses letters from another alphabet that look like ordinary ones ({host}). It may be pretending to be a site you know."
            ),
            Warning::NumericAddress(host) => {
                format!("The link goes to a numeric address ({host}) instead of a named website.")
            }
            Warning::HiddenDestination { actual } => {
                format!("The start of the address is a disguise; it really goes to {actual}.")
            }
        }
    }
}

/// The link if it's one we allow (web, mail and phone links), normalised.
pub fn allowed(href: &str) -> Option<Url> {
    let href = href.trim();
    let url = Url::parse(href).ok()?;
    match url.scheme() {
        "http" | "https" => url.host().is_some().then_some(url),
        "mailto" | "tel" => Some(url),
        _ => None,
    }
}

/// The host as people read it (`www.` dropped, look-alike letters decoded).
fn readable_host(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let (unicode, _) = idna::domain_to_unicode(&host);
    Some(unicode.trim_start_matches("www.").to_string())
}

/// What's wrong with a link whose text is `text`.
pub fn warnings(url: &Url, text: &str) -> Vec<Warning> {
    let mut out = Vec::new();
    if !matches!(url.scheme(), "http" | "https") {
        return out;
    }
    let Some(actual) = readable_host(url) else { return out };
    if !url.username().is_empty() || url.password().is_some() {
        out.push(Warning::HiddenDestination { actual: actual.clone() });
    }
    match url.host() {
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => out.push(Warning::NumericAddress(actual.clone())),
        Some(Host::Domain(d)) if d.split('.').any(|l| l.starts_with("xn--")) => {
            out.push(Warning::LookAlikeLetters { host: d.to_string(), reads_as: actual.clone() })
        }
        _ => {}
    }
    if let Some(shown) = shown_site(text)
        && shown != actual
        && !actual.ends_with(&format!(".{shown}"))
        && !shown.ends_with(&format!(".{actual}"))
    {
        out.push(Warning::TextShowsOtherSite { shown, actual });
    }
    out
}

/// The site the link text names, if the text is a web address
/// ("www.bank.com", "https://bank.com/login").
fn shown_site(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['/', '.', ',', ')']);
    if t.is_empty() || t.contains(char::is_whitespace) || t.contains('@') && !t.contains("://") {
        return None;
    }
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let url = Url::parse(&with_scheme).ok()?;
    let host = url.host_str()?;
    // Looks like a domain: has a dot and a letters-only last part.
    let tld = host.rsplit('.').next()?;
    if !host.contains('.') || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    readable_host(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warn(href: &str, text: &str) -> Vec<Warning> {
        warnings(&allowed(href).unwrap(), text)
    }

    #[test]
    fn only_web_mail_and_phone_links() {
        assert!(allowed("https://example.com/a").is_some());
        assert!(allowed("mailto:sam@example.com").is_some());
        assert!(allowed("javascript:alert(1)").is_none());
        assert!(allowed(" JaVaScRiPt:alert(1)").is_none());
        assert!(allowed("data:text/html,<script>").is_none());
        assert!(allowed("file:///etc/passwd").is_none());
        assert!(allowed("/relative/path").is_none());
    }

    #[test]
    fn honest_links_get_no_warning() {
        assert!(warn("https://www.example.com/login", "www.example.com").is_empty());
        assert!(warn("https://mail.example.com/x", "example.com").is_empty());
        assert!(warn("https://example.com/x", "Click here").is_empty());
        assert!(warn("https://example.com/x", "Read more on Example.com today").is_empty());
    }

    #[test]
    fn fooling_links_are_explained() {
        assert_eq!(
            warn("https://evil.test/login", "https://mybank.com/login"),
            [Warning::TextShowsOtherSite { shown: "mybank.com".into(), actual: "evil.test".into() }]
        );
        assert!(matches!(warn("https://xn--pypal-4ve.com/", "Pay")[0], Warning::LookAlikeLetters { .. }));
        assert!(matches!(warn("http://192.168.1.4/", "Bank")[0], Warning::NumericAddress(_)));
        assert!(matches!(warn("https://mybank.com@evil.test/", "Bank")[0], Warning::HiddenDestination { .. }));
    }
}
