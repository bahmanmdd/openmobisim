"""What ``Scenario.run()`` returns: ``Run``, and ``Run.open`` to read one back from its folder.

Split from ``scenario.py``; ``openmobisim.scenario`` re-exports every name.
"""

from __future__ import annotations

import csv
import json
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import numpy as np

from openmobisim import _core
from openmobisim.demand import demand_read_trips
from openmobisim.table import Columns, Table

__all__ = ["LAYERS", "MODES", "NO_ALTERNATIVE_CHANGES", "SERVICE_MODES", "Run"]

LAYERS = ("road", "bike", "walk")
#: The modes a trip may have or choose (the ``mode`` column of ``trips.parquet``).
MODES = ("car", "bike", "walk", "transit", "car_transit", "bike_transit")
#: The modes a service of the run offers (``bike_share``, and ``bike_share_transit`` with a
#: timetable: a docked bike service, ``service_bike_docked``): chosen, and listed in a run's
#: results, only in a run that has the service.
SERVICE_MODES = ("bike_share", "bike_share_transit")


#: What to change for each reason a mode gave a trip no alternative, as
#: ``Run.report_no_alternative()``'s ``change`` column and the warning name them.
NO_ALTERNATIVE_CHANGES = {
    "walk_too_long": "mode_options['walk_max_s'] or the class's walk_max_s",
    "ride_too_long": "mode_options['bike_max_s'] or the class's bike_max_s",
    "no_vehicle": "the class's owns_car or owns_bike, or its modes",
    "no_road_route": "the road network between the two (network_check)",
    "no_stop_in_reach": "transit_options['access_walk_max_s'] or the class's access_walk_max_s",
    "no_transit_journey": "transit_options['journey_max_s'], the window, or the timetable",
    "too_short": "parking_options['pr_min_km']",
    "no_dock_in_reach": "the service's walk_max_s, or its docks",
    "unexplained": "nothing: a defect, please report it",
}


def _clock(seconds: float) -> str:
    """Seconds after midnight as a clock time, ``07:30`` (``24:30`` past midnight, in order)."""
    s = round(seconds)
    return f"{s // 3600:02d}:{s % 3600 // 60:02d}" + (f":{s % 60:02d}" if s % 60 else "")


#: What ``Run.timings()`` calls each stage, as the printout shows it.
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


def _check_layer(layer: str) -> str:
    if layer not in LAYERS:
        raise ValueError(f"layer must be one of {LAYERS}, got {layer!r}")
    return layer


#: The per-iteration measures ``Run.open`` reads back from ``kpis.parquet`` (mode ``all``).
_FILE_CONVERGENCE = (
    "reselected_share", "changed_share", "total_travel_time_s", "completed_trips",
    "completed_people", "truncated_trips", "time_change", "gap", "gap_network", "gap_expected",
    "gap_excess", "gap_network_excess", "incomplete_share", "gap_flow", "gap_flow_floor",
    "gap_flow_excess", "mode_changed_share", "itinerary_gap_excess", "gap_generalised_excess",
    "gap_route", "gap_route_expected", "gap_route_excess", "gap_route_network",
    "gap_route_network_excess", "gap_mode", "gap_mode_expected", "gap_mode_excess",
    "excess_min_route", "excess_min_mode", "unfinished_cost_share",
)  # fmt: skip
#: ``completion``'s keys and the ``kpis.parquet`` metrics they are.
_FILE_COMPLETION = {
    "total_trips": "trips", "completed": "completed_trips", "truncated": "truncated_trips",
    "no_vehicle_available": "no_vehicle_available_trips",
    "no_feasible_path": "no_feasible_path_trips", "mode_not_available": "mode_not_available_trips",
}  # fmt: skip


def _run_ids(path: str | None) -> set[str]:
    """The run ids a ``kpis.parquet`` (or ``kpis.csv``) holds: one, for a run's own file."""
    if path is None:
        return set()
    if path.endswith(".csv"):
        with open(path, newline="") as f:
            return {row["run_id"] for row in csv.DictReader(f)}
    import pyarrow.parquet as pq

    return set(pq.read_table(path, columns=["run_id"]).column("run_id").to_pylist())


class _Files:
    """A finished run's folder in place of the core's summary (``Run.open``, C2).

    It gives what the files hold; anything else raises, naming what is missing.
    """

    def __init__(self, folder: Path) -> None:
        manifest_path = folder / "manifest.json"
        if not manifest_path.exists():
            raise FileNotFoundError(f"{str(folder)!r} holds no run: no manifest.json")
        self._folder = folder
        m = json.loads(manifest_path.read_text())
        self._manifest = m
        self.manifest_path = str(manifest_path)
        for name in (
            "kpis",
            "diagnostics",
            "events",
            "link_bins",
            "parking_bins",
            "transit_calls",
            "trip_results",
        ):
            path = folder / f"{name}.parquet"
            setattr(self, f"{name}_path", str(path) if path.exists() else None)
        if self.kpis_path is None and (folder / "kpis.csv").exists():  # the minimal outputs
            self.kpis_path = str(folder / "kpis.csv")
        self.outputs = m.get("outputs", "all")
        for layer in ("bike", "walk"):
            path = folder / f"link_bins_{layer}.parquet"
            setattr(self, f"link_bins_{layer}_path", str(path) if path.exists() else None)
        timings = folder / "timings.csv"
        self.timings_path = str(timings) if timings.exists() else None
        self.fingerprint = m["run_fingerprint"]
        self.master_seed = int(m["master_seed"])
        self.choice_model = m["choice_model"]
        self.equilibration = m["equilibration"]
        self.route_update = m.get("route_update") or "none"
        self.converged = bool(m.get("converged", False))

    def _kpis(self) -> list[tuple[int, str, str, float]]:
        if self.kpis_path is not None and self.kpis_path.endswith(".csv"):
            with open(self.kpis_path, newline="") as f:
                rows = list(csv.DictReader(f))
            return [(int(r["iteration"]), r["mode"], r["metric"], float(r["value"])) for r in rows]
        import pyarrow.parquet as pq

        t = pq.read_table(self.kpis_path, columns=["iteration", "mode", "metric", "value"])
        columns = [t.column(c).to_pylist() for c in ("iteration", "mode", "metric", "value")]
        return list(zip(*columns, strict=True))

    @property
    def trip_results(self) -> dict[str, Any]:
        """``trip_results.parquet``'s columns, as the run in memory gives them (null: ``NaN``)."""
        if self.trip_results_path is None:
            raise RuntimeError(
                "this run's folder holds no trip_results.parquet (a run with outputs='minimal' "
                "writes none); run the scenario again with outputs='all' for it"
            )
        import pyarrow.parquet as pq

        table = pq.read_table(self.trip_results_path)
        text = {"traveller_id", "user_class", "mode", "outcome", "mode_choice"}
        maybe = {"arrival_s", "travel_time_s", "link_at_end", "vehicle_s", "access_egress_s"}
        out: dict[str, Any] = {}
        for name in table.column_names:
            column = table.column(name)
            if name == "run_id":
                continue
            if name in text:
                out[name] = column.to_pylist()
            elif name in maybe:
                out[name] = column.to_numpy(zero_copy_only=False).astype(float)  # null: NaN
            else:
                out[name] = column.to_numpy()
        return out

    @property
    def trip_modes(self) -> dict[str, Any]:
        """``trip_modes()``'s columns, from ``trip_results.parquet``."""
        t = self.trip_results
        keys = ("traveller_id", "trip_seq", "departure_s", "mode", "mode_choice", "weight")
        return {k: t[k] for k in keys}

    def __getattr__(self, name: str) -> Any:
        raise RuntimeError(
            f"this run was opened from its folder (Run.open), whose files do not hold {name!r}; "
            "run the scenario again for it"
        )


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
        transit: _core.Transit | None = None,
        parkings: _core.Parkings | None = None,
        car_snap_motorway_m: float | None = None,
    ) -> None:
        """Wrap a `RunSummary` from `run_pipeline`, with what it was run on."""
        self._summary = summary
        self._network = network
        self._transit = transit
        self._parkings = parkings
        # Where the run's trips joined the roads (``mode_options``; None: the default), so a
        # report snapping points to nodes agrees with the run (U-41).
        self._car_snap_motorway_m = car_snap_motorway_m
        self.run_id = run_id
        self.flow_level = flow_level
        self.flow_step_s = flow_step_s
        self._files: _Files | None = None
        # Which files the run wrote (C2): "all", or "minimal" (kpis.csv, manifest.json).
        self._outputs = "all"

    def _written(self, path: str | None, what: str) -> str | None:
        """``path``, or a clear refusal for a file the run's minimal outputs leave out."""
        if self._outputs == "minimal":
            raise RuntimeError(
                f"this run wrote its minimal outputs (outputs='minimal': kpis.csv and "
                f"manifest.json), so not {what}; run with outputs='all' for it"
            )
        return path

    @classmethod
    def open(cls, folder: str | Path) -> Run:
        """A finished run, read back from the folder it wrote, without running it again.

        It gives what the files hold: ``fingerprint``, ``master_seed``, ``choice_model``,
        ``equilibration``, ``route_update``, ``converged`` and ``manifest()`` (from
        ``manifest.json``); ``completion``, ``completion_by_mode``, ``total_travel_time_s``,
        ``mean_travel_time_s``, ``convergence()``, ``convergence_gap`` and
        ``convergence_verdict`` (from ``kpis.parquet``); ``timings()``; ``trip_results()`` and
        ``trip_modes()`` (from ``trip_results.parquet``); and every table (``kpis()``,
        ``diagnostics()``, ``events()``, ``link_bins_table()``, ``parking_bins_table()``,
        ``transit_calls_table()``). What only the run in memory holds
        (the network, the route sets and choices, the per-link arrays, the itinerary choices)
        raises a ``RuntimeError`` that says so. Needs pyarrow.

        Args:
            folder: The run's ``output_dir``, holding its ``manifest.json``.

        Raises:
            FileNotFoundError: If the folder holds no ``manifest.json``.
        """
        files = _Files(Path(folder))
        m = files._manifest
        run = cls(
            files,  # in place of the core's summary
            run_id=Path(folder).name,
            flow_level=int(m.get("flow_level", 0)),
            flow_step_s=int(m.get("flow_step_seconds", 300)),
        )
        run._files = files
        run._outputs = files.outputs
        kpis = files._kpis()
        run_ids = {r for r in _run_ids(files.kpis_path)}
        run.run_id = run_ids.pop() if len(run_ids) == 1 else run.run_id
        last = max((i for i, mode, _, _ in kpis if mode == "all"), default=0)
        totals = {name: v for i, mode, name, v in kpis if mode == "all" and i == last}
        files.total_trips, files.total_travel_time_s = int(totals.get("trips", 0)), 0.0
        for key, metric in _FILE_COMPLETION.items():
            setattr(files, key, int(totals.get(metric, 0)))
        files.total_travel_time_s = float(totals.get("total_travel_time_s", 0.0))
        by_mode: dict[str, dict[str, float]] = {}
        for i, mode, name, v in kpis:
            if mode != "all" and i == last:
                by_mode.setdefault(mode, {})[name] = v
        files.completion_by_mode = {
            mode: {
                **{key: int(row.get(metric, 0)) for key, metric in _FILE_COMPLETION.items()},
                "total_travel_time_s": float(row.get("total_travel_time_s", 0.0)),
                "people": float(row.get("people", 0.0)),
                "completed_people": float(row.get("completed_people", 0.0)),
            }
            for mode, row in sorted(by_mode.items())
            if "trips" in row
        }
        iterations = sorted({i for i, mode, _, _ in kpis if mode == "all"})
        values = {(i, name): v for i, mode, name, v in kpis if mode == "all"}
        nan = float("nan")
        files.convergence = {
            "iteration": np.array(iterations, dtype=np.uint32),
            **{
                name: np.array([values.get((i, name), nan) for i in iterations], dtype=float)
                for name in _FILE_CONVERGENCE
            },
        }
        files.convergence["completed"] = files.convergence.pop("completed_trips")
        files.convergence["truncated"] = files.convergence.pop("truncated_trips")
        if files.timings_path is not None:
            with open(files.timings_path, newline="") as f:
                rows = list(csv.reader(f))[1:]
            files.timings = [
                (stage, int(it) if it else None, float(sec)) for stage, it, sec in rows
            ]
        return run

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
        model (the defaults, ``"nested_logit"`` and ``"logit"``) it decides who takes which
        route; under ``"deterministic"`` nothing draws from it and it changes only the
        fingerprint.
        """
        return self._summary.master_seed

    @property
    def choice_model(self) -> str:
        """The name of the choice model that picked each trip's route, itinerary or mode.

        ``"nested_logit"``, ``"logit"`` or ``"deterministic"``, or the ``name`` of a model
        written in Python.
        """
        return self._summary.choice_model

    @property
    def equilibration(self) -> str:
        """The name of the equilibration strategy: ``"free_flow"``, ``"msa"`` or ``"sra"``."""
        return self._summary.equilibration

    @property
    def route_update(self) -> str:
        """The name of the route update that grew the route sets between iterations.

        ``"none"`` (the sets stayed as the route method made them), ``"best_response"`` (the
        internal name of ``"least_cost"``, column generation) or ``"sampled"`` (the internal name
        of ``"doubly_stochastic"``).
        """
        return self._summary.route_update

    @property
    def converged(self) -> bool:
        """Whether the run met its stopping rule by its last loading.

        Under ``"msa"`` and ``"sra"``: the disequilibrium, averaged over the last three
        iterations, under ``gap_tolerance`` with the route sets no longer growing, whether that
        stopped the run early or held at its last allowed iteration. Always ``False`` with a
        ``gap_tolerance`` of 0, or under ``"free_flow"``.
        """
        return self._summary.converged

    @property
    def convergence_gap(self) -> float:
        """How far the run ended from equilibrium.

        **The worst of** the last iteration's disequilibrium against the choice set
        (``convergence()["gap_excess"][-1]``; ``gap_generalised_excess`` where it is measured, a
        run whose choice model weighs more than time on a car route, as its stopping rule
        does), against the whole network
        (``convergence()["gap_network_excess"][-1]``, a sample), so a route the choice set
        was missing cannot hide behind a small in-set number, and of the itineraries
        (``convergence()["itinerary_gap_excess"][-1]``: transit, park-and-ride, bike-and-ride
        and trips choosing their mode, each against the best of the mode it kept). The
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
        generalised = c.get("gap_generalised_excess")
        if generalised is not None and len(generalised) and generalised[-1] == generalised[-1]:
            values[0] = float(generalised[-1])  # the in-set gap the stopping rule reads
        values.append(float(c["itinerary_gap_excess"][-1]))
        finite = [v for v in values if v == v]  # not nan
        return max(finite) if finite else float("nan")

    @property
    def convergence_verdict(self) -> str | None:
        """``"good"`` (disequilibrium below 5%), ``"acceptable"`` (below 15%) or ``"poor"``.

        ``None`` if no gap was measured (a ``"free_flow"`` run). Judged on
        ``convergence_gap``, the disequilibrium: what the choice model does not explain.

        **``"poor"`` whatever the gap when more than 5 % of the trips did not finish**
        (``unfinished_share``): the gap is measured over the trips that finished, so a jammed
        run's few finishers can read as settled while most of its traffic is still on the road.
        """
        gap = self.convergence_gap
        if gap != gap:  # nan
            return None
        if self.unfinished_share > 0.05:
            return "poor"
        return "good" if gap < 0.05 else "acceptable" if gap < 0.15 else "poor"

    @property
    def unfinished_share(self) -> float:
        """The share of trips still travelling when the window ended, in the last loading.

        ``truncated`` over ``total_trips`` of ``completion``; 0 without trips. Above 0.05 the
        ``convergence_verdict`` is ``"poor"`` whatever the gap.
        """
        c = self.completion
        return c["truncated"] / c["total_trips"] if c["total_trips"] else 0.0

    def convergence(self) -> Columns:
        """What each iteration showed: how far the run is from an equilibrium.

        NumPy arrays, one entry per iteration (a ``"free_flow"`` run has one).
        ``nan`` marks a number that was not measured. The result of the run
        (``completion``, ``total_travel_time_s``, ``link_bins()``, ``route_choices()``)
        is that of the **last** iteration; with disruptions travellers did not know of, it is
        that of the one more loading made with them, which no row here describes.

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
          and what ``convergence_verdict`` judges (``gap_generalised_excess`` instead where that
          is measured). Equal to ``gap`` for ``"deterministic"``.
        * ``incomplete_share`` — the share of travellers (by weight) whose trip had not finished
          when the window ended, who are left out of the gaps.
        * ``gap_network`` — the same measure against the **whole network**: the least
          time over *any* route at the current times (read off the run's shared search
          trees, one time-dependent search per origin and departure bin), for every trip
          that finished, at the **last** iteration only, ``nan`` elsewhere (``gap_sample`` 0
          switches it off). It shows whether the choice set was missing a route that traffic
          has made worthwhile.
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
          (``route_update="least_cost"``), how many routes it added to the route sets
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
        * ``gap_transit_plan_s`` — **the planned-against-realised difference of transit**: over
          the trips with a transit part that ended in this loading (by traveller weight), the
          mean of the second the transit part ended (at the destination, or at the parking for
          a trip back to its vehicle) less the second it would have ended had every vehicle
          kept to the timetable the trip planned on, followed the same way from the same moment
          at its first stop, in seconds (negative: earlier). Travellers plan on the timetable,
          so the gaps above never see it: 0 when the buses keep to the timetable (the
          default); with ``transit_options`` ``buses_on_roads`` 1, what traffic did to the
          buses. ``nan`` when no trip rode.
        * ``gap_transit_plan_share`` — its share of the itineraries' gap with transit: that mean
          over itself plus the mean seconds the transit, park-and-ride and bike-and-ride trips
          measured in ``gap_transit``, ``gap_car_transit`` and ``gap_bike_transit`` paid above
          the best of their mode. 0 when the difference is 0; ``nan`` when it is, or when no
          such trip was measured (a run of one loading). Neither is in ``kpis.parquet``.
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
          ``convergence_gap`` and of what ``gap_tolerance`` stops on.
        * ``gap_generalised_excess`` — **the disequilibrium in generalised minutes**: over the
          car trips that finished (by traveller weight) and whose route taken was offered to
          them, ``(Σ w g_chosen − Σ w g_expected) / Σ w g_least``, ``g`` a route's generalised
          minutes at this loading's times (defined under the full gaps below), ``g_expected``
          its expectation under the model's probabilities over the routes offered and
          ``g_least`` the least of them: ``gap_excess`` with money, length and the user's link
          values counted beside time. A ratio, not minutes. Where the choice model weighs more
          than time on a car route (a price per kilometre, a toll, a user link value), it is
          what ``gap_tolerance`` stops on in place of ``gap_excess``, together with
          ``itinerary_gap_excess``, and what ``convergence_gap`` and ``convergence_verdict``
          read. ``nan`` where the route cost is travel time alone (the
          defaults: ``gap_excess`` is then its counterpart in travel time), for a model that
          gives no probabilities, and for a run of one loading.

        **The full gaps**, measured over every trip after the last loading (after every loading with
        ``equilibration_options`` ``gap_every`` 1, else ``nan`` before the last), in the choice
        model's own currency (``gap_currency``). ``g``, an alternative's **generalised minutes**, is
        its utility under the choice model (without the path-size term, which only corrects for
        routes that overlap) divided by the traveller's time coefficient: a minute of travel counts
        one, a euro ``beta_cost_eur / beta_time_min`` minutes. Nothing stops or judges the run on
        them (``gap_tolerance``, ``convergence_gap`` and ``convergence_verdict`` read the gaps
        above); ``nan`` for a run of one loading. ``gap_trips()`` gives each trip's numbers.

        * ``gap_route`` — **the full route gap**: over every trip that drove a car route in this
          loading (given the car, or choosing its mode and keeping a car alternative), how much
          more the route taken costs than the best route the trip was offered, ``Σ w (g_chosen −
          g_best) / Σ w g_best``. **A trip still under way when the window ended counts**: as the
          time it had travelled plus the time of the links it had not yet left, at their last
          recorded times held, so trips left on the road weigh in the gap instead of being left
          out of it. (Where the queue is still growing at the end, those times are below what the
          rest of it will take.)
        * ``gap_route_expected`` — what the choice model itself expects ``gap_route`` to be at
          these costs: the same sum with each trip's expected ``g`` under the model's
          probabilities over the routes it was offered; ``nan`` without probabilities.
        * ``gap_route_excess`` — ``gap_route − gap_route_expected``: the deterministic part of
          the duality gap (the relative duality gap of the route choice less the part the logit's
          own dispersion produces), what the model does not explain.
        * ``gap_route_network`` — ``gap_route`` against the **whole network**: the best of the
          routes offered or the fastest route a time-dependent search finds for the trip, every
          trip (``gap_network``: the trips that finished). The search looks for the fastest
          route, not the cheapest in ``g``, so where money weighs this gap can read low.
        * ``gap_route_network_excess`` — its disequilibrium: ``gap_route_network`` less what the
          model expects inside the set, so a route the set was missing counts in full.
        * ``gap_mode`` — **the full mode gap**: over every trip choosing its mode (``modes``),
          how much more the alternative taken costs than the best of its whole choice set, every
          mode, planned on this loading's costs; ``nan`` without mode choice.
        * ``gap_mode_expected`` — what the choice model expects ``gap_mode`` to be, from its
          probabilities over the whole choice set.
        * ``gap_mode_excess`` — ``gap_mode − gap_mode_expected``.
        * ``excess_min_route`` — the deterministic part of the route gap as minutes per
          traveller: what the average traveller pays, in ``g``, above what the model expects of
          them.
        * ``excess_min_mode`` — the same over the trips choosing their mode.
        * ``unfinished_cost_share`` — of what the route gap's trips paid (their ``g``, summed),
          the share that trips still under way at the end had still to pay; 0 when every trip
          arrived.

        The numbers up to ``gap_flow_excess`` are also in ``kpis.parquet``, a row per metric
        per iteration, and so are the itinerary gaps (under their modes, as ``gap``),
        ``hub_mismatch_s``, ``mode_changed_share``, ``itinerary_gap_excess``,
        ``gap_generalised_excess`` and the full gaps.
        """
        return Columns(self._summary.convergence)

    @property
    def gap_currency(self) -> str:
        """What the full gaps of ``convergence()`` and ``gap_trips()`` are measured in.

        ``"utility"``: generalised minutes, each alternative's utility under the choice model
        (without the path-size term) over the traveller's time coefficient. ``"time"``:
        door-to-door minutes, for a model that gives no utilities (``"deterministic"``, or one
        written in Python) or a time coefficient that is not below 0. Not in the run's files, so
        a run read back with ``Run.open`` has none.
        """
        return self._summary.gap_currency

    def gap_trips(self) -> Columns | None:
        """Each trip's numbers in the full gaps of the last loading, in generalised minutes.

        One row per trip measured, in the order of the demand: a trip that drove a car route,
        and a trip choosing its mode. ``trip`` is its index in the demand; ``g_chosen_min`` the
        alternative it took, at the last loading's costs (a car trip still under way when the
        window ended: the time it had travelled plus what it still had to drive);
        ``g_best_set_min`` the best of the car routes it was offered; ``g_best_network_min`` the
        best over the whole network (the fastest route a time-dependent search finds, if
        better); ``g_expected_min`` what the choice model expects of it (over every mode for a
        trip choosing its mode, over its car routes for the others); ``g_best_all_min`` the best
        of its whole choice set, every mode (``nan`` for a trip given its mode); ``unfinished``
        whether it was still under way at the end. The units are ``gap_currency``'s; the sums
        are ``convergence()``'s ``gap_route`` and ``gap_mode``, at its last row.

        In memory, from the run that produced it: ``None`` for a run of one loading, and a run
        read back with ``Run.open`` has none.
        """
        gap = self._summary.gap_trips
        return None if gap is None else Columns(gap)

    def gap_pairs(self, departure_bin_s: int = 300) -> Columns | None:
        """The gap per origin, destination and departure bin, on the costs the trips paid.

        The trips of ``gap_trips()`` (the last loading's) grouped by their origin point, their
        destination point (their coordinates as given, rounded to 1e-6 degrees so that equal
        points group) and their departure bin, ``floor((departure_s − start_s) /
        departure_bin_s)`` counted from the window's start (``window["start_s"]``). A group's
        best, ``g_best_min``, is the least ``g_chosen_min`` of its trips: the best cost a trip of
        the group paid, at the last loading's costs. Every trip of the group is measured against
        it. This is the literature's dynamic relative gap per origin, destination and departure
        interval, read on the costs paid rather than against a shortest route (Boyles, Lownes &
        Unnikrishnan 2025, *Transportation Network Analysis*, the dynamic relative gap): at a
        dynamic user equilibrium the trips that leave one
        place for another at about the same time pay the same, and what a trip paid above the
        best of its group is what it could have saved by doing as that one did. It needs no
        search.

        It is meaningful when the trips of a pair share their two points: the benchmark networks'
        nodes, ``demand_from_od``'s ``points_per_zone``, ``Case.demand``'s
        ``one_point_per_zone``. With trip ends spread over their zones nearly every group holds
        one trip, and the gap reads 0. Within a bin the trips leave at different seconds, so
        where the costs move within it (a queue growing) a trip leaving late reads a gap it could
        not have closed: the wider the bin, the more it overstates. Trips choosing their mode are
        grouped the same way, their ``g_chosen_min`` the alternative they took, so a group may
        hold them beside trips given the car, and travellers of other classes, whose generalised
        minutes count money differently. The units are ``gap_currency``'s. Under a stochastic
        choice model (the logit) it holds the model's own spread over routes as well as the
        disequilibrium, so it compares with ``convergence()``'s ``gap_route``, not with
        ``gap_route_excess``.

        One row per group, sorted by origin, destination and bin: ``origin_lon``,
        ``origin_lat``, ``destination_lon``, ``destination_lat`` (the rounded points),
        ``bin_start_s`` (the bin's start, seconds after midnight), ``trips``, ``people`` (their
        weights summed), ``g_best_min``, ``g_mean_min`` (the mean of ``g_chosen_min`` over the
        group's people), ``excess_min`` (``Σ w (g_chosen − g_best)``, minutes summed over the
        group's people), ``gap`` (``excess_min / Σ w g_best``; ``nan`` where that is 0) and
        ``unfinished`` (the group's trips still under way when the window ended).
        ``gap_pairs_relative`` sums it over every group.

        Args:
            departure_bin_s: The bins' width, in seconds: a whole number of at least 1. A bin
                as wide as the window gives one group per pair.

        Returns:
            The columns, or ``None`` for a run of one loading (``gap_trips()`` is ``None``).

        Raises:
            ValueError: If ``departure_bin_s`` is not a whole number of at least 1.
            RuntimeError: For a run read back with ``Run.open``, which has no ``gap_trips()``.
        """
        bin_s = departure_bin_s
        if isinstance(bin_s, bool) or int(bin_s) != bin_s or bin_s < 1:
            raise ValueError(f"departure_bin_s must be a whole number of at least 1, got {bin_s}")
        gap = self.gap_trips()
        if gap is None:
            return None
        trips = self.trip_results()
        trip = np.asarray(gap["trip"], dtype=np.int64)
        g = np.asarray(gap["g_chosen_min"], dtype=float)
        keep = np.isfinite(g)
        trip, g = trip[keep], g[keep]
        unfinished = np.asarray(gap["unfinished"], dtype=bool)[keep]
        w = np.asarray(trips["weight"], dtype=float)[trip]
        start_s = int(self.window["start_s"])
        departure = np.asarray(trips["departure_s"], dtype=np.int64)[trip]
        bins = np.floor_divide(departure - start_s, int(bin_s))
        micro = [
            np.rint(np.asarray(trips[name], dtype=float)[trip] * 1e6).astype(np.int64)
            for name in ("origin_lon", "origin_lat", "destination_lon", "destination_lat")
        ]
        keys = np.column_stack([*micro, bins]) if len(trip) else np.empty((0, 5), np.int64)
        groups, of = np.unique(keys, axis=0, return_inverse=True)
        of = np.asarray(of).reshape(-1)
        n = len(groups)
        best = np.full(n, np.inf)
        np.minimum.at(best, of, g)
        count = np.bincount(of, minlength=n)
        people = np.bincount(of, weights=w, minlength=n)
        paid = np.bincount(of, weights=w * g, minlength=n)
        excess = np.bincount(of, weights=w * (g - best[of]), minlength=n)
        late = np.bincount(of, weights=unfinished.astype(float), minlength=n)
        reference = people * best
        with np.errstate(divide="ignore", invalid="ignore"):
            mean = np.where(people > 0, paid / people, np.nan)
            ratio = np.where(reference != 0, excess / reference, np.nan)
        return Columns(
            {
                "origin_lon": groups[:, 0] / 1e6,
                "origin_lat": groups[:, 1] / 1e6,
                "destination_lon": groups[:, 2] / 1e6,
                "destination_lat": groups[:, 3] / 1e6,
                "bin_start_s": start_s + groups[:, 4] * int(bin_s),
                "trips": count.astype(np.int64),
                "people": people,
                "g_best_min": best,
                "g_mean_min": mean,
                "excess_min": excess,
                "gap": ratio,
                "unfinished": late.astype(np.int64),
            }
        )

    def gap_pairs_relative(self, departure_bin_s: int = 300) -> float:
        """The gap per origin, destination and departure bin, as one number for the run.

        ``gap_pairs``' excess summed over every group, over what the groups' people would have
        paid at their groups' best: ``Σ excess_min / Σ people × g_best_min``, a share like the
        run's other gaps (0.01 is 1 %). Where every group's trips are the same trip (one point
        each end, one second of departure, one class, each given the car and arrived), each
        group's best is no lower than the best route its trips were offered, so this is at most
        ``gap_route``; trips leaving at other seconds of a bin can make it read higher.

        Args:
            departure_bin_s: The bins' width, in seconds (``gap_pairs``').

        Returns:
            The share, or ``nan`` for a run of one loading (``gap_trips()`` is ``None``).

        Raises:
            ValueError: If ``departure_bin_s`` is not a whole number of at least 1.
            RuntimeError: For a run read back with ``Run.open``, which has no ``gap_trips()``.
        """
        pairs = self.gap_pairs(departure_bin_s)
        if pairs is None:
            return float("nan")
        reference = float(np.sum(pairs["people"] * pairs["g_best_min"]))
        excess = float(np.sum(pairs["excess_min"]))
        return excess / reference if reference != 0 else float("nan")

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

    def bike_route_sets(self) -> _core.RouteSets | None:
        """The bike route sets, with ``bike_route_choice="doubly_stochastic"``; ``None`` otherwise.

        One set per pair of bike-layer nodes, made once per run on the layer's static costs
        by the doubly stochastic method (its internal name, the sets' ``method``, is
        ``"sampled"``); the first route of a set is the least-cost one.
        """
        return self._summary.bike_route_sets

    def bike_route_choices(self) -> _core.RouteChoices | None:
        """Which route each stated bike trip took among its set; ``None`` without bike sets.

        The same arrays as ``route_choices()``, one entry per trip of the demand; ``-1`` marks
        a trip that is not a stated bike trip or has no bike route. A trip that chose its mode
        and took the bike is in ``itinerary_choices()`` instead.
        """
        return self._summary.bike_route_choices

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
        path = self._written(path, f"the {layer} layer's link bins")
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
        return Table(self._summary.kpis_path)  # kpis.csv for a run's minimal outputs

    def diagnostics(self) -> Table:
        """``diagnostics.parquet``: aggregated counters, one row per issue."""
        return Table(self._written(self._summary.diagnostics_path, "diagnostics.parquet"))

    def events(self) -> Table:
        """``events.parquet``: sampled per-trip events."""
        return Table(self._written(self._summary.events_path, "events.parquet"))

    def manifest(self) -> dict[str, Any]:
        """``manifest.json``: the file that makes this run reproducible."""
        with Path(self._summary.manifest_path).open(encoding="utf-8") as f:
            data: dict[str, Any] = json.load(f)
        return data

    def timings(self) -> Columns:
        """Wall-clock time by stage, measured on this machine's clock.

        Columns by name, one row per stage in the order the stages ended: ``stage``,
        ``iteration`` (``None`` for a stage outside the iterations) and ``seconds``. Before the
        iterations: ``network_read`` (if the network recorded how long its reading took; it was
        read before the run, and is not in the total), ``demand_read``, ``setup`` (the bike and
        walk layers, the timetable and the parkings prepared, the fingerprint),
        ``static_routes``, ``itineraries_setup``, ``route_sets``. Per iteration:
        ``route_choice`` and ``itinerary_choice``, ``partial_loading`` and ``route_update``
        (iteration 0's: the free-flow loading's groups, one row each), ``loading``, then after a
        loading what prepares the next (``route_update``, ``route_choice``,
        ``itinerary_choice``), ``gap`` (the full gaps of ``convergence()``, after the last, or
        after each with ``gap_every`` 1) and
        ``network_gap`` after the last. After them:
        ``results_assembled``, ``results_written`` and ``total`` (the run alone). Also in
        ``timings.csv`` next to the other files. Times differ between runs, so they are in no
        result and not in the fingerprint.
        """
        rows = self._summary.timings
        return Columns(
            stage=[r[0] for r in rows],
            iteration=[r[1] for r in rows],
            seconds=[r[2] for r in rows],
        )

    @property
    def timings_path(self) -> str:
        """Path to ``timings.csv``: ``Run.timings()`` as a file."""
        return self._written(self._summary.timings_path, "timings.csv")

    @property
    def fallback_car(self) -> Columns:
        """The trips that had no alternative and travelled by car, as columns.

        No mode they may use reached their destination within the limits (a walk too long, no
        transit journey, no vehicle), so they went by car — for a traveller without one, a lift
        or a taxi. They are counted under the car everywhere. Columns: ``traveller_id``,
        ``trip_seq``, ``departure_s``, ``weight`` (people), ``origin_lon``, ``origin_lat``,
        ``destination_lon``, ``destination_lat``. Empty unless ``mode_options={"fallback_car":
        1}`` (experimental, off by default, when such trips stay unsimulated).
        ``report_no_alternative()`` says why each had no alternative.
        """
        return Columns(self._summary.fallback_car)

    def report_no_alternative(self) -> Columns:
        """The trips the last choice left with no alternative, and why, as columns.

        No mode they may use reached their destination within the limits, so they were not
        simulated (unless ``mode_options={"fallback_car": 1}``, when they went by car). One row
        per trip, in trip order: ``traveller_id``, ``trip_seq``, ``user_class``,
        ``departure_s``, ``weight`` (people), ``origin_lon``, ``origin_lat``,
        ``destination_lon``, ``destination_lat``, ``mode_stated`` (``None``: the trip chose its
        mode; else its stated transit, park-and-ride or bike-and-ride, the only mode it was
        offered), ``fallback_car`` (whether it then went by car), and per mode, ``reason_car``,
        ``reason_bike``, ``reason_walk``, ``reason_transit``, ``reason_car_transit``,
        ``reason_bike_transit``: ``""`` if the mode was not offered (not among the run's
        ``modes``, not one the class may use, or not the stated mode), else why it gave nothing:

        - ``walk_too_long``: no walk within the walk limit (``walk_max_s``);
        - ``ride_too_long``: no ride within the ride limit (``bike_max_s``);
        - ``no_vehicle``: the traveller has no such vehicle, or left it elsewhere that day;
        - ``no_road_route``: no road route between origin and destination;
        - ``no_stop_in_reach``: no stop within the walk to a stop (``access_walk_max_s``) of the
          origin or of the destination;
        - ``no_transit_journey``: stops in reach, but no journey within the longest offered
          (``journey_max_s``) on the window's timetable;
        - ``too_short``: shorter than park-and-ride's and bike-and-ride's shortest trip
          (``pr_min_km``);
        - ``unexplained``: none of these; a defect to report.

        ``change`` names, for each row, what to change: the options behind its reasons
        (``NO_ALTERNATIVE_CHANGES``).
        """
        table = Columns(self._summary.no_alternative)
        columns = [f"reason_{mode}" for mode in self._modes_listed(table)]
        table["change"] = [
            "; ".join(dict.fromkeys(NO_ALTERNATIVE_CHANGES[r] for r in reasons if r))
            for reasons in zip(*(table[c] for c in columns), strict=True)
        ]
        return table

    @staticmethod
    def _modes_listed(table: Any) -> tuple[str, ...]:
        """The modes a no-alternative table lists: ``MODES``, and a service's mode with one."""
        return MODES + tuple(m for m in SERVICE_MODES if f"reason_{m}" in table)

    def _no_alternative_text(self, places: int = 5) -> str | None:
        """The trips left with no alternative and not simulated, in words; ``None`` if none."""
        t = self.report_no_alternative()
        rows = [i for i, fell in enumerate(t["fallback_car"]) if not fell]
        if not rows:
            return None
        weight = [float(t["weight"][i]) for i in rows]
        people = sum(weight)
        total = sum(r["people"] for r in self.completion_by_mode.values())
        # Who: by class and stated mode ("choosing" for a trip that chose its mode).
        groups: dict[tuple[str, str], int] = {}
        for i in rows:
            key = (t["user_class"][i], t["mode_stated"][i] or "choosing")
            groups[key] = groups.get(key, 0) + 1
        who = ", ".join(
            f"{n:,} {cls!r} ({mode})"
            for (cls, mode), n in sorted(groups.items(), key=lambda kv: (-kv[1], kv[0]))
        )
        # Why: per mode offered, the trips each reason held back.
        why = []
        listed = self._modes_listed(t)
        for mode in listed:
            counts: dict[str, int] = {}
            for i in rows:
                reason = t[f"reason_{mode}"][i]
                if reason:
                    counts[reason] = counts.get(reason, 0) + 1
            if counts:
                parts = ", ".join(
                    f"{r} {n:,}" for r, n in sorted(counts.items(), key=lambda kv: -kv[1])
                )
                why.append(f"{mode}: {parts}")
        # Where: origins and destinations on a grid of about a kilometre.
        cell = 0.01
        pairs: dict[tuple, float] = {}
        for i, w in zip(rows, weight, strict=True):
            values = (
                t["origin_lat"][i],
                t["origin_lon"][i],
                t["destination_lat"][i],
                t["destination_lon"][i],
            )
            key = tuple(round(v / cell) * cell for v in values)
            pairs[key] = pairs.get(key, 0.0) + w
        top = sorted(pairs.items(), key=lambda kv: (-kv[1], kv[0]))[:places]
        where = "; ".join(
            f"{w:,.0f} people from ({a:.2f}, {b:.2f}) to ({c:.2f}, {d:.2f})"
            for (a, b, c, d), w in top
        )
        reasons = {r for mode in listed for i in rows if (r := t[f"reason_{mode}"][i])}
        change = "; ".join(
            f"{r}: {NO_ALTERNATIVE_CHANGES[r]}" for r in NO_ALTERNATIVE_CHANGES if r in reasons
        )
        return (
            f"{len(rows):,} trips ({people:,.0f} people, {people / max(total, 1):.1%} of run "
            f"{self.run_id!r}) had no alternative — no mode they may use reached their "
            "destination within the limits — and were not simulated. By class (stated mode): "
            f"{who}. Why, by mode offered (trips): {'; '.join(why)}. Most between (latitude, "
            f"longitude, about 1 km): {where}. To change: {change}. "
            "Run.report_no_alternative() lists every trip with its reasons."
        )

    def _fallback_text(self, places: int = 5) -> str | None:
        """The trips that fell back to the car, in words, and where most went; ``None`` if none."""
        f = self.fallback_car
        n = len(f["traveller_id"])
        if not n:
            return None
        people = float(sum(f["weight"]))
        total = sum(r["people"] for r in self.completion_by_mode.values())
        # Where: origins and destinations on a grid of about a kilometre.
        cell = 0.01
        pairs: dict[tuple, float] = {}
        for olon, olat, dlon, dlat, w in zip(
            f["origin_lon"],
            f["origin_lat"],
            f["destination_lon"],
            f["destination_lat"],
            f["weight"],
            strict=True,
        ):
            key = tuple(round(v / cell) * cell for v in (olat, olon, dlat, dlon))
            pairs[key] = pairs.get(key, 0.0) + float(w)
        top = sorted(pairs.items(), key=lambda kv: (-kv[1], kv[0]))[:places]
        where = "; ".join(
            f"{w:,.0f} people from ({a:.2f}, {b:.2f}) to ({c:.2f}, {d:.2f})"
            for (a, b, c, d), w in top
        )
        return (
            f"{n:,} trips ({people:,.0f} people, {people / max(total, 1):.1%} of run "
            f"{self.run_id!r}) had no alternative — no mode they may use reached their "
            "destination within the limits — and travelled by car instead (a lift or a taxi for "
            "a traveller without one); they are counted under the car. Most between (latitude, "
            f"longitude, about 1 km): {where}. Run.fallback_car lists them all and "
            "Run.report_no_alternative() says why; mode_options={'fallback_car': 0}, the "
            "default, leaves them unsimulated instead."
        )

    @property
    def window(self) -> dict[str, Any]:
        """The simulation window and what departed outside it, left out of the run.

        Keys: ``start_s`` and ``end_s`` (seconds after midnight), ``start_given`` and
        ``end_given`` (whether the scenario gave them; else from the earliest departure, rounded
        down to the quarter hour, for ``default_s``), ``trips`` and ``people`` (the whole demand),
        ``before_trips``, ``before_people``, ``before_first_s``, ``before_last_s`` and the same
        ``after_*`` (left out), and ``timetable_runs_kept``, ``timetable_runs_cut``,
        ``timetable_runs_dropped``, ``timetable_calls_dropped`` (the timetable kept only its
        calls inside the window; ``None`` without one).
        """
        return dict(self._summary.window)

    def _window_text(self) -> str | None:
        """What the run left out, departing outside the window, in words; ``None`` if nothing."""
        w = self.window
        out = w["before_trips"] + w["after_trips"]
        if not out:
            return None
        people = w["before_people"] + w["after_people"]
        share = people / w["people"] if w["people"] else 0.0
        span = f"{_clock(w['start_s'])}–{_clock(w['end_s'])}"
        sides = []
        if w["before_trips"]:
            sides.append(
                f"{w['before_trips']:,} depart before it "
                f"({_clock(w['before_first_s'])}–{_clock(w['before_last_s'])})"
            )
        if w["after_trips"]:
            sides.append(
                f"{w['after_trips']:,} at or after its end "
                f"({_clock(w['after_first_s'])}–{_clock(w['after_last_s'])})"
            )
        if w["start_given"] and w["end_given"]:
            origin = "the scenario's window"
        else:
            origin = (
                "a window set from the earliest departure (rounded down to the quarter hour), "
                f"{w['default_s'] / 3600:g} hours long, as no window was given"
            )
        first = min(w["start_s"], w["before_first_s"] or w["start_s"])
        last = max(w["end_s"], (w["after_last_s"] or 0) + 3600)
        wide = (f"{_clock(first // 900 * 900)}", f"{_clock(-(-last // 900) * 900)}")
        return (
            f"{out:,} of the demand's {w['trips']:,} trips ({people:,} of {w['people']:,} "
            f"people, {share:.1%}) depart outside the simulation window {span} and are left "
            f"out of run {self.run_id!r}: {'; '.join(sides)}. They are not simulated and not "
            "counted in any result (not as completed, unfinished or without an alternative). "
            f"The window is {origin}. To run them, widen it, for example "
            f"Scenario(..., window=({wide[0]!r}, {wide[1]!r})), leaving each trip time to "
            "finish before the end; or take them out of the demand."
        )

    def _summary_line(self) -> str:
        """The one line a run says when it ends (``logging``, INFO; C2)."""
        c, w = self.completion, self.window
        seconds = sum(s for stage, it, s in self._summary.timings if stage == "total")
        verdict = self.convergence_verdict
        return (
            f"Run {self.run_id!r}: {seconds:.1f} s, window {_clock(w['start_s'])}–"
            f"{_clock(w['end_s'])}, {c['completed']:,} of {c['total_trips']:,} trips completed, "
            f"mean trip {self.mean_travel_time_s / 60:.1f} min"
            + (f", verdict {verdict}" if verdict else "")
            + " (run.timings() has every stage)"
        )

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
            ("gap", "gap"),
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
        w = self.window
        out = w["before_trips"] + w["after_trips"]
        fell = len(self.fallback_car["traveller_id"])
        stranded = sum(not f for f in self._summary.no_alternative["fallback_car"])
        lines = [
            f"Run {self.run_id!r}: window {_clock(w['start_s'])}–{_clock(w['end_s'])}"
            + (f", {out:,} trips left out (departing outside it)" if out else "")
            + (f", {fell:,} trips with no alternative went by car" if fell else "")
            + (f", {stranded:,} trips with no alternative not simulated" if stranded else ""),
            f"Run {self.run_id!r}: wall-clock seconds by stage",
        ]
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
        lines.append("")
        lines.append(
            f"Run {self.run_id!r}: viz.summary(run) writes a one-page summary (the numbers, the "
            "modes, the convergence, a map per mode); viz.map_interactive(run, path) an "
            "interactive map"
        )
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
        path = self._written(self._summary.transit_calls_path, "transit_calls.parquet")
        return None if path is None else Table(path)

    def transit_calls(self) -> Columns | None:
        """Every call of the timetable in this run, as columns; ``None`` without one.

        One row per call (a run at a stop, in order): ``run`` (the GTFS
        ``trip_id``), ``line`` (its short name), ``kind`` (``"bus"``, ``"tram"``,
        ``"metro"``, ``"rail"``, ``"ferry"``, ``"other"``), ``stop_id``, ``sequence``
        (the call's place in its run, from 0), ``scheduled_arrival_s``,
        ``scheduled_departure_s``, and ``arrival_s``, ``departure_s`` — the times in
        the run: a bus that rode the roads keeps the loading's, the rest the
        schedule's; ``NaN`` (null in ``transit_calls.parquet``) where a bus had not got there by the
        end of the window —
        then ``boardings``, ``alightings`` and ``on_board`` (as the vehicle leaves),
        weighted by traveller weight. Seconds count from the service day's midnight.
        """
        calls = self._summary.transit_calls
        return None if calls is None else Columns(calls)

    @property
    def transit_summary(self) -> dict[str, float] | None:
        """How transit went, as numbers by name; ``None`` without a timetable.

        ``boardings`` (weighted); and, with buses on the roads (``transit_options``
        ``buses_on_roads`` 1), for the buses: ``bus_groups_on_roads`` (bus
        patterns that rode the roads), ``bus_groups_by_schedule_off_road``,
        ``bus_groups_by_schedule_no_route`` and ``bus_groups_by_schedule_implausible``
        (those run by the schedule, and why), ``bus_runs_on_roads``,
        ``bus_runs_arrived`` (reached their last stop within the window) and
        ``bus_delay_mean_s`` (their mean delay there, realised minus scheduled;
        negative is early). With itinerary trips, how their planning searched the timetable
        (RAPTOR queries over the whole run): ``timetable_searches_transit``,
        ``timetable_searches_out`` and ``timetable_searches_back`` (made, for plain transit,
        out of a parking and back to one), ``timetable_reused_transit``,
        ``timetable_reused_out`` and ``timetable_reused_back`` (answered from a search kept
        earlier in the run instead: the timetable planned on is the same in every iteration,
        so the same query has the same answer), ``timetable_kept`` (searches kept at the end)
        and ``timetable_kept_bytes`` (the memory they hold).
        """
        summary = self._summary.transit_summary
        return None if summary is None else dict(summary)

    def route_set_summary(self) -> dict[str, float] | None:
        """What the car route sets held and how they were used; ``None`` without car route sets.

        Read from ``route_sets()``, ``route_choices()`` and ``convergence()``; every number is a
        ``float``, ``nan`` where it has nothing to count. A share is weighted by the people a trip
        stands for (``weight``) where it is about trips, and unweighted where it is about routes.

        * ``pairs`` — how many origin-destination pairs have a route set.
        * ``routes`` — how many routes the sets hold in all, the route update's included.
        * ``routes_per_pair_mean``, ``routes_per_pair_max`` — the mean and the largest number of
          routes in one pair's set.
        * ``routes_added`` — how many routes the route update added over the run (the sum of
          ``convergence()["routes_added"]``); 0 without one. A route added to a full set takes
          the place of one nobody chose, so ``routes`` can grow by less.
        * ``route_searches`` — how many searches the route update made over the run (the sum of
          ``convergence()["route_searches"]``).
        * ``share_on_best`` — of the car trips with a route, the share whose route is its pair's
          free-flow shortest (the first route of the set, ``route_choices().rank`` 0).
        * ``share_routes_unchosen`` — the share of the routes held that no trip took at the
          last choice.
        * ``alternatives_mean``, ``alternatives_p95`` — the mean and the 95th percentile of the
          number of routes a car trip could choose from (``route_choices().alternatives``),
          over the trips with a route, each trip counted once.
        """
        sets = self.route_sets()
        if sets.key_count == 0:
            return None
        nan = float("nan")
        sizes = np.diff(sets.set_offsets().astype(np.int64))
        rc = self.route_choices()
        routed = rc.route >= 0
        weight = rc.weight[routed].astype(np.float64)
        alternatives = rc.alternatives[routed].astype(np.float64)
        # The method's routes come best first and the update's after them, so the position in
        # the set (rank) 0 is the pair's free-flow shortest route (S298).
        on_best = rc.rank[routed] == 0
        taken = np.unique(rc.route[routed]).size
        c = self.convergence()
        return {
            "pairs": float(sets.key_count),
            "routes": float(sets.route_count),
            "routes_per_pair_mean": float(sizes.mean()),
            "routes_per_pair_max": float(sizes.max()),
            "routes_added": float(np.sum(c["routes_added"])),
            "route_searches": float(np.sum(c["route_searches"])),
            "share_on_best": (
                float(np.sum(weight[on_best]) / np.sum(weight)) if weight.sum() > 0 else nan
            ),
            "share_routes_unchosen": (
                float(1.0 - taken / sets.route_count) if sets.route_count > 0 else nan
            ),
            "alternatives_mean": float(alternatives.mean()) if alternatives.size else nan,
            "alternatives_p95": (
                float(np.percentile(alternatives, 95)) if alternatives.size else nan
            ),
        }

    def parking_bins(self) -> Columns | None:
        """How full each parking was over the day, bin by bin; ``None`` without parkings.

        Columns, one row per parking and bin: ``bin`` (counted from midnight, like the link
        bins': ``bin = start_s // bin_s``, so a window starting at 07:30 in 15-minute bins
        starts at bin 30), ``start_s``, ``parking`` (its
        index in the run), ``parking_id``, ``hub_id``, ``vehicle`` (``"car"`` or
        ``"bike"``), ``capacity``, ``arrivals`` and ``departures`` (vehicles parked and
        fetched in the bin, weighted by traveller weight), ``occupancy_mean``,
        ``occupancy_max``, ``full_s`` (seconds of the bin the parking was full) and
        ``overflow_max`` (the most vehicles above capacity at once: a full parking
        refuses nobody; it costs more, and the overflow is counted). The same table is
        written to ``parking_bins.parquet``: read it with ``parking_bins_table()``.
        """
        bins = self._summary.parking_bins
        return None if bins is None else Columns(bins)

    def parking_bins_table(self) -> Table | None:
        """``parking_bins.parquet``: ``parking_bins()`` after ``run_id``; ``None`` without parkings.

        The file's metadata carries the run's fingerprint and its master seed.
        """
        path = self._written(self._summary.parking_bins_path, "parking_bins.parquet")
        return None if path is None else Table(path)

    def parking_places(self) -> Columns | None:
        """Each parking the run kept, as columns; ``None`` without parkings.

        ``parking_id``, ``hub_id``, ``vehicle``, ``capacity``, ``lon``, ``lat`` and
        ``name``. ``parking`` in ``parking_bins()`` indexes these.
        """
        places = self._summary.parking_places
        return None if places is None else Columns(places)

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

    def itinerary_choices(self) -> Columns | None:
        """Each itinerary trip's choice, and each choice of mode; ``None`` without any.

        One row per transit, park-and-ride and bike-and-ride trip, and per trip choosing its mode
        (``modes``), by traveller and then by the order of their day. Columns: ``trip`` (its index
        in the run, where trips are grouped by traveller), ``traveller_id`` and ``trip_seq`` (its
        place in that traveller's day, from 0), ``user_class`` (the traveller's class),
        ``mode`` (the mode taken; ``None`` for a trip choosing its mode that had nothing to choose
        from), ``mode_choice`` (whether the trip chose its mode), ``parking_id`` (the parking used;
        ``None`` for plain transit or a trip that did not travel), ``direction`` (``"out"``: vehicle
        first; ``"back"``: transit first, to where the vehicle is; ``""`` otherwise),
        ``alternatives`` (how many it chose among when it last chose; 0: it did not travel),
        ``probability`` (what the model gave its choice), ``expected_s`` (the chosen itinerary's
        door-to-door time expected at the choice), ``rides`` (vehicles boarded) and ``logsum``
        (the **logsum** of the trip's choice set when it was last planned in full — its
        first choice and each time it chose again; a trip that kept its alternative keeps it —
        the expected
        utility of its best alternative, ``ln Σ exp(utility)`` for the logit and its nested form
        for the nested logit; ``nan`` for a model that gives none; a model of one's own may give
        one with a ``logsum(batch)`` method, one number per situation). In the utility's units:
        divided by ``-beta_time_min`` it reads in minutes. It is the standard utility-based
        accessibility measure: compare a trip's logsum between two scenarios, or average it by
        class or by origin zone.
        """
        choices = self._summary.itinerary_choices
        return None if choices is None else Columns(choices)

    def logsum_od(
        self,
        trips: Any,
        zones: Mapping[str, tuple[float, float]] | None = None,
        *,
        by_class: bool = False,
    ) -> Columns:
        """The trips' logsums by origin-destination pair: utility-based accessibility.

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
            within = self._car_snap_motorway_m
            origin = [snap(lon, lat, within) for lon, lat in points[:, :2]]
            destination = [snap(lon, lat, within) for lon, lat in points[:, 2:]]
        groups: dict[tuple, list[float]] = {}
        for j, i in enumerate(keep):
            key = (origin[j], destination[j]) + ((choices["user_class"][i],) if by_class else ())
            w = people[keys[j]]
            g = groups.setdefault(key, [0.0, 0.0, 0.0])
            g[0] += 1
            g[1] += w
            g[2] += w * float(choices["logsum"][i])
        out = Columns(origin=[], destination=[])
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
        """A skim: seconds from each origin to each destination by ``mode``, after this run.

        The door-to-door time leaving at ``departure_s`` (seconds after midnight), by
        ``"car"`` — the earliest arrival on the **last loading's link times**, congestion by
        time bin included (free flow after a run of one loading; at ``flow_level=1`` the static
        equilibrium's times, the same all day), nearest drivable node to
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

    def report_spillback(self, min_excess_pcu: float = 5.0) -> Columns | None:
        """The links where a queue outgrew the road in the last loading.

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
        return Columns({name: values[keep] for name, values in s.items()})

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

    def route_changes(self) -> Columns:
        """Every reroute of the last loading, as columns (empty without rerouting).

        A vehicle whose next link is closed by a road disruption may re-route where it meets
        it, at the downstream end of its link (or at its origin, if the closed link is its first);
        at flow levels 3 and 4, so may one blocked at the downstream end of its link for
        ``reroute_after_s`` (``loading_options``). One row per reroute, in the order they
        happened: ``trip`` (its index in the run), ``traveller_id``, ``trip_seq``, ``second``,
        ``link`` (the link at whose downstream end it re-routed; for a trip re-routed at its
        origin, the closed first link it did not take), ``next_planned`` (the next link of its
        route until then), ``next_taken`` (of its new route) and ``reason`` (``"closed_ahead"``:
        its next link closed; ``"stuck"``: blocked for ``reroute_after_s``). Its planned route is
        its route choice (``route_choices``); what it actually took is in ``route_realised``.
        """
        return Columns(self._summary.route_changes)

    def route_realised(self) -> Columns:
        """The realised routes of the trips that re-routed in the last loading and arrived.

        ``trip`` (their indices) and ``links`` (for each, the link indices it took, in order).
        Every other trip followed its planned route, which nothing stores twice.
        """
        return Columns(self._summary.route_realised)

    def trip_results(self) -> Columns:
        """Every trip and what became of it, as columns: one row per trip, in trip order.

        The table to find the trips that did not finish, or to follow any trip: filter on
        ``outcome`` (``t = run.trip_results().to_pandas(); t[t.outcome == "truncated"]``). Also
        written as ``trip_results.parquet`` (with ``run_id``), which ``Run.open`` reads back.
        Columns:

        * ``trip`` (its index in the run, as in ``route_changes``), ``traveller_id``,
          ``trip_seq`` (its place in its traveller's day, from 0), ``user_class``, ``weight``
          (the people its traveller stands for);
        * ``origin_lon``, ``origin_lat``, ``destination_lon``, ``destination_lat`` (as given),
          ``departure_s`` (seconds after midnight);
        * ``mode`` (the mode taken, as in ``trip_modes``; ``None`` for a trip that chose and
          had none to choose) and ``mode_choice`` (whether it chose its mode);
        * ``outcome``: ``"completed"``, ``"truncated"`` (still travelling when the window
          ended), ``"no_vehicle_available"``, ``"no_feasible_path"``,
          ``"mode_not_available"`` or ``"no_alternative"`` (the trips ``report_no_alternative``
          lists, but for those that then went by car: a trip that chose its mode has ``mode``
          ``None``, one given transit, park-and-ride or bike-and-ride keeps it; ``completion``
          counts them under the outcome they had, most often ``no_feasible_path``);
        * ``arrival_s`` and ``travel_time_s``: ``NaN`` unless the trip completed.
          ``travel_time_s`` is door to door: the fixed times of the trip's own vehicle
          (``mode_options`` ``car_access_s``, ``car_egress_s``, ``bike_access_s``,
          ``bike_egress_s``; set to 0, the drive alone), its walks, waits and rides included;
        * ``vehicle_s``: the completed trip's seconds in a vehicle of its own (the car's drive,
          the bike's ride, in a direct trip or one through a parking; 0 without one), and
          ``access_egress_s``: its fixed seconds around that vehicle (reaching it, leaving it,
          its parking's time; 0 without one). ``NaN`` unless the trip completed; so
          ``travel_time_s - vehicle_s - access_egress_s`` is the trip's walking, waiting and
          riding in transit;
        * ``link_at_end``: for a trip still travelling when the window ended whose car was on
          the roads, the road link it was on (driving, or at the stop line) or waited at its
          origin to enter, an index into the network's links (as ``link_bins``'); ``NaN`` for
          every other trip, and for one on foot, on a bike, in a transit vehicle or at a stop
          at the end.

        The outcomes are the last loading's. Counted by ``outcome``, the trips agree with
        ``completion``, the no-alternative trips aside.
        """
        return Columns(self._summary.trip_results)

    def trip_modes(self) -> Columns:
        """Every trip's departure and the mode it took, as columns.

        One row per trip, by traveller and then by the order of their day: ``traveller_id``,
        ``trip_seq`` (its place in that traveller's day, from 0), ``departure_s``, ``mode``
        (the stated mode, a trip without one counting as a car trip; with ``modes``, the
        mode taken, ``None`` for a trip that had nothing to choose from), ``mode_choice``
        (whether the trip chose its mode) and ``weight`` (the people its traveller stands
        for). A trip that could not travel keeps its mode. ``viz.chart_mode_share`` draws it.
        """
        return Columns(self._summary.trip_modes)

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
        """The mean travel time of the completed trips, in seconds, per person.

        ``total_travel_time_s`` over the people the completed trips stand for (their travellers'
        weights). Dividing by ``completion["completed"]`` instead counts simulated trips, which
        differs from people when travellers stand for more than one (``default_weight``,
        ``demand_sample``). ``nan`` if no trip completed. Door to door (every trip's fixed times,
        walks, waits and rides included); ``mean_vehicle_time_s`` is the part spent in the
        trips' own vehicles.
        """
        people = sum(row["completed_people"] for row in self.completion_by_mode.values())
        return self.total_travel_time_s / people if people > 0 else float("nan")

    @property
    def total_vehicle_time_s(self) -> float:
        """The completed trips' time in their own vehicles, in seconds, scaled by traveller weight.

        The car's drive and the bike's ride, in a direct trip or one through a parking: what
        ``total_travel_time_s`` holds less the fixed times (``mode_options`` ``car_access_s`` and
        the like, the parkings' times), the walks, the waits and the transit rides. With the
        fixed times at 0, a run of car trips alone has the two equal.
        """
        return self._summary.total_vehicle_time_s

    @property
    def mean_vehicle_time_s(self) -> float:
        """The mean time in their own vehicles of the completed trips, in seconds, per person.

        ``total_vehicle_time_s`` over the people the completed trips stand for, as
        ``mean_travel_time_s``; the trips without a vehicle of their own count with 0. ``nan``
        if no trip completed.
        """
        people = sum(row["completed_people"] for row in self.completion_by_mode.values())
        return self.total_vehicle_time_s / people if people > 0 else float("nan")

    def __repr__(self) -> str:
        """The completion counts, as a one-line summary."""
        c = self.completion
        return (
            f"Run(completed={c['completed']}, truncated={c['truncated']}, "
            f"no_vehicle_available={c['no_vehicle_available']}, "
            f"no_feasible_path={c['no_feasible_path']})"
        )
