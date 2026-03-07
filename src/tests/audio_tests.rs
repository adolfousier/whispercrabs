use std::io::Cursor;

/// Helper: build a valid WAV buffer from f32 samples at given sample rate
fn encode_wav(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut buf, spec).unwrap();
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        writer
            .write_sample((clamped * i16::MAX as f32) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
    buf.into_inner()
}

#[test]
fn wav_encode_produces_valid_wav() {
    let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0).sin()).collect();
    let wav = encode_wav(&samples, 44100, 1);

    // Verify WAV header magic
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");

    // Parse back and verify sample count
    let reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(reader.spec().sample_rate, 44100);
    assert_eq!(reader.len(), 1000);
}

#[test]
fn wav_encode_stereo() {
    // 500 stereo frames = 1000 samples
    let samples: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.001).sin()).collect();
    let wav = encode_wav(&samples, 48000, 2);

    let reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 48000);
    assert_eq!(reader.len(), 1000);
}

#[test]
fn wav_clamps_out_of_range_samples() {
    let samples = vec![-2.0, -1.5, 0.0, 1.5, 2.0];
    let wav = encode_wav(&samples, 16000, 1);

    let reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
    let decoded: Vec<i16> = reader.into_samples::<i16>().map(|s| s.unwrap()).collect();

    // Out-of-range values should be clamped to i16 min/max
    assert_eq!(decoded[0], i16::MIN + 1); // -1.0 clamped
    assert_eq!(decoded[1], i16::MIN + 1); // -1.5 clamped to -1.0
    assert_eq!(decoded[2], 0);
    assert_eq!(decoded[3], i16::MAX); // 1.5 clamped to 1.0
    assert_eq!(decoded[4], i16::MAX); // 2.0 clamped to 1.0
}

#[test]
fn mono_downmix_averages_channels() {
    // Simulate stereo: L=1.0, R=-1.0 should average to 0.0
    let stereo = vec![1.0f32, -1.0, 0.5, 0.5, -0.5, 0.3];
    let channels: u16 = 2;

    let mono: Vec<f32> = stereo
        .chunks(channels as usize)
        .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
        .collect();

    assert_eq!(mono.len(), 3);
    assert!((mono[0] - 0.0).abs() < f32::EPSILON);
    assert!((mono[1] - 0.5).abs() < f32::EPSILON);
    assert!((mono[2] - (-0.1)).abs() < 0.001);
}

#[test]
fn mono_passthrough_single_channel() {
    let samples = vec![0.1f32, 0.2, 0.3];
    let channels: u16 = 1;

    let mono: Vec<f32> = if channels > 1 {
        samples
            .chunks(channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
            .collect()
    } else {
        samples.clone()
    };

    assert_eq!(mono, samples);
}

#[test]
fn i16_to_f32_conversion_roundtrip() {
    // Test the conversion used in audio.rs for i16 input streams
    let i16_samples: Vec<i16> = vec![0, i16::MAX, i16::MIN, 16384, -16384];
    let floats: Vec<f32> = i16_samples
        .iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect();

    assert!((floats[0] - 0.0).abs() < 0.001);
    assert!((floats[1] - 1.0).abs() < 0.001);
    assert!(floats[2] < -0.99);
    assert!((floats[3] - 0.5).abs() < 0.01);
    assert!((floats[4] - (-0.5)).abs() < 0.01);
}

#[test]
fn u16_to_f32_conversion() {
    // Test the conversion used in audio.rs for u16 input streams
    let u16_samples: Vec<u16> = vec![0, u16::MAX, u16::MAX / 2];
    let floats: Vec<f32> = u16_samples
        .iter()
        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
        .collect();

    assert!((floats[0] - (-1.0)).abs() < 0.001);
    assert!((floats[1] - 1.0).abs() < 0.001);
    assert!(floats[2].abs() < 0.01); // midpoint ~0
}
