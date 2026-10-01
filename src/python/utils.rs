use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyList};

/// Convert a Rust StatementData to a Python StatementData object
pub fn rust_statement_data_to_py_statement_data(
    rust_statement_data: &crate::structs::StatementData,
) -> PyResult<Py<PyAny>> {
    convert_statement_data(rust_statement_data, false)
}

/// Convert a Rust StatementData to a Python StatementData object, allowing
/// checker errors through (lenient parse mode). The errors are attached to
/// the Python object so review tooling can surface the offending rows.
pub fn rust_statement_data_to_py_statement_data_lenient(
    rust_statement_data: &crate::structs::StatementData,
) -> PyResult<Py<PyAny>> {
    convert_statement_data(rust_statement_data, true)
}

/// Shared conversion; `lenient` permits (and carries) checker errors.
fn convert_statement_data(
    rust_statement_data: &crate::structs::StatementData,
    lenient: bool,
) -> PyResult<Py<PyAny>> {
    Python::attach(|py| {
        // Import the Python StatementData and Transaction classes
        let statement_data_module = py.import("transtractor.structs.statement_data")?;
        let statement_data_class = statement_data_module.getattr("StatementData")?;

        let transaction_module = py.import("transtractor.structs.transaction")?;
        let transaction_class = transaction_module.getattr("Transaction")?;
        let benchmark_module = py.import("transtractor.structs.benchmark")?;
        let benchmark_class = benchmark_module.getattr("Benchmark")?;

        // Get key (required field)
        let key = rust_statement_data.key.as_ref().ok_or_else(|| {
            PyRuntimeError::new_err("StatementData is missing required field: key")
        })?;

        // Get account_number (required field)
        let account_number = rust_statement_data.account_number.as_ref().ok_or_else(|| {
            PyRuntimeError::new_err("StatementData is missing required field: account_number")
        })?;

        // Get start_date (required field)
        let start_date = rust_statement_data.start_date.ok_or_else(|| {
            PyRuntimeError::new_err("StatementData is missing required field: start_date")
        })?;

        // Get opening_balance (required field)
        let opening_balance = rust_statement_data.opening_balance.ok_or_else(|| {
            PyRuntimeError::new_err("StatementData is missing required field: opening_balance")
        })?;

        // Get closing_balance (required field)
        let closing_balance = rust_statement_data.closing_balance.ok_or_else(|| {
            PyRuntimeError::new_err("StatementData is missing required field: closing_balance")
        })?;

        if !rust_statement_data.errors.is_empty() && !lenient {
            return Err(PyRuntimeError::new_err(
                "StatementData must be error-free before export to Python",
            ));
        }

        // Convert proto_transactions to Transaction objects
        let py_transactions = PyList::empty(py);
        for proto_tx in &rust_statement_data.proto_transactions {
            // Check if the proto transaction is complete
            if !proto_tx.is_ready() {
                return Err(PyRuntimeError::new_err(format!(
                    "Incomplete transaction found: date={:?}, date_index='{}', description='{}', amount={:?}, balance={:?}",
                    proto_tx.date,
                    proto_tx.index,
                    proto_tx.description,
                    proto_tx.amount,
                    proto_tx.balance
                )));
            }

            // Create Python Transaction object
            let py_transaction = transaction_class.call1((
                proto_tx.date.unwrap(),
                proto_tx.index,
                proto_tx.description.clone(),
                proto_tx.amount.unwrap(),
                proto_tx.balance.unwrap(),
                account_number,
            ))?;

            py_transactions.append(py_transaction)?;
        }

        // Create Python StatementData object.
        // The parser wrapper can overwrite the blank filename later if needed.
        let kwargs = PyDict::new(py);
        kwargs.set_item("key", key)?;
        kwargs.set_item("filename", "")?;
        kwargs.set_item("account_number", account_number)?;
        kwargs.set_item("start_date", start_date)?;
        kwargs.set_item("opening_balance", opening_balance)?;
        kwargs.set_item("closing_balance", closing_balance)?;
        kwargs.set_item("transactions", py_transactions)?;
        kwargs.set_item("errors", &rust_statement_data.errors)?;
        let benchmark = rust_statement_data.benchmark.as_micros();
        let benchmark_kwargs = PyDict::new(py);
        benchmark_kwargs.set_item("total", benchmark.total)?;
        benchmark_kwargs.set_item("pdf_extractor", benchmark.pdf_extractor)?;
        benchmark_kwargs.set_item("tokeniser", benchmark.tokeniser)?;
        benchmark_kwargs.set_item("typer", benchmark.typer)?;
        benchmark_kwargs.set_item("parsers", benchmark.parsers)?;
        benchmark_kwargs.set_item(
            "parsers_account_number_parser_prime",
            benchmark.parsers_account_number_parser_prime,
        )?;
        benchmark_kwargs.set_item(
            "parsers_account_number_parser_parse",
            benchmark.parsers_account_number_parser_parse,
        )?;
        benchmark_kwargs.set_item(
            "parsers_start_date_parser_prime",
            benchmark.parsers_start_date_parser_prime,
        )?;
        benchmark_kwargs.set_item(
            "parsers_start_date_parser_parse",
            benchmark.parsers_start_date_parser_parse,
        )?;
        benchmark_kwargs.set_item(
            "parsers_opening_balance_parser_prime",
            benchmark.parsers_opening_balance_parser_prime,
        )?;
        benchmark_kwargs.set_item(
            "parsers_opening_balance_parser_parse",
            benchmark.parsers_opening_balance_parser_parse,
        )?;
        benchmark_kwargs.set_item(
            "parsers_closing_balance_parser_prime",
            benchmark.parsers_closing_balance_parser_prime,
        )?;
        benchmark_kwargs.set_item(
            "parsers_closing_balance_parser_parse",
            benchmark.parsers_closing_balance_parser_parse,
        )?;
        benchmark_kwargs.set_item(
            "parsers_transaction_parser_start_prime",
            benchmark.parsers_transaction_parser_start_prime,
        )?;
        benchmark_kwargs.set_item(
            "parsers_transaction_parser_parse",
            benchmark.parsers_transaction_parser_parse,
        )?;
        benchmark_kwargs.set_item(
            "parsers_transaction_parser_stop_prime",
            benchmark.parsers_transaction_parser_stop_prime,
        )?;
        benchmark_kwargs.set_item("fixers", benchmark.fixers)?;
        benchmark_kwargs.set_item("checkers", benchmark.checkers)?;
        kwargs.set_item(
            "benchmark",
            benchmark_class.call((), Some(&benchmark_kwargs))?,
        )?;
        let py_statement_data = statement_data_class.call((), Some(&kwargs))?;

        Ok(py_statement_data.into())
    })
}
