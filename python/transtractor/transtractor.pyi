"""Stub file for transtractor Rust extension module."""

from .structs.statement_data import StatementData

class LibParser:
    """Parser for extracting statement data from text items."""

    def __init__(self) -> None:
        """Create a new LibParser instance."""

    def register_config_from_json(self, py_config_json_path: str) -> None:
        """
        Register JSON configuration file into the parser database.

        :param py_config_json_path: Path to the JSON configuration file
        :raises ConfigLoadError: If the configuration file cannot be loaded
        """

    def register_config_from_json_str(self, json_str: str) -> list[str]:
        """
        Register a configuration from a JSON string, update the
        StatementTyper and return any deprecation warnings.

        :param json_str: The configuration JSON text
        :return: List of deprecation warnings
        :raises ConfigLoadError: If the configuration is invalid
        """

    def get_deprecation_warnings(self) -> list[str]:
        """
        Get deprecation warnings from the last loaded configuration.

        :return: List of deprecation warnings
        """

    def configure_llm(
        self,
        base_url: str,
        model: str,
        api_key: str | None = None,
        timeout_secs: int = 120,
        schema_mode: bool = True,
        no_think: bool = False,
        correction_rounds: int = 1,
    ) -> None:
        """
        Configure an OpenAI-compatible local inference endpoint used as the
        fallback when the rules engine cannot parse a statement.

        Only present when the library is built with the `llm` cargo feature.

        :param base_url: Endpoint base URL including the `/v1` prefix,
            e.g. `http://127.0.0.1:8080/v1`
        :param model: Model name expected by the endpoint
        :param api_key: Optional Bearer token (llama-server `--api-key`)
        :param timeout_secs: Global request timeout in seconds
        :param schema_mode: Attempt `response_format: json_schema` guided decoding
        :param no_think: Disable thinking for Qwen3-style models
        :param correction_rounds: Feed checker errors back to the model for
            another attempt when validation fails (0 disables)
        """

    def is_llm_configured(self) -> bool:
        """
        Whether an LLM fallback endpoint is configured.

        Only present when the library is built with the `llm` cargo feature.
        """

    def py_pdf_path_has_text_layer(self, py_pdf_path: str) -> bool:
        """
        Check whether the PDF has a usable text layer. Statements without one
        (scans) should be processed via page images with
        `py_pdf_path_to_py_statement_data_with_images`.

        :param py_pdf_path: Path to the PDF file
        """

    def py_pdf_path_to_py_statement_data(self, py_pdf_path: str) -> StatementData:
        """
        Process a PDF file path from Python caller and return a Python StatementData
        object.

        Falls back to the configured local LLM endpoint when the rules engine
        cannot parse the statement.

        :param py_pdf_path: Path to the PDF file
        :raises ParseError: If statement is not recognisable or not parsed correctly
        """

    def py_pdf_path_to_py_statement_data_with_images(
        self, py_pdf_path: str, py_images: list[tuple[str, str]]
    ) -> StatementData:
        """
        Process a scanned PDF via caller-supplied page images using a local
        vision LLM endpoint.

        Only present when the library is built with the `llm` cargo feature.

        :param py_pdf_path: Path to the PDF file
        :param py_images: `(base64_data, mime_type)` tuples, one per page, in order
        :raises ParseError: If extraction fails or the numbers do not reconcile
        """

    def py_pdf_path_to_py_statement_data_lenient(
        self, py_pdf_path: str
    ) -> StatementData:
        """
        Process a PDF file path leniently: rules engine first, then the local
        LLM fallback with validation errors attached to the returned
        StatementData instead of raised.

        Only present when the library is built with the `llm` cargo feature.

        :param py_pdf_path: Path to the PDF file
        :raises ParseError: Only when no extraction at all is possible
        """

    def py_pdf_path_to_py_statement_data_with_images_lenient(
        self, py_pdf_path: str, py_images: list[tuple[str, str]]
    ) -> StatementData:
        """
        Lenient variant of the vision path; validation errors are attached to
        the returned StatementData instead of raised.

        Only present when the library is built with the `llm` cargo feature.

        :param py_pdf_path: Path to the PDF file
        :param py_images: `(base64_data, mime_type)` tuples, one per page, in order
        """

    def py_pdf_path_to_layout(self, py_pdf_path: str, py_layout_path: str) -> None:
        """
        Process a PDF file into layout text str.

        :param py_pdf_path: Path to the PDF file
        :param py_layout_path: Path to the output layout text file
        """

    def py_pdf_path_to_debug(self, py_pdf_path: str, py_debug_path: str) -> None:
        """Process a PDF file path from Python caller and write debug information to a
        file.

        :param py_pdf_path: Path to the PDF file
        :param py_debug_path: Path to the output debug text file
        """

    def py_layout_path_to_py_statement_data(self, py_layout_path: str) -> StatementData:
        """
        Process a layout text file from Python caller and return statement data as a
        Python object of type StatementData.

        :param py_layout_path: Path to the layout text file
        :raises ParseError: If statement is not recognisable or not parsed correctly
        """

    def py_layout_path_to_debug(self, py_layout_path: str, py_debug_path: str) -> None:
        """
        Process a layout text file from Python caller and write debug information to a
        text file.

        :param py_layout_path: Path to the layout text file
        :param py_debug_path: Path to the output debug text file
        :raises ParseError: If statement is not recognisable or not parsed correctly
        """

    def py_pdf_path_to_spec(self, py_pdf_path: str, py_spec_path: str) -> None:
        """
        Process a PDF file path from Python caller and write a JSON spec string to a
        file.

        :param py_pdf_path: Path to the PDF file
        :param py_spec_path: Path to the output JSON spec file
        :raises ParseError: If statement is not recognisable or not parsed correctly
        """

    def py_layout_path_to_spec(self, py_layout_path: str, py_spec_path: str) -> None:
        """
        Process a layout text file from Python caller and write a JSON spec string to a
        file.

        :param py_layout_path: Path to the layout text file
        :param py_spec_path: Path to the output JSON spec file
        :raises ParseError: If statement is not recognisable or not parsed correctly
        """

    def py_spec_path_to_validate(self, py_spec_path: str) -> None:
        """
        Validate a JSON spec file from Python caller and return any validation errors.

        If invalid, a diff summary will be included in the error message of the raised
        SpecError.

        :param py_spec_path: Path to the JSON spec file
        :raises SpecError: If the spec is invalid against the current configuration
        """

class ParseError(Exception):
    """Raised when the content of a PDF file cannot be parsed correctly."""

class SpecError(Exception):
    """Raised when a JSON spec file is invalid against the current configuration
    database."""

class ConfigLoadError(Exception):
    """Raised when a configuration cannot be loaded."""
