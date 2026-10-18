/// Text-to-speech engines.
///
/// Local synthesis runs through Piper (Python `piper-tts` package). Remote
/// synthesis talks to any OpenAI-compatible `/audio/speech` endpoint
/// (OpenAI, OpenRouter, LocalAI, and similar servers).
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Longest text sent in a single API request. OpenAI caps TTS input at 4096
/// characters, so stay just below it and concatenate the audio chunks.
const MAX_TTS_CHARS: usize = 3900;

/// Sample rate assumed for raw PCM responses without a `rate` parameter.
const DEFAULT_PCM_RATE: u32 = 24000;

/// Synthesized mono 16-bit PCM audio.
pub struct Speech {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

/// Active TTS engine.
pub enum TtsEngine {
    Piper(PiperTts),
    Api(ApiTts),
}

impl TtsEngine {
    /// Synthesize speech. Returns mono PCM samples and their sample rate.
    pub fn synthesize(&self, text: &str) -> Result<Speech, String> {
        match self {
            Self::Piper(engine) => engine.synthesize(text),
            Self::Api(engine) => engine.synthesize(text),
        }
    }
}

/// OpenAI-compatible text-to-speech engine.
///
/// Sends `POST {base_url}/audio/speech` with `response_format: "pcm"` and
/// turns the response into samples. WAV responses are accepted too; MP3
/// responses are rejected with an actionable error because this binary
/// ships no MP3 decoder.
#[derive(Debug)]
pub struct ApiTts {
    base_url: String,
    api_key: Option<String>,
    model: String,
    voice: String,
}

impl ApiTts {
    /// Create an engine. `base_url` must be http(s), `model` must be set.
    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        model: &str,
        voice: &str,
    ) -> Result<Self, String> {
        let base_url = base_url.trim_end_matches('/');
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err("Invalid TTS API URL: only http:// and https:// are allowed".into());
        }
        if base_url.starts_with("http://")
            && !base_url.starts_with("http://localhost")
            && !base_url.starts_with("http://127.0.0.1")
            && !base_url.starts_with("http://[::1]")
        {
            eprintln!("WARNING: TTS endpoint uses unencrypted HTTP for a remote host");
        }

        let model = model.trim();
        if model.is_empty() {
            return Err("TTS model must not be empty".into());
        }

        Ok(Self {
            base_url: base_url.to_string(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            model: model.to_string(),
            voice: normalize_voice(voice),
        })
    }

    /// The speech endpoint for the configured base URL.
    pub fn endpoint_url(&self) -> String {
        format!("{}/audio/speech", self.base_url)
    }

    /// Synthesize speech. Long text is split into chunks and concatenated.
    pub fn synthesize(&self, text: &str) -> Result<Speech, String> {
        let text = clean_for_speech(text);
        if text.is_empty() {
            return Err("nothing to speak after cleaning".into());
        }
        dbg_log!("[TTS] API synthesize, cleaned len={}", text.len());
        let t0 = std::time::Instant::now();

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let mut samples = Vec::new();
        let mut sample_rate = 0;
        for chunk in chunk_text(&text, MAX_TTS_CHARS) {
            let (part, rate) = self.synthesize_chunk(&client, &chunk)?;
            if sample_rate == 0 {
                sample_rate = rate;
            } else if sample_rate != rate {
                return Err(format!(
                    "TTS endpoint changed sample rate between chunks ({sample_rate} Hz vs {rate} Hz)"
                ));
            }
            samples.extend(part);
        }

        dbg_log!(
            "[TTS] API synthesized {} samples ({:.1}s audio) in {:.1}s",
            samples.len(),
            samples.len() as f32 / sample_rate.max(1) as f32,
            t0.elapsed().as_secs_f32()
        );

        Ok(Speech {
            samples,
            sample_rate,
        })
    }

    fn synthesize_chunk(
        &self,
        client: &reqwest::blocking::Client,
        text: &str,
    ) -> Result<(Vec<i16>, u32), String> {
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": self.voice,
            "response_format": "pcm",
        });

        let mut request = client.post(self.endpoint_url()).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let resp = request.send().map_err(|e| format!("Request failed: {e}"))?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();

        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(format!("TTS API error {status}: {body}"));
        }

        let bytes = resp.bytes().map_err(|e| format!("Read TTS audio: {e}"))?;
        decode_audio(&bytes, &content_type)
    }
}

/// Empty voice values fall back to the OpenAI default.
pub(crate) fn normalize_voice(voice: &str) -> String {
    let voice = voice.trim();
    if voice.is_empty() {
        "alloy".to_string()
    } else {
        voice.to_string()
    }
}

/// Decode a TTS response body into mono PCM samples and a sample rate.
///
/// Accepts 16-bit WAV and raw 16-bit little-endian PCM. The raw path reads
/// `rate` and `channels` from the Content-Type parameters (for example
/// `audio/pcm;rate=24000;channels=1`) and defaults to 24 kHz mono.
pub(crate) fn decode_audio(bytes: &[u8], content_type: &str) -> Result<(Vec<i16>, u32), String> {
    if bytes.is_empty() {
        return Err("TTS endpoint returned no audio".into());
    }

    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return decode_wav(bytes);
    }

    let ct = content_type.to_ascii_lowercase();
    if ct.contains("mpeg")
        || ct.contains("mp3")
        || bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        return Err(
            "TTS endpoint returned MP3 audio; WhisperCrabs needs raw PCM. \
             Use a provider or model that supports response_format \"pcm\"."
                .into(),
        );
    }

    let (rate, channels) = parse_pcm_params(&ct);
    if bytes.len() % 2 != 0 {
        return Err("TTS audio has an odd byte count".into());
    }
    let raw: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    Ok((downmix(&raw, channels), rate))
}

/// Read `rate` and `channels` from a Content-Type parameter list.
fn parse_pcm_params(content_type: &str) -> (u32, u16) {
    let mut rate = DEFAULT_PCM_RATE;
    let mut channels = 1u16;
    for param in content_type.split(';').skip(1) {
        let param = param.trim();
        if let Some(value) = param.strip_prefix("rate=")
            && let Ok(parsed) = value.trim().parse::<u32>()
            && parsed > 0
        {
            rate = parsed;
        } else if let Some(value) = param.strip_prefix("channels=")
            && let Ok(parsed) = value.trim().parse::<u16>()
            && parsed > 0
        {
            channels = parsed;
        }
    }
    (rate, channels)
}

fn decode_wav(bytes: &[u8]) -> Result<(Vec<i16>, u32), String> {
    let mut reader =
        hound::WavReader::new(Cursor::new(bytes)).map_err(|e| format!("WAV parse error: {e}"))?;
    let spec = reader.spec();
    if spec.bits_per_sample != 16 {
        return Err(format!(
            "unsupported WAV bit depth: {} (expected 16)",
            spec.bits_per_sample
        ));
    }
    let raw: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap_or(0)).collect();
    Ok((downmix(&raw, spec.channels), spec.sample_rate))
}

/// Average interleaved channels into mono.
fn downmix(raw: &[i16], channels: u16) -> Vec<i16> {
    if channels <= 1 {
        return raw.to_vec();
    }
    let channels = channels as usize;
    let frames = raw.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let start = frame * channels;
        let sum: i32 = raw[start..start + channels].iter().map(|s| *s as i32).sum();
        mono.push((sum / channels as i32) as i16);
    }
    mono
}

/// Split text into API-sized chunks, preferring sentence boundaries.
pub(crate) fn chunk_text(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(text) {
        for piece in pack_piece(&sentence, max) {
            let extra = if current.is_empty() {
                piece.chars().count()
            } else {
                piece.chars().count() + 1
            };
            if !current.is_empty() && current.chars().count() + extra > max {
                chunks.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
            current.clear();
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

/// Split one over-long sentence on word boundaries, hard-splitting a single
/// word only when it alone exceeds the limit.
fn pack_piece(sentence: &str, max: usize) -> Vec<String> {
    if sentence.chars().count() <= max {
        return vec![sentence.to_string()];
    }
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        if word.chars().count() > max {
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
            let mut buf = String::new();
            for ch in word.chars() {
                buf.push(ch);
                if buf.chars().count() == max {
                    pieces.push(std::mem::take(&mut buf));
                }
            }
            current = buf;
            continue;
        }
        let extra = if current.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if !current.is_empty() && current.chars().count() + extra > max {
            pieces.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Piper TTS wrapper.
pub struct PiperTts {
    piper_bin: PathBuf,
    model_path: PathBuf,
    sample_rate: u32,
}

impl PiperTts {
    /// Load Piper from `piper_dir/` with a specific voice model.
    /// Expects `venv/bin/piper` and `{voice_id}.onnx` + `{voice_id}.onnx.json`.
    pub fn new(piper_dir: &Path, voice_id: &str) -> Result<Self, String> {
        dbg_log!(
            "[TTS] PiperTts::new from {} voice={}",
            piper_dir.display(),
            voice_id
        );

        let piper_bin = piper_dir.join("venv/bin/piper");
        let model_path = piper_dir.join(format!("{voice_id}.onnx"));
        let config_path = piper_dir.join(format!("{voice_id}.onnx.json"));

        if !piper_bin.exists() {
            return Err(format!("missing piper binary: {}", piper_bin.display()));
        }
        if !model_path.exists() {
            return Err(format!("missing voice model: {}", model_path.display()));
        }
        if !config_path.exists() {
            return Err(format!("missing voice config: {}", config_path.display()));
        }

        // Extract sample_rate from config
        let config_str =
            std::fs::read_to_string(&config_path).map_err(|e| format!("read config: {e}"))?;
        let sample_rate = extract_sample_rate(&config_str).unwrap_or(22050);
        dbg_log!("[TTS] sample_rate={sample_rate}");

        dbg_log!("[TTS] Piper ready");
        Ok(Self {
            piper_bin,
            model_path,
            sample_rate,
        })
    }

    /// Synthesize speech from text. Returns mono 16-bit PCM samples.
    pub fn synthesize(&self, text: &str) -> Result<Speech, String> {
        let text = clean_for_speech(text);
        if text.is_empty() {
            return Err("nothing to speak after cleaning".into());
        }
        dbg_log!("[TTS] synthesize, cleaned len={}", text.len());
        let t0 = std::time::Instant::now();

        let output = std::process::Command::new(&self.piper_bin)
            .arg("--model")
            .arg(&self.model_path)
            .arg("--output_raw")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn piper: {e}"))?;

        use std::io::Write;
        let mut child = output;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("write to piper: {e}"))?;
        }

        let result = child
            .wait_with_output()
            .map_err(|e| format!("piper wait: {e}"))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!("piper failed: {stderr}"));
        }

        let raw = &result.stdout;
        if raw.len() % 2 != 0 {
            return Err("piper output has odd byte count".into());
        }

        let samples: Vec<i16> = raw
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        dbg_log!(
            "[TTS] synthesized {} samples ({:.1}s audio) in {:.1}s",
            samples.len(),
            samples.len() as f32 / self.sample_rate as f32,
            t0.elapsed().as_secs_f32()
        );

        Ok(Speech {
            samples,
            sample_rate: self.sample_rate,
        })
    }
}

/// Clean text for speech synthesis — strip terminal/markdown noise.
fn clean_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        // Skip empty lines, lines that are just symbols/decoration
        if trimmed.is_empty()
            || trimmed
                .chars()
                .all(|c| "─━═⏺•◆▸▹►▪■□●○-=_*#>|+".contains(c))
        {
            continue;
        }
        // Strip leading bullet/decoration chars
        let cleaned = trimmed
            .trim_start_matches(|c: char| "⏺•◆▸▹►▪■□●○-*>#".contains(c))
            .trim();
        if cleaned.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(cleaned);
    }
    // Collapse multiple spaces
    let mut result = String::with_capacity(out.len());
    let mut prev_space = false;
    for c in out.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Extract sample_rate from piper config JSON.
fn extract_sample_rate(config: &str) -> Option<u32> {
    let needle = "\"sample_rate\"";
    let pos = config.find(needle)?;
    let rest = &config[pos + needle.len()..];
    let colon = rest.find(':')?;
    let after_colon = rest[colon + 1..].trim_start();
    let num_end = after_colon.find(|c: char| !c.is_ascii_digit())?;
    after_colon[..num_end].parse().ok()
}
