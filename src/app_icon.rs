//! OS-level application icon.
//!
//! GTK offers no portable way to give an application an icon:
//!
//! - **Linux** looks the icon up in the desktop icon theme by name, so the
//!   app id has to match an installed `.desktop` entry and themed PNGs.
//! - **Windows** reads the icon from a resource compiled into the `.exe`
//!   (see `build.rs`).
//! - **macOS** takes the icon from the `.app` bundle, and when the binary
//!   runs on its own (no bundle) the Dock shows the generic executable
//!   icon. GTK's quartz backend never calls `setApplicationIconImage:`, so
//!   a bare binary has to set it itself.
//!
//! The icon bytes are embedded in the binary so a plain `cargo build`
//! output is branded with no extra files next to it.

/// Application id. Also the icon name used by the Linux desktop entry.
pub const APP_ID: &str = "dev.whispercrabs.app";

/// Apply the application icon to the running process.
///
/// Call after GTK has initialised (from `connect_startup`), so the macOS
/// `NSApplication` instance already exists.
pub fn install() {
    #[cfg(target_os = "macos")]
    {
        macos::set_dock_icon();

        // The Dock paints whatever `NSApplication.applicationIconImage`
        // holds, so confirm nothing replaced it once the window is up.
        gtk4::glib::timeout_add_seconds_local_once(3, || {
            macos::report_icon_state("after the window is up");
        });
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    gtk4::Window::set_default_icon_name(APP_ID);
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::AllocAnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    /// Multi-resolution icon, embedded so the binary is self-contained.
    const ICON_ICNS: &[u8] = include_bytes!("../assets/icon.icns");

    /// Replace the generic executable icon in the Dock with the app icon.
    pub fn set_dock_icon() {
        let Some(mtm) = MainThreadMarker::new() else {
            dbg_log!("[icon] not on the main thread, skipping dock icon");
            return;
        };

        let data = NSData::with_bytes(ICON_ICNS);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            eprintln!("Failed to decode the bundled application icon");
            return;
        };

        // SAFETY: runs on the main thread, and `image` is a valid NSImage
        // that outlives the call.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
        dbg_log!("[icon] dock icon applied");
    }

    /// Log what the application icon currently is.
    ///
    /// `applicationIconImage` hands back a rasterised copy rather than the
    /// image that was set, so this reports its dimensions instead of trying
    /// to compare bytes. Only runs under `--debug`.
    pub fn report_icon_state(when: &str) {
        if !crate::log::is_enabled() {
            return;
        }

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        match app.applicationIconImage() {
            Some(current) => {
                let size = current.size();
                dbg_log!("[icon] {when}: {}x{} pt", size.width, size.height);
            }
            None => dbg_log!("[icon] {when}: no application icon set"),
        }
    }
}
