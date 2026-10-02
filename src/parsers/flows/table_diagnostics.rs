//! Table diagnostics: explain, per table line, why it did or did not match
//! the configured column orders and formats.
//!
//! This is an explanatory pass, not the extraction path: it replays the
//! table region line by line against the configured `transaction_formats`
//! orders using the engine's own format parsers, and names the first column
//! that could not be satisfied — including the common case where a token
//! matches a format that is simply not declared for that column. Intended
//! for configuration authoring (humans and LLM-assisted harnesses).
//!
//! Like the extraction pipeline, items are tokenised into words before
//! format matching: multi-item formats (e.g. date format1) consume two
//! adjacent words ("01" + "Jan"), whether or not the PDF extractor merged
//! them into one text item.

use crate::formats::amount::MultiAmountFormatParser;
use crate::formats::date::MultiDateFormatParser;
use crate::structs::{StatementConfig, TextItem};

/// Maximum unmatched lines to report per config.
const MAX_REPORTED_LINES: usize = 8;

/// All amount format names, for "matches an undeclared format" hints.
const ALL_AMOUNT_FORMATS: [&str; 7] = [
    "format1", "format2", "format3", "format4", "format5", "format6", "format7",
];

/// All date format names, for the same purpose.
const ALL_DATE_FORMATS: [&str; 15] = [
    "format1", "format2", "format3", "format4", "format5", "format6", "format7", "format8",
    "format9", "format10", "format11", "format12", "format13", "format14", "format15",
];

/// A tokenised line: individual words in reading order.
#[derive(Debug)]
struct Line {
    text: String,
    words: Vec<String>,
}

/// Group pre-sorted text items into lines by (page, y1_bin) and split each
/// line into words, mirroring the pipeline's tokeniser.
fn group_tokenised_lines(items: &[TextItem]) -> Vec<Line> {
    let mut lines: Vec<Vec<&TextItem>> = Vec::new();
    let mut current: Option<(i32, i32)> = None;
    for item in items {
        let key = (item.page, item.y1_bin);
        if current != Some(key) {
            lines.push(Vec::new());
            current = Some(key);
        }
        lines.last_mut().expect("lines is non-empty").push(item);
    }
    lines
        .into_iter()
        .map(|items| {
            let words: Vec<String> = items
                .iter()
                .flat_map(|item| item.text.split_whitespace())
                .map(str::to_string)
                .collect();
            let text = words.join(" ");
            Line { text, words }
        })
        .filter(|line| !line.words.is_empty())
        .collect()
}

/// Try to match a date at `index`, consuming one or two adjacent words
/// (multi-item formats parse "01" + "Jan" joined). A nominal year is
/// supplied for year-less formats; it does not change structural matching.
fn date_matches_at(formats: &[String], words: &[String], index: usize) -> Option<usize> {
    let names: Vec<&str> = formats.iter().map(String::as_str).collect();
    let parser = MultiDateFormatParser::new(&names);
    if parser.parse(&words[index], "2025", 1).is_some() {
        return Some(1);
    }
    if index + 1 < words.len() {
        let joined = format!("{} {}", words[index], words[index + 1]);
        if parser.parse(&joined, "2025", 2).is_some() {
            return Some(2);
        }
    }
    None
}

/// Try to match an amount at `index`, consuming one or two adjacent words
/// (multi-item formats parse "90,350" + "CR" joined).
fn amount_matches_at(formats: &[String], words: &[String], index: usize) -> Option<usize> {
    let names: Vec<&str> = formats.iter().map(String::as_str).collect();
    let parser = MultiAmountFormatParser::new(&names);
    if parser.parse(&words[index], 1).is_some() {
        return Some(1);
    }
    if index + 1 < words.len() {
        let joined = format!("{} {}", words[index], words[index + 1]);
        if parser.parse(&joined, 2).is_some() {
            return Some(2);
        }
    }
    None
}

/// Find a format name (from `candidates`) that accepts the word(s).
fn which_format<'a>(candidates: &[&'a str], words: &[&str], date: bool) -> Option<&'a str> {
    let single = words[0].to_string();
    let joined = if words.len() > 1 {
        Some(words.join(" "))
    } else {
        None
    };
    for name in candidates {
        let names = [*name];
        let ok = if date {
            let parser = MultiDateFormatParser::new(&names);
            parser.parse(&single, "2025", 1).is_some()
                || joined
                    .as_ref()
                    .is_some_and(|j| parser.parse(j, "2025", 2).is_some())
        } else {
            let parser = MultiAmountFormatParser::new(&names);
            parser.parse(&single, 1).is_some()
                || joined
                    .as_ref()
                    .is_some_and(|j| parser.parse(j, 2).is_some())
        };
        if ok {
            return Some(name);
        }
    }
    None
}

/// Produce per-line diagnostic explanations for the table region.
pub fn table_diagnostics(config: &StatementConfig, items: &[TextItem]) -> Vec<String> {
    let lines = group_tokenised_lines(items);

    // Locate the table region: start at any transaction term, end at any
    // stop term (or the end of the document).
    let start = lines
        .iter()
        .position(|line| {
            config
                .transaction_terms
                .iter()
                .any(|term| line.text.contains(term.as_str()))
        })
        .unwrap_or(0);
    let end = lines
        .iter()
        .skip(start + 1)
        .position(|line| {
            config
                .transaction_terms_stop
                .iter()
                .any(|term| line.text.contains(term.as_str()))
        })
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    let header_words: Vec<String> = [
        config.transaction_date_headers.as_slice(),
        config.transaction_description_headers.as_slice(),
        config.transaction_amount_headers.as_slice(),
        config.transaction_balance_headers.as_slice(),
    ]
    .concat();

    let mut matched = 0usize;
    let mut total_unmatched = 0usize;
    let mut unmatched: Vec<String> = Vec::new();
    let mut report = Vec::new();

    for line in lines.iter().take(end).skip(start + 1) {
        // Skip the column-header line itself: it names the columns but is
        // not a data row.
        if header_words.iter().any(|h| line.text.contains(h.as_str())) {
            continue;
        }

        match diagnose_line(config, &line.words) {
            None => matched += 1,
            Some(reason) => {
                total_unmatched += 1;
                if unmatched.len() < MAX_REPORTED_LINES {
                    unmatched.push(format!("line {:?}: {}", line.text, reason));
                }
            }
        }
    }

    report.push(format!(
        "table region: lines {}-{}; lines matching a declared order: {}; lines not matched: {}",
        start + 1,
        end,
        matched,
        total_unmatched,
    ));
    if total_unmatched > MAX_REPORTED_LINES {
        report.push(format!(
            "(showing the first {} unmatched lines of {})",
            MAX_REPORTED_LINES, total_unmatched
        ));
    }
    report.extend(unmatched);
    report
}

/// Try every declared column order against a line. None if any order fits;
/// otherwise the failure reasons of the first two orders.
fn diagnose_line(config: &StatementConfig, words: &[String]) -> Option<String> {
    if config.transaction_formats.is_empty() {
        return Some("transaction_formats is empty".to_string());
    }
    let mut reasons = Vec::new();
    for order in &config.transaction_formats {
        // A matching order ends the diagnosis (None); otherwise record why.
        let reason = diagnose_order(config, words, order)?;
        if reasons.len() < 2 {
            reasons.push(format!("order [{:?}]: {}", order.join(", "), reason));
        }
    }
    Some(format!(
        "no declared order matched — {}",
        reasons.join("; ")
    ))
}

/// First unconsumed word index at or after `from`.
fn find_free(used: &[bool], from: usize, len: usize) -> Option<usize> {
    (from..len).find(|&index| !used[index])
}

/// Try one column order against a line, returning the first failing column.
fn diagnose_order(config: &StatementConfig, words: &[String], order: &[String]) -> Option<String> {
    let mut used = vec![false; words.len()];

    for column in order {
        match column.as_str() {
            "date" => {
                let mut satisfied = false;
                let mut index = 0;
                while let Some(at) = find_free(&used, index, words.len()) {
                    if let Some(consumed) =
                        date_matches_at(&config.transaction_date_formats, words, at)
                    {
                        for slot in used.iter_mut().skip(at).take(consumed) {
                            *slot = true;
                        }
                        satisfied = true;
                        break;
                    }
                    index = at + 1;
                }
                if !satisfied {
                    let hint = date_hint(words, &used);
                    return Some(format!(
                        "date column unsatisfied; declared formats {:?} accept no word{}",
                        config.transaction_date_formats, hint
                    ));
                }
            }
            "amount" | "balance" => {
                let formats = if column == "amount" {
                    &config.transaction_amount_formats
                } else {
                    &config.transaction_balance_formats
                };
                let mut satisfied = false;
                let mut index = 0;
                while let Some(at) = find_free(&used, index, words.len()) {
                    if let Some(consumed) = amount_matches_at(formats, words, at) {
                        for slot in used.iter_mut().skip(at).take(consumed) {
                            *slot = true;
                        }
                        satisfied = true;
                        break;
                    }
                    index = at + 1;
                }
                if !satisfied {
                    let hint = amount_hint(words, &used, column, formats);
                    return Some(format!(
                        "{} column unsatisfied; declared formats {:?} accept no word{}",
                        column, formats, hint
                    ));
                }
            }
            "description" => {
                if find_free(&used, 0, words.len()).is_none() {
                    return Some("description column unsatisfied; no words remain".to_string());
                }
                // Descriptions leave the remaining words for later columns;
                // the diagnostic only verifies one is available.
            }
            other => {
                return Some(format!(
                    "unknown column name {other:?} in transaction_formats"
                ));
            }
        }
    }
    None
}

/// Hint text naming an undeclared date format present on the line.
fn date_hint(words: &[String], used: &[bool]) -> String {
    for index in 0..words.len() {
        if used[index] {
            continue;
        }
        let pair: Vec<&str> = words[index..(index + 2).min(words.len())]
            .iter()
            .map(String::as_str)
            .collect();
        if let Some(name) = which_format(&ALL_DATE_FORMATS, &pair, true) {
            return format!(
                " (token {:?} is a date in {} format — add it to transaction_date_formats)",
                pair.join(" "),
                name
            );
        }
    }
    String::new()
}

/// Hint text naming an undeclared amount format present on the line.
fn amount_hint(words: &[String], used: &[bool], column: &str, formats: &[String]) -> String {
    for index in 0..words.len() {
        if used[index] {
            continue;
        }
        let pair: Vec<&str> = words[index..(index + 2).min(words.len())]
            .iter()
            .map(String::as_str)
            .collect();
        if let Some(name) = which_format(&ALL_AMOUNT_FORMATS, &pair, false) {
            return format!(
                " (token {:?} is an amount in {} format — add it to \
transaction_{}_formats, currently {:?})",
                pair.join(" "),
                name,
                column,
                formats
            );
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::field_reassign_with_default)]
    fn config_with(formats: Vec<Vec<&str>>, amount: &[&str], date: &[&str]) -> StatementConfig {
        let mut config = StatementConfig::default();
        config.key = "test".to_string();
        config.transaction_terms = vec!["Transactions".to_string()];
        config.transaction_terms_stop = vec!["End".to_string()];
        config.transaction_formats = formats
            .into_iter()
            .map(|order| order.into_iter().map(String::from).collect())
            .collect();
        config.transaction_amount_formats = amount.iter().map(|s| s.to_string()).collect();
        config.transaction_balance_formats = amount.iter().map(|s| s.to_string()).collect();
        config.transaction_date_formats = date.iter().map(|s| s.to_string()).collect();
        config
    }

    fn item(text: &str, x: i32, y: i32) -> TextItem {
        TextItem::new(text.to_string(), x, y, x + 10, y + 10, 0)
    }

    #[test]
    fn test_matching_line_is_counted() {
        let config = config_with(
            vec![vec!["date", "description", "amount"]],
            &["format1"],
            &["format1"],
        );
        // "01 Jan" arrives as one merged item; tokenisation splits it so the
        // two-word date format can consume it.
        let items = vec![
            item("Transactions", 0, 10),
            item("01 Jan", 0, 30),
            item("Coffee", 60, 30),
            item("5.00", 200, 30),
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        assert!(
            report[0].contains("matching a declared order: 1"),
            "got: {:?}",
            report
        );
        assert_eq!(report.len(), 1);
    }

    #[test]
    fn test_undeclared_format_is_named() {
        let config = config_with(
            vec![vec!["date", "description", "amount"]],
            &["format4"], // amount "5.00" is format1, not format4
            &["format1"],
        );
        let items = vec![
            item("Transactions", 0, 10),
            item("01 Jan", 0, 30),
            item("Coffee", 60, 30),
            item("5.00", 200, 30),
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        assert!(report[1].contains("amount column unsatisfied"));
        assert!(report[1].contains("format1"), "got: {:?}", report[1]);
        assert!(report[1].contains("add it to"), "got: {:?}", report[1]);
    }

    #[test]
    fn test_any_order_matching_satisfies_line() {
        let config = config_with(
            vec![
                vec!["date", "description", "amount", "balance"],
                vec!["description", "amount"],
            ],
            &["format1"],
            &["format1"],
        );
        // Continuation row without a date matches the second order.
        let items = vec![
            item("Transactions", 0, 10),
            item("Coffee", 60, 30),
            item("5.00", 200, 30),
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        assert!(
            report[0].contains("matching a declared order: 1"),
            "got: {:?}",
            report
        );
    }

    #[test]
    fn test_multi_item_balance_format_is_matched() {
        // Balance "1,234.56 CR" arrives as one merged item; format4 consumes
        // the two joined words.
        let mut config = config_with(
            vec![vec!["date", "description", "amount", "balance"]],
            &["format1"],
            &["format1"],
        );
        config.transaction_balance_formats = vec!["format4".to_string()];
        let items = vec![
            item("Transactions", 0, 10),
            item("01 Jan", 0, 30),
            item("Coffee", 60, 30),
            item("5.00", 200, 30),
            item("1,234.56 CR", 300, 30),
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        assert!(
            report[0].contains("matching a declared order: 1"),
            "got: {:?}",
            report
        );
    }

    #[test]
    fn test_unparseable_balance_is_reported() {
        // "90,350 CR" (no decimals) is rejected by every amount format; the
        // engine handles such columns by implicitly computing balances, so
        // the diagnostic reports the column as unsatisfiable as-declared.
        let mut config = config_with(
            vec![vec!["date", "description", "amount", "balance"]],
            &["format1"],
            &["format1"],
        );
        config.transaction_balance_formats = vec!["format4".to_string()];
        let items = vec![
            item("Transactions", 0, 10),
            item("01 Jan", 0, 30),
            item("Coffee", 60, 30),
            item("5.00", 200, 30),
            item("90,350 CR", 300, 30),
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        assert!(
            report[1].contains("balance column unsatisfied"),
            "got: {:?}",
            report
        );
    }
}
