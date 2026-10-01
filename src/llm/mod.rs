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
/// Validation failures (numbers that do not reconcile) are hard errors.
pub fn llm_text_items_to_statement_data(
    config: &LlmConfig,
    items: &[TextItem],
) -> Result<StatementData, String> {
    llm_text_items_to_statement_data_inner(config, items, false)
}

/// Lenient variant: numbers that fail validation are returned alongside the
/// checker errors in `StatementData.errors`, for review-then-correct
/// workflows. Transport, schema and conversion failures are still errors.
pub fn llm_text_items_to_statement_data_lenient(
    config: &LlmConfig,
    items: &[TextItem],
) -> Result<StatementData, String> {
    llm_text_items_to_statement_data_inner(config, items, true)
}

fn llm_text_items_to_statement_data_inner(
    config: &LlmConfig,
    items: &[TextItem],
    lenient: bool,
) -> Result<StatementData, String> {
    if items.is_empty() {
        return Err(
            "LLM text fallback requires extracted text items, but none were found. \
For scanned statements, supply page images via the vision path instead."
                .to_string(),
        );
    }
    let messages = json!([
        { "role": "system", "content": prompt::SYSTEM_PROMPT },
        { "role": "user", "content": prompt::text_items_to_prompt(items) },
    ]);
    llm_request_to_statement_data(config, messages, lenient)
}

/// Extract statement data from page images via a local vision LLM endpoint.
///
/// Suitable for scanned statements. Images are supplied by the caller (the
/// Rust core deliberately does not rasterise PDFs); all pages are sent in a
/// single request, in order. Validation failures are hard errors.
pub fn llm_images_to_statement_data(
    config: &LlmConfig,
    images: &[ImageInput],
) -> Result<StatementData, String> {
    llm_images_to_statement_data_inner(config, images, false)
}

/// Lenient variant of the vision path; see
/// [`llm_text_items_to_statement_data_lenient`].
pub fn llm_images_to_statement_data_lenient(
    config: &LlmConfig,
    images: &[ImageInput],
) -> Result<StatementData, String> {
    llm_images_to_statement_data_inner(config, images, true)
}

fn llm_images_to_statement_data_inner(
    config: &LlmConfig,
    images: &[ImageInput],
    lenient: bool,
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
    let messages = json!([
        { "role": "system", "content": prompt::SYSTEM_PROMPT },
        { "role": "user", "content": Value::Array(content) },
    ]);
    llm_request_to_statement_data(config, messages, lenient)
}

/// Assemble the chat completion request body for the given conversation.
fn build_request_body(config: &LlmConfig, messages: Value) -> Value {
    let mut body = json!({
        "model": config.model,
        "temperature": config.temperature,
        "messages": messages,
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
    if config.no_think {
        // Qwen3-style thinking models burn the token budget reasoning before
        // answering; llama-server exposes the chat-template switch to stop it.
        body["chat_template_kwargs"] = json!({ "enable_thinking": false });
    }
    body
}

/// Run the extraction conversation end to end: call the endpoint, parse the
/// response into an [`schema::LlmStatement`], convert, then fix and check.
///
/// Self-correction loop: when an attempt fails the balance validation and
/// [`LlmConfig::correction_rounds`] remains, the checker errors are fed back
/// to the model (assistant answer + named errors) for another attempt. The
/// attempt with the fewest errors is kept. In strict mode exhausting the
/// rounds is a hard error; in lenient mode the best attempt is returned with
/// `errors` populated for review workflows.
fn llm_request_to_statement_data(
    config: &LlmConfig,
    mut messages: Value,
    lenient: bool,
) -> Result<StatementData, String> {
    let attempts = config.correction_rounds as usize + 1;
    let mut best: Option<(StatementData, String)> = None;

    for attempt in 0..attempts {
        let body = build_request_body(config, messages.clone());
        let content = client::chat_completion(config, &body)?;
        let json_str = extract_json(&content)?.to_string();
        let statement: schema::LlmStatement = serde_json::from_str(&json_str).map_err(|e| {
            format!(
                "LLM returned a response that does not match the statement schema: {}",
                e
            )
        })?;
        let provenance = format!("{}/{}", PROVENANCE_PREFIX, config.model);
        let mut data = statement.to_statement_data(&provenance)?;

        // Identical post-processing to the rules engine: fixers backfill
        // implicit balances/dates and correct amount signs, then the checkers
        // verify that the numbers reconcile with the opening and closing
        // balances.
        fix_statement_data(&mut data);
        check_statement_data(&mut data);

        if data.errors.is_empty() {
            return Ok(data);
        }
        let error_count = data.errors.len();
        let errors = data.errors.clone();
        if best
            .as_ref()
            .is_none_or(|(b, _)| error_count < b.errors.len())
        {
            best = Some((data, json_str.clone()));
        }

        if attempt + 1 < attempts {
            // Feed the failing attempt back: assistant answer, then the
            // checker errors naming the offending rows.
            if let Some(conversation) = messages.as_array_mut() {
                conversation.push(json!({ "role": "assistant", "content": content }));
                conversation.push(json!({
                    "role": "user",
                    "content": prompt::correction_prompt(&errors, &json_str),
                }));
            }
        }
    }

    let (data, _) = best.ok_or_else(|| "LLM extraction produced no result".to_string())?;
    if lenient {
        return Ok(data);
    }
    Err(format!(
        "LLM extraction failed validation after {} correction round(s) (numbers do not \
reconcile): {}",
        config.correction_rounds,
        data.errors.join("; ")
    ))
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
        // Default correction_rounds is 1: both the first attempt and the
        // correction round return the bad extraction.
        let server = MockServer::start(vec![
            (200, completion_body(&bad_json.to_string())),
            (200, completion_body(&bad_json.to_string())),
        ]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let error = llm_text_items_to_statement_data(&config, &sample_items())
            .expect_err("Expected validation failure");
        assert!(error.contains("failed validation"), "got: {}", error);
        assert!(error.contains("balance mismatch"), "got: {}", error);
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn test_correction_loop_recovers_on_second_attempt() {
        let bad_json = json!({
            "account_number": "1234 5678 9123 4567",
            "start_date": "2025-01-01",
            "opening_balance": 1000.0,
            "closing_balance": 800.0,
            "transactions": [
                { "date": "2025-01-02", "description": "Payment", "amount": -150.0, "balance": 850.0 },
                { "date": "2025-01-03", "description": "Deposit", "amount": 50.0, "balance": 900.0 }
            ]
        });
        // First attempt fails validation; the correction round sees the
        // checker errors and returns a reconciling extraction.
        let server = MockServer::start(vec![
            (200, completion_body(&bad_json.to_string())),
            (200, completion_body(&valid_statement_json())),
        ]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let data = llm_text_items_to_statement_data(&config, &sample_items())
            .expect("Expected the correction round to reconcile");
        assert!(data.errors.is_empty());
        assert_eq!(data.proto_transactions.len(), 2);

        // The second request continues the conversation: assistant answer,
        // then a user message quoting the checker errors.
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].contains("\"role\":\"assistant\""));
        assert!(requests[1].contains("failed arithmetic validation"));
        assert!(requests[1].contains("balance mismatch"));
    }

    #[test]
    fn test_lenient_mode_returns_rows_with_errors() {
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
        let server = MockServer::start(vec![
            (200, completion_body(&bad_json.to_string())),
            (200, completion_body(&bad_json.to_string())),
        ]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        // Same extraction as the strict test above, but lenient: the rows come
        // back with the checker errors attached for review workflows (both
        // attempts fail, so the best attempt is returned after the loop).
        let data = llm_text_items_to_statement_data_lenient(&config, &sample_items())
            .expect("Expected lenient extraction to succeed");
        assert_eq!(data.key.as_deref(), Some("llm/mock-model"));
        assert_eq!(data.proto_transactions.len(), 2);
        assert!(!data.errors.is_empty());
        assert!(
            data.errors[0].contains("balance mismatch"),
            "got: {:?}",
            data.errors
        );

        // A good extraction in lenient mode is simply error-free.
        let server = MockServer::start(vec![(200, completion_body(&valid_statement_json()))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");
        let data = llm_text_items_to_statement_data_lenient(&config, &sample_items())
            .expect("Expected lenient extraction to succeed");
        assert!(data.errors.is_empty());
    }

    #[test]
    fn test_lenient_vision_path_returns_rows_with_errors() {
        let bad_json = json!({
            "account_number": "1234",
            "start_date": "2025-01-01",
            "opening_balance": 1000.0,
            "closing_balance": 500.0,
            "transactions": [
                { "date": "2025-01-02", "description": "Payment", "amount": -100.0, "balance": 900.0 }
            ]
        });
        let server = MockServer::start(vec![
            (200, completion_body(&bad_json.to_string())),
            (200, completion_body(&bad_json.to_string())),
        ]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");

        let data = llm_images_to_statement_data_lenient(
            &config,
            &[ImageInput::new("aGVsbG8=", "image/png")],
        )
        .expect("Expected lenient vision extraction to succeed");
        assert_eq!(data.proto_transactions.len(), 1);
        assert!(!data.errors.is_empty());
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
    fn test_no_think_sends_chat_template_kwargs() {
        let server = MockServer::start(vec![(200, completion_body(&valid_statement_json()))]);
        let mut config = LlmConfig::new(server.url.clone(), "mock-model");
        config.no_think = true;

        llm_text_items_to_statement_data(&config, &sample_items())
            .expect("Expected extraction to succeed");

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("chat_template_kwargs"));
        assert!(requests[0].contains("enable_thinking"));

        // Without the flag the field must be absent.
        let server = MockServer::start(vec![(200, completion_body(&valid_statement_json()))]);
        let config = LlmConfig::new(server.url.clone(), "mock-model");
        llm_text_items_to_statement_data(&config, &sample_items())
            .expect("Expected extraction to succeed");
        assert!(!server.requests()[0].contains("chat_template_kwargs"));
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
