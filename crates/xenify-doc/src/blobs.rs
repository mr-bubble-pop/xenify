//! The bytes of pictures and clips, kept apart from the document.
//!
//! Written [`by_reference`], a project names each picture's or clip's file
//! by its SHA-256 (`"sha256:<hex>"`): what a Xenify server and `.xdraw`
//! files keep, each file once beside the document. Otherwise the bytes go
//! in the JSON (base64), for copies that stay in this program. Reading
//! either kind takes the bytes from this store (bytes read inline are put
//! in it), so a document read by reference needs its files [`put`] first.

use base64::Engine;
use serde::{Deserialize, Deserializer, Serializer};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

const PREFIX: &str = "sha256:";

#[derive(Default)]
struct Store {
    /// By where the bytes are in memory: them, and their hash. A file
    /// that hasn't arrived is an empty placeholder here (not in `by_hash`),
    /// so writing it again keeps its name.
    by_ptr: HashMap<usize, (Arc<[u8]>, String)>,
    by_hash: HashMap<String, Arc<[u8]>>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Default::default);

thread_local! {
    static BY_REF: Cell<u32> = const { Cell::new(0) };
}

fn store() -> std::sync::MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

fn ptr(a: &Arc<[u8]>) -> usize {
    Arc::as_ptr(a) as *const u8 as usize
}

/// SHA-256 of `data`, in lowercase hex.
pub fn sha256(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn valid(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Keeps `data`; its hash. The same bytes kept already are shared.
pub fn put(data: impl Into<Arc<[u8]>>) -> String {
    let data = data.into();
    let hash = sha256(&data);
    keep(data, &hash);
    hash
}

/// Keeps `data` under `hash`, returning the copy kept (the earlier one, if
/// any).
fn keep(data: Arc<[u8]>, hash: &str) -> Arc<[u8]> {
    let mut s = store();
    if let Some(a) = s.by_hash.get(hash) {
        return a.clone();
    }
    s.by_ptr.insert(ptr(&data), (data.clone(), hash.to_string()));
    s.by_hash.insert(hash.to_string(), data.clone());
    data
}

/// The file with this hash, if it's here.
pub fn get(hash: &str) -> Option<Arc<[u8]>> {
    store().by_hash.get(hash).cloned()
}

pub fn has(hash: &str) -> bool {
    store().by_hash.contains_key(hash)
}

/// The hash of bytes a project holds (worked out once, then remembered).
pub fn hash_of(data: &Arc<[u8]>) -> String {
    if let Some((_, h)) = store().by_ptr.get(&ptr(data)) {
        return h.clone();
    }
    let hash = sha256(data);
    let mut s = store();
    s.by_ptr.insert(ptr(data), (data.clone(), hash.clone()));
    s.by_hash.entry(hash.clone()).or_insert_with(|| data.clone());
    hash
}

/// Whether these bytes stand in for a file that hasn't arrived.
pub fn is_missing(data: &Arc<[u8]>) -> bool {
    if !data.is_empty() {
        return false;
    }
    let s = store();
    s.by_ptr.get(&ptr(data)).is_some_and(|(_, h)| !s.by_hash.get(h).is_some_and(|a| Arc::ptr_eq(a, data)))
}

/// Writes JSON (serde) with files named by hash, not inside it.
pub fn by_reference<R>(f: impl FnOnce() -> R) -> R {
    BY_REF.with(|c| c.set(c.get() + 1));
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            BY_REF.with(|c| c.set(c.get() - 1));
        }
    }
    let _done = Done;
    f()
}

/// The files `json` names, each once, in order.
pub fn refs(json: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = json;
    while let Some(i) = rest.find(PREFIX) {
        let h = rest.get(i + PREFIX.len()..i + PREFIX.len() + 64).unwrap_or("");
        if valid(h) && !out.iter().any(|o| o == h) {
            out.push(h.to_string());
        }
        rest = &rest[i + PREFIX.len()..];
    }
    out
}

/// The files `json` names that aren't here.
pub fn missing(json: &str) -> Vec<String> {
    refs(json).into_iter().filter(|h| !has(h)).collect()
}

/// Lets go of files no project holds any more.
pub fn forget_unused() {
    let mut s = store();
    let Store { by_ptr, by_hash } = &mut *s;
    by_ptr.retain(|_, (a, h)| {
        let held_here = 1 + usize::from(by_hash.get(h).is_some_and(|b| Arc::ptr_eq(a, b)));
        Arc::strong_count(a) > held_here
    });
    by_hash.retain(|_, a| by_ptr.contains_key(&ptr(a)));
}

/// A field holding a file's bytes (`#[serde(with = "crate::blobs::field")]`).
pub mod field {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Arc<[u8]>, s: S) -> Result<S::Ok, S::Error> {
        if is_missing(v) || BY_REF.with(Cell::get) > 0 {
            return s.serialize_str(&format!("{PREFIX}{}", hash_of(v)));
        }
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<[u8]>, D::Error> {
        let text = String::deserialize(d)?;
        if let Some(hash) = text.strip_prefix(PREFIX) {
            if !valid(hash) {
                return Err(serde::de::Error::custom("not a file's name"));
            }
            if let Some(a) = get(hash) {
                return Ok(a);
            }
            // Not here (yet): an empty stand-in that remembers the name.
            let stand_in: Arc<[u8]> = Arc::from(Vec::new());
            store().by_ptr.insert(ptr(&stand_in), (stand_in.clone(), hash.to_string()));
            return Ok(stand_in);
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(text).map_err(serde::de::Error::custom)?;
        let hash = sha256(&bytes);
        Ok(keep(bytes.into(), &hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "field")]
        data: Arc<[u8]>,
    }

    #[test]
    fn inline_or_by_reference() {
        let h = Holder { data: Arc::from(b"blobs test one".to_vec()) };
        let inline = serde_json::to_string(&h).unwrap();
        assert!(!inline.contains(PREFIX), "{inline}");
        let named = by_reference(|| serde_json::to_string(&h).unwrap());
        let hash = sha256(b"blobs test one");
        assert_eq!(named, format!(r#"{{"data":"sha256:{hash}"}}"#));
        assert_eq!(refs(&named), vec![hash.clone()]);
        // Read back by name: the same bytes, shared.
        let back: Holder = serde_json::from_str(&named).unwrap();
        assert_eq!(back, h);
        assert!(Arc::ptr_eq(&back.data, &h.data));
        assert_eq!(serde_json::from_str::<Holder>(&inline).unwrap(), h);
    }

    #[test]
    fn a_file_not_here_keeps_its_name() {
        let hash = sha256(b"blobs test never put");
        let json = format!(r#"{{"data":"sha256:{hash}"}}"#);
        assert_eq!(missing(&json), vec![hash.clone()]);
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert!(back.data.is_empty() && is_missing(&back.data));
        // Written again, even inline: still its name, not an empty file.
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        put(b"blobs test never put".to_vec());
        assert!(missing(&json).is_empty());
        let now: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(&*now.data, b"blobs test never put");
    }
}
