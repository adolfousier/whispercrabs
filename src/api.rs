use reqwest::multipart;

/// Send WAV audio to an OpenAI-compatible transcription endpoint and return the text.
pub async fn transcribe(
    base_url: &str,
    api_key: &str,
    model: &str,
    wav_data: Vec<u8>,
) -> Result<String, String> {
    let url = format!("{}/audio/transcriptions", base_url.trim_end_matches('/'));

    let file_part = multipart::Part::bytes(wav_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("Multipart error: {e}"))?;

    let form = multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "json")
        .part("file", file_part);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("API error {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;

    json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("No 'text' field in response: {json}"))
}
