//! For trying Xenify Chat's email screens without a real account: an IMAP and an
//! SMTP server on 127.0.0.1 for one address (no encryption, everything in
//! memory, gone when it stops). Prints the ports. Mail sent through the SMTP
//! server to the test address lands in its inbox.
//!
//! `cargo run -p xenify-mail-core --example test_mail_server [address]`

#[path = "../tests/common/imap_server.rs"]
mod imap_server;
#[path = "../tests/common/smtp_server.rs"]
mod smtp_server;

const PASSWORD: &str = "test-password";

fn main() {
    let user = std::env::args().nth(1).unwrap_or_else(|| "sam@example.org".into());
    let user = user.as_str();
    let (imap_port, imap) = imap_server::start(user, PASSWORD);
    let (smtp_port, sent) = smtp_server::start(user, PASSWORD);
    {
        let mut s = imap.lock().unwrap();
        s.add("INBOX", &imap_server::message("Ana Ruiz <ana@example.com>", "Lunch on Friday", "Pizza at noon? The usual place."), &["\\Seen"]);
        s.add(
            "INBOX",
            "From: Build Robot <builds@example.com>\nTo: Sam Lee <sam@example.org>\nSubject: Weekly report\nDate: Fri, 16 Oct 2026 08:00:00 +0100\nMessage-ID: <report-1@test>\nMIME-Version: 1.0\nContent-Type: text/html; charset=utf-8\n\n<html><body><h2 style=\"color:#C2410C\">Weekly report</h2><p>All <b>42</b> builds passed.</p><p><img src=\"https://tracker.example.com/pixel.png\" width=1 height=1><img src=\"https://example.com/chart.png\"></p><script>alert('no')</script><p><a href=\"https://example.com/report\">Full report</a></p></body></html>\n",
            &[],
        );
        s.add("INBOX", &imap_server::message("Jordan Ortiz <jordan@example.com>", "Contract draft", "Could you read the contract before Monday?\n\nThanks,\nJordan"), &[]);
        // A meeting invitation (weekly, starting tomorrow at 10:00 UTC).
        let tomorrow = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() / 86_400 + 1) as i64;
        let (y, m, d) = civil(tomorrow);
        let ics = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Test//EN\nMETHOD:REQUEST\nBEGIN:VEVENT\nUID:design-review-1@test\nDTSTAMP:20261018T090000Z\n\
             DTSTART:{y:04}{m:02}{d:02}T100000Z\nDTEND:{y:04}{m:02}{d:02}T110000Z\nRRULE:FREQ=WEEKLY\nSUMMARY:Design review\nLOCATION:Room 4\n\
             ORGANIZER;CN=Jordan Ortiz:mailto:jordan@example.com\nATTENDEE;CN=Sam Lee;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{user}\nSEQUENCE:0\nEND:VEVENT\nEND:VCALENDAR\n"
        );
        s.add(
            "INBOX",
            &format!(
                "From: Jordan Ortiz <jordan@example.com>\nTo: Sam Lee <{user}>\nSubject: Invitation: Design review\nDate: Sun, 18 Oct 2026 09:00:00 +0000\nMessage-ID: <invite-1@test>\nMIME-Version: 1.0\n\
                 Content-Type: multipart/alternative; boundary=b1\n\n--b1\nContent-Type: text/plain; charset=utf-8\n\nYou're invited to the design review.\n\
                 --b1\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\n\n{ics}--b1--\n"
            ),
            &[],
        );
    }
    println!("{user} / {PASSWORD}: IMAP 127.0.0.1:{imap_port}, SMTP 127.0.0.1:{smtp_port} (not encrypted)");
    // Deliver what's sent to the test address.
    let mut delivered = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let got = sent.lock().unwrap().clone();
        for r in got.iter().skip(delivered) {
            let subject = r.data.lines().find(|l| l.starts_with("Subject:")).unwrap_or("");
            let calendar = r.data.lines().find(|l| l.to_ascii_lowercase().starts_with("content-type: text/calendar")).unwrap_or("");
            let answer = r.data.lines().find(|l| l.contains("PARTSTAT=")).unwrap_or("");
            println!("received for {:?}: {subject} | {calendar} | {answer}", r.to);
            if r.to.iter().any(|t| t.contains(user)) {
                imap.lock().unwrap().add("INBOX", &r.data.replace("\r\n", "\n"), &[]);
            }
        }
        delivered = got.len();
    }
}

/// The calendar date of `days` since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
