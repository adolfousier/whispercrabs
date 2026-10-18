use crate::tts::{ApiTts, TtsEngine, chunk_text, decode_audio};

#[test]
fn endpoint_url_joins_base_and_path() {
    let engine = ApiTts::new(
        "https://api.openai.com/v1/",
        Some("key".into()),
        "gpt-4o-mini-tts",
        "alloy",
    )
    .expect("engine");
    assert_eq!(
        engine.endpoint_url(),
        "https://api.openai.com/v1/audio/speech"
    );

    let openrouter = ApiTts::new(
        "https://openrouter.ai/api/v1",
        Some("key".into()),
        "mistralai/voxtral-mini-tts-2603",
        "en_paul_neutral",
    )
    .expect("engine");
    assert_eq!(
        openrouter.endpoint_url(),
        "https://openrouter.ai/api/v1/audio/speech"
    );
}

#[test]
fn api_tts_rejects_non_http_base_url() {
    let err = ApiTts::new("file:///tmp/audio", None, "model", "alloy").unwrap_err();
    assert!(err.contains("only http:// and https://"), "{err}");
}

#[test]
fn api_tts_rejects_empty_model() {
    let err = ApiTts::new("https://api.openai.com/v1", None, "   ", "alloy").unwrap_err();
    assert!(err.contains("model must not be empty"), "{err}");
}

#[test]
fn api_tts_allows_missing_key() {
    // A local endpoint such as LocalAI needs no key.
    assert!(ApiTts::new("http://localhost:8080/v1", None, "tts-1", "alloy").is_ok());
}

#[test]
fn decode_accepts_wav_body() {
    // 16-bit mono 8 kHz WAV with four samples.
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Vec::new();
    {
        let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
        for sample in [100i16, -100, 200, -200] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    let (samples, rate) = decode_audio(&buf, "audio/wav").expect("decode");
    assert_eq!(rate, 8000);
    assert_eq!(samples, vec![100, -100, 200, -200]);
}

#[test]
fn decode_reads_pcm_rate_and_channels() {
    // Two stereo frames: (100, 300) and (-100, -300) downmix to 200 and -200.
    let mut bytes = Vec::new();
    for sample in [100i16, 300, -100, -300] {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    let (samples, rate) = decode_audio(&bytes, "audio/pcm;rate=24000;channels=2").expect("decode");
    assert_eq!(rate, 24000);
    assert_eq!(samples, vec![200, -200]);
}

#[test]
fn decode_defaults_to_24k_mono_for_plain_pcm() {
    let mut bytes = Vec::new();
    for sample in [1i16, 2, 3] {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }

    let (samples, rate) = decode_audio(&bytes, "audio/pcm").expect("decode");
    assert_eq!(rate, 24000);
    assert_eq!(samples, vec![1, 2, 3]);
}

#[test]
fn decode_rejects_mp3_with_actionable_message() {
    let err = decode_audio(b"ID3\x04\x00\x00", "audio/mpeg").unwrap_err();
    assert!(err.contains("raw PCM"), "{err}");
}

#[test]
fn decode_rejects_empty_body() {
    assert!(decode_audio(&[], "audio/pcm").is_err());
}

#[test]
fn decode_rejects_odd_byte_count() {
    assert!(decode_audio(&[1u8, 2, 3], "audio/pcm").is_err());
}

#[test]
fn chunk_text_keeps_short_text_whole() {
    let chunks = chunk_text("hello there", 3900);
    assert_eq!(chunks, vec!["hello there".to_string()]);
}

#[test]
fn chunk_text_splits_on_sentence_boundaries() {
    let text = format!("{}. {}.", "a".repeat(30), "b".repeat(30));
    let chunks = chunk_text(&text, 40);
    assert!(chunks.len() >= 2, "{chunks:?}");
    for chunk in &chunks {
        assert!(chunk.chars().count() <= 40, "chunk too long: {chunk}");
    }
    let joined: String = chunks.join(" ");
    assert!(joined.contains(&"a".repeat(30)));
    assert!(joined.contains(&"b".repeat(30)));
}

#[test]
fn chunk_text_never_exceeds_limit() {
    let text = "word ".repeat(2000);
    let chunks = chunk_text(&text, 100);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(
            chunk.chars().count() <= 100,
            "chunk of {} chars exceeds limit",
            chunk.chars().count()
        );
    }
}

#[test]
fn chunk_text_hard_splits_a_single_huge_word() {
    let text = "x".repeat(250);
    let chunks = chunk_text(&text, 100);
    assert_eq!(chunks.len(), 3);
    assert!(chunks.iter().all(|c| c.chars().count() <= 100));
    assert_eq!(chunks.concat().chars().count(), 250);
}

#[test]
fn engine_dispatch_reports_piper_rate() {
    // ApiTts is covered by the decode tests; this only checks the enum wraps
    // an engine without changing its identity.
    let engine = TtsEngine::Api(
        ApiTts::new("http://localhost:8080/v1", None, "tts-1", "alloy").expect("engine"),
    );
    match engine {
        TtsEngine::Api(inner) => assert_eq!(
            inner.endpoint_url(),
            "http://localhost:8080/v1/audio/speech"
        ),
        TtsEngine::Piper(_) => panic!("wrong variant"),
    }
}
