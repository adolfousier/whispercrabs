models_dir := env("HOME") / ".local/share/whispercrabs/models"
default_model := "ggml-base.en.bin"

# Install system dependencies
deps:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname)" == "Darwin" ]]; then
        if ! brew list gtk4 &>/dev/null; then
            echo "Installing gtk4 via Homebrew..."
            brew install gtk4
        else
            echo "gtk4 already installed."
        fi
    elif command -v apt-get &>/dev/null; then
        if ! pkg-config --exists gtk4 2>/dev/null; then
            echo "Installing GTK4 dev packages..."
            sudo apt-get install -y libgtk-4-dev
        else
            echo "GTK4 already installed."
        fi
    elif command -v dnf &>/dev/null; then
        if ! pkg-config --exists gtk4 2>/dev/null; then
            echo "Installing GTK4 dev packages..."
            sudo dnf install -y gtk4-devel
        else
            echo "GTK4 already installed."
        fi
    else
        echo "Unsupported platform. Please install GTK4 manually."
        exit 1
    fi

# Build release binary
build: deps
    cargo build --release

# Run with current .env settings
run: deps
    cargo run --release

# Download whisper model and run in local mode
run-local model=default_model: deps
    @mkdir -p {{models_dir}}
    @if [ ! -f "{{models_dir}}/{{model}}" ]; then \
        echo "Downloading {{model}}..."; \
        curl -L -o "{{models_dir}}/{{model}}" \
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{{model}}"; \
    else \
        echo "Model {{model}} already downloaded."; \
    fi
    PRIMARY_TRANSCRIPTION_SERVICE=local WHISPER_MODEL={{model}} cargo run --release

# Run in API mode (default: Groq)
run-api: deps
    PRIMARY_TRANSCRIPTION_SERVICE=api cargo run --release

# Download a whisper model (without running)
download-model model=default_model:
    @mkdir -p {{models_dir}}
    curl -L -o "{{models_dir}}/{{model}}" \
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{{model}}"
    @echo "Saved to {{models_dir}}/{{model}}"

# List available whisper models
list-models:
    @echo "Available models (pass to run-local or download-model):"
    @echo "  ggml-tiny.en.bin     (~75MB, fastest, English only)"
    @echo "  ggml-base.en.bin     (~142MB, good balance, English only) [default]"
    @echo "  ggml-small.en.bin    (~466MB, better accuracy, English only)"
    @echo "  ggml-medium.en.bin   (~1.5GB, high accuracy, English only)"
    @echo "  ggml-large-v3.bin    (~3.1GB, best accuracy, multilingual)"
    @echo ""
    @echo "Example: just run-local ggml-small.en.bin"
