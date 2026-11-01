//! Encryption at rest (optional, Linux): the server's own PostgreSQL and
//! its files live in one folder the kernel encrypts (fscrypt, v2 policy,
//! AES-256-XTS for contents and AES-256-CTS for names). After each start
//! the folder is locked until someone gives the key (`xenify-server
//! unlock` on the server's computer, or a key file). Then the server reads
//! and writes as usual; a reboot locks it again.
//!
//! The key is made from a passphrase with Argon2id. Kept beside the folder
//! (`<folder>.json`): the salt, the settings and the kernel's identifier of
//! the key, which tells a wrong passphrase from a right one. Never the key.
//!
//! The ext4 file system must allow encryption once (`tune2fs -O encrypt`,
//! as root); `install-server.sh --encrypt` does that.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `[at_rest]` in the settings file. Unknown keys (a typo, or from a newer
/// version after going back to an older one) are warned about by the
/// settings reader, not refused, so the server still starts.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AtRest {
    /// The encrypted folder: `postgres/` (the database), `blobs/` and
    /// `chat-files/` (people's files) go in it.
    pub dir: PathBuf,
    /// A file holding the passphrase: the server unlocks itself at start.
    /// Less safe (whoever reads the disk reads the key file too, unless it's
    /// elsewhere), for people who accept that.
    pub key_file: Option<PathBuf>,
}

/// What's kept beside the folder.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Params {
    /// How the key is made from the passphrase. Files from before this was
    /// kept used Argon2id version 0x13; a newer kind needs a newer server.
    #[serde(default = "argon2id")]
    pub kdf: String,
    pub salt: String,
    /// Argon2id memory (KiB) and passes.
    pub m: u32,
    pub t: u32,
    /// The kernel's identifier of the key (hex).
    pub key_id: String,
}

fn params_path(dir: &Path) -> PathBuf {
    let mut p = dir.as_os_str().to_owned();
    p.push(".json");
    PathBuf::from(p)
}

pub fn params(dir: &Path) -> Result<Params, String> {
    let text = std::fs::read_to_string(params_path(dir))
        .map_err(|e| format!("{}: {e} (set it up with `xenify-server at-rest setup`)", params_path(dir).display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", params_path(dir).display()))
}

const ARGON2ID: &str = "argon2id-13";

fn argon2id() -> String {
    ARGON2ID.into()
}

/// The 64-byte key a passphrase makes.
fn derive(passphrase: &str, p: &Params) -> Result<[u8; 64], String> {
    if p.kdf != ARGON2ID {
        return Err(format!(
            "the encrypted folder's key is made a way this version doesn't know ({}): it was set up by a newer Xenify server, so update the server",
            p.kdf
        ));
    }
    let salt = xenify_sync::unb64(&p.salt).map_err(|e| e.to_string())?;
    let params = argon2::Params::new(p.m, p.t, 1, Some(64)).map_err(|e| e.to_string())?;
    let mut key = [0u8; 64];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 16]> {
    let v: Vec<u8> = (0..s.len()).step_by(2).filter_map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect();
    v.try_into().ok()
}

/// A warning when the folder's disk is mounted the way Linux 7.0 can
/// crash with (ext4's dioread_nolock, its default, with encrypted files).
pub fn mount_warning(dir: &Path) -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let dir = std::fs::canonicalize(dir).ok()?;
    let (point, fs, opts) = mounts
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.len() > 3 && dir.starts_with(f[1])).then(|| (f[1].to_string(), f[2].to_string(), f[3].to_string()))
        })
        .max_by_key(|(p, _, _)| p.len())?;
    (fs == "ext4" && !opts.split(',').any(|o| o == "nodioread_nolock")).then(|| {
        format!(
            "{point} is mounted without nodioread_nolock: some Linux versions crash writing encrypted files then. \
             Run: sudo mount -o remount,nodioread_nolock {point}, and add nodioread_nolock to its options in /etc/fstab"
        )
    })
}

/// Whether the folder can be read now (its key is in the kernel).
pub fn unlocked(at: &AtRest) -> bool {
    params(&at.dir)
        .ok()
        .and_then(|p| unhex(&p.key_id))
        .is_some_and(|id| sys::key_present(&at.dir, &id).unwrap_or(false))
}

/// Gives the kernel the key `passphrase` makes. A wrong passphrase makes
/// another key, which is taken back out and refused.
pub fn unlock(at: &AtRest, passphrase: &str) -> Result<(), String> {
    let p = params(&at.dir)?;
    let want = unhex(&p.key_id).ok_or("the key identifier kept beside the folder is damaged")?;
    let key = derive(passphrase.trim_end_matches(['\r', '\n']), &p)?;
    let got = sys::add_key(&at.dir, &key)?;
    if got != want {
        let _ = sys::remove_key(&at.dir, &got);
        return Err("that isn't the key".into());
    }
    Ok(())
}

/// Unlocks with the key file, when there's one.
pub fn unlock_with_file(at: &AtRest) -> Result<bool, String> {
    let Some(f) = &at.key_file else { return Ok(false) };
    let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
    unlock(at, text.trim()).map(|()| true)
}

/// Makes `dir` (new and empty) an encrypted folder whose key comes from
/// `passphrase`, and keeps its settings beside it.
pub fn setup(dir: &Path, passphrase: &str) -> Result<(), String> {
    if dir.exists() && std::fs::read_dir(dir).map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} isn't empty: give a new folder", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p0 = Params { kdf: argon2id(), salt: xenify_sync::b64(&xenify_sync::random(16)), m: 64 * 1024, t: 3, key_id: String::new() };
    let key = derive(passphrase, &p0)?;
    let id = sys::add_key(dir, &key)?;
    sys::set_policy(dir, &id)?;
    let p = Params { key_id: hex(&id), ..p0 };
    crate::tls::write_private(&params_path(dir), &serde_json::to_string_pretty(&p).expect("serialises"))
        .map_err(|e| format!("{}: {e}", params_path(dir).display()))?;
    Ok(())
}

/// A new passphrase to write down: six groups of four letters and digits.
pub fn new_passphrase() -> String {
    let alphabet = b"abcdefghjkmnpqrstuvwxyz23456789";
    let r = xenify_sync::random(24);
    r.chunks(4)
        .map(|c| c.iter().map(|b| alphabet[*b as usize % alphabet.len()] as char).collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(target_os = "linux")]
mod sys {
    //! The kernel's fscrypt calls (linux/fscrypt.h).

    use std::os::fd::AsRawFd;
    use std::path::Path;

    const SPEC_IDENTIFIER: u32 = 2;
    // _IOWR('f', 23, struct fscrypt_add_key_arg): 80 bytes before the key.
    const ADD_KEY: libc::c_ulong = 0xC050_6617;
    // _IOWR('f', 24, struct fscrypt_remove_key_arg): 64 bytes.
    const REMOVE_KEY: libc::c_ulong = 0xC040_6618;
    // _IOWR('f', 26, struct fscrypt_get_key_status_arg): 128 bytes.
    const KEY_STATUS: libc::c_ulong = 0xC080_661A;
    // _IOR('f', 19, struct fscrypt_policy_v1), used for v2 policies too.
    const SET_POLICY: libc::c_ulong = 0x800C_6613;

    fn open(dir: &Path) -> Result<std::fs::File, String> {
        std::fs::File::open(dir).map_err(|e| format!("{}: {e}", dir.display()))
    }

    fn call(f: &std::fs::File, req: libc::c_ulong, arg: &mut [u8]) -> Result<(), std::io::Error> {
        // SAFETY: `arg` is the struct the request expects, sized for it.
        let r = unsafe { libc::ioctl(f.as_raw_fd(), req as _, arg.as_mut_ptr()) };
        if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }

    fn spec(id: &[u8; 16]) -> [u8; 40] {
        let mut s = [0u8; 40];
        s[..4].copy_from_slice(&SPEC_IDENTIFIER.to_ne_bytes());
        s[8..24].copy_from_slice(id);
        s
    }

    fn explain(e: std::io::Error, dir: &Path) -> String {
        match e.raw_os_error() {
            Some(libc::EOPNOTSUPP) | Some(libc::ENOTTY) => format!(
                "{}'s file system can't encrypt folders: it must be ext4 (or f2fs) with encryption allowed, \
                 once, as root: sudo tune2fs -O encrypt <its disk> (install-server.sh --encrypt does it)",
                dir.display()
            ),
            Some(libc::EDQUOT) => "too many keys added by this user: restart the computer".into(),
            _ => format!("{}: {e}", dir.display()),
        }
    }

    pub fn add_key(dir: &Path, key: &[u8; 64]) -> Result<[u8; 16], String> {
        let f = open(dir)?;
        let mut arg = vec![0u8; 80 + key.len()];
        arg[..4].copy_from_slice(&SPEC_IDENTIFIER.to_ne_bytes());
        arg[40..44].copy_from_slice(&(key.len() as u32).to_ne_bytes());
        arg[80..].copy_from_slice(key);
        let r = call(&f, ADD_KEY, &mut arg).map_err(|e| explain(e, dir));
        let id: [u8; 16] = arg[8..24].try_into().expect("16 bytes");
        arg.iter_mut().for_each(|b| *b = 0);
        r.map(|()| id)
    }

    pub fn remove_key(dir: &Path, id: &[u8; 16]) -> Result<(), String> {
        let f = open(dir)?;
        let mut arg = [0u8; 64];
        arg[..40].copy_from_slice(&spec(id));
        call(&f, REMOVE_KEY, &mut arg).map_err(|e| explain(e, dir))
    }

    pub fn key_present(dir: &Path, id: &[u8; 16]) -> Result<bool, String> {
        let f = open(dir)?;
        let mut arg = [0u8; 128];
        arg[..40].copy_from_slice(&spec(id));
        call(&f, KEY_STATUS, &mut arg).map_err(|e| explain(e, dir))?;
        // FSCRYPT_KEY_STATUS_PRESENT.
        Ok(u32::from_ne_bytes(arg[64..68].try_into().expect("4 bytes")) == 2)
    }

    pub fn set_policy(dir: &Path, id: &[u8; 16]) -> Result<(), String> {
        let f = open(dir)?;
        // struct fscrypt_policy_v2: version 2, AES-256-XTS contents,
        // AES-256-CTS names, names padded to 32, then the key identifier.
        let mut p = [0u8; 24];
        p[0] = 2;
        p[1] = 1;
        p[2] = 4;
        p[3] = 0x03;
        p[8..24].copy_from_slice(id);
        call(&f, SET_POLICY, &mut p).map_err(|e| explain(e, dir))
    }
}

#[cfg(not(target_os = "linux"))]
mod sys {
    use std::path::Path;

    const NO: &str = "encryption at rest needs Linux (on Windows or a Mac, use BitLocker or FileVault)";

    pub fn add_key(_: &Path, _: &[u8; 64]) -> Result<[u8; 16], String> {
        Err(NO.into())
    }
    pub fn remove_key(_: &Path, _: &[u8; 16]) -> Result<(), String> {
        Err(NO.into())
    }
    pub fn key_present(_: &Path, _: &[u8; 16]) -> Result<bool, String> {
        Err(NO.into())
    }
    pub fn set_policy(_: &Path, _: &[u8; 16]) -> Result<(), String> {
        Err(NO.into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn passphrases_and_identifiers() {
        let p = super::new_passphrase();
        assert_eq!(p.len(), 6 * 4 + 5);
        let id = [7u8; 16];
        assert_eq!(super::unhex(&super::hex(&id)), Some(id));
        assert_eq!(super::unhex("zz"), None);
    }

    #[test]
    fn key_settings_say_how_the_key_is_made() {
        let old: super::Params = serde_json::from_str(r#"{"salt":"c2FsdA==","m":65536,"t":3,"key_id":""}"#).unwrap();
        assert_eq!(old.kdf, "argon2id-13", "made before kdf was kept");
        let newer = super::Params { kdf: "argon3".into(), ..old };
        assert!(super::derive("pass", &newer).unwrap_err().contains("newer Xenify server"));
    }
}
