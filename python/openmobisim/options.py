"""Typed options (D-5 a): every parameter of a group as a keyword with its default.

Generated from the core's parameter table by ``python/tests/test_options.py --write``; do not edit
by hand. Each class is a read-only ``dict`` of the options given, so it goes wherever the options
dict goes: ``Scenario.from_parts(..., loading_options=LoadingOptions(reroute=1))`` is
``loading_options={"reroute": 1}``. An editor completes the names and shows the defaults, and a
misspelt name fails where it is written. A value equal to its default is left out (it changes
nothing); a field whose default is a rule of the run's level is ``None`` until set.
``parameters()`` lists every one with its unit, and the manual explains each.
"""

from __future__ import annotations

from typing import Any, ClassVar

__all__ = [
    "BestResponseOptions",
    "FreeFlowOptions",
    "LinkClassOptions",
    "LoadingOptions",
    "LogitOptions",
    "ModeOptions",
    "MonteCarloOptions",
    "MsaOptions",
    "NestedLogitOptions",
    "NetworkOptions",
    "ParkingOptions",
    "PenaltyOptions",
    "PriceOptions",
    "TransitOptions",
]


class _Options(dict):
    """A read-only dict of options, built by a class's keywords."""

    _argument: ClassVar[str] = ""
    _defaults: ClassVar[dict[str, Any]] = {}

    def _keep(self, given: dict[str, Any]) -> None:
        for name, value in given.items():
            if value is not None and value != self._defaults[name]:
                dict.__setitem__(self, name, value)

    def __getattr__(self, name: str) -> Any:
        if name in self._defaults:
            return self.get(name, self._defaults[name])
        raise AttributeError(f"{type(self).__name__} has no option {name!r}")

    def _read_only(self, *args: Any, **kwargs: Any) -> None:
        raise TypeError(f"{type(self).__name__} is read-only: make a new one")

    __setitem__ = __delitem__ = _read_only  # type: ignore[assignment]
    clear = pop = popitem = setdefault = update = _read_only  # type: ignore[assignment]

    def __repr__(self) -> str:
        given = ", ".join(f"{k}={v!r}" for k, v in self.items())
        return f"{type(self).__name__}({given})"


class LinkClassOptions(_Options):
    """One road class's parameters, for ``NetworkOptions`` (``motorway=LinkClassOptions(lanes=3)``).

    Each is ``None`` by default: the class's own shipped value, which ``parameters()`` lists (as
    ``motorway.lanes`` …). A class without one of them refuses it when the network is read.

    Attributes:
        free_flow_km_h: km/h.
        jam_density_veh_km_lane: veh/km per lane.
        lanes: count.
        saturation_flow_veh_h_lane: veh/h per lane.
    """

    _defaults: ClassVar[dict[str, Any]] = {
        "free_flow_km_h": None,
        "jam_density_veh_km_lane": None,
        "lanes": None,
        "saturation_flow_veh_h_lane": None,
    }

    def __init__(
        self,
        *,
        free_flow_km_h: float | None = None,
        jam_density_veh_km_lane: float | None = None,
        lanes: int | None = None,
        saturation_flow_veh_h_lane: float | None = None,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class NetworkOptions(_Options):
    """The network readers' parameters: ``network_options``.

    Attributes:
        bike_dedicated_km_h: km/h; ``18.0``.
        bike_lane_cost_factor: —; ``1.0``.
        bike_mixed_cost_factor: —; ``1.2``.
        bike_mixed_km_h: km/h; ``15.0``.
        capacity_factor: —; ``1.0``.
        control_delay_factor: —; ``1.0``.
        ferry_km_h: km/h; ``10.0``.
        ferry_wait_s: s; ``300.0``.
        free_flow_speed_factor: —; ``1.0``.
        green_fraction_factor: —; ``1.0``.
        jam_density_factor: —; ``1.0``.
        signal_cycle_s: s; ``90.0``.
        signal_degree_of_saturation: —; ``0.85``.
        signal_green_fraction: —; ``0.48``.
        signal_merge_m: m; ``25.0``.
        walk_km_h: km/h; ``4.8``.
        busway: the class's ``LinkClassOptions``; its own values by default.
        cycleway: the class's ``LinkClassOptions``; its own values by default.
        ferry: the class's ``LinkClassOptions``; its own values by default.
        footway: the class's ``LinkClassOptions``; its own values by default.
        living_street: the class's ``LinkClassOptions``; its own values by default.
        motorway: the class's ``LinkClassOptions``; its own values by default.
        motorway_link: the class's ``LinkClassOptions``; its own values by default.
        pedestrian: the class's ``LinkClassOptions``; its own values by default.
        primary: the class's ``LinkClassOptions``; its own values by default.
        primary_link: the class's ``LinkClassOptions``; its own values by default.
        residential: the class's ``LinkClassOptions``; its own values by default.
        secondary: the class's ``LinkClassOptions``; its own values by default.
        secondary_link: the class's ``LinkClassOptions``; its own values by default.
        service: the class's ``LinkClassOptions``; its own values by default.
        street_30: the class's ``LinkClassOptions``; its own values by default.
        street_50: the class's ``LinkClassOptions``; its own values by default.
        street_70: the class's ``LinkClassOptions``; its own values by default.
        tertiary: the class's ``LinkClassOptions``; its own values by default.
        tertiary_link: the class's ``LinkClassOptions``; its own values by default.
        trunk: the class's ``LinkClassOptions``; its own values by default.
        trunk_link: the class's ``LinkClassOptions``; its own values by default.
        unclassified: the class's ``LinkClassOptions``; its own values by default.
    """

    _argument: ClassVar[str] = "network_options"
    _link_classes: ClassVar[tuple[str, ...]] = (
        "busway",
        "cycleway",
        "ferry",
        "footway",
        "living_street",
        "motorway",
        "motorway_link",
        "pedestrian",
        "primary",
        "primary_link",
        "residential",
        "secondary",
        "secondary_link",
        "service",
        "street_30",
        "street_50",
        "street_70",
        "tertiary",
        "tertiary_link",
        "trunk",
        "trunk_link",
        "unclassified",
    )
    _defaults: ClassVar[dict[str, Any]] = {
        "bike_dedicated_km_h": 18.0,
        "bike_lane_cost_factor": 1.0,
        "bike_mixed_cost_factor": 1.2,
        "bike_mixed_km_h": 15.0,
        "capacity_factor": 1.0,
        "control_delay_factor": 1.0,
        "ferry_km_h": 10.0,
        "ferry_wait_s": 300.0,
        "free_flow_speed_factor": 1.0,
        "green_fraction_factor": 1.0,
        "jam_density_factor": 1.0,
        "signal_cycle_s": 90.0,
        "signal_degree_of_saturation": 0.85,
        "signal_green_fraction": 0.48,
        "signal_merge_m": 25.0,
        "walk_km_h": 4.8,
        "busway": None,
        "cycleway": None,
        "ferry": None,
        "footway": None,
        "living_street": None,
        "motorway": None,
        "motorway_link": None,
        "pedestrian": None,
        "primary": None,
        "primary_link": None,
        "residential": None,
        "secondary": None,
        "secondary_link": None,
        "service": None,
        "street_30": None,
        "street_50": None,
        "street_70": None,
        "tertiary": None,
        "tertiary_link": None,
        "trunk": None,
        "trunk_link": None,
        "unclassified": None,
    }

    def __init__(
        self,
        *,
        bike_dedicated_km_h: float = 18.0,
        bike_lane_cost_factor: float = 1.0,
        bike_mixed_cost_factor: float = 1.2,
        bike_mixed_km_h: float = 15.0,
        capacity_factor: float = 1.0,
        control_delay_factor: float = 1.0,
        ferry_km_h: float = 10.0,
        ferry_wait_s: float = 300.0,
        free_flow_speed_factor: float = 1.0,
        green_fraction_factor: float = 1.0,
        jam_density_factor: float = 1.0,
        signal_cycle_s: float = 90.0,
        signal_degree_of_saturation: float = 0.85,
        signal_green_fraction: float = 0.48,
        signal_merge_m: float = 25.0,
        walk_km_h: float = 4.8,
        busway: LinkClassOptions | None = None,
        cycleway: LinkClassOptions | None = None,
        ferry: LinkClassOptions | None = None,
        footway: LinkClassOptions | None = None,
        living_street: LinkClassOptions | None = None,
        motorway: LinkClassOptions | None = None,
        motorway_link: LinkClassOptions | None = None,
        pedestrian: LinkClassOptions | None = None,
        primary: LinkClassOptions | None = None,
        primary_link: LinkClassOptions | None = None,
        residential: LinkClassOptions | None = None,
        secondary: LinkClassOptions | None = None,
        secondary_link: LinkClassOptions | None = None,
        service: LinkClassOptions | None = None,
        street_30: LinkClassOptions | None = None,
        street_50: LinkClassOptions | None = None,
        street_70: LinkClassOptions | None = None,
        tertiary: LinkClassOptions | None = None,
        tertiary_link: LinkClassOptions | None = None,
        trunk: LinkClassOptions | None = None,
        trunk_link: LinkClassOptions | None = None,
        unclassified: LinkClassOptions | None = None,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        for link_class in self._link_classes:
            for name, value in (given.pop(link_class) or {}).items():
                dict.__setitem__(self, f"{link_class}.{name}", value)
        self._keep(given)


class LoadingOptions(_Options):
    """The loading's parameters: ``loading_options``.

    Attributes:
        node_inflow: 0 or 1; ``1``.
        origin_inflow: 0 or 1; ``0``.
        pocket_length_m: m; ``50.0``.
        priority: 0 or 1; ``0``.
        reroute: 0 or 1; by default ``None``: on at flow_level 3 and 4, and at 2 in a loading
            with road disruptions; off at 2 otherwise unless asked for (1).
        reroute_after_s: s; ``300.0``.
        reroute_max: count; ``3``.
        reroute_min_gain: —; ``0.1``.
    """

    _argument: ClassVar[str] = "loading_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "node_inflow": 1,
        "origin_inflow": 0,
        "pocket_length_m": 50.0,
        "priority": 0,
        "reroute": None,
        "reroute_after_s": 300.0,
        "reroute_max": 3,
        "reroute_min_gain": 0.1,
    }

    def __init__(
        self,
        *,
        node_inflow: int = 1,
        origin_inflow: int = 0,
        pocket_length_m: float = 50.0,
        priority: int = 0,
        reroute: int | None = None,
        reroute_after_s: float = 300.0,
        reroute_max: int = 3,
        reroute_min_gain: float = 0.1,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class ModeOptions(_Options):
    """The modes' parameters: ``mode_options``.

    Attributes:
        bike_max_s: s; ``3600.0``.
        car_access_s: s; ``120.0``.
        car_egress_s: s; ``180.0``.
        car_snap_motorway_m: m; ``50.0``.
        fallback_car: 0 or 1; ``0``.
        walk_max_s: s; ``1800.0``.
    """

    _argument: ClassVar[str] = "mode_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "bike_max_s": 3600.0,
        "car_access_s": 120.0,
        "car_egress_s": 180.0,
        "car_snap_motorway_m": 50.0,
        "fallback_car": 0,
        "walk_max_s": 1800.0,
    }

    def __init__(
        self,
        *,
        bike_max_s: float = 3600.0,
        car_access_s: float = 120.0,
        car_egress_s: float = 180.0,
        car_snap_motorway_m: float = 50.0,
        fallback_car: int = 0,
        walk_max_s: float = 1800.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class TransitOptions(_Options):
    """Public transport's parameters: ``transit_options``.

    Attributes:
        access_walk_max_s: s; ``1800.0``.
        board_slack_s: s; ``60.0``.
        bus_dwell_s: s; ``20.0``.
        bus_pcu: PCU; ``2.0``.
        bus_plausibility_ratio: —; ``1.6``.
        bus_stop_snap_m: m; ``300.0``.
        journey_max_s: s; ``7200.0``.
        max_rides: count; ``8``.
        stop_transfer_s: s; ``0.0``.
        stop_walk_snap_m: m; ``300.0``.
        transfer_walk_max_s: s; ``300.0``.
    """

    _argument: ClassVar[str] = "transit_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "access_walk_max_s": 1800.0,
        "board_slack_s": 60.0,
        "bus_dwell_s": 20.0,
        "bus_pcu": 2.0,
        "bus_plausibility_ratio": 1.6,
        "bus_stop_snap_m": 300.0,
        "journey_max_s": 7200.0,
        "max_rides": 8,
        "stop_transfer_s": 0.0,
        "stop_walk_snap_m": 300.0,
        "transfer_walk_max_s": 300.0,
    }

    def __init__(
        self,
        *,
        access_walk_max_s: float = 1800.0,
        board_slack_s: float = 60.0,
        bus_dwell_s: float = 20.0,
        bus_pcu: float = 2.0,
        bus_plausibility_ratio: float = 1.6,
        bus_stop_snap_m: float = 300.0,
        journey_max_s: float = 7200.0,
        max_rides: int = 8,
        stop_transfer_s: float = 0.0,
        stop_walk_snap_m: float = 300.0,
        transfer_walk_max_s: float = 300.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class ParkingOptions(_Options):
    """The parkings' parameters: ``parking_options``.

    Attributes:
        bin_s: s; ``900.0``.
        candidates: count; ``3``.
        floor_bike_s: s; ``30.0``.
        floor_car_s: s; ``120.0``.
        pr_min_km: km; ``3.0``.
        rank_speed_km_h: km/h; ``30.0``.
        reach_bike_s: s; ``1200.0``.
        reach_car_s: s; ``1800.0``.
        reach_car_step_s: s; ``0.0``.
        slope_bike_s: s; ``180.0``.
        slope_car_s: s; ``600.0``.
        snap_m: m; ``300.0``.
        walk_max_s: s; ``300.0``.
    """

    _argument: ClassVar[str] = "parking_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "bin_s": 900.0,
        "candidates": 3,
        "floor_bike_s": 30.0,
        "floor_car_s": 120.0,
        "pr_min_km": 3.0,
        "rank_speed_km_h": 30.0,
        "reach_bike_s": 1200.0,
        "reach_car_s": 1800.0,
        "reach_car_step_s": 0.0,
        "slope_bike_s": 180.0,
        "slope_car_s": 600.0,
        "snap_m": 300.0,
        "walk_max_s": 300.0,
    }

    def __init__(
        self,
        *,
        bin_s: float = 900.0,
        candidates: int = 3,
        floor_bike_s: float = 30.0,
        floor_car_s: float = 120.0,
        pr_min_km: float = 3.0,
        rank_speed_km_h: float = 30.0,
        reach_bike_s: float = 1200.0,
        reach_car_s: float = 1800.0,
        reach_car_step_s: float = 0.0,
        slope_bike_s: float = 180.0,
        slope_car_s: float = 600.0,
        snap_m: float = 300.0,
        walk_max_s: float = 300.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class PriceOptions(_Options):
    """The prices: ``price_options``.

    Attributes:
        bike_eur_km: EUR/km; ``0.0``.
        car_eur_km: EUR/km; ``0.0``.
        fare_base_eur: EUR; ``0.0``.
        fare_km_eur: EUR/km; ``0.0``.
        fare_transfer_eur: EUR; ``0.0``.
        parking_bike_eur: EUR; ``0.0``.
        parking_car_eur: EUR; ``0.0``.
    """

    _argument: ClassVar[str] = "price_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "bike_eur_km": 0.0,
        "car_eur_km": 0.0,
        "fare_base_eur": 0.0,
        "fare_km_eur": 0.0,
        "fare_transfer_eur": 0.0,
        "parking_bike_eur": 0.0,
        "parking_car_eur": 0.0,
    }

    def __init__(
        self,
        *,
        bike_eur_km: float = 0.0,
        car_eur_km: float = 0.0,
        fare_base_eur: float = 0.0,
        fare_km_eur: float = 0.0,
        fare_transfer_eur: float = 0.0,
        parking_bike_eur: float = 0.0,
        parking_car_eur: float = 0.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class MsaOptions(_Options):
    """``equilibration="msa"``: ``equilibration_options``.

    Attributes:
        cost_bin_s: s; ``300.0``.
        gap_sample: count; ``300``.
        gap_tolerance: —; ``0.05``.
        increments: count; by default ``None``: 1 at flow_level 0 whatever is set.
        iterations: count; ``10``.
        itinerary_gap_sample: count; ``100000000``.
        route_growth_tolerance: —; ``1.0``.
        warmup: count; ``1``.
    """

    _argument: ClassVar[str] = "equilibration_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "cost_bin_s": 300.0,
        "gap_sample": 300,
        "gap_tolerance": 0.05,
        "increments": None,
        "iterations": 10,
        "itinerary_gap_sample": 100000000,
        "route_growth_tolerance": 1.0,
        "warmup": 1,
    }

    def __init__(
        self,
        *,
        cost_bin_s: float = 300.0,
        gap_sample: int = 300,
        gap_tolerance: float = 0.05,
        increments: int | None = None,
        iterations: int = 10,
        itinerary_gap_sample: int = 100000000,
        route_growth_tolerance: float = 1.0,
        warmup: int = 1,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class FreeFlowOptions(_Options):
    """``equilibration="free_flow"``: ``equilibration_options``.

    Attributes:
        increments: count; by default ``None``: 1 at flow_level 0 whatever is set.
        warmup: count; ``1``.
    """

    _argument: ClassVar[str] = "equilibration_options"
    _defaults: ClassVar[dict[str, Any]] = {"increments": None, "warmup": 1}

    def __init__(
        self,
        *,
        increments: int | None = None,
        warmup: int = 1,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class PenaltyOptions(_Options):
    """``route_method="penalty"``: ``route_options``.

    Attributes:
        max_attempts: count; ``15``.
        max_detour: —; ``1.3``.
        max_overlap: —; ``0.75``.
        max_paths: count; ``5``.
        penalty: —; ``1.5``.
    """

    _argument: ClassVar[str] = "route_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "max_attempts": 15,
        "max_detour": 1.3,
        "max_overlap": 0.75,
        "max_paths": 5,
        "penalty": 1.5,
    }

    def __init__(
        self,
        *,
        max_attempts: int = 15,
        max_detour: float = 1.3,
        max_overlap: float = 0.75,
        max_paths: int = 5,
        penalty: float = 1.5,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class MonteCarloOptions(_Options):
    """``route_method="montecarlo"``: ``route_options``.

    Attributes:
        biased: 0 or 1; ``1``.
        draws: count; ``16``.
        max_detour: —; ``2.0``.
        max_overlap: —; ``0.9``.
        max_paths: count; ``10``.
        seed: count; ``0``.
        sigma: —; ``4.0``.
    """

    _argument: ClassVar[str] = "route_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "biased": 1,
        "draws": 16,
        "max_detour": 2.0,
        "max_overlap": 0.9,
        "max_paths": 10,
        "seed": 0,
        "sigma": 4.0,
    }

    def __init__(
        self,
        *,
        biased: int = 1,
        draws: int = 16,
        max_detour: float = 2.0,
        max_overlap: float = 0.9,
        max_paths: int = 10,
        seed: int = 0,
        sigma: float = 4.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class BestResponseOptions(_Options):
    """``route_update="best_response"``: ``route_update_options``.

    Attributes:
        max_routes: count; ``10``.
        searches: count; ``1``.
        slack: —; ``0.02``.
        tree_step_s: s; ``0.0``.
    """

    _argument: ClassVar[str] = "route_update_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "max_routes": 10,
        "searches": 1,
        "slack": 0.02,
        "tree_step_s": 0.0,
    }

    def __init__(
        self,
        *,
        max_routes: int = 10,
        searches: int = 1,
        slack: float = 0.02,
        tree_step_s: float = 0.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class LogitOptions(_Options):
    """``choice_model="logit"``: ``choice_options``.

    Attributes:
        beta_bike_lane_km: per km; ``0.0``.
        beta_bike_min: per min; ``0.0``.
        beta_bike_mixed_km: per km; ``-0.16``.
        beta_bike_separated_km: per km; ``0.0``.
        beta_car_min: per min; ``0.0``.
        beta_cost_eur: per EUR; ``-1.2``.
        beta_cost_fare_eur: per EUR; ``0.0``.
        beta_cost_parking_eur: per EUR; ``0.0``.
        beta_cost_running_eur: per EUR; ``0.0``.
        beta_cost_toll_eur: per EUR; ``0.0``.
        beta_detour: per unit of the attribute; ``0.0``.
        beta_length_km: per km; ``0.0``.
        beta_ln_path_size: per unit of the attribute; ``1.0``.
        beta_mode_bike: per unit of the attribute; ``0.0``.
        beta_mode_bike_transit: per unit of the attribute; ``0.0``.
        beta_mode_car: per unit of the attribute; ``0.0``.
        beta_mode_car_transit: per unit of the attribute; ``0.0``.
        beta_mode_transit: per unit of the attribute; ``0.0``.
        beta_mode_walk: per unit of the attribute; ``0.0``.
        beta_n_links: per unit of the attribute; ``0.0``.
        beta_overlap: per unit of the attribute; ``0.0``.
        beta_parking_min: per min; ``0.0``.
        beta_ride_bus_min: per min; ``0.0``.
        beta_ride_ferry_min: per min; ``0.0``.
        beta_ride_metro_min: per min; ``0.0``.
        beta_ride_min: per min; ``0.0``.
        beta_ride_other_min: per min; ``0.0``.
        beta_ride_rail_min: per min; ``0.0``.
        beta_ride_tram_min: per min; ``0.0``.
        beta_time_min: per min; ``-0.2``.
        beta_transfers: per unit of the attribute; ``-1.0``.
        beta_trip_departure_h: per unit of the attribute; ``0.0``.
        beta_wait_first_min: per min; ``0.0``.
        beta_wait_min: per min; ``-0.09``.
        beta_wait_transfer_min: per min; ``0.0``.
        beta_walk_access_min: per min; ``0.0``.
        beta_walk_egress_min: per min; ``0.0``.
        beta_walk_min: per min; ``-0.13``.
        beta_walk_transfer_min: per min; ``0.0``.
    """

    _argument: ClassVar[str] = "choice_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "beta_bike_lane_km": 0.0,
        "beta_bike_min": 0.0,
        "beta_bike_mixed_km": -0.16,
        "beta_bike_separated_km": 0.0,
        "beta_car_min": 0.0,
        "beta_cost_eur": -1.2,
        "beta_cost_fare_eur": 0.0,
        "beta_cost_parking_eur": 0.0,
        "beta_cost_running_eur": 0.0,
        "beta_cost_toll_eur": 0.0,
        "beta_detour": 0.0,
        "beta_length_km": 0.0,
        "beta_ln_path_size": 1.0,
        "beta_mode_bike": 0.0,
        "beta_mode_bike_transit": 0.0,
        "beta_mode_car": 0.0,
        "beta_mode_car_transit": 0.0,
        "beta_mode_transit": 0.0,
        "beta_mode_walk": 0.0,
        "beta_n_links": 0.0,
        "beta_overlap": 0.0,
        "beta_parking_min": 0.0,
        "beta_ride_bus_min": 0.0,
        "beta_ride_ferry_min": 0.0,
        "beta_ride_metro_min": 0.0,
        "beta_ride_min": 0.0,
        "beta_ride_other_min": 0.0,
        "beta_ride_rail_min": 0.0,
        "beta_ride_tram_min": 0.0,
        "beta_time_min": -0.2,
        "beta_transfers": -1.0,
        "beta_trip_departure_h": 0.0,
        "beta_wait_first_min": 0.0,
        "beta_wait_min": -0.09,
        "beta_wait_transfer_min": 0.0,
        "beta_walk_access_min": 0.0,
        "beta_walk_egress_min": 0.0,
        "beta_walk_min": -0.13,
        "beta_walk_transfer_min": 0.0,
    }

    def __init__(
        self,
        *,
        beta_bike_lane_km: float = 0.0,
        beta_bike_min: float = 0.0,
        beta_bike_mixed_km: float = -0.16,
        beta_bike_separated_km: float = 0.0,
        beta_car_min: float = 0.0,
        beta_cost_eur: float = -1.2,
        beta_cost_fare_eur: float = 0.0,
        beta_cost_parking_eur: float = 0.0,
        beta_cost_running_eur: float = 0.0,
        beta_cost_toll_eur: float = 0.0,
        beta_detour: float = 0.0,
        beta_length_km: float = 0.0,
        beta_ln_path_size: float = 1.0,
        beta_mode_bike: float = 0.0,
        beta_mode_bike_transit: float = 0.0,
        beta_mode_car: float = 0.0,
        beta_mode_car_transit: float = 0.0,
        beta_mode_transit: float = 0.0,
        beta_mode_walk: float = 0.0,
        beta_n_links: float = 0.0,
        beta_overlap: float = 0.0,
        beta_parking_min: float = 0.0,
        beta_ride_bus_min: float = 0.0,
        beta_ride_ferry_min: float = 0.0,
        beta_ride_metro_min: float = 0.0,
        beta_ride_min: float = 0.0,
        beta_ride_other_min: float = 0.0,
        beta_ride_rail_min: float = 0.0,
        beta_ride_tram_min: float = 0.0,
        beta_time_min: float = -0.2,
        beta_transfers: float = -1.0,
        beta_trip_departure_h: float = 0.0,
        beta_wait_first_min: float = 0.0,
        beta_wait_min: float = -0.09,
        beta_wait_transfer_min: float = 0.0,
        beta_walk_access_min: float = 0.0,
        beta_walk_egress_min: float = 0.0,
        beta_walk_min: float = -0.13,
        beta_walk_transfer_min: float = 0.0,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)


class NestedLogitOptions(_Options):
    """``choice_model="nested_logit"``: ``choice_options``.

    Attributes:
        beta_bike_lane_km: per km; ``0.0``.
        beta_bike_min: per min; ``0.0``.
        beta_bike_mixed_km: per km; ``-0.16``.
        beta_bike_separated_km: per km; ``0.0``.
        beta_car_min: per min; ``0.0``.
        beta_cost_eur: per EUR; ``-1.2``.
        beta_cost_fare_eur: per EUR; ``0.0``.
        beta_cost_parking_eur: per EUR; ``0.0``.
        beta_cost_running_eur: per EUR; ``0.0``.
        beta_cost_toll_eur: per EUR; ``0.0``.
        beta_detour: per unit of the attribute; ``0.0``.
        beta_length_km: per km; ``0.0``.
        beta_ln_path_size: per unit of the attribute; ``1.0``.
        beta_mode_bike: per unit of the attribute; ``0.0``.
        beta_mode_bike_transit: per unit of the attribute; ``0.0``.
        beta_mode_car: per unit of the attribute; ``0.0``.
        beta_mode_car_transit: per unit of the attribute; ``0.0``.
        beta_mode_transit: per unit of the attribute; ``0.0``.
        beta_mode_walk: per unit of the attribute; ``0.0``.
        beta_n_links: per unit of the attribute; ``0.0``.
        beta_overlap: per unit of the attribute; ``0.0``.
        beta_parking_min: per min; ``0.0``.
        beta_ride_bus_min: per min; ``0.0``.
        beta_ride_ferry_min: per min; ``0.0``.
        beta_ride_metro_min: per min; ``0.0``.
        beta_ride_min: per min; ``0.0``.
        beta_ride_other_min: per min; ``0.0``.
        beta_ride_rail_min: per min; ``0.0``.
        beta_ride_tram_min: per min; ``0.0``.
        beta_time_min: per min; ``-0.2``.
        beta_transfers: per unit of the attribute; ``-1.0``.
        beta_trip_departure_h: per unit of the attribute; ``0.0``.
        beta_wait_first_min: per min; ``0.0``.
        beta_wait_min: per min; ``-0.09``.
        beta_wait_transfer_min: per min; ``0.0``.
        beta_walk_access_min: per min; ``0.0``.
        beta_walk_egress_min: per min; ``0.0``.
        beta_walk_min: per min; ``-0.13``.
        beta_walk_transfer_min: per min; ``0.0``.
        mu: —; ``0.5``.
    """

    _argument: ClassVar[str] = "choice_options"
    _defaults: ClassVar[dict[str, Any]] = {
        "beta_bike_lane_km": 0.0,
        "beta_bike_min": 0.0,
        "beta_bike_mixed_km": -0.16,
        "beta_bike_separated_km": 0.0,
        "beta_car_min": 0.0,
        "beta_cost_eur": -1.2,
        "beta_cost_fare_eur": 0.0,
        "beta_cost_parking_eur": 0.0,
        "beta_cost_running_eur": 0.0,
        "beta_cost_toll_eur": 0.0,
        "beta_detour": 0.0,
        "beta_length_km": 0.0,
        "beta_ln_path_size": 1.0,
        "beta_mode_bike": 0.0,
        "beta_mode_bike_transit": 0.0,
        "beta_mode_car": 0.0,
        "beta_mode_car_transit": 0.0,
        "beta_mode_transit": 0.0,
        "beta_mode_walk": 0.0,
        "beta_n_links": 0.0,
        "beta_overlap": 0.0,
        "beta_parking_min": 0.0,
        "beta_ride_bus_min": 0.0,
        "beta_ride_ferry_min": 0.0,
        "beta_ride_metro_min": 0.0,
        "beta_ride_min": 0.0,
        "beta_ride_other_min": 0.0,
        "beta_ride_rail_min": 0.0,
        "beta_ride_tram_min": 0.0,
        "beta_time_min": -0.2,
        "beta_transfers": -1.0,
        "beta_trip_departure_h": 0.0,
        "beta_wait_first_min": 0.0,
        "beta_wait_min": -0.09,
        "beta_wait_transfer_min": 0.0,
        "beta_walk_access_min": 0.0,
        "beta_walk_egress_min": 0.0,
        "beta_walk_min": -0.13,
        "beta_walk_transfer_min": 0.0,
        "mu": 0.5,
    }

    def __init__(
        self,
        *,
        beta_bike_lane_km: float = 0.0,
        beta_bike_min: float = 0.0,
        beta_bike_mixed_km: float = -0.16,
        beta_bike_separated_km: float = 0.0,
        beta_car_min: float = 0.0,
        beta_cost_eur: float = -1.2,
        beta_cost_fare_eur: float = 0.0,
        beta_cost_parking_eur: float = 0.0,
        beta_cost_running_eur: float = 0.0,
        beta_cost_toll_eur: float = 0.0,
        beta_detour: float = 0.0,
        beta_length_km: float = 0.0,
        beta_ln_path_size: float = 1.0,
        beta_mode_bike: float = 0.0,
        beta_mode_bike_transit: float = 0.0,
        beta_mode_car: float = 0.0,
        beta_mode_car_transit: float = 0.0,
        beta_mode_transit: float = 0.0,
        beta_mode_walk: float = 0.0,
        beta_n_links: float = 0.0,
        beta_overlap: float = 0.0,
        beta_parking_min: float = 0.0,
        beta_ride_bus_min: float = 0.0,
        beta_ride_ferry_min: float = 0.0,
        beta_ride_metro_min: float = 0.0,
        beta_ride_min: float = 0.0,
        beta_ride_other_min: float = 0.0,
        beta_ride_rail_min: float = 0.0,
        beta_ride_tram_min: float = 0.0,
        beta_time_min: float = -0.2,
        beta_transfers: float = -1.0,
        beta_trip_departure_h: float = 0.0,
        beta_wait_first_min: float = 0.0,
        beta_wait_min: float = -0.09,
        beta_wait_transfer_min: float = 0.0,
        beta_walk_access_min: float = 0.0,
        beta_walk_egress_min: float = 0.0,
        beta_walk_min: float = -0.13,
        beta_walk_transfer_min: float = 0.0,
        mu: float = 0.5,
    ) -> None:
        """Keep the options given that differ from their defaults."""
        given = dict(locals())
        del given["self"]
        self._keep(given)
