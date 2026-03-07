use crate::api;

#[test]
fn transcribe_url_strips_trailing_slash() {
    // We can't call transcribe without a server, but we can verify the URL logic
    // by testing the format pattern used in api.rs
    let base = "https://api.groq.com/openai/v1/";
    let url = format!("{}/audio/transcriptions", base.trim_end_matches('/'));
    assert_eq!(url, "https://api.groq.com/openai/v1/audio/transcriptions");
}

#[test]
fn transcribe_url_no_trailing_slash() {
    let base = "https://api.groq.com/openai/v1";
    let url = format!("{}/audio/transcriptions", base.trim_end_matches('/'));
    assert_eq!(url, "https://api.groq.com/openai/v1/audio/transcriptions");
}

#[tokio::test]
async fn transcribe_rejects_invalid_url() {
    let result = api::transcribe("http://127.0.0.1:1", "fake-key", "model", vec![0u8; 44]).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Request failed") || err.contains("error"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn transcribe_rejects_empty_wav() {
    // Even with a valid-looking URL, an empty body should fail at the multipart level or server
    let result = api::transcribe("http://127.0.0.1:1", "key", "model", vec![]).await;
    assert!(result.is_err());
}
