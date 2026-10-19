# Demand

% src: crates/core-demand/src/{lib.rs, trips.rs, persons.rs, travellers.rs, vehicles.rs, mode.rs};
% crates/py-bindings/src/pipeline.rs (Window, the window filter, build_travellers); python/openmobisim/demand.py;
% python/openmobisim/scenario.py (window, default_weight, WindowWarning); python/openmobisim/_cases.py (Case.demand)

## What it is

The **demand** is who travels, from where to where, and when. It is a table of **trips**. Each trip belongs to a
**traveller**, and a traveller's trips, in order, make their day: home to work, work to the shop, the shop to home. A
traveller may stand for several people: their **weight**. Each traveller belongs to a **traveller class**, which says
what they own, which modes they may use and how they weigh their choices (see [Modes and itineraries](modes.md)).

openmobisim does not generate travel behaviour beyond the trips it is given: the departure times are fixed, and the
mode and route are chosen in the run. The demand is input.

## The trips table

% src: core-demand/src/trips.rs RawTrip, read_trips_parquet; python/openmobisim/demand.py TRIP_COLUMNS, demand_read_trips

One row per trip, the same columns whether the table is a `trips.parquet` file, a `trips.csv` file or rows in memory:

| Column | Type | Meaning |
|---|---|---|
| `traveller_id` | text or integer | who travels |
| `trip_seq` | integer | the trip's place in the traveller's day |
| `origin_lon`, `origin_lat` | degrees (WGS84) | where the trip starts |
| `destination_lon`, `destination_lat` | degrees (WGS84) | where it ends |
| `departure_time_s` | whole seconds after midnight | when it leaves |
| `user_class` | text | the traveller's class |
| `weight` | whole number, optional | how many people the traveller stands for; empty: the scenario's `default_weight` (1) |
| `mode` | text, optional | the stated mode: `car`, `bike`, `walk`, `transit`, `car_transit` or `bike_transit`; empty: the trip is a car trip, or chooses its mode (see [Modes and itineraries](modes.md)) |

A mode outside this list is refused with the list of accepted values, so a typo never becomes a car trip. Other
columns are not read.

`Scenario.from_parts(network, demand, ...)` takes the table as rows, a `.parquet` path or a `.csv` path.
{py:func}`openmobisim.demand_read_trips` and {py:func}`openmobisim.demand_write_trips` read and write the CSV form.

### Persons

% src: core-demand/src/persons.rs RawPerson; travellers.rs build (the override)

An optional **persons** table (`persons=`, rows or a `persons.parquet` path) overrides, per traveller, what their class
says they own: `traveller_id`, and optionally `owns_car`, `owns_bike`, `has_transit_pass` and `user_class`. An empty
value means no override.

## From rows to travellers

% src: core-demand/src/travellers.rs build, codes

A run gathers each traveller's trips, orders them by `trip_seq`, and builds its travellers. Problems in the data are
repaired by a fixed rule and counted in `diagnostics.parquet`, never silently:

| Rule | What happens | Counted as |
|---|---|---|
| **The chain** | each trip starts where the trip before it ended: a stated origin that differs is replaced | `trip_chain_origin_overridden` |
| Repeated `trip_seq` | the first row in file order is kept | `duplicate_trip_seq` |
| Rows disagreeing on `weight` | the weight of the earliest trip (by `trip_seq`) that states one is kept | `inconsistent_traveller_weight` |
| Rows disagreeing on `user_class` | the earliest trip's class is kept, before any persons override | `inconsistent_user_class` |
| A persons row for an unknown traveller | ignored | `person_unknown_traveller` |
| Two persons rows for one traveller | the first is kept | `duplicate_person_row` |

A traveller's car and bike, if they own them, start the day at the origin of their first trip.

**Weights.** A traveller's weight is a whole number. Every result that counts people (travel times, link volumes, mode
shares, parking occupancy) counts a trip as many times as its weight, and on the roads a traveller of weight $w$ is a
vehicle of $w$ cars.

## The simulation window

% src: py-bindings/src/pipeline.rs Window::resolve, WINDOW_DEFAULT_S, raw_trips.retain; python/openmobisim/scenario.py _window,
% WindowWarning, Run.window; core-sim/src/run.rs (with_window_start)

A run simulates a **window** of the day, `Scenario(window=(start, end))`, given as clock times (`("07:30", "09:30")`)
or seconds after midnight. The run starts at the window's start and stops at its end:

- a trip still under way at the end is **unfinished** (`truncated`), counted and reported;
- a timetable keeps only each vehicle run's calls inside the window (see [Transit](transit.md));
- a trip that **departs outside** the window is left out: not simulated and not counted in any result.

When trips are left out, the run warns with a `WindowWarning`, at every run, saying how many trips and people, when they
depart, and how to widen the window. {py:attr}`openmobisim.Run.window` gives the same numbers.

Without a window, or for an end not given, the run starts at the earliest departure, rounded down to the quarter hour,
and lasts three hours. Leave each trip time to finish: a demand whose departures end well before the window does
finishes inside it.

The trips outside the window are removed **before** the travellers are built. So a traveller whose first trip departs
before the window starts their day at their first trip inside it, with their car and bike there.

## Making trips from an OD matrix

% src: python/openmobisim/demand.py demand_read_zones, demand_read_od, demand_from_od, _integerise

Demand often comes as an **OD matrix**: the number of trips from each origin zone to each destination zone.
{py:func}`openmobisim.demand_read_zones` reads the zones (`zone_id` and a point, `x_coord`/`y_coord` or `lon`/`lat`),
{py:func}`openmobisim.demand_read_od` the matrix (`origin_zone`, `destination_zone`, `trips`; optionally a departure
window `start_s`/`end_s`, a `user_class` and a `mode` per row), and {py:func}`openmobisim.demand_from_od` turns them into
trips:

1. **Whole trips.** Within each group of rows with the same window, class and mode, every cell gets the whole part of
   its trips, and the cells with the largest remainders one more each, until the group's total (rounded) is reached. A
   tie is broken by a draw keyed on the seed and the cell, so the result never depends on the order of the rows.
2. **One traveller per trip**, with one trip each (a matrix holds no chains), named `od0000001`, ….
3. **When:** a departure in the row's window (or `start_s`–`end_s`, 07:00–09:00 by default), drawn from a `profile`:
   `"uniform"`, or `"am_peak"`, a bell over the window (a Beta(3, 3) distribution, which puts two thirds of the trips in
   the middle 40 % of the window).
4. **Where:** the zone's point moved by a random distance of up to `spread_m` (300 m) in a random direction, uniform
   over the disc, so a zone's trips do not all start at one node; `spread_m_by_zone` gives some zones their own (0 for a
   gateway, where trips from outside the study area enter on a road). With `points_per_zone`, a trip end is instead one
   of that many fixed points around the zone's point, so trips between the same zones share their routes and their
   searches. With `zone_points` (a [zone points table](../reference/inputs.md#zone-points-zone_pointscsv-optional)), a
   zone's trip ends are drawn among its own points by weight, origins by one weight (where people live) and
   destinations by the other (where they work), so a trip starts and ends where the buildings are; a trip within such
   a zone ends at a point other than its origin's.

The whole trips (step 1) are the same on any machine; the departures and places (steps 3 and 4) come from one NumPy
generator, so they are the same for one NumPy version (NumPy does not promise the same stream across its releases).
Keep the trips you ran, not only the seed, to repeat a study.

## Traveller classes

% src: python/openmobisim/demand.py demand_read_classes, demand_assign_classes

{py:func}`openmobisim.demand_read_classes` reads a class table, and {py:func}`openmobisim.demand_assign_classes` gives
every traveller a class, drawn by the classes' `share`. The shares are of people (weights), scaled to sum to one, and
are met to within one traveller: the travellers are put in a random order fixed by the seed and their id, and the
classes take consecutive runs of that order. Who falls in which class is random; how many is not. A traveller's trips
all get the same class. What a class says is in [Modes and itineraries](modes.md).

## Sampling the demand

% src: python/openmobisim/demand.py demand_sample

{py:func}`openmobisim.demand_sample` keeps one traveller in every `weight`, each standing for `weight` times as many
people, so a run costs about `weight` times less to choose, route and load:

- a traveller is kept or left out with all their trips, so their day stays whole;
- the travellers are put in a random order fixed by the seed and their id, and every `weight`-th is kept **within each
  group of the same class and the same stated mode of their first trip**, so a rare class or mode is not lost or
  doubled by chance;
- every kept trip's weight is multiplied by `weight`, so totals estimate those of the whole table.

This is a small **aggregation**, not a rescaled network: a kept traveller is a group of `weight` people who travel and
choose together; on the roads it is a platoon of `weight` cars, each counted at its full size, and the network keeps its
real capacities, storage and signal timings. A group gives up individuality: one choice for all its members, one platoon
in the queues. Under congestion its queues form and clear in blocks, so delays read a little higher than with one
traveller per person. Another `seed` is another sample, for replications.

## The starter cases' demand

% src: python/openmobisim/_cases.py Case.demand, Case.scenario

A starter case's demand is an OD matrix with its zones and a class table, in the case's files, turned into trips by
{py:meth}`openmobisim.examples.Case.demand`:

1. the matrix is multiplied by `multiple` (1) and divided by the case's `weight`;
2. {py:func}`openmobisim.demand_from_od` makes the trips with the options the case's `case.json` gives (its departure
   window, profile, spread and points per zone);
3. the travellers of rows without a class are given one by the classes' shares
   ({py:func}`openmobisim.demand_assign_classes`); rows that name a class and a mode keep them (trips from outside the
   study area, goods vehicles);
4. with a case `weight` above 1, every trip carries it: each simulated traveller stands for that many people (the *fast
   run*); `weight=1` simulates one traveller per person (the *complete run*).

How each case's matrix was made, and from which sources, is in its `case.json` (`info`) and in
[Starter cases and the data bundle](../reference/cases.md).

## Results

% src: python/openmobisim/scenario.py Run.window, Run.completion, Run.completion_by_mode, Run.trip_modes

| Accessor | What it gives |
|---|---|
| {py:attr}`openmobisim.Run.window` | the window, and the trips and people left out before and after it |
| {py:attr}`openmobisim.Run.completion` | trips completed, unfinished, without a vehicle, without a path, with a mode the run cannot simulate |
| {py:attr}`openmobisim.Run.completion_by_mode` | the same per mode, with people and travel time |
| {py:meth}`openmobisim.Run.trip_modes` | every trip's departure and mode |

## For bikes

% src: core-demand/src/mode.rs; persons.rs (owns_bike); demand.py demand_sample (groups by stated mode), demand_read_od (mode)

- A bike trip is a trip whose `mode` is `bike`, or a trip that chooses its mode and takes the bike.
- Whether a traveller owns a bike comes from their class (`owns_bike`, or the modes it may use) and can be set per
  person in the persons table.
- An OD matrix row may state the mode `bike`; {py:func}`openmobisim.demand_from_od` keeps it on its trips.
- {py:func}`openmobisim.demand_sample` samples within each stated mode, so the cyclists of a demand keep their share.

## Limits

- Departure times are fixed: there is no choice of when to travel.
- Trips made from an OD matrix are single trips, not chains; a day of several trips needs a trips table.
- Columns of the trips table beyond the eight required and two optional ones are not read. Values per trip or traveller
  for the choice model are given to the `Scenario` instead (`trip_values`, `person_values`; see [Choice](choice.md)).
- A traveller's weight is a whole number.
- One day: departures are seconds after midnight of one day.
