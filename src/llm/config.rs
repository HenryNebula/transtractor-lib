//! Configuration for the optional local LLM fallback extraction backend.

/// Configuration for an OpenAI-compatible local inference endpoint.
///
/// The endpoint must expose `POST {base_url}/chat/completions`. llama.cpp
/// `llama-server`, vLLM and Ollama all speak this API, so any of them can be
/// used. Only plain HTTP is supported (the feature targets local endpoints,
/// keeping the dependency tree free of TLS stacks).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmConfig {
    /// Base URL of the endpoint including the `/v1` prefix,
    /// e.g. `http://127.0.0.1:8080/v1`
    pub base_url: String,
    /// Model name as expected by the endpoint, e.g. `qwen2.5-vl-7b-instruct`
    pub model: String,
    /// Optional API key sent as a Bearer token (llama-server `--api-key`)
    pub api_key: Option<String>,
    /// Global request timeout in seconds; local inference on large statements
    /// can take well over a minute
    pub timeout_secs: u64,
    /// Sampling temperature; 0.0 for deterministic extraction
    pub temperature: f64,
    /// Attempt `response_format: json_schema` guided decoding first, retrying
    /// once without it if the endpoint rejects the parameter
    pub schema_mode: bool,
}

impl LlmConfig {
    /// Build a config for the given endpoint and model with default settings
    /// (no API key, 120s timeout, temperature 0, schema mode on).
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: normalise_base_url(&base_url.into()),
            model: model.into(),
            api_key: None,
            timeout_secs: 120,
            temperature: 0.0,
            schema_mode: true,
        }
    }

    /// Read a config from the `TRANSTRACTOR_LLM_URL`, `TRANSTRACTOR_LLM_MODEL`
    /// and (optionally) `TRANSTRACTOR_LLM_API_KEY` environment variables.
    /// Returns `None` unless both URL and model are set and non-empty.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("TRANSTRACTOR_LLM_URL").ok()?;
        let model = std::env::var("TRANSTRACTOR_LLM_MODEL").ok()?;
        if base_url.trim().is_empty() || model.trim().is_empty() {
            return None;
        }
        let mut config = Self::new(base_url, model);
        if let Ok(key) = std::env::var("TRANSTRACTOR_LLM_API_KEY")
            && !key.trim().is_empty()
        {
            config.api_key = Some(key);
        }
        Some(config)
    }

    /// Full chat completions endpoint URL.
    pub fn chat_completions_url(&self) -> String {
        if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self::new("http://127.0.0.1:8080/v1", "local-model")
    }
}

/// Trim whitespace and a trailing slash so endpoint URLs compose predictably.
fn normalise_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// A base64-encoded page image supplied by the caller for vision models.
#[derive(Clone, Debug)]
pub struct ImageInput {
    /// Base64-encoded image bytes
    pub b64: String,
    /// MIME type of the encoded image, e.g. `image/png` or `image/jpeg`
    pub mime: String,
}

impl ImageInput {
    /// Create a new image input from base64 bytes and a MIME type.
    pub fn new(b64: impl Into<String>, mime: impl Into<String>) -> Self {
        Self {
            b64: b64.into(),
            mime: mime.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_normalises_base_url() {
        let config = LlmConfig::new("http://127.0.0.1:8080/v1/", "test-model");
        assert_eq!(config.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(config.timeout_secs, 120);
        assert_eq!(config.temperature, 0.0);
        assert!(config.schema_mode);
        assert_eq!(config.api_key, None);
    }

    #[test]
    fn test_chat_completions_url_composition() {
        let config = LlmConfig::new("http://127.0.0.1:8080/v1", "test-model");
        assert_eq!(
            config.chat_completions_url(),
            "http://127.0.0.1:8080/v1/chat/completions"
        );

        // A full endpoint URL is used as-is rather than double-suffixed.
        let config = LlmConfig::new("http://localhost:1234/v1/chat/completions", "test-model");
        assert_eq!(
            config.chat_completions_url(),
            "http://localhost:1234/v1/chat/completions"
        );
    }
}
