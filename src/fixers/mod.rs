use crate::structs::StatementData;

pub mod amounts;
pub mod closing_balance;
pub mod date;
pub mod implicit_balance;
pub mod implicit_date;
pub mod opening_balance;
pub mod set_indices;
pub mod transaction_order;

pub use amounts::fix_amounts;
pub use closing_balance::fix_closing_balance;
pub use date::fix_year_crossovers;
pub use implicit_balance::fix_implicit_balances;
pub use implicit_date::fix_implicit_dates;
pub use opening_balance::fix_opening_balance;
pub use set_indices::fix_set_indices;
pub use transaction_order::fix_transaction_order;

/// Apply all fixers to the StatementData in a logical order
pub fn fix_statement_data(sd: &mut StatementData) {
    fix_implicit_dates(sd);
    fix_year_crossovers(sd);
    fix_transaction_order(sd);
    fix_opening_balance(sd);
    fix_amounts(sd);
    fix_implicit_balances(sd);
    fix_set_indices(sd);
    fix_closing_balance(sd);
}

#[cfg(test)]
mod robustness_tests {
    use super::fix_statement_data;
    use crate::checkers::check_statement_data;
    use crate::structs::{ProtoTransaction, StatementData};

    /// An imperfect (e.g. machine-drafted) configuration can produce protos
    /// without dates; the fixer/checker chain must record errors instead of
    /// panicking so callers see a parse failure.
    #[test]
    fn test_chain_survives_undated_first_transaction() {
        let mut sd = StatementData::new();
        sd.set_opening_balance(1000.0);
        sd.set_closing_balance(900.0);

        let mut undated = ProtoTransaction::new();
        undated.description = "no date".to_string();
        undated.set_amount(-100.0);
        sd.add_proto_transaction(undated);

        let mut dated = ProtoTransaction::new();
        dated.set_date(1735776000000);
        dated.description = "dated".to_string();
        dated.set_amount(0.0);
        sd.add_proto_transaction(dated);

        fix_statement_data(&mut sd);
        check_statement_data(&mut sd);

        assert!(!sd.errors.is_empty(), "expected errors, got none");
        assert!(
            sd.errors.iter().any(|e| e.contains("does not have a date")),
            "got: {:?}",
            sd.errors
        );
    }
}
