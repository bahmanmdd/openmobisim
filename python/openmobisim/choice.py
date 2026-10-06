"""Choice models: what to write to add your own.

A *choice model* says which alternative each traveller takes. Today the
alternatives are the routes of the trip's origin-destination pair; the same
interface will carry modes, hubs and departure times. openmobisim ships two
models, selected by name (``choice_models()``):

* ``"deterministic"``: everyone takes the route with the least travel time.
  All-or-nothing; no randomness. Useful for debugging or an upper bound.
* ``"logit"`` (``Scenario.from_parts``'s default since the car-ready
  checkpoint, 2026-09-23): a random utility model, linear in named attributes
  and sampled exactly (Gumbel-max). With its defaults it is the standard
  **path-size logit** for route choice: ``U = -0.2 · time_min + 1 ·
  ln_path_size``. Set any coefficient with ``choice_options``:
  ``{"beta_time_min": -0.3, "beta_length_km": -0.1}``. **The defaults are an
  assumption, not a calibration**; pass estimated coefficients for real work.

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
  facility and a transit itinerary's walks, waits and rides split (see
  ``ROUTE_ATTRIBUTES``).
  Routes and the itineraries of transit, park-and-ride and bike-and-ride trips
  carry the same names, so one model serves both.
* ``batch.gumbel`` — each alternative's standard Gumbel error, keyed on
  (traveller, trip, iteration, alternative identity), so a model that adds it
  to a utility gets exactly the common random numbers the built-in models use.
* ``batch.traveller``, ``batch.trip`` (per situation), ``batch.identity``
  (per alternative; stable when other alternatives come and go),
  ``batch.situation_of`` (per alternative), ``batch.iteration``.
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
expected utility of its best alternative (S238): ``Run.itinerary_choices()["logsum"]``,
for accessibility; without it that column is ``nan``.
 Optional attributes of the model: ``name``, ``descriptor`` (both in
the manifest and part of the run's fingerprint: **change the descriptor when the
model's behaviour changes**, or two different models look like one run),
``sampled`` (``False`` if it never uses ``batch.gumbel``; default ``True``) and
``attributes`` (the ones it reads; default all).

The helpers below do the array bookkeeping so a model is a few lines.
"""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable

import numpy as np

__all__ = [
    "ROUTE_ATTRIBUTES",
    "ChoiceModel",
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
#: sees of a ride's quality, S236; a ferry crossing counts in none);
#: ``walk_access_min``, ``walk_egress_min``, ``walk_transfer_min`` (the walk to the
#: first stop, from the last, and between stops: parts of ``walk_min``),
#: ``wait_first_min``, ``wait_transfer_min`` (parts of ``wait_min``) and
#: ``ride_rail_min``, ``ride_metro_min``, ``ride_tram_min``, ``ride_bus_min``,
#: ``ride_ferry_min``, ``ride_other_min`` (minutes on board by kind of service: parts
#: of ``ride_min``); a part is weighed on top of its total, or instead of it; ``nest``
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
    "nest",
    "mode_walk",
    "mode_bike",
    "mode_car",
    "mode_transit",
    "mode_car_transit",
    "mode_bike_transit",
)


@runtime_checkable
class ChoiceModel(Protocol):
    """What a choice model looks like: a ``choose(batch)`` method."""

    def choose(self, batch: Any) -> Any:
        """The chosen alternative of each situation, as an index within it."""


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
