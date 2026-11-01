use crate::choose::{Local, effective, in_window, pick, update};
use crate::manifest::{Refused, check_file, new_key, sha256_hex, sign, verify};
use crate::*;

fn release(version: &str, channel: Channel, bytes: &[u8], url: &str) -> Release {
    Release {
        version: version.into(),
        channel,
        date: "2026-10-24".into(),
        notes: String::new(),
        downloads: vec![Download {
            os: "linux".into(),
            arch: "x86_64".into(),
            url: url.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            run: "xenify-write".into(),
        }],
    }
}

fn manifest(published: &str) -> Manifest {
    Manifest {
        published: published.into(),
        products: vec![Product {
            id: "write".into(),
            name: "Write".into(),
            about: String::new(),
            component: false,
            requires: Vec::new(),
            releases: vec![
                release("1.2.0", Channel::Release, b"a", "https://x/w.bin"),
                release("1.3.0-beta.1", Channel::Preview, b"b", "https://x/w.bin"),
                release("1.3.0-nightly.20261024", Channel::Nightly, b"c", "https://x/w.bin"),
                release("1.1.0", Channel::Release, b"d", "https://x/w.bin"),
            ],
        }],
    }
}

#[test]
fn only_lists_signed_by_a_known_key_and_unchanged_are_used() {
    let key = new_key();
    let other = new_key();
    let trusted = [key.verifying_key().to_bytes()];
    let signed = sign(&manifest("2026-10-24T10:00:00Z"), &key);
    assert_eq!(verify(&signed, &trusted, None).unwrap(), manifest("2026-10-24T10:00:00Z"));

    let mut changed = signed.clone();
    changed.manifest = changed.manifest.replace("1.2.0", "9.9.9");
    assert_eq!(verify(&changed, &trusted, None), Err(Refused::BadSignature));
    assert_eq!(verify(&sign(&manifest("2026-10-24T10:00:00Z"), &other), &trusted, None), Err(Refused::UnknownKey));
    // A list older than one already seen is refused (updates held back).
    assert_eq!(verify(&signed, &trusted, Some("2026-10-25T00:00:00Z")), Err(Refused::Older));
    assert!(verify(&signed, &trusted, Some("2026-10-23T00:00:00Z")).is_ok());
}

#[test]
fn each_channel_gets_its_newest_and_steadier_ones() {
    let m = manifest("t");
    let w = m.product("write").unwrap();
    let at = |c: Choice| pick(w, &c, "linux", "x86_64").map(|r| r.version.clone());
    assert_eq!(at(Choice::Channel(Channel::Release)).as_deref(), Some("1.2.0"));
    assert_eq!(at(Choice::Channel(Channel::Preview)).as_deref(), Some("1.3.0-beta.1"));
    assert_eq!(at(Choice::Channel(Channel::Nightly)).as_deref(), Some("1.3.0-nightly.20261024"));
    assert_eq!(at(Choice::Fixed("1.1.0".into())).as_deref(), Some("1.1.0"));
    assert_eq!(at(Choice::Fixed("0.1.0".into())), None);
    // Nothing for another system.
    assert_eq!(pick(w, &Choice::Channel(Channel::Nightly), "windows", "x86_64"), None);
}

#[test]
fn a_release_newer_than_the_preview_reaches_preview_people() {
    let mut m = manifest("t");
    m.products[0].releases.push(release("1.4.0", Channel::Release, b"e", "https://x/w.bin"));
    let w = m.product("write").unwrap();
    assert_eq!(pick(w, &Choice::Channel(Channel::Preview), "linux", "x86_64").unwrap().version, "1.4.0");
}

#[test]
fn all_apps_is_used_where_an_app_row_leaves_it_open() {
    let local = Local { choice: Choice::Channel(Channel::Release), mode: UpdateMode::Automatic };
    let mut policy = Policy { server: "Office".into(), ..Default::default() };
    policy.all_apps = crate::AppDefaults {
        choice: Some(Choice::Channel(Channel::Preview)),
        mode: Some(UpdateMode::Automatic),
        window: Some((22, 6)),
    };
    policy.products.insert(
        "write".into(),
        ProductPolicy { allowed: true, choice: Some(Choice::Channel(Channel::Nightly)), ..Default::default() },
    );
    // Its own row wins for the channel; "All apps" still says when.
    let w = effective(&local, Some(&policy), "write");
    assert_eq!(w.choice, Choice::Channel(Channel::Nightly));
    assert!(w.locked && w.mode_locked && w.window == Some((22, 6)));
    // Not mentioned: "All apps".
    let chat = effective(&local, Some(&policy), "chat");
    assert_eq!(chat.choice, Choice::Channel(Channel::Preview));
    assert!(chat.locked && chat.mode_locked);
    // The server has its own row: never "All apps".
    let srv = effective(&local, Some(&policy), "server");
    assert_eq!(srv.choice, Choice::Channel(Channel::Release));
    assert!(!srv.locked && !srv.mode_locked && srv.window.is_none());
    // An app's own "when" brings its own window (none: any time).
    policy.products.get_mut("write").unwrap().mode = Some(UpdateMode::Manual);
    let w = effective(&local, Some(&policy), "write");
    assert!(w.mode == UpdateMode::Manual && w.window.is_none());
}

#[test]
fn the_server_decides_when_it_manages_this_computer() {
    let local = Local { choice: Choice::Channel(Channel::Nightly), mode: UpdateMode::Automatic };
    let mut policy = Policy { server: "Office".into(), ..Default::default() };
    policy.products.insert(
        "write".into(),
        ProductPolicy {
            allowed: true,
            required: true,
            choice: Some(Choice::Fixed("1.1.0".into())),
            mode: Some(UpdateMode::Manual),
            window: Some((22, 6)),
        },
    );
    policy.products.insert("draw".into(), ProductPolicy { allowed: false, ..Default::default() });

    let e = effective(&local, Some(&policy), "write");
    assert_eq!(e.choice, Choice::Fixed("1.1.0".into()));
    assert!(e.locked && e.required);
    assert_eq!(e.managed_by.as_deref(), Some("Office"));
    // Not mentioned: the person's own choice.
    let chat = effective(&local, Some(&policy), "chat");
    assert_eq!(chat.choice, Choice::Channel(Channel::Nightly));
    assert!(!chat.locked && !chat.mode_locked && chat.allowed);
    // Only when it updates set: the version is still the person's choice.
    let mut only_mode = Policy::default();
    only_mode
        .products
        .insert("write".into(), ProductPolicy { allowed: true, mode: Some(UpdateMode::Manual), ..Default::default() });
    let e2 = effective(&local, Some(&only_mode), "write");
    assert!(!e2.locked && e2.mode_locked && e2.mode == UpdateMode::Manual);
    // Not allowed: never installed or updated.
    let m = manifest("t");
    let draw = effective(&local, Some(&policy), "draw");
    assert_eq!(update(m.product("write").unwrap(), None, &draw, "linux", "x86_64"), None);
    // Without a server, the person's choice.
    assert_eq!(effective(&local, None, "write").choice, Choice::Channel(Channel::Nightly));

    // Fixed below what's installed: back to it; at it: nothing to do.
    let w = m.product("write").unwrap();
    assert_eq!(update(w, Some("1.2.0"), &e, "linux", "x86_64").unwrap().version, "1.1.0");
    assert_eq!(update(w, Some("1.1.0"), &e, "linux", "x86_64"), None);

    assert!(in_window(Some((22, 6)), 23) && in_window(Some((22, 6)), 2) && !in_window(Some((22, 6)), 12));
    assert!(in_window(Some((1, 5)), 1) && !in_window(Some((1, 5)), 5) && in_window(None, 12));
}

fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, name, *data).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

#[test]
fn installs_keep_the_version_before_for_roll_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let v1 = tarball(&[("xenify-write", b"one"), ("lib/data.txt", b"x")]);
    let v2 = tarball(&[("xenify-write", b"two")]);
    let v3 = tarball(&[("xenify-write", b"three")]);
    let r1 = release("1.0.0", Channel::Release, &v1, "https://x/write-1.0.0.tar.gz");
    let r2 = release("1.1.0", Channel::Release, &v2, "https://x/write-1.1.0.tar.gz");
    let r3 = release("1.2.0", Channel::Release, &v3, "https://x/write-1.2.0.tar.gz");

    store.install("write", "1.0.0", &r1.downloads[0], &v1).unwrap();
    assert_eq!(std::fs::read(store.program("write").unwrap()).unwrap(), b"one");
    store.install("write", "1.1.0", &r2.downloads[0], &v2).unwrap();
    store.install("write", "1.2.0", &r3.downloads[0], &v3).unwrap();
    let now = store.installed("write").unwrap();
    assert_eq!((now.version.as_str(), now.previous.as_ref().unwrap().version.as_str()), ("1.2.0", "1.1.0"));
    // 1.0.0 is gone; 1.1.0 kept.
    assert!(!dir.path().join("apps/write/1.0.0").exists());
    assert!(dir.path().join("apps/write/1.1.0").exists());

    let back = store.roll_back("write").unwrap();
    assert_eq!(back.version, "1.1.0");
    assert_eq!(std::fs::read(store.program("write").unwrap()).unwrap(), b"two");
    // And forward again.
    assert_eq!(store.roll_back("write").unwrap().version, "1.2.0");
    // Putting the version in use back again keeps the one before.
    store.install("write", "1.2.0", &r3.downloads[0], &v3).unwrap();
    assert_eq!(store.installed("write").unwrap().previous.unwrap().version, "1.1.0");
    assert!(dir.path().join("apps/write/1.1.0").exists());

    store.remove("write").unwrap();
    assert_eq!(store.installed("write"), None);
}

#[test]
fn a_changed_or_cut_short_download_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let good = tarball(&[("xenify-write", b"one")]);
    let r = release("1.0.0", Channel::Release, &good, "https://x/w.tar.gz");
    assert!(check_file(&r.downloads[0], &good).is_ok());
    let mut bad = good.clone();
    bad[20] ^= 1;
    assert!(store.install("write", "1.0.0", &r.downloads[0], &bad).is_err());
    assert!(store.install("write", "1.0.0", &r.downloads[0], &good[..good.len() - 1]).is_err());
    assert_eq!(store.installed("write"), None);
}

#[test]
fn files_outside_the_apps_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hub"));
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("../../evil", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"x").unwrap();
    z.start_file("xenify-write", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"x").unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let r = release("1.0.0", Channel::Release, &bytes, "https://x/w.zip");
    assert!(store.install("write", "1.0.0", &r.downloads[0], &bytes).is_err());
    assert!(!dir.path().join("evil").exists());
}

#[test]
fn an_app_learns_a_newer_version_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let v1 = tarball(&[("bin/xenify-write", b"one")]);
    let v2 = tarball(&[("bin/xenify-write", b"two")]);
    let mut r1 = release("1.0.0", Channel::Release, &v1, "https://x/w1.tar.gz");
    let mut r2 = release("1.1.0", Channel::Release, &v2, "https://x/w2.tar.gz");
    r1.downloads[0].run = "bin/xenify-write".into();
    r2.downloads[0].run = "bin/xenify-write".into();
    store.install("write", "1.0.0", &r1.downloads[0], &v1).unwrap();
    let running = store.program("write").unwrap();
    assert_eq!(crate::install::newer_installed(&running), None);
    store.install("write", "1.1.0", &r2.downloads[0], &v2).unwrap();
    let (v, program) = crate::install::newer_installed(&running).unwrap();
    assert_eq!(v, "1.1.0");
    assert_eq!(std::fs::read(program).unwrap(), b"two");
    // Not installed by the Hub: nothing to say.
    assert_eq!(crate::install::newer_installed(std::path::Path::new("/usr/bin/xenify-write")), None);
}

#[test]
fn installs_from_a_downloaded_file_too() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("hub"));
    let v1 = tarball(&[("xenify-write", b"one")]);
    let r = release("1.0.0", Channel::Release, &v1, "https://x/w.tar.gz");
    let file = dir.path().join("download");
    std::fs::write(&file, &v1).unwrap();
    store.install_file("write", "1.0.0", &r.downloads[0], &file).unwrap();
    assert_eq!(std::fs::read(store.program("write").unwrap()).unwrap(), b"one");
    std::fs::write(&file, b"changed").unwrap();
    assert!(store.install_file("write", "1.0.0", &r.downloads[0], &file).is_err());
}

#[test]
fn calc_is_listed_installed_and_run_like_the_other_apps() {
    // A list with Calc beside Write, as the Release workflow publishes it.
    let pkg = tarball(&[("xenify-calc-1.0.0/xenify-calc", b"calc")]);
    let mut r = release("1.0.0", Channel::Release, &pkg, "https://x/calc-1.0.0-linux-x86_64.tar.gz");
    r.downloads[0].run = "xenify-calc-1.0.0/xenify-calc".into();
    let mut m = manifest("2026-10-25T10:00:00Z");
    m.products.push(Product {
        id: "calc".into(),
        name: "Calc".into(),
        about: "Spreadsheets with tables that keep their totals.".into(),
        component: false,
        requires: Vec::new(),
        releases: vec![r],
    });
    let key = new_key();
    let back = verify(&sign(&m, &key), &[key.verifying_key().to_bytes()], None).unwrap();
    let calc = back.product("calc").unwrap();
    assert_eq!(calc.name, "Calc");

    let local = Local { choice: Choice::Channel(Channel::Release), mode: UpdateMode::Automatic };
    let e = effective(&local, None, "calc");
    let rel = update(calc, None, &e, "linux", "x86_64").unwrap();
    assert_eq!(rel.version, "1.0.0");

    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.install("calc", "1.0.0", rel.download("linux", "x86_64").unwrap(), &pkg).unwrap();
    assert_eq!(std::fs::read(store.program("calc").unwrap()).unwrap(), b"calc");
    // Installing Calc leaves Write alone.
    assert_eq!(store.installed("write"), None);
}

#[test]
fn releases_older_than_the_oldest_safe_one_are_never_picked() {
    use crate::choose::{Policy, ProductPolicy};
    use crate::manifest::{Limits, safe, sign_with, verify};
    let key = new_key();
    let trusted = [key.verifying_key().to_bytes()];
    let limits = Limits { min_safe: [("write".to_string(), "1.2.0".to_string())].into(), ..Default::default() };
    let signed = sign_with(&manifest("2026-10-31T00:00:00Z"), &limits, &key);
    let m = verify(&signed, &trusted, None).unwrap();
    let w = m.product("write").unwrap();
    assert!(w.releases.iter().all(|r| r.version != "1.1.0"), "1.1.0 is below the floor");
    assert!(w.releases.iter().any(|r| r.version == "1.2.0"));
    // A server's policy pinning the unsafe version gets nothing to install.
    let policy = Policy {
        server: "Office".into(),
        products: [(
            "write".to_string(),
            ProductPolicy { allowed: true, choice: Some(Choice::Fixed("1.1.0".into())), ..Default::default() },
        )]
        .into(),
        ..Default::default()
    };
    let e = effective(&Local::default(), Some(&policy), "write");
    assert!(pick(w, &e.choice, "linux", "x86_64").is_none());
    assert!(!safe(&crate::manifest::limits(&signed.manifest), "write", "1.1.0"));
    assert!(safe(&crate::manifest::limits(&signed.manifest), "write", "1.3.0-beta.1"));
    assert!(safe(&crate::manifest::limits(&signed.manifest), "draw", "0.0.1"), "no floor for Draw");
}

#[test]
fn the_database_password_file_is_private_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("postgres-data.pw");
    std::fs::write(&p, "old").unwrap();
    crate::pg::write_private(&p, b"secret").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn a_list_from_a_newer_release_still_reads_or_says_update_the_hub() {
    use crate::manifest::{Limits, sign_with, verify};
    let key = new_key();
    let trusted = [key.verifying_key().to_bytes()];
    // A newer list: a release on a channel this Hub doesn't know, a product
    // whose shape changed, and a field it hasn't heard of.
    let mut list: serde_json::Value = serde_json::to_value(manifest("2026-10-31T00:00:00Z")).unwrap();
    let mut lts = list["products"][0]["releases"][0].clone();
    lts["channel"] = "long_term".into();
    list["products"][0]["releases"].as_array_mut().unwrap().push(lts);
    list["products"].as_array_mut().unwrap().push(serde_json::json!({ "id": "x", "releases": "later" }));
    list["mirrors"] = serde_json::json!(["https://m/"]);
    let text = serde_json::to_string(&list).unwrap();
    let signed = Signed {
        signature: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            ed25519_dalek::Signer::sign(&key, text.as_bytes()).to_bytes(),
        ),
        key: sign(&manifest("x"), &key).key,
        manifest: text,
    };
    let m = verify(&signed, &trusted, None).unwrap();
    assert_eq!(m, manifest("2026-10-31T00:00:00Z"), "the unknown parts are left out");

    let needs = Limits { min_hub: Some("999.0.0".into()), ..Default::default() };
    let signed = sign_with(&manifest("2026-10-31T00:00:00Z"), &needs, &key);
    assert_eq!(
        verify(&signed, &trusted, None),
        Err(Refused::NeedsNewerHub("999.0.0".into()))
    );
    let fine = Limits { min_hub: Some("0.0.1".into()), ..Default::default() };
    assert!(
        verify(&sign_with(&manifest("2026-10-31T00:00:00Z"), &fine, &key), &trusted, None)
            .is_ok()
    );
}

#[test]
fn settings_and_policies_from_a_newer_hub_still_read() {
    use crate::choose::{Choice, Local, Policy, UpdateMode};
    let local: Local = serde_json::from_str(r#"{"choice":{"channel":"long_term"},"mode":"when_idle"}"#).unwrap();
    assert_eq!(local, Local::default());
    let p: Policy = serde_json::from_str(
        r#"{"server":"Acme","products":{"write":{"required":true,"choice":{"channel":"long_term"},"mode":"manual","pin_os":"x"}}}"#,
    )
    .unwrap();
    let w = &p.products["write"];
    assert!(w.required && w.allowed);
    assert_eq!((w.choice.clone(), w.mode), (None, Some(UpdateMode::Manual)));
    let fixed: Local = serde_json::from_str(r#"{"choice":{"fixed":"1.2.0"}}"#).unwrap();
    assert_eq!(fixed.choice, Choice::Fixed("1.2.0".into()));
}
