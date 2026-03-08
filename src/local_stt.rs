use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::io::Cursor;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_SAMPLE_RATE: u32 = 16000;

/// Local speech-to-text engine using whisper.cpp.
pub struct LocalWhisper {
    ctx: WhisperContext,
}

impl LocalWhisper {
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let path_str = model_path.to_str().ok_or("Model path is not valid UTF-8")?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| format!("Failed to load whisper model: {e}"))?;
        Ok(Self { ctx })
    }

    pub fn transcribe(&self, wav_data: &[u8], device_sample_rate: u32) -> Result<String, String> {
        // Parse WAV to f32 samples
        let cursor = Cursor::new(wav_data);
        let mut reader =
            hound::WavReader::new(cursor).map_err(|e| format!("WAV parse error: {e}"))?;
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap_or(0) as f32 / i16::MAX as f32)
            .collect();

        if samples.is_empty() {
            return Err("No audio samples in WAV".into());
        }

        // Resample to 16kHz if needed
        let audio_16k = if device_sample_rate == WHISPER_SAMPLE_RATE {
            samples
        } else {
            resample(&samples, device_sample_rate, WHISPER_SAMPLE_RATE)?
        };

        // Run whisper inference
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {e}"))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, &audio_16k)
            .map_err(|e| format!("Whisper inference failed: {e}"))?;

        // Collect transcription text
        let mut text = String::new();
        for segment in state.as_iter() {
            if let Ok(s) = segment.to_str() {
                text.push_str(s);
            }
        }

        Ok(text.trim().to_string())
    }
}

fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, String> {
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1)
        .map_err(|e| format!("Resampler init error: {e}"))?;

    let mut output = Vec::with_capacity((input.len() as f64 * ratio) as usize + 1024);
    let mut pos = 0;

    while pos + chunk_size <= input.len() {
        let chunk = &input[pos..pos + chunk_size];
        let result = resampler
            .process(&[chunk], None)
            .map_err(|e| format!("Resample error: {e}"))?;
        output.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    // Handle remaining samples
    if pos < input.len() {
        let remaining = &input[pos..];
        let result = resampler
            .process_partial(Some(&[remaining]), None)
            .map_err(|e| format!("Resample error: {e}"))?;
        output.extend_from_slice(&result[0]);
    }

    Ok(output)
}
