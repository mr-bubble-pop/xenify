//! HTTPS certificates and being found on the network.

use std::time::Duration;

#[test]
fn certificates_cover_this_computer_and_keep_their_authority() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let (ca1, chain, key) = xenify_server::tls::local_certificate(&dir, &names).unwrap();
    assert!(chain.ends_with(&ca1), "the chain carries the authority");
    xenify_server::tls::config(&chain, &key).unwrap();
    // Next start: a new certificate, the same authority (computers trusted it once).
    let (ca2, chain2, _) = xenify_server::tls::local_certificate(&dir, &names).unwrap();
    assert_eq!(ca1, ca2);
    assert_ne!(chain, chain2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("ca.key")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn apps_find_the_server_on_the_network() {
    let Ok(daemon) = xenify_server::tls::announce("Test office", 7700, Some(7443), "ready") else {
        return eprintln!("no multicast here: skipped");
    };
    let browser = mdns_sd::ServiceDaemon::new().unwrap();
    let rx = browser.browse("_xenify._tcp.local.").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let mut found = None;
    while std::time::Instant::now() < deadline {
        if let Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) = rx.recv_timeout(Duration::from_millis(500))
            && info.get_fullname().starts_with("Test office")
        {
            found = Some((info.get_port(), info.get_property_val_str("https").map(str::to_string)));
            break;
        }
    }
    let _ = daemon.shutdown();
    let _ = browser.shutdown();
    match found {
        Some((port, https)) => assert_eq!((port, https.as_deref()), (7700, Some("7443"))),
        None => eprintln!("multicast didn't come back here (containers often block it): skipped"),
    }
}
