# Changelog

## Unreleased

- **Application icon** — WhisperCrabs no longer shows the generic executable placeholder:
  - **macOS** ships `WhisperCrabs.app` in the release tarball (`scripts/bundle-macos.sh`), which is where macOS reads the icon from, and the binary also sets the Dock icon from an embedded `.icns` at startup so a loose binary is branded while it runs
  - **Windows** embeds the `.ico` as a resource in `whispercrabs.exe` via `build.rs`
  - **Linux** sets the themed icon by app id, and the release tarball ships a `.desktop` entry plus hicolor PNGs (`just install-desktop` installs them from a checkout)
- **Icon tests** — guards the app id, desktop entry, and icon assets so the OS lookup cannot silently break again

## v0.1.23 — 2026-03-09

- **Auto-cleanup old voice models** — switching TTS voices deletes the previous voice files (~63 MB) to free disk space
- **README TTS usage steps** — clear 1-2-3 instructions for Read Clipboard workflow

## v0.1.22 — 2026-03-09

- **Text-to-speech via Piper** — optional local TTS powered by [Piper](https://github.com/rhasspy/piper) (Python venv, auto-installed on first use)
- **6 built-in TTS voices** — Amy, Lessac, Ryan, Kristin, Joe (US English), Cori (UK English), switchable from the right-click menu
- **Read Clipboard** — select any text, copy it, click "Read Clipboard" to hear it spoken aloud
- **TTS button states** — yellow while synthesizing, green while speaking; click to stop playback
- **TTS management** — Reset TTS (re-download on errors) and Delete TTS (remove all TTS data) in the right-click menu
- **Smart text cleaning** — strips terminal formatting, markdown decoration, and collapsed whitespace before speech
- **Right-click menu reorganized** — clear STT/TTS separation: "STT — API", "STT — Local", "TTS — Voices"
- **README updated** — local-first messaging, TTS documentation, voices table, D-Bus TTS commands
- **`cargo publish` in CI** — crates.io publish step added before building release artifacts

## v0.1.21 — 2026-03-08

- **Security hardening** — full audit and fixes across all modules:
  - API URL validation: reject `file://`, `ftp://`, and other non-HTTP schemes; warn on unencrypted HTTP for remote hosts
  - Request timeouts: 120s request timeout, 10s connect timeout on all API calls
  - D-Bus input sanitization: typed JSON deserialization, size cap (4 KB), API keys redacted from logs
  - Database setting key whitelist: only known keys accepted, values capped at 4 KB
  - File permissions: data directory and model downloads set to `0o700`/`0o600` on Unix
  - Replaced all `.unwrap()` with `.expect()` or proper error propagation
  - D-Bus param extraction uses safe `Option` unwrapping instead of panic-on-None
  - Local whisper model gracefully returns error if not loaded instead of panicking
- **Security tests** — setting key rejection, `file://` URL rejection, `ftp://` URL rejection (34 total tests)

## v0.1.20 — 2026-03-08

- **macOS error dialog** — errors ("No microphone found", "No API key set", etc.) now show as a native modal dialog on macOS instead of truncated inline text
- **Clean macOS button** — no inline status text ever renders on macOS; the button stays pristine
- Status overlay approach on macOS prevents window layout from breaking
- **Crate-level documentation** — added `//!` module docs for `cargo doc` / docs.rs landing page
- **Docstrings** — added `///` doc comments to all public structs, enums, functions, and constants
- Added `rust-version = "1.85"` (MSRV) to `Cargo.toml`

## v0.1.19 — 2026-03-07

- **macOS status notifications** — "No microphone found", "No API key set", and all error/status messages now properly show on macOS (previously hidden due to layout fix)
- Use system icon on Linux, bundled SVG on macOS for mic button

## v0.1.18 — 2026-03-07

- **macOS glassmorphic UI** — dark semi-transparent background, branded label, hidden status placeholder for balanced layout
- **Crisp mic icon** — SVG rasterized at 64px for sharp rendering at all display sizes
- **Expanded test suite** — 31 tests covering API, audio encoding, config, database, and model downloads
- Added macOS floating demo screenshot to README

## v0.1.17 — 2026-03-05

- **Cross-platform mic icon** — replaced freedesktop icon theme with bundled SVG; mic button now renders correctly on macOS
- **macOS UI polish** — dark window background, branded "WHISPER CRABS" label, tuned button/icon sizing for macOS
- **Unit tests** — added test suite for config presets, database operations, and model download URLs
- Fixed clippy warnings (unused variables on non-Linux platforms)
- Code formatting cleanup (`cargo fmt`)

## v0.1.16 — 2026-02-18

- **API provider presets in right-click menu** — Groq, Ollama, OpenRouter, LM Studio as one-click radio items
- **Custom API dialog** — "Custom API..." opens a GTK4 dialog with Base URL, API Key, and Model fields; persists to DB
- **Local model size picker** — choose between Tiny (~75 MB), Base (~142 MB), Small (~466 MB), Medium (~1.5 GB)
- **D-Bus `set-api-config` action** — AI agents can programmatically configure custom API endpoints via JSON
- Provider and local model selection persists across restarts
- Switching between local model sizes auto-deletes the previous model to free disk space
- API key per-provider: stored in DB, looked up per preset on switch
- Providers that don't need an API key (Ollama, LM Studio) skip the key check
- AI Agent-Ready: fully controllable via D-Bus — switch providers, set custom endpoints, record/stop

## v0.1.15 — 2026-02-18

- Runtime transcription mode switching via right-click menu (API Mode / Local Mode radio items)
- Switching to Local auto-downloads the whisper model if missing, with progress display (MB / total MB)
- Switching to API deletes the local model file to free disk space
- Mode choice persists across restarts (saved to DB)
- App no longer panics on startup when API key is missing or model file is absent
- Guards: blocks recording during model download, blocks mode switch during recording/processing
- Graceful error handling: missing API key or model shows status message instead of crashing

## v0.1.14 — 2026-02-13

- Global keyboard shortcuts via D-Bus actions (`record`, `stop`) — works on GNOME, KDE, Sway, Hyprland, i3
- Sound notification on transcription completion (`SOUND_NOTIFICATION_ON_COMPLETION=true`)
- Removed auto-paste — text is now copied to clipboard, user pastes manually with Ctrl+V
- Removed X11 dependency for core features (xdotool, wmctrl no longer required)
- Fixed button shape changing between states (single Image widget with locked pixel size)
- Esc key stops recording when window is focused
- Wayland-native: works on GNOME Wayland without X11 tools
- Updated README with keyboard shortcut setup for all major DEs
- Added dates to all changelog entries
- Renamed project from LinWhisper to **WhisperCrabs**

## v0.1.13 — 2026-02-13

- Custom OpenAI-compatible API base URL via `API_BASE_URL` env var
- Works with any OpenAI-compatible backend: Groq (default), Ollama, OpenRouter, LM Studio, LocalAI, etc.
- Renamed env vars: `API_KEY`, `API_MODEL`, `API_BASE_URL` (old `GROQ_API_KEY`/`GROQ_STT_MODEL` still work as fallback)
- `TranscriptionService::Groq` renamed to `Api`
- `PRIMARY_TRANSCRIPTION_SERVICE` now accepts `api` (and `groq` as legacy alias)

## v0.1.12 — 2026-02-13

- Added fully local transcription via whisper.cpp (whisper-rs + rubato)
- New `PRIMARY_TRANSCRIPTION_SERVICE` env var: `local` or `groq`
- `GROQ_API_KEY` only required when using Groq backend
- New `WHISPER_MODEL` env var for selecting whisper model (default: ggml-base.en.bin)
- Models stored in `~/.local/share/whispercrabs/models/`
- Clear error message with download instructions if model file is missing

## v0.1.11 — 2026-02-13

- Replaced Unicode emoji icons with GTK4 symbolic SVG icons (properly centered)
- Removed all box shadows for a clean flat look on any background
- Removed white border artifact
- Fixed accidental recording via Enter key (button no longer focusable)
- Bumped icon and status label font sizes
- Added button states screenshot to README
- Fixed misleading "runs entirely on your machine" wording
- Removed em-dashes from README
- Updated Cargo edition to 2024

## v0.1.1 — 2026-02-13

- Improved UI: square button with soft border radius, red idle / green recording
- Drag-to-move with position persistence across sessions
- Right-click popover menu (History, Quit)
- Hover effect styling fixes
- History dialog no longer resets button position

## v0.1.0 — 2026-02-13

- Initial release
- Floating always-on-top mic button (GTK4)
- One-click voice recording via cpal
- Transcription via Groq API (whisper-large-v3-turbo)
- Auto-paste into focused input via xclip + xdotool
- SQLite transcription history
- Privacy-first: audio in-memory only, never written to disk
