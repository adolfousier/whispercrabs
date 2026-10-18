//! # WhisperCrabs
//!
//! Local-first floating voice-to-text and text-to-speech tool for Linux, macOS, and Windows.
//!
//! Click to record, click to transcribe, text copied to clipboard.
//! Copy any text and click to hear it read aloud.
//!
//! **Local-first** — runs entirely on your machine with whisper.cpp (STT)
//! and Piper (TTS). No cloud required. Also supports any OpenAI-compatible
//! API endpoint for STT (Groq, Ollama, OpenRouter, LM Studio, etc.).
//!
//! ## Features
//!
//! - Floating always-on-top mic button (GTK4)
//! - STT — Local (whisper.cpp) or API (OpenAI-compatible)
//! - TTS — Local via Piper, 6 built-in voices, optional
//! - One-click STT/TTS switching via right-click menu
//! - Global keyboard shortcuts via D-Bus
//! - AI Agent-Ready: full D-Bus control

#[macro_use]
mod log;
mod api;
mod app_icon;
mod audio;
mod config;
mod db;
mod input;
mod local_stt;
#[cfg(test)]
mod tests;
mod tts;
mod ui;

use gtk4::prelude::*;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let debug = args.iter().any(|a| a == "--debug");
    log::init(debug);

    let config = Arc::new(config::Config::load());

    let app = gtk4::Application::builder()
        .application_id(app_icon::APP_ID)
        .build();

    // The OS-level icon is not something GTK can set portably: macOS needs
    // the Dock icon applied through NSApplication, Linux resolves it from
    // the desktop icon theme. Run once GTK is up, before the window shows.
    app.connect_startup(|_| app_icon::install());

    let config_c = Arc::clone(&config);
    app.connect_activate(move |app| {
        ui::build_ui(app, Arc::clone(&config_c));
    });

    // Filter out --debug so GTK4 doesn't reject it as unknown option
    let gtk_args: Vec<String> = args.into_iter().filter(|a| a != "--debug").collect();
    let gtk_args_ref: Vec<&str> = gtk_args.iter().map(|s| s.as_str()).collect();
    app.run_with_args(&gtk_args_ref);
}
