Local LLM Fallback
==================

The rules engine parses statements deterministically, but only when a matching
extraction configuration exists. The **local LLM fallback** covers the remaining
cases: statements from unsupported banks, and scanned statements with no text
layer. When the rules engine fails, the statement is sent to a local
OpenAI-compatible inference endpoint, and the model's extraction is run
through the standard fixer and checker pipeline — the extraction is only
returned if its numbers reconcile with the statement's opening and closing
balances. Extractions that do not reconcile raise the usual ``ParseError``.

Nothing is sent anywhere unless the fallback is explicitly configured, and the
client speaks plain HTTP only, so statement data never leaves the machine.

Compatibility
-------------

Any endpoint implementing ``POST {base_url}/chat/completions`` works:

* `llama.cpp <https://github.com/ggml-org/llama.cpp>`_ ``llama-server`` (the
  primary target)
* `vLLM <https://docs.vllm.ai/>`_
* `Ollama <https://ollama.com/>`_ (OpenAI-compatible endpoint)

Vision models (e.g. Qwen-VL) additionally enable the scanned-statement path;
text-only models work for digital statements with a text layer.

Quick start
-----------

.. code-block:: bash

   pip install 'transtractor[llm]'

Start the endpoint, for example llama-server with a vision model and its
mmproj file:

.. code-block:: bash

   llama-server -m qwen2.5-vl-7b-instruct-Q4_K_M.gguf \
      --mmproj mmproj-qwen2.5-vl-7b-instruct-f16.gguf \
      --host 127.0.0.1 --port 8080

Then parse statements as usual:

.. code-block:: python

   from transtractor import LlmConfig, Parser

   parser = Parser(llm=LlmConfig(
       base_url="http://127.0.0.1:8080/v1",
       model="qwen2.5-vl-7b-instruct",
   ))

   statement_data = parser.parse('statement.pdf')  # rules first, LLM fallback
   statement_data.to_csv('statement.csv')

The same configuration can be provided without code changes via environment
variables (``Parser()`` picks them up automatically):

.. code-block:: bash

   export TRANSTRACTOR_LLM_URL=http://127.0.0.1:8080/v1
   export TRANSTRACTOR_LLM_MODEL=qwen2.5-vl-7b-instruct
   # export TRANSTRACTOR_LLM_API_KEY=...   # if llama-server runs with --api-key

How it works
------------

1. The parser extracts the PDF text layer and runs the deterministic rules
   engine exactly as before. **The rules engine always takes precedence.**
2. Only when no configuration matches (or matched configurations fail) and an
   endpoint is configured does the fallback engage:

   * **Text path** — the extracted lines are rendered as a page-fenced
     plain-text prompt.
   * **Vision path** — PDFs without a usable text layer are rendered to page
     images (via ``pypdfium2``) and sent to the model. Requires a vision model.
3. The model must answer with a JSON statement (guided decoding via
   ``response_format: json_schema`` is attempted first and transparently
   retried without it on endpoints that reject the parameter).
4. The response is converted and passed through the same fixers and checkers
   as the rules engine: implicit balances are backfilled, amount signs are
   corrected against the balance column, and the running balance must match
   every stated balance within one cent.
5. Statements extracted via the fallback carry the key ``llm/<model>``, so
   downstream consumers can distinguish them from rules-parsed statements.

If validation fails, the error message lists each mismatching transaction
(the same messages the debug output produces) and no data is returned.

Thinking models (Qwen3-style)
-----------------------------

Qwen3-style hybrid thinking models spend the token budget reasoning before
answering, which on a small context window leaves an empty answer. llama-server
exposes the chat-template switch for this; enable it with ``no_think=True``:

.. code-block:: python

   parser = Parser(llm=LlmConfig(
       base_url="http://127.0.0.1:8080/v1",
       model="Qwen_Qwen3.5-4B-Q8_0.gguf",
       no_think=True,
   ))

or ``TRANSTRACTOR_LLM_NO_THINK=1`` via the environment.

Limitations
-----------

* Plain HTTP only (no TLS); the feature targets ``127.0.0.1`` endpoints.
* All page images are sent in a single request — very long scanned statements
  may exceed the model's context window.
* The text-layer heuristic treats PDFs with fewer than 20 text items as scans.
* The GIL is released during inference, so other Python threads keep running,
  but each ``parse`` call remains blocking.

Cargo feature note
------------------

In Rust, the fallback requires the opt-in ``llm`` cargo feature and a
configured endpoint:

.. code-block:: rust

   use transtractor::llm::LlmConfig;
   use transtractor::parser::Parser;

   let parser = Parser::new().with_llm(LlmConfig::new(
       "http://127.0.0.1:8080/v1",
       "qwen2.5-vl-7b-instruct",
   ));
   let data = parser.parse("statement.pdf")?;

Python wheels published from this fork are built with the feature enabled; the
``transtractor[llm]`` pip extra only adds the optional page-rendering
dependencies.
