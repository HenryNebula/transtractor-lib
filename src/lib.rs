pub mod checkers;
pub mod configs;
pub mod fixers;
pub mod formats;
#[cfg(feature = "llm")]
pub mod llm;
pub mod parser;
pub mod parsers;
#[cfg(feature = "python-bindings")]
pub mod python;
pub mod structs;
#[cfg(feature = "wasm-bindings")]
pub mod wasm;

#[cfg(feature = "python-bindings")]
use crate::python::exceptions::{ConfigLoadError, ParseError, SpecError};
#[cfg(feature = "python-bindings")]
use crate::python::lib_parser::LibParser;
#[cfg(feature = "python-bindings")]
use pyo3::prelude::*;

/// Python module definition
#[cfg(feature = "python-bindings")]
#[pymodule]
fn transtractor(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<LibParser>()?;
    m.add("ConfigLoadError", m.py().get_type::<ConfigLoadError>())?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add("SpecError", m.py().get_type::<SpecError>())?;
    Ok(())
}
