"""Base data structure for recording extracted statement data for
subsequent processing in Python."""

import csv
from dataclasses import dataclass, field

from .benchmark import Benchmark
from .transaction import Transaction


@dataclass(repr=False, slots=True)
class StatementData:
    """Class representing bank statement data.

    ``errors`` is only populated by the lenient LLM parse mode
    (:meth:`transtractor.Parser.parse` with ``lenient=True``): every
    validation failure found by the checkers, naming the offending
    transaction. Empty means the numbers reconciled with the statement's
    opening and closing balances.
    """

    key: str = ""
    filename: str = ""
    account_number: str = ""
    start_date: int = 0
    opening_balance: float = 0.0
    closing_balance: float = 0.0
    transactions: list[Transaction] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    benchmark: Benchmark = field(default_factory=Benchmark)

    def __repr__(self) -> str:
        return (
            f"StatementData(key={self.key!r}, "
            f"filename={self.filename!r}, "
            f"account_number={self.account_number!r}, "
            f"start_date={self.start_date!r}, "
            f"opening_balance={self.opening_balance!r}, "
            f"closing_balance={self.closing_balance!r}, "
            f"transactions=[{len(self.transactions)} transactions])"
        )

    def to_csv(self, file_path: str) -> None:
        """Export the statement data to a CSV file.

        :param file_path: Path to the output CSV file
        :type file_path: str
        """
        fields = ("date", "description", "amount", "balance")

        with open(file_path, mode="w", newline="", encoding="utf-8") as csvfile:
            writer = csv.writer(csvfile)
            # Write header
            writer.writerow(fields)
            # Write transaction data
            for transaction in self.transactions:
                writer.writerow(
                    [
                        transaction.date,
                        transaction.description,
                        transaction.amount,
                        transaction.balance,
                    ]
                )

    def to_pandas_dict(self) -> dict[str, list]:
        """Convert the statement data to a dictionary suitable for pandas DataFrame.

        :return: Dictionary containing date, description, amount, and balance lists
        :rtype: dict[str, list]
        """
        fields = ("date", "description", "amount", "balance")
        data_dict = {field_name: [] for field_name in fields}

        for transaction in self.transactions:
            data_dict["date"].append(transaction.date)
            data_dict["description"].append(transaction.description)
            data_dict["amount"].append(transaction.amount)
            data_dict["balance"].append(transaction.balance)

        return data_dict
