//! Every file under `assets/` must have an entry in `assets/SOURCES.toml`
//! with its source, an accepted licence and its current SHA-256, and every
//! generated entry must name code that exists.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use xenify_export::sources::{self, ACCEPTED, Origin};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn sha256(path: &Path) -> String {
    Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn every_asset_has_a_source_licence_and_hash() {
    let root = root();
    let text = std::fs::read_to_string(root.join("assets/SOURCES.toml")).expect("assets/SOURCES.toml");
    let entries = sources::parse(&text).expect("SOURCES.toml parses");
    let mut problems = Vec::new();

    let mut all = Vec::new();
    files(&root.join("assets"), &mut all);
    for f in all {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if rel == "assets/SOURCES.toml" {
            continue;
        }
        if !entries.iter().any(|e| e.path.as_deref() == Some(rel.as_str())) {
            problems.push(format!("{rel}: no entry in assets/SOURCES.toml"));
        }
    }

    for e in &entries {
        let who = e.path.clone().unwrap_or_else(|| e.name.clone());
        // The website alone may show a model's outputs under that model's own
        // terms (decided for MiniMax H3, whose clips site/ai/ only loads where
        // its licence allows); nothing that ships in the apps may.
        let site_only = e.path.as_deref().is_some_and(|p| p.starts_with("site/"))
            && e.license.starts_with("MiniMax H3 Community License Agreement");
        if !ACCEPTED.contains(&e.license.as_str()) && !site_only {
            problems.push(format!("{who}: licence {:?} isn't one we accept", e.license));
        }
        if e.license.starts_with("CC-BY") && e.attribution.as_deref().unwrap_or("").is_empty() {
            problems.push(format!("{who}: {} needs attribution text", e.license));
        }
        if e.fetched.as_deref().unwrap_or("").is_empty() {
            problems.push(format!("{who}: no date (fetched)"));
        }
        match e.source {
            Origin::Download => {
                if e.source_url.as_deref().unwrap_or("").is_empty() || e.version.as_deref().unwrap_or("").is_empty() {
                    problems.push(format!("{who}: a download needs source_url and version"));
                }
            }
            Origin::Generated => {
                let generator = e.generator.as_deref().unwrap_or("");
                if generator.is_empty() {
                    problems.push(format!("{who}: generated media must name its generator"));
                }
                // "path/a.rs (fn), path/b.rs": each named source file must exist
                // (words without a slash, like a model's name, aren't files).
                for part in generator.split(',') {
                    let file = part.split_whitespace().next().unwrap_or("");
                    if file.contains('/') && !root.join(file).exists() {
                        problems.push(format!("{who}: generator {file} doesn't exist"));
                    }
                }
                if !e.notes.as_deref().unwrap_or("").contains("o third-party material")
                    && !e.notes.as_deref().unwrap_or("").contains("Original")
                {
                    problems.push(format!("{who}: notes must say it's original / has no third-party material"));
                }
            }
        }
        if let Some(lf) = &e.license_file {
            if !entries.iter().any(|x| x.path.as_ref() == Some(lf)) {
                problems.push(format!("{who}: license_file {lf} has no entry of its own"));
            }
            if sources::licence_text(lf).is_none() {
                problems.push(format!("{who}: license_file {lf} isn't compiled in (rebuild?)"));
            }
        }
        if let Some(p) = &e.path {
            let full = root.join(p);
            if !full.exists() {
                problems.push(format!("{p}: listed but missing"));
            } else if e.sha256.as_deref() != Some(sha256(&full).as_str()) {
                problems.push(format!("{p}: SHA-256 is {}, SOURCES.toml says {:?}", sha256(&full), e.sha256));
            }
        }
    }
    assert!(problems.is_empty(), "asset provenance problems:\n{}", problems.join("\n"));
}

#[test]
fn the_compiled_in_record_matches_the_file() {
    assert_eq!(sources::bundled().len(), sources::parse(sources::SOURCES_TOML).unwrap().len());
    assert!(sources::licence_text("assets/fonts/OFL.txt").unwrap().contains("SIL OPEN FONT LICENSE"));
}

#[test]
fn the_shape_library_is_recorded_as_original_work() {
    assert!(sources::bundled().iter().any(|e| e.generator.as_deref() == Some("crates/xenify-ops/src/library.rs")));
    let src = std::fs::read_to_string(root().join("crates/xenify-ops/src/library.rs")).unwrap();
    assert!(src.contains("original work"), "library.rs must keep its provenance note");
}
