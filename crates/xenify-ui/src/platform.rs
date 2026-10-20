//! The OS services the app uses, swappable for fakes in tests (so tests never
//! touch the real clipboard, file manager or printers).

use xenify_platform::PlatformError;
use xenify_platform::desktop::{ClipContent, Clipboard, PrintOptions, Printer, Shell};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Folders shared with this container, if Xenify Vector runs in one (host path, path here).
pub fn shared_folders() -> &'static [(PathBuf, PathBuf)] {
    static SHARED: std::sync::OnceLock<Vec<(PathBuf, PathBuf)>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        if !xenify_platform::desktop::in_container() {
            return Vec::new();
        }
        let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        xenify_platform::desktop::shared_folders(&info)
    })
}

/// A file path from the desktop (drag and drop, copied files), as this
/// process can open it: host paths inside shared folders are translated.
pub fn local_path(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    xenify_platform::desktop::host_to_local(path, shared_folders()).unwrap_or_else(|| path.to_path_buf())
}

/// Why `path` (which doesn't exist here) can't be opened, in plain words.
pub fn unreachable_reason(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let shared = shared_folders();
    if shared.is_empty() {
        return format!("Couldn't find {}", path.display());
    }
    let folders: Vec<String> = shared.iter().map(|(h, _)| h.display().to_string()).collect();
    format!(
        "“{name}” is outside the folders Xenify Vector's dev container can see ({}). Move it there, or copy the picture itself and paste it (Ctrl+V).",
        folders.join(", ")
    )
}

pub struct Platform {
    pub clipboard: Box<dyn Clipboard>,
    pub shell: Box<dyn Shell>,
    pub printer: Box<dyn Printer>,
}

impl Platform {
    pub fn native() -> Self {
        use xenify_platform::desktop::native::{CupsPrinter, SystemClipboard, SystemShell};
        Self { clipboard: Box::new(SystemClipboard), shell: Box::new(SystemShell), printer: Box::new(CupsPrinter) }
    }

    /// Fakes that record what the app asked for.
    pub fn fake(log: FakeLog) -> Self {
        Self {
            clipboard: Box::new(FakeClipboard(log.clone())),
            shell: Box::new(FakeShell(log.clone())),
            printer: Box::new(FakePrinter(log)),
        }
    }
}

/// Shared state of the fakes: what the clipboard holds, and what was done.
#[derive(Clone, Default)]
pub struct FakeLog(pub Arc<Mutex<FakeState>>);

#[derive(Default)]
pub struct FakeState {
    pub clipboard: Option<ClipContent>,
    pub clipboard_reads: usize,
    pub revealed: Vec<PathBuf>,
    pub printers: Vec<String>,
    pub printed: Vec<(PathBuf, PrintOptions)>,
}

impl FakeLog {
    pub fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
}

struct FakeClipboard(FakeLog);
impl Clipboard for FakeClipboard {
    fn read(&mut self) -> Option<ClipContent> {
        self.0.with(|s| {
            s.clipboard_reads += 1;
            s.clipboard.clone()
        })
    }
    fn write_image(&mut self, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), PlatformError> {
        self.0.with(|s| s.clipboard = Some(ClipContent::Image { width, height, rgba }));
        Ok(())
    }
}

struct FakeShell(FakeLog);
impl Shell for FakeShell {
    fn reveal(&self, path: &Path) -> Result<(), PlatformError> {
        self.0.with(|s| s.revealed.push(path.to_path_buf()));
        Ok(())
    }
}

struct FakePrinter(FakeLog);
impl Printer for FakePrinter {
    fn unavailable(&self) -> Option<String> {
        self.0.with(|s| s.printers.is_empty().then(|| "no printers".to_string()))
    }
    fn printers(&self) -> Vec<String> {
        self.0.with(|s| s.printers.clone())
    }
    fn print_file(&self, path: &Path, opts: &PrintOptions) -> Result<(), PlatformError> {
        self.0.with(|s| s.printed.push((path.to_path_buf(), opts.clone())));
        Ok(())
    }
}
