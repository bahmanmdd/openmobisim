# Traveller classes

A **class** groups travellers who share three things:
- the modes they may use;
- what they own (a car, a bike, a transit pass);
- how they weigh time and money: their choice model coefficients.

Each trip names its class (`user_class`). A run's choices then follow the class of the traveller making them.

## A class table

% src: python/openmobisim/demand.py demand_read_classes (columns), _class_entry
Read classes from a CSV file with {py:func}`openmobisim.demand_read_classes`, one row per class:

| Column | Meaning | Empty means |
|---|---|---|
| `class` | The class's name, as the trips' `user_class` | (required) |
| `share` | Its share of the people, for {py:func}`openmobisim.demand_assign_classes` | no share |
| `modes` | The modes its travellers may use, separated by `;` (`car;bike;walk`) | every mode the run offers |
| `owns_car`, `owns_bike`, `has_transit_pass` | `1` or `0` | the vehicles as the modes imply: a car for a class that may drive or park and ride, a bike for one that may cycle; no pass (a pass waives the fare and is never implied) |
| `beta_*` | Any choice model coefficient (`beta_mode_bike`, `beta_time_min`, …) | the model's own |
| `walk_max_s`, `bike_max_s`, `access_walk_max_s` | The class's own choice-set limits, in seconds | the run's `mode_options` and `transit_options` |

A row without a class, a class listed twice, or an unknown column is refused with the row's number. You can also give
the same columns as a dict: `{"commuter": {"modes": ["car", "transit"], "beta_time_min": -0.1}}`.

A class that says only what its travellers own is a tuple `(owns_car, owns_bike, has_transit_pass)`:
`classes={"driver": (True, False, False)}`. It may use every mode the run offers, with the model's coefficients.

## Giving travellers a class

% src: demand.py demand_assign_classes; examples.py traveller_classes; scenario.py Scenario.__init__ (classes turn
% mode choice on)
```python
classes = ms.demand_read_classes("classes.csv")
trips = ms.demand_assign_classes(trips, classes, seed=0)
scenario = ms.Scenario.from_parts(network, trips, classes=classes)
```

`demand_assign_classes` draws a class for every traveller by the shares. All of a traveller's trips get the same
class, which replaces the one they had. Who falls in which class is random; how many people each class gets is met to
within one traveller. To keep the classes some trips already have (through traffic, goods vehicles), draw for the
others only, as in [Build a case](build_a_case.md).

**Classes turn mode choice on.** When the classes say their modes, every trip without a stated mode chooses among the
modes its class may use.

## Example classes

% src: examples.py traveller_classes
`ms.examples.traveller_classes()` gives four classes by what people own. They are illustrative, not calibrated:

| Class | Share | Modes |
|---|---|---|
| `car_and_bike` | 0.40 | all six |
| `no_vehicle` | 0.30 | walk, transit |
| `bike_only` | 0.20 | walk, bike, transit, bike-and-ride |
| `car_only` | 0.10 | walk, car, transit, park-and-ride |

The two classes that cycle carry a bike constant (`beta_mode_bike` 1.23, fitted so that Amsterdam's residents drive as often as the city's survey says they do: 23 % of their trips, with 38 % by bike); every other mode's constant is 0. The
classes assume a timetable: without one, a `no_vehicle` traveller's trip longer than the walk limit has no
alternative.

## Coefficients: the class wins

% src: scenario.py Scenario.__init__ (the warning)
A class's coefficients replace the run's (`choice_options`) for its travellers, name by name. If you set a run-wide
coefficient that some classes also set, the run warns which classes keep their own. See
[Calibrating parameters](calibration.md).

## For bikes

A class may cycle when `bike` (or `bike_transit`) is among its modes; it then owns a bike unless `owns_bike` says
otherwise. Its own `bike_max_s` limits the rides it is offered. Its `beta_mode_bike` and the per-facility weights
(`beta_bike_separated_km`, `beta_bike_lane_km`, `beta_bike_mixed_km`) set how much it likes cycling, and on what
kind of street.

## Limits

% src: crates/core-sim/src/itinerary_choice.rs (fare: 0 for a pass holder); crates/core-demand/src/travellers.rs Ownership
`has_transit_pass` is money, never access: whether a class may take transit is set by its `modes`, and a fare, if you
set one (`price_options`), is charged to everyone but the classes and persons that say they hold a pass. No class
holds one unless it says so; the pass's own price is not counted. Prices are 0 by default, so without `price_options`
the pass changes nothing.
