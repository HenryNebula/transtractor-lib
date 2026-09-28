# The Transtractor

![PyPI version](https://img.shields.io/pypi/v/transtractor)
![Development Status](https://img.shields.io/pypi/status/transtractor)
![Tests](https://github.com/weberdak/transtractor-lib/actions/workflows/ci.yml/badge.svg)
![Read the Docs](https://readthedocs.org/projects/transtractor-lib/badge/?version=latest)
![codecov](https://codecov.io/gh/transtractor/transtractor-lib/branch/main/graph/badge.svg)
![License](https://img.shields.io/github/license/transtractor/transtractor-lib)


## The Universal Bank Statement PDF Extractor
The Transtractor is a rules-based PDF bank statement parser for extracting structured transaction data from statements issued by different banks and financial institutions. It combines a fast Rust parsing engine with an installable Python API, WebAssembly bindings, and a [web-based GUI](https://www.transtractor.net/).

### Key features

* Parse bank statement PDFs with deterministic, configuration-driven rules
* Normalise extracted data into consistent dates, descriptions, amounts, balances, and account details
* Infer missing transaction dates and running balances
* Normalise transaction signs so debits are negative and credits are positive
* Validate transaction totals against opening and closing balances
* Run locally without sending financial documents to an external AI service
* Use the same core parser in Python applications or browser-based WebAssembly integrations

Each supported statement format is implemented as a lightweight configuration module. This design keeps the parser portable and extensible while making its output predictable and suitable for downstream processing, reporting, and financial analysis.

## Installation
### Install from PyPI
Transtractor is available on PyPI and can be installed with pip:

```shell
pip install transtractor
```

### Compile from Source
1. **Install Rust**: Download and install Rust from [rustup.rs](https://rustup.rs/)

2. **Install uv**: Follow instructions from [Astral](https://docs.astral.sh/uv/getting-started/installation/)

3. **Sync Python environment and compile**: Clone the repository and build
   ```shell
   git clone https://github.com/transtractor/transtractor.git
   cd transtractor-lib
   uv sync --locked --group dev
   ```

4. **Test the package**: Run Rust and Python unit tests
   ```shell
   cargo test
   uv run pytest
   ```

### Basic Usage
Detailed [documentation](https://transtractor-lib.readthedocs.io/en/latest/) maintained on Read the Docs, but you can get started using the following steps.

1. **Import and initialise the parser**
   ```python
   from transtractor import Parser

   parser = Parser()
   ```

2. **Convert PDF to CSV**: All CSV files are written in a standard format
   ```python
   parser.parse('statement.pdf').to_csv('statement.csv')
   ```

3. **Convert PDF to DataFrame**: Load into a DataFrame for analysis
   ```python
   import pandas as pd

   data = parser.parse('statement.pdf').to_pandas_dict()
   df = pd.DataFrame(data)
   ```

The `parse` method returns a `StatementData` object containing the account number, statement date, opening and closing balances, and transaction table. Transaction dates, descriptions, amounts, and running balances are extracted or derived when they are not explicitly recorded in the statement. Transaction amounts are validated against the opening and closing balances; if validation fails, the method raises a `ParserError`.

## Supported Statements
See the documentation for a current list of [supported statements](https://transtractor-lib.readthedocs.io/en/latest/supported_statements.html). You may also create your own parsing configuration files by following these [instructions](https://transtractor-lib.readthedocs.io/en/latest/configuration.html)
and loading it by:

```python
from transtractor import Parser

parser = Parser()
parser.load('my_config.json')
parser.parse('statement.pdf').to_csv('statement.csv')
```

## Performance
The Transtractor typically parses between 10 and 20 statements per second on average. Parsing time scales linearly with the number of transactions in each statement. Actual performance varies by statement type and the density of extractable information. Statements with a higher proportion of non-extractable content create more noise for the parser to filter through.

![Parsing performance vs. number of transactions](md/performance.svg)

## WASM Implementation
WASM bindings are also provided for in-browser parsing of PDF bank statements. See [this guide](md/wasm.md) for an introductory guide on how to compile and use them. 

You may also want to checkout [www.transtractor.net](https://www.transtractor.net) to see these bindings in action.

## Developers
The following pages provide further information about how this package is built and developed:

* [Architecture Guide](md/architecture.md): Overview of key application components and design principles.
* [Developer Guide](md/develop.md): Reference page for core development and maintenance.
* [Contributor Guide](CONTRIBUTING.md): Extending the package to parse additional bank statements.
* [WASM Guide](md/wasm.md): Build and usage notes for the TypeScript/WASM package.

Please get involved or email gravytoast@pm.me if you have any questions.
