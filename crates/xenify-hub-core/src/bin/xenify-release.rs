//! Makes Xenify's signed release list (scripts/release.sh runs this).
//!
//!   xenify-release keygen <key file>
//!       A new release key (keep it offline); prints its public half for
//!       the Hub's trusted keys.
//!   xenify-release public <key file>
//!       Prints a key's public half (for the Hub's trusted keys).
//!   xenify-release add <list.json> <product> <name> <version> <channel> <os> <arch> <file> <url> <run>
//!       Adds a build to the (unsigned) list, with its size and sha256.
//!   xenify-release describe <list.json> <product> <name> <about> [part] [requires,…]
//!       Sets what the Hub shows for a product; `part` marks one the server
//!       uses (PostgreSQL), `requires` the parts installed with it.
//!   xenify-release sign <list.json> <key file> <signed.json> [--min-safe product=version]… [--min-hub version]
//!       Signs the list (stamping when it was published), with each
//!       product's oldest safe version (Hubs never install an older one).
//!       A signed list never runs out.

use std::process::ExitCode;
use xenify_hub_core::manifest::{Limits, new_key, now, rfc3339, sha256_hex, sign_with};
use xenify_hub_core::{Channel, Download, Manifest, Product, Release};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xenify-release: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_list(path: &str) -> Result<Manifest, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{path}: {e}")),
        Err(_) => Ok(Manifest { published: String::new(), products: Vec::new() }),
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let a = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| "missing arguments (see the top of xenify-release.rs)".to_string())
    };
    match a(0)? {
        "keygen" => {
            let key = new_key();
            let path = a(1)?;
            if std::path::Path::new(path).exists() {
                return Err(format!("{path} exists: not replacing a release key"));
            }
            std::fs::write(path, key.to_bytes()).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
            }
            println!("{:?}", key.verifying_key().to_bytes());
            Ok(())
        }
        "public" => {
            let seed: [u8; 32] =
                std::fs::read(a(1)?).map_err(|e| e.to_string())?.try_into().map_err(|_| "that isn't a release key")?;
            println!("{:?}", ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes());
            Ok(())
        }
        "add" => {
            let (list, product, name, version, channel, os, arch, file, url, run) =
                (a(1)?, a(2)?, a(3)?, a(4)?, a(5)?, a(6)?, a(7)?, a(8)?, a(9)?, a(10)?);
            semver::Version::parse(version).map_err(|e| format!("version {version}: {e}"))?;
            let channel: Channel = serde_json::from_value(serde_json::json!(channel))
                .map_err(|_| "channel: release, preview or nightly")?;
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let mut m = read_list(list)?;
            let at = match m.products.iter().position(|p| p.id == product) {
                Some(i) => i,
                None => {
                    m.products.push(Product {
                        id: product.into(),
                        name: name.into(),
                        about: String::new(),
                        component: false,
                        requires: Vec::new(),
                        releases: Vec::new(),
                    });
                    m.products.len() - 1
                }
            };
            let p = &mut m.products[at];
            let r = match p.releases.iter().position(|r| r.version == version) {
                Some(i) => &mut p.releases[i],
                None => {
                    p.releases.push(Release {
                        version: version.into(),
                        channel,
                        date: today(),
                        notes: String::new(),
                        downloads: Vec::new(),
                    });
                    p.releases.last_mut().unwrap()
                }
            };
            r.downloads.retain(|d| !(d.os == os && d.arch == arch));
            r.downloads.push(Download {
                os: os.into(),
                arch: arch.into(),
                url: url.into(),
                size: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
                run: run.into(),
            });
            std::fs::write(list, serde_json::to_string_pretty(&m).unwrap()).map_err(|e| e.to_string())
        }
        "describe" => {
            let (list, product, name, about) = (a(1)?, a(2)?, a(3)?, a(4)?);
            let mut m = read_list(list)?;
            let rest: Vec<&str> = args[5..].iter().map(String::as_str).collect();
            let at = match m.products.iter().position(|p| p.id == product) {
                Some(i) => i,
                None => {
                    m.products.push(Product {
                        id: product.into(),
                        name: name.into(),
                        about: String::new(),
                        component: false,
                        requires: Vec::new(),
                        releases: Vec::new(),
                    });
                    m.products.len() - 1
                }
            };
            let p = &mut m.products[at];
            p.name = name.into();
            p.about = about.into();
            p.component = rest.contains(&"part");
            p.requires = rest
                .iter()
                .filter(|r| **r != "part")
                .flat_map(|r| r.split(','))
                .filter(|r| !r.is_empty())
                .map(String::from)
                .collect();
            std::fs::write(list, serde_json::to_string_pretty(&m).unwrap()).map_err(|e| e.to_string())
        }
        "sign" => {
            let mut m = read_list(a(1)?)?;
            let seed: [u8; 32] =
                std::fs::read(a(2)?).map_err(|e| e.to_string())?.try_into().map_err(|_| "that isn't a release key")?;
            let mut limits = Limits::default();
            let out = a(3)?;
            let mut rest = args.get(4..).unwrap_or(&[]).iter();
            while let Some(opt) = rest.next() {
                let value = rest.next().ok_or_else(|| format!("{opt} needs a value"))?;
                match opt.as_str() {
                    "--min-safe" => {
                        let (product, version) =
                            value.split_once('=').ok_or_else(|| format!("--min-safe {value}: use product=version"))?;
                        semver::Version::parse(version).map_err(|e| format!("version {version}: {e}"))?;
                        limits.min_safe.insert(product.to_string(), version.to_string());
                    }
                    // Only when the list changes in a way older Hubs would misread.
                    "--min-hub" => {
                        semver::Version::parse(value).map_err(|e| format!("version {value}: {e}"))?;
                        limits.min_hub = Some(value.to_string());
                    }
                    other => return Err(format!("unknown option “{other}”")),
                }
            }
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            m.published = rfc3339(secs as i64);
            let signed = sign_with(&m, &limits, &ed25519_dalek::SigningKey::from_bytes(&seed));
            std::fs::write(out, serde_json::to_string_pretty(&signed).unwrap()).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown command “{other}”")),
    }
}

fn today() -> String {
    now()[..10].to_string()
}
