//! Guards the icon wiring.
//!
//! The app id, the Linux desktop entry, and the icon files have to agree,
//! otherwise the OS quietly falls back to a generic executable icon and
//! nothing else fails.

use crate::app_icon::APP_ID;
use std::path::{Path, PathBuf};

fn asset(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(name)
}

#[test]
fn icon_assets_are_present() {
    for name in ["icon.icns", "icon.ico", "icon-128.png", "icon-256.png"] {
        let path = asset(name);
        assert!(path.is_file(), "missing icon asset: {}", path.display());
        let size = std::fs::metadata(&path).expect("icon asset metadata").len();
        assert!(size > 0, "empty icon asset: {}", path.display());
    }
}

#[test]
fn linux_desktop_entry_matches_app_id() {
    let path = asset(&format!("{APP_ID}.desktop"));
    let text = std::fs::read_to_string(&path).expect("desktop entry missing");

    assert!(
        text.contains("Type=Application"),
        "not an application entry"
    );
    assert!(
        text.contains(&format!("Icon={APP_ID}")),
        "Icon= must match the app id, got:\n{text}"
    );
    assert!(
        text.contains("Exec=whispercrabs"),
        "Exec must name the binary"
    );
}

#[test]
fn windows_build_script_embeds_the_icon() {
    let build_script =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
            .expect("build.rs missing");

    assert!(
        build_script.contains("assets/icon.ico"),
        "build.rs no longer attaches the .ico to the executable"
    );
}
