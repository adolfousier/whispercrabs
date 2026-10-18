use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::audio::Recorder;
use crate::config::{self, Config, TranscriptionService, TtsProvider};
use crate::db::Db;
use crate::local_stt::LocalWhisper;
use crate::tts::{ApiTts, PiperTts, Speech, TtsEngine};

const MIC_SVG: &[u8] = include_bytes!("icons/microphone.svg");
const NOTIFICATION_SOUND: &[u8] = include_bytes!("audio/notification.wav");

fn play_notification() {
    std::thread::spawn(|| {
        use rodio::{Decoder, OutputStream, Sink};
        use std::io::Cursor;
        if let Ok((_stream, handle)) = OutputStream::try_default()
            && let Ok(sink) = Sink::try_new(&handle)
            && let Ok(source) = Decoder::new(Cursor::new(NOTIFICATION_SOUND))
        {
            sink.append(source);
            sink.sleep_until_end();
        }
    });
}

const CSS: &str = r#"
    window.main-window {
        background-color: transparent;
    }
    window.main-window.macos-bg {
        background-color: rgba(17, 17, 17, 0.92);
    }
    .macos-bg .mic-btn {
        min-width: 68px;
        min-height: 68px;
    }
    .mic-btn {
        min-width: 72px;
        min-height: 72px;
        border-radius: 9999px;
        background-image: none;
        background-color: #dc2626;
        color: white;
        font-size: 32px;
        font-weight: 600;
        border: none;
        box-shadow: none;
        outline: none;
        -gtk-icon-shadow: none;
        -gtk-icon-size: 32px;
        padding: 0;
    }
    .mic-btn:hover {
        background-image: none;
        background-color: #b91c1c;
        box-shadow: none;
    }
    .mic-btn:active {
        background-image: none;
        background-color: #991b1b;
        box-shadow: none;
    }
    .mic-btn.recording,
    .mic-btn.recording:hover {
        background-image: none;
        background-color: #16a34a;
        box-shadow: none;
        animation: pulse 1s ease-in-out infinite;
    }
    .mic-btn.processing,
    .mic-btn.processing:hover {
        background-image: none;
        background-color: #d97706;
        box-shadow: none;
    }
    .mic-btn.done,
    .mic-btn.done:hover {
        background-image: none;
        background-color: #16a34a;
        box-shadow: none;
    }
    .mic-btn.synthesizing,
    .mic-btn.synthesizing:hover {
        background-image: none;
        background-color: #d97706;
        box-shadow: none;
    }
    .mic-btn.speaking,
    .mic-btn.speaking:hover {
        background-image: none;
        background-color: #16a34a;
        box-shadow: none;
        animation: pulse 1s ease-in-out infinite;
    }
    @keyframes pulse {
        0%   { opacity: 1.0; }
        50%  { opacity: 0.7; }
        100% { opacity: 1.0; }
    }
    .brand-label {
        color: rgba(255, 255, 255, 0.4);
        font-size: 9px;
        font-weight: 500;
        letter-spacing: 1px;
        margin-top: 4px;
    }
    .status-label {
        color: #e2e8f0;
        font-size: 12px;
        font-weight: 500;
        background-color: rgba(15, 23, 42, 0.75);
        border-radius: 6px;
        padding: 3px 8px;
    }
"#;

/// Show a status message inline. On macOS, also shows a dialog for errors.
fn show_status(label: &gtk4::Label, text: &str) {
    dbg_log!("[STATUS] {text}");
    label.set_label(text);
    label.set_opacity(1.0);

    #[cfg(target_os = "macos")]
    {
        let is_error = text.starts_with("No ")
            || text.starts_with("Err:")
            || text.contains("failed")
            || text.contains("Failed");
        if is_error
            && let Some(window) = label.root().and_then(|r| r.downcast::<gtk4::Window>().ok())
        {
            let dialog = gtk4::MessageDialog::builder()
                .transient_for(&window)
                .modal(true)
                .message_type(gtk4::MessageType::Error)
                .buttons(gtk4::ButtonsType::Ok)
                .text(text)
                .build();
            let content = dialog.message_area();
            content.set_margin_top(12);
            content.set_margin_bottom(12);
            content.set_margin_start(16);
            content.set_margin_end(16);
            dialog.connect_response(|d, _| d.close());
            dialog.show();
        }
    }
}

/// Hide the status label
fn hide_status(label: &gtk4::Label) {
    label.set_opacity(0.0);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Recording,
    Processing,
    Synthesizing,
    Speaking,
}

struct RuntimeState {
    active_service: TranscriptionService,
    active_provider: String, // "groq", "ollama", ..., "custom", "local"
    api_base_url: String,    // active API base URL
    api_key: Option<String>, // active API key
    api_model: String,       // active API model
    local_whisper: Option<Arc<LocalWhisper>>,
    downloading: bool,
    tts_provider: TtsProvider,
    tts_voice: String,
    /// Active preset id for API TTS ("openai", "openrouter", "custom").
    tts_preset: String,
    tts_api_url: String,
    tts_api_key: Option<String>,
    tts_api_model: String,
    tts_engine: Option<Arc<TtsEngine>>,
    tts_downloading: bool,
    tts_stop: Arc<std::sync::atomic::AtomicBool>,
}

pub fn build_ui(app: &gtk4::Application, config: Arc<Config>) {
    // Load CSS
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(CSS);
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().expect("no default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("WhisperCrabs")
        .default_width(88)
        .default_height(100)
        .decorated(false)
        .resizable(false)
        .css_classes(vec!["main-window"])
        .build();

    #[cfg(target_os = "macos")]
    window.add_css_class("macos-bg");

    // Layout
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    vbox.set_halign(gtk4::Align::Center);
    vbox.set_valign(gtk4::Align::Center);

    // The mic button (no keyboard activation to prevent accidental recordings)
    // Try system icon first (works on Linux), fall back to SVG pixbuf
    let icon = gtk4::Image::from_icon_name("audio-input-microphone-symbolic");
    icon.set_pixel_size(32);

    // Check if icon resolved; if not, load bundled SVG
    #[cfg(target_os = "macos")]
    {
        let loader =
            gtk4::gdk_pixbuf::PixbufLoader::with_type("svg").expect("SVG loader unavailable");
        loader.write(MIC_SVG).expect("failed to write SVG data");
        loader.close().expect("failed to finalize SVG loader");
        let pixbuf = loader.pixbuf().expect("failed to load mic icon");
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        icon.set_paintable(Some(&texture));
    }

    let button = gtk4::Button::new();
    button.set_child(Some(&icon));
    button.add_css_class("mic-btn");
    button.set_size_request(72, 72);
    button.set_halign(gtk4::Align::Center);
    button.set_focusable(false);

    let status = gtk4::Label::new(Some(" "));
    status.add_css_class("status-label");
    status.set_opacity(0.0);

    vbox.append(&button);

    // On macOS there's no transparent window, so show branding
    #[cfg(target_os = "macos")]
    {
        icon.set_pixel_size(36);
        button.set_size_request(68, 68);
        let brand = gtk4::Label::new(Some("WHISPER\nCRABS"));
        brand.set_justify(gtk4::Justification::Center);
        brand.add_css_class("brand-label");
        vbox.append(&brand);
        window.set_default_size(96, 110);
    }

    // On macOS, status floats as overlay so it doesn't affect window layout.
    // On Linux, it's appended normally (transparent window handles it fine).
    #[cfg(not(target_os = "macos"))]
    vbox.append(&status);

    // WindowHandle wraps everything — makes the empty area around
    // the button draggable like a titlebar. Clicks on the Button
    // itself still go through to the button's click handler.
    let handle = gtk4::WindowHandle::new();

    #[cfg(target_os = "macos")]
    {
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&vbox));
        status.set_halign(gtk4::Align::Center);
        status.set_valign(gtk4::Align::End);
        status.set_margin_bottom(4);
        overlay.add_overlay(&status);
        handle.set_child(Some(&overlay));
    }
    #[cfg(not(target_os = "macos"))]
    handle.set_child(Some(&vbox));

    window.set_child(Some(&handle));

    // Open DB
    let db = Arc::new(Mutex::new(
        Db::open(&config.db_path).expect("Failed to open database"),
    ));

    // Determine initial provider: DB setting overrides env var
    let (initial_service, initial_provider, initial_base_url, initial_api_key, initial_api_model) = {
        let db_provider = db
            .lock()
            .ok()
            .and_then(|d| d.get_setting("transcription_mode").ok().flatten());
        match db_provider.as_deref() {
            // Legacy "local" maps to default local model
            Some("local") => (
                TranscriptionService::Local,
                config::DEFAULT_LOCAL_MODEL.to_string(),
                config.api_base_url.clone(),
                config.api_key.clone(),
                config.api_model.clone(),
            ),
            Some("custom") => {
                let d = db.lock().expect("db lock poisoned");
                let url = d
                    .get_setting("api_custom_url")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| config.api_base_url.clone());
                let key = d
                    .get_setting("api_custom_key")
                    .ok()
                    .flatten()
                    .or_else(|| config.api_key.clone());
                let model = d
                    .get_setting("api_custom_model")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| config.api_model.clone());
                (
                    TranscriptionService::Api,
                    "custom".to_string(),
                    url,
                    key,
                    model,
                )
            }
            Some(provider_id) => {
                if let Some(preset) = config::find_preset(provider_id) {
                    // API preset
                    let key = if preset.needs_key {
                        db.lock()
                            .ok()
                            .and_then(|d| {
                                d.get_setting(&format!("api_key_{}", preset.id))
                                    .ok()
                                    .flatten()
                            })
                            .or_else(|| config.api_key.clone())
                    } else {
                        None
                    };
                    (
                        TranscriptionService::Api,
                        provider_id.to_string(),
                        preset.base_url.to_string(),
                        key,
                        preset.default_model.to_string(),
                    )
                } else if config::find_local_model(provider_id).is_some() {
                    // Local model preset (e.g. "local-base", "local-small")
                    (
                        TranscriptionService::Local,
                        provider_id.to_string(),
                        config.api_base_url.clone(),
                        config.api_key.clone(),
                        config.api_model.clone(),
                    )
                } else {
                    // Unknown provider in DB, fall back to env var config
                    (
                        config.transcription_service,
                        "groq".to_string(),
                        config.api_base_url.clone(),
                        config.api_key.clone(),
                        config.api_model.clone(),
                    )
                }
            }
            None => {
                // No DB setting — use env var config
                let provider = if config.transcription_service == TranscriptionService::Local {
                    config::DEFAULT_LOCAL_MODEL
                } else {
                    "groq"
                };
                (
                    config.transcription_service,
                    provider.to_string(),
                    config.api_base_url.clone(),
                    config.api_key.clone(),
                    config.api_model.clone(),
                )
            }
        }
    };

    // Init local whisper only if Local mode AND the selected model file exists
    let initial_whisper: Option<Arc<LocalWhisper>> =
        if initial_service == TranscriptionService::Local {
            let lm = config::find_local_model(&initial_provider)
                .unwrap_or(&config::LOCAL_MODEL_PRESETS[0]); // default to "tiny"
            let model_path = config.models_dir.join(lm.file_name);
            if model_path.exists() {
                match LocalWhisper::new(&model_path) {
                    Ok(w) => Some(Arc::new(w)),
                    Err(e) => {
                        eprintln!("Failed to load whisper model: {e}");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

    // Load TTS state from DB, falling back to the env defaults
    let piper_dir = config.models_dir.join("piper");
    let tts_settings = db.lock().ok().map(|d| TtsSettings {
        provider: d.get_setting("tts_provider").ok().flatten(),
        voice: d.get_setting("tts_voice").ok().flatten(),
        preset: d.get_setting("tts_api_preset").ok().flatten(),
        url: d.get_setting("tts_api_url").ok().flatten(),
        key: d.get_setting("tts_api_key").ok().flatten(),
        model: d.get_setting("tts_api_model").ok().flatten(),
    });
    let tts_state = resolve_tts_state(
        tts_settings,
        &config,
        &piper_dir,
        config::DEFAULT_PIPER_VOICE,
    );
    let initial_tts_voice = tts_state.voice.clone();
    let initial_tts_provider = tts_state.provider;
    let initial_tts_preset = tts_state.preset.clone();
    let initial_tts_url = tts_state.url.clone();
    let initial_tts_key = tts_state.key.clone();
    let initial_tts_model = tts_state.model.clone();
    let initial_tts_engine = tts_state.engine;

    // Runtime state (UI-thread only)
    let runtime = Rc::new(RefCell::new(RuntimeState {
        active_service: initial_service,
        active_provider: initial_provider.clone(),
        api_base_url: initial_base_url,
        api_key: initial_api_key,
        api_model: initial_api_model,
        local_whisper: initial_whisper,
        downloading: false,
        tts_provider: initial_tts_provider,
        tts_voice: initial_tts_voice,
        tts_preset: initial_tts_preset.clone(),
        tts_api_url: initial_tts_url,
        tts_api_key: initial_tts_key,
        tts_api_model: initial_tts_model,
        tts_engine: initial_tts_engine,
        tts_downloading: false,
        tts_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }));

    // Shared state
    let state = Rc::new(RefCell::new(State::Idle));
    let recorder = Rc::new(RefCell::new(Recorder::new()));

    // --- Left-click handler (on the Button) ---
    let btn = button.clone();
    let st = status.clone();
    let state_c = Rc::clone(&state);
    let rec_c = Rc::clone(&recorder);
    let config_c = Arc::clone(&config);
    let db_c = Arc::clone(&db);
    let runtime_c = Rc::clone(&runtime);

    button.connect_clicked(move |_| {
        let current = *state_c.borrow();
        match current {
            State::Idle => {
                // Guard: block recording during model download
                if runtime_c.borrow().downloading {
                    show_status(&st, "Downloading model...");
                    return;
                }

                // Guard: Local mode without loaded model
                let rt = runtime_c.borrow();
                if rt.active_service == TranscriptionService::Local && rt.local_whisper.is_none() {
                    drop(rt);
                    show_status(&st, "No local model loaded");
                    return;
                }

                // Guard: API mode — check if provider needs key and none is set
                if rt.active_service == TranscriptionService::Api {
                    let needs_key = config::find_preset(&rt.active_provider)
                        .map(|p| p.needs_key)
                        .unwrap_or(true); // custom defaults to needing a key check
                    if needs_key && rt.api_key.is_none() {
                        drop(rt);
                        show_status(&st, "No API key set");
                        return;
                    }
                }
                drop(rt);

                if !Recorder::input_available() {
                    show_status(&st, "No microphone found");
                    return;
                }

                if let Err(e) = rec_c.borrow_mut().start() {
                    eprintln!("Record start error: {e}");
                    show_status(&st, &format!("Err: {e}"));
                    return;
                }
                *state_c.borrow_mut() = State::Recording;
                btn.add_css_class("recording");
                btn.remove_css_class("done");

                show_status(&st, "Recording...");
            }
            State::Recording => {
                *state_c.borrow_mut() = State::Processing;
                btn.remove_css_class("recording");
                btn.add_css_class("processing");

                show_status(&st, "Transcribing...");

                let wav = match rec_c.borrow_mut().stop() {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("Record stop error: {e}");
                        show_status(&st, &format!("Err: {e}"));
                        *state_c.borrow_mut() = State::Idle;
                        btn.remove_css_class("processing");
                        return;
                    }
                };

                let db_inner = Arc::clone(&db_c);
                let sample_rate = rec_c.borrow().sample_rate();

                let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();

                let rt = runtime_c.borrow();
                match rt.active_service {
                    TranscriptionService::Api => {
                        let base_url = rt.api_base_url.clone();
                        let api_key = rt.api_key.clone().unwrap_or_default();
                        let model = rt.api_model.clone();
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new()
                                .expect("failed to create tokio runtime");
                            let result = rt
                                .block_on(crate::api::transcribe(&base_url, &api_key, &model, wav));
                            let _ = tx.send(result);
                        });
                    }
                    TranscriptionService::Local => {
                        let Some(whisper) = rt.local_whisper.clone() else {
                            let _ = tx.send(Err("Local model not loaded".into()));
                            return;
                        };
                        std::thread::spawn(move || {
                            let result = whisper.transcribe(&wav, sample_rate);
                            let _ = tx.send(result);
                        });
                    }
                }
                drop(rt);

                let btn2 = btn.clone();
                let st2 = st.clone();
                let state_c2 = Rc::clone(&state_c);
                let notify = config_c.sound_notification;
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match rx.try_recv() {
                        Ok(Ok(text)) => {
                            if let Ok(db) = db_inner.lock()
                                && let Err(e) = db.insert(&text)
                            {
                                eprintln!("DB insert error: {e}");
                            }
                            match crate::input::copy_to_clipboard(&text) {
                                Ok(_) => {
                                    if notify {
                                        play_notification();
                                    }
                                    btn2.remove_css_class("processing");
                                    btn2.add_css_class("done");

                                    show_status(&st2, "Copied!");
                                    let st3 = st2.clone();
                                    let btn3 = btn2.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(2),
                                        move || {
                                            hide_status(&st3);
                                            btn3.remove_css_class("done");
                                        },
                                    );
                                }
                                Err(e) => {
                                    eprintln!("Clipboard error: {e}");
                                    btn2.remove_css_class("processing");

                                    show_status(&st2, "Error!");
                                    let st3 = st2.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(3),
                                        move || hide_status(&st3),
                                    );
                                }
                            }
                            *state_c2.borrow_mut() = State::Idle;
                            glib::ControlFlow::Break
                        }
                        Ok(Err(e)) => {
                            eprintln!("Transcription error: {e}");
                            btn2.remove_css_class("processing");
                            show_status(&st2, "Error!");
                            let st3 = st2.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(3),
                                move || hide_status(&st3),
                            );
                            *state_c2.borrow_mut() = State::Idle;
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => {
                            *state_c2.borrow_mut() = State::Idle;
                            btn2.remove_css_class("processing");
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
            State::Processing | State::Synthesizing => {}
            State::Speaking => {
                // Stop TTS playback — completion callback will reset to Idle
                dbg_log!("[TTS] stop requested via button click");
                runtime_c
                    .borrow()
                    .tts_stop
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });

    // --- Right-click popover menu (on the button) ---
    let mode_action = gtk4::gio::SimpleAction::new_stateful(
        "transcription-mode",
        Some(&String::static_variant_type()),
        &initial_provider.to_variant(),
    );

    let stt_api_section = gtk4::gio::Menu::new();
    for preset in config::API_PRESETS {
        stt_api_section.append(
            Some(preset.label),
            Some(&format!("app.transcription-mode::{}", preset.id)),
        );
    }
    stt_api_section.append(
        Some("Custom API..."),
        Some("app.transcription-mode::custom"),
    );

    let stt_local_section = gtk4::gio::Menu::new();
    for lm in config::LOCAL_MODEL_PRESETS {
        stt_local_section.append(
            Some(&format!("{} ({})", lm.label, lm.size_label)),
            Some(&format!("app.transcription-mode::{}", lm.id)),
        );
    }

    // TTS sections — local Piper voices and API providers
    let tts_initial = match initial_tts_provider {
        TtsProvider::Piper => runtime.borrow().tts_voice.clone(),
        TtsProvider::Api => format!("api:{}", initial_tts_preset),
        TtsProvider::None => "none".to_string(),
    };
    let tts_mode_action = gtk4::gio::SimpleAction::new_stateful(
        "tts-mode",
        Some(&String::static_variant_type()),
        &tts_initial.to_variant(),
    );
    let read_clipboard_action = gtk4::gio::SimpleAction::new("read-clipboard", None);
    read_clipboard_action.set_enabled(initial_tts_provider != TtsProvider::None);

    let tts_local_section = gtk4::gio::Menu::new();
    for voice in config::PIPER_VOICES {
        tts_local_section.append(
            Some(voice.label),
            Some(&format!("app.tts-mode::{}", voice.id)),
        );
    }

    let tts_api_section = gtk4::gio::Menu::new();
    for preset in config::TTS_API_PRESETS {
        tts_api_section.append(
            Some(preset.label),
            Some(&format!("app.tts-mode::api:{}", preset.id)),
        );
    }

    let tts_manage = gtk4::gio::Menu::new();
    tts_manage.append(Some("Off"), Some("app.tts-mode::none"));
    tts_manage.append(Some("Reset TTS"), Some("app.tts-reset"));
    tts_manage.append(Some("Delete TTS"), Some("app.tts-delete"));

    let actions_section = gtk4::gio::Menu::new();
    actions_section.append(Some("Read Clipboard"), Some("app.read-clipboard"));
    actions_section.append(Some("History"), Some("app.show-history"));
    actions_section.append(Some("Quit"), Some("app.quit"));

    let menu = gtk4::gio::Menu::new();
    menu.append_section(Some("STT — API"), &stt_api_section);
    menu.append_section(Some("STT — Local"), &stt_local_section);
    menu.append_section(Some("TTS — Local (Piper)"), &tts_local_section);
    menu.append_section(Some("TTS — API"), &tts_api_section);
    menu.append_section(None, &tts_manage);
    menu.append_section(None, &actions_section);

    let popover = gtk4::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&button);
    popover.set_has_arrow(true);

    // Right-click on button → show our popover, suppress WM menu
    let pop = popover.clone();
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |g, _, _, _| {
        g.set_state(gtk4::EventSequenceState::Claimed);
        pop.popup();
    });
    button.add_controller(gesture);

    // Action: transcription mode switch (provider-based)
    let runtime_mode = Rc::clone(&runtime);
    let state_mode = Rc::clone(&state);
    let config_mode = Arc::clone(&config);
    let db_mode = Arc::clone(&db);
    let status_mode = status.clone();
    let win_mode = window.clone();
    mode_action.connect_activate(move |action, param| {
        let Some(param) = param else { return };
        let Some(chosen) = param.get::<String>() else {
            return;
        };

        // Guard: block mode switch during recording/processing
        if *state_mode.borrow() != State::Idle {
            return;
        }

        // Guard: block mode switch during download
        if runtime_mode.borrow().downloading {
            return;
        }

        // No-op if already on this provider
        if chosen == runtime_mode.borrow().active_provider {
            return;
        }

        if let Some(local_preset) = config::find_local_model(&chosen) {
            switch_to_local(
                &runtime_mode,
                &config_mode,
                &db_mode,
                action,
                &status_mode,
                local_preset,
            );
        } else if chosen == "custom" {
            show_custom_api_dialog(
                &win_mode,
                &runtime_mode,
                &db_mode,
                action,
                &status_mode,
                &config_mode,
            );
        } else if let Some(preset) = config::find_preset(&chosen) {
            switch_to_preset(
                &win_mode,
                &runtime_mode,
                &config_mode,
                &db_mode,
                action,
                &status_mode,
                preset,
            );
        }
    });
    app.add_action(&mode_action);

    // Action: show history
    let history_action = gtk4::gio::SimpleAction::new("show-history", None);
    let db_hist = Arc::clone(&db);
    let win_ref = window.clone();
    history_action.connect_activate(move |_, _| {
        show_history_dialog(&win_ref, &db_hist);
    });
    app.add_action(&history_action);

    // Action: quit
    let quit_action = gtk4::gio::SimpleAction::new("quit", None);
    quit_action.connect_activate(move |_, _| {
        std::process::exit(0);
    });
    app.add_action(&quit_action);

    // --- Save position on close ---
    let db_close = Arc::clone(&db);
    window.connect_close_request(move |win| {
        save_window_position(win, &db_close);
        glib::Propagation::Proceed
    });

    // --- Position: saved or bottom-right ---
    let db_pos = Arc::clone(&db);
    window.connect_realize(move |win| {
        if let Some(surface) = win.surface()
            && let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>()
        {
            toplevel.set_decorated(false);
        }
        let w = win.clone();
        let db_p = Arc::clone(&db_pos);
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            position_window(&w, &db_p);
        });
    });

    // --- Esc key: stop recording ---
    let esc_btn = button.clone();
    let esc_state = Rc::clone(&state);
    let esc_shortcut = gtk4::Shortcut::new(
        gtk4::ShortcutTrigger::parse_string("Escape"),
        Some(gtk4::CallbackAction::new(move |_, _| {
            if *esc_state.borrow() == State::Recording {
                esc_btn.emit_clicked();
            }
            glib::Propagation::Stop
        })),
    );
    let esc_controller = gtk4::ShortcutController::new();
    esc_controller.set_scope(gtk4::ShortcutScope::Global);
    esc_controller.add_shortcut(esc_shortcut);
    window.add_controller(esc_controller);

    // --- D-Bus action: "record" — triggered by GNOME shortcut ---
    let record_action = gtk4::gio::SimpleAction::new("record", None);
    let btn_rec = button.clone();
    let state_rec = Rc::clone(&state);
    let win_rec = window.clone();
    record_action.connect_activate(move |_, _| {
        eprintln!("[dbus] 'record' action activated");
        win_rec.present();
        // GNOME Wayland: force-activate via Shell D-Bus (falls back silently on other DEs)
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("gdbus")
                .args([
                    "call", "--session",
                    "--dest=org.gnome.Shell",
                    "--object-path=/org/gnome/Shell",
                    "--method=org.gnome.Shell.Eval",
                    r#"global.get_window_actors().find(a=>a.meta_window.title==='WhisperCrabs')?.meta_window.activate(0)"#,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        if *state_rec.borrow() == State::Idle {
            btn_rec.emit_clicked();
        }
    });
    app.add_action(&record_action);

    // --- D-Bus action: "stop" — triggered by GNOME shortcut ---
    let stop_action = gtk4::gio::SimpleAction::new("stop", None);
    let btn_stop = button.clone();
    let state_stop = Rc::clone(&state);
    stop_action.connect_activate(move |_, _| {
        eprintln!("[dbus] 'stop' action activated");
        if *state_stop.borrow() == State::Recording {
            btn_stop.emit_clicked();
        }
    });
    app.add_action(&stop_action);

    // --- D-Bus action: "set-api-config" — programmatic custom API setup ---
    let api_config_action =
        gtk4::gio::SimpleAction::new("set-api-config", Some(&String::static_variant_type()));
    let runtime_api_cfg = Rc::clone(&runtime);
    let db_api_cfg = Arc::clone(&db);
    let config_api_cfg = Arc::clone(&config);
    let mode_action_ref = mode_action.clone();
    api_config_action.connect_activate(move |_, param| {
        let Some(param) = param else { return };
        let Some(json_str) = param.get::<String>() else {
            eprintln!("set-api-config: expected string parameter");
            return;
        };

        // Cap incoming JSON size to prevent abuse
        if json_str.len() > 4096 {
            eprintln!("set-api-config: JSON too large");
            return;
        }

        eprintln!("[dbus] 'set-api-config' action activated");

        #[derive(serde::Deserialize)]
        struct ApiConfigInput {
            base_url: String,
            model: String,
            api_key: Option<String>,
        }

        let input: ApiConfigInput = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("set-api-config: invalid JSON: {e}");
                return;
            }
        };

        // Validate URL scheme
        if !input.base_url.starts_with("http://") && !input.base_url.starts_with("https://") {
            eprintln!("set-api-config: base_url must use http:// or https://");
            return;
        }

        let base_url = input.base_url;
        let model = input.model;
        let api_key = input.api_key;

        // Persist to DB
        if let Ok(d) = db_api_cfg.lock() {
            let _ = d.set_setting("api_custom_url", &base_url);
            if let Some(ref k) = api_key {
                let _ = d.set_setting("api_custom_key", k);
            }
            let _ = d.set_setting("api_custom_model", &model);
            let _ = d.set_setting("transcription_mode", "custom");
        }

        // Update RuntimeState
        {
            let mut rt = runtime_api_cfg.borrow_mut();
            rt.active_service = TranscriptionService::Api;
            rt.active_provider = "custom".to_string();
            rt.api_base_url = base_url;
            rt.api_key = api_key;
            rt.api_model = model;
            rt.local_whisper = None;
        }

        // Delete model file to free disk space
        delete_all_local_models(&config_api_cfg.models_dir);

        mode_action_ref.set_state(&"custom".to_variant());
    });
    app.add_action(&api_config_action);

    // --- TTS mode action (voice selection) ---
    let runtime_tts = Rc::clone(&runtime);
    let db_tts = Arc::clone(&db);
    let config_tts = Arc::clone(&config);
    let status_tts = status.clone();
    let read_cb_action_ref = read_clipboard_action.clone();
    let window_tts = window.clone();
    tts_mode_action.connect_activate(move |action, param| {
        let Some(param) = param else {
            dbg_log!("[TTS] tts-mode activated with no param");
            return;
        };
        let Some(chosen) = param.get::<String>() else {
            dbg_log!("[TTS] tts-mode param not a string");
            return;
        };
        dbg_log!("[TTS] tts-mode chosen: {chosen}");

        if runtime_tts.borrow().tts_downloading {
            dbg_log!("[TTS] already downloading, ignoring");
            return;
        }

        if chosen == "none" {
            {
                let mut rt = runtime_tts.borrow_mut();
                rt.tts_provider = TtsProvider::None;
                rt.tts_engine = None;
            }
            if let Ok(d) = db_tts.lock() {
                let _ = d.set_setting("tts_provider", "none");
            }
            read_cb_action_ref.set_enabled(false);
            action.set_state(&"none".to_variant());
            return;
        }

        // API provider selection, e.g. "api:openai" or "api:custom"
        if let Some(preset_id) = chosen.strip_prefix("api:") {
            let Some(preset) = config::find_tts_preset(preset_id) else {
                dbg_log!("[TTS] unknown API preset: {preset_id}");
                return;
            };
            let ui = TtsUi {
                action: action.clone(),
                read_action: read_cb_action_ref.clone(),
                status: status_tts.clone(),
            };
            if preset_id == "custom" {
                show_tts_custom_dialog(&window_tts, &runtime_tts, &db_tts, ui);
                return;
            }
            switch_to_tts_preset(&window_tts, &runtime_tts, &config_tts, &db_tts, ui, preset);
            return;
        }

        // Any other value is a Piper voice ID
        let Some(voice) = config::find_piper_voice(&chosen) else {
            dbg_log!("[TTS] unknown voice: {chosen}");
            return;
        };

        let piper_dir = config_tts.models_dir.join("piper");
        let has_venv = config::piper_venv_exists(&piper_dir);
        let has_voice = config::piper_voice_exists(&piper_dir, voice.id);

        if has_venv && has_voice {
            // Delete old voice files before loading new one
            let old_voice = runtime_tts.borrow().tts_voice.clone();
            if old_voice != voice.id {
                cleanup_old_voice(&piper_dir, &old_voice);
            }
            // Already downloaded — just load
            dbg_log!("[TTS] loading voice {}...", voice.id);
            show_status(&status_tts, "Loading TTS...");
            match PiperTts::new(&piper_dir, voice.id) {
                Ok(engine) => {
                    let mut rt = runtime_tts.borrow_mut();
                    rt.tts_provider = TtsProvider::Piper;
                    rt.tts_voice = voice.id.to_string();
                    rt.tts_engine = Some(Arc::new(TtsEngine::Piper(engine)));
                }
                Err(e) => {
                    eprintln!("Failed to load Piper: {e}");
                    show_status(&status_tts, "TTS load failed");
                }
            }
            hide_status(&status_tts);
            if let Ok(d) = db_tts.lock() {
                let _ = d.set_setting("tts_provider", "piper");
                let _ = d.set_setting("tts_voice", voice.id);
            }
            read_cb_action_ref.set_enabled(true);
            action.set_state(&chosen.to_variant());
        } else {
            // Need to download (venv and/or voice model)
            dbg_log!("[TTS] downloading voice {}...", voice.id);
            download_tts_models(
                &runtime_tts,
                &config_tts.models_dir,
                &db_tts,
                action,
                &read_cb_action_ref,
                &status_tts,
                &window_tts,
                voice,
                has_venv,
            );
        }
    });
    app.add_action(&tts_mode_action);

    // --- TTS Reset action ---
    let runtime_reset = Rc::clone(&runtime);
    let config_reset = Arc::clone(&config);
    let db_reset = Arc::clone(&db);
    let status_reset = status.clone();
    let tts_action_reset = tts_mode_action.clone();
    let read_cb_reset = read_clipboard_action.clone();
    let reset_action = gtk4::gio::SimpleAction::new("tts-reset", None);
    reset_action.connect_activate(move |_, _| {
        dbg_log!("[TTS] reset requested");
        let piper_dir = config_reset.models_dir.join("piper");
        if piper_dir.exists() {
            let _ = std::fs::remove_dir_all(&piper_dir);
            dbg_log!("[TTS] deleted {}", piper_dir.display());
        }
        {
            let mut rt = runtime_reset.borrow_mut();
            rt.tts_provider = TtsProvider::None;
            rt.tts_engine = None;
        }
        if let Ok(d) = db_reset.lock() {
            let _ = d.set_setting("tts_provider", "none");
        }
        read_cb_reset.set_enabled(false);
        tts_action_reset.set_state(&"none".to_variant());
        show_status(&status_reset, "TTS reset");
        let st = status_reset.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || hide_status(&st));
    });
    app.add_action(&reset_action);

    // --- TTS Delete action ---
    let runtime_del = Rc::clone(&runtime);
    let config_del = Arc::clone(&config);
    let db_del = Arc::clone(&db);
    let status_del = status.clone();
    let tts_action_del = tts_mode_action.clone();
    let read_cb_del = read_clipboard_action.clone();
    let delete_action = gtk4::gio::SimpleAction::new("tts-delete", None);
    delete_action.connect_activate(move |_, _| {
        dbg_log!("[TTS] delete requested");
        let piper_dir = config_del.models_dir.join("piper");
        if piper_dir.exists() {
            let _ = std::fs::remove_dir_all(&piper_dir);
            dbg_log!("[TTS] deleted {}", piper_dir.display());
        }
        {
            let mut rt = runtime_del.borrow_mut();
            rt.tts_provider = TtsProvider::None;
            rt.tts_engine = None;
            rt.tts_api_key = None;
        }
        if let Ok(d) = db_del.lock() {
            let _ = d.set_setting("tts_provider", "none");
            // Drop the stored API credentials too: "Delete TTS" means all
            // TTS data, not just the local voice files.
            let _ = d.set_setting("tts_api_key", "");
            let _ = d.set_setting("tts_api_key_openai", "");
            let _ = d.set_setting("tts_api_key_openrouter", "");
        }
        read_cb_del.set_enabled(false);
        tts_action_del.set_state(&"none".to_variant());
        show_status(&status_del, "TTS deleted");
        let st = status_del.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || hide_status(&st));
    });
    app.add_action(&delete_action);

    // --- Read Clipboard action (TTS playback) ---
    let runtime_rc = Rc::clone(&runtime);
    let state_tts_rc = Rc::clone(&state);
    let btn_tts = button.clone();
    read_clipboard_action.connect_activate(move |_, _| {
        dbg_log!("[TTS] read-clipboard activated");
        let rt = runtime_rc.borrow();
        if rt.tts_provider == TtsProvider::None {
            dbg_log!("[TTS] provider is None, skipping");
            return;
        }
        let Some(ref engine) = rt.tts_engine else {
            dbg_log!("[TTS] no engine loaded");
            return;
        };

        let text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(t) if !t.trim().is_empty() => {
                dbg_log!("[TTS] clipboard text len={}", t.len());
                t
            }
            Ok(_) => {
                dbg_log!("[TTS] clipboard empty or whitespace");
                return;
            }
            Err(e) => {
                dbg_log!("[TTS] clipboard error: {e}");
                return;
            }
        };

        let engine = Arc::clone(engine);
        let preview: String = text.chars().take(80).collect();
        dbg_log!("[TTS] synthesizing: {:?}", preview);

        // Yellow button while synthesizing
        *state_tts_rc.borrow_mut() = State::Synthesizing;
        btn_tts.add_css_class("synthesizing");

        let stop_flag = Arc::clone(&rt.tts_stop);
        stop_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        drop(rt);

        let (tx, rx) = std::sync::mpsc::channel::<Result<Speech, String>>();

        std::thread::spawn(move || {
            let result = engine.synthesize(&text);
            let _ = tx.send(result);
        });

        let btn2 = btn_tts.clone();
        let state2 = Rc::clone(&state_tts_rc);
        let stop2 = stop_flag;

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(speech)) => {
                    dbg_log!(
                        "[TTS] got {} samples, playing at {}Hz...",
                        speech.samples.len(),
                        speech.sample_rate
                    );
                    btn2.remove_css_class("synthesizing");

                    // Green button while speaking
                    *state2.borrow_mut() = State::Speaking;
                    btn2.add_css_class("speaking");

                    let btn3 = btn2.clone();
                    let state3 = Rc::clone(&state2);
                    let stop3 = Arc::clone(&stop2);

                    play_tts_audio(speech.samples, speech.sample_rate, stop3, move || {
                        btn3.remove_css_class("speaking");
                        *state3.borrow_mut() = State::Idle;
                    });

                    glib::ControlFlow::Break
                }
                Ok(Err(e)) => {
                    dbg_log!("[TTS] synthesis error: {e}");
                    btn2.remove_css_class("synthesizing");
                    *state2.borrow_mut() = State::Idle;
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    btn2.remove_css_class("synthesizing");
                    *state2.borrow_mut() = State::Idle;
                    glib::ControlFlow::Break
                }
            }
        });
    });
    app.add_action(&read_clipboard_action);

    // --- D-Bus action: "speak" — reads clipboard and speaks ---
    let speak_action = gtk4::gio::SimpleAction::new("speak", None);
    let runtime_speak = Rc::clone(&runtime);
    let state_speak = Rc::clone(&state);
    let btn_speak = button.clone();
    speak_action.connect_activate(move |_, _| {
        eprintln!("[dbus] 'speak' action activated");
        let rt = runtime_speak.borrow();
        if rt.tts_provider == TtsProvider::None {
            return;
        }
        let Some(ref engine) = rt.tts_engine else {
            return;
        };
        let text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => return,
        };
        let engine = Arc::clone(engine);
        let stop_flag = Arc::clone(&rt.tts_stop);
        stop_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        drop(rt);

        *state_speak.borrow_mut() = State::Synthesizing;
        btn_speak.add_css_class("synthesizing");

        let (tx, rx) = std::sync::mpsc::channel::<Result<Speech, String>>();
        std::thread::spawn(move || {
            let _ = tx.send(engine.synthesize(&text));
        });

        let btn2 = btn_speak.clone();
        let state2 = Rc::clone(&state_speak);
        let stop2 = stop_flag;
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            match rx.try_recv() {
                Ok(Ok(speech)) => {
                    btn2.remove_css_class("synthesizing");
                    *state2.borrow_mut() = State::Speaking;
                    btn2.add_css_class("speaking");
                    let btn3 = btn2.clone();
                    let state3 = Rc::clone(&state2);
                    play_tts_audio(
                        speech.samples,
                        speech.sample_rate,
                        Arc::clone(&stop2),
                        move || {
                            btn3.remove_css_class("speaking");
                            *state3.borrow_mut() = State::Idle;
                        },
                    );
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) => {
                    btn2.remove_css_class("synthesizing");
                    *state2.borrow_mut() = State::Idle;
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => {
                    btn2.remove_css_class("synthesizing");
                    *state2.borrow_mut() = State::Idle;
                    glib::ControlFlow::Break
                }
            }
        });
    });
    app.add_action(&speak_action);

    window.present();
}

/// Widgets the TTS provider switch needs to update.
#[derive(Clone)]
struct TtsUi {
    action: gtk4::gio::SimpleAction,
    read_action: gtk4::gio::SimpleAction,
    status: gtk4::Label,
}

/// Persisted TTS settings read from the database.
struct TtsSettings {
    provider: Option<String>,
    voice: Option<String>,
    preset: Option<String>,
    url: Option<String>,
    key: Option<String>,
    model: Option<String>,
}

/// Resolved TTS state for startup, including a loaded engine when possible.
struct ResolvedTts {
    provider: TtsProvider,
    voice: String,
    preset: String,
    url: String,
    key: Option<String>,
    model: String,
    engine: Option<Arc<TtsEngine>>,
}

/// Resolve the TTS provider for startup from the database, falling back to
/// the environment defaults. Piper stays the default when a local install is
/// present; API TTS is used when it was selected or configured.
fn resolve_tts_state(
    settings: Option<TtsSettings>,
    config: &Config,
    piper_dir: &std::path::Path,
    default_piper_voice: &str,
) -> ResolvedTts {
    let settings = settings.unwrap_or(TtsSettings {
        provider: None,
        voice: None,
        preset: None,
        url: None,
        key: None,
        model: None,
    });

    let voice = settings.voice.unwrap_or_else(|| {
        if config.tts_voice.trim().is_empty() {
            default_piper_voice.to_string()
        } else {
            config.tts_voice.clone()
        }
    });
    let provider_setting = settings
        .provider
        .unwrap_or_else(|| config.tts_provider.clone());

    let mut resolved = ResolvedTts {
        provider: TtsProvider::None,
        voice,
        preset: settings
            .preset
            .unwrap_or_else(|| config.tts_api_preset.clone()),
        url: settings
            .url
            .unwrap_or_else(|| config.tts_api_base_url.clone()),
        key: settings.key.or_else(|| config.tts_api_key.clone()),
        model: settings
            .model
            .unwrap_or_else(|| config.tts_api_model.clone()),
        engine: None,
    };

    match provider_setting.as_str() {
        "api" => {
            match ApiTts::new(
                &resolved.url,
                resolved.key.clone(),
                &resolved.model,
                &resolved.voice,
            ) {
                Ok(engine) => {
                    resolved.provider = TtsProvider::Api;
                    resolved.engine = Some(Arc::new(TtsEngine::Api(engine)));
                }
                Err(e) => eprintln!("Failed to load API TTS: {e}"),
            }
        }
        "piper" | "local"
            if config::piper_venv_exists(piper_dir)
                && config::piper_voice_exists(piper_dir, &resolved.voice) =>
        {
            match PiperTts::new(piper_dir, &resolved.voice) {
                Ok(engine) => {
                    resolved.provider = TtsProvider::Piper;
                    resolved.engine = Some(Arc::new(TtsEngine::Piper(engine)));
                }
                Err(e) => eprintln!("Failed to load Piper TTS: {e}"),
            }
        }
        _ => {}
    }

    resolved
}

/// Activate a built-in API TTS preset, prompting for an API key when needed.
fn switch_to_tts_preset(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    config: &Arc<Config>,
    db: &Arc<Mutex<Db>>,
    ui: TtsUi,
    preset: &config::TtsApiPreset,
) {
    let resolved_key = db
        .lock()
        .ok()
        .and_then(|d| {
            d.get_setting(&format!("tts_api_key_{}", preset.id))
                .ok()
                .flatten()
        })
        .or_else(|| {
            db.lock()
                .ok()
                .and_then(|d| d.get_setting("tts_api_key").ok().flatten())
        })
        .or_else(|| config.tts_api_key.clone())
        .filter(|k| !k.trim().is_empty());

    if preset.needs_key && resolved_key.is_none() {
        show_tts_api_key_dialog(parent, runtime, db, ui, preset);
        return;
    }

    apply_tts_preset(
        runtime,
        db,
        &ui,
        preset,
        preset.base_url.to_string(),
        preset.default_model.to_string(),
        preset.default_voice.to_string(),
        resolved_key,
    );
}

/// Build the API engine and make it the active TTS provider.
#[allow(clippy::too_many_arguments)]
fn apply_tts_preset(
    runtime: &Rc<RefCell<RuntimeState>>,
    db: &Arc<Mutex<Db>>,
    ui: &TtsUi,
    preset: &config::TtsApiPreset,
    base_url: String,
    model: String,
    voice: String,
    api_key: Option<String>,
) {
    let engine = match ApiTts::new(&base_url, api_key.clone(), &model, &voice) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("TTS preset {} failed: {e}", preset.id);
            show_status(&ui.status, "TTS load failed");
            ui.action.set_state(&"none".to_variant());
            return;
        }
    };

    {
        let mut rt = runtime.borrow_mut();
        rt.tts_provider = TtsProvider::Api;
        rt.tts_preset = preset.id.to_string();
        rt.tts_api_url = base_url.clone();
        rt.tts_api_key = api_key.clone();
        rt.tts_api_model = model.clone();
        rt.tts_voice = voice.clone();
        rt.tts_engine = Some(Arc::new(TtsEngine::Api(engine)));
    }

    if let Ok(d) = db.lock() {
        let _ = d.set_setting("tts_provider", "api");
        let _ = d.set_setting("tts_api_preset", preset.id);
        let _ = d.set_setting("tts_api_url", &base_url);
        let _ = d.set_setting("tts_api_model", &model);
        let _ = d.set_setting("tts_voice", &voice);
        if let Some(key) = &api_key {
            let _ = d.set_setting("tts_api_key", key);
        }
    }

    ui.read_action.set_enabled(true);
    ui.action
        .set_state(&format!("api:{}", preset.id).to_variant());
    show_status(&ui.status, &format!("TTS: {}", preset.label));
    let st = ui.status.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || hide_status(&st));
}

fn show_tts_api_key_dialog(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    db: &Arc<Mutex<Db>>,
    ui: TtsUi,
    preset: &config::TtsApiPreset,
) {
    let previous = tts_state_id(runtime);

    let dialog = gtk4::Window::builder()
        .title(format!("{} TTS API Key", preset.label))
        .default_width(380)
        .default_height(140)
        .transient_for(parent)
        .modal(true)
        .build();

    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let label = gtk4::Label::new(Some(&format!("Enter your {} API key:", preset.label)));
    label.set_halign(gtk4::Align::Start);
    grid.attach(&label, 0, 0, 2, 1);

    let key_entry = gtk4::Entry::new();
    key_entry.set_hexpand(true);
    key_entry.set_placeholder_text(Some("API key"));
    key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_entry.set_visibility(false);
    grid.attach(&key_entry, 0, 1, 2, 1);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    grid.attach(&btn_box, 0, 2, 2, 1);

    dialog.set_child(Some(&grid));

    let action_cancel = ui.action.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        action_cancel.set_state(&previous.to_variant());
        dialog_cancel.close();
    });

    let runtime_save = Rc::clone(runtime);
    let db_save = Arc::clone(db);
    let ui_save = ui.clone();
    let dialog_save = dialog.clone();
    let preset_id = preset.id;
    let preset_label = preset.label;
    let preset_base_url = preset.base_url;
    let preset_model = preset.default_model;
    let preset_voice = preset.default_voice;
    save_btn.connect_clicked(move |_| {
        let key_text = key_entry.text().to_string();
        if key_text.trim().is_empty() {
            return;
        }

        let static_preset = config::TtsApiPreset {
            id: preset_id,
            label: preset_label,
            base_url: preset_base_url,
            default_model: preset_model,
            default_voice: preset_voice,
            needs_key: true,
        };

        apply_tts_preset(
            &runtime_save,
            &db_save,
            &ui_save,
            &static_preset,
            preset_base_url.to_string(),
            preset_model.to_string(),
            preset_voice.to_string(),
            Some(key_text),
        );

        dialog_save.close();
    });

    dialog.present();
}

/// The current TTS selection as a `tts-mode` action value.
fn tts_state_id(runtime: &Rc<RefCell<RuntimeState>>) -> String {
    let rt = runtime.borrow();
    match rt.tts_provider {
        TtsProvider::None => "none".to_string(),
        TtsProvider::Piper => rt.tts_voice.clone(),
        TtsProvider::Api => format!("api:{}", rt.tts_preset),
    }
}

/// Dialog for any OpenAI-compatible TTS endpoint (LocalAI, LM Studio, custom).
fn show_tts_custom_dialog(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    db: &Arc<Mutex<Db>>,
    ui: TtsUi,
) {
    let previous = tts_state_id(runtime);

    let dialog = gtk4::Window::builder()
        .title("Custom TTS API")
        .default_width(420)
        .default_height(260)
        .transient_for(parent)
        .modal(true)
        .build();

    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let url_label = gtk4::Label::new(Some("Base URL"));
    url_label.set_halign(gtk4::Align::End);
    let url_entry = gtk4::Entry::new();
    url_entry.set_hexpand(true);
    url_entry.set_placeholder_text(Some("https://api.example.com/v1"));
    grid.attach(&url_label, 0, 0, 1, 1);
    grid.attach(&url_entry, 1, 0, 2, 1);

    let key_label = gtk4::Label::new(Some("API Key"));
    key_label.set_halign(gtk4::Align::End);
    let key_entry = gtk4::Entry::new();
    key_entry.set_hexpand(true);
    key_entry.set_placeholder_text(Some("(optional)"));
    key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_entry.set_visibility(false);
    grid.attach(&key_label, 0, 1, 1, 1);
    grid.attach(&key_entry, 1, 1, 2, 1);

    let model_label = gtk4::Label::new(Some("Model"));
    model_label.set_halign(gtk4::Align::End);
    let model_entry = gtk4::Entry::new();
    model_entry.set_hexpand(true);
    model_entry.set_placeholder_text(Some("gpt-4o-mini-tts"));
    grid.attach(&model_label, 0, 2, 1, 1);
    grid.attach(&model_entry, 1, 2, 2, 1);

    let voice_label = gtk4::Label::new(Some("Voice"));
    voice_label.set_halign(gtk4::Align::End);
    let voice_entry = gtk4::Entry::new();
    voice_entry.set_hexpand(true);
    voice_entry.set_placeholder_text(Some("alloy"));
    grid.attach(&voice_label, 0, 3, 1, 1);
    grid.attach(&voice_entry, 1, 3, 2, 1);

    // Pre-populate from the active runtime state
    {
        let rt = runtime.borrow();
        url_entry.set_text(&rt.tts_api_url);
        if let Some(key) = &rt.tts_api_key {
            key_entry.set_text(key);
        }
        model_entry.set_text(&rt.tts_api_model);
        voice_entry.set_text(&rt.tts_voice);
    }

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    grid.attach(&btn_box, 0, 4, 3, 1);

    dialog.set_child(Some(&grid));

    let action_cancel = ui.action.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        action_cancel.set_state(&previous.to_variant());
        dialog_cancel.close();
    });

    let runtime_save = Rc::clone(runtime);
    let db_save = Arc::clone(db);
    let ui_save = ui.clone();
    let dialog_save = dialog.clone();
    save_btn.connect_clicked(move |_| {
        let url = url_entry.text().trim().to_string();
        let model = model_entry.text().trim().to_string();
        let voice = voice_entry.text().trim().to_string();
        let key_text = key_entry.text().trim().to_string();

        if url.is_empty() || model.is_empty() {
            return;
        }

        let api_key = if key_text.is_empty() {
            None
        } else {
            Some(key_text)
        };

        let custom = match config::find_tts_preset("custom") {
            Some(preset) => preset,
            None => return,
        };

        apply_tts_preset(
            &runtime_save,
            &db_save,
            &ui_save,
            custom,
            url,
            model,
            voice,
            api_key,
        );

        dialog_save.close();
    });

    dialog.present();
}

fn delete_all_local_models(models_dir: &std::path::Path) {
    for lm in config::LOCAL_MODEL_PRESETS {
        let path = models_dir.join(lm.file_name);
        if path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            eprintln!("Failed to delete model file {}: {e}", lm.file_name);
        }
    }
}

fn switch_to_preset(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    config: &Arc<Config>,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    preset: &config::ApiPreset,
) {
    // Resolve API key: DB per-provider key → env var fallback
    let resolved_key = if preset.needs_key {
        db.lock()
            .ok()
            .and_then(|d| {
                d.get_setting(&format!("api_key_{}", preset.id))
                    .ok()
                    .flatten()
            })
            .or_else(|| config.api_key.clone())
    } else {
        None
    };

    // If provider needs a key and we don't have one, show a dialog to collect it
    if preset.needs_key && resolved_key.is_none() {
        show_api_key_dialog(parent, runtime, config, db, action, status, preset);
        return;
    }

    apply_preset(runtime, config, db, action, status, preset, resolved_key);
}

fn apply_preset(
    runtime: &Rc<RefCell<RuntimeState>>,
    config: &Arc<Config>,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    preset: &config::ApiPreset,
    api_key: Option<String>,
) {
    {
        let mut rt = runtime.borrow_mut();
        rt.active_service = TranscriptionService::Api;
        rt.active_provider = preset.id.to_string();
        rt.api_base_url = preset.base_url.to_string();
        rt.api_model = preset.default_model.to_string();
        rt.api_key = api_key;
        rt.local_whisper = None;
    }

    // Delete all local model files to free disk space
    delete_all_local_models(&config.models_dir);

    // Persist to DB
    if let Ok(d) = db.lock() {
        let _ = d.set_setting("transcription_mode", preset.id);
    }

    action.set_state(&preset.id.to_variant());

    show_status(status, &format!("{} mode", preset.label));
    let st = status.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
        hide_status(&st);
    });
}

fn show_api_key_dialog(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    config: &Arc<Config>,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    preset: &config::ApiPreset,
) {
    let previous_provider = runtime.borrow().active_provider.clone();

    let dialog = gtk4::Window::builder()
        .title(format!("{} API Key", preset.label))
        .default_width(380)
        .default_height(140)
        .transient_for(parent)
        .modal(true)
        .build();

    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let label = gtk4::Label::new(Some(&format!("Enter your {} API key:", preset.label)));
    label.set_halign(gtk4::Align::Start);
    grid.attach(&label, 0, 0, 2, 1);

    let key_entry = gtk4::Entry::new();
    key_entry.set_hexpand(true);
    key_entry.set_placeholder_text(Some("API key"));
    key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_entry.set_visibility(false);
    grid.attach(&key_entry, 0, 1, 2, 1);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    grid.attach(&btn_box, 0, 2, 2, 1);

    dialog.set_child(Some(&grid));

    // Cancel → revert radio to previous provider
    let action_cancel = action.clone();
    let prev = previous_provider.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        action_cancel.set_state(&prev.to_variant());
        dialog_cancel.close();
    });

    // Save → persist key to DB, then switch
    let runtime_save = Rc::clone(runtime);
    let config_save = Arc::clone(config);
    let db_save = Arc::clone(db);
    let action_save = action.clone();
    let status_save = status.clone();
    let dialog_save = dialog.clone();
    let preset_id = preset.id;
    let preset_label = preset.label;
    let preset_base_url = preset.base_url;
    let preset_default_model = preset.default_model;
    save_btn.connect_clicked(move |_| {
        let key_text = key_entry.text().to_string();
        if key_text.is_empty() {
            return;
        }

        // Persist key to DB
        if let Ok(d) = db_save.lock() {
            let _ = d.set_setting(&format!("api_key_{}", preset_id), &key_text);
        }

        let static_preset = config::ApiPreset {
            id: preset_id,
            label: preset_label,
            base_url: preset_base_url,
            default_model: preset_default_model,
            needs_key: true,
        };

        apply_preset(
            &runtime_save,
            &config_save,
            &db_save,
            &action_save,
            &status_save,
            &static_preset,
            Some(key_text),
        );

        dialog_save.close();
    });

    dialog.present();
}

fn show_custom_api_dialog(
    parent: &gtk4::ApplicationWindow,
    runtime: &Rc<RefCell<RuntimeState>>,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    config: &Arc<Config>,
) {
    let previous_provider = runtime.borrow().active_provider.clone();

    let dialog = gtk4::Window::builder()
        .title("Custom API Configuration")
        .default_width(400)
        .default_height(220)
        .transient_for(parent)
        .modal(true)
        .build();

    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    // Base URL
    let url_label = gtk4::Label::new(Some("Base URL"));
    url_label.set_halign(gtk4::Align::End);
    let url_entry = gtk4::Entry::new();
    url_entry.set_hexpand(true);
    url_entry.set_placeholder_text(Some("https://api.example.com/v1"));
    grid.attach(&url_label, 0, 0, 1, 1);
    grid.attach(&url_entry, 1, 0, 2, 1);

    // API Key
    let key_label = gtk4::Label::new(Some("API Key"));
    key_label.set_halign(gtk4::Align::End);
    let key_entry = gtk4::Entry::new();
    key_entry.set_hexpand(true);
    key_entry.set_placeholder_text(Some("(optional)"));
    key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    key_entry.set_visibility(false);
    grid.attach(&key_label, 0, 1, 1, 1);
    grid.attach(&key_entry, 1, 1, 2, 1);

    // Model
    let model_label = gtk4::Label::new(Some("Model"));
    model_label.set_halign(gtk4::Align::End);
    let model_entry = gtk4::Entry::new();
    model_entry.set_hexpand(true);
    model_entry.set_placeholder_text(Some("whisper-1"));
    grid.attach(&model_label, 0, 2, 1, 1);
    grid.attach(&model_entry, 1, 2, 2, 1);

    // Pre-populate from DB
    if let Ok(d) = db.lock() {
        if let Ok(Some(url)) = d.get_setting("api_custom_url") {
            url_entry.set_text(&url);
        }
        if let Ok(Some(key)) = d.get_setting("api_custom_key") {
            key_entry.set_text(&key);
        }
        if let Ok(Some(model)) = d.get_setting("api_custom_model") {
            model_entry.set_text(&model);
        }
    }

    // Buttons
    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let cancel_btn = gtk4::Button::with_label("Cancel");
    let save_btn = gtk4::Button::with_label("Save");
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    grid.attach(&btn_box, 0, 3, 3, 1);

    dialog.set_child(Some(&grid));

    // Cancel → revert radio to previous provider
    let action_cancel = action.clone();
    let prev = previous_provider.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        action_cancel.set_state(&prev.to_variant());
        dialog_cancel.close();
    });

    // Save → persist + switch
    let runtime_save = Rc::clone(runtime);
    let db_save = Arc::clone(db);
    let config_save = Arc::clone(config);
    let action_save = action.clone();
    let status_save = status.clone();
    let dialog_save = dialog.clone();
    save_btn.connect_clicked(move |_| {
        let url = url_entry.text().to_string();
        let key_text = key_entry.text().to_string();
        let model = model_entry.text().to_string();

        if url.is_empty() || model.is_empty() {
            return; // require at least URL and model
        }

        let api_key = if key_text.is_empty() {
            None
        } else {
            Some(key_text.clone())
        };

        // Persist to DB
        if let Ok(d) = db_save.lock() {
            let _ = d.set_setting("api_custom_url", &url);
            if let Some(ref k) = api_key {
                let _ = d.set_setting("api_custom_key", k);
            }
            let _ = d.set_setting("api_custom_model", &model);
            let _ = d.set_setting("transcription_mode", "custom");
        }

        // Update RuntimeState
        {
            let mut rt = runtime_save.borrow_mut();
            rt.active_service = TranscriptionService::Api;
            rt.active_provider = "custom".to_string();
            rt.api_base_url = url;
            rt.api_key = api_key;
            rt.api_model = model;
            rt.local_whisper = None;
        }

        // Delete model file to free disk space
        delete_all_local_models(&config_save.models_dir);

        action_save.set_state(&"custom".to_variant());

        show_status(&status_save, "Custom API mode");
        let st = status_save.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            hide_status(&st);
        });

        dialog_save.close();
    });

    dialog.present();
}

fn switch_to_local(
    runtime: &Rc<RefCell<RuntimeState>>,
    config: &Arc<Config>,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    local_preset: &config::LocalModelPreset,
) {
    // Delete any previously loaded model files from other presets
    {
        let rt = runtime.borrow();
        if rt.active_service == TranscriptionService::Local
            && let Some(old_model) = config::find_local_model(&rt.active_provider)
            && old_model.id != local_preset.id
        {
            let old_path = config.models_dir.join(old_model.file_name);
            if old_path.exists()
                && let Err(e) = std::fs::remove_file(&old_path)
            {
                eprintln!("Failed to delete old model file: {e}");
            }
        }
    }

    // Set active service immediately so the menu reflects the choice
    {
        let mut rt = runtime.borrow_mut();
        rt.active_service = TranscriptionService::Local;
        rt.active_provider = local_preset.id.to_string();
        rt.local_whisper = None;
    }
    action.set_state(&local_preset.id.to_variant());

    // Persist to DB
    if let Ok(d) = db.lock() {
        let _ = d.set_setting("transcription_mode", local_preset.id);
    }

    let model_path = config.models_dir.join(local_preset.file_name);
    if model_path.exists() {
        load_whisper_model(runtime, &model_path, action, status);
    } else {
        let url = config::model_url(local_preset.file_name);
        download_and_load_model(runtime, &model_path, &url, action, status);
    }
}

fn load_whisper_model(
    runtime: &Rc<RefCell<RuntimeState>>,
    model_path: &std::path::Path,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
) {
    show_status(status, "Loading model...");

    let model_path = model_path.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel::<Result<Arc<LocalWhisper>, String>>();

    std::thread::spawn(move || {
        let result = LocalWhisper::new(&model_path).map(Arc::new);
        let _ = tx.send(result);
    });

    let runtime_c = Rc::clone(runtime);
    let action_c = action.clone();
    let st = status.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        match rx.try_recv() {
            Ok(Ok(whisper)) => {
                runtime_c.borrow_mut().local_whisper = Some(whisper);
                show_status(&st, "Local mode ready");
                let st2 = st.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                    hide_status(&st2);
                });
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                eprintln!("Failed to load whisper model: {e}");
                // Revert to default API provider
                {
                    let mut rt = runtime_c.borrow_mut();
                    rt.active_service = TranscriptionService::Api;
                    rt.active_provider = "groq".to_string();
                    rt.api_base_url = config::API_PRESETS[0].base_url.to_string();
                    rt.api_model = config::API_PRESETS[0].default_model.to_string();
                }
                action_c.set_state(&"groq".to_variant());
                show_status(&st, "Model load failed");
                let st2 = st.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                    hide_status(&st2);
                });
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => {
                {
                    let mut rt = runtime_c.borrow_mut();
                    rt.active_service = TranscriptionService::Api;
                    rt.active_provider = "groq".to_string();
                    rt.api_base_url = config::API_PRESETS[0].base_url.to_string();
                    rt.api_model = config::API_PRESETS[0].default_model.to_string();
                }
                action_c.set_state(&"groq".to_variant());
                show_status(&st, "Model load failed");
                let st2 = st.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                    hide_status(&st2);
                });
                glib::ControlFlow::Break
            }
        }
    });
}

/// Download progress messages sent from the background thread
enum DownloadMsg {
    Progress(u64, Option<u64>), // downloaded, total
    StepLabel(String),          // update the label text
    Done,
    Error(String),
}

fn download_and_load_model(
    runtime: &Rc<RefCell<RuntimeState>>,
    model_path: &std::path::Path,
    url: &str,
    action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
) {
    runtime.borrow_mut().downloading = true;

    show_status(status, "Downloading model...");

    let url = url.to_string();
    let model_path = model_path.to_path_buf();
    let loaded_model_path = model_path.clone();
    let part_path = model_path.with_extension("bin.part");

    let (tx, rx) = std::sync::mpsc::channel::<DownloadMsg>();

    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let resp = reqwest::blocking::Client::new()
                .get(&url)
                .send()
                .map_err(|e| format!("Download request failed: {e}"))?;

            if !resp.status().is_success() {
                return Err(format!("Download failed: HTTP {}", resp.status()));
            }

            let total = resp.content_length();
            let mut downloaded: u64 = 0;

            let mut file = {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&part_path)
                        .map_err(|e| format!("Failed to create file: {e}"))?
                }
                #[cfg(not(unix))]
                {
                    std::fs::File::create(&part_path)
                        .map_err(|e| format!("Failed to create file: {e}"))?
                }
            };

            use std::io::{Read, Write};
            let mut reader = resp;
            let mut buf = [0u8; 65536];
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| format!("Download read error: {e}"))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])
                    .map_err(|e| format!("File write error: {e}"))?;
                downloaded += n as u64;
                let _ = tx.send(DownloadMsg::Progress(downloaded, total));
            }

            // Rename .part → final path
            std::fs::rename(&part_path, &model_path)
                .map_err(|e| format!("Failed to rename model file: {e}"))?;

            Ok(())
        })();

        match result {
            Ok(()) => {
                let _ = tx.send(DownloadMsg::Done);
            }
            Err(e) => {
                // Clean up partial file
                let _ = std::fs::remove_file(&part_path);
                let _ = tx.send(DownloadMsg::Error(e));
            }
        }
    });

    let runtime_c = Rc::clone(runtime);
    let action_c = action.clone();
    let st = status.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        // Drain all pending messages, keep the last one
        let mut last_msg = None;
        while let Ok(msg) = rx.try_recv() {
            last_msg = Some(msg);
        }

        match last_msg {
            Some(DownloadMsg::Progress(downloaded, total)) => {
                let dl_mb = downloaded as f64 / (1024.0 * 1024.0);
                if let Some(t) = total {
                    let total_mb = t as f64 / (1024.0 * 1024.0);
                    show_status(&st, &format!("Downloading: {dl_mb:.0} / {total_mb:.0} MB"));
                } else {
                    show_status(&st, &format!("Downloading: {dl_mb:.0} MB"));
                }
                glib::ControlFlow::Continue
            }
            Some(DownloadMsg::Done) => {
                runtime_c.borrow_mut().downloading = false;
                show_status(&st, "Loading model...");
                // Now load the model
                load_whisper_model(&runtime_c, &loaded_model_path, &action_c, &st);
                glib::ControlFlow::Break
            }
            Some(DownloadMsg::StepLabel(_)) => glib::ControlFlow::Continue,
            Some(DownloadMsg::Error(e)) => {
                eprintln!("Model download failed: {e}");
                {
                    let mut rt = runtime_c.borrow_mut();
                    rt.downloading = false;
                    rt.active_service = TranscriptionService::Api;
                    rt.active_provider = "groq".to_string();
                    rt.api_base_url = config::API_PRESETS[0].base_url.to_string();
                    rt.api_model = config::API_PRESETS[0].default_model.to_string();
                }
                action_c.set_state(&"groq".to_variant());
                show_status(&st, "Download failed");
                let st2 = st.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                    hide_status(&st2);
                });
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}

fn save_window_position(win: &gtk4::ApplicationWindow, db: &Arc<Mutex<Db>>) {
    #[cfg(not(target_os = "linux"))]
    let _ = (&win, &db);

    #[cfg(target_os = "linux")]
    {
        let title = win.title().map(|t| t.to_string()).unwrap_or_default();
        if let Ok(output) = std::process::Command::new("xdotool")
            .args(["search", "--name", &title, "getwindowgeometry"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(pos) = line.strip_prefix("  Position: ")
                    && let Some((xs, ys)) = pos.split_once(',')
                {
                    let x = xs.trim();
                    let y = ys.split_whitespace().next().unwrap_or("0");
                    if let Ok(db) = db.lock() {
                        let _ = db.set_setting("window_x", x);
                        let _ = db.set_setting("window_y", y);
                    }
                }
            }
        }
    }
}

fn position_window(_window: &gtk4::ApplicationWindow, db: &Arc<Mutex<Db>>) {
    let saved = db.lock().ok().and_then(|db| {
        let x = db.get_setting("window_x").ok()??.parse::<i32>().ok()?;
        let y = db.get_setting("window_y").ok()??.parse::<i32>().ok()?;
        Some((x, y))
    });

    #[allow(unused_variables)]
    let (x, y) = match saved {
        Some(pos) => pos,
        None => {
            if let Some(display) = gdk::Display::default() {
                let monitors = display.monitors();
                if let Some(monitor) = monitors
                    .item(0)
                    .and_then(|m| m.downcast::<gdk::Monitor>().ok())
                {
                    let geom = monitor.geometry();
                    (
                        geom.x() + geom.width() - 100,
                        geom.y() + geom.height() - 140,
                    )
                } else {
                    (100, 100)
                }
            } else {
                (100, 100)
            }
        }
    };

    #[cfg(target_os = "linux")]
    {
        let title = "WhisperCrabs";
        let _ = std::process::Command::new("xdotool")
            .args([
                "search",
                "--name",
                title,
                "windowmove",
                &x.to_string(),
                &y.to_string(),
            ])
            .status();
    }
}

fn show_history_dialog(_window: &gtk4::ApplicationWindow, db: &Arc<Mutex<Db>>) {
    let dialog = gtk4::Window::builder()
        .title("WhisperCrabs History")
        .default_width(400)
        .default_height(300)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let header = gtk4::Label::new(Some("Recent Transcriptions"));
    header.add_css_class("heading");
    vbox.append(&header);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_vexpand(true);

    let list_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);

    if let Ok(db) = db.lock()
        && let Ok(entries) = db.recent(20)
    {
        if entries.is_empty() {
            let empty = gtk4::Label::new(Some("No transcriptions yet."));
            list_box.append(&empty);
        } else {
            for entry in entries {
                let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
                let time = gtk4::Label::new(Some(&entry.created_at));
                time.set_halign(gtk4::Align::Start);
                time.set_opacity(0.6);

                let text = gtk4::Label::new(Some(&entry.text));
                text.set_halign(gtk4::Align::Start);
                text.set_wrap(true);
                text.set_selectable(true);

                row.append(&time);
                row.append(&text);

                let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
                list_box.append(&row);
                list_box.append(&sep);
            }
        }
    }

    scroll.set_child(Some(&list_box));
    vbox.append(&scroll);

    dialog.set_child(Some(&vbox));
    dialog.present();
}

// ── TTS helpers ─────────────────────────────────────────────────────────────

/// Play TTS audio with stop support. Calls `on_done` on the UI thread when finished.
fn play_tts_audio<F: FnOnce() + 'static>(
    samples: Vec<i16>,
    sample_rate: u32,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    on_done: F,
) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
        if let Ok((_stream, handle)) = OutputStream::try_default()
            && let Ok(sink) = Sink::try_new(&handle)
        {
            let source = SamplesBuffer::new(1, sample_rate, samples);
            sink.append(source);
            // Poll stop flag while playing
            while !sink.empty() {
                if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    sink.stop();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        let _ = tx.send(());
    });

    let mut on_done = Some(on_done);
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        match rx.try_recv() {
            Ok(()) => {
                if let Some(cb) = on_done.take() {
                    cb();
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        }
    });
}

/// Delete old voice model files (onnx + config) to free disk space.
fn cleanup_old_voice(piper_dir: &std::path::Path, old_voice_id: &str) {
    if old_voice_id.is_empty() || old_voice_id == "none" {
        return;
    }
    let onnx = piper_dir.join(format!("{old_voice_id}.onnx"));
    let json = piper_dir.join(format!("{old_voice_id}.onnx.json"));
    if onnx.exists() {
        dbg_log!("[TTS] deleting old voice model: {}", onnx.display());
        let _ = std::fs::remove_file(&onnx);
    }
    if json.exists() {
        let _ = std::fs::remove_file(&json);
    }
}

/// Download Piper TTS (venv + voice model) with a dialog + progress bar.
/// If `skip_venv` is true, only downloads the voice model files.
#[allow(clippy::too_many_arguments)]
fn download_tts_models(
    runtime: &Rc<RefCell<RuntimeState>>,
    models_dir: &std::path::Path,
    db: &Arc<Mutex<Db>>,
    action: &gtk4::gio::SimpleAction,
    read_cb_action: &gtk4::gio::SimpleAction,
    status: &gtk4::Label,
    parent: &gtk4::ApplicationWindow,
    voice: &'static config::PiperVoice,
    skip_venv: bool,
) {
    runtime.borrow_mut().tts_downloading = true;
    dbg_log!(
        "[TTS] download_tts_models voice={} skip_venv={}",
        voice.id,
        skip_venv
    );

    let piper_dir = models_dir.join("piper");
    std::fs::create_dir_all(&piper_dir).ok();

    // Build download dialog with progress bar
    let dialog = gtk4::Window::builder()
        .title("Downloading TTS")
        .transient_for(parent)
        .modal(true)
        .default_width(340)
        .default_height(120)
        .resizable(false)
        .deletable(false)
        .build();

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.set_margin_top(20);
    vbox.set_margin_bottom(20);
    vbox.set_margin_start(20);
    vbox.set_margin_end(20);

    let initial_label = if skip_venv {
        format!("Downloading voice: {}...", voice.label)
    } else {
        "Setting up Piper TTS engine...".to_string()
    };
    let progress_label = gtk4::Label::new(Some(&initial_label));
    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_show_text(true);
    progress_bar.set_text(Some("Starting..."));

    vbox.append(&progress_label);
    vbox.append(&progress_bar);
    dialog.set_child(Some(&vbox));
    dialog.present();

    let (tx, rx) = std::sync::mpsc::channel::<DownloadMsg>();
    let sdir = piper_dir.clone();
    let voice_id = voice.id.to_string();
    let onnx_url = voice.onnx_url();
    let config_url = voice.config_url();

    std::thread::spawn(move || {
        // Step 1: Set up venv (if needed)
        if !skip_venv {
            let run_cmd = |cmd: &str, args: &[&str]| -> Result<(), String> {
                dbg_log!("[TTS] running: {cmd} {}", args.join(" "));
                let output = std::process::Command::new(cmd)
                    .args(args)
                    .output()
                    .map_err(|e| format!("run {cmd}: {e}"))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!("{cmd} failed: {stderr}"));
                }
                Ok(())
            };

            let _ = tx.send(DownloadMsg::StepLabel(
                "Creating Python environment...".into(),
            ));
            let venv_dir = sdir.join("venv");
            if let Err(e) = run_cmd("python3", &["-m", "venv", &venv_dir.to_string_lossy()]) {
                let _ = tx.send(DownloadMsg::Error(format!("python3 venv: {e}")));
                return;
            }

            let _ = tx.send(DownloadMsg::StepLabel(
                "Installing piper-tts (may take a moment)...".into(),
            ));
            let pip = venv_dir.join("bin/pip");
            if let Err(e) = run_cmd(
                &pip.to_string_lossy(),
                &["install", "piper-tts", "pathvalidate"],
            ) {
                let _ = tx.send(DownloadMsg::Error(format!("pip install: {e}")));
                return;
            }
            dbg_log!("[TTS] piper-tts installed");
        }

        // Step 2: Download voice model files
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("http client");

        let tx_ref = &tx;
        let download_file = |url: &str, dest: &std::path::Path| -> Result<(), String> {
            let part_path = dest.with_extension("part");
            let resp = client
                .get(url)
                .send()
                .map_err(|e| format!("Download: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status()));
            }
            let content_len = resp.content_length();
            let mut file = std::fs::File::create(&part_path).map_err(|e| format!("Create: {e}"))?;
            use std::io::{Read, Write};
            let mut reader = resp;
            let mut buf = [0u8; 65536];
            let mut downloaded: u64 = 0;
            loop {
                let n = reader.read(&mut buf).map_err(|e| format!("Read: {e}"))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])
                    .map_err(|e| format!("Write: {e}"))?;
                downloaded += n as u64;
                let _ = tx_ref.send(DownloadMsg::Progress(downloaded, content_len));
            }
            std::fs::rename(&part_path, dest).map_err(|e| format!("Rename: {e}"))?;
            Ok(())
        };

        let _ = tx.send(DownloadMsg::StepLabel("Downloading voice model...".into()));
        let onnx_dest = sdir.join(format!("{voice_id}.onnx"));
        if let Err(e) = download_file(&onnx_url, &onnx_dest) {
            let _ = tx.send(DownloadMsg::Error(format!("voice model: {e}")));
            return;
        }

        let _ = tx.send(DownloadMsg::StepLabel("Downloading voice config...".into()));
        let config_dest = sdir.join(format!("{voice_id}.onnx.json"));
        if let Err(e) = download_file(&config_url, &config_dest) {
            let _ = tx.send(DownloadMsg::Error(format!("voice config: {e}")));
            return;
        }

        let _ = tx.send(DownloadMsg::Done);
    });

    let runtime_c = Rc::clone(runtime);
    let old_voice_id = runtime.borrow().tts_voice.clone();
    let db_c = Arc::clone(db);
    let action_c = action.clone();
    let read_cb_c = read_cb_action.clone();
    let st = status.clone();
    let sdir = piper_dir;
    let dialog_ref = dialog.clone();
    let pbar = progress_bar.clone();
    let plabel = progress_label.clone();
    let vid = voice.id.to_string();

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let mut last_progress = None;
        let mut last_label = None;
        let mut terminal = None;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                DownloadMsg::Progress(dl, total) => {
                    last_progress = Some((dl, total));
                }
                DownloadMsg::StepLabel(s) => {
                    last_label = Some(s);
                }
                DownloadMsg::Done => {
                    terminal = Some(Ok(()));
                }
                DownloadMsg::Error(e) => {
                    terminal = Some(Err(e));
                }
            }
        }

        if let Some(label) = last_label {
            plabel.set_label(&label);
        }

        if let Some((downloaded, total)) = last_progress {
            if let Some(total) = total {
                if total > 0 {
                    let frac = downloaded as f64 / total as f64;
                    pbar.set_fraction(frac);
                    let dl_mb = downloaded as f64 / 1_048_576.0;
                    let total_mb = total as f64 / 1_048_576.0;
                    pbar.set_text(Some(&format!("{dl_mb:.1} / {total_mb:.1} MB")));
                }
            } else {
                pbar.pulse();
                let dl_mb = downloaded as f64 / 1_048_576.0;
                pbar.set_text(Some(&format!("{dl_mb:.1} MB")));
            }
        }

        match terminal {
            Some(Ok(())) => {
                dialog_ref.close();
                // Clean up previous voice files
                if old_voice_id != vid {
                    cleanup_old_voice(&sdir, &old_voice_id);
                }
                show_status(&st, "Loading TTS...");
                match PiperTts::new(&sdir, &vid) {
                    Ok(engine) => {
                        let mut rt = runtime_c.borrow_mut();
                        rt.tts_provider = TtsProvider::Piper;
                        rt.tts_voice = vid.clone();
                        rt.tts_engine = Some(Arc::new(TtsEngine::Piper(engine)));
                        rt.tts_downloading = false;
                        if let Ok(d) = db_c.lock() {
                            let _ = d.set_setting("tts_provider", "piper");
                            let _ = d.set_setting("tts_voice", &vid);
                        }
                        read_cb_c.set_enabled(true);
                        action_c.set_state(&vid.to_variant());
                        hide_status(&st);
                    }
                    Err(e) => {
                        eprintln!("TTS load failed: {e}");
                        runtime_c.borrow_mut().tts_downloading = false;
                        show_status(&st, "TTS load failed");
                        let st2 = st.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(3),
                            move || hide_status(&st2),
                        );
                    }
                }
                glib::ControlFlow::Break
            }
            Some(Err(e)) => {
                dialog_ref.close();
                eprintln!("TTS download failed: {e}");
                runtime_c.borrow_mut().tts_downloading = false;
                show_status(&st, "TTS download failed");
                let st2 = st.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
                    hide_status(&st2);
                });
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}
