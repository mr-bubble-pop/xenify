//! Hostile and tricky HTML: nothing dangerous may come out, and ordinary
//! email formatting must survive.

use xenify_safe_html::{Options, clean, from_text};

fn no_images(_: &str) -> Option<String> {
    None
}

fn cleaned(html: &str) -> String {
    clean(html, &Options::default(), &no_images).html
}

/// Constructs that must never appear in the output, in any case.
const FORBIDDEN: &[&str] = &[
    "<script",
    "javascript:",
    "vbscript:",
    "onerror",
    "onload",
    "onclick",
    "onmouseover",
    "<iframe",
    "<object",
    "<embed",
    "<svg",
    "<math",
    "<form",
    "<input",
    "<style",
    "<link",
    "<meta",
    "<base",
    "expression(",
    "url(",
    "srcdoc",
    "<frame",
    "<template",
    "data:text",
];

fn assert_safe(html: &str) {
    let out = cleaned(html);
    let lower = out.to_ascii_lowercase();
    for f in FORBIDDEN {
        assert!(!lower.contains(f), "{f:?} survived:\n  in:  {html}\n  out: {out}");
    }
}

#[test]
fn xss_vectors_are_removed() {
    for v in [
        "<script>alert(1)</script>",
        "<SCRIPT SRC=//evil/x.js></SCRIPT>",
        "<img src=x onerror=alert(1)>",
        "<IMG SRC=\"jav&#x09;ascript:alert('XSS');\">",
        "<a href=\"javascript:alert(1)\">click</a>",
        "<a href=\" &#14;  javascript:alert(1)\">x</a>",
        "<a href=\"JaVaScRiPt:alert(1)\">x</a>",
        "<a href=\"vbscript:msgbox(1)\">x</a>",
        "<a href=\"data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==\">x</a>",
        "<body onload=alert(1)>hi</body>",
        "<svg><script>alert(1)</script></svg>",
        "<svg onload=alert(1)>",
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
        "<iframe src=\"https://evil\"></iframe>",
        "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe>",
        "<object data=\"evil.swf\"></object><embed src=evil.swf>",
        "<form action=https://evil><input name=password><button>Log in</button></form>",
        "<div style=\"background:url(javascript:alert(1))\">x</div>",
        "<div style=\"width: expression(alert(1))\">x</div>",
        "<div style=\"behavior: url(x.htc)\">x</div>",
        "<style>@import 'https://evil/x.css'; body{background:url(https://tracker/p.gif)}</style>hi",
        "<link rel=stylesheet href=https://evil/x.css>",
        "<meta http-equiv=refresh content=\"0;url=https://evil\">",
        "<base href=\"https://evil/\"><a href=\"/login\">x</a>",
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></noscript>",
        "<template><script>alert(1)</script></template>",
        "<!--<img src=x onerror=alert(1)>-->",
        "<![CDATA[<script>alert(1)</script>]]>",
        "<p>Hi<scr<script>ipt>alert(1)</script></p>",
        "<a href=\"https://ok.example\" onclick=\"steal()\">ok</a>",
        "<frameset><frame src=evil></frameset>",
        "<div style=\"color:red;\\62 ackground:url(x)\">x</div>",
    ] {
        assert_safe(v);
    }
}

#[test]
fn ordinary_formatting_survives() {
    let out = cleaned(
        "<html><head><style>.big { font-size: 18px; color: #123456 } td p { font-weight: bold }</style></head>
         <body><p class=\"big\">Hello <b>Sam</b>,</p>
         <table border=\"1\"><tr><td bgcolor=\"#eeeeee\"><p>Total</p></td><td>£12.50</td></tr></table>
         <ul><li>one</li><li>two</li></ul><a href=\"https://example.com/report\">the report</a></body></html>",
    );
    assert!(out.contains("<p style=\"font-size:18px;color:#123456;\">Hello <b>Sam</b>,</p>"), "{out}");
    assert!(out.contains("<td bgcolor=\"#eeeeee\"><p style=\"font-weight:bold;\">Total</p></td>"), "{out}");
    assert!(out.contains("<li>one</li>"), "{out}");
    assert!(out.contains("<a href=\"https://example.com/report\">the report</a>"), "{out}");
}

#[test]
fn hidden_preheaders_and_trackers_go() {
    let c = clean(
        "<div style=\"display:none;max-height:0;overflow:hidden\">Secret preheader text</div>
         <p>Visible</p><img src=\"https://t.example/open.gif\" width=\"1\" height=\"1\">
         <img src=\"https://cdn.example/x.gif\" style=\"width:1px;height:1px\">",
        &Options { remote_images: true, ..Default::default() },
        &no_images,
    );
    assert!(!c.html.contains("Secret"));
    assert!(c.html.contains("Visible"));
    assert_eq!(c.trackers, 2);
    assert!(c.to_fetch.is_empty(), "trackers are never fetched");
}

#[test]
fn remote_images_are_blocked_until_allowed() {
    let html = "<p>Logo: <img src=\"https://cdn.example/logo.png\" alt=\"Acme logo\" width=\"120\" height=\"40\"></p>";
    let c = clean(html, &Options::default(), &no_images);
    assert_eq!(c.blocked_images, 1);
    assert!(c.html.contains("[Acme logo]") && !c.html.contains("cdn.example"), "{}", c.html);

    let allowed = Options { remote_images: true, ..Default::default() };
    let c = clean(html, &allowed, &no_images);
    assert_eq!(c.to_fetch, ["https://cdn.example/logo.png"]);
    // After fetching, the resolver hands the picture back as data.
    let c = clean(html, &allowed, &|src| {
        (src == "https://cdn.example/logo.png").then(|| "data:image/png;base64,AAAA".into())
    });
    assert!(c.html.contains("<img src=\"data:image/png;base64,AAAA\" width=\"120\" height=\"40\""), "{}", c.html);
}

#[test]
fn pictures_inside_the_message_come_from_its_parts() {
    let c = clean("<img src=\"cid:logo@1\">", &Options::default(), &|src| {
        (src == "cid:logo@1").then(|| "data:image/png;base64,QUJD".into())
    });
    assert!(c.html.contains("data:image/png;base64,QUJD"));
    assert_eq!(c.blocked_images, 0);
    // Embedded pictures of the wrong kind are dropped.
    assert!(!cleaned("<img src=\"data:image/svg+xml;base64,PHN2Zz4=\">").contains("svg"));
}

#[test]
fn links_are_listed_with_warnings() {
    let c = clean(
        "<a href=\"https://evil.test/login\">https://mybank.com/login</a> <a href=\"https://mybank.com\">Your bank</a>",
        &Options::default(),
        &no_images,
    );
    assert_eq!(c.links.len(), 2);
    assert_eq!(c.links[0].text, "https://mybank.com/login");
    assert_eq!(c.links[0].warnings.len(), 1);
    assert!(c.links[1].warnings.is_empty());
}

#[test]
fn deep_nesting_does_not_crash() {
    let html = "<div>".repeat(50_000) + "deep" + &"</div>".repeat(50_000);
    assert!(cleaned(&html).contains("deep"));
    let html = "<b>".repeat(20_000) + "bold";
    assert!(cleaned(&html).contains("bold"));
}

#[test]
fn plain_text_becomes_paragraphs_with_links() {
    assert_eq!(
        from_text("Hi Sam,\n\nSee https://example.com/a.\nThanks <3"),
        "<p>Hi Sam,</p><p>See <a href=\"https://example.com/a\">https://example.com/a</a>.<br>Thanks &lt;3</p>"
    );
}
