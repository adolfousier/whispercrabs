//! Simple file-based debug logger.
//!
//! When `--debug` is passed, all `log!()` calls write timestamped lines
//! to `~/whispercrabs-debug.log`. When debug is off, `log!()` is a no-op.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Initialize the logger. Call once at startup.
pub fn init(debug: bool) {
    if !debug {
        return;
    }
    ENABLED.store(true, Ordering::Relaxed);

    let path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("whispercrabs-debug.log");

    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        *LOG_FILE.lock().unwrap() = Some(file);
        eprintln!("Debug logging to {}", path.display());
    }
}

/// Whether `--debug` enabled logging. Use it to skip work whose only
/// purpose is producing a log line.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Write a debug log line (with timestamp). No-op if `--debug` was not passed.
pub fn debug(msg: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }

    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut f) = *guard {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "[{now}] {msg}");
            let _ = f.flush();
        }
    }
}

/// Convenience macro for formatted debug logging.
#[macro_export]
macro_rules! dbg_log {
    ($($arg:tt)*) => {
        $crate::log::debug(&format!($($arg)*))
    };
}
