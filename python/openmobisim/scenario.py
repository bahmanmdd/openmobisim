"""``Scenario`` and what ``.run()`` returns.

Phase 1's cut of the interface doc's API: a network, demand (an in-memory
table or a ``trips.parquet`` path — the same schema either way, S97), and
the handful of parameters a run needs. No hubs, no equilibration, no
disruptions — those arrive with the layers and mechanisms that give them
meaning (Foundations §10).
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import Any

from openmobisim import _core

__all__ = ["Run", "Scenario", "Table"]


class Table:
    """A results table backed by a Parquet file.

    Phase 1's form of the interface doc's zero-copy ``openmobisim.Table``:
    ``.to_polars()``/``.to_pandas()`` read the file on demand rather than
    streaming Arrow across the boundary without a copy — the call shape the
    real zero-copy ``Table`` will keep, so nothing at a call site has to
    change when that lands; only the mechanism underneath does.
    """

    def __init__(self, path: str) -> None:
        """Wrap the Parquet file at `path`."""
        self.path = path

    def to_polars(self) -> Any:
        """Read this table with polars.

        Raises:
            ImportError: If polars is not installed — install
                ``openmobisim[notebooks]``.
        """
        try:
            import polars as pl
        except ImportError as e:
            raise ImportError(
                "reading a Table needs polars: pip install 'openmobisim[notebooks]'"
            ) from e
        return pl.read_parquet(self.path)

    def to_pandas(self) -> Any:
        """Read this table with pandas.

        Raises:
            ImportError: If pandas is not installed — install
                ``openmobisim[notebooks]``.
        """
        try:
            import pandas as pd
        except ImportError as e:
            raise ImportError(
                "reading a Table needs pandas: pip install 'openmobisim[notebooks]'"
            ) from e
        return pd.read_parquet(self.path)

    def __repr__(self) -> str:
        """The path this table reads from."""
        return f"Table({self.path!r})"


class Run:
    """What one run produced."""

    def __init__(self, summary: _core.RunSummary) -> None:
        """Wrap a `RunSummary` from `run_pipeline`."""
        self._summary = summary

    def kpis(self) -> Table:
        """``kpis.parquet``: long format, one row per metric (Foundations §6)."""
        return Table(self._summary.kpis_path)

    def diagnostics(self) -> Table:
        """``diagnostics.parquet``: aggregated counters, one row per issue."""
        return Table(self._summary.diagnostics_path)

    def events(self) -> Table:
        """``events.parquet``: sampled per-trip events."""
        return Table(self._summary.events_path)

    def manifest(self) -> dict[str, Any]:
        """``manifest.json``: the file that makes this run reproducible."""
        with Path(self._summary.manifest_path).open(encoding="utf-8") as f:
            data: dict[str, Any] = json.load(f)
        return data

    @property
    def completion(self) -> dict[str, int]:
        """S57's completion statistics, without reading any file.

        Keys: ``total_trips``, ``completed``, ``truncated``,
        ``no_vehicle_available``, ``no_feasible_path``.
        """
        s = self._summary
        return {
            "total_trips": s.total_trips,
            "completed": s.completed,
            "truncated": s.truncated,
            "no_vehicle_available": s.no_vehicle_available,
            "no_feasible_path": s.no_feasible_path,
        }

    @property
    def total_travel_time_s(self) -> float:
        """Phase 1's one KPI: total travel time, traveller-weight-scaled.

        Confirmed by the user (S136): once comprehensive KPIs exist, an
        unweighted form joins this as a second ``kpis.parquet`` row, not a
        replacement for it.
        """
        return self._summary.total_travel_time_s

    def __repr__(self) -> str:
        """S57's completion counts, as a one-line summary."""
        c = self.completion
        return (
            f"Run(completed={c['completed']}, truncated={c['truncated']}, "
            f"no_vehicle_available={c['no_vehicle_available']}, "
            f"no_feasible_path={c['no_feasible_path']})"
        )


class Scenario:
    """A network, demand, and the parameters a Phase 1 run needs.

    Build with ``from_parts`` rather than the constructor directly.
    """

    def __init__(
        self,
        network: _core.Network,
        *,
        trips: list[tuple] | None = None,
        trips_path: str | None = None,
        persons: list[tuple] | None = None,
        persons_path: str | None = None,
        class_defaults: dict[str, tuple[bool, bool, bool]] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
    ) -> None:
        """Store the parts; prefer `from_parts` to calling this directly."""
        self._network = network
        self._trips = trips
        self._trips_path = trips_path
        self._persons = persons
        self._persons_path = persons_path
        self._class_defaults = class_defaults
        self._default_weight = default_weight
        self._window_s = round(window_hours * 3600)

    @classmethod
    def from_parts(
        cls,
        network: _core.Network,
        demand: list[tuple] | str,
        persons: list[tuple] | str | None = None,
        class_defaults: dict[str, tuple[bool, bool, bool]] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
    ) -> Scenario:
        """Build a scenario from a network and demand.

        Args:
            network: Built by, for example, ``examples.manhattan_grid``.
            demand: Either an in-memory trips table (rows in the schema
                ``examples.fixed_car_trips`` returns, or hand-built the same
                way) or a path to a ``trips.parquet`` file (S97) — the same
                schema either way, never two different shapes.
            persons: The ``persons.parquet`` equivalent — an in-memory table,
                a file path, or ``None`` if every traveller takes their
                class's default ownership.
            class_defaults: ``{class_name: (owns_car, owns_bike,
                has_transit_pass)}`` — S127's per-class default. A class not
                listed here owns nothing unless a `persons` row overrides it.
            default_weight: The scenario's ``traveller_weight`` (S89): 1 for
                `default`, 10 for `fast`, for a trip whose row gives none.
            window_hours: Trips still in progress after this many hours are
                truncated (S57).

        Returns:
            A ``Scenario``, ready to ``.run()``.
        """
        trips = demand if not isinstance(demand, str) else None
        trips_path = demand if isinstance(demand, str) else None
        persons_rows = persons if not isinstance(persons, str) else None
        persons_path = persons if isinstance(persons, str) else None
        return cls(
            network,
            trips=trips,
            trips_path=trips_path,
            persons=persons_rows,
            persons_path=persons_path,
            class_defaults=class_defaults,
            default_weight=default_weight,
            window_hours=window_hours,
        )

    def run(self, run_id: str = "run", output_dir: str | None = None) -> Run:
        """Run the scenario and write its output artifacts.

        Args:
            run_id: Recorded in every output row; also names the default
                output directory.
            output_dir: Where ``kpis.parquet`` etc. are written. Defaults to
                a directory under the system temp directory — Phase 1 has no
                artifact cache (S65 is Phase 2 item 1) to place it in yet.

        Returns:
            A ``Run`` with the four artifacts and the completion statistics.
        """
        if output_dir is None:
            output_dir = str(Path(tempfile.gettempdir()) / "openmobisim-runs" / run_id)
        summary = _core.run_pipeline(
            self._network,
            run_id,
            output_dir,
            trips=self._trips,
            trips_path=self._trips_path,
            persons=self._persons,
            persons_path=self._persons_path,
            class_defaults=self._class_defaults,
            default_weight=self._default_weight,
            window_s=self._window_s,
        )
        return Run(summary)
