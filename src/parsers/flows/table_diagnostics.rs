//! Table diagnostics: explain, per table line, why it did or did not match
//! the configured column orders, formats AND column alignments.
//!
//! This is an explanatory pass, not the extraction path: it replays the
//! table region line by line against the configured `transaction_formats`
//! orders using the engine's own format parsers, and names the first column
//! that could not be satisfied — a token matching an undeclared format, or a
//! token that matches a format but sits at the wrong x-position for the
//! declared column alignment. It also warns when a table has multiple money
//! columns but no alignment anchoring them. Intended for configuration
//! authoring (humans and LLM-assisted harnesses).
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

/// A tokenised word, carrying its (item-level) x-extent for alignment checks.
#[derive(Debug)]
struct Word {
    text: String,
    x1: i32,
    x2: i32,
}

/// A tokenised line: individual words in reading order.
#[derive(Debug)]
struct Line {
    text: String,
    words: Vec<Word>,
}

/// Column-header x-extents used as alignment anchors.
#[derive(Debug, Default)]
struct HeaderAnchors {
    date: Option<(i32, i32)>,
    amount: Option<(i32, i32)>,
    balance: Option<(i32, i32)>,
}

/// Group pre-sorted text items into lines by (page, y1_bin) and split each
/// line into words, mirroring the pipeline's tokeniser. Words inherit their
/// item's x-extent (approximate for merged items, exact for separate ones).
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
            let words: Vec<Word> = items
                .iter()
                .flat_map(|item| {
                    item.text.split_whitespace().map(|word| Word {
                        text: word.to_string(),
                        x1: item.x1,
                        x2: item.x2,
                    })
                })
                .collect();
            let text = words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            Line { text, words }
        })
        .filter(|line| !line.words.is_empty())
        .collect()
}

/// x-extent of the first word containing any of the declared header terms.
fn header_extent(line: &Line, headers: &[String]) -> Option<(i32, i32)> {
    let word = line
        .words
        .iter()
        .find(|w| headers.iter().any(|h| w.text.contains(h.as_str())))?;
    Some((word.x1, word.x2))
}

/// Check a candidate word against a declared column alignment.
fn aligned(word: &Word, alignment: &str, header: Option<(i32, i32)>, tol: i32) -> bool {
    match (alignment, header) {
        ("x1", Some((hx1, _))) => (word.x1 - hx1).abs() <= tol,
        ("x2", Some((_, hx2))) => (word.x2 - hx2).abs() <= tol,
        _ => true, // no constraint, or header position unknown
    }
}

/// Try to match a date at `index`, consuming one or two adjacent words
/// (multi-item formats parse "01" + "Jan" joined). A nominal year is
/// supplied for year-less formats; it does not change structural matching.
fn date_matches_at(formats: &[String], words: &[Word], index: usize) -> Option<usize> {
    let names: Vec<&str> = formats.iter().map(String::as_str).collect();
    let parser = MultiDateFormatParser::new(&names);
    if parser.parse(&words[index].text, "2025", 1).is_some() {
        return Some(1);
    }
    if index + 1 < words.len() {
        let joined = format!("{} {}", words[index].text, words[index + 1].text);
        if parser.parse(&joined, "2025", 2).is_some() {
            return Some(2);
        }
    }
    None
}

/// Try to match an amount at `index`, consuming one or two adjacent words
/// (multi-item formats parse "90,350" + "CR" joined).
fn amount_matches_at(formats: &[String], words: &[Word], index: usize) -> Option<usize> {
    let names: Vec<&str> = formats.iter().map(String::as_str).collect();
    let parser = MultiAmountFormatParser::new(&names);
    if parser.parse(&words[index].text, 1).is_some() {
        return Some(1);
    }
    if index + 1 < words.len() {
        let joined = format!("{} {}", words[index].text, words[index + 1].text);
        if parser.parse(&joined, 2).is_some() {
            return Some(2);
        }
    }
    None
}

/// Whether any declared amount format accepts the word (any item count).
fn word_is_amount(formats: &[String], word: &Word) -> bool {
    let names: Vec<&str> = formats.iter().map(String::as_str).collect();
    let parser = MultiAmountFormatParser::new(&names);
    parser.parse(&word.text, 1).is_some()
}

/// Find a format name (from `candidates`) that accepts the word texts,
/// trying the single and joined forms like the matchers above.
fn which_format<'a>(candidates: &[&'a str], texts: &[&str], date: bool) -> Option<&'a str> {
    let single = texts[0].to_string();
    let joined = if texts.len() > 1 {
        Some(texts.join(" "))
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

    let mut anchors = HeaderAnchors::default();
    let mut matched = 0usize;
    let mut total_unmatched = 0usize;
    let mut unmatched: Vec<String> = Vec::new();
    let mut report = Vec::new();
    // Votes from the data: per money column (0 = amount, 1 = balance), how
    // many multi-money-line tokens align with the header by left edge (x1)
    // vs right edge (x2).
    let mut votes: [(usize, usize); 2] = [(0, 0), (0, 0)];
    let tol = config.transaction_alignment_tol;

    for line in lines.iter().take(end).skip(start + 1) {
        // The column-header line names the columns but is not a data row;
        // record its header positions as alignment anchors.
        if header_words.iter().any(|h| line.text.contains(h.as_str())) {
            anchors.date = anchors
                .date
                .or_else(|| header_extent(line, &config.transaction_date_headers));
            anchors.amount = anchors
                .amount
                .or_else(|| header_extent(line, &config.transaction_amount_headers));
            anchors.balance = anchors
                .balance
                .or_else(|| header_extent(line, &config.transaction_balance_headers));
            continue;
        }

        let money: Vec<&Word> = line
            .words
            .iter()
            .filter(|w| word_is_amount(&config.transaction_amount_formats, w))
            .collect();
        if money.len() >= 2 {
            // First money column votes for the amount anchor, last for the
            // balance anchor.
            for (slot, word) in [(0usize, money[0]), (1usize, money[money.len() - 1])] {
                let anchor = if slot == 0 {
                    anchors.amount
                } else {
                    anchors.balance
                };
                let Some((hx1, hx2)) = anchor else {
                    continue;
                };
                if (word.x1 - hx1).abs() <= tol {
                    votes[slot].0 += 1;
                }
                if (word.x2 - hx2).abs() <= tol {
                    votes[slot].1 += 1;
                }
            }
        }

        match diagnose_line(config, line, &anchors) {
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

    // The silent row-killer: the declared money-column alignment disagrees
    // with where the tokens actually sit. Data votes win.
    let declares_money_columns = config
        .transaction_formats
        .iter()
        .any(|o| o.iter().any(|c| c == "amount" || c == "balance"));
    if declares_money_columns {
        for (column, alignment, (x1_votes, x2_votes)) in [
            ("amount", &config.transaction_amount_alignment, votes[0]),
            ("balance", &config.transaction_balance_alignment, votes[1]),
        ] {
            let data_alignment = if x2_votes > x1_votes {
                "x2"
            } else if x1_votes > x2_votes {
                "x1"
            } else {
                ""
            };
            if !data_alignment.is_empty() && x1_votes + x2_votes > 0 && alignment != data_alignment
            {
                report.push(format!(
                    "WARNING: transaction_{column}_alignment is {alignment:?} but the \
table's own tokens disagree — {x2_votes} money tokens right-align with the header (x2) \
vs {x1_votes} left-align (x1). Set transaction_{column}_alignment to \"{data_alignment}\" \
or rows will mis-bind and drop."
                ));
            }
        }
    }

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
fn diagnose_line(config: &StatementConfig, line: &Line, anchors: &HeaderAnchors) -> Option<String> {
    if config.transaction_formats.is_empty() {
        return Some("transaction_formats is empty".to_string());
    }
    let mut reasons = Vec::new();
    for order in &config.transaction_formats {
        // A matching order ends the diagnosis (None); otherwise record why.
        let reason = diagnose_order(config, line, order, anchors)?;
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
fn diagnose_order(
    config: &StatementConfig,
    line: &Line,
    order: &[String],
    anchors: &HeaderAnchors,
) -> Option<String> {
    let words = &line.words;
    let mut used = vec![false; words.len()];

    for column in order {
        match column.as_str() {
            "date" => {
                let mut satisfied = false;
                let mut format_matched: Option<usize> = None;
                let mut index = 0;
                while let Some(at) = find_free(&used, index, words.len()) {
                    if let Some(consumed) =
                        date_matches_at(&config.transaction_date_formats, words, at)
                    {
                        if aligned(
                            &words[at],
                            &config.transaction_date_alignment,
                            anchors.date,
                            config.transaction_alignment_tol,
                        ) {
                            for slot in used.iter_mut().skip(at).take(consumed) {
                                *slot = true;
                            }
                            satisfied = true;
                            break;
                        }
                        format_matched = format_matched.or(Some(at));
                    }
                    index = at + 1;
                }
                if !satisfied {
                    if let Some(at) = format_matched {
                        return Some(misalignment_reason(
                            "date",
                            &words[at],
                            &config.transaction_date_alignment,
                            anchors.date,
                            config.transaction_alignment_tol,
                        ));
                    }
                    let hint = date_hint(words, &used);
                    return Some(format!(
                        "date column unsatisfied; declared formats {:?} accept no word{}",
                        config.transaction_date_formats, hint
                    ));
                }
            }
            "amount" | "balance" => {
                let (formats, alignment, anchor) = if column == "amount" {
                    (
                        &config.transaction_amount_formats,
                        &config.transaction_amount_alignment,
                        anchors.amount,
                    )
                } else {
                    (
                        &config.transaction_balance_formats,
                        &config.transaction_balance_alignment,
                        anchors.balance,
                    )
                };
                let mut satisfied = false;
                let mut format_matched: Option<usize> = None;
                let mut index = 0;
                while let Some(at) = find_free(&used, index, words.len()) {
                    if let Some(consumed) = amount_matches_at(formats, words, at) {
                        if aligned(
                            &words[at],
                            alignment,
                            anchor,
                            config.transaction_alignment_tol,
                        ) {
                            for slot in used.iter_mut().skip(at).take(consumed) {
                                *slot = true;
                            }
                            satisfied = true;
                            break;
                        }
                        format_matched = format_matched.or(Some(at));
                    }
                    index = at + 1;
                }
                if !satisfied {
                    if let Some(at) = format_matched {
                        return Some(misalignment_reason(
                            column,
                            &words[at],
                            alignment,
                            anchor,
                            config.transaction_alignment_tol,
                        ));
                    }
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

/// Failure text for a token that matches a format but fails the alignment
/// constraint, naming both positions so the fix is mechanical.
fn misalignment_reason(
    column: &str,
    word: &Word,
    alignment: &str,
    anchor: Option<(i32, i32)>,
    tol: i32,
) -> String {
    let Some((hx1, hx2)) = anchor else {
        return format!(
            "{column} column: token {:?} matches a declared format but no column header \
was found to anchor alignment against — check the *_headers fields",
            word.text
        );
    };
    match alignment {
        "x1" => format!(
            "{column} column: token {:?} matches a declared format but is misaligned \
(x1={} vs header x1={}, tol={}) — check transaction_{column}_alignment",
            word.text, word.x1, hx1, tol
        ),
        "x2" => format!(
            "{column} column: token {:?} matches a declared format but is misaligned \
(x2={} vs header x2={}, tol={}) — check transaction_{column}_alignment",
            word.text, word.x2, hx2, tol
        ),
        _ => format!(
            "{column} column: token {:?} matches a declared format but \
transaction_{column}_alignment is not set while multiple money columns are present — \
set it (\"x2\" for right-aligned money columns)",
            word.text
        ),
    }
}

/// Hint text naming an undeclared date format present on the line.
fn date_hint(words: &[Word], used: &[bool]) -> String {
    for index in 0..words.len() {
        if used[index] {
            continue;
        }
        let pair: Vec<&str> = words[index..(index + 2).min(words.len())]
            .iter()
            .map(|w| w.text.as_str())
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
fn amount_hint(words: &[Word], used: &[bool], column: &str, formats: &[String]) -> String {
    for index in 0..words.len() {
        if used[index] {
            continue;
        }
        let pair: Vec<&str> = words[index..(index + 2).min(words.len())]
            .iter()
            .map(|w| w.text.as_str())
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
        config.transaction_date_headers = vec!["Date".to_string()];
        config.transaction_amount_headers = vec!["Amount".to_string()];
        config.transaction_balance_headers = vec!["Balance".to_string()];
        config
    }

    fn item(text: &str, x1: i32, y: i32) -> TextItem {
        TextItem::new(
            text.to_string(),
            x1,
            y,
            x1 + text.len() as i32 * 6,
            y + 10,
            0,
        )
    }

    /// Item with an explicit x2, for alignment-sensitive fixtures.
    fn item_x2(text: &str, x1: i32, x2: i32, y: i32) -> TextItem {
        TextItem::new(text.to_string(), x1, y, x2, y + 10, 0)
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
    fn test_data_voted_alignment_mismatch_warns() {
        // Money tokens RIGHT-align with their headers, but the default
        // amount alignment anchors the left edge: the data-vote warning
        // names the fix (this is the test1.pdf failure signature).
        let config = config_with(
            vec![vec!["date", "description", "amount", "balance"]],
            &["format1"],
            &["format1"],
        );
        let items = vec![
            item("Transactions", 0, 10),
            item_x2("Date", 0, 40, 20),
            item_x2("Description", 60, 140, 20),
            item_x2("Amount", 200, 260, 20),
            item_x2("Balance", 300, 360, 20),
            item_x2("01 Jan", 0, 40, 30),
            item_x2("Coffee", 60, 140, 30),
            item_x2("5.00", 230, 260, 30), // right-aligned under Amount
            item_x2("6.00", 330, 360, 30), // right-aligned under Balance
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        let joined = report.join("\n");
        assert!(
            joined.contains("WARNING: transaction_amount_alignment"),
            "got: {:?}",
            report
        );
        assert!(joined.contains("to \"x2\""), "got: {:?}", report);
    }

    #[test]
    fn test_misaligned_token_with_declared_alignment_is_named() {
        // Alignment x2 declared against the Amount header, but the line's
        // money token sits under the Balance column instead.
        let mut config = config_with(
            vec![vec!["date", "description", "amount"]],
            &["format1"],
            &["format1"],
        );
        config.transaction_amount_alignment = "x2".to_string();
        config.transaction_alignment_tol = 10;
        let items = vec![
            item("Transactions", 0, 10),
            item_x2("Date", 0, 40, 20),
            item_x2("Description", 60, 140, 20),
            item_x2("Amount", 200, 260, 20),
            item_x2("01 Jan", 0, 40, 30),
            item_x2("Coffee", 60, 140, 30),
            item_x2("5.00", 330, 360, 30), // right-aligned under Balance, not Amount
            item("End", 0, 40),
        ];
        let report = table_diagnostics(&config, &items);
        let joined = report.join("\n");
        assert!(
            joined.contains("matches a declared format but is misaligned"),
            "got: {:?}",
            report
        );
        assert!(joined.contains("transaction_amount_alignment"));
    }

    #[test]
    fn test_aligned_token_with_declared_alignment_passes() {
        let mut config = config_with(
            vec![vec!["date", "description", "amount"]],
            &["format1"],
            &["format1"],
        );
        config.transaction_amount_alignment = "x2".to_string();
        let items = vec![
            item("Transactions", 0, 10),
            item_x2("Date", 0, 40, 20),
            item_x2("Description", 60, 140, 20),
            item_x2("Amount", 200, 260, 20),
            item_x2("01 Jan", 0, 40, 30),
            item_x2("Coffee", 60, 140, 30),
            item_x2("5.00", 230, 260, 30), // right edge matches the Amount header
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
        let joined = report.join("\n");
        assert!(
            joined.contains("matching a declared order: 1"),
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
        let joined = report.join("\n");
        assert!(
            joined.contains("balance column unsatisfied"),
            "got: {:?}",
            report
        );
    }
}
