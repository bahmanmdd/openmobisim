"""Results as tables: ``Table`` (a file a run wrote) and ``Columns`` (a result held as columns).

Split from ``scenario.py``; ``openmobisim.scenario`` re-exports ``Table``.
"""

from __future__ import annotations

from typing import Any

__all__ = ["Columns", "Table"]


def _need(module: str, what: str) -> Any:
    import importlib

    try:
        return importlib.import_module(module)
    except ImportError as e:
        raise ImportError(f"{what} needs {module}: pip install 'openmobisim[all]'") from e


class Columns(dict):
    """A table held as columns: a ``dict`` from each column's name to its values, in order.

    Every result given "as columns" is one (``Run.trip_modes()``, ``Run.itinerary_choices()``,
    ``Run.transit_calls()``, ``parameters()`` …): read a column by name as from any dict
    (``columns["mode"]``), or hand the whole table to a data-frame library with ``to_pandas()``,
    ``to_polars()`` or ``to_arrow()``. Each column is a list or a NumPy array, all of one length.
    """

    @property
    def row_count(self) -> int:
        """How many rows the columns hold (0 without columns)."""
        return len(next(iter(self.values()))) if self else 0

    def to_pandas(self) -> Any:
        """The columns as a ``pandas.DataFrame``.

        Raises:
            ImportError: If pandas is not installed — install ``openmobisim[all]``.
        """
        return _need("pandas", "to_pandas()").DataFrame(dict(self))

    def to_polars(self) -> Any:
        """The columns as a ``polars.DataFrame``.

        Raises:
            ImportError: If polars is not installed — install ``openmobisim[all]``.
        """
        return _need("polars", "to_polars()").DataFrame(dict(self))

    def to_arrow(self) -> Any:
        """The columns as a ``pyarrow.Table``.

        Raises:
            ImportError: If pyarrow is not installed — install ``openmobisim[all]``.
        """
        return _need("pyarrow", "to_arrow()").table(dict(self))

    def _repr_html_(self) -> str | None:
        """In a notebook, the table itself (by pandas, if installed)."""
        try:
            import pandas as pd
        except ImportError:
            return None
        return pd.DataFrame(dict(self))._repr_html_()

    def __repr__(self) -> str:
        """The size and the column names (the values: ``dict(columns)`` or a data frame)."""
        return f"Columns({self.row_count} rows; {', '.join(self)})"


class Table:
    """A results table backed by a Parquet file.

    ``.to_polars()``, ``.to_pandas()`` and ``.to_arrow()`` read the file on demand. A later version
    may hand the table over without reading a file; the call shape stays the
    same.
    """

    def __init__(self, path: str) -> None:
        """Wrap the Parquet file (or, for a run's minimal outputs, the CSV file) at `path`."""
        self.path = path
        self._csv = str(path).endswith(".csv")

    def to_polars(self) -> Any:
        """Read this table with polars.

        Raises:
            ImportError: If polars is not installed — install
                ``openmobisim[all]``.
        """
        try:
            import polars as pl
        except ImportError as e:
            raise ImportError("reading a Table needs polars: pip install 'openmobisim[all]'") from e
        return pl.read_csv(self.path) if self._csv else pl.read_parquet(self.path)

    def to_pandas(self) -> Any:
        """Read this table with pandas.

        Raises:
            ImportError: If pandas is not installed — install
                ``openmobisim[all]``.
        """
        try:
            import pandas as pd
        except ImportError as e:
            raise ImportError("reading a Table needs pandas: pip install 'openmobisim[all]'") from e
        if self._csv:  # exact: the default parser may be one unit off in the last place
            return pd.read_csv(self.path, float_precision="round_trip")
        return pd.read_parquet(self.path)

    def to_arrow(self) -> Any:
        """Read this table with pyarrow, as a ``pyarrow.Table``.

        Raises:
            ImportError: If pyarrow is not installed — install
                ``openmobisim[all]``.
        """
        try:
            import pyarrow.csv as pc
            import pyarrow.parquet as pq
        except ImportError as e:
            raise ImportError(
                "reading a Table needs pyarrow: pip install 'openmobisim[all]'"
            ) from e
        return pc.read_csv(self.path) if self._csv else pq.read_table(self.path)

    def __repr__(self) -> str:
        """The path this table reads from."""
        return f"Table({self.path!r})"
