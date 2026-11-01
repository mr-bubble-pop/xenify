//! The `.xplan` file: a zip holding `plan.json` (the plan, with its
//! history), and `preview.png` when the app adds a picture of the Gantt.

use crate::model::Plan;
use std::io::{Cursor, Read, Write};

/// The format level this Plan writes and understands. A file's `version` is
/// the lowest level a reader needs (keep writing the lowest one the plan
/// needs, so plain plans stay readable by older Plans); `saved_with` is the
/// writer's own level. Raise this when older Plans would misread something
/// new: a change kind kept in the history (`ops::Op`), a link or resource
/// kind, a changed meaning. See docs/formats.md.
pub const VERSION: u32 = 1;

const NEWER: &str = "This plan was saved by a newer Plan: update Plan to open it";

/// The most `plan.json` may hold (512 MiB). A plan of many thousands of
/// issues with years of history is a few tens of MB; a made-up file
/// claiming more would only fill the memory.
const MAX_JSON: u64 = 512 << 20;
/// The largest picture of the Gantt kept with a file (as the server takes).
const MAX_PREVIEW: u64 = 64 << 20;
/// The most tasks a plan may have. Big real plans have a few thousand; far
/// more would make working out the dates (which every copy and the server
/// do whenever it changes) take too long.
pub const MAX_TASKS: usize = 50_000;

const TOO_BIG: &str = "This plan is too big to open.";

/// Entry `name` of the zip, refused when it's bigger than `cap` (what
/// the zip says it holds isn't trusted).
fn read_capped(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str, cap: u64) -> Result<Option<Vec<u8>>, String> {
    let Ok(f) = z.by_name(name) else { return Ok(None) };
    let mut out = Vec::new();
    f.take(cap + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > cap {
        return Err(TOO_BIG.into());
    }
    Ok(Some(out))
}

pub fn save(plan: &Plan, preview: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let e = |e: zip::result::ZipError| e.to_string();
        z.start_file("plan.json", opts).map_err(e)?;
        let mut v = serde_json::to_value(plan).map_err(|e| e.to_string())?;
        v["version"] = VERSION.into();
        v["saved_with"] = VERSION.into();
        z.write_all(serde_json::to_string(&v).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
        if let Some(png) = preview {
            z.start_file(
                "preview.png",
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .map_err(e)?;
            z.write_all(png).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(e)?;
    }
    Ok(buf.into_inner())
}

pub fn open(bytes: &[u8]) -> Result<Plan, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "This isn't a Plan file".to_string())?;
    let json = read_capped(&mut z, "plan.json", MAX_JSON)?.ok_or_else(|| "This isn't a Plan file".to_string())?;
    let v: serde_json::Value = serde_json::from_slice(&json).map_err(|e| format!("This plan is damaged: {e}"))?;
    drop(json);
    if v["version"].as_u64().unwrap_or(1) > VERSION as u64 {
        return Err(NEWER.into());
    }
    let newer = v["saved_with"].as_u64().unwrap_or(0) > VERSION as u64;
    if v["tasks"].as_object().is_some_and(|t| t.len() > MAX_TASKS) {
        return Err(TOO_BIG.into());
    }
    serde_json::from_value(v).map_err(|e| if newer { NEWER.to_string() } else { format!("This plan is damaged: {e}") })
}

/// The picture saved in a file, if any.
pub fn preview(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    read_capped(&mut z, "preview.png", MAX_PREVIEW).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_bigger_than_its_cap_is_refused() {
        // A deflated plan.json that inflates past the cap: refused after
        // reading just past the cap.
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            z.start_file("plan.json", o).unwrap();
            z.write_all(&vec![b' '; 4 << 20]).unwrap();
            z.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        assert_eq!(read_capped(&mut z, "plan.json", 1 << 20).unwrap_err(), TOO_BIG);
        assert_eq!(read_capped(&mut z, "plan.json", 4 << 20).unwrap().unwrap().len(), 4 << 20);
        assert_eq!(read_capped(&mut z, "other", 1).unwrap(), None);
    }

    #[test]
    fn too_many_tasks_are_refused() {
        let mut p = Plan::default();
        for i in 0..=MAX_TASKS {
            p.tasks.insert(format!("t{i}"), Default::default());
        }
        assert_eq!(open(&save(&p, None).unwrap()).unwrap_err(), TOO_BIG);
        p.tasks.pop_first();
        assert!(open(&save(&p, None).unwrap()).is_ok());
    }

    #[test]
    fn what_a_newer_plan_added_says_update_not_damaged() {
        let zipped = |v: &serde_json::Value| {
            let mut buf = Cursor::new(Vec::new());
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("plan.json", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(v.to_string().as_bytes()).unwrap();
            z.finish().unwrap();
            buf.into_inner()
        };
        let saved = save(&Plan::default(), None).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&saved[..])).unwrap();
        let mut v: serde_json::Value = serde_json::from_reader(z.by_name("plan.json").unwrap()).unwrap();
        assert_eq!(v["saved_with"], VERSION);
        // Something this version reads differently (here: tasks of another shape).
        v["tasks"] = "kept another way".into();
        v["saved_with"] = (VERSION + 1).into();
        assert_eq!(open(&zipped(&v)).unwrap_err(), NEWER);
        v["saved_with"] = VERSION.into();
        assert!(open(&zipped(&v)).unwrap_err().contains("damaged"));
        v["version"] = (VERSION + 1).into();
        assert_eq!(open(&zipped(&v)).unwrap_err(), NEWER);
    }
}
