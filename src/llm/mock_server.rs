//! Test-only minimal HTTP server emulating an OpenAI-compatible endpoint.
//!
//! Serves a fixed sequence of canned `(status, body)` responses, one per
//! `POST /v1/chat/completions` request, and records the raw requests so
//! tests can assert on their contents.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A mock chat-completions endpoint bound to an ephemeral local port.
pub struct MockServer {
    /// Base URL including the `/v1` prefix, ready for [`crate::llm::config::LlmConfig`].
    pub url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    /// Start a server that answers each accepted request with the next
    /// canned response. The server thread exits once all responses are used.
    pub fn start(responses: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind mock server");
        let port = listener
            .local_addr()
            .expect("Failed to read mock server address")
            .port();
        let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);

        std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener
                    .accept()
                    .expect("Mock server failed to accept a connection");
                let request = read_request(&mut stream);
                recorded
                    .lock()
                    .expect("Mock server request lock poisoned")
                    .push(request);
                let reason = if (200..300).contains(&status) {
                    "OK"
                } else {
                    "Error"
                };
                let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    reason,
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("Mock server failed to write response");
            }
        });

        Self {
            url: format!("http://127.0.0.1:{}/v1", port),
            requests,
        }
    }

    /// The raw requests received so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("Mock server request lock poisoned")
            .clone()
    }
}

/// Read one full HTTP request (headers plus Content-Length body).
fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("Failed to set read timeout");

    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    // Read until the end of the headers.
    let header_end = loop {
        let n = stream
            .read(&mut chunk)
            .expect("Mock server failed to read request");
        if n == 0 {
            panic!("Mock server client closed connection before sending headers");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
    };

    // Read the remaining body if Content-Length says there is one.
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);

    while buf.len() < header_end + content_length {
        let n = stream
            .read(&mut chunk)
            .expect("Mock server failed to read request body");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    String::from_utf8_lossy(&buf).into_owned()
}

/// Find the first occurrence of `pattern` in `haystack`.
fn find_subsequence(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() || haystack.len() < pattern.len() {
        return None;
    }
    haystack
        .windows(pattern.len())
        .position(|window| window == pattern)
}
