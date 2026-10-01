"""Tests for loading configurations from JSON strings."""

from pathlib import Path

import pytest
from transtractor import ConfigLoadError, Parser

FIXTURES_DIR = Path(__file__).parent.parent / "fixtures"


def test_load_str_registers_config_and_parses():
    """A config loaded from a string behaves like one loaded from a file."""
    config_text = (FIXTURES_DIR / "test1_config.json").read_text()
    parser = Parser()
    parser.load_str(config_text)

    statement_data = parser.parse(str(FIXTURES_DIR / "test1.pdf"))

    assert statement_data.key == "au__gtb__fake_account__1"
    assert len(statement_data.transactions) == 62
    assert statement_data.closing_balance == 11663.82


def test_load_str_raises_config_load_error_for_invalid_config():
    """Invalid config text raises ConfigLoadError with the validator message."""
    parser = Parser()

    with pytest.raises(ConfigLoadError):
        parser.load_str('{"key": "bad config", "unknown_field": true}')


def test_load_str_emits_deprecation_warnings():
    """Deprecated fields in a string config emit DeprecationWarnings."""
    config_text = (FIXTURES_DIR / "test1_config_deprecated.json").read_text()
    parser = Parser()

    with pytest.warns(DeprecationWarning):
        parser.load_str(config_text)
