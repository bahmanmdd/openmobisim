# A case from your own data

% src: crates/binding/README.md (the interface, "Inputs")

A run needs a network, travellers and, when some of them may take public transport, a timetable. Each is built by one
function, and `run` takes the three:

| Step | Function | It reads | It gives |
|---|---|---|---|
| The network | `Network.from_tables` or `Network.from_osm` | a nodes table and a links table, or an OpenStreetMap extract | a `Network` |
| The timetable (optional) | `Transit.from_gtfs` | a GTFS feed and the network | a `Transit` |
| The travellers | `Demand.from_table` | a trips table, the list of traveller classes, the network and the timetable | a `Demand` |
| The run | `run` | the three, and the window | a `Run`, and its files in a folder |

A table is given in memory or as the path of a Parquet file. In memory it is anything that gives a column by name: a
dict of lists, a pandas or polars frame, a pyarrow table. The columns are fixed and may come in any order;
[Inputs](../reference/inputs.md) lists every column, and [The Python interface](../reference/api.md) every argument.

## A network from two tables

% src: crates/binding/README.md ("Inputs": tables, a cell, mode sets); crates/graph/README.md ("Meanings and
% assumptions", "Bikes"); run by the writer

The example is three nodes at the corners of a right angle, 1 000 m apart: two streets for cars, bikes and walkers,
and a cycle path along the diagonal, for bikes and walkers only. A link runs from its upstream end to its downstream
end, so each street and the path are two links.

```python
import openmobisim as oms
import pyarrow.parquet as pq

nodes = {
    "node_id": ["a", "b", "c"],
    "x_m": [0.0, 1000.0, 1000.0],
    "y_m": [0.0, 0.0, 1000.0],
}
links = {
    "link_id":        ["ab", "ba", "bc", "cb", "path_ac", "path_ca"],
    "upstream":       ["a", "b", "b", "c", "a", "c"],
    "downstream":     ["b", "a", "c", "b", "c", "a"],
    "modes":          ["car;bike;walk"] * 4 + ["bike;walk"] * 2,
    "length_m":       [1000.0, 1000.0, 1000.0, 1000.0, None, None],   # None: the straight line
    "lanes":          [1, 1, 1, 1, None, None],
    "car_speed_km_h": [50.0, 50.0, 50.0, 50.0, None, None],
    "capacity_veh_h": [None] * 6,                                     # None: lanes x capacity_per_lane
}
network = oms.Network.from_tables(nodes, links)
```

The rules that apply:

- **Positions** are either `x_m`, `y_m` (metres on a plane) or `lon_deg`, `lat_deg` (WGS84 degrees), one kind for the
  whole network. A timetable needs a network in longitude and latitude.
- **`modes`** is one string, the mode names joined by `;`, each of `car`, `bike`, `walk`.
- **The car fields** (`lanes`, `car_speed_km_h`, `capacity_veh_h`) are given on a link that allows cars and are `None`
  on any other. A capacity left `None` is the lanes times `capacity_per_lane` (1 800 vehicles per hour per lane by
  default); `inf` is no limit.
- **A length** left `None` is the straight line between the two nodes.
- Two optional columns, `shape` and `road_class`, are for drawing only: no run reads them
  ([The map and the summary](pages.md)).
- The keywords of this step are `capacity_per_lane`, `bike_speed` and `walk_speed`
  ([Parameters](../reference/parameters.md)).

## Trips from a table, and the run

% src: crates/binding/README.md ("Inputs": `trips`, `classes`, `run`); crates/demand/README.md ("Meanings and
% assumptions": linking); run by the writer

A trip is one row: its two ends, of the same kind as the network's nodes, its departure in seconds after midnight, its
traveller class, and the modes open to it. Three travellers go from `a` to `c` at 08:00, each with one mode:

```python
trips = {
    "trip_id":         ["by_car", "by_bike", "on_foot"],
    "class_id":        ["everyone"] * 3,
    "departure_s":     [28800] * 3,                                   # 08:00
    "modes":           ["car", "bike", "walk"],
    "origin_x_m":      [0.0] * 3,
    "origin_y_m":      [0.0] * 3,
    "destination_x_m": [1000.0] * 3,
    "destination_y_m": [1000.0] * 3,
}
demand = oms.Demand.from_table(trips, ["everyone"], network)
run = oms.run(network, demand, window=(28800, 32400), out="tiny-run")   # 08:00 to 09:00

for row in pq.read_table(run.trips).to_pylist():
    print(f'{row["trip_id"]:8} {row["mode"]:5} {row["travel_time_s"]:7.1f} s')
```

```text
by_bike  bike    339.4 s
by_car   car     144.0 s
on_foot  walk   1060.7 s
```

Each time can be checked by hand. The car drives the two streets, 2 000 m at 50 km/h, in 144 s. The bike and the
walker take the path, 1 414.2 m: 339.4 s at 15 km/h and 1 060.7 s at 4.8 km/h, the default speeds.

An end of a trip need not be at a node: for each mode it is joined to the nearest node of that mode's links by a
straight line, which is walked at the walking speed ([Demand](../concepts/demand.md)). The window is required and never
defaulted ([The window and unfinished trips](window.md)); every other keyword of `run` is a row of
[Parameters](../reference/parameters.md).

## A network from OpenStreetMap

% src: crates/io-osm/README.md ("Inputs", "Outputs", "Meanings and assumptions", "The class table");
% crates/binding/README.md (`from_osm`, `Network.tables`); run by the writer on the repository's Delft extract

`Network.from_osm` reads one `.osm.pbf` extract, whole or cut to a region: a `(west, south, east, north)` box, or the
points of a polygon, in degrees. It builds three sets of links of one mode each, the car links, the bike links and the
walk links, each from the ways its own rules admit, and keeps of each set its largest part in which every link can be
reached from every other. A car link takes its lanes and free-flow speed from the way's tags, else from a table by
road class, and its capacity from that table, which counts the class's junctions in
([The network](../concepts/networks.md)).

The example needs an OpenStreetMap extract in the format `.osm.pbf`, which is not part of the published cases:
download one that covers your area and give its path. The counts shown are those of the extract of Delft used here.

```python
delft = (4.315, 51.965, 4.410, 52.035)                     # west, south, east, north, in degrees
network = oms.Network.from_osm("delft.osm.pbf", region=delft)
nodes, links = network.tables()
print(len(nodes["node_id"]), "nodes;", len(links["link_id"]), "links")
```

```text
21468 nodes; 88519 links
```

`Network.tables()` gives the network back as the two tables `Network.from_tables` reads, so a network read from an
extract can be looked at, changed and kept as tables. The traffic-signal marks of an extract are stored and applied
nowhere.

## A timetable from GTFS

% src: crates/io-gtfs/README.md ("Inputs", "Meanings and assumptions"); crates/binding/README.md (`from_gtfs`)

`Transit.from_gtfs(path, network)` reads one GTFS feed, a `.zip`, on a network built from longitude and latitude. It
reads one service day, the feed's busiest weekday (the Monday to Friday with the most trips in service); no other day
can be asked for. The walks between stops are derived once on the network's walk links, up to `transfer_walk_max`
(400 m by default). The travellers are then built with the timetable, which gives each trip the stops within
`stop_walk_max` (1 500 m by default) of its two ends:

```python
transit = oms.Transit.from_gtfs("delft/gtfs.zip", network)

classes = ["bike_only", "car_and_bike", "car_only", "from_outside_car",
           "from_outside_transit", "goods", "no_vehicle"]
demand = oms.Demand.from_table("delft/trips.parquet", classes, network, transit=transit)

run = oms.run(network, demand, transit, window=(27000, 34200), out="osm-run")
last = pq.read_table(run.iterations).to_pylist()[-1]
print(last["iteration"] + 1, "iterations;", last["finished"], "finished;", last["unfinished"], "unfinished")
print(run.osm_report)
```

```text
12 iterations; 37649 finished; 0 unfinished
{'nodes': 21468, 'links': 88519, 'links_unreachable': 2496, 'loops_dropped': 49, 'ways_cut': 218}
```

This run takes the timetable and the travellers of the published Delft case ([The cases](cases.md)) on the network
just read. The demand must be built on the network and the timetable given to `run`, or the run is refused.
`run.osm_report` and `run.gtfs_report` hold the two readers' counts.

Here the travellers and the timetable were given as files, which the run's record names. A table given in memory has
no file to name, and a run cannot be made again from its record then: for a case to keep, give the tables as Parquet
files, or write a settings file that names them ([The settings file](settings.md),
[Running a run again](reproducibility.md)).

## Experimental helpers

% src: crates/binding/README.md ("Experimental: a lighter car network, and a case in one call");
% crates/io-osm/README.md ("Inputs": experimental); run by the writer

Two helpers to get started are **experimental**: they may change, and neither is a model of a city.

- `Network.from_osm(path, region=None, car_roads="main")` gives cars the main roads alone (motorways down to
  unclassified roads), and bikes and walkers every street as before.
- `make_case(folder, osm, population, region=None, gtfs=None, car_roads="all", trips_per_person=0.1, seed=0)` writes
  a `trips.parquet` and a `run.toml` into `folder`. The trips number `population × trips_per_person` and are drawn
  from the streets of the extract by fixed, uncalibrated rules (homes along residential streets, activities along the
  other streets, four classes, and 5 % through traffic by car where the region cuts a motorway or a trunk road, departures evenly over one hour from 07:30, the window
  07:30 to 09:30).

This example needs the same extract, `delft.osm.pbf`, which is not part of the published cases.

```python
folder = oms.make_case("made-case", "delft.osm.pbf", population=100_000, region=delft,
                       gtfs="delft/gtfs.zip", car_roads="main")
run = oms.run_file(folder, out="made-run")
```

Nothing is tuned or removed: how congested such a case is, and whether every trip finishes, follows from the numbers
given. Read the unfinished count of the last row of the iterations table before using such a case
([The window and unfinished trips](window.md)).

## For bikes

% src: crates/binding/README.md ("Bikes"); crates/io-osm/README.md ("Bikes"); crates/demand/README.md ("Bikes")

`bike` in a link's `modes` lets bikes ride it (`path_ac` above is a cycle path, `bike;walk` with no car field), and
`bike` in a trip's `modes` opens the bike to that traveller. From an extract, a cycleway is a bike link and a walk link
and no car link. `make_case` gives 60 % of its travellers a bike. Bike links
in a table and from an extract, and where a bike's trip joins the network, are in [Cycling](cycling.md).

## What can go wrong

% src: crates/binding/README.md ("Repairs", "Errors")

The repairs and refusals of a table are in [Troubleshooting](troubleshooting.md) (the common ones and what to do) and
in [Warnings](../reference/warnings.md) (all of them).
