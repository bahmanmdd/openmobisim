"""``Columns``: one shape for every result given as columns (C2, D-5 a)."""

from __future__ import annotations

from typing import Any

__all__ = ["Columns"]


def _need(module: str, what: str) -> Any:
    import importlib

    try:
        return importlib.import_module(module)
    except ImportError as e:
        raise ImportError(f"{what} needs {module}: pip install 'openmobisim[notebooks]'") from e


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
            ImportError: If pandas is not installed — install ``openmobisim[notebooks]``.
        """
        return _need("pandas", "to_pandas()").DataFrame(dict(self))

    def to_polars(self) -> Any:
        """The columns as a ``polars.DataFrame``.

        Raises:
            ImportError: If polars is not installed — install ``openmobisim[notebooks]``.
        """
        return _need("polars", "to_polars()").DataFrame(dict(self))

    def to_arrow(self) -> Any:
        """The columns as a ``pyarrow.Table``.

        Raises:
            ImportError: If pyarrow is not installed — install ``openmobisim[notebooks]``.
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
