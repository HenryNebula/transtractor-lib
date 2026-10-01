//! Optional local-LLM fallback extraction backend.
//!
//! When the deterministic rules engine cannot parse a statement (unknown bank
//! format, or a scanned statement with no usable text layer), this backend
//! asks an OpenAI-compatible local inference endpoint — llama.cpp
//! `llama-server`, vLLM or Ollama — to extract the statement, then runs the
//! result through the standard fixer and checker pipeline. Statements whose
//! numbers do not reconcile with the opening and closing balances are
//! rejected, so no unverified figures are ever returned.
//!
//! Enable with the `llm` cargo feature. Nothing is sent anywhere unless a
//! [`LlmConfig`] is explicitly configured.

pub mod client;
pub mod config;
pub mod prompt;
pub mod schema;

#[cfg(test)]
pub(crate) mod mock_server;

use crate::checkers::check_statement_data;
use crate::fixers::fix_statement_data;
use crate::structs::{StatementData, TextItem};
use serde_json::{Value, json};

pub use config::{ImageInput, LlmConfig};

/// Provenance prefix applied to the key of LLM-extracted statements
/// (the final key is `llm/<model>`).
pub const PROVENANCE_PREFIX: &str = "llm";

/// Extract statement data from text items via a local LLM endpoint.
///
/// Suitable for digital statements where the PDF has a text layer. The
/// extracted lines are rendered as a page-fenced plain-text prompt.
pub fn llm_text_items_to_statement_data(
    config: &LlmConfig,
    items: &[TextItem],
) -> Result<StatementData, String> {
    if items.is_empty() {
        return Err(
            "LLM text fallback requires extracted text items, but none were found. \
For scanned statements, supply page images via the vision path instead."
                .to_string(),
        );
    }
    let user_content = json!([{
        "type": "text",
        "text": prompt::text_items_to_prompt(items),
    }]);
    let body = build_request_body(config, user_content);
    llm_request_to_statement_data(config, body)
}

/// Extract statement data from page images via a local vision LLM endpoint.
///
/// Suitable for scanned statements. Images are supplied by the caller (the
/// Rust core deliberately does not rasterise PDFs); all pages are sent in a
/// single request, in order.
pub fn llm_images_to_statement_data(
    config: &LlmConfig,
    images: &[ImageInput],
) -> Result<StatementData, String> {
    if images.is_empty() {
        return Err("LLM vision fallback requires at least one page image.".to_string());
    }
    let mut content: Vec<Value> = images
        .iter()
        .map(|image| {
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", image.mime, image.b64) },
            })
        })
        .collect();
    content.push(json!({
        "type": "text",
        "text": prompt::vision_prompt(images.len()),
    }));
    let body = build_request_body(config, Value::Array(content));
    llm_request_to_statement_data(config, body)
}

/// Assemble the chat completion request body for either extraction path.
fn build_request_body(config: &LlmConfig, user_content: Value) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": config.temperature,
        "messages": [
            { "role": "system", "content": prompt::SYSTEM_PROMPT },
            { "role": "user", "content": user_content },
        ],
    });
    if config.schema_mode {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "bank_statement",
                "strict": true,
                "schema": schema::llm_response_json_schema(),
            },
        });
    }
    body
}

/// Run one extraction request end to end: call the endpoint, parse the
/// response into an [`schema::LlmStatement`], convert, then fix and check.
fn llm_request_to_statement_data(config: &LlmConfig, body: Value) -> Result<StatementData, String> {
    let content = client::chat_completion(config, &body)?;
    let json_str = extract_json(&content)?;
    let statement: schema::LlmStatement = serde_json::from_str(json_str).map_err(|e| {
        format!(
            "LLM returned a response that does not match the statement schema: {}",
            e
        )
    })?;
    let provenance = format!("{}/{}", PROVENANCE_PREFIX, config.model);
    let mut data = statement.to_statement_data(&provenance)?;

    // Identical post-processing to the rules engine: fixers backfill implicit
    // balances/dates and correct amount signs, then the checkers verify that
    // the numbers reconcile with the opening and closing balances.
    fix_statement_data(&mut data);
    check_statement_data(&mut data);

    if !data.errors.is_empty() {
        return Err(format!(
            "LLM extraction failed validation (numbers do not reconcile): {}",
            data.errors.join("; ")
        ));
    }
    Ok(data)
}

/// Extract the first balanced JSON object from an LLM response, tolerating
/// markdown code fences and surrounding prose.
pub fn extract_json(content: &str) -> Result<&str, String> {
    let trimmed = content.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| "LLM response contains no JSON object".to_string())?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in trimmed.as_bytes().iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&trimmed[start..=index]);
                }
            }
            _ => {}
        }
    }
    Err("LLM response contains an unbalanced JSON object".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock_server::MockServer;
    use crate::structs::TextItem;

    /// A valid canned statement JSON string (as assistant message content).
    fn valid_statement_json() -> String {
        json!({
            "account_number": "1234 5678 9123 4567",
            "start_date": "2025-01-01",
            "opening_balance": 1000.0,
            "closing_balance": 900.0,
            "transactions": [
                { "date": "2025-01-02", "description": "Payment", "amount": -150.0, "balance": 850.0 },
                { "date": "2025-01-03", "description": "Deposit", "amount": 50.0 }
            ]
        })
        .to_string()
    }

    fn completion_body(content: &str) -> String {
        json!({ "choices": [ { "message": { "role": "assistant", "content": content } } ] })
            .to_string()
    }

    fn sample_items() -> Vec<TextItem> {
        vec![
            TextItem::new("Opening".to_string(), 0, 10, 20, 30, 0),
            TextItem::new("balance:".to_string(), 60, 10, 80, 30, 0),
            TextItem::new("1000.00".to_string(), 200, 10, 260, 30, 0),
        ]
    }

    #[test]
    fn test_extract_json_plain_object() {
        assert_eq!(extract_json(r#"{"a": 1}"#).unwrap(), r#"{"a": 1}"#);
    }

    #[test]
    fn test_extract_json_inside_markdown_fence() {
        let content = "Here is the extraction:\n```json\n{\"a\": {\"b\": 2}}\n```\nDone.";
        assert_eq!(extract_json(content).unwrap(), r#"{"a": {"b": 2}}"#);
    }

    #[test]
    fn test_extract_json_with_braces_inside_strings() {
        let content = r#"prose {"desc": "paid {not json}", "n": 1} trailing"#;
        assert_eq!(
            extract_json(content).unwrap(),
            r#"{"desc": "paid {not json}", "n": 1}"#
        );
    }

    #[test]
    fn test_extract_json_rejects_content_without_object() {
        assert!(extract_json("no json here").is_err());
        assert!(extract_json(r#"{"unbalanced": 1"#).is_err());
    }

    #[test]
    fn test_text_path_happy_path_runs_fixers_and_checkers() {
        let server = MockServer::start(vec![(200, completion_body(&valid_statement_json()))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let data = llm_text_items_to_statement_data(&config, &sample_items())
            .expect("Expected extraction to succeed");

        assert_eq!(data.key.as_deref(), Some("llm/mock-model"));
        assert_eq!(data.opening_balance, Some(1000.0));
        assert_eq!(data.closing_balance, Some(900.0));
        assert!(data.errors.is_empty());
        // The second transaction had no balance; the implicit-balance fixer
        // must have backfilled it: 850 + 50 = 900.
        assert_eq!(data.proto_transactions[1].balance, Some(900.0));

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("/v1/chat/completions"));
        assert!(requests[0].contains("Opening balance: 1000.00"));
        assert!(requests[0].contains("json_schema"));
    }

    #[test]
    fn test_text_path_rejects_non_reconciling_numbers() {
        let bad_json = json!({
            "account_number": "1234 5678 9123 4567",
            "start_date": "2025-01-01",
            "opening_balance": 1000.0,
            "closing_balance": 800.0, // Wrong: 1000 - 150 + 50 = 900
            "transactions": [
                { "date": "2025-01-02", "description": "Payment", "amount": -150.0, "balance": 850.0 },
                { "date": "2025-01-03", "description": "Deposit", "amount": 50.0, "balance": 900.0 }
            ]
        });
        let server = MockServer::start(vec![(200, completion_body(&bad_json.to_string()))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let error = llm_text_items_to_statement_data(&config, &sample_items())
            .expect_err("Expected validation failure");
        assert!(error.contains("failed validation"), "got: {}", error);
        assert!(error.contains("balance mismatch"), "got: {}", error);
    }

    #[test]
    fn test_text_path_rejects_schema_mismatch() {
        let content = r#"{"account_number": "123", "transactions": "not-a-list"}"#;
        let server = MockServer::start(vec![(200, completion_body(content))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let error = llm_text_items_to_statement_data(&config, &sample_items())
            .expect_err("Expected schema failure");
        assert!(
            error.contains("does not match the statement schema"),
            "got: {}",
            error
        );
    }

    #[test]
    fn test_text_path_requires_items() {
        let config = LlmConfig::new("http://127.0.0.1:9/v1", "mock-model");
        let error = llm_text_items_to_statement_data(&config, &[])
            .expect_err("Expected empty-items failure");
        assert!(error.contains("none were found"));
    }

    #[test]
    fn test_vision_path_happy_path() {
        let server = MockServer::start(vec![(200, completion_body(&valid_statement_json()))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");
        let images = vec![ImageInput::new("aGVsbG8=", "image/png")];

        let data =
            llm_images_to_statement_data(&config, &images).expect("Expected extraction success");
        assert_eq!(data.key.as_deref(), Some("llm/mock-model"));
        assert!(data.errors.is_empty());

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("data:image/png;base64,aGVsbG8="));
    }

    #[test]
    fn test_vision_path_requires_images() {
        let config = LlmConfig::new("http://127.0.0.1:9/v1", "mock-model");
        assert!(llm_images_to_statement_data(&config, &[]).is_err());
    }

    #[test]
    fn test_markdown_fenced_response_is_tolerated() {
        let fenced = format!("```json\n{}\n```", valid_statement_json());
        let server = MockServer::start(vec![(200, completion_body(&fenced))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let data = llm_text_items_to_statement_data(&config, &sample_items())
            .expect("Expected fenced response to parse");
        assert!(data.errors.is_empty());
    }
}
