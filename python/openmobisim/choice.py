"""Choice models: what to write to add your own.

A *choice model* says which alternative each traveller takes: a route among the
routes of the trip's origin-destination pair, an itinerary (a transit journey, a
park-and-ride or bike-and-ride chain), or, with mode choice, a mode together with its
route or itinerary. The alternatives of every kind carry the same named attributes, so
one model serves them all. openmobisim ships three models, selected by name
(``choice_models()``):

* ``"deterministic"``: everyone takes the alternative with the least travel time.
  All-or-nothing; no randomness. Useful for debugging or an upper bound.
* ``"logit"`` (``Scenario.from_parts``'s default when the run offers one mode): a
  random utility model, linear in named attributes and sampled exactly (Gumbel-max).
  With its defaults it is the standard **path-size logit** for route choice:
  ``U = -0.2 · time_min + 1 · ln_path_size``. Set any coefficient with
  ``choice_options``: ``{"beta_time_min": -0.3, "beta_length_km": -0.1}``. **The
  defaults are an assumption, not a calibration**; pass estimated coefficients for
  real work.
* ``"nested_logit"`` (the default when the run offers more than one mode): the same
  utility, with the alternatives of each mode in a nest of their own (``mu``, 0.5), so
  a mode's share does not grow with the number of its routes or journeys.

**Your own model** is any object with a ``choose(batch)`` method (no base class
needed), passed as ``Scenario.from_parts(..., choice_model=my_model)``. It is
called once per batch of up to 32 768 trips and returns, for each situation,
the index of the alternative taken::

    import numpy as np
    from openmobisim import choice

    class ShortAndSimple:
        name = "short_and_simple"          # shown in the manifest
        descriptor = "short_and_simple;v1"  # anything that identifies your settings
        attributes = ["time_min", "length_km"]  # what you read (optional)

        def choose(self, batch):
            utility = -0.3 * batch.attributes["time_min"] - 0.1 * batch.attributes["length_km"]
            return choice.sample_random_utility(batch, utility)

What the ``batch`` holds (all NumPy arrays; ``a`` indexes alternatives, ``s``
situations, one situation being one traveller's one trip):

* ``batch.offsets`` — situation ``s`` owns alternatives ``offsets[s]:offsets[s+1]``.
* ``batch.attributes`` — a dict of float arrays, one value per alternative:
  ``time_min``, ``length_km``, ``detour``, ``overlap``, ``ln_path_size``,
  ``n_links``, ``car_min``, ``bike_min``, ``walk_min``, ``wait_min``,
  ``ride_min``, ``transfers``, ``parking_min``, the bike leg's kilometres by
  facility, a transit itinerary's walks, waits and rides split, and what the
  alternative costs in euros, ``cost_eur`` and its parts (see ``ROUTE_ATTRIBUTES``
  and ``Scenario.from_parts``' ``price_options``).
  Routes and the itineraries of transit, park-and-ride and bike-and-ride trips
  carry the same names, so one model serves both.
* ``batch.gumbel`` — each alternative's standard Gumbel error, keyed on
  (traveller, trip, iteration, alternative identity), so a model that adds it
  to a utility gets exactly the common random numbers the built-in models use.
* ``batch.traveller``, ``batch.trip`` (per situation), ``batch.identity``
  (per alternative; stable when other alternatives come and go),
  ``batch.situation_of`` (per alternative), ``batch.iteration``.
* The user's traveller and trip values (``Scenario.from_parts``' ``person_values`` and
  ``trip_values``) are attributes too, ``person_<name>`` and ``trip_<name>``: the
  same for every alternative of one situation.
* ``batch.user_class`` — each situation's traveller class, an index into
  ``batch.class_names`` (the demand's ``user_class`` values), so a model may
  weigh classes differently: ``batch.class_names[batch.user_class[s]]``.

Return the chosen indices **within each situation** (an integer array, one per
situation), or ``(indices, probabilities)`` when the model knows how likely each
choice was. A model may also have a ``probabilities(batch)`` method returning the
probability of **every** alternative (one number per alternative, summing to one
within each situation): it lets an iterated run (``equilibration="msa"``) measure
how far the pattern is from equilibrium; without it those measures are ``nan``.
And it may have a ``logsum(batch)`` method returning one number per situation, the
expected utility of its best alternative: ``Run.itinerary_choices()["logsum"]``,
for accessibility; without it that column is ``nan``.
Optional attributes of the model: ``name``, ``descriptor`` (both in
the manifest and part of the run's fingerprint: **change the descriptor when the
model's behaviour changes**, or two different models look like one run),
``sampled`` (``False`` if it never uses ``batch.gumbel``; default ``True``) and
``attributes`` (the ones it reads; default all).

The helpers below do the array bookkeeping so a model is a few lines.
"""

from __future__ import annotations

import contextlib
from collections.abc import Callable
from typing import Any, Protocol, runtime_checkable

import numpy as np

__all__ = [
    "ROUTE_ATTRIBUTES",
    "SERVICE_ATTRIBUTES",
    "TAXI_ATTRIBUTES",
    "ChoiceModel",
    "choice_model_register",
    "choice_models",
    "sample_random_utility",
    "segment_argmax",
    "segment_softmax",
]

#: The attributes every alternative carries — a route, or the itinerary of a transit,
#: park-and-ride or bike-and-ride trip — and what they mean: ``time_min`` expected
#: travel time in minutes (door to door for an itinerary); ``length_km`` (a route's; an
#: itinerary's car or bike leg's); ``detour`` (time over the best alternative's, minus
#: one); ``overlap`` (a route's largest share of its cost shared with an earlier route;
#: 0 for an itinerary); ``ln_path_size`` (log of the path-size factor: 0 for an
#: alternative that shares nothing, negative as it shares more); ``n_links`` (a
#: route's; an itinerary's car or bike leg's); ``car_min``, ``bike_min``, ``walk_min``,
#: ``wait_min`` (at stops), ``ride_min`` (in transit vehicles), ``transfers`` and
#: ``parking_min`` (parking or fetching the car or bike): the parts of the time, 0
#: where an alternative has none (a car route is all ``car_min``);
#: ``bike_separated_km``, ``bike_lane_km``, ``bike_mixed_km`` (the bike leg's
#: kilometres on separated tracks, painted lanes and in mixed traffic: what a model
#: sees of a ride's quality; a ferry crossing counts in none);
#: ``walk_access_min``, ``walk_egress_min``, ``walk_transfer_min`` (the walk to the
#: first stop, from the last, and between stops: parts of ``walk_min``),
#: ``wait_first_min``, ``wait_transfer_min`` (parts of ``wait_min``) and
#: ``ride_rail_min``, ``ride_metro_min``, ``ride_tram_min``, ``ride_bus_min``,
#: ``ride_ferry_min``, ``ride_other_min`` (minutes on board by kind of service: parts
#: of ``ride_min``); a part is weighed on top of its total, or instead of it;
#: ``cost_eur``, what the alternative costs in euros, and its parts
#: ``cost_running_eur`` (the car or bike leg's kilometres at the vehicle's price per km),
#: ``cost_toll_eur`` (the car leg's tolls: a road link value named ``toll_eur``),
#: ``cost_parking_eur`` (a park-and-ride or bike-and-ride parking's fee, on the trip that
#: parks, or the trip's destination's for one arriving by car) and ``cost_fare_eur`` (a
#: transit journey's fare); ``trip_departure_h`` (the trip's departure in hours after
#: midnight, the same for each alternative of a choice); ``nest``
#: (the alternative's mode as a number: the nest of ``"nested_logit"``) and
#: ``mode_walk``, ``mode_bike``, ``mode_car``, ``mode_transit``, ``mode_car_transit``,
#: ``mode_bike_transit`` (1 for the alternative's mode, 0 for the rest: a coefficient
#: on one is that mode's constant).
ROUTE_ATTRIBUTES: tuple[str, ...] = (
    "time_min",
    "length_km",
    "detour",
    "overlap",
    "ln_path_size",
    "n_links",
    "car_min",
    "bike_min",
    "walk_min",
    "wait_min",
    "ride_min",
    "transfers",
    "parking_min",
    "bike_separated_km",
    "bike_lane_km",
    "bike_mixed_km",
    "walk_access_min",
    "walk_egress_min",
    "walk_transfer_min",
    "wait_first_min",
    "wait_transfer_min",
    "ride_rail_min",
    "ride_metro_min",
    "ride_tram_min",
    "ride_bus_min",
    "ride_ferry_min",
    "ride_other_min",
    "cost_eur",
    "cost_running_eur",
    "cost_toll_eur",
    "cost_parking_eur",
    "cost_fare_eur",
    "trip_departure_h",
    "nest",
    "mode_walk",
    "mode_bike",
    "mode_car",
    "mode_transit",
    "mode_car_transit",
    "mode_bike_transit",
)


#: The attributes a shared bike adds, in a run with a docked bike service (``services``), after
#: ``ROUTE_ATTRIBUTES``: ``mode_bike_share`` (1 for a shared bike, 0 for the rest: its
#: constant), ``dock_min`` (the minutes of extra walk assumed to another dock when the pickup dock
#: is empty or the return dock full; part of ``walk_min``, 0 for the other modes) and
#: ``mode_bike_share_transit`` (1 for a shared bike to a stop's hub then transit). A run without
#: a service offers none.
SERVICE_ATTRIBUTES: tuple[str, ...] = ("mode_bike_share", "dock_min", "mode_bike_share_transit")

#: The attributes a taxi adds, in a run with a taxi fleet (``services``), after
#: ``ROUTE_ATTRIBUTES`` (and ``SERVICE_ATTRIBUTES`` with a docked bike service too): ``mode_taxi``
#: (1 for a taxi, 0 for the rest: its constant) and ``fail_prob`` (the chance of getting no taxi
#: the travellers assume, ``Scenario.run``'s ``assumed_no_taxi_share``; 0 for the other modes). A
#: taxi's ``wait_min`` is the assumed waiting time, its ``parking_min`` the boarding, its
#: ``car_min`` the drive; its ``time_min`` is their sum. With a chance ``p`` of no taxi above 0,
#: its ``time_min`` and ``wait_min`` also hold the fleet's cancel limit × ``p / (1 − p)``: asking
#: for a taxi, then taking the best other alternative after a cancelled request, is chosen over
#: that alternative exactly when the ride is, by that margin. A run without a taxi fleet offers
#: neither.
TAXI_ATTRIBUTES: tuple[str, ...] = ("mode_taxi", "fail_prob")


@runtime_checkable
class ChoiceModel(Protocol):
    """What a choice model looks like: a ``choose(batch)`` method."""

    def choose(self, batch: Any) -> Any:
        """The chosen alternative of each situation, as an index within it."""


#: The Python models registered by name: name -> factory(options) -> model.
_REGISTERED: dict[str, Callable[[dict[str, float]], Any]] = {}


def choice_model_register(name: str, factory: Callable[[dict[str, float]], Any]) -> None:
    """Make a Python choice model usable by name, like the built-in ones.

    After ``choice_model_register("my_model", MyModel)``, ``Scenario.from_parts(...,
    choice_model="my_model", choice_options={...})`` builds the model as
    ``factory(choice_options)`` (a dict, empty when none are given) and runs it; the scenario
    keeps the name, so ``replace`` and a plain design can name it. The model is shown in the
    manifest by its ``name`` (the registered name unless it has its own) and identified in the
    fingerprint by its ``descriptor``, which should hold every option it read. A name is
    registered once per session: registering it again replaces the factory.

    Args:
        name: Letters, digits and underscores, not a built-in model's name.
        factory: Called with the options dict; returns an object with ``choose(batch)``.

    Raises:
        ValueError: If the name is a built-in model's or not a plain identifier.
    """
    from openmobisim import _core

    if not name.isidentifier():
        raise ValueError(f"a choice model's name is letters, digits and underscores, got {name!r}")
    if name in _core.choice_models():
        raise ValueError(f"{name!r} is a built-in choice model; register yours under another name")
    if not callable(factory):
        raise ValueError(f"the factory must be callable (options -> model), got {factory!r}")
    _REGISTERED[name] = factory


def choice_models() -> list[str]:
    """The choice models that can be selected by name.

    The built-in ones (``"deterministic"``, ``"logit"``, ``"nested_logit"``), then those
    registered with ``choice_model_register``, in the order registered. Which built-in one a
    ``Scenario`` uses unless told: ``"nested_logit"`` when it offers more than one mode,
    ``"logit"`` otherwise.
    """
    from openmobisim import _core

    return [*_core.choice_models(), *_REGISTERED]


def _choice_model_built(name: str, options: dict[str, float] | None) -> Any:
    """The registered model ``name``, built from its options, named if it has no name."""
    model = _REGISTERED[name](dict(options or {}))
    if not hasattr(model, "choose"):
        raise ValueError(f"the factory of {name!r} returned {model!r}, which has no choose(batch)")
    if getattr(model, "name", None) is None:
        with contextlib.suppress(AttributeError):
            model.name = name
    return model


def _segments(offsets: Any) -> tuple[np.ndarray, np.ndarray]:
    offsets = np.asarray(offsets, dtype=np.int64)
    return offsets[:-1], np.diff(offsets)


def segment_argmax(values: Any, offsets: Any) -> np.ndarray:
    """The index within each situation of its largest value (the first of a tie).

    Args:
        values: One number per alternative.
        offsets: The batch's ``offsets``: situation ``s`` owns
            ``values[offsets[s]:offsets[s + 1]]``; every situation has at least one.

    Returns:
        An ``int64`` array, one index per situation.
    """
    values = np.asarray(values, dtype=np.float64)
    starts, counts = _segments(offsets)
    if len(starts) == 0:
        return np.zeros(0, dtype=np.int64)
    top = np.maximum.reduceat(values, starts)
    at_top = np.where(values == np.repeat(top, counts), np.arange(len(values)), len(values))
    return np.minimum.reduceat(at_top, starts) - starts


def segment_softmax(values: Any, offsets: Any) -> np.ndarray:
    """The logit probabilities of each situation's utilities, one per alternative.

    Computed with each situation's largest utility subtracted, so it cannot overflow.
    """
    values = np.asarray(values, dtype=np.float64)
    starts, counts = _segments(offsets)
    if len(starts) == 0:
        return np.zeros(0)
    top = np.repeat(np.maximum.reduceat(values, starts), counts)
    weight = np.exp(values - top)
    return weight / np.repeat(np.add.reduceat(weight, starts), counts)


def sample_random_utility(batch: Any, utility: Any) -> tuple[np.ndarray, np.ndarray]:
    """Sample the logit of ``utility`` with the batch's common random numbers.

    Each situation takes the alternative with the largest ``utility + gumbel``,
    which is an exact draw from the logit. Returns ``(chosen, probability)``: the
    index within each situation and the logit probability of the alternative taken.
    Give this straight back from ``choose``.
    """
    utility = np.asarray(utility, dtype=np.float64)
    chosen = segment_argmax(utility + batch.gumbel, batch.offsets)
    probability = segment_softmax(utility, batch.offsets)
    return chosen, probability[np.asarray(batch.offsets[:-1], dtype=np.int64) + chosen]
