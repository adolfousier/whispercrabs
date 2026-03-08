//! # WhisperCrabs
//!
//! Floating voice-to-text tool for Linux, macOS, and Windows.
//!
//! Click to record, click to transcribe, text copied to clipboard.
//! Supports fully local transcription via whisper.cpp or any
//! OpenAI-compatible API endpoint (Groq, Ollama, OpenRouter, LM Studio, etc.).
//!
//! ## Features
//!
//! - Floating always-on-top mic button (GTK4)
//! - One-click provider switching via right-click menu
//! - Local transcription via whisper.cpp (no internet required)
//! - API transcription via any OpenAI-compatible endpoint
//! - Global keyboard shortcuts via D-Bus
//! - AI Agent-Ready: full D-Bus control

mod api;
mod audio;
mod config;
mod db;
mod input;
mod local_stt;
#[cfg(test)]
mod tests;
mod ui;

use gtk4::prelude::*;
use std::sync::Arc;

fn main() {
    let config = Arc::new(config::Config::load());

    let app = gtk4::Application::builder()
        .application_id("dev.whispercrabs.app")
        .build();

    let config_c = Arc::clone(&config);
    app.connect_activate(move |app| {
        ui::build_ui(app, Arc::clone(&config_c));
    });

    app.run();
}
