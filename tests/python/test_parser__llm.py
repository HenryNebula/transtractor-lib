"""Tests for the local LLM fallback extraction backend.

Uses a stdlib-only mock OpenAI endpoint, so no live inference server is
required. The rules engine is exercised through the test1.pdf fixture
without loading its config, which makes the rules path fail and the LLM
fallback take over.
"""

import base64
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
from transtractor import LlmConfig, ParseError, Parser
from transtractor.structs.statement_data import StatementData

FIXTURES_DIR = Path(__file__).parent.parent / "fixtures"
TEST_PDF = FIXTURES_DIR / "test1.pdf"

VALID_STATEMENT = {
    "account_number": "1234 5678 9123 4567",
    "start_date": "2025-01-01",
    "opening_balance": 1000.0,
    "closing_balance": 900.0,
    "transactions": [
        {
            "date": "2025-01-02",
            "description": "Payment",
            "amount": -150.0,
            "balance": 850.0,
        },
        {"date": "2025-01-03", "description": "Deposit", "amount": 50.0},
    ],
}


def assistant_content(content):
    """Wrap statement JSON as an assistant chat completion response."""
    return {"choices": [{"message": {"role": "assistant", "content": content}}]}


class MockLlmEndpoint:
    """A throwaway chat-completions server serving canned responses in order."""

    def __init__(self, responses):
        """Start the server on an ephemeral port.

        :param responses: `(status, payload)` pairs served for each POST to
            `/v1/chat/completions`, in order
        """
        self.requests = []

        outer = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                if self.path != "/v1/chat/completions":
                    self.send_error(404)
                    return
                length = int(self.headers.get("Content-Length", 0))
                outer.requests.append(json.loads(self.rfile.read(length)))
                status, payload = responses.pop(0)
                encoded = json.dumps(payload).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

            def log_message(self, format, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}/v1"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        """Stop the server."""
        self.server.shutdown()
        self.server.server_close()


@pytest.fixture()
def mock_llm():
    """Start one mock endpoint, yield a starter, and clean up."""
    endpoints = []

    def start(responses):
        endpoint = MockLlmEndpoint(list(responses))
        endpoints.append(endpoint)
        return endpoint

    yield start

    for endpoint in endpoints:
        endpoint.close()


def test_parse_falls_back_to_llm_when_rules_fail(mock_llm):
    """Rules failure with a configured endpoint extracts via the LLM path."""
    endpoint = mock_llm([(200, assistant_content(json.dumps(VALID_STATEMENT)))])
    parser = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))

    statement_data = parser.parse(str(TEST_PDF))

    assert isinstance(statement_data, StatementData)
    assert statement_data.key == "llm/mock-model"
    assert statement_data.opening_balance == 1000.0
    assert statement_data.closing_balance == 900.0
    assert len(statement_data.transactions) == 2

    # The request hit the chat completions endpoint with the page-fenced
    # statement text and guided decoding enabled.
    assert len(endpoint.requests) == 1
    body = endpoint.requests[0]
    assert body["model"] == "mock-model"
    assert body["response_format"]["type"] == "json_schema"
    user_content = body["messages"][1]["content"]
    assert isinstance(user_content, str)
    assert "<<<PAGE 1>>>" in user_content


def test_llm_numbers_that_do_not_reconcile_raise_parse_error(mock_llm):
    """A response whose numbers fail balance validation is rejected."""
    bad_statement = dict(VALID_STATEMENT)
    bad_statement["closing_balance"] = 800.0  # 1000 - 150 + 50 = 900
    # The correction loop runs once by default: both attempts fail.
    endpoint = mock_llm(
        [
            (200, assistant_content(json.dumps(bad_statement))),
            (200, assistant_content(json.dumps(bad_statement))),
        ]
    )
    parser = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))

    with pytest.raises(ParseError) as excinfo:
        parser.parse(str(TEST_PDF))

    assert "failed validation" in str(excinfo.value)


def test_correction_loop_recovers_on_second_attempt(mock_llm):
    """Checker errors are fed back to the model, which fixes its answer."""
    bad_statement = dict(VALID_STATEMENT)
    bad_statement["closing_balance"] = 800.0
    endpoint = mock_llm(
        [
            (200, assistant_content(json.dumps(bad_statement))),
            (200, assistant_content(json.dumps(VALID_STATEMENT))),
        ]
    )
    parser = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))

    statement_data = parser.parse(str(TEST_PDF))  # strict mode, reconciled

    assert statement_data.errors == []
    assert len(statement_data.transactions) == 2
    assert len(endpoint.requests) == 2
    second = endpoint.requests[1]
    assert second["messages"][2]["role"] == "assistant"
    assert "balance mismatch" in second["messages"][3]["content"]
    assert "Your previous answer" in second["messages"][3]["content"]
    # Pattern-driven diagnosis is included: this fixture is a final-only gap
    # of 100 = 2 x 50.00, so the Deposit row is flagged as a sign-flip candidate.
    assert "Diagnosis (final-only-total-gap)" in second["messages"][3]["content"]
    assert "sign may be flipped" in second["messages"][3]["content"]


def test_llm_schema_rejection_is_retried_without_response_format(mock_llm):
    """A 400 for `response_format` triggers one retry without the parameter."""
    endpoint = mock_llm(
        [
            (400, {"error": "response_format not supported"}),
            (200, assistant_content(json.dumps(VALID_STATEMENT))),
        ]
    )
    parser = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))

    statement_data = parser.parse(str(TEST_PDF))

    assert statement_data.key == "llm/mock-model"
    assert len(endpoint.requests) == 2
    assert "response_format" in endpoint.requests[0]
    assert "response_format" not in endpoint.requests[1]


def test_lenient_parse_returns_rows_with_errors(mock_llm):
    """Lenient mode returns near-correct rows plus checker errors."""
    bad_statement = dict(VALID_STATEMENT)
    bad_statement["closing_balance"] = 800.0  # 1000 - 150 + 50 = 900
    # Correction loop disabled to keep this test single-shot: one canned
    # response per attempt (strict attempt, then lenient attempt).
    endpoint = mock_llm(
        [
            (200, assistant_content(json.dumps(bad_statement))),
            (200, assistant_content(json.dumps(bad_statement))),
        ]
    )
    parser = Parser(
        llm=LlmConfig(base_url=endpoint.url, model="mock-model", correction_rounds=0)
    )

    with pytest.raises(ParseError):
        parser.parse(str(TEST_PDF))  # strict default still raises

    statement_data = parser.parse(str(TEST_PDF), lenient=True)

    assert statement_data.key == "llm/mock-model"
    assert len(statement_data.transactions) == 2
    assert statement_data.errors, "Expected checker errors to be attached"
    assert "balance mismatch" in statement_data.errors[0]

    # A reconciling extraction comes back error-free even in lenient mode.
    good = mock_llm([(200, assistant_content(json.dumps(VALID_STATEMENT)))])
    parser_good = Parser(
        llm=LlmConfig(base_url=good.url, model="mock-model", correction_rounds=0)
    )
    clean = parser_good.parse(str(TEST_PDF), lenient=True)
    assert clean.errors == []
    assert len(clean.transactions) == 2


def test_without_llm_configuration_rules_errors_are_unchanged():
    """No endpoint configured: the original rules error surfaces."""
    parser = Parser()

    with pytest.raises(ParseError) as excinfo:
        parser.parse(str(TEST_PDF))

    assert "cannot be identified" in str(excinfo.value)


def test_llm_config_is_read_from_environment(monkeypatch):
    """TRANSTRACTOR_LLM_* variables configure the fallback with no code."""
    monkeypatch.setenv("TRANSTRACTOR_LLM_URL", "http://127.0.0.1:9999/v1")
    monkeypatch.setenv("TRANSTRACTOR_LLM_MODEL", "env-model")

    parser = Parser()

    assert parser.llm_config is not None
    assert parser.llm_config.base_url == "http://127.0.0.1:9999/v1"
    assert parser.llm_config.model == "env-model"
    assert parser._inner.is_llm_configured()


def test_llm_config_env_requires_both_url_and_model(monkeypatch):
    """A model without a URL (or vice versa) leaves the fallback disabled."""
    monkeypatch.setenv("TRANSTRACTOR_LLM_MODEL", "env-model")
    monkeypatch.delenv("TRANSTRACTOR_LLM_URL", raising=False)

    parser = Parser()

    assert parser.llm_config is None
    assert not parser._inner.is_llm_configured()


def test_vision_path_requires_llm_configuration(mock_llm):
    """Calling the images path without configure_llm raises RuntimeError."""
    endpoint = mock_llm([(200, assistant_content(json.dumps(VALID_STATEMENT)))])
    configured = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))
    unconfigured = Parser()

    statement_data = configured._inner.py_pdf_path_to_py_statement_data_with_images(
        str(TEST_PDF), [("aGVsbG8=", "image/png")]
    )

    assert statement_data.key == "llm/mock-model"
    assert (
        "data:image/png;base64,aGVsbG8="
        in endpoint.requests[0]["messages"][1]["content"][0]["image_url"]["url"]
    )

    with pytest.raises(RuntimeError):
        unconfigured._inner.py_pdf_path_to_py_statement_data_with_images(
            str(TEST_PDF), [("aGVsbG8=", "image/png")]
        )


def test_has_text_layer_detects_text_and_scan(tmp_path):
    """Digital PDFs report a text layer; image-only PDFs do not."""
    pytest.importorskip("PIL")
    from PIL import Image

    parser = Parser()
    assert parser._inner.py_pdf_path_has_text_layer(str(TEST_PDF))

    scan_pdf = tmp_path / "scan.pdf"
    Image.new("RGB", (24, 24), color="white").save(scan_pdf, format="PDF")
    assert not parser._inner.py_pdf_path_has_text_layer(str(scan_pdf))


def test_scanned_pdf_is_routed_through_the_vision_path(mock_llm, tmp_path):
    """A statement with no text layer renders pages and calls the vision path."""
    pytest.importorskip("PIL")
    from PIL import Image

    endpoint = mock_llm([(200, assistant_content(json.dumps(VALID_STATEMENT)))])
    parser = Parser(llm=LlmConfig(base_url=endpoint.url, model="mock-model"))

    scan_pdf = tmp_path / "scan.pdf"
    Image.new("RGB", (24, 24), color="white").save(scan_pdf, format="PDF")

    statement_data = parser.parse(str(scan_pdf))

    assert statement_data.key == "llm/mock-model"
    assert len(endpoint.requests) == 1
    content = endpoint.requests[0]["messages"][1]["content"]
    assert content[-1]["type"] == "text"
    assert all(part["type"] == "image_url" for part in content[:-1])
    assert all(
        part["image_url"]["url"].startswith("data:image/") for part in content[:-1]
    )


def test_render_pdf_pages_encodes_jpeges_in_order():
    """The rendering helper produces ordered base64 JPEG pages."""
    pytest.importorskip("pypdfium2")
    from transtractor.utils.rendering import render_pdf_pages

    pages = render_pdf_pages(str(TEST_PDF), max_width_px=300)

    assert len(pages) == 3
    for b64, mime in pages:
        assert mime == "image/jpeg"
        assert base64.b64decode(b64)[:2] == b"\xff\xd8"  # JPEG magic bytes
