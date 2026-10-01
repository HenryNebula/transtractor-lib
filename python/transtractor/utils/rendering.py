"""Page rendering helpers for the LLM vision fallback.

The Rust core deliberately does not rasterise PDFs (keeping the extension
free of native imaging dependencies). For scanned statements with no text
layer, the Python side renders pages with ``pypdfium2`` and hands the encoded
images to the core's vision extraction path.
"""

from __future__ import annotations

import base64
import io

#: Default maximum rendered page width in pixels. Bank statements are
#: portrait documents; 1500 px keeps text legible for vision models while
#: bounding the base64 request size.
DEFAULT_MAX_WIDTH_PX = 1500

_MISSING_DEP_MESSAGE = (
    "Rendering scanned statements requires pypdfium2. "
    "Install it with: pip install 'transtractor[llm]'"
)


def render_pdf_pages(
    pdf_file_path: str,
    max_width_px: int = DEFAULT_MAX_WIDTH_PX,
) -> list[tuple[str, str]]:
    """Render every page of a PDF to base64-encoded JPEGs.

    :param pdf_file_path: Path to the PDF file
    :param max_width_px: Maximum rendered page width in pixels; pages are
        scaled down to fit
    :return: ``(base64_data, mime_type)`` tuples, one per page, in order
    :raises ImportError: When ``pypdfium2`` is not installed
    """
    try:
        import pypdfium2 as pdfium
    except ImportError as error:  # pragma: no cover - trivial guard
        raise ImportError(_MISSING_DEP_MESSAGE) from error

    pages: list[tuple[str, str]] = []
    pdf = pdfium.PdfDocument(pdf_file_path)
    try:
        for page in pdf:
            # pypdfium2 renders at a fixed scale; compute the scale that
            # fits the requested maximum width.
            original_width = page.get_width()
            scale = (
                min(1.0, max_width_px / original_width) if original_width > 0 else 1.0
            )
            # pypdfium2 accepts float scales at runtime; the bundled stub
            # narrows the parameter to int.
            pil_image = (
                page.render(scale=scale)  # pyright: ignore[reportArgumentType]
                .to_pil()
                .convert("RGB")
            )
            buffer = io.BytesIO()
            pil_image.save(buffer, format="JPEG", quality=90)
            pages.append(
                (base64.b64encode(buffer.getvalue()).decode("ascii"), "image/jpeg")
            )
    finally:
        pdf.close()
    return pages
