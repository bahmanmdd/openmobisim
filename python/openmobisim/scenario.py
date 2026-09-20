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

#: The flow-motor levels ``Scenario`` accepts (design §10.1): 0 is free flow
#: with no interaction between vehicles; 2, 3 and 4 are the link transmission
#: model as a point queue, a spatial queue and the full triangular diagram.
FLOW_LEVELS = (0, 2, 3, 4)


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

    def __init__(
        self,
        summary: _core.RunSummary,
        *,
        network: _core.Network | None = None,
        run_id: str = "run",
        flow_level: int = 0,
        flow_step_s: int = 300,
        window_s: int = 86_400,
    ) -> None:
        """Wrap a `RunSummary` from `run_pipeline`, with what it was run on."""
        self._summary = summary
        self._network = network
        self.run_id = run_id
        self.flow_level = flow_level
        self.flow_step_s = flow_step_s
        self.window_s = window_s

    @property
    def network(self) -> _core.Network | None:
        """The network this run loaded."""
        return self._network

    @property
    def fingerprint(self) -> str:
        """A hash of everything that decides this run's results (16 hex digits).

        The network's content, the demand, every setting, the route method and
        its options, the master seed and the code version. Two runs with the
        same fingerprint were given the same inputs; on one machine they give
        the same results. Figures print its first eight digits, every results
        file carries it, so a picture or a table traces back to its run.
        """
        return self._summary.fingerprint

    @property
    def master_seed(self) -> int:
        """The run's master seed (``Scenario`` argument ``master_seed``).

        The one number that starts every random stream. Under a sampled choice
        model (``"logit"``) it decides who takes which route; under the default
        ``"deterministic"`` nothing draws from it and it changes only the
        fingerprint.
        """
        return self._summary.master_seed

    @property
    def choice_model(self) -> str:
        """The name of the choice model that picked each trip's route.

        ``"deterministic"`` or ``"logit"``, or the ``name`` of a model written in
        Python.
        """
        return self._summary.choice_model

    @property
    def equilibration(self) -> str:
        """The name of the equilibration strategy: ``"none"`` or ``"msa"``."""
        return self._summary.equilibration

    @property
    def converged(self) -> bool:
        """Whether the run stopped before its most iterations because it had converged."""
        return self._summary.converged

    def convergence(self) -> dict[str, Any]:
        """What each iteration showed: how far the run is from an equilibrium.

        NumPy arrays, one entry per loading (a run without equilibration has one).
        ``nan`` marks a number that was not measured. The result of the run
        (``completion``, ``total_travel_time_s``, ``link_bins()``, ``route_choices()``)
        is that of the **last** iteration.

        * ``iteration`` — 0 is everyone's first choice, on free-flow costs.
        * ``reselected_share``, ``changed_share`` — the share of trips (by weight) that
          chose again on the way to this iteration, and the share whose route changed.
        * ``total_travel_time_s``, ``completed``, ``truncated`` — this loading's.
        * ``time_change`` — how much the link times moved since the last loading, as a
          share of it, weighted by traffic. The stability of the pattern.
        * ``gap_flow`` — the share of travellers whose route differs from where the
          model's probabilities, at the times this loading produced, would put them
          (half the total variation between observed and expected route flows, by
          origin-destination pair). ``gap_flow_floor`` — what that would be by chance
          alone: the same measure for a fresh sample drawn from the same probabilities.
          ``gap_flow_excess`` — the difference: the disequilibrium left after noise,
          near zero at a stochastic user equilibrium (and positive or negative by a
          little from noise, more with few travellers per pair).
        * ``gap_cost`` — the travel time the chosen routes cost over what the model
          expects a traveller to pay, as a share of the former: 0 at equilibrium.

        The gaps need a model that can give probabilities (``"logit"`` and
        ``"deterministic"`` do; a Python model may have a ``probabilities(batch)``
        method) and are ``nan`` without one.

        The same numbers are in ``kpis.parquet``, a row per metric per iteration.
        """
        return dict(self._summary.convergence)

    def route_sets(self) -> _core.RouteSets:
        """The route sets the trips were routed from.

        Every alternative the method found for every origin-destination pair
        the demand asked for. Draw one pair's alternatives with
        ``openmobisim.viz.map_route``.
        """
        return self._summary.route_sets

    def route_choices(self) -> _core.RouteChoices:
        """Which route each trip took, out of how many, and how likely.

        NumPy arrays with one entry per trip, in the order of the demand:
        ``route`` (an index into ``route_sets()``), ``rank`` (0 is the pair's best
        route), ``pair``, ``alternatives`` (how many it could choose from),
        ``probability`` (what the model gave the route taken; ``nan`` if it gave
        none) and ``weight`` (how many people the trip stands for). ``-1`` marks a
        trip with no route. ``np.bincount(rc.route[rc.route >= 0], rc.weight[rc.route >= 0],
        minlength=route_sets.route_count)`` is each route's number of travellers.
        """
        return self._summary.route_choices

    def link_bins(self) -> _core.LinkBins | None:
        """Per-link, per-time-bin results, or ``None`` if the run did not ask.

        Ask with ``Scenario.run(...)`` on a scenario built with ``link_bin_s``.
        One row per (bin, link) that saw traffic: how much traffic left the
        link in that bin, and how long it took. Every traversal that finished
        inside the window counts, including those of trips still under way
        when it ended. The same table is written to ``link_bins.parquet``: read
        it with ``Run.link_bins_table()``.
        """
        return self._summary.link_bins

    def link_bins_table(self) -> Table | None:
        """``link_bins.parquet``: the per-link results as a table, or ``None``.

        ``None`` unless the scenario was built with ``link_bin_s``. One row per
        (bin, link) that saw traffic, sorted by bin then link, with columns
        ``run_id, bin, start_s, link, link_index, crossings, pcu, pcu_seconds``:

        * ``link`` is the link's external id (an OSM way and direction, say),
          the id that survives a rebuilt network; ``link_index`` is its row in
          the network's arrays, meaningful only next to this run's network
          fingerprint, which the file's metadata carries.
        * ``crossings`` counts traversals that finished in the bin; ``pcu`` is
          the traffic that left the link (traveller weight and vehicle size
          included); ``pcu_seconds`` is the PCU-weighted time they took. Mean
          traversal time is ``pcu_seconds / pcu`` and the flow in PCU per hour
          is ``pcu * 3600 / bin_seconds``.
        """
        path = self._summary.link_bins_path
        return None if path is None else Table(path)

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
        flow_level: int = 0,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str = "penalty",
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any = "deterministic",
        choice_options: dict[str, float] | None = None,
        equilibration: str = "none",
        equilibration_options: dict[str, float] | None = None,
    ) -> None:
        """Store the parts; prefer `from_parts` to calling this directly."""
        if flow_level not in FLOW_LEVELS:
            raise ValueError(f"flow_level must be one of {FLOW_LEVELS}, got {flow_level}")
        if flow_step_s <= 0:
            raise ValueError(f"flow_step_s must be positive, got {flow_step_s}")
        if link_bin_s is not None and link_bin_s <= 0:
            raise ValueError(f"link_bin_s must be positive, got {link_bin_s}")
        if not 0 <= master_seed < 2**64:
            raise ValueError(
                f"master_seed must be an integer from 0 to 2**64 - 1, got {master_seed}"
            )
        self._network = network
        self._trips = trips
        self._trips_path = trips_path
        self._persons = persons
        self._persons_path = persons_path
        self._class_defaults = class_defaults
        self._default_weight = default_weight
        self._window_s = round(window_hours * 3600)
        self._flow_level = flow_level
        self._flow_step_s = flow_step_s
        self._link_bin_s = link_bin_s
        self._route_method = route_method
        self._route_options = route_options
        self._master_seed = master_seed
        self._choice_model = choice_model
        self._choice_options = choice_options
        self._equilibration = equilibration
        self._equilibration_options = equilibration_options

    @classmethod
    def from_parts(
        cls,
        network: _core.Network,
        demand: list[tuple] | str,
        persons: list[tuple] | str | None = None,
        class_defaults: dict[str, tuple[bool, bool, bool]] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
        flow_level: int = 0,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str = "penalty",
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any = "deterministic",
        choice_options: dict[str, float] | None = None,
        equilibration: str = "none",
        equilibration_options: dict[str, float] | None = None,
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
            flow_level: How vehicles load the network. ``0`` (the default) is
                free flow: no vehicle affects another, so there is no
                congestion to show. ``2``, ``3`` and ``4`` are the link
                transmission model (design §10.1): a point queue, a spatial
                queue, and the full triangular diagram with spillback.
            flow_step_s: The loading step in seconds, for levels 2-4. It is a
                bookkeeping boundary: results do not depend on it.
            link_bin_s: If given, also record per-link results in time bins of
                this many seconds, read with ``Run.link_bins()`` and drawn
                with ``openmobisim.viz.map_link``.
            route_method: How route sets are generated, by name (see
                ``openmobisim.route_methods()``; the default ``"penalty"``
                finds distinct alternatives). Until choice exists each trip
                takes the best route of its pair, so the method changes the
                sets you can inspect (``Run.route_sets()``) but not the run.
            route_options: The method's options, numbers by name; unknown names
                and out-of-range values are refused.
            master_seed: The one number that starts every random stream, so a
                run is reproduced by giving it the same one. Under a sampled
                choice model (``"logit"``) it decides who takes which route;
                under the default ``"deterministic"`` nothing draws from it and
                it changes only the run's fingerprint. A non-negative integer
                below 2**64.
            choice_model: How each trip picks a route from its pair's set: a
                name from ``openmobisim.choice_models()`` or an object with a
                ``choose(batch)`` method (see ``openmobisim.choice`` for how to
                write one). ``"deterministic"`` (the default) sends everyone
                down the best route; ``"logit"`` is the standard path-size
                logit, sampled per traveller.
            choice_options: A built-in model's options, numbers by name: for
                ``"logit"`` a coefficient per attribute, ``beta_time_min`` (default
                -0.2), ``beta_ln_path_size`` (1), ``beta_length_km``,
                ``beta_detour``, ``beta_overlap``, ``beta_n_links`` (0). Unknown
                names and non-numbers are refused. The defaults are an
                assumption, not a calibration.
            equilibration: How choice and loading are repeated (see
                ``openmobisim.equilibration_strategies()``). ``"none"`` (the
                default) chooses every trip's route once, on free-flow costs, and
                loads the network once. ``"msa"`` is the method of successive
                averages in traveller form: load the network, read the link times it
                produced, let a share ``1/(i + 1)`` of travellers choose again on
                those times at iteration ``i``, load again. It only changes anything
                when vehicles interact (``flow_level`` 2 to 4) and a choice model
                gives travellers something to choose between (``"logit"``).
                ``Run.convergence()`` says how the iterations went.
            equilibration_options: The strategy's options, numbers by name: for
                ``"msa"``, ``iterations`` (10; the most loadings), ``gap_tolerance``
                (0: never stop early; otherwise stop once the flow gap in excess of
                chance, averaged over the last three iterations, is at most this share
                of travellers) and ``cost_bin_s`` (300: the length of the time bins the
                link times are read in).

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
            flow_level=flow_level,
            flow_step_s=flow_step_s,
            link_bin_s=link_bin_s,
            route_method=route_method,
            route_options=route_options,
            master_seed=master_seed,
            choice_model=choice_model,
            choice_options=choice_options,
            equilibration=equilibration,
            equilibration_options=equilibration_options,
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
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            link_bin_s=self._link_bin_s,
            route_method=self._route_method,
            route_options=self._route_options,
            master_seed=self._master_seed,
            choice_model=self._choice_model,
            choice_options=self._choice_options,
            equilibration=self._equilibration,
            equilibration_options=self._equilibration_options,
        )
        return Run(
            summary,
            network=self._network,
            run_id=run_id,
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            window_s=self._window_s,
        )
