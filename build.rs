//! Build script.
//!
//! Windows executables carry their icon as a compiled resource, so the icon
//! has to be attached at link time. Other platforms get their icon from the
//! app bundle (macOS) or the desktop icon theme (Linux), both handled at
//! runtime, so there is nothing to do here.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "WhisperCrabs");
        res.set(
            "FileDescription",
            "WhisperCrabs - local-first voice-to-text and text-to-speech",
        );
        res.compile().expect("failed to compile Windows resources");
    }
}
