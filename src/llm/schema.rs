//! LLM-facing statement contract.
//!
//! A simplified, LLM-friendly mirror of `StatementData`: ISO `YYYY-MM-DD`
//! dates and plain decimal numbers instead of millisecond timestamps and
//! optional fields, which models handle poorly.

use crate::structs::{ProtoTransaction, StatementData};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A single transaction as returned by the LLM.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmTransaction {
    /// Transaction date as `YYYY-MM-DD`
    pub date: String,
    /// Transaction description as printed on the statement
    pub description: String,
    /// Signed amount: negative = money out, positive = money in
    pub amount: f64,
    /// Running balance after the transaction, when the statement shows one
    #[serde(default)]
    pub balance: Option<f64>,
}

/// A full bank statement as returned by the LLM.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmStatement {
    /// Account number as printed on the statement
    pub account_number: String,
    /// Statement period start date as `YYYY-MM-DD`; the earliest transaction
    /// date when the period start is not printed
    pub start_date: String,
    /// Balance before the first transaction
    pub opening_balance: f64,
    /// Balance after the last transaction
    pub closing_balance: f64,
    /// Transactions in statement order
    pub transactions: Vec<LlmTransaction>,
}

/// Parse an ISO `YYYY-MM-DD` date into milliseconds since the Unix epoch.
fn iso_date_to_millis(date: &str) -> Result<i64, String> {
    let trimmed = date.trim();
    // The model may wrap the date in quotes inside the JSON string value;
    // stripping them is harmless for the plain format.
    let trimmed = trimmed.trim_matches('"');
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map_err(|e| format!("Invalid date '{}': {}", date, e))
        .and_then(|d| {
            d.and_hms_opt(0, 0, 0).ok_or_else(|| {
                format!(
                    "Invalid date '{}': could not construct midnight timestamp",
                    date
                )
            })
        })
        .map(|dt| dt.and_utc().timestamp_millis())
}

impl LlmStatement {
    /// Convert into a `StatementData` ready for the standard fixer and
    /// checker pipeline. `provenance_key` (e.g. `llm/qwen-vl`) is stored as
    /// the statement key so downstream consumers can tell LLM-extracted
    /// statements from rules-parsed ones.
    pub fn to_statement_data(self, provenance_key: &str) -> Result<StatementData, String> {
        let start_ms = iso_date_to_millis(&self.start_date)?;
        let mut data = StatementData::new();
        data.set_key(provenance_key.to_string());
        data.set_account_number(self.account_number.trim().to_string());
        data.set_start_date(start_ms);
        data.set_opening_balance(self.opening_balance);
        data.set_closing_balance(self.closing_balance);
        for (index, tx) in self.transactions.into_iter().enumerate() {
            let date_ms = iso_date_to_millis(&tx.date)?;
            let mut proto = ProtoTransaction::new();
            proto.set_date(date_ms);
            proto.set_index(index);
            proto.description = tx.description.trim().to_string();
            proto.set_amount(tx.amount);
            if let Some(balance) = tx.balance {
                proto.set_balance(balance);
            }
            data.add_proto_transaction(proto);
        }
        Ok(data)
    }
}

/// JSON Schema describing [`LlmStatement`], used for `response_format`
/// guided decoding on endpoints that support it.
pub fn llm_response_json_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "account_number": {
                "type": "string",
                "description": "Account number copied exactly as printed, preserving any spaces or dashes"
            },
            "start_date": {
                "type": "string",
                "description": "Statement period start date as YYYY-MM-DD"
            },
            "opening_balance": {
                "type": "number",
                "description": "Balance before the first transaction"
            },
            "closing_balance": {
                "type": "number",
                "description": "Balance after the last transaction"
            },
            "transactions": {
                "type": "array",
                "description": "Every transaction ledger row, in statement order",
                "items": {
                    "type": "object",
                    "properties": {
                        "date": {
                            "type": "string",
                            "description": "Transaction date as YYYY-MM-DD"
                        },
                        "description": {
                            "type": "string",
                            "description": "Transaction description as printed"
                        },
                        "amount": {
                            "type": "number",
                            "description": "Signed amount: negative = money out, positive = money in"
                        },
                        "balance": {
                            "type": "number",
                            "description": "Running balance printed on the row; omit when not shown"
                        }
                    },
                    "required": ["date", "description", "amount"]
                }
            }
        },
        "required": [
            "account_number",
            "start_date",
            "opening_balance",
            "closing_balance",
            "transactions"
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_statement() -> LlmStatement {
        LlmStatement {
            account_number: "1234 5678 9123 4567".to_string(),
            start_date: "2025-01-01".to_string(),
            opening_balance: 1000.0,
            closing_balance: 900.0,
            transactions: vec![
                LlmTransaction {
                    date: "2025-01-02".to_string(),
                    description: " EFTPOS WDL MELBOURNE ".to_string(),
                    amount: -150.0,
                    balance: Some(850.0),
                },
                LlmTransaction {
                    date: "2025-01-03".to_string(),
                    description: "Deposit".to_string(),
                    amount: 50.0,
                    balance: None,
                },
            ],
        }
    }

    #[test]
    fn test_to_statement_data_happy_path() {
        let data = sample_statement()
            .to_statement_data("llm/mock-model")
            .expect("Expected conversion to succeed");

        assert_eq!(data.key.as_deref(), Some("llm/mock-model"));
        assert_eq!(data.account_number.as_deref(), Some("1234 5678 9123 4567"));
        assert_eq!(data.start_date, Some(1735689600000));
        assert_eq!(data.start_date_year, Some(2025));
        assert_eq!(data.opening_balance, Some(1000.0));
        assert_eq!(data.closing_balance, Some(900.0));
        assert_eq!(data.proto_transactions.len(), 2);

        let first = &data.proto_transactions[0];
        assert_eq!(first.date, Some(1735776000000)); // 2025-01-02
        assert_eq!(first.description, "EFTPOS WDL MELBOURNE");
        assert_eq!(first.amount, Some(-150.0));
        assert_eq!(first.balance, Some(850.0));

        let second = &data.proto_transactions[1];
        assert_eq!(second.amount, Some(50.0));
        assert_eq!(second.balance, None);
    }

    #[test]
    fn test_to_statement_data_rejects_invalid_start_date() {
        let mut statement = sample_statement();
        statement.start_date = "01/02/2025".to_string();
        let result = statement.to_statement_data("llm/mock-model");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid date '01/02/2025'"));
    }

    #[test]
    fn test_to_statement_data_rejects_invalid_transaction_date() {
        let mut statement = sample_statement();
        statement.transactions[0].date = "2025-13-40".to_string();
        let result = statement.to_statement_data("llm/mock-model");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid date"));
    }

    #[test]
    fn test_llm_statement_round_trips_through_json() {
        let statement = sample_statement();
        let json = serde_json::to_string(&statement).expect("serialise");
        let parsed: LlmStatement = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(parsed.start_date, statement.start_date);
        assert_eq!(parsed.transactions.len(), 2);
        assert_eq!(parsed.transactions[1].balance, None);
    }
}
