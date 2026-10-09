"""``Scenario`` and what ``.run()`` returns.

A network, demand (an in-memory table or a ``trips.parquet`` path — the same
schema either way), a loading with queues at capacity (the point queue, ``flow_level=2``),
route choice among each trip's alternatives (a path-size ``"logit"``) and
iteration towards an equilibrium (``"msa"``), during which the route sets may
grow (``route_update="best_response"``) — all by default. Trips drive, cycle,
walk, take transit (``transit=``) or drive or cycle to a parking and go on by
transit (``parkings=``). Roads and lines can be disrupted at a time of day
(``disruptions=``).
"""

from __future__ import annotations

import json
import tempfile
import warnings
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from openmobisim import _core
from openmobisim.demand import _class_table, demand_read_trips

__all__ = ["Run", "Scenario", "Table"]

#: The flow levels ``Scenario`` accepts: 0 is free flow
#: with no interaction between vehicles; 2, 3 and 4 are the link transmission
#: model as a point queue, a spatial queue and the full triangular diagram.
FLOW_LEVELS = (0, 2, 3, 4)

#: In how many groups the free-flow loading is built up unless ``equilibration_options`` says
#: (S223; uncalibrated, the user's suggestion: groups of a fifth of the travellers).
FREE_FLOW_INCREMENTS = 5

#: ``"msa"`` stops once the disequilibrium, averaged over the last three iterations, is below this
#: (S229; uncalibrated: a tight "good", which is below 0.05), unless ``equilibration_options``
#: says otherwise; ``iterations`` stays the most it makes.
GAP_TOLERANCE = 0.02

#: ... and while the last route update added routes for at most this share of the pairs it
#: searched (S229). 1 by default: the gap alone decides, since it is measured against the sets as
#: grown (a route found faster than the set's best raises it; an equally fast one, which grids
#: keep yielding, does not). Lower it to also wait for the sets to settle.
ROUTE_GROWTH_TOLERANCE = 1.0

#: What ``Run.timings()`` calls each stage, as the printout shows it (S223).
_STAGE_LABELS = {
    "network_read": "network read (before the run)",
    "demand_read": "demand read",
    "setup": "setup (layers, transit, parkings)",
    "static_routes": "bike and walk routes",
    "itineraries_setup": "itineraries prepared",
    "route_sets": "route sets",
    "results_assembled": "results assembled",
    "results_written": "results written",
    "total": "run total (without the network read)",
}


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
        """The name of the equilibration strategy: ``"free_flow"`` or ``"msa"``."""
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

        **The worst of** the last iteration's disequilibrium against the choice set
        (``convergence()["gap_excess"][-1]``), against the whole network
        (``convergence()["gap_network_excess"][-1]``, a sample), so a route the choice set
        was missing cannot hide behind a small in-set number, and of the itineraries
        (``convergence()["itinerary_gap_excess"][-1]``: transit, park-and-ride, bike-and-ride
        and trips choosing their mode, each against the best of the mode it kept; S227). The
        disequilibrium is the gap **less what the choice model itself expects** (a logit sends
        some travellers down a slower route by design, so its gap is never 0); where the model
        gives no probabilities the plain gaps are used instead.

        ``nan`` if no gap was measured (a ``"free_flow"`` run). See
        ``convergence_verdict``.
        """
        c = self._summary.convergence
        values = []
        for excess, plain in (("gap_excess", "gap"), ("gap_network_excess", "gap_network")):
            value = float(c[excess][-1])
            values.append(value if value == value else float(c[plain][-1]))
        values.append(float(c["itinerary_gap_excess"][-1]))
        finite = [v for v in values if v == v]  # not nan
        return max(finite) if finite else float("nan")

    @property
    def convergence_verdict(self) -> str | None:
        """``"good"`` (disequilibrium below 5%), ``"acceptable"`` (below 15%) or ``"poor"``.

        ``None`` if no gap was measured (a ``"free_flow"`` run). Judged on
        ``convergence_gap``, the disequilibrium: what the choice model does not explain.
        """
        gap = self.convergence_gap
        if gap != gap:  # nan
            return None
        return "good" if gap < 0.05 else "acceptable" if gap < 0.15 else "poor"

    def convergence(self) -> dict[str, Any]:
        """What each iteration showed: how far the run is from an equilibrium.

        NumPy arrays, one entry per iteration (a ``"free_flow"`` run has one).
        ``nan`` marks a number that was not measured. The result of the run
        (``completion``, ``total_travel_time_s``, ``link_bins()``, ``route_choices()``)
        is that of the **last** iteration.

        * ``iteration`` — 0 is the free-flow loading: everyone's first choice, in groups each
          on the congestion of those before it (``increments``), loaded with the point queue.
        * ``reselected_share``, ``changed_share`` — the share of trips (by weight) that
          chose again on the way to this iteration, and the share whose route changed.
        * ``total_travel_time_s``, ``completed``, ``truncated`` — this loading's;
          ``completed_people``, the people its completed trips stand for (their travellers'
          weights, summed), so its mean trip is ``total_travel_time_s / completed_people``.
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
          travellers may choose from next. Iteration 0's also counts what the update found
          while the free-flow loading was built up in groups.
        * ``reroutes``, ``reroute_searches`` — en-route rerouting in this loading: how many
          times a vehicle took a new route, and how many searches for one were made (one per
          offer to a vehicle stuck at the end of its link, taken or not). Iteration 0's
          searches include those of the free-flow loading's groups.

        * ``gap_transit``, ``gap_car_transit``, ``gap_bike_transit`` — the same relative gap
          for the itineraries of transit, park-and-ride and bike-and-ride trips: each
          itinerary kept against the least door-to-door time of its choice set at the costs
          this loading produced; ``nan`` for a mode without such trips. ``gap`` above is the
          car routes'. With mode choice (``modes``), a trip's itinerary is measured against
          the best of its own mode (with mode constants, the fastest mode need not be the
          best), and ``gap_car``, ``gap_bike`` and ``gap_walk`` are the same for the trips
          that chose those modes (``gap`` above covers the car trips given their mode).
          With ``itinerary_gap_sample`` set below the number of such trips, it is **measured
          on a sample** before the last iteration: the trips choosing again and that many
          others plan their whole choice set; the rest that keep an itinerary out through a
          parking are re-costed there. The last iteration's is always exact.
        * ``itinerary_recosted`` — how many trips were re-costed that way rather than
          re-planned; 0 at the last iteration.
        * ``hub_mismatch_s`` — the hub expectation mismatch: per traveller who parked, the
          mean difference between the parking time expected when they chose and the one
          they paid, in seconds; ``nan`` without parkings.
        * ``mode_changed_share`` — with ``modes``: of the trips choosing their mode that
          had chosen before, the share (by traveller weight) whose mode changed on the way
          to this loading; ``nan`` at the first iteration and without ``modes``. How the
          mode split is settling.
        * ``mode_changed_floor`` — what that share would be from the choice model's own
          randomness alone (among the trips that chose again, the probability of a mode other
          than the one they had); the settling is done when the share is near its floor.
        * ``itinerary_gap_excess`` — **the itineraries' disequilibrium**: over the transit,
          park-and-ride, bike-and-ride and mode-choosing trips, the gap to the best alternative
          of the mode each kept, less what the choice model itself expects. Part of
          ``convergence_gap`` and of what ``gap_tolerance`` stops on (S227).

        The numbers up to ``gap_flow_excess`` are also in ``kpis.parquet``, a row per metric
        per iteration, and so are the itinerary gaps (under their modes, as ``gap``),
        ``hub_mismatch_s`` and ``mode_changed_share``.
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
        ``itinerary_replanned_trips`` and ``return_mismatch_s`` under ``"all"``. With mode
        choice (``modes``) each trip counts under the mode it took, each mode adds its
        ``mode_share`` (of the trips, weighted by the travellers), and every iteration adds
        ``mode_changed_share`` under ``"all"``.
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

    def timings(self) -> dict[str, list[Any]]:
        """Wall-clock time by stage, measured on this machine's clock (S223).

        Columns by name, one row per stage in the order the stages ended: ``stage``,
        ``iteration`` (``None`` for a stage outside the iterations) and ``seconds``. Before the
        iterations: ``network_read`` (if the network recorded how long its reading took; it was
        read before the run, and is not in the total), ``demand_read``, ``setup`` (the bike and
        walk layers, the timetable and the parkings prepared, the fingerprint),
        ``static_routes``, ``itineraries_setup``, ``route_sets``. Per iteration:
        ``route_choice`` and ``itinerary_choice``, ``partial_loading`` and ``route_update``
        (iteration 0's: the free-flow loading's groups, one row each), ``loading``, then after a
        loading what prepares the next (``route_update``, ``route_choice``,
        ``itinerary_choice``) and ``network_gap`` after the last. After them:
        ``results_assembled``, ``results_written`` and ``total`` (the run alone). Also in
        ``timings.csv`` next to the other files. Times differ between runs, so they are in no
        result and not in the fingerprint.
        """
        rows = self._summary.timings
        return {
            "stage": [r[0] for r in rows],
            "iteration": [r[1] for r in rows],
            "seconds": [r[2] for r in rows],
        }

    @property
    def timings_path(self) -> str:
        """Path to ``timings.csv``: ``Run.timings()`` as a file."""
        return self._summary.timings_path

    def _timings_text(self) -> str:
        """``Run.timings()`` as the run prints it.

        The stages outside the iterations, then a table of the iterations with one column per
        kind of stage.
        """
        columns = [
            ("partial_loading", "partial loadings"),
            ("loading", "loading"),
            ("route_update", "route update"),
            ("route_choice", "route choice"),
            ("itinerary_choice", "itinerary choice"),
            ("network_gap", "network gap"),
        ]
        rows = self._summary.timings
        per: dict[int, dict[str, float]] = {}
        outside: list[tuple[str, float]] = []
        for stage, iteration, seconds in rows:
            if iteration is None:
                outside.append((stage, seconds))
            else:
                cell = per.setdefault(iteration, {})
                cell[stage] = cell.get(stage, 0.0) + seconds
        lines = [f"Run {self.run_id!r}: wall-clock seconds by stage"]
        width = max(len(label) for label in _STAGE_LABELS.values())
        iterations_total = sum(sum(c.values()) for c in per.values())
        for stage, seconds in outside:
            if stage == "results_assembled" and per:
                lines.append(f"  {'iterations (below)':<{width}}  {iterations_total:9.2f}")
            lines.append(f"  {_STAGE_LABELS.get(stage, stage):<{width}}  {seconds:9.2f}")
        used = [(key, label) for key, label in columns if any(key in c for c in per.values())]
        if per:
            header = ["iteration"] + [label for _, label in used] + ["total"]
            lines.append("")
            lines.append("  " + "  ".join(f"{h:>{max(len(h), 9)}}" for h in header))
            for iteration in sorted(per):
                cell = per[iteration]
                values = [cell.get(key, 0.0) for key, _ in used] + [sum(cell.values())]
                parts = [f"{iteration:>9}"] + [
                    f"{v:>{max(len(h), 9)}.2f}" for h, v in zip(header[1:], values, strict=True)
                ]
                lines.append("  " + "  ".join(parts))
        return "\n".join(lines)

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
        traveller weight, ``people`` (the people its trips stand for: their travellers'
        weights, summed) and ``completed_people`` (the same for its completed trips). The
        counts of ``completion`` are of simulated trips; a mode's mean trip is
        ``total_travel_time_s / completed_people``. They add up to the run's.
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
        """Each itinerary trip's choice, and each choice of mode; ``None`` without any.

        One row per transit, park-and-ride and bike-and-ride trip, and per trip choosing its mode
        (``modes``), by traveller and then by the order of their day. Columns: ``trip`` (its index
        in the run, where trips are grouped by traveller), ``traveller_id`` and ``trip_seq`` (its
        place in that traveller's day, from 0), ``user_class`` (the traveller's class, S231),
        ``mode`` (the mode taken; ``None`` for a trip choosing its mode that had nothing to choose
        from), ``mode_choice`` (whether the trip chose its mode), ``parking_id`` (the parking used;
        ``None`` for plain transit or a trip that did not travel), ``direction`` (``"out"``: vehicle
        first; ``"back"``: transit first, to where the vehicle is; ``""`` otherwise),
        ``alternatives`` (how many it chose among when it last chose; 0: it did not travel),
        ``probability`` (what the model gave its choice), ``expected_s`` (the chosen itinerary's
        door-to-door time expected at the choice), ``rides`` (vehicles boarded) and ``logsum``
        (S238: the **logsum** of the trip's choice set when it was last planned — the expected
        utility of its best alternative, ``ln Σ exp(utility)`` for the logit and its nested form
        for the nested logit; ``nan`` for a model that gives none; a model of one's own may give
        one with a ``logsum(batch)`` method, one number per situation). In the utility's units:
        divided by ``-beta_time_min`` it reads in minutes. It is the standard utility-based
        accessibility measure: compare a trip's logsum between two scenarios, or average it by
        class or by origin zone.
        """
        choices = self._summary.itinerary_choices
        return None if choices is None else dict(choices)

    def logsum_od(
        self,
        trips: Any,
        zones: Mapping[str, tuple[float, float]] | None = None,
        *,
        by_class: bool = False,
    ) -> dict[str, list[Any]]:
        """The trips' logsums by origin-destination pair (S245): utility-based accessibility.

        The logsum of a trip's choice set (``itinerary_choices()["logsum"]``) averaged over the
        trips of each pair, weighted by the people they stand for. Trips with a logsum are those
        that chose their mode or an itinerary (transit, park-and-ride, bike-and-ride); a trip
        given another mode has none and is left out. Compare a pair's logsum between two runs
        (two designs): the change, divided by ``-beta_time_min`` (0.2 by default), is what the
        change is worth in minutes per trip; the level alone means nothing (a logsum is defined
        up to a constant).

        Args:
            trips: The trips the run was given: rows of ``Scenario.from_parts``' ``demand``, or a
                ``trips.csv`` path.
            zones: ``{zone: (lon, lat)}`` (``demand_read_zones``): each trip's origin and
                destination go to the nearest zone point. ``None``: the pair of drivable nodes
                nearest them (the nodes the trip starts and ends at).
            by_class: Also by the traveller's class.

        Returns:
            Columns by name, one row per pair (and class) with at least one logsum, sorted:
            ``origin``, ``destination`` (zone names, or node indices), ``user_class`` (with
            ``by_class``), ``trips`` (simulated), ``people`` (their weights) and ``logsum`` (the
            people-weighted mean, in the utility's units).

        Raises:
            ValueError: If the run made no itinerary choice, or a trip with a logsum is not in
                ``trips``.
        """
        import numpy as np

        choices = self.itinerary_choices()
        if choices is None:
            raise ValueError("this run made no itinerary or mode choice, so it has no logsums")
        rows = demand_read_trips(trips) if isinstance(trips, (str, Path)) else trips
        ends = {(str(r[0]), int(r[1])): (r[2], r[3], r[4], r[5]) for r in rows}
        modes = self.trip_modes()
        people = {
            (str(t), int(s)): float(w)
            for t, s, w in zip(
                modes["traveller_id"], modes["trip_seq"], modes["weight"], strict=True
            )
        }
        # A NaN logsum: no choice set (a trip that did not travel).
        keep = [i for i, v in enumerate(choices["logsum"]) if not np.isnan(v)]
        keys = [(str(choices["traveller_id"][i]), int(choices["trip_seq"][i])) for i in keep]
        missing = [k for k in keys if k not in ends]
        if missing:
            raise ValueError(f"{len(missing)} trips of the run are not in trips, e.g. {missing[0]}")
        points = np.array([ends[k] for k in keys], dtype=float).reshape(-1, 4)
        if zones is not None:
            names = list(zones)
            z = np.array([zones[n] for n in names], dtype=float)
            scale = np.cos(np.radians(float(np.mean(z[:, 1])) if len(z) else 0.0))

            def nearest(lon: np.ndarray, lat: np.ndarray) -> list[str]:
                out = []
                for a in range(0, len(lon), 4096):  # chunks keep the distance matrix small
                    dx = (lon[a : a + 4096, None] - z[None, :, 0]) * scale
                    dy = lat[a : a + 4096, None] - z[None, :, 1]
                    out.extend(names[j] for j in np.argmin(dx * dx + dy * dy, axis=1))
                return out

            origin = nearest(points[:, 0], points[:, 1])
            destination = nearest(points[:, 2], points[:, 3])
        else:
            if self._network is None:
                raise ValueError("give zones: this run does not know its network")
            snap = self._network.node_nearest
            origin = [snap(lon, lat) for lon, lat in points[:, :2]]
            destination = [snap(lon, lat) for lon, lat in points[:, 2:]]
        groups: dict[tuple, list[float]] = {}
        for j, i in enumerate(keep):
            key = (origin[j], destination[j]) + ((choices["user_class"][i],) if by_class else ())
            w = people[keys[j]]
            g = groups.setdefault(key, [0.0, 0.0, 0.0])
            g[0] += 1
            g[1] += w
            g[2] += w * float(choices["logsum"][i])
        out: dict[str, list[Any]] = {"origin": [], "destination": []}
        if by_class:
            out["user_class"] = []
        out |= {"trips": [], "people": [], "logsum": []}
        for key in sorted(groups):
            n, w, s = groups[key]
            out["origin"].append(key[0] if zones is not None else int(key[0]))
            out["destination"].append(key[1] if zones is not None else int(key[1]))
            if by_class:
                out["user_class"].append(key[2])
            out["trips"].append(int(n))
            out["people"].append(w)
            out["logsum"].append(s / w if w else float("nan"))
        return out

    def skim(
        self,
        mode: str,
        origins: Any,
        destinations: Any = None,
        *,
        departure_s: float = 8 * 3600,
        max_s: float = 3 * 3600,
    ) -> Any:
        """A skim: seconds from each origin to each destination by ``mode``, after this run (S238).

        The door-to-door time leaving at ``departure_s`` (seconds after midnight), by
        ``"car"`` — the earliest arrival on the **last loading's link times**, congestion by
        time bin included (free flow after a run of one loading), nearest drivable node to
        nearest — ``"bike"`` or ``"walk"`` — the time of the least-cost route on the layer, as
        the trips ride or walk — or ``"transit"`` — the earliest arrival with at least one
        vehicle, on the times the runs kept, walking to and from stops within
        ``transit_options``' ``access_walk_max_s``. Classes' own limits do not apply. The same
        point (one node) takes 0; ``nan`` where nothing arrives within ``max_s``.

        What accessibility measures are made of: the opportunities within 30 minutes by bike
        from each zone (cumulative), or weighed by a decaying function of time (gravity), by
        mode and by time of day.

        Args:
            mode: ``"car"``, ``"bike"``, ``"walk"`` or ``"transit"``.
            origins: The points to leave from: ``(lon, lat)`` pairs, or a dict of them by name
                (``demand_read_zones``' zones).
            destinations: The points to arrive at, likewise; ``None``: the origins.
            departure_s: When to leave.
            max_s: The longest trip looked for.

        Returns:
            A ``(len(origins), len(destinations))`` float64 array of seconds.

        Raises:
            ValueError: For another mode, a transit skim of a run without a timetable, or a
                departure or limit out of range.
        """
        import numpy as np

        def points(given: Any) -> tuple[list[float], list[float]]:
            values = list(given.values()) if isinstance(given, Mapping) else list(given)
            return [float(p[0]) for p in values], [float(p[1]) for p in values]

        (olon, olat) = points(origins)
        (dlon, dlat) = points(origins if destinations is None else destinations)
        return np.asarray(
            self._summary.skim(
                self._network, mode, olon, olat, dlon, dlat, float(departure_s), float(max_s)
            )
        )

    def report_spillback(self, min_excess_pcu: float = 5.0) -> dict[str, Any] | None:
        """The links where a queue outgrew the road in the last loading (S229).

        Under the point queue (``flow_level=2``, the default) a queue takes no road space; a link
        whose peak occupancy (the most PCU on it at once) exceeded its storage (jam density x
        length x lanes) by more than ``min_excess_pcu`` is where the real queue would have backed
        up into the links upstream, so the run's delays around it are optimistic. The minimum
        (uncalibrated: five cars) leaves out the very short links of an OpenStreetMap network,
        where a queue of a few cars overflows a few metres of road without consequence.

        Args:
            min_excess_pcu: How many PCU past its storage a link's peak must be to be listed.

        Returns:
            NumPy arrays, worst first (by the PCU that did not fit): ``link`` (internal link id),
            ``peak_pcu`` and ``storage_pcu``; ``None`` at level 0. The full model (level 4)
            keeps queues within storage, so its list is about empty. ``viz.map_interactive``
            can show them.
        """
        s = self._summary.spillback
        if s is None:
            return None
        keep = (s["peak_pcu"] - s["storage_pcu"]) > min_excess_pcu
        return {name: values[keep] for name, values in s.items()}

    def report_gridlock(self) -> dict[str, Any] | None:
        """What stood still when the last loading's window ended; ``None`` at level 0.

        ``vehicles_on_network`` (still on a link), ``vehicles_outside`` (still waiting to
        enter, at an origin or a stop line), ``links_waiting`` (links whose front vehicle was
        waiting), ``loops`` (closed loops of links whose front vehicles wait on one another —
        gridlock — each a list of link indices in waiting order), ``loop_count``,
        ``loop_links``, and ``room_waits_with_room``: front vehicles waiting for room on their
        next link while it had room. The loading lets traffic stop only at jam density, so
        that is always 0; anything else is a defect to report. A run that ends with loops
        also records a ``gridlock`` diagnostic per loop.
        """
        report = self._summary.gridlock
        return None if report is None else dict(report)

    def route_changes(self) -> dict[str, Any]:
        """Every reroute of the last loading, as columns (empty without rerouting).

        A vehicle blocked at the front of its link for ``reroute_after_s`` re-routes from where
        it is (``loading_options``). One row per reroute, in the order they happened: ``trip``
        (its index in the run), ``traveller_id``, ``trip_seq``, ``second``, ``link`` (the link
        at whose end it re-routed), ``next_planned`` (the next link of its route until then),
        ``next_taken`` (of its new route) and ``reason`` (``"stuck"``: blocked for
        ``reroute_after_s``; other reasons, such as alerts and disruptions, are to come). Its
        planned route is its route choice
        (``route_choices``); what it actually took is in ``route_realised``.
        """
        return dict(self._summary.route_changes)

    def route_realised(self) -> dict[str, Any]:
        """The realised routes of the trips that re-routed in the last loading and arrived.

        ``trip`` (their indices) and ``links`` (for each, the link indices it took, in order).
        Every other trip followed its planned route, which nothing stores twice.
        """
        return dict(self._summary.route_realised)

    def trip_modes(self) -> dict[str, Any]:
        """Every trip's departure and the mode it took, as columns.

        One row per trip, by traveller and then by the order of their day: ``traveller_id``,
        ``trip_seq`` (its place in that traveller's day, from 0), ``departure_s``, ``mode``
        (the stated mode, a trip without one counting as a car trip; with ``modes``, the
        mode taken, ``None`` for a trip that had nothing to choose from), ``mode_choice``
        (whether the trip chose its mode) and ``weight`` (the people its traveller stands
        for). A trip that could not travel keeps its mode. ``viz.chart_mode_share`` draws it.
        """
        return dict(self._summary.trip_modes)

    @property
    def total_travel_time_s(self) -> float:
        """Total travel time of the completed trips, in seconds, scaled by traveller weight.

        Each trip counts as many times as the people it stands for (its
        traveller weight): the population's total, not the sum over simulated
        travellers.
        """
        return self._summary.total_travel_time_s

    @property
    def mean_travel_time_s(self) -> float:
        """The mean travel time of the completed trips, in seconds, per person (S232).

        ``total_travel_time_s`` over the people the completed trips stand for (their travellers'
        weights). Dividing by ``completion["completed"]`` instead counts simulated trips, which
        differs from people when travellers stand for more than one (``default_weight``,
        ``demand_sample``). ``nan`` if no trip completed.
        """
        people = sum(row["completed_people"] for row in self.completion_by_mode.values())
        return self.total_travel_time_s / people if people > 0 else float("nan")

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
#: The modes a trip may have or choose (the ``mode`` column of ``trips.parquet``).
MODES = ("car", "bike", "walk", "transit", "car_transit", "bike_transit")


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


def _link_values(
    network: _core.Network, given: dict[str, dict[str, Any]]
) -> dict[str, dict[str, list[float]]]:
    """The user's link values (S236) as the core takes them: per layer and name, a value per link.

    Each column is an array in the layer's link order, or a dict from link id to value (a link
    not in it taking 0). Its length, the layer and the ids are checked here; the names and the
    values by the core.
    """
    if not isinstance(given, dict):
        raise ValueError("link_values is {layer: {name: values}}")
    out: dict[str, dict[str, list[float]]] = {}
    for layer, columns in given.items():
        if layer not in ("road", "bike", "walk"):
            raise ValueError(
                f"link_values has no layer {layer!r}; the layers are road, bike and walk"
            )
        graph = network if layer == "road" else network.layer(layer)
        n = graph.link_count
        ids: dict[str, int] | None = None
        out[layer] = {}
        for name, values in columns.items():
            if isinstance(values, dict):
                if ids is None:
                    ids = {link: i for i, link in enumerate(graph.link_ids())}
                column = [0.0] * n
                for link, value in values.items():
                    if str(link) not in ids:
                        raise ValueError(
                            f"link_values {layer!r} {name!r}: the {layer} layer has no link "
                            f"{link!r} (see network.layer({layer!r}).link_ids())"
                        )
                    column[ids[str(link)]] = float(value)
            else:
                column = [float(v) for v in values]
                if len(column) != n:
                    raise ValueError(
                        f"link_values {layer!r} {name!r} has {len(column)} values; the {layer} "
                        f"layer has {n} links"
                    )
            out[layer][str(name)] = column
    return out


def _person_values(given: Mapping[str, Any]) -> dict[str, list[tuple[str, float]]]:
    """The user's traveller values (S249) as the core takes them: per name, ``(id, value)`` pairs.

    Each column maps a traveller id to a value: a dict, or anything with ``items()`` (a pandas
    Series indexed by id). A traveller not in it takes 0; an id the demand does not have is
    ignored (a sampled demand keeps a share of the people). The names and values are checked by
    the core.
    """
    if not isinstance(given, Mapping):
        raise ValueError("person_values is {name: {traveller_id: value}}")
    out: dict[str, list[tuple[str, float]]] = {}
    for name, values in given.items():
        if not hasattr(values, "items"):
            raise ValueError(
                f"person_values {name!r}: a mapping from traveller id to value (a dict or a "
                "Series indexed by id)"
            )
        out[str(name)] = [(str(k), float(v)) for k, v in values.items()]
    return out


def _trip_values(
    given: Mapping[str, Any],
) -> tuple[dict[str, list[float]], dict[str, list[tuple[str, int, float]]]]:
    """The user's trip values (S249) as the core takes them: ``(aligned, keyed)``.

    A column is either a sequence with one value per row of the demand, in its order (a list, an
    array, a Series), or a dict from ``(traveller_id, trip_seq)`` to value, a trip not in it
    taking 0 and a key the demand does not have ignored. The lengths are checked by the core,
    which has the rows.
    """
    if not isinstance(given, Mapping):
        raise ValueError("trip_values is {name: values}")
    aligned: dict[str, list[float]] = {}
    keyed: dict[str, list[tuple[str, int, float]]] = {}
    for name, values in given.items():
        if isinstance(values, dict):
            column = []
            for key, value in values.items():
                if not (isinstance(key, tuple) and len(key) == 2):
                    raise ValueError(
                        f"trip_values {name!r}: a dict's keys are (traveller_id, trip_seq), "
                        f"got {key!r}"
                    )
                column.append((str(key[0]), int(key[1]), float(value)))
            keyed[str(name)] = column
        else:
            aligned[str(name)] = [float(v) for v in values]
    return aligned, keyed


def _disruptions(
    network: _core.Network, transit: _core.Transit | None, given: list[dict[str, Any]]
) -> tuple[list[tuple], list[tuple]]:
    """The disruptions (S239) as the core takes them: links and lines by index."""
    from openmobisim.transit import _line_index

    road: list[tuple] = []
    lines_of: list[tuple] = []
    ids: dict[str, int] | None = None
    lines = transit.lines() if transit is not None else None
    for k, d in enumerate(given):
        if not isinstance(d, dict):
            raise ValueError(f"disruption {k}: a dict, as Scenario's disruptions says")
        known = {"links", "capacity_factor", "line", "delay_s", "cancel", "from_s", "to_s"}
        unknown = set(d) - known
        if unknown:
            raise ValueError(
                f"disruption {k}: no such key {sorted(unknown)}; the keys: {sorted(known)}"
            )
        if "from_s" not in d or "to_s" not in d:
            raise ValueError(f"disruption {k}: from_s and to_s (seconds after midnight) are needed")
        start, end = float(d["from_s"]), float(d["to_s"])
        if "links" in d:
            if "line" in d or "capacity_factor" not in d:
                raise ValueError(f"disruption {k}: a road one has links and a capacity_factor")
            out = []
            for link in d["links"]:
                if isinstance(link, int):
                    out.append(link)
                    continue
                if ids is None:
                    ids = {name: i for i, name in enumerate(network.link_ids())}
                if str(link) not in ids:
                    raise ValueError(f"disruption {k}: the network has no link {link!r}")
                out.append(ids[str(link)])
            road.append((out, float(d["capacity_factor"]), start, end))
        elif "line" in d:
            if lines is None:
                raise ValueError(f"disruption {k}: a line's disruption needs transit=")
            cancel = bool(d.get("cancel", False))
            if cancel == ("delay_s" in d):
                raise ValueError(f"disruption {k}: a line's is either delay_s or cancel=True")
            delay = -1 if cancel else int(round(float(d["delay_s"])))
            if not cancel and delay < 0:
                raise ValueError(f"disruption {k}: delay_s is 0 or more")
            if start < 0:
                raise ValueError(f"disruption {k}: from_s is a second of the day")
            lines_of.append((_line_index(lines, str(d["line"])), delay, round(start), round(end)))
        else:
            raise ValueError(f"disruption {k}: give links (a road) or line (transit)")
    return road, lines_of


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
        classes: dict[str, Any] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
        flow_level: int = 2,
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
        modes: tuple[str, ...] | list[str] | None = None,
        mode_options: dict[str, float] | None = None,
        loading_options: dict[str, float] | None = None,
        price_options: dict[str, float] | None = None,
        link_values: dict[str, dict[str, Any]] | None = None,
        person_values: dict[str, Any] | None = None,
        trip_values: dict[str, Any] | None = None,
        disruptions: list[dict[str, Any]] | None = None,
        disruptions_known: bool = False,
        class_defaults: None = None,
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
        if class_defaults is not None:
            raise ValueError(
                "class_defaults is now classes (S231): the same {class: (owns_car, owns_bike, "
                "has_transit_pass)} works there, and a class may also say its modes and its "
                "coefficients"
            )
        # Traveller classes (S231): what each owns, the modes it may use, its coefficients.
        table = _class_table(classes) if classes is not None else {}
        self._class_defaults = {
            name: (c["owns_car"], c["owns_bike"], c["has_transit_pass"])
            for name, c in table.items()
        } or None
        self._class_modes = {
            name: c["modes"] for name, c in table.items() if c["modes"] is not None
        } or None
        self._class_options = {name: c["betas"] for name, c in table.items() if c["betas"]} or None
        self._class_limits = {name: c["limits"] for name, c in table.items() if c["limits"]} or None
        if modes is None and self._class_modes:
            # The classes say which modes their travellers may use: the run offers them all.
            modes = [m for m in MODES if any(m in ms for ms in self._class_modes.values())]
        self._default_weight = default_weight
        self._window_s = round(window_hours * 3600)
        self._flow_level = flow_level
        self._flow_step_s = flow_step_s
        self._link_bin_s = link_bin_s
        options = dict(equilibration_options or {})
        iterates = equilibration != "free_flow" and int(options.get("iterations", 10)) > 1
        if equilibration == "msa":
            # Stop when converged (S229), not always after `iterations`.
            options.setdefault("gap_tolerance", GAP_TOLERANCE)
            options.setdefault("route_growth_tolerance", ROUTE_GROWTH_TOLERANCE)
        if equilibration in ("free_flow", "msa"):
            # The free-flow loading (S223): built up in groups, each choosing on the congestion
            # of those before it, with the point-queue model, which cannot gridlock. At free
            # flow nothing congests, so there are no groups.
            if flow_level >= 2:
                options.setdefault("increments", FREE_FLOW_INCREMENTS)
            if flow_level >= 2 or iterates:
                options.setdefault("warmup", 1)
        # A run that loads more than once: iterations, or the free-flow loading's groups.
        reloads = iterates or (flow_level >= 2 and int(options.get("increments", 1)) > 1)
        if route_method is None:
            # Such a run starts from one route per pair and lets the route update find the
            # rest (S179, S223); a run that loads once has no later loading to choose among new
            # routes. Options given for a method mean the default method of a single loading.
            route_method = "shortest" if reloads and route_options is None else "penalty"
        if route_update is None:
            route_update = "best_response" if reloads or route_update_options else "none"
        equilibration_options = options or None
        self._route_method = route_method
        self._route_options = route_options
        self._master_seed = master_seed
        self._choice_model = choice_model
        self._choice_options = choice_options
        if choice_options and self._class_options:
            # A class's coefficients replace the run's by name: say so, or a study varying a
            # run-wide one that every class sets would see no effect and not know why.
            for name in sorted(choice_options):
                classes = sorted(c for c, betas in self._class_options.items() if name in betas)
                if classes:
                    whom = "every class" if len(classes) == len(table) else ", ".join(classes)
                    warnings.warn(
                        f"choice_options' {name} is replaced by the class's own for {whom}: a "
                        "class's coefficients win by name, so the run-wide value applies to the "
                        "other classes only.",
                        stacklevel=2,
                    )
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
        if modes is not None:
            modes = list(modes)
            unknown = [m for m in modes if m not in MODES]
            if unknown:
                raise ValueError(f"modes must be among {MODES}, got {unknown}")
        self._modes = modes
        self._mode_options = mode_options
        loading = dict(loading_options or {})
        if flow_level <= 2:
            # In a point queue nothing waits for room, so nothing is stuck to re-route (S229);
            # rerouting stays a feature of the levels with spillback.
            loading.setdefault("reroute", 0)
        self._loading_options = loading or None
        self._price_options = price_options
        self._link_values = _link_values(network, link_values) if link_values else None
        self._person_values = _person_values(person_values) if person_values else None
        self._trip_values, self._trip_values_keyed = (
            _trip_values(trip_values) if trip_values else (None, None)
        )
        self._road_disruptions, self._transit_disruptions = _disruptions(
            network, transit, disruptions or []
        )
        self._disruptions_known = bool(disruptions_known)

    @classmethod
    def from_parts(
        cls,
        network: _core.Network,
        demand: list[tuple] | str | Path,
        persons: list[tuple] | str | None = None,
        classes: dict[str, Any] | None = None,
        default_weight: int = 1,
        window_hours: float = 24.0,
        flow_level: int = 2,
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
        modes: tuple[str, ...] | list[str] | None = None,
        mode_options: dict[str, float] | None = None,
        loading_options: dict[str, float] | None = None,
        price_options: dict[str, float] | None = None,
        link_values: dict[str, dict[str, Any]] | None = None,
        person_values: dict[str, Any] | None = None,
        trip_values: dict[str, Any] | None = None,
        disruptions: list[dict[str, Any]] | None = None,
        disruptions_known: bool = False,
        class_defaults: None = None,
    ) -> Scenario:
        """Build a scenario from a network and demand.

        Args:
            network: Built by, for example, ``examples.manhattan_grid``.
            demand: Either an in-memory trips table (rows in the schema
                ``examples.fixed_car_trips`` returns, or hand-built the same
                way) or a path to a ``trips.parquet`` file or a ``trips.csv``
                (read by ``demand_read_trips``) — the same schema either way,
                never two different shapes. A row may end with
                a **mode** (the ``mode`` column of ``trips.parquet``): ``"car"``,
                ``"bike"``, ``"walk"``, ``"transit"``, ``"car_transit"`` (park-and-ride)
                or ``"bike_transit"`` (bike-and-ride; see ``parkings``). A trip without
                one is a car trip, or, with ``modes``, chooses one. Bike and walk trips
                travel on the network's bike and walk layers (``network.layer("bike")``),
                each by its shortest route
                there; a bike trip needs the traveller's bike at its origin, as a
                car trip needs their car. A transit trip needs ``transit``: it
                walks to a stop, rides and walks on (see ``transit``).
            persons: The ``persons.parquet`` equivalent — an in-memory table,
                a file path, or ``None`` if every traveller takes their
                class's default ownership.
            classes: The traveller classes (S231), ``{class_name: columns}``, a trip's class being
                its ``user_class``: ``demand_read_classes`` reads them from a CSV file, or give a
                dict of the same columns. A class may say ``modes`` (the modes its travellers may
                use: ``["car", "transit", "car_transit"]``; unsaid: every mode the run offers),
                ``owns_car``, ``owns_bike``, ``has_transit_pass`` (what each of them owns, unless a
                ``persons`` row says otherwise; unsaid: as its modes imply, a car for a class that
                may drive or park and ride, a bike for one that may cycle, all three for one whose
                modes are unsaid) and any ``beta_*`` coefficient of the choice model (its own mode
                constants, ``beta_mode_bike`` …, or its own value of time, ``beta_time_min``;
                unsaid: ``choice_options``'), for ``"logit"`` and ``"nested_logit"``, and its own
                choice-set limits in seconds (S235): ``walk_max_s`` and ``bike_max_s``, the longest
                walk and ride offered to its trips choosing their mode, and ``access_walk_max_s``,
                the longest walk to or from a stop (unsaid: ``mode_options``' and
                ``transit_options``'; for example a class of keen cyclists riding up to 90 minutes,
                ``{"bike_max_s": 5400}``); its ``share`` is read only by ``demand_assign_classes``.
                The tuple ``(owns_car, owns_bike, has_transit_pass)`` is a class that says only what
                it owns. **When the classes say their modes and ``modes`` is not given, the run
                offers every mode a class names**, and a trip without a stated mode chooses among
                those its class may use. A class not listed (all of them, without ``classes``) may
                use every mode, with the model's coefficients, and owns a car, a bike and a transit
                pass unless a ``persons`` row says otherwise (S243). A class with none of the run's
                modes leaves its choosing trips without an itinerary. Every value is the user's, not
                a calibration.
            default_weight: How many people a simulated traveller stands for,
                for a trip whose row gives no weight. 1 simulates everyone; a
                larger number is faster and coarser.
            window_hours: Trips still in progress after this many hours are
                truncated.
            flow_level: How vehicles load the network. ``2`` (the default, S229) is the
                **point queue**: the link transmission model with capacities at every link
                and junction, where a queue forms behind capacity and delays everyone in it,
                but takes no road space, so it never blocks the links upstream and nothing
                gridlocks. Equilibrium iterations converge steadily with it, and it is the
                cheapest loading with congestion: the right model for planning studies
                (Vickrey's bottleneck queue with a first-order node model, as in quasi-dynamic
                assignment). ``Run.report_spillback()`` lists the links whose queue would not
                have fitted on the road. ``4`` is the full triangular fundamental diagram:
                queues take road space and **spill back** to the links upstream, with turn
                pockets and en-route rerouting against gridlock — for studies where spillback
                itself matters (traffic control, incidents, evacuation); under heavy congestion
                its iterations can drift (S227). ``3`` is a spatial queue. ``0`` is free flow:
                no vehicle affects another, so there is no congestion, and a run that iterates
                has nothing to settle — for debugging, or as a lower bound. Under
                ``equilibration="free_flow"`` (the free-flow loading alone, no iterations) every
                level from 2 loads as the point queue (``equilibration_options`` ``warmup``, 1 by
                default), so levels 2, 3 and 4 differ only in whether stuck vehicles may re-route
                (``loading_options`` ``reroute``: off at 2, on at 3 and 4); the level sets how
                ``"msa"``'s iterations after it load (X-25).
            flow_step_s: The loading step in seconds, for levels 2-4. It is a
                bookkeeping boundary: results do not depend on it.
            link_bin_s: If given, also record per-link results in time bins of
                this many seconds, read with ``Run.link_bins()`` and drawn
                with ``openmobisim.viz.map_link``. It changes no result: the link times
                travellers choose on are binned at the equilibration's ``cost_bin_s``.
            route_method: How route sets are generated, by name (see
                ``openmobisim.route_methods()``). **The default depends on the run:** a run
                that loads once uses ``"penalty"``; a run that **loads more than once** (an
                ``equilibration`` with more than one iteration, or a free-flow loading built up
                in groups, the default at ``flow_level`` 2 to 4) starts from ``"shortest"``, one
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
                traveller; ``"nested_logit"`` is the logit with the alternatives of each
                mode in a nest of their own, for mode choice (``modes``): a mode's many
                routes do not outweigh another mode's one; ``"deterministic"`` sends
                everyone down the best route, with probability 1 — useful for debugging or
                an upper bound, not for a result to report.
            choice_options: A built-in model's options, numbers by name: for
                ``"logit"`` and ``"nested_logit"`` a coefficient per attribute,
                ``beta_time_min`` (default -0.2), ``beta_ln_path_size`` (1),
                ``beta_walk_min`` (-0.13) and ``beta_wait_min`` (-0.09, on top of the
                time), ``beta_transfers`` (-1), and 0 for the rest, such as
                ``beta_length_km``, ``beta_parking_min``, ``beta_cost_eur`` (money, see
                ``price_options``) or the mode constants
                ``beta_mode_bike`` … (``openmobisim.choice.ROUTE_ATTRIBUTES`` lists them), and
                products of two attributes, ``beta_<a>*<b>`` (see ``trip_values``);
                for ``"nested_logit"`` also ``mu`` (0.5), the nests' scale, from above 0
                to 1 (1 is the logit). Unknown names and non-numbers are refused. The
                defaults are an assumption, not a calibration.
            choice_detour_limit: A **time-dependent choice set**: a traveller is offered only the
                routes of the pair's set whose expected time, at the times of the last loading
                (free flow for the first choice), is within this share of the best route's:
                ``0.5`` offers a route only if it is at most 50% slower than the best right now.
                A route far slower than the best is no realistic alternative, and in a logit it
                only takes probability that belongs to routes that compete (the gap grows with the
                size of the set). ``0`` offers every route of the set; ``None`` (the default) is the
                library's default, **0.5**. With mode choice it applies within each mode: a walk
                slower than a drive is still offered. The best route is always offered; a route
                left out is still in the set and returns when times change. The gaps are still
                measured against the whole set. The ``ln_path_size`` attribute is computed over
                the whole set.
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
                ``"free_flow"`` makes only the free-flow loading: a quick
                estimate of the day without iterating.
                **The free-flow loading** is how every run starts (iteration 0,
                S223). The travellers are split into groups of about equal size
                (``increments``, 5 by default); the first group chooses on
                free-flow costs, then the network is loaded with the groups so
                far and the next group chooses on the times that produced (its
                pairs searched for routes on those times first), until everyone
                has chosen and the whole demand is loaded. Each group sees the
                congestion of those before it, so routes spread before the
                first full loading (incremental assignment). It uses the
                point-queue model (``warmup``): queues delay traffic but take
                no space, so nothing spills back and nothing locks; the
                iterations that follow use ``flow_level``. At ``flow_level=0``
                there are no groups, since nothing congests.
                Either way, equilibration only changes anything when vehicles
                interact (``flow_level`` 2 to 4, the default 2) and a choice
                model gives travellers something to choose between
                (``"logit"``): at ``flow_level=0`` (free flow, no interaction),
                ``"msa"`` still runs but has nothing to disagree with itself
                about, so it settles immediately at the choice model's own
                floor. ``Run.convergence()`` says how
                the iterations went, and ``Run.convergence_verdict`` judges
                the disequilibrium the last iteration ended at.
            equilibration_options: The strategy's options, numbers by name: for
                ``"msa"``, ``iterations`` (10; the most loadings), ``gap_tolerance``
                (0.02, S229: stop once the disequilibrium, averaged over the last three
                iterations, is below this; 0.05 is good, 0.15 acceptable; 0 never stops
                early), ``route_growth_tolerance`` (1: and only while the last route update
                added routes for at most this share of the pairs it searched; 1 lets the gap
                alone decide, since it is measured against the sets as grown),
                ``gap_sample`` (300; how many trips are tested against the whole
                network at the last iteration, 0 for none), ``itinerary_gap_sample``
                (100 000 000, that is every trip: how many transit, park-and-ride and
                bike-and-ride trips, besides those choosing again, plan their whole choice set
                after each loading to measure their gap; with fewer, the others that keep an
                itinerary out through a parking are re-costed at that parking, which is faster
                but makes the gap before the last iteration read low; the last iteration always
                plans every trip) and ``cost_bin_s`` (300: the
                length of the time bins the link times are read in). For both ``"msa"`` and
                ``"free_flow"``: ``increments`` (5 at ``flow_level`` 2 to 4, else 1: in how many
                groups the free-flow loading is built up; 1 lets everyone choose at once on
                free-flow costs) and ``warmup`` (1 at ``flow_level`` 2 to 4 or for a run that
                iterates, else 0: how many of the first loadings, the free-flow loading's groups
                included, use the point-queue model, which cannot gridlock; the last loading of a
                run that iterates, its result, always uses ``flow_level``, while a run of one
                loading is its free-flow loading. ``"free_flow"`` with ``increments`` 1 and
                ``warmup`` 0 is a single loading at ``flow_level`` on free-flow choices).
            route_update: How the route sets grow between iterations (see
                ``openmobisim.route_update_methods()``). **The default depends on the run:**
                ``"best_response"`` when the run loads more than once (it iterates, or builds
                its free-flow loading in groups; or ``route_update_options`` is given),
                ``"none"`` when it loads once. ``"none"`` leaves
                the sets as the route method made them, at free-flow costs. ``"best_response"``
                is for a run that loads more than once: after each loading it searches, for every
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
                ``board_slack_s`` (60), ``max_rides`` (8), ``access_walk_max_s`` (1800),
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
                a parking full for a whole time bin adds), ``snap_m`` (300), ``bin_s``
                (900) and ``pr_min_km`` (3: the shortest trip, as the crow flies, that mode
                choice offers park-and-ride and bike-and-ride to). Uncalibrated defaults.
                Unknown names and out-of-range values are refused.
            modes: **Mode choice.** The modes a trip without a stated mode chooses among,
                with its route, by ``choice_model``: any of ``"walk"``, ``"bike"``,
                ``"car"``, ``"transit"``, ``"car_transit"`` and ``"bike_transit"``. Each is
                offered where the traveller can use it: their car or bike where the trip
                starts (a car left at a car park is fetched on the way back), the walk and
                bike layers, the timetable (``transit``) and the parkings (``parkings``);
                park-and-ride and bike-and-ride only for trips longer than
                ``parking_options["pr_min_km"]``. A traveller's trips choose in order, since
                each may move a vehicle the next needs; a trip with a stated mode keeps it.
                With ``choice_model="nested_logit"`` the alternatives of a mode share a nest
                (``choice_options["mu"]``), and ``beta_mode_<mode>`` is a mode's constant (0
                by default: time decides). One mode is no choice: such trips take it
                (``("car",)`` is the run without ``modes``, exactly). ``None`` (the default):
                such trips are car trips. A walk or a ride is offered only up to
                ``mode_options``' times (30 minutes' walk, 60 minutes' ride), or the
                trip's class's (``classes``).
            mode_options: Mode choice's parameters by name, each replacing its default:
                ``walk_max_s`` (1800) and ``bike_max_s`` (3600), the longest walk and ride
                offered to a trip choosing its mode (a trip given the mode takes it at any
                length; a class may give its own, ``classes``); ``float("inf")`` offers
                every walk or ride. Uncalibrated defaults.
                Unknown names and values that are not above 0 are refused.
            loading_options: The loading's rules by name, for ``flow_level`` 2–4 (gridlock
                remedies; at ``flow_level`` 2, where nothing waits for room, ``reroute`` is off
                unless asked for): ``priority`` (0 or 1, off by default: without per-turn queues a
                vehicle giving way holds up everything behind it, which made locks worse) —
                at an unsignalised merge a vehicle gives way to an approach of higher road
                class, or of the same class and at least 1.5 times the capacity, whose front
                vehicle is bound for the same link, and a departing vehicle gives way to every
                approach; ``reroute`` (0 or 1, **on** by default at levels 3 and 4) — a vehicle
                blocked at the front of its link for ``reroute_after_s`` (300) re-routes from
                where it is, at
                most ``reroute_max`` (3) times, if the new route is at least
                ``reroute_min_gain`` (0.1) faster; ``pocket_length_m`` (50, 0 turns it off) —
                on an approach of two lanes or more, a vehicle passes those ahead of it that
                wait for another movement while they fit in their turn pockets, this many metres
                per lane, split among the approach's movements.
                Uncalibrated defaults; unknown names and values out of range are refused.
            price_options: What travel costs, in euros, by name (S248): ``car_eur_km``
                (0.12, the fuel a driver pays per km), ``bike_eur_km`` (0), a transit
                journey's fare ``fare_base_eur`` (2.0, once) + ``fare_km_eur`` (0, per km from
                each boarding stop to its alighting stop as the crow flies) +
                ``fare_transfer_eur`` (0, per transfer), and the fee per stay of a car park or
                bike parking whose table gives none (``parking_car_eur``,
                ``parking_bike_eur``: 0; a parking table's ``fee_eur`` column gives each its
                own). A toll is a road link value named ``toll_eur`` (``link_values``), paid at
                each passage. Every alternative carries what it costs: ``cost_eur`` and its
                parts ``cost_running_eur``, ``cost_toll_eur``, ``cost_parking_eur`` (park-and-ride
                and bike-and-ride, on the trip that parks) and ``cost_fare_eur``. The built-in
                models weigh money by ``beta_cost_eur`` (or a part's coefficient), **0 unless
                given**, so no run changes until it is: a value of time of ``V`` euros per hour is
                ``beta_cost_eur = beta_time_min * 60 / V`` (-1.2 at the default -0.2 and 10 €/h);
                a class's own ``beta_cost_eur`` (``classes``) is its own value of time.
                Uncalibrated defaults (a flat fare of the order of a single urban ticket: Lyon
                2.10 €, Paris 2.55 €; Amsterdam charges 1.16 € plus 0.217 € per km); unknown
                names and values below 0 are refused. Recorded in the fingerprint and the
                manifest when the choice model reads money.
            link_values: The user's own numbers per link, for choice models (S236):
                ``{layer: {name: values}}``, the layer ``"road"``, ``"bike"`` or ``"walk"``,
                the values either one per link of that layer in link order (an array as long as
                ``network.layer("bike").link_count``, aligned with its other per-link arrays) or
                a dict from link id (``network.layer("bike").link_ids()``) to value, a link not
                in it taking 0. Names are lower-case letters, digits and ``_``. Each column
                becomes two attributes of every alternative with a leg on that layer:
                ``<layer>_<name>_km``, the sum of the value times each link's length in km
                (greenery along a route, divided by ``length_km`` for its mean), and
                ``<layer>_<name>_sum``, the plain sum (a toll, a count of crossings); 0 for an
                alternative with no leg there (a transit itinerary's walks to and from stops
                carry none). The built-in models weigh them through ``choice_options``
                (``{"beta_bike_greenery_km": 0.2}``), a model of one's own reads them from the
                batch. For example, the greenery around each bike link computed from
                OpenStreetMap, or the car flow on the road beside each bike link taken from a
                previous run's ``link_bins`` (``link_ids()`` ties a bike link to its street's
                road links).
                Recorded in the fingerprint, and their names in the manifest.
            person_values: The user's own numbers per traveller, for choice models (S249):
                ``{name: {traveller_id: value}}`` (a dict, or a pandas Series indexed by id); a
                traveller not in it takes 0, and an id the demand does not have is ignored (a
                sampled demand keeps a share of the people). Each becomes the attribute
                ``person_<name>`` of every alternative of that traveller's choices: income, age,
                a household's children.
            trip_values: The same per trip (S249): ``{name: values}``, the values one per row of
                ``demand`` in its order (a list, an array, a Series), or a dict from
                ``(traveller_id, trip_seq)`` to value (a trip not in it taking 0); the attribute
                ``trip_<name>``: a purpose, a time budget. **``parking_eur``** is what parking a car
                at the trip's destination costs, in euros: it is added to ``cost_parking_eur`` of
                every alternative that arrives by car (see ``price_options``). Beside them every
                alternative carries ``trip_departure_h``, the trip's departure in hours after
                midnight. Being the same for every alternative of a choice, such a value weighs in
                the built-in models **in a product with another attribute**, a coefficient named
                ``beta_<a>*<b>`` in ``choice_options`` or a class: ``beta_mode_car*person_age``, a
                car constant growing with age; ``beta_cost_eur*person_income_inv``, money weighing
                less as income grows (give ``income_inv`` as one over the income); a model of one's
                own reads them from the batch. Names are lower-case letters, digits and ``_``.
                Recorded in the fingerprint, and their names in the manifest.
            disruptions: Things that happen at a time of day (S239), a list of dicts: on roads,
                ``{"links": [...], "capacity_factor": 0.0, "from_s": 8 * 3600, "to_s": 9 * 3600}``
                — the links (by id, ``network.link_ids()``, or index; each direction of a road
                is a link) keep that share of their capacity from ``from_s`` to ``to_s``
                (seconds after midnight): 0 closes them, nothing entering or leaving, 0.5
                halves it; overlapping ones multiply; on lines, ``{"line": "T1", "delay_s": 600,
                "from_s": ..., "to_s": ...}`` or ``{"line": "T1", "cancel": True, ...}`` — the
                line's runs leaving their first stop in that window are late by ``delay_s`` at
                every call (a bus on the roads leaves late and drives on among the cars) or do
                not run. Road disruptions act through capacity, so not at ``flow_level`` 0.
            disruptions_known: Whether travellers know of the disruptions in advance. ``False``
                (the default: an accident, a breakdown): the run reaches its equilibrium without
                them, then loads the day once more with them, every choice kept — only cars
                stuck behind a closure re-route (at ``flow_level`` 3 and 4) and passengers whose
                run never comes plan again at the stop; that last loading is the run's result
                (the convergence report stays the undisrupted one's), so the impact is the
                **zero-adaptation** one. ``True`` (works announced, a strike): every loading has
                them and choices adapt over the iterations. Both are in the fingerprint and the
                manifest (``disruptions``).
            class_defaults: Replaced by ``classes`` (S231), which takes the same tuples;
                refused, with a pointer to it.

        Returns:
            A ``Scenario``, ready to ``.run()``.
        """
        if isinstance(demand, Path):
            demand = str(demand)
        if isinstance(demand, str) and demand.lower().endswith(".csv"):
            demand = demand_read_trips(demand)
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
            classes=classes,
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
            modes=modes,
            mode_options=mode_options,
            loading_options=loading_options,
            price_options=price_options,
            link_values=link_values,
            person_values=person_values,
            trip_values=trip_values,
            disruptions=disruptions,
            disruptions_known=disruptions_known,
            class_defaults=class_defaults,
        )

    def run(self, run_id: str = "run", output_dir: str | None = None, quiet: bool = False) -> Run:
        """Run the scenario and write its output artifacts.

        Args:
            run_id: Recorded in every output row; also names the default
                output directory.
            output_dir: Where ``kpis.parquet`` etc. are written. Defaults to
                a directory under the system temp directory, named after
                ``run_id``.
            quiet: If true, do not print the run's wall-clock time by stage when it ends
                (``Run.timings()`` and ``timings.csv`` have it either way).

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
            class_modes=self._class_modes,
            class_options=self._class_options,
            class_limits=self._class_limits,
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
            modes=self._modes,
            mode_options=self._mode_options,
            loading_options=self._loading_options,
            price_options=self._price_options,
            link_values=self._link_values,
            person_values=self._person_values,
            trip_values=self._trip_values or None,
            trip_values_keyed=self._trip_values_keyed or None,
            road_disruptions=self._road_disruptions,
            transit_disruptions=self._transit_disruptions,
            disruptions_known=self._disruptions_known,
        )
        run = Run(
            summary,
            network=self._network,
            run_id=run_id,
            flow_level=self._flow_level,
            flow_step_s=self._flow_step_s,
            window_s=self._window_s,
            transit=self._transit,
            parkings=self._parkings,
        )
        if not quiet:
            print(run._timings_text())
        return run
