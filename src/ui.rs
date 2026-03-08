use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::audio::Recorder;
use crate::config::{self, Config, TranscriptionService};
use crate::db::Db;
use crate::local_stt::LocalWhisper;

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

/// Show a status message. On Linux, sets the inline label. On macOS, shows a modal dialog for errors.
fn show_status(label: &gtk4::Label, text: &str) {
    #[cfg(not(target_os = "macos"))]
    {
        label.set_label(text);
        label.set_opacity(1.0);
    }

    #[cfg(target_os = "macos")]
    {
        // On macOS, never show inline text — use a dialog for errors,
        // silently ignore non-error status to keep the button clean.
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
}

struct RuntimeState {
    active_service: TranscriptionService,
    active_provider: String, // "groq", "ollama", ..., "custom", "local"
    api_base_url: String,    // active API base URL
    api_key: Option<String>, // active API key
    api_model: String,       // active API model
    local_whisper: Option<Arc<LocalWhisper>>,
    downloading: bool,
}

pub fn build_ui(app: &gtk4::Application, config: Arc<Config>) {
    // Load CSS
    let provider = gtk4::CssProvider::new();
    provider.load_from_data(CSS);
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
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
        let loader = gtk4::gdk_pixbuf::PixbufLoader::with_type("svg").unwrap();
        loader.write(MIC_SVG).unwrap();
        loader.close().unwrap();
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
                let d = db.lock().unwrap();
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

    // Runtime state (UI-thread only)
    let runtime = Rc::new(RefCell::new(RuntimeState {
        active_service: initial_service,
        active_provider: initial_provider.clone(),
        api_base_url: initial_base_url,
        api_key: initial_api_key,
        api_model: initial_api_model,
        local_whisper: initial_whisper,
        downloading: false,
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
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            let result = rt
                                .block_on(crate::api::transcribe(&base_url, &api_key, &model, wav));
                            let _ = tx.send(result);
                        });
                    }
                    TranscriptionService::Local => {
                        let whisper = rt.local_whisper.clone().unwrap();
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
            State::Processing => {}
        }
    });

    // --- Right-click popover menu (on the button) ---
    let mode_action = gtk4::gio::SimpleAction::new_stateful(
        "transcription-mode",
        Some(&String::static_variant_type()),
        &initial_provider.to_variant(),
    );

    let providers_section = gtk4::gio::Menu::new();
    for preset in config::API_PRESETS {
        providers_section.append(
            Some(preset.label),
            Some(&format!("app.transcription-mode::{}", preset.id)),
        );
    }
    providers_section.append(
        Some("Custom API..."),
        Some("app.transcription-mode::custom"),
    );

    let local_section = gtk4::gio::Menu::new();
    for lm in config::LOCAL_MODEL_PRESETS {
        local_section.append(
            Some(&format!("Local — {} ({})", lm.label, lm.size_label)),
            Some(&format!("app.transcription-mode::{}", lm.id)),
        );
    }

    let actions_section = gtk4::gio::Menu::new();
    actions_section.append(Some("History"), Some("app.show-history"));
    actions_section.append(Some("Quit"), Some("app.quit"));

    let menu = gtk4::gio::Menu::new();
    menu.append_section(Some("Transcription"), &providers_section);
    menu.append_section(None, &local_section);
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
        let chosen: String = param.unwrap().get::<String>().unwrap();

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
        let json_str: String = param.unwrap().get::<String>().unwrap();
        eprintln!("[dbus] 'set-api-config' action activated: {json_str}");

        let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("set-api-config: invalid JSON: {e}");
                return;
            }
        };

        let base_url = match parsed["base_url"].as_str() {
            Some(u) => u.to_string(),
            None => {
                eprintln!("set-api-config: missing 'base_url'");
                return;
            }
        };
        let model = match parsed["model"].as_str() {
            Some(m) => m.to_string(),
            None => {
                eprintln!("set-api-config: missing 'model'");
                return;
            }
        };
        let api_key = parsed["api_key"].as_str().map(|s| s.to_string());

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

    window.present();
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

            let mut file = std::fs::File::create(&part_path)
                .map_err(|e| format!("Failed to create file: {e}"))?;

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
