//! Prompt construction for the LLM fallback backend.
//!
//! The extraction guidance adapts ideas from bankstatementparser's
//! Apache-2.0 extraction prompt (reading-order warning, chronological
//! sorting, never invent values).

use crate::structs::TextItem;

/// Instructions shared by the text and vision extraction paths.
pub const SYSTEM_PROMPT: &str = "You are a meticulous bank statement data extraction engine. \
Your job is to read the provided statement document and transcribe its contents into STRICT \
JSON. Never invent values; transcription only — do no arithmetic.\n\
Rules:\n\
- Copy numbers exactly as printed; ignore thousand separators and currency symbols.\n\
- Copy the account number exactly as printed, including any spaces or dashes.\n\
- Output amounts as plain decimal numbers with a sign: negative for money leaving the \
account, positive for money coming in.\n\
- Use ISO dates (YYYY-MM-DD).\n\
- Include every transaction ledger row. Do not invent, merge or omit transactions.\n\
- PDF text can arrive out of reading order: sort transactions chronologically by date \
(oldest first), and where two rows share a date, keep the order they appeared in the \
document.\n\
- Skip page headers and footers, page numbers, marketing text, and end-of-statement summary \
tables.\n\
- The balance field of a transaction is the running balance printed on that row; omit it \
when the statement does not show one.\n\
- The opening and closing balances are the balances immediately before the first and after \
the last transaction of the statement period.\n\
- Your answer is verified by arithmetic: previous balance + each amount must equal that \
row's stated balance, and the total must reach the stated closing balance. A wrong digit, \
flipped sign, or dropped row will be detected.\n\
- Respond with a single JSON object and nothing else — no prose, no markdown.";

/// User instruction for the correction round of the self-correction loop:
/// the checker errors from the previous attempt are fed back verbatim, plus
/// any pattern-driven hints from [`crate::llm::diagnostics`].
pub fn correction_prompt(
    errors: &[String],
    previous_json: &str,
    diagnosis: &crate::llm::diagnostics::FailureDiagnosis,
) -> String {
    let mut prompt = String::from(
        "Your previous answer failed arithmetic validation against the statement's own \
balances. The validation errors are:\n",
    );
    for error in errors {
        prompt.push_str(&format!("- {}\n", error));
    }
    if !diagnosis.hints.is_empty() {
        prompt.push_str(&format!("\nDiagnosis ({}):\n", diagnosis.pattern));
        for hint in &diagnosis.hints {
            prompt.push_str(&format!("- {}\n", hint));
        }
    }
    prompt.push_str(&format!(
        "\nYour previous answer was:\n{}\n\n\
Fix the errors and return the complete corrected JSON object only. Check that every \
transaction appears exactly once, that each running balance equals the previous balance \
plus the amount, and that the total reaches the stated closing balance.",
        previous_json
    ));
    prompt
}

/// Render extracted text items as page-fenced plain text for the LLM.
///
/// Items are expected in pipeline order (page, then line bin, then x); items
/// on the same line are joined with single spaces.
pub fn text_items_to_prompt(items: &[TextItem]) -> String {
    let mut prompt =
        String::from("Extract the bank statement data from the following statement text.\n");
    let mut current_page: Option<i32> = None;
    let mut current_line: Option<(i32, i32)> = None;

    for item in items {
        let line = (item.page, item.y1_bin);
        if current_line == Some(line) {
            // Same line as the previous item: join with a single space.
            prompt.push(' ');
            prompt.push_str(&item.text);
        } else {
            if current_page != Some(item.page) {
                prompt.push_str(&format!("\n<<<PAGE {}>>>\n", item.page + 1));
                current_page = Some(item.page);
            } else {
                prompt.push('\n');
            }
            prompt.push_str(&item.text);
            current_line = Some(line);
        }
    }

    prompt
}

/// User instruction accompanying page images for vision models.
pub fn vision_prompt(page_count: usize) -> String {
    format!(
        "The attached images are the {page_count} pages of a bank statement, in order. \
Extract the complete statement data per the system instructions."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, x1: i32, page: i32, y1_bin: i32) -> TextItem {
        TextItem::new(text.to_string(), x1, y1_bin, x1 + 10, y1_bin + 10, page)
    }

    #[test]
    fn test_empty_items_produce_instruction_only_prompt() {
        let prompt = text_items_to_prompt(&[]);
        assert!(prompt.contains("statement text"));
        assert!(!prompt.contains("<<<PAGE"));
    }

    #[test]
    fn test_same_line_items_are_joined_with_spaces() {
        let items = vec![item("Opening", 0, 0, 10), item("balance:", 60, 0, 10)];
        let prompt = text_items_to_prompt(&items);
        assert!(prompt.contains("<<<PAGE 1>>>"));
        assert!(prompt.contains("Opening balance:"));
    }

    #[test]
    fn test_new_lines_and_pages_are_fenced() {
        let items = vec![
            item("Line", 0, 0, 10),
            item("one", 40, 0, 10),
            item("Line", 0, 0, 30),
            item("two", 40, 0, 30),
            item("Second", 0, 1, 10),
            item("page", 60, 1, 10),
        ];
        let prompt = text_items_to_prompt(&items);
        assert!(prompt.contains("<<<PAGE 1>>>\nLine one\nLine two"));
        assert!(prompt.contains("<<<PAGE 2>>>\nSecond page"));
    }

    #[test]
    fn test_vision_prompt_mentions_page_count() {
        assert!(vision_prompt(3).contains("3 pages"));
    }
}
