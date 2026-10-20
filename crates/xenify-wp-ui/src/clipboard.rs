//! The system clipboard beyond plain text: HTML (so formatting survives
//! pasting into and out of other programs) and pictures. One long-lived
//! handle, because on Linux the copying program has to keep serving what it
//! copied. Without a clipboard (headless tests) every call quietly does
//! nothing.

use std::cell::RefCell;

thread_local! {
    static BOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

/// Off in tests: they run side by side and mustn't share the computer's clipboard.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn with<R>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<R>) -> Option<R> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    BOARD.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = arboard::Clipboard::new().ok();
        }
        f(b.as_mut()?)
    })
}

/// Puts `html` on the clipboard, with `plain` for programs that only take
/// text. False when there's no clipboard to use.
pub fn write(plain: &str, html: &str) -> bool {
    with(|c| c.set().html(html, Some(plain)).ok()).is_some()
}

/// The clipboard's text.
pub fn text() -> Option<String> {
    with(|c| c.get().text().ok()).filter(|t| !t.is_empty())
}

/// The clipboard's HTML, if what was copied had formatting.
pub fn html() -> Option<String> {
    with(|c| c.get().html().ok()).filter(|h| !h.trim().is_empty())
}

/// A picture on the clipboard, as PNG bytes and its size in pixels.
pub fn image_png() -> Option<(Vec<u8>, u32, u32)> {
    let img = with(|c| c.get_image().ok())?;
    let (w, h) = (img.width as u32, img.height as u32);
    let buf = image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?;
    let mut png = Vec::new();
    buf.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((png, w, h))
}

/// Whether plain `text` reads as Markdown (headings, lists, tables,
/// emphasis), so pasting it can keep that structure.
pub fn looks_like_markdown(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim_start).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let marked = lines
        .iter()
        .filter(|l| {
            let heading = l.starts_with('#') && l.trim_start_matches('#').starts_with(' ');
            let bullet = ["* ", "- ", "+ "].iter().any(|b| l.starts_with(b));
            let number =
                l.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            let table = l.starts_with('|') && l.ends_with('|');
            let quote = l.starts_with("> ");
            let fence = l.starts_with("```");
            heading || bullet || number || table || quote || fence
        })
        .count();
    marked * 3 >= lines.len() || text.contains("**") && marked > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_recognised() {
        assert!(looks_like_markdown("# Plan\n\n* milk\n* eggs\n"));
        assert!(looks_like_markdown("| a | b |\n|---|---|\n| 1 | 2 |"));
        assert!(!looks_like_markdown("Just a sentence.\nAnd another one."));
        assert!(!looks_like_markdown("# not a heading alone"));
    }
}
