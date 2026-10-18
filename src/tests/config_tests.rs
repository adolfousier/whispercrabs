use crate::config;

#[test]
fn find_preset_returns_known_providers() {
    let groq = config::find_preset("groq");
    assert!(groq.is_some());
    let groq = groq.unwrap();
    assert_eq!(groq.id, "groq");
    assert!(groq.needs_key);
    assert!(!groq.base_url.is_empty());

    let ollama = config::find_preset("ollama");
    assert!(ollama.is_some());
    assert!(!ollama.unwrap().needs_key);
}

#[test]
fn find_preset_returns_none_for_unknown() {
    assert!(config::find_preset("nonexistent").is_none());
}

#[test]
fn all_api_presets_have_required_fields() {
    for preset in config::API_PRESETS {
        assert!(!preset.id.is_empty(), "preset id must not be empty");
        assert!(!preset.label.is_empty(), "preset label must not be empty");
        assert!(
            preset.base_url.starts_with("http"),
            "preset {} base_url must start with http",
            preset.id
        );
        assert!(
            !preset.default_model.is_empty(),
            "preset {} default_model must not be empty",
            preset.id
        );
    }
}

#[test]
fn find_local_model_returns_known_models() {
    let tiny = config::find_local_model("local-tiny");
    assert!(tiny.is_some());
    let tiny = tiny.unwrap();
    assert_eq!(tiny.file_name, "ggml-tiny.en.bin");

    let medium = config::find_local_model("local-medium");
    assert!(medium.is_some());
}

#[test]
fn find_local_model_returns_none_for_unknown() {
    assert!(config::find_local_model("local-nonexistent").is_none());
}

#[test]
fn all_local_model_presets_have_required_fields() {
    for model in config::LOCAL_MODEL_PRESETS {
        assert!(!model.id.is_empty());
        assert!(!model.label.is_empty());
        assert!(
            model.file_name.ends_with(".bin"),
            "model {} file_name should end with .bin",
            model.id
        );
        assert!(!model.size_label.is_empty());
    }
}

#[test]
fn default_local_model_is_valid() {
    assert!(config::find_local_model(config::DEFAULT_LOCAL_MODEL).is_some());
}

#[test]
fn model_url_produces_valid_huggingface_url() {
    let url = config::model_url("ggml-tiny.en.bin");
    assert!(url.starts_with("https://huggingface.co/"));
    assert!(url.ends_with("ggml-tiny.en.bin"));
}

#[test]
fn find_tts_preset_returns_known_providers() {
    let openai = config::find_tts_preset("openai");
    assert!(openai.is_some());
    let openai = openai.unwrap();
    assert!(openai.needs_key);
    assert!(openai.base_url.starts_with("https://"));
    assert!(!openai.default_model.is_empty());
    assert!(!openai.default_voice.is_empty());

    let openrouter = config::find_tts_preset("openrouter");
    assert!(openrouter.is_some());
    assert!(openrouter.unwrap().base_url.contains("openrouter.ai"));

    let custom = config::find_tts_preset("custom");
    assert!(custom.is_some());
    assert!(!custom.unwrap().needs_key);
}

#[test]
fn find_tts_preset_returns_none_for_unknown() {
    assert!(config::find_tts_preset("nonexistent").is_none());
}

#[test]
fn all_tts_api_presets_have_required_fields() {
    for preset in config::TTS_API_PRESETS {
        assert!(!preset.id.is_empty(), "preset id must not be empty");
        assert!(!preset.label.is_empty(), "preset label must not be empty");
        assert!(
            preset.base_url.starts_with("http"),
            "preset {} base_url must start with http",
            preset.id
        );
        assert!(
            !preset.default_model.is_empty(),
            "preset {} default_model must not be empty",
            preset.id
        );
        assert!(
            !preset.default_voice.is_empty(),
            "preset {} default_voice must not be empty",
            preset.id
        );
    }
}

#[test]
fn tts_presets_do_not_collide_with_piper_voice_ids() {
    for preset in config::TTS_API_PRESETS {
        assert!(
            config::find_piper_voice(preset.id).is_none(),
            "TTS preset id {} collides with a Piper voice id",
            preset.id
        );
    }
}

#[test]
fn default_piper_voice_is_valid() {
    assert!(config::find_piper_voice(config::DEFAULT_PIPER_VOICE).is_some());
}
