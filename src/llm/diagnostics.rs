//! Pattern-driven diagnosis of checker failures for the correction loop.
//!
//! The checker errors carry a failure signature; this module classifies it
//! and emits remediation hints targeted at what actually went wrong (a
//! broken balance chain, a dropped row, a sign flip), rather than generic
//! advice. Hints may cross-reference amounts from the previous attempt when
//! the arithmetic identifies a likely culprit row.

use crate::llm::schema::LlmStatement;
use regex::Regex;

/// A classified failure with targeted remediation hints.
pub struct FailureDiagnosis {
    /// Machine-readable pattern name, e.g. `final-only-total-gap`.
    pub pattern: &'static str,
    /// Targeted guidance lines for the model (may be empty for unknown
    /// patterns; the correction prompt then falls back to the raw errors).
    pub hints: Vec<String>,
}

/// A row-level balance mismatch parsed from a checker error string.
struct RowMismatch {
    row: usize,
    difference: f64,
}

/// Maximum number of amount-candidate hints to emit per diagnosis.
const MAX_AMOUNT_CANDIDATES: usize = 3;

/// Two amounts are considered equal at the checker's rounding precision.
fn amounts_match(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.011
}

/// Classify the checker errors of a failed attempt and produce hints.
pub fn diagnose(errors: &[String], previous: &LlmStatement) -> FailureDiagnosis {
    let row_re = Regex::new(
        r"^Transaction (\d+) balance mismatch\. Calculated: [-\d.]+, Stated: [-\d.]+, Difference: ([-\d.]+)$",
    )
    .expect("static row mismatch regex must compile");
    let final_re = Regex::new(
        r"^Final balance mismatch\. Calculated: ([-\d.]+), Stated: ([-\d.]+), Difference: ([-\d.]+)$",
    )
    .expect("static final mismatch regex must compile");

    let missing_fields = errors
        .iter()
        .find(|e| e.starts_with("Missing required fields: "))
        .map(|e| e["Missing required fields: ".len()..].to_string());
    let rows: Vec<RowMismatch> = errors
        .iter()
        .filter_map(|e| {
            let captures = row_re.captures(e)?;
            Some(RowMismatch {
                row: captures[1].parse::<usize>().ok()?,
                difference: captures[2].parse::<f64>().ok()?,
            })
        })
        .collect();
    let final_gap: Option<f64> = errors
        .iter()
        .filter_map(|e| {
            final_re.captures(e).map(|c| {
                let calculated: f64 = c[1].parse().unwrap_or(0.0);
                let stated: f64 = c[2].parse().unwrap_or(0.0);
                calculated - stated
            })
        })
        .next();

    // Pattern 1: statement-level fields missing — extraction missed header data.
    if let Some(fields) = missing_fields {
        return FailureDiagnosis {
            pattern: "missing-header-fields",
            hints: vec![format!(
                "Required statement fields are missing: {}. Find them in the statement header \
or footer: the account number, and the balances printed immediately before the first and \
after the last transaction.",
                fields
            )],
        };
    }

    // Pattern 2: row-level chain break.
    if !rows.is_empty() {
        let first = rows
            .iter()
            .min_by_key(|r| r.row)
            .expect("rows is non-empty");
        let constant_offset = rows
            .iter()
            .all(|r| amounts_match(r.difference, first.difference));
        let mut hints = Vec::new();
        if constant_offset {
            hints.push(format!(
                "All row mismatches share the same offset ({:.2}), so the running-balance \
chain broke at transaction {} and every later row inherited the error. Fix transaction {}: \
compare its amount, sign and balance against the document; the later rows are probably \
correct as printed.",
                first.difference, first.row, first.row
            ));
            // The offset magnitude identifies candidate culprit amounts:
            // a sign flip on amount X offsets the total by 2X.
            for (index, tx) in previous.transactions.iter().enumerate() {
                if hints.len() > MAX_AMOUNT_CANDIDATES {
                    break;
                }
                if amounts_match(first.difference.abs(), 2.0 * tx.amount.abs()) {
                    hints.push(format!(
                        "The offset is twice the amount of transaction {} (\"{}\", {:.2}) — \
its sign is likely flipped.",
                        index + 1,
                        tx.description,
                        tx.amount
                    ));
                }
            }
            return FailureDiagnosis {
                pattern: "chain-break-constant-offset",
                hints,
            };
        }
        let named: Vec<String> = rows.iter().map(|r| r.row.to_string()).take(6).collect();
        hints.push(format!(
            "Transactions {} disagree with their stated running balances in different \
amounts, so several rows carry independent errors. Re-check each named row's amount, sign \
and balance against the document.",
            named.join(", ")
        ));
        return FailureDiagnosis {
            pattern: "row-mismatches-varied",
            hints,
        };
    }

    // Pattern 3: totals-only mismatch — internally consistent but wrong total.
    if let Some(gap) = final_gap {
        let mut hints = vec![format!(
            "Every row is internally consistent but the total misses the stated closing \
balance by {:+.2}. This signature means whole rows were dropped or duplicated, or the \
opening/closing balances were misread — not a digit error within a row.",
            gap
        )];
        let mut candidates = 0;
        for (index, tx) in previous.transactions.iter().enumerate() {
            if candidates >= MAX_AMOUNT_CANDIDATES {
                break;
            }
            if amounts_match(gap.abs(), 2.0 * tx.amount.abs()) {
                hints.push(format!(
                    "The gap is twice the amount of transaction {} (\"{}\", {:.2}) — its \
sign may be flipped.",
                    index + 1,
                    tx.description,
                    tx.amount
                ));
                candidates += 1;
            } else if amounts_match(gap, -tx.amount) {
                hints.push(format!(
                    "The gap equals the negated amount of transaction {} (\"{}\", {:.2}) — \
that row may be missing from your answer.",
                    index + 1,
                    tx.description,
                    tx.amount
                ));
                candidates += 1;
            } else if amounts_match(gap, tx.amount) {
                hints.push(format!(
                    "The gap equals the amount of transaction {} (\"{}\", {:.2}) — that \
row may be duplicated in your answer.",
                    index + 1,
                    tx.description,
                    tx.amount
                ));
                candidates += 1;
            }
        }
        hints.push(
            "Recount every ledger row in the document — including rows at page boundaries \
and continuation lines — and verify the opening balance is the balance printed immediately \
before the first transaction and the closing balance immediately after the last."
                .to_string(),
        );
        return FailureDiagnosis {
            pattern: "final-only-total-gap",
            hints,
        };
    }

    // Unknown signature: no targeted advice.
    FailureDiagnosis {
        pattern: "unknown",
        hints: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::schema::LlmTransaction;

    fn previous_with_amounts(amounts: &[f64]) -> LlmStatement {
        LlmStatement {
            account_number: "1234".to_string(),
            start_date: "2025-01-01".to_string(),
            opening_balance: 1000.0,
            closing_balance: 900.0,
            transactions: amounts
                .iter()
                .enumerate()
                .map(|(i, amount)| LlmTransaction {
                    date: "2025-01-02".to_string(),
                    description: format!("Row {}", i + 1),
                    amount: *amount,
                    balance: None,
                })
                .collect(),
        }
    }

    #[test]
    fn test_missing_fields_pattern() {
        let errors = vec!["Missing required fields: account number, opening balance".to_string()];
        let diagnosis = diagnose(&errors, &previous_with_amounts(&[-100.0]));
        assert_eq!(diagnosis.pattern, "missing-header-fields");
        assert!(diagnosis.hints[0].contains("account number, opening balance"));
    }

    #[test]
    fn test_constant_offset_points_at_first_row_and_sign_flip() {
        let errors = vec![
            "Transaction 4 balance mismatch. Calculated: -12350.00, Stated: 90350.00, Difference: 102700.00".to_string(),
            "Transaction 5 balance mismatch. Calculated: -12880.99, Stated: 89819.01, Difference: 102700.00".to_string(),
        ];
        // 102700 == 2 * 51350 -> a transaction with amount 51350 is a candidate.
        let previous = previous_with_amounts(&[-50.0, 51350.0, -20.0]);
        let diagnosis = diagnose(&errors, &previous);
        assert_eq!(diagnosis.pattern, "chain-break-constant-offset");
        assert!(diagnosis.hints[0].contains("broke at transaction 4"));
        assert!(
            diagnosis
                .hints
                .iter()
                .any(|h| h.contains("twice the amount of transaction 2")
                    && h.contains("sign is likely flipped"))
        );
    }

    #[test]
    fn test_varied_row_mismatches() {
        let errors = vec![
            "Transaction 2 balance mismatch. Calculated: 10.00, Stated: 12.00, Difference: 2.00"
                .to_string(),
            "Transaction 7 balance mismatch. Calculated: 30.00, Stated: 25.00, Difference: 5.00"
                .to_string(),
        ];
        let diagnosis = diagnose(&errors, &previous_with_amounts(&[-1.0]));
        assert_eq!(diagnosis.pattern, "row-mismatches-varied");
        assert!(diagnosis.hints[0].contains("Transactions 2, 7"));
    }

    #[test]
    fn test_final_only_gap_identifies_missing_row() {
        // gap == -1234.56 means a row of amount +1234.56 may be missing.
        let errors = vec![
            "Final balance mismatch. Calculated: 8765.44, Stated: 10000.00, Difference: -1234.56"
                .to_string(),
        ];
        let previous = previous_with_amounts(&[500.0, 1234.56, -300.0]);
        let diagnosis = diagnose(&errors, &previous);
        assert_eq!(diagnosis.pattern, "final-only-total-gap");
        assert!(diagnosis.hints[0].contains("dropped or duplicated"));
        assert!(
            diagnosis
                .hints
                .iter()
                .any(|h| h.contains("transaction 2") && h.contains("missing from your answer"))
        );
        assert!(
            diagnosis
                .hints
                .last()
                .unwrap()
                .contains("Recount every ledger row")
        );
    }

    #[test]
    fn test_unknown_pattern_when_balances_cannot_be_checked() {
        let errors =
            vec!["Cannot check balances if opening or closing balance is missing".to_string()];
        let diagnosis = diagnose(&errors, &previous_with_amounts(&[1.0]));
        assert_eq!(diagnosis.pattern, "unknown");
        assert!(diagnosis.hints.is_empty());
    }
}
