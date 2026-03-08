use std::path::PathBuf;

/// Active transcription backend.
#[derive(Clone, Copy, PartialEq)]
pub enum TranscriptionService {
    Api,
    Local,
}

/// Built-in API provider configuration.
pub struct ApiPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub needs_key: bool,
}

/// Pre-configured API providers (Groq, Ollama, OpenRouter, LM Studio).
pub const API_PRESETS: &[ApiPreset] = &[
    ApiPreset {
        id: "groq",
        label: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        default_model: "whisper-large-v3-turbo",
        needs_key: true,
    },
    ApiPreset {
        id: "ollama",
        label: "Ollama",
        base_url: "http://localhost:11434/v1",
        default_model: "whisper",
        needs_key: false,
    },
    ApiPreset {
        id: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openai/whisper-1",
        needs_key: true,
    },
    ApiPreset {
        id: "lmstudio",
        label: "LM Studio",
        base_url: "http://localhost:1234/v1",
        default_model: "whisper-1",
        needs_key: false,
    },
];

/// Look up an API preset by its short identifier.
pub fn find_preset(id: &str) -> Option<&'static ApiPreset> {
    API_PRESETS.iter().find(|p| p.id == id)
}

/// Built-in local whisper model preset.
pub struct LocalModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub file_name: &'static str,
    pub size_label: &'static str,
}

/// Available local whisper model sizes (Tiny through Medium).
pub const LOCAL_MODEL_PRESETS: &[LocalModelPreset] = &[
    LocalModelPreset {
        id: "local-tiny",
        label: "Tiny",
        file_name: "ggml-tiny.en.bin",
        size_label: "~75 MB",
    },
    LocalModelPreset {
        id: "local-base",
        label: "Base",
        file_name: "ggml-base.en.bin",
        size_label: "~142 MB",
    },
    LocalModelPreset {
        id: "local-small",
        label: "Small",
        file_name: "ggml-small.en.bin",
        size_label: "~466 MB",
    },
    LocalModelPreset {
        id: "local-medium",
        label: "Medium",
        file_name: "ggml-medium.en.bin",
        size_label: "~1.5 GB",
    },
];

/// Default local model preset ID.
pub const DEFAULT_LOCAL_MODEL: &str = "local-tiny";

/// Look up a local model preset by its short identifier.
pub fn find_local_model(id: &str) -> Option<&'static LocalModelPreset> {
    LOCAL_MODEL_PRESETS.iter().find(|m| m.id == id)
}

/// Build the HuggingFace download URL for a whisper model file.
pub fn model_url(file_name: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        file_name
    )
}

/// Application configuration loaded from environment and `.env` file.
pub struct Config {
    pub transcription_service: TranscriptionService,
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub api_model: String,
    pub db_path: PathBuf,
    pub models_dir: PathBuf,
    pub sound_notification: bool,
}

impl Config {
    pub fn load() -> Self {
        // Try loading .env from current dir, ignore if missing
        let _ = dotenvy::dotenv();

        let transcription_service = match std::env::var("PRIMARY_TRANSCRIPTION_SERVICE")
            .unwrap_or_else(|_| "api".into())
            .to_lowercase()
            .as_str()
        {
            "local" => TranscriptionService::Local,
            // Accept "api" and legacy "groq"
            _ => TranscriptionService::Api,
        };

        // API_BASE_URL with default pointing to Groq (backwards compatible)
        let api_base_url = std::env::var("API_BASE_URL")
            .unwrap_or_else(|_| "https://api.groq.com/openai/v1".into());

        // API_KEY with GROQ_API_KEY as legacy fallback
        let api_key = std::env::var("API_KEY")
            .or_else(|_| std::env::var("GROQ_API_KEY"))
            .ok();

        // API_MODEL with GROQ_STT_MODEL as legacy fallback
        let api_model = std::env::var("API_MODEL")
            .or_else(|_| std::env::var("GROQ_STT_MODEL"))
            .unwrap_or_else(|_| "whisper-large-v3-turbo".into());

        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("whispercrabs");
        std::fs::create_dir_all(&data_dir).ok();
        let db_path = data_dir.join("history.db");

        let models_dir = data_dir.join("models");
        std::fs::create_dir_all(&models_dir).ok();

        let sound_notification = std::env::var("SOUND_NOTIFICATION_ON_COMPLETION")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        Self {
            transcription_service,
            api_base_url,
            api_key,
            api_model,
            db_path,
            models_dir,
            sound_notification,
        }
    }
}
