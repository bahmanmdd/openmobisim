# Demand

% src: crates/demand/README.md; crates/binding/README.md; crates/loop/README.md; PARAMETERS.md

The **demand** is who travels: a table of **trips**, each from one place to another at a given time. openmobisim
does not generate trips. It reads them, links each one to the network and to the transit stops, and then lets each
traveller choose a mode and a route in the run.

The demand is the second of the seven concepts of this part in their reading order: the network, the demand, the
routes, the choice, transit, the loading and the iteration. It takes the [network](networks.md): the nodes,
the modes allowed on each link and the walking speed. It gives the [search](search.md) what it needs to look for
routes and journeys: for every trip and every mode open to it, the node at each end, and the stops within walking
reach of each end. The search turns these into a set of alternatives, and the [choice](choice.md) takes one of them.

```{image} ../_static/figures/trip_ends_paper.svg
:alt: A trip end: the door linked by a straight walk to its nearest node of each mode, and to the stops within walking reach
:class: only-light
:width: 100%
```
```{image} ../_static/figures/trip_ends_night.svg
:alt: A trip end: the door linked by a straight walk to its nearest node of each mode, and to the stops within walking reach
:class: only-dark
:width: 100%
```

## A trip

% src: crates/demand/README.md ("Inputs"); crates/binding/README.md ("Inputs": trips); crates/choice/README.md ("Inputs")

A **trip** is one movement of one traveller from an **origin** to a **destination**, door to door. The **door**
of a trip end is its position as given in the trips table, usually not on a node. In this manual
"the traveller" of a trip is the person making it; every trip is its own traveller, chooses on its own and is counted
once. A trip holds:

| Column of the trips table | Type | Meaning |
|---|---|---|
| `trip_id` | text | the trip's identifier |
| `class_id` | text | the traveller class of the trip (below) |
| `departure_s` | whole seconds of the clock | when the traveller leaves the origin |
| `modes` | text | the modes open to the trip, joined by `;`, for example `car;bike;walk;transit` |
| `origin_lon_deg`, `origin_lat_deg`, `destination_lon_deg`, `destination_lat_deg` | degrees (WGS84) | the two ends, on a network built from longitude and latitude; or |
| `origin_x_m`, `origin_y_m`, `destination_x_m`, `destination_y_m` | m | the two ends, on a network built in metres |

`Demand.from_table(trips, classes, network, transit=None)` reads the table, which is a dict of lists, a pandas or
polars frame, a pyarrow table, or the path of a Parquet file ([Input tables](../reference/inputs.md) gives every
rule). The positions must be of the network's kind, degrees or metres. Trips are numbered by the sorted order of
their identifiers, so the order of the rows changes nothing.

## Traveller classes

% src: crates/demand/README.md ("Inputs", "Repairs"); crates/loop/README.md ("Inputs"); crates/binding/README.md ("Inputs": keywords, run_file)

A **traveller class** is a group of travellers who weigh their alternatives in the same way. A class is an
identifier in the list `classes` given to `Demand.from_table`, and each trip names its class in `class_id`. In a run,
every class has its own sixteen coefficients of the cost of an alternative (see [Choice](choice.md)); this is all a
class is. Which modes a traveller may use is not a property of the class: it is written on each trip.

A trip that names a class not in the list is dropped and counted. [Traveller classes](../guides/classes.md) shows
how to give each class its coefficients.

## The modes open to a trip

% src: crates/demand/README.md ("Inputs", "Repairs", "Meanings and assumptions"); crates/binding/README.md ("Inputs": a mode set); crates/types/README.md ("Outputs": mode)

Each trip carries its own **mode set**: any of `car`, `bike`, `walk` and `transit`. The set says what the traveller
may choose among, for example whether a car is available for this trip. A set of one mode is a trip with a given
mode: it still chooses its route, but not its mode.

| Mode in the set | The alternative it opens |
|---|---|
| `car` | a walk to the car network, a car route, a walk to the door |
| `bike` | a walk to the bike network, a bike route, a walk to the door |
| `walk` | a walk to the walk network, a walk route, a walk to the door |
| `transit` | a walk to a stop, a journey on the timetable, a walk from a stop to the door |

A mode is taken out of a trip's set, and counted, when no link of the network allows it (for transit: when no link
allows walking, since the stops are reached on foot). A mode name that is not one of the four is left out and
counted. A trip left with no mode is dropped and counted.

## Linking a trip end to the network

% src: crates/demand/README.md ("Outputs", "Meanings and assumptions": linking, "Bikes"); crates/loop/README.md ("Meanings and assumptions": an alternative, money and gaps)

A trip's ends are positions, usually not on a node. Each end is linked to the network **once per mode of its set**,
because the modes do not use the same links:

- The origin is linked to the nearest node, by straight line, that has a link leaving it which allows the mode.
- The destination is linked to the nearest node that has a link entering it which allows the mode.
- Where both ends are nearest the same node of the mode's links, both are linked to it and no link is travelled.
- Of two nodes equally near, the one with the lower number is taken.

For transit the ends are linked through the walk links, as for walking. So the car end of a trip can be a node on
the nearest street, its bike end a node on a cycle path, and its walk end a node on a footway.

The straight line between an end and its node is the **gap of the trip end**, the gap for short. It is walked, by every mode, in a straight line at
the walking speed:

$$
t_{\text{gap}} = \frac{g}{v_{\text{walk}}}
$$

with $g$ the gap in metres and $v_{\text{walk}}$ the network's walking speed (`walk_speed`, 4.8 km/h by default). A
door 30 m from its node is 22.5 s away. A gap is on no link: it is counted in the trip's time and in its cost as
walking, and it appears in no link count. The linking does not check that the two nodes are connected: that is the
search's to find.

The example gives three trips the same two doors, each 30 m beside the ends of a link of 1 500 m, and one mode each:

```python
import openmobisim as oms
import pyarrow.parquet as pq

nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1500.0], "y_m": [0.0, 0.0]}
links = {
    "link_id":        ["street", "path"],
    "upstream":       ["a", "a"],
    "downstream":     ["b", "b"],
    "modes":          ["car", "bike;walk"],
    "length_m":       [1500.0, 1500.0],
    "lanes":          [1, None],
    "car_speed_km_h": [50.0, None],
    "capacity_veh_h": [float("inf"), None],
}
network = oms.Network.from_tables(nodes, links)

trips = {
    "trip_id":         ["by_bike", "by_car", "on_foot"],
    "class_id":        ["adult", "adult", "adult"],
    "departure_s":     [28800, 28800, 28800],
    "modes":           ["bike", "car", "walk"],
    "origin_x_m":      [0.0, 0.0, 0.0],
    "origin_y_m":      [30.0, 30.0, 30.0],
    "destination_x_m": [1500.0, 1500.0, 1500.0],
    "destination_y_m": [30.0, 30.0, 30.0],
}
demand = oms.Demand.from_table(trips, ["adult"], network)
run = oms.run(network, demand, window=(28800, 32400), out="gap-run")

table = pq.read_table(run.trips).to_pydict()
for trip, mode, seconds in zip(table["trip_id"], table["mode"], table["travel_time_s"]):
    print(trip, mode, seconds)
routes = pq.read_table(run.routes).to_pydict()
for trip, leg, kind, seconds in zip(routes["trip_id"], routes["leg"], routes["kind"], routes["seconds"]):
    print(trip, leg, kind, seconds)
```

```text
by_bike bike 405.0
by_car car 153.0
on_foot walk 1170.0
by_bike 0 walk_gap 22.5
by_bike 1 bike None
by_bike 2 walk_gap 22.5
by_car 0 walk_gap 22.5
by_car 1 car None
by_car 2 walk_gap 22.5
on_foot 0 walk_gap 22.5
on_foot 1 walk None
on_foot 2 walk_gap 22.5
```

Each trip has three legs: a gap walked in 22.5 s, the route on the network, and a gap walked in 22.5 s. The times
are those of a hand calculation:

| Trip | Mode | Time on the link (s) | Two gaps (s) | Door-to-door time (s) |
|---|---|---|---|---|
| `by_bike` | bike | 1 500 m at 15 km/h: 360 | 45 | 405 |
| `by_car` | car | 1 500 m at 50 km/h: 108 | 45 | 153 |
| `on_foot` | walk | 1 500 m at 4.8 km/h: 1 125 | 45 | 1 170 |

## Linking a trip end to stops

% src: crates/demand/README.md ("Depends on"; "Outputs"; "Meanings and assumptions": reach, lists); crates/io-gtfs/README.md ("Footpaths"); PARAMETERS.md (stop_walk_max); crates/loop/README.md ("Outputs": iterations)

A trip with `transit` in its set, in a run with a timetable, gets two lists:

- the **access list**: every stop the traveller can walk to from the origin, with the walking time;
- the **egress list**: every stop from which the traveller can walk to the destination, with the walking time.

Each stop is linked to its nearest node of the walk links. The walk between a door and a stop goes over the walk
links, plus two walk gaps, the gap of the trip end and the stop's:

$$
t_{\text{access}} = \frac{g_{\text{door}}}{v_{\text{walk}}} + t_{\text{links}} + \frac{g_{\text{stop}}}{v_{\text{walk}}}
$$

where $g_{\text{door}}$ is the gap between the door and its walk node, $t_{\text{links}}$ the shortest walking time
over the walk links between that node and the stop's node, in the direction walked, and $g_{\text{stop}}$ the gap
between the stop and its node. A stop is in the list when the whole walk is at most `stop_walk_max`:

| Parameter | Default | Unit | What it sets |
|---|---|---|---|
| `stop_walk_max` | 1 500 | m | the longest walk between a door and a stop, at either end of a trip |

These walking times come from the route search's bounded search over the walk links at free-flow times, once per
stop in both directions; this is why the module `demand` uses `search` and `loading`. The GTFS reader finds its
footpaths with the same search.

By hand: a door 30 m from its node, 1 000 m of walk links and a stop 40 m from its node make a walk of 1 070 m, 802.5 s
at 4.8 km/h. At the default, a walk of 1 500 m is in the list and one of 1 501 m is not.

The lists hold every stop in reach, not the nearest one alone. Which stops a journey uses is decided by the transit
search (see [Transit](transit.md)), which is given both lists. A trip with `transit` in its set and an empty list at
either end is offered no transit journey. This is not an error: the trip keeps its other modes, and the run counts
such trips in the column `without_stop` of the iterations table. Without a timetable, every trip with `transit` in
its set is such a trip.

## Departure times and the window

% src: crates/loop/README.md ("Inputs", "Repairs", "The window and the mean"); crates/binding/README.md ("Inputs": run)

A trip's **departure** is the second at which the traveller leaves the origin door, in seconds of the clock (28 800
is 08:00). It is read from the table and never changed: every alternative of a trip leaves at that second, and the
traveller chooses a mode and a route, not a time.

A run simulates a **window**, a period `(start_s, end_s)` given by the user and never defaulted. The window decides
which trips are run:

| Case | What is done |
|---|---|
| a trip departing before the window's start, or at or after its end | dropped and counted (`trips_outside_window`) |
| a mode whose walk from the door to the network would end at or after the window's end | taken out of the trip's set for the run (`modes_after_window`) |
| a trip left with no mode by that | dropped and counted (`trips_after_window`) |
| a trip that has not reached its destination door when the window ends | run, and reported as unfinished, with no travel time |

[The simulated window](../guides/window.md) explains how to choose a window long enough for the trips to finish.

## What is repaired instead of refused

% src: crates/demand/README.md ("Errors", "Repairs"); crates/binding/README.md ("Repairs")

A demand is refused only when none can be built: `stop_walk_max` is not a number at or above zero, or the positions
are of another kind than the network's. Every other faulty row is set right or dropped, counted by kind, and shown
as one warning per kind. `Demand.trips_given` is the number of rows given and `Demand.repairs` the counts:

```python
trips = {
    "trip_id":         ["t1", "t2", "t3"],
    "class_id":        ["adult", "adult", "child"],
    "departure_s":     [28800, 28800, 28800],
    "modes":           ["car;bike;walk", "bike", "Bike; walk; tram"],
    "origin_x_m":      [0.0, 0.0, 0.0],
    "origin_y_m":      [0.0, 40.0, 0.0],
    "destination_x_m": [1500.0, 1500.0, 1500.0],
    "destination_y_m": [0.0, 30.0, 0.0],
}
demand = oms.Demand.from_table(trips, ["adult"], network)
print(demand.trips_given, demand.repairs)
```

```text
3 {'mode_names_unknown': 1, 'trips_of_unknown_class': 1}
```

Trip `t3` names the mode `tram`, which is left out of its set, and the class `child`, which is not in the list, so
the trip is dropped. The call gives one warning for each. [Warnings](../reference/warnings.md) lists every kind.

## The Delft case

% src: the published case `delft` (doi:10.5281/zenodo.24585967), its `trips.parquet` and `run.toml`, counted with pyarrow

The published case `delft` has 37 649 trips departing between 07:30:00 and 08:29:59 (27 000 s to 30 599 s), in a
window from 07:30 to 09:30, in seven classes:

| Class | Modes open to the trip | Trips (count) |
|---|---|---|
| `bike_only` | bike, walk, transit | 2 663 |
| `car_and_bike` | car, bike, walk, transit | 5 261 |
| `car_only` | car, walk, transit | 1 264 |
| `from_outside_car` | car | 16 544 |
| `from_outside_transit` | transit | 6 325 |
| `from_outside_transit` | transit, bike | 89 |
| `goods` | car | 1 505 |
| `no_vehicle` | walk, transit | 3 998 |

```python
import collections
import pyarrow.parquet as pq

trips = pq.read_table("delft/trips.parquet").to_pydict()
count = collections.Counter(zip(trips["class_id"], trips["modes"]))
for (class_id, modes), n in sorted(count.items()):
    print(class_id, modes, n)
print(min(trips["departure_s"]), max(trips["departure_s"]))
```

```text
bike_only bike;walk;transit 2663
car_and_bike car;bike;walk;transit 5261
car_only car;walk;transit 1264
from_outside_car car 16544
from_outside_transit transit 6325
from_outside_transit transit;bike 89
goods car 1505
no_vehicle walk;transit 3998
27000 30599
```

The names of the classes say what the case's author meant by them; to the simulator a class is its coefficients,
and the modes open to a trip are those written on its row.

## For bikes

% src: crates/demand/README.md ("Bikes"); crates/binding/README.md ("Bikes")

- `bike` in a trip's `modes` opens the bike alternative; a trip without it is never offered a bike.
- A bike's end is its own: the nearest node with a link that a bike may use. It can differ from the car's and the
  walker's end of the same trip. Next to a cycle path the bike and the walker are linked to the path's node while
  the car is linked to the nearest street; next to a motorway the car may be linked to it and the bike is not.
- The walk gap between the door and the bike's node is walked at the walking speed, not ridden.
- A bike is never used to reach a stop: the access and egress walks are on foot.

## Limits

% src: crates/demand/README.md ("Inputs", "Meanings and assumptions", "Bikes"); crates/loop/README.md ("Meanings and assumptions")

- **Trips are input.** Nothing generates trips, destinations or departure times, and a traveller does not choose
  when to leave.
- **A trip stands alone.** A row holds no person and no link to another trip: there are no tours, and a mode taken
  for one trip does not bind the next.
- **One row is one traveller.** A row has no weight; a car trip is one vehicle.
- **A trip end is linked to the nearest node by straight line**, whatever lies between the door and the node, and
  the gap of the trip end is walked at the walking speed by every mode. The linking does not check that the origin's node and the
  destination's node are connected.
- **Stops are reached on foot only**: no bike-and-ride and no park-and-ride.
- **A class changes the coefficients only.** A class has no vehicle fleet, income or other attribute.

## References

- El-Geneidy, A., Grimsrud, M., Wasfi, R., Tétreault, P. and Surprenant-Legault, J. (2014). New evidence on walking
  distances to transit stops: identifying redundancies and gaps using variable service areas. *Transportation* 41,
  193–210. (The source of the default `stop_walk_max`: the 85th percentile of the walk to a stop is 524 m for buses
  and 1 259 m for commuter rail.)
- Knoblauch, R. L., Pietrucha, M. T. and Nitzburg, M. (1996). Field studies of pedestrian walking speed and start-up
  time. *Transportation Research Record* 1538, 27–38. (The source of the default `walk_speed`.)

The full list of the manual is in the [Bibliography](../reference/bibliography.md).
