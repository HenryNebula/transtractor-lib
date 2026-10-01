"""Live integration tests for the local LLM fallback.

Skipped unless both ``TRANSTRACTOR_LLM_URL`` and ``TRANSTRACTOR_LLM_MODEL``
point at a running OpenAI-compatible endpoint (e.g. llama-server with a
vision model and its mmproj file):

    TRANSTRACTOR_LLM_URL=http://127.0.0.1:8080/v1 \
    TRANSTRACTOR_LLM_MODEL=qwen2.5-vl-7b-instruct \
    uv run pytest tests/python/test_parser__llm_live.py
"""

import os
from pathlib import Path

import pytest
from transtractor import Parser

FIXTURES_DIR = Path(__file__).parent.parent / "fixtures"
TEST_PDF = FIXTURES_DIR / "test1.pdf"

pytestmark = pytest.mark.skipif(
    not (
        os.environ.get("TRANSTRACTOR_LLM_URL")
        and os.environ.get("TRANSTRACTOR_LLM_MODEL")
    ),
    reason="live LLM endpoint not configured",
)


def test_live_fallback_extracts_test_fixture():
    """test1.pdf without its config: rules fail, the local model extracts it."""
    parser = Parser()

    statement_data = parser.parse(str(TEST_PDF))

    assert statement_data.key is not None
    assert statement_data.key.startswith("llm/")
    assert statement_data.opening_balance is not None
    assert statement_data.closing_balance is not None
    assert len(statement_data.transactions) > 0
