//! Minimal blocking client for OpenAI-compatible chat completions endpoints.

use super::config::LlmConfig;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

/// An HTTP-level failure, carrying the status code when there was one.
struct RequestError {
    status: Option<u16>,
    message: String,
}

/// POST a chat completion request and return the assistant message content.
///
/// When the request carries a `response_format` and the endpoint rejects it
/// with a client error (some builds do not support guided decoding), the
/// request is retried once without the parameter.
pub fn chat_completion(config: &LlmConfig, body: &Value) -> Result<String, String> {
    let payload =
        serde_json::to_string(body).map_err(|e| format!("Failed to serialise request: {}", e))?;
    match send(config, &payload) {
        Ok(content) => Ok(content),
        Err(error) => {
            let can_retry_without_schema = config.schema_mode
                && body.get("response_format").is_some()
                && matches!(error.status, Some(400) | Some(422));
            if !can_retry_without_schema {
                return Err(error.message);
            }
            let mut retry_body = body.clone();
            if let Some(object) = retry_body.as_object_mut() {
                object.remove("response_format");
            }
            let retry_payload = serde_json::to_string(&retry_body)
                .map_err(|e| format!("Failed to serialise request: {}", e))?;
            send(config, &retry_payload).map_err(|retry_error| {
                format!(
                    "{}; retry without response_format also failed: {}",
                    error.message, retry_error.message
                )
            })
        }
    }
}

/// Send one request and extract `choices[0].message.content`.
fn send(config: &LlmConfig, payload: &str) -> Result<String, RequestError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(config.timeout_secs.max(1))))
        // Non-2xx responses are returned as regular responses so the error
        // body can be included in the message.
        .http_status_as_error(false)
        .build()
        .new_agent();

    let mut request = agent
        .post(config.chat_completions_url())
        .header("Content-Type", "application/json");
    if let Some(api_key) = &config.api_key {
        let bearer = format!("Bearer {}", api_key);
        request = request.header("Authorization", &bearer);
    }

    let response = request
        .send(payload.to_string())
        .map_err(|e| RequestError {
            status: None,
            message: format!("LLM request failed: {}", e),
        })?;

    let status = response.status().as_u16();
    let mut body = String::new();
    response
        .into_body()
        .into_reader()
        .read_to_string(&mut body)
        .map_err(|e| RequestError {
            status: Some(status),
            message: format!("Failed to read LLM response body: {}", e),
        })?;

    if !(200..300).contains(&status) {
        return Err(RequestError {
            status: Some(status),
            message: format!(
                "LLM endpoint returned HTTP {}: {}",
                status,
                truncate(&body, 200)
            ),
        });
    }

    extract_message_content(&body).ok_or_else(|| RequestError {
        status: None,
        message: format!(
            "LLM response has no choices[0].message.content: {}",
            truncate(&body, 200)
        ),
    })
}

/// Pull the assistant text out of a chat completion response body.
fn extract_message_content(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let content = value
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?;
    content.as_str().map(|s| s.to_string())
}

/// Clamp an error-reporting snippet to a readable length.
fn truncate(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock_server::MockServer;
    use serde_json::json;

    fn config_for(server: &MockServer) -> LlmConfig {
        LlmConfig::new(server.url.clone(), "mock-model")
    }

    fn completion_body(content: &str) -> String {
        json!({ "choices": [ { "message": { "role": "assistant", "content": content } } ] })
            .to_string()
    }

    #[test]
    fn test_chat_completion_returns_message_content() {
        let server = MockServer::start(vec![(200, completion_body("hello"))]);
        let body = json!({ "model": "mock-model", "messages": [] });
        let content = chat_completion(&config_for(&server), &body).expect("Expected success");
        assert_eq!(content, "hello");
    }

    #[test]
    fn test_chat_completion_retries_without_response_format_on_400() {
        let server = MockServer::start(vec![
            (
                400,
                json!({ "error": "response_format not supported" }).to_string(),
            ),
            (200, completion_body("retried ok")),
        ]);
        let body = json!({
            "model": "mock-model",
            "messages": [],
            "response_format": { "type": "json_schema" }
        });
        let content = chat_completion(&config_for(&server), &body).expect("Expected success");
        assert_eq!(content, "retried ok");

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("response_format"));
        assert!(!requests[1].contains("response_format"));
    }

    #[test]
    fn test_chat_completion_does_not_retry_on_500() {
        let server = MockServer::start(vec![(500, "boom".to_string())]);
        let body = json!({
            "model": "mock-model",
            "messages": [],
            "response_format": { "type": "json_schema" }
        });
        let error = chat_completion(&config_for(&server), &body)
            .expect_err("Expected failure after single attempt");
        assert!(error.contains("HTTP 500"));
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn test_chat_completion_surfaces_http_error_body() {
        let server = MockServer::start(vec![(401, json!({ "error": "bad key" }).to_string())]);
        let body = json!({ "model": "mock-model", "messages": [] });
        let error =
            chat_completion(&config_for(&server), &body).expect_err("Expected auth failure");
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("bad key"));
    }

    #[test]
    fn test_extract_message_content_rejects_malformed_body() {
        assert!(extract_message_content("not json").is_none());
        assert!(extract_message_content("{\"choices\": []}").is_none());
    }
}
