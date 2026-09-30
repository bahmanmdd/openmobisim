"""``Scenario`` and what ``.run()`` returns.

A network, demand (an in-memory table or a ``trips.parquet`` path — the same
schema either way), a loading with queues and spillback (``flow_level=4``),
route choice among each trip's alternatives (a path-size ``"logit"``) and
iteration towards an equilibrium (``"msa"``), during which the route sets may
grow (``route_update="best_response"``) — all by default. Trips drive, cycle,
walk, take transit (``transit=``) or drive or cycle to a parking and go on by
transit (``parkings=``). No disruptions yet.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import Any

from openmobisim import _core

__all__ = ["Run", "Scenario", "Table"]

#: The flow levels ``Scenario`` accepts: 0 is free flow
#: with no interaction between vehicles; 2, 3 and 4 are the link transmission
#: model as a point queue, a spatial queue and the full triangular diagram.
FLOW_LEVELS = (0, 2, 3, 4)


class Table:
    """A results table backed by a Parquet file.

    ``.to_polars()``/``.to_pandas()`` read the file on demand. A later version
    may hand the table over without reading a file; the call shape stays the
    same.
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
        transit: _core.Transit | None = None,
        parkings: _core.Parkings | None = None,
    ) -> None:
        """Wrap a `RunSummary` from `run_pipeline`, with what it was run on."""
        self._summary = summary
        self._network = network
        self._transit = transit
        self._parkings = parkings
        self.run_id = run_id
        self.flow_level = flow_level
        self.flow_step_s = flow_step_s
        self.window_s = window_s

    @property
    def network(self) -> _core.Network | None:
        """The network this run loaded."""
        return self._network

    @property
    def transit(self) -> _core.Transit | None:
        """The timetable this run's transit trips rode, if it had one."""
        return self._transit

    @property
    def parkings(self) -> _core.Parkings | None:
        """The parkings this run's park-and-ride and bike-and-ride trips used, if any."""
        return self._parkings

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
        model (``"logit"``, the default) it decides who takes which route; under
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
    def route_update(self) -> str:
        """The name of the route update that grew the route sets between iterations.

        ``"none"`` (the sets stayed as the route method made them) or ``"best_response"``.
        """
        return self._summary.route_update

    @property
    def converged(self) -> bool:
        """Whether the run stopped before its most iterations because it had converged."""
        return self._summary.converged

    @property
    def convergence_gap(self) -> float:
        """How far the run ended from equilibrium.

        **The worse of** the last iteration's disequilibrium against the choice set
        (``convergence()["gap_excess"][-1]``) and against the whole network
        (``convergence()["gap_network_excess"][-1]``, a sample), so a route the choice set
        was missing cannot hide behind a small in-set number. The disequilibrium is the gap
        **less what the choice model itself expects** (a logit sends some travellers down
        a slower route by design, so its gap is never 0); where the model gives no
        probabilities the plain gaps are used instead.

        ``nan`` if no gap was measured (a run without equilibration). See
        ``convergence_verdict``.
        """
        c = self._summary.convergence
        values = []
        for excess, plain in (("gap_excess", "gap"), ("gap_network_excess", "gap_network")):
            value = float(c[excess][-1])
            values.append(value if value == value else float(c[plain][-1]))
        finite = [v for v in values if v == v]  # not nan
        return max(finite) if finite else float("nan")

    @property
    def convergence_verdict(self) -> str | None:
        """``"good"`` (disequilibrium below 5%), ``"acceptable"`` (below 15%) or ``"poor"``.

        ``None`` if no gap was measured (no equilibration). Judged on
        ``convergence_gap``, the disequilibrium: what the choice model does not explain.
        """
        gap = self.convergence_gap
        if gap != gap:  # nan
            return None
        return "good" if gap < 0.05 else "acceptable" if gap < 0.15 else "poor"

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
        * ``gap`` — the usual measure of how far an assignment is from equilibrium: how
          much more travel time the routes chosen cost than the cheapest route of the same
          choice set at the times this loading produced, ``Σ w (t_chosen − t_least) /
          Σ w t_least``, **over the trips that finished** (the times of trips still under way
          when the window ended are lower bounds, not measurements). **A stochastic choice model
          does not reach 0 by design**: a logit at −0.2 per minute has a gap of about 7% at
          equilibrium.
        * ``gap_expected`` — **the gap the choice model itself expects** at these times: the
          same sum with each traveller's expected travel time under the model's
          probabilities. 0 for the all-or-nothing ``"deterministic"`` model; ``nan`` for a
          model that gives no probabilities.
        * ``gap_excess`` — **the disequilibrium**, ``gap − gap_expected``: what the choice
          model does not explain (it also holds the sampling noise of a finite population).
          **Below 0.05 is good, below 0.15 acceptable**; it is what ``gap_tolerance`` stops on
          and what ``convergence_verdict`` judges. Equal to ``gap`` for ``"deterministic"``.
        * ``incomplete_share`` — the share of travellers (by weight) whose trip had not finished
          when the window ended, who are left out of the gaps.
        * ``gap_network`` — the same measure against the **whole network**: the least
          time over *any* route at the current times (a time-dependent search), for a
          sample of the trips that finished (the ``gap_sample`` option) at the **last**
          iteration only, ``nan`` elsewhere. It shows whether the choice set was missing a
          route that traffic has made worthwhile.
        * ``gap_network_excess`` — its disequilibrium: ``gap_network`` less what the model
          explains *inside the set*, so a route the set was missing counts in full.
        * ``gap_flow``, ``gap_flow_floor``, ``gap_flow_excess`` — a check of the
          stochastic choice model with itself, needing its probabilities: the share of
          travellers whose route differs from where the probabilities at the current
          times would put them (half the total variation between observed and expected
          route flows, by origin-destination pair), what that would be by chance alone
          (the same measure for a fresh sample from the same probabilities), and the
          difference. Not a gap to the shortest path.

        ``gap_flow*`` need a model that can give probabilities (``"logit"`` and
        ``"deterministic"`` do; a Python model may have a ``probabilities(batch)``
        method) and are ``nan`` without one; ``gap`` does not.

        * ``routes_added``, ``route_searches`` — under a route update
          (``route_update="best_response"``), how many routes it added to the route sets
          **after** this loading, for the next iteration to choose among, and how many
          searches that took; 0 without one, and at the last iteration, which no choice
          follows. **``gap`` is measured against the sets as grown**: the routes the
          travellers may choose from next.

        * ``gap_transit``, ``gap_car_transit``, ``gap_bike_transit`` — the same relative gap
          for the itineraries of transit, park-and-ride and bike-and-ride trips: each
          itinerary kept against the least door-to-door time of its choice set at the costs
          this loading produced; ``nan`` for a mode without such trips. ``gap`` above is the
          car routes'.
        * ``hub_mismatch_s`` — the hub expectation mismatch: per traveller who parked, the
          mean difference between the parking time expected when they chose and the one
          they paid, in seconds; ``nan`` without parkings.

        The numbers up to ``gap_flow_excess`` are also in ``kpis.parquet``, a row per metric
        per iteration, and so are the itinerary gaps (under their modes, as ``gap``) and
        ``hub_mismatch_s``.
        """
        return dict(self._summary.convergence)

    def route_sets(self) -> _core.RouteSets:
        """The route sets the trips were routed from.

        Every alternative the method found for every origin-destination pair
        the demand asked for, and, if the run had a route update, the routes it added
        while iterating (``stamps()`` says in which iteration each was added; 0 for the
        method's own). Draw one pair's alternatives with ``openmobisim.viz.map_route``.
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

    def link_bins(self, layer: str = "road") -> _core.LinkBins | None:
        """Per-link, per-time-bin results, or ``None`` if the run did not ask.

        Ask with ``Scenario.run(...)`` on a scenario built with ``link_bin_s``.
        One row per (bin, link) that saw traffic: how much traffic left the
        link in that bin, and how long it took. Every traversal that finished
        inside the window counts, including those of trips still under way
        when it ended. The same table is written to ``link_bins.parquet``: read
        it with ``Run.link_bins_table()``.

        ``layer`` is ``"road"`` (the default), ``"bike"`` or ``"walk"``: the bike and
        walk layers' results are indexed by their own links
        (``run.network.layer("bike")``), and their ``pcu`` counts travellers.
        They are ``None`` when no trip of that mode ran.
        """
        return {
            "road": self._summary.link_bins,
            "bike": self._summary.bike_link_bins,
            "walk": self._summary.walk_link_bins,
        }[_check_layer(layer)]

    def link_bins_table(self, layer: str = "road") -> Table | None:
        """``link_bins.parquet``: the per-link results as a table, or ``None``.

        ``layer="bike"`` or ``"walk"`` reads ``link_bins_bike.parquet`` or
        ``link_bins_walk.parquet`` instead: the same columns, except that the
        weighted count is ``travellers`` and its time ``traveller_seconds``.

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
        path = {
            "road": self._summary.link_bins_path,
            "bike": self._summary.link_bins_bike_path,
            "walk": self._summary.link_bins_walk_path,
        }[_check_layer(layer)]
        return None if path is None else Table(path)

    def kpis(self) -> Table:
        """``kpis.parquet``: long format, one row per metric.

        Columns ``run_id, design_id, replication, iteration, mode, metric, value``.
        ``mode`` is ``"all"`` for the whole run; the trip metrics of the last loading
        (``trips``, ``total_travel_time_s``, ``completed_trips``, ``truncated_trips``,
        ``no_vehicle_available_trips``, ``no_feasible_path_trips``,
        ``mode_not_available_trips``, ``completion_rate``) are also given for each mode
        that has trips (``"car"``, ``"bike"``, ``"walk"``, …). Transit adds
        ``boardings`` and the buses' numbers under ``"transit"``; the parkings add
        ``parking_arrivals``, ``parking_overflow_arrivals``, ``parking_full_share``,
        ``hub_mismatch_s`` and ``vehicles_left_at_parkings`` — car parkings under
        ``"car_transit"``, bike parkings under ``"bike_transit"`` — and the itineraries
        ``itinerary_replanned_trips`` and ``return_mismatch_s`` under ``"all"``.
        """
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
        """How many trips completed, and why the others did not, without reading any file.

        Keys: ``total_trips``, ``completed``, ``truncated``,
        ``no_vehicle_available``, ``no_feasible_path``, ``mode_not_available``
        (a trip whose mode this run cannot simulate: transit without a timetable,
        park-and-ride and bike-and-ride without parkings).
        """
        s = self._summary
        return {
            "total_trips": s.total_trips,
            "completed": s.completed,
            "truncated": s.truncated,
            "no_vehicle_available": s.no_vehicle_available,
            "no_feasible_path": s.no_feasible_path,
            "mode_not_available": s.mode_not_available,
        }

    @property
    def completion_by_mode(self) -> dict[str, dict[str, float]]:
        """``completion`` for each mode that has trips, and its travel time.

        ``{"car": {...}, "bike": {...}}``: the keys of ``completion`` and
        ``total_travel_time_s``, the mode's completed trips' travel time weighted by
        traveller weight. They add up to the run's.
        """
        return {mode: dict(row) for mode, row in self._summary.completion_by_mode.items()}

    def transit_calls_table(self) -> Table | None:
        """``transit_calls.parquet``: every call of the timetable, or ``None`` without one.

        The columns of ``transit_calls()``, after ``run_id``; a time a bus had not
        reached by the end of the window is null. The file's metadata carries the
        run's fingerprint, its master seed and the service date.
        """
        path = self._summary.transit_calls_path
        return None if path is None else Table(path)

    def transit_calls(self) -> dict[str, Any] | None:
        """Every call of the timetable in this run, as columns; ``None`` without one.

        One row per call (a run at a stop, in order): ``run`` (the GTFS
        ``trip_id``), ``line`` (its short name), ``kind`` (``"bus"``, ``"tram"``,
        ``"metro"``, ``"rail"``, ``"ferry"``, ``"other"``), ``stop_id``, ``sequence``
        (the call's place in its run, from 0), ``scheduled_arrival_s``,
        ``scheduled_departure_s``, and ``arrival_s``, ``departure_s`` — the times in
        the run: a bus that rode the roads keeps the loading's, the rest the
        schedule's; ``NaN`` where a bus had not got there by the end of the window —
        then ``boardings``, ``alightings`` and ``on_board`` (as the vehicle leaves),
        weighted by traveller weight. Seconds count from the service day's midnight.
        """
        calls = self._summary.transit_calls
        return None if calls is None else dict(calls)

    @property
    def transit_summary(self) -> dict[str, float] | None:
        """How transit went, as numbers by name; ``None`` without a timetable.

        ``boardings`` (weighted); and for the buses: ``bus_groups_on_roads`` (bus
        patterns that rode the roads), ``bus_groups_by_schedule_off_road``,
        ``bus_groups_by_schedule_no_route`` and ``bus_groups_by_schedule_implausible``
        (those run by the schedule, and why), ``bus_runs_on_roads``,
        ``bus_runs_arrived`` (reached their last stop within the window) and
        ``bus_delay_mean_s`` (their mean delay there, realised minus scheduled;
        negative is early).
        """
        summary = self._summary.transit_summary
        return None if summary is None else dict(summary)

    def parking_bins(self) -> dict[str, Any] | None:
        """How full each parking was over the day, bin by bin; ``None`` without parkings.

        Columns, one row per parking and bin: ``bin``, ``start_s``, ``parking`` (its
        index in the run), ``parking_id``, ``hub_id``, ``vehicle`` (``"car"`` or
        ``"bike"``), ``capacity``, ``arrivals`` and ``departures`` (vehicles parked and
        fetched in the bin, weighted by traveller weight), ``occupancy_mean``,
        ``occupancy_max``, ``full_s`` (seconds of the bin the parking was full) and
        ``overflow_max`` (the most vehicles above capacity at once: a full parking
        refuses nobody; it costs more, and the overflow is counted). The same table is
        written to ``parking_bins.parquet``: read it with ``parking_bins_table()``.
        """
        bins = self._summary.parking_bins
        return None if bins is None else dict(bins)

    def parking_bins_table(self) -> Table | None:
        """``parking_bins.parquet``: ``parking_bins()`` after ``run_id``; ``None`` without parkings.

        The file's metadata carries the run's fingerprint and its master seed.
        """
        path = self._summary.parking_bins_path
        return None if path is None else Table(path)

    def parking_places(self) -> dict[str, Any] | None:
        """Each parking the run kept, as columns; ``None`` without parkings.

        ``parking_id``, ``hub_id``, ``vehicle``, ``capacity``, ``lon``, ``lat`` and
        ``name``. ``parking`` in ``parking_bins()`` indexes these.
        """
        places = self._summary.parking_places
        return None if places is None else dict(places)

    @property
    def parking_summary(self) -> dict[str, float] | None:
        """What the parkings did, as numbers by name; ``None`` without parkings.

        ``parkings_given``, ``parkings_car`` and ``parkings_bike`` (kept),
        ``parkings_off_layer`` and ``parkings_no_stop`` (left out: too far from their
        layers, or no stop within walking distance); and for ``car`` and ``bike``:
        ``*_arrivals`` (vehicles parked), ``*_overflow_arrivals`` (of those, the ones
        that found the parking full), ``*_mismatch_s`` (the hub expectation mismatch)
        and ``*_left_at_end`` (still parked when the window ended).
        """
        summary = self._summary.parking_summary
        return None if summary is None else dict(summary)

    def itinerary_choices(self) -> dict[str, Any] | None:
        """Each transit, park-and-ride and bike-and-ride trip's choice; ``None`` without any.

        Columns, one row per such trip, by traveller and then by the order of their day:
        ``trip`` (its index in the run, where trips are grouped by traveller),
        ``traveller_id`` and ``trip_seq`` (its place in that traveller's day, from 0),
        ``mode``, ``parking_id`` (the parking used; ``None`` for plain transit or a trip
        that did not travel), ``direction`` (``"out"``: vehicle first; ``"back"``: transit
        first, to where the vehicle is; ``""`` otherwise), ``alternatives`` (how many it
        chose among; 0: it did not travel), ``probability`` (what the model gave its
        choice), ``expected_s`` (the chosen itinerary's door-to-door time expected at the
        choice) and ``rides`` (vehicles boarded).
        """
        choices = self._summary.itinerary_choices
        return None if choices is None else dict(choices)

    @property
    def total_travel_time_s(self) -> float:
        """Total travel time of the completed trips, in seconds, scaled by traveller weight.

        Each trip counts as many times as the people it stands for (its
        traveller weight): the population's total, not the sum over simulated
        travellers.
        """
        return self._summary.total_travel_time_s

    def __repr__(self) -> str:
        """The completion counts, as a one-line summary."""
        c = self.completion
        return (
            f"Run(completed={c['completed']}, truncated={c['truncated']}, "
            f"no_vehicle_available={c['no_vehicle_available']}, "
            f"no_feasible_path={c['no_feasible_path']})"
        )


BIKE_COSTS = ("dedicated", "time")
LAYERS = ("road", "bike", "walk")


def _check_layer(layer: str) -> str:
    if layer not in LAYERS:
        raise ValueError(f"layer must be one of {LAYERS}, got {layer!r}")
    return layer


def _trip_row(row: tuple) -> tuple:
    """A trips row with its mode: nine fields take none (a car trip), ten give it."""
    if len(row) == 9:
        return (*row, None)
    if len(row) == 10:
        return tuple(row)
    raise ValueError(
        "a trips row has 9 fields (traveller_id, trip_seq, origin_lon, origin_lat, "
        "destination_lon, destination_lat, departure_time_s, user_class, weight) or 10 "
        f"(and mode), not {len(row)}: {row!r}"
    )


class Scenario:
    """A network, demand, and the settings of a run.

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
        flow_level: int = 4,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str | None = None,
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any = "logit",
        choice_options: dict[str, float] | None = None,
        equilibration: str = "msa",
        equilibration_options: dict[str, float] | None = None,
        route_update: str | None = None,
        route_update_options: dict[str, float] | None = None,
        choice_detour_limit: float | None = None,
        route_cache: bool = False,
        bike_cost: str = "dedicated",
        transit: _core.Transit | None = None,
        transit_options: dict[str, float] | None = None,
        parkings: _core.Parkings | None = None,
        parking_options: dict[str, float] | None = None,
    ) -> None:
        """Store the parts; prefer `from_parts` to calling this directly."""
        if bike_cost not in BIKE_COSTS:
            raise ValueError(f"bike_cost must be one of {BIKE_COSTS}, got {bike_cost!r}")
        if trips is not None:
            trips = [_trip_row(row) for row in trips]
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
        iterates = (
            equilibration != "none" and int((equilibration_options or {}).get("iterations", 10)) > 1
        )
        if route_method is None:
            # An iterating run starts from one route per pair and lets the route update find the
            # rest (S179); a run that loads once has no next iteration to choose among new
            # routes. Options given for a method mean the default method of a single loading.
            route_method = "shortest" if iterates and route_options is None else "penalty"
        if route_update is None:
            route_update = "best_response" if iterates or route_update_options else "none"
        if iterates and equilibration == "msa" and "warmup" not in (equilibration_options or {}):
            equilibration_options = {**(equilibration_options or {}), "warmup": 1}
        self._route_method = route_method
        self._route_options = route_options
        self._master_seed = master_seed
        self._choice_model = choice_model
        self._choice_options = choice_options
        self._equilibration = equilibration
        self._equilibration_options = equilibration_options
        self._route_update = route_update
        self._route_update_options = route_update_options
        self._choice_detour_limit = choice_detour_limit
        self._route_cache = route_cache
        self._bike_cost = bike_cost
        self._transit = transit
        self._transit_options = transit_options
        self._parkings = parkings
        self._parking_options = parking_options

    @classmethod
    def from_parts(
        cls,
        network: _core.Network,
        demand: list[tuple] | str,
        persons: list[tuple] | str | None = None,
        class_defaults: dict[str, tuple[bool, bool, bool]] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
        flow_level: int = 4,
        flow_step_s: int = 300,
        link_bin_s: int | None = None,
        route_method: str | None = None,
        route_options: dict[str, float] | None = None,
        master_seed: int = 0,
        choice_model: str | Any = "logit",
        choice_options: dict[str, float] | None = None,
        equilibration: str = "msa",
        equilibration_options: dict[str, float] | None = None,
        route_update: str | None = None,
        route_update_options: dict[str, float] | None = None,
        choice_detour_limit: float | None = None,
        route_cache: bool = False,
        bike_cost: str = "dedicated",
        transit: _core.Transit | None = None,
        transit_options: dict[str, float] | None = None,
        parkings: _core.Parkings | None = None,
        parking_options: dict[str, float] | None = None,
    ) -> Scenario:
        """Build a scenario from a network and demand.

        Args:
            network: Built by, for example, ``examples.manhattan_grid``.
            demand: Either an in-memory trips table (rows in the schema
                ``examples.fixed_car_trips`` returns, or hand-built the same
                way) or a path to a ``trips.parquet`` file — the same
                schema either way, never two different shapes. A row may end with
                a **mode** (the ``mode`` column of ``trips.parquet``): ``"car"``,
                ``"bike"``, ``"walk"``, ``"transit"``, or the not-yet-built
                ``"car_transit"``, ``"bike_transit"``. A trip without one is a car
                trip. Bike and walk trips travel on the network's bike and walk
                layers (``network.layer("bike")``), each by its shortest route
                there; a bike trip needs the traveller's bike at its origin, as a
                car trip needs their car. A transit trip needs ``transit``: it
                walks to a stop, rides and walks on (see ``transit``).
            persons: The ``persons.parquet`` equivalent — an in-memory table,
                a file path, or ``None`` if every traveller takes their
                class's default ownership.
            class_defaults: ``{class_name: (owns_car, owns_bike,
                has_transit_pass)}`` — what each class owns by default. A class not
                listed here owns nothing unless a `persons` row overrides it.
            default_weight: How many people a simulated traveller stands for,
                for a trip whose row gives no weight. 1 simulates everyone; a
                larger number is faster and coarser.
            window_hours: Trips still in progress after this many hours are
                truncated.
            flow_level: How vehicles load the network. ``4`` (the default) is
                the link transmission model with the full triangular
                fundamental diagram: queues that take up road space and spill
                back to the links upstream. ``3`` is a spatial queue and ``2`` a
                point queue (queues without spillback, so no gridlock). ``0`` is
                free flow: no vehicle affects another, so there is no
                congestion, and a run that iterates has nothing to settle — for
                debugging, or as a lower bound.
            flow_step_s: The loading step in seconds, for levels 2-4. It is a
                bookkeeping boundary: results do not depend on it.
            link_bin_s: If given, also record per-link results in time bins of
                this many seconds, read with ``Run.link_bins()`` and drawn
                with ``openmobisim.viz.map_link``.
            route_method: How route sets are generated, by name (see
                ``openmobisim.route_methods()``). **The default depends on the run:** a run
                that loads once uses ``"penalty"``; a run that **iterates** (an
                ``equilibration`` with more than one iteration) starts from ``"shortest"``, one
                route per pair, and lets the route update find the rest (measured: as good
                as the alternatives of the other methods at about half the time). Naming a
                method, or giving ``route_options``, overrides this (options alone mean
                ``"penalty"``). ``"penalty"`` finds
                distinct alternatives by penalising the links of the routes found.
                ``"shortest"`` makes one route per pair. ``"montecarlo"`` finds routes
                by searching under random link costs **biased towards the links the
                demand is likely to congest** (the busiest half-hour's load on shortest
                routes over each link's capacity, from this scenario's trips and their
                weights): routes that go round what the demand will jam. It has been
                measured on few networks: better alternatives than ``"penalty"`` on one
                city's heavy load, worse on another's. Which route a trip takes among its
                alternatives is ``choice_model``'s business.
            route_options: The method's options, numbers by name; unknown names
                and out-of-range values are refused. For ``"penalty"``: ``max_paths``
                (5), ``max_detour`` (1.3), ``max_overlap`` (0.75), ``penalty`` (1.5),
                ``max_attempts`` (15). For ``"montecarlo"``: ``draws`` (16),
                ``sigma`` (4: how far a link's cost may rise, times its bias),
                ``max_detour`` (2), ``max_overlap`` (0.9), ``max_paths`` (10), ``seed``
                (0: another seed is another set of draws) and ``biased`` (1; 0 for no
                bias, which measured worse on the one heavy load it was tried on).
            master_seed: The one number that starts every random stream, so a
                run is reproduced by giving it the same one. Under a sampled
                choice model (``"logit"``, the default) it decides who takes
                which route; under ``"deterministic"`` nothing draws from it
                and it changes only the run's fingerprint. A non-negative
                integer below 2**64.
            choice_model: How each trip picks a route from its pair's set: a
                name from ``openmobisim.choice_models()`` or an object with a
                ``choose(batch)`` method (see ``openmobisim.choice`` for how to
                write one). ``"logit"`` (the default since the car-ready
                checkpoint, 2026-09-23) is the standard path-size logit, sampled per
                traveller; ``"deterministic"`` sends everyone down the best
                route, with probability 1 — useful for debugging or an upper
                bound, not for a result to report.
            choice_options: A built-in model's options, numbers by name: for
                ``"logit"`` a coefficient per attribute, ``beta_time_min`` (default
                -0.2), ``beta_ln_path_size`` (1), ``beta_length_km``,
                ``beta_detour``, ``beta_overlap``, ``beta_n_links`` (0). Unknown
                names and non-numbers are refused. The defaults are an
                assumption, not a calibration.
            choice_detour_limit: A **time-dependent choice set**: a traveller is offered only the
                routes of the pair's set whose expected time, at the times of the last loading
                (free flow for the first choice), is within this share of the best route's:
                ``0.5`` offers a route only if it is at most 50% slower than the best right now.
                A route far slower than the best is no realistic alternative, and in a logit it
                only takes probability that belongs to routes that compete (the gap grows with the
                size of the set). ``0`` offers every route of the set; ``None`` (the default) is the
                library's default, **0.5**. The best route is always offered; a route left out is
                still in the set and returns when times change. The gaps are still measured
                against the whole set. The ``ln_path_size`` attribute is computed over the whole
                set.
            bike_cost: How bike trips choose their route on the bike layer: ``"dedicated"``
                (the default) is the fastest route with every minute in mixed traffic
                counting a little more (1.2 times), so cyclists go a little out of their way
                for a cycle track or lane; ``"time"`` is the fastest route. Bikes ride at 15
                km/h in mixed traffic and 18 km/h on dedicated infrastructure; these values
                and the 1.2 are defaults, not a calibration.
            route_cache: Keep the generated route sets in memory for the next run **of this
                process** that asks for the same: the same network, the same origin-destination
                pairs, the same route method and options and, for a method that reads the demand
                (``"montecarlo"``), the same trips. Generating them is the largest single cost of
                a run that iterates, so a study that runs one scenario many times, changing only
                the equilibration, the choice model, the flow level or a seed, pays it once. Results
                do not depend on whether the cache was used. Holds at most four sets (about 17 MB at
                the scale of a country's network each); ``openmobisim.route_cache_clear()`` forgets
                them, ``openmobisim.route_cache_info()`` says ``(hits, misses, held)``.
            equilibration: How choice and loading are repeated (see
                ``openmobisim.equilibration_strategies()``). ``"msa"`` (the
                default since the car-ready checkpoint, 2026-09-23) is the method of successive
                averages in traveller form: load the network, read the link
                times it produced, let a share ``1/(i + 1)`` of travellers
                choose again on those times at iteration ``i``, load again.
                ``"none"`` chooses every trip's route once, on free-flow
                costs, and loads the network once — still the right choice
                for a one-shot, day-of or disruption study.
                Either way, equilibration only changes anything when vehicles
                interact (``flow_level`` 2 to 4, the default 4) and a choice
                model gives travellers something to choose between
                (``"logit"``): at ``flow_level=0`` (free flow, no interaction),
                ``"msa"`` still runs but has nothing to disagree with itself
                about, so it settles immediately at the choice model's own
                floor. ``Run.convergence()`` says how
                the iterations went, and ``Run.convergence_verdict`` judges
                the disequilibrium the last iteration ended at.
            equilibration_options: The strategy's options, numbers by name: for
                ``"msa"``, ``iterations`` (10; the most loadings), ``gap_tolerance``
                (0: never stop early; otherwise stop once the gap, averaged over the
                last three iterations, is below this: 0.05 is good, 0.15 acceptable),
                ``gap_sample`` (300; how many trips are tested against the whole
                network at the last iteration, 0 for none) and ``cost_bin_s`` (300: the
                length of the time bins the link times are read in) and ``warmup`` (1 for a run
                that iterates, else 0: how many of
                the first loadings use the point-queue model, which cannot gridlock; the last
                loading, the run's result, always uses ``flow_level``. A narrow route set is often
                in gridlock in its first loading, and every later iteration inherits its times).
            route_update: How the route sets grow between iterations (see
                ``openmobisim.route_update_methods()``). **The default depends on the run:**
                ``"best_response"`` when the run iterates (or ``route_update_options`` is
                given), ``"none"`` when it loads once. ``"none"`` leaves
                the sets as the route method made them, at free-flow costs. ``"best_response"``
                is for a run that iterates: after each loading it searches, for every
                origin-destination pair, the fastest route at the congested times the
                loading produced, and adds it to the pair's set if it is new and at least as
                fast as the best route already there, before travellers choose again. It
                closes the gap that ``Run.convergence()["gap_network"]`` shows when traffic
                has made a route worthwhile that no set held. Where nothing queues it adds
                little or nothing. It is for congestion that is heavy but not gridlock: near
                gridlock it can make a run worse. **The equilibrium is then over the sets the
                route method and the update produce**, which the run's fingerprint and
                manifest say.
            route_update_options: The update's options, numbers by name: for
                ``"best_response"``, ``searches`` (1: how many searches per pair per
                iteration, at spread quantiles of the pair's departures; more finds routes
                that pay only at some hours and makes bigger sets) and ``max_routes`` (10:
                the most routes a pair's set may hold; a pair at the limit is not searched) and
                ``slack`` (0: a pair is not searched while its set's best route is within this
                share of free flow, since nothing is faster than free flow and so at most that
                much is left to gain).
                Unknown names and out-of-range values are refused.
            transit: The timetable of a service day (``openmobisim.transit_read_gtfs``),
                for the ``"transit"`` trips. A transit trip walks (4.8 km/h, at most 15
                minutes) to a stop, rides one or more vehicles and walks on, boarding only if
                at the stop a minute before a vehicle leaves; walks between stops are at most
                5 minutes. **Its journey is chosen by** ``choice_model`` among the
                competitive ones: for each number of vehicles, the earliest arrival that beats
                every journey with fewer (the attributes ``walk_min``, ``wait_min``,
                ``ride_min`` and ``transfers`` say what it costs). It is chosen on the times
                expected — the schedule at first, the last iteration's after — and ridden on
                the times the run makes: the traveller boards the first vehicle of each line
                chosen they can catch. The run's clock starts at the service day's midnight.
                **Buses ride the roads**, 2 PCU each, among the cars: routed stop to stop at
                free flow, dwelling 20 s at a stop without blocking the traffic behind, never
                leaving a stop before their scheduled time, so congestion makes them late
                and their passengers with them. A bus line the roads cannot carry plausibly
                runs by the schedule (``Run.transit_summary`` counts them). Rail, metro, trams
                and ferries run by the schedule. These values are defaults, not a calibration:
                ``transit_options`` overrides them.
            transit_options: Transit's parameters by name, each replacing its default:
                ``board_slack_s`` (60), ``max_rides`` (8), ``access_walk_max_s`` (900),
                ``transfer_walk_max_s`` (300), ``stop_walk_snap_m`` (300),
                ``stop_transfer_s`` (0), ``bus_dwell_s`` (20), ``bus_pcu`` (2),
                ``bus_plausibility_ratio`` (1.6), ``bus_stop_snap_m`` (300). Unknown names
                and out-of-range values are refused.
            parkings: Park-and-ride car parks and bike parkings
                (``openmobisim.parking_read_osm``, ``openmobisim.parking_read_table``), for
                the ``"car_transit"`` and ``"bike_transit"`` trips; needs ``transit``. **The
                parking is chosen by** ``choice_model``: a trip whose car (or bike) is at its
                origin drives (rides) to one of the parkings within reach, parks, walks to a
                stop and rides on; the alternatives are the parking × the route to it × the
                journey from it, and ``parking_min`` says what parking costs. A trip whose
                vehicle is parked goes back by transit to where it is, fetches it and drives
                (rides) on. **Soft capacity:** a full parking refuses nobody; parking there
                takes longer (``parking_options``), the overflow is counted
                (``Run.parking_bins()``), and the choice sees how full each parking was in
                the iterations so far.
            parking_options: Parking's parameters by name, each replacing its default:
                ``walk_max_s`` (300: the longest walk from a parking to a stop),
                ``reach_car_s`` (1800) and ``reach_bike_s`` (1200: the longest drive or ride
                to a parking), ``candidates`` (5: the most parkings a trip chooses among),
                ``rank_speed_km_h`` (30: ranks candidates before they are tried),
                ``floor_car_s`` (120) and ``floor_bike_s`` (30: the time to park when there
                is room, and to fetch), ``slope_car_s`` (600) and ``slope_bike_s`` (180: what
                a parking full for a whole time bin adds), ``snap_m`` (300) and ``bin_s``
                (900). Uncalibrated defaults. Unknown names and out-of-range values are
                refused.

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
            route_update=route_update,
            route_update_options=route_update_options,
            choice_detour_limit=choice_detour_limit,
            route_cache=route_cache,
            bike_cost=bike_cost,
            transit=transit,
            transit_options=transit_options,
            parkings=parkings,
            parking_options=parking_options,
        )

    def run(self, run_id: str = "run", output_dir: str | None = None) -> Run:
        """Run the scenario and write its output artifacts.

        Args:
            run_id: Recorded in every output row; also names the default
                output directory.
            output_dir: Where ``kpis.parquet`` etc. are written. Defaults to
                a directory under the system temp directory, named after
                ``run_id``.

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
            route_update=self._route_update,
            route_update_options=self._route_update_options,
            choice_detour_limit=self._choice_detour_limit,
            route_cache=self._route_cache,
            bike_cost=self._bike_cost,
            transit=self._transit,
            parkings=self._parkings,
            parking_options=self._parking_options,
            transit_options=self._transit_options,
        )
        return Run(
            summary,
            network=self._network,
            run_id=run_id,
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            window_s=self._window_s,
            transit=self._transit,
            parkings=self._parkings,
        )
