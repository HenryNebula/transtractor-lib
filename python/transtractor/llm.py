"""Configuration for the optional local LLM fallback extraction backend.

The fallback sends statement text (or rendered page images for scans) to an
OpenAI-compatible local inference endpoint — llama.cpp ``llama-server``, vLLM
or Ollama — when the deterministic rules engine cannot parse a statement. The
model's output is validated against the statement's own opening/closing
balances before it is returned, so no unverified numbers are ever produced.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

#: Environment variables recognised by :meth:`LlmConfig.from_env`.
ENV_BASE_URL = "TRANSTRACTOR_LLM_URL"
ENV_MODEL = "TRANSTRACTOR_LLM_MODEL"
ENV_API_KEY = "TRANSTRACTOR_LLM_API_KEY"


@dataclass
class LlmConfig:
    """Configuration for an OpenAI-compatible local inference endpoint.

    The endpoint must expose ``POST {base_url}/chat/completions``. Only plain
    HTTP is supported; the fallback targets local endpoints, so statement data
    never leaves the machine.

    :param base_url: Endpoint base URL including the ``/v1`` prefix,
        e.g. ``http://127.0.0.1:8080/v1``
    :param model: Model name as expected by the endpoint,
        e.g. ``qwen2.5-vl-7b-instruct``
    :param api_key: Optional Bearer token (llama-server ``--api-key``)
    :param timeout_secs: Global request timeout in seconds; local inference on
        large statements can take over a minute
    :param schema_mode: Attempt ``response_format: json_schema`` guided
        decoding first, retrying without it if the endpoint rejects the
        parameter
    """

    base_url: str
    model: str
    api_key: str | None = None
    timeout_secs: int = 120
    schema_mode: bool = True

    @classmethod
    def from_env(cls) -> LlmConfig | None:
        """Build a config from ``TRANSTRACTOR_LLM_URL``, ``TRANSTRACTOR_LLM_MODEL``
        and (optionally) ``TRANSTRACTOR_LLM_API_KEY``.

        Returns ``None`` unless both the URL and the model are set.
        """
        base_url = os.environ.get(ENV_BASE_URL, "").strip()
        model = os.environ.get(ENV_MODEL, "").strip()
        if not base_url or not model:
            return None
        api_key = os.environ.get(ENV_API_KEY, "").strip() or None
        return cls(
            base_url=base_url,
            model=model,
            api_key=api_key,
        )
