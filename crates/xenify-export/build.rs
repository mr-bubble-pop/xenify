//! Compiles the licence texts named in assets/SOURCES.toml into the binary,
//! so Help › Credits can show them (see src/sources.rs).

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sources = root.join("assets/SOURCES.toml");
    println!("cargo:rerun-if-changed={}", sources.display());
    let text = std::fs::read_to_string(&sources).expect("assets/SOURCES.toml");
    let mut files: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("license_file"))
        .filter_map(|rest| rest.split('"').nth(1).map(str::to_string))
        .collect();
    files.sort();
    files.dedup();
    let mut out = String::from("pub const LICENCE_TEXTS: &[(&str, &str)] = &[\n");
    for f in &files {
        let full =
            root.join(f).canonicalize().unwrap_or_else(|_| panic!("{f} (license_file in SOURCES.toml) is missing"));
        println!("cargo:rerun-if-changed={}", full.display());
        out += &format!("    ({f:?}, include_str!({:?})),\n", full.display().to_string());
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("licence_texts.rs");
    std::fs::write(dest, out).unwrap();
}
