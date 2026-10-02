use crate::structs::StatementData;
use std::collections::HashMap;

/// Reset prototransaction indices based on their order within each day.
/// This fixer should be applied after implicit_balance to lock the order
/// so that transactions can be reordered safely without breaking the running balance.
///
/// If any transaction lacks a date, the fixer records an error on the
/// statement and returns (callers treat non-empty errors as a parse failure).
pub fn fix_set_indices(sd: &mut StatementData) {
    if sd.proto_transactions.is_empty() {
        return;
    }

    let mut index_for_date = HashMap::new();

    for (i, proto_transaction) in sd.proto_transactions.iter_mut().enumerate() {
        // Validate that transaction has a date. An imperfect configuration
        // can leave dates missing; report through the error channel instead
        // of panicking.
        let current_date = match proto_transaction.date {
            Some(date) => date,
            None => {
                sd.add_error(format!(
                    "Transaction at position {i} does not have a date. This should not \
happen after date fixers. Check the date column configuration."
                ));
                return;
            }
        };

        // If date not encountered before, initialize its index to 0 otherwise increment the existing index.
        let day_index = index_for_date.entry(current_date).or_insert(0);
        let day_index = *day_index;

        // Set the index for this transaction
        proto_transaction.index = day_index;
        *index_for_date.get_mut(&current_date).unwrap() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::{ProtoTransaction, StatementData};

    fn create_proto_transaction(date: i64, index: usize) -> ProtoTransaction {
        ProtoTransaction {
            date: Some(date),
            index,
            description: format!("Transaction {}", index),
            amount: Some(100.0),
            balance: None,
        }
    }

    #[test]
    fn test_fix_set_indices_empty_transactions() {
        let mut sd = StatementData {
            proto_transactions: vec![],
            account_number: None,
            opening_balance: None,
            closing_balance: None,
            start_date: None,
            start_date_year: None,
            key: None,
            errors: Vec::new(),
            benchmark: Default::default(),
        };

        fix_set_indices(&mut sd);
        assert_eq!(sd.proto_transactions.len(), 0);
    }

    #[test]
    fn test_fix_set_indices_single_transaction() {
        let mut sd = StatementData {
            proto_transactions: vec![create_proto_transaction(1000, 5)],
            account_number: None,
            opening_balance: None,
            closing_balance: None,
            start_date: None,
            start_date_year: None,
            key: None,
            errors: Vec::new(),
            benchmark: Default::default(),
        };

        fix_set_indices(&mut sd);
        assert_eq!(sd.proto_transactions[0].index, 0);
    }

    #[test]
    fn test_fix_set_indices_all_none() {
        let mut sd = StatementData {
            proto_transactions: vec![
                create_proto_transaction(1000, 5),
                create_proto_transaction(1000, 10),
                create_proto_transaction(1000, 15),
            ],
            account_number: None,
            opening_balance: None,
            closing_balance: None,
            start_date: None,
            start_date_year: None,
            key: None,
            errors: Vec::new(),
            benchmark: Default::default(),
        };

        fix_set_indices(&mut sd);

        // All should have sequential indices within the same day
        assert_eq!(sd.proto_transactions[0].index, 0);
        assert_eq!(sd.proto_transactions[1].index, 1);
        assert_eq!(sd.proto_transactions[2].index, 2);
    }

    #[test]
    fn test_fix_set_indices_different_days() {
        let mut sd = StatementData {
            proto_transactions: vec![
                create_proto_transaction(1000, 10), // Day 1, transaction 0
                create_proto_transaction(1000, 20), // Day 1, transaction 1
                create_proto_transaction(2000, 30), // Day 2, transaction 0
                create_proto_transaction(2000, 40), // Day 2, transaction 1
                create_proto_transaction(3000, 50), // Day 3, transaction 0
            ],
            account_number: None,
            opening_balance: None,
            closing_balance: None,
            start_date: None,
            start_date_year: None,
            key: None,
            errors: Vec::new(),
            benchmark: Default::default(),
        };

        fix_set_indices(&mut sd);

        // Day 1 transactions
        assert_eq!(sd.proto_transactions[0].index, 0);
        assert_eq!(sd.proto_transactions[1].index, 1);

        // Day 2 transactions (indices reset)
        assert_eq!(sd.proto_transactions[2].index, 0);
        assert_eq!(sd.proto_transactions[3].index, 1);

        // Day 3 transaction (index reset)
        assert_eq!(sd.proto_transactions[4].index, 0);
    }

    #[test]
    fn test_missing_date_reports_error_not_panic() {
        let mut sd = StatementData {
            proto_transactions: vec![
                create_proto_transaction(1000, 0),
                ProtoTransaction {
                    date: None, // Missing date: recorded as an error, no panic
                    index: 1,
                    description: "No date transaction".to_string(),
                    amount: Some(100.0),
                    balance: None,
                },
            ],
            account_number: None,
            opening_balance: None,
            closing_balance: None,
            start_date: None,
            start_date_year: None,
            key: None,
            errors: Vec::new(),
            benchmark: Default::default(),
        };

        fix_set_indices(&mut sd);

        assert_eq!(sd.errors.len(), 1);
        assert!(
            sd.errors[0].contains("Transaction at position 1 does not have a date"),
            "got: {:?}",
            sd.errors
        );
    }
}
