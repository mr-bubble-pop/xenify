use xenify_wp_doc::{CharFmt, Document, Editor, Pos, Selection, StyleSheet, ops};
use xenify_wp_io::{Format, load, save};

#[test]
fn igdoc_round_trips_everything() {
    let mut ed = Editor::new(Document::from_plain_text("Heading\nSome bold text ✓\n"));
    ops::set_style(&mut ed, &StyleSheet::heading(2)).unwrap();
    let p = ed.doc.next_para(ed.doc.start().para).unwrap();
    ed.sel = Selection { anchor: Pos::new(p, 5), focus: Pos::new(p, 9) };
    ops::apply_format(&mut ed, &CharFmt { bold: Some(true), ..Default::default() }, "Bold").unwrap();
    ed.doc.styles.get_mut(StyleSheet::NORMAL).unwrap().char.font = Some("Source Serif 4".into());
    ed.doc.meta.title = "Test".into();

    let bytes = save(&ed.doc, Format::Igdoc).unwrap();
    let back = load(&bytes, Format::Igdoc).unwrap().doc;
    let strip = |d: &Document| serde_json::to_value(d).unwrap();
    assert_eq!(strip(&back), strip(&ed.doc));
    // Saving is deterministic.
    assert_eq!(save(&back, Format::Igdoc).unwrap(), bytes);
}

#[test]
fn newer_files_are_refused_politely() {
    let mut z = xenify_wp_io::zip::ZipWriter::new();
    z.add("document.json", br#"{"format":"x","format_version":99,"document":{}}"#, true);
    let err = load(&z.finish(), Format::Igdoc).err().unwrap().to_string();
    assert!(err.contains("newer"), "{err}");
}

#[test]
fn formats_from_names() {
    assert_eq!(Format::from_name("a/b/Report.IGDOC"), Some(Format::Igdoc));
    assert_eq!(Format::from_name("notes.txt"), Some(Format::Text));
    assert_eq!(Format::from_name("noext"), None);
}

/// Other zip tools can open our packages (checked with `unzip -t` when installed).
#[test]
fn packages_open_in_other_zip_tools() {
    let Ok(out) = std::process::Command::new("unzip").arg("-v").output() else { return };
    if !out.status.success() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("igw-zip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.igdoc");
    std::fs::write(&path, save(&Document::from_plain_text("hello"), Format::Igdoc).unwrap()).unwrap();
    let t = std::process::Command::new("unzip").arg("-t").arg(&path).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(t.status.success(), "{}", String::from_utf8_lossy(&t.stdout));
}
