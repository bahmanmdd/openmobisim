# Input tables and files

Everything openmobisim reads: three tables (nodes, links, trips), a GTFS feed, an OpenStreetMap extract, and the
settings file `run.toml`, which has [its own page](record.md). A network comes from the nodes and links tables or
from an extract; a timetable from a GTFS feed; the trips from the trips table.

All tables on this page share one layout: **Column**, **Type** (how the cell is read), **Unit**, **Required** (yes, or
no), **Null** (whether a cell may be empty, and what an empty cell means) and **Meaning**.

## What counts as a table

% src: crates/binding/README.md ("Inputs": Tables, A Parquet file)

A table is given in one of two ways, which build the same network or demand (the same fingerprint).

| Form | What is accepted | How its columns are found |
|---|---|---|
| In memory | Anything that gives a column by name, `table[name]`: a `dict` of lists, a pandas or polars frame, a pyarrow table | Its column names are read from `column_names` (pyarrow), else `columns` (pandas, polars), else `keys()` (a `dict`). Each needed column is read once into a plain list, by `to_pylist()` when the column has one, else by iterating it. |
| A Parquet file | The path of the file, a `str` or an `os.PathLike` | The file is read whole; each column becomes the same plain list a table in memory gives. |

The columns are fixed. They may come in any order. There are no aliases: a column is found by exactly its name. A
required column that is missing is refused by name. Any other column is ignored, counted and warned once
(`columns_ignored`, [Repairs](warnings.md#repairs)).

**Parquet column types that are read.**

| Arrow type | Read as |
|---|---|
| `Utf8`, `LargeUtf8` | text |
| `Int8` to `Int64`, `UInt8` to `UInt64` | a whole number, exact |
| `Float32`, `Float64` | a real number (`Float32` widened exactly) |
| `null` (pyarrow's type for a column of nulls alone) | every cell null |
| Any other type (a boolean, a date, a list) | refused, naming the column and its type |

Compression read: none, and snappy (pyarrow's default). Any other codec is refused by name.

## How a cell is read

% src: crates/binding/README.md ("Inputs": A cell)

Each cell is read as the type of its field, whatever the type it was written in.

| Field type | A cell that is read | Example |
|---|---|---|
| text (an id, `modes`, `shape`, `road_class`) | A string; or a whole number written in decimal, whether stored as an integer or as a real number without a fraction | the integer `101` and the real number `101.0` are both the id `"101"` |
| whole number (`lanes`, `departure_s`) | An integer in the field's range; a real number with no fraction; or a string of one | `2`, `2.0` and `"2"` are 2 lanes |
| real number (a position, a length, a speed, a capacity) | An integer, a real number, or a string of one | `50`, `50.0` and `"50"` are 50 km/h |
| any | A boolean is never a number | `True` is not read as 1 |

**Null.** Where a column allows a null, `None`, a real number `NaN` and pandas' `NA` are null; in a Parquet file, a
null of the file. A position that is `NaN` is a number, not a null: the node or the trip is left out
(`nodes_unplaced`, `trips_unplaced`).

**A row that cannot be read** (a fractional or negative second, 300 lanes, a null where none is allowed) is left out,
counted and warned once, naming the row and the column: `nodes_unread` (the node, with its links), `links_unread`,
`trips_unread` ([Repairs](warnings.md#repairs)).

## Mode sets

% src: crates/binding/README.md ("Inputs": A mode set); crates/types/README.md ("Outputs": mode)

A mode set is one string: mode names joined by `;`, for example `car;bike`. There are four modes, `car`, `bike`,
`walk` and `transit`, and none ranks above another. A name is read trimmed and in lower case (`Car; bike` is read as
`car;bike`). A name that is still unknown is left out of its set (`mode_names_unknown`). A link or a trip whose set is
left empty is left out (`links_without_mode`, `trips_without_mode`).

| Where | What the set means | Names read |
|---|---|---|
| `modes` of a link | The modes allowed on the link | `car`, `bike`, `walk`. `transit` is taken out (`links_transit_removed`): transit runs on its timetable, not on links. |
| `modes` of a trip | The modes open to the traveller; a set of one name gives the trip that mode | `car`, `bike`, `walk`, `transit` |

## The nodes table

% src: crates/binding/README.md ("Inputs": nodes); crates/graph/README.md ("Inputs", "Repairs")

One row per node. Read by [`Network.from_tables`](api.md#networkfrom_tables).

| Column | Type | Unit | Required | Null | Meaning |
|---|---|---|---|---|---|
| `node_id` | text | none | yes | no | The node's id. Of two rows with one id the first that can be placed is kept (`nodes_repeated`). |
| `lon_deg` | real number | degree (WGS84) | yes, for a network of longitude and latitude | no | Longitude. A node beyond ±180° is left out, with every link naming it. |
| `lat_deg` | real number | degree (WGS84) | yes, for a network of longitude and latitude | no | Latitude. A node beyond ±84° is left out, with every link naming it. |
| `x_m` | real number | m | yes, for a network in metres | no | The position on a plane, east. |
| `y_m` | real number | m | yes, for a network in metres | no | The position on a plane, north. |

**One kind of position per network.** The kind is told by the column names: longitude and latitude when either
`lon_deg` or `lat_deg` is a column, else metres. A nodes table that holds `lon_deg`, `lat_deg`, `x_m` and `y_m` is not
refused: it is read as longitude and latitude, and `x_m` and `y_m` are ignored and counted (`columns_ignored: 2`).
Positions in longitude and latitude are projected to metres in the
UTM zone of the centre of the nodes' bounding box ([Networks](../concepts/networks.md)). A GTFS feed needs a network
of longitude and latitude.

## The links table

% src: crates/binding/README.md ("Inputs": links); crates/graph/README.md ("Inputs", "Meanings and assumptions", "Repairs", "Bikes")

One row per link. A link is travelled from its upstream end to its downstream end; a two-way street is two links.
Read by [`Network.from_tables`](api.md#networkfrom_tables).

| Column | Type | Unit | Required | Null | Meaning |
|---|---|---|---|---|---|
| `link_id` | text | none | yes | no | The link's id. Of two rows with one id the first usable one is kept (`links_repeated`). |
| `upstream` | text | none | yes | no | The `node_id` where the link starts. A link naming a node not held is left out (`links_to_no_node`). |
| `downstream` | text | none | yes | no | The `node_id` where the link ends. |
| `modes` | text | none | yes | no | The [mode set](#mode-sets) allowed on the link. |
| `length_m` | real number | m | yes | yes: the straight line between the two nodes | The link's length. A length that is not finite or not above zero is replaced by the straight line (`lengths_defaulted`). A link of 0 m is left out (`links_of_no_length`). |
| `lanes` | whole number | lanes | yes | yes, on a link without cars | A car link's lanes in its direction. |
| `car_speed_km_h` | real number | km/h | yes | yes, on a link without cars | A car link's free-flow speed. |
| `capacity_veh_h` | real number | vehicles per hour | yes | yes: lanes × `capacity_per_lane` | A car link's capacity over all its lanes: the flow its downstream end passes. `inf` is no limit. A value that is `NaN` or not above zero is replaced by lanes × `capacity_per_lane` (`capacities_defaulted`). |
| `shape` | text | m, or degree for a network of longitude and latitude | no | yes: a straight line | For drawing only. The points between the link's two ends in its order of travel, `x y;x y` in metres or `lon lat;lon lat` in degrees, the nodes' kind, each a finite number. The empty string is a straight line. A shape that is not points is drawn straight (`shapes_unread`). |
| `road_class` | text | none | no | yes: no class | For drawing only. The name of the road class, for example `motorway`, `residential`, `cycleway`. The empty string is no class. |

**The car fields.** `lanes`, `car_speed_km_h` and `capacity_veh_h` are given on a link whose `modes` hold `car` and
null on any other. A car link without lanes, with 0 lanes, or without a car speed that is a finite number above zero
loses the car mode and is kept for its other modes (`car_mode_removed`). A car field given on a link without cars is
ignored (`car_fields_ignored`).

**The two drawing columns** are read by the map and the summary only. No run reads them; the network's fingerprint
and the run's record do not see them; a link's length is never measured along its shape.

**For bikes.** A link a bike may ride has `bike` in its `modes`. A cycle path is a link whose `modes` are `bike;walk`
with the three car fields null. Every bike rides every bike link at the one speed `bike_speed`
([Parameters](parameters.md)).

## The trips table

% src: crates/binding/README.md ("Inputs": trips, classes); crates/demand/README.md ("Inputs", "Repairs", "Bikes")

One row per trip. Read by [`Demand.from_table`](api.md#demandfrom_table), with the list of class ids.

| Column | Type | Unit | Required | Null | Meaning |
|---|---|---|---|---|---|
| `trip_id` | text | none | yes | no | The trip's id. Of two rows with one id the first given is kept (`trips_repeated`). |
| `class_id` | text | none | yes | no | The id of the trip's traveller class, one of the list `classes`. A trip naming a class not given is dropped (`trips_of_unknown_class`). |
| `departure_s` | whole number | s after midnight | yes | no | The departure, a second of the clock. A trip departing before the run's window starts, or at or after its end, is dropped by the run (`trips_outside_window`). |
| `modes` | text | none | yes | no | The [mode set](#mode-sets) open to the trip. A mode that no link of the network allows is taken out of the set. |
| `origin_lon_deg`, `origin_lat_deg` | real number | degree (WGS84) | yes, on a network of longitude and latitude | no | Where the trip starts. |
| `destination_lon_deg`, `destination_lat_deg` | real number | degree (WGS84) | yes, on a network of longitude and latitude | no | Where the trip ends. |
| `origin_x_m`, `origin_y_m` | real number | m | yes, on a network in metres | no | Where the trip starts, in the network's plane. |
| `destination_x_m`, `destination_y_m` | real number | m | yes, on a network in metres | no | Where the trip ends, in the network's plane. |

The positions are of the network's kind. A coordinate that is not finite, or a longitude and latitude that cannot be
projected, drops the trip (`trips_unplaced`). Each end is joined to the nearest node of each mode's links by a straight
line, walked at the walking speed ([Demand](../concepts/demand.md)).

**The classes** are a list of class ids (`str`), not a table. Each class has its own choice coefficients
([the coefficients of a class](record.md#the-coefficients-of-a-class)).

**For bikes.** `bike` in a trip's `modes` opens the bike alternative to the trip. A bike's end is the nearest node
with a link a bike may use, which may differ from the car's and the walker's.

## An example in memory

% src: run for this page; the two times are length over speed (crates/graph/README.md, "Bikes"; PARAMETERS.md, bike_speed and walk_speed)

Two nodes 1 500 m apart on a plane, a cycle path each way, a bike trip and a walk trip. At the default speeds,
15 km/h by bike and 4.8 km/h on foot, the two trips take 360 s and 1 125 s. The tables a run writes are Parquet
files; reading them in Python needs a Parquet reader, for instance pandas with pyarrow (`pip install pandas pyarrow`),
as [Reading the results](../guides/results.md) explains.

```python
import openmobisim as oms
import pyarrow.parquet as pq

nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1500.0], "y_m": [0.0, 0.0]}
links = {
    "link_id": ["ab", "ba"],
    "upstream": ["a", "b"],
    "downstream": ["b", "a"],
    "modes": ["bike;walk", "bike;walk"],
    "length_m": [None, None],
    "lanes": [None, None],
    "car_speed_km_h": [None, None],
    "capacity_veh_h": [None, None],
}
trips = {
    "trip_id": ["t1", "t2"],
    "class_id": ["everyone", "everyone"],
    "departure_s": [25200, 25260],
    "modes": ["bike", "walk"],
    "origin_x_m": [0.0, 0.0],
    "origin_y_m": [0.0, 0.0],
    "destination_x_m": [1500.0, 1500.0],
    "destination_y_m": [0.0, 0.0],
}

network = oms.Network.from_tables(nodes, links)
demand = oms.Demand.from_table(trips, ["everyone"], network)
result = oms.run(network, demand, window=(25200, 28800), out="two-trips-run")
print(pq.read_table(result.trips).select(["trip_id", "mode", "travel_time_s"]).to_pylist())
```

```text
[{'trip_id': 't1', 'mode': 'bike', 'travel_time_s': 360.0}, {'trip_id': 't2', 'mode': 'walk', 'travel_time_s': 1125.0}]
```

## A GTFS feed

% src: crates/io-gtfs/README.md ("Inputs", "Outputs", "Meanings and assumptions", "Errors", "Repairs", "Bikes"); column names confirmed in crates/io-gtfs/src/

Read by [`Transit.from_gtfs`](api.md#transitfrom_gtfs): one `.zip` with the files at its root or in one folder inside
it, read on a network built from longitude and latitude. One service day is read, the busiest weekday of the feed: the
Monday to Friday with the most trips in service over the feed's whole range of dates, the earliest on a tie. No other
day can be asked for. The method is in [Transit](../concepts/transit.md).

| File | Required | Fields read | What they give |
|---|---|---|---|
| `stops.txt` | yes | `stop_id`, `stop_lat`, `stop_lon` (required); `location_type`; `stop_name` | The stops: every row with `location_type` 0 or blank and a position. A row of `location_type` 1 to 4 is not a stop. `stop_name` is read for the run's pages only. |
| `trips.txt` | yes | `trip_id`, `service_id` (required); `route_id` | The trips and the service each runs on. `route_id` is read for the run's pages only. |
| `stop_times.txt` | yes | `trip_id`, `stop_id`, `stop_sequence` (required); `arrival_time`, `departure_time`; `shape_dist_traveled`; `pickup_type`, `drop_off_type` | The calls of each trip in `stop_sequence` order. A time past `24:00:00` is kept (`25:10:00` is 90 600 s). A blank arrival or departure is the other one; a call with neither is interpolated between the timed calls around it. `pickup_type` and `drop_off_type` 1 are counted and ignored: every call is open to boarding and alighting. |
| `calendar.txt` | this or `calendar_dates.txt`, or both | `service_id`, `start_date`, `end_date`, `monday` to `sunday` (required when the file is there) | The dates each service runs. |
| `calendar_dates.txt` | this or `calendar.txt`, or both | `service_id`, `date`, `exception_type` (required when the file is there) | The exceptions: a service added to or removed from a date. |
| `frequencies.txt` | no | `trip_id`, `start_time`, `end_time`, `headway_secs` | A listed trip is a template: one run from each row's `start_time` every `headway_secs` while before its `end_time`. `exact_times` 0 and 1 are read alike. |
| `transfers.txt` | no | `from_stop_id`, `to_stop_id`, `transfer_type`, `min_transfer_time`; `from_route_id`, `to_route_id`, `from_trip_id`, `to_trip_id` | A row with `transfer_type` 2 (`min_transfer_time`) or 1 (zero) between the two stops of a footpath, with no route or trip: the footpath's seconds become the larger of the walk's and the row's. Every other row is skipped and counted. |
| `routes.txt` | no | `route_id`, `route_short_name`, `route_long_name`, `route_type` | For the run's pages only: each line's name and its kind of service (rail, metro, tram, bus, ferry or other). The timetable and its fingerprint do not read it. |

Nothing else is read: no shapes, no fares, no `bikes_allowed`. The footpaths between stops are not read from the feed:
they are derived on the walk links of the network, bounded by `transfer_walk_max` ([Parameters](parameters.md)).

**The kinds of service**, from `route_type`:

| `route_type` | Kind |
|---|---|
| 0, 5, 900–999 | tram |
| 1, 12, 400–699 | metro |
| 2, 100–199, 300–399 | rail |
| 3, 11, 200–299, 700–899 | bus |
| 4, 1000–1099, 1200–1299 | ferry |
| anything else, or a type that is not a number | other |

**What the reader reports.** `Run.gtfs_report` and the record's `[result.transit.gtfs_report]` hold fifteen counts
([the record](record.md#resulttransit)). Rows that cannot be read are set right or left out and counted
([Repairs](warnings.md#repairs)).

**For bikes.** Nothing in a feed is a bike: a run carries passengers on its timetable, and a footpath is a walk.

## An OpenStreetMap extract

% src: crates/io-osm/README.md ("Inputs", "Outputs", "Units", "Meanings and assumptions", "The class table", "Bikes")

Read by [`Network.from_osm`](api.md#networkfrom_osm): one file in the format `.osm.pbf`, or the part of it a region
holds. The reader builds three layers of one mode each, car links, bike links and walk links, so every link has
exactly one mode. How the layers are split at junctions, trimmed and contracted is in
[Networks](../concepts/networks.md).

**What is read.** The ways tagged `highway` or `route=ferry` and the nodes they name. A way with `area=yes` and a way
of fewer than two nodes are no street.

| Read | Tags | What it gives |
|---|---|---|
| The road class | `highway`; `route=ferry` | One of `motorway`, `motorway_link`, `trunk`, `trunk_link`, `primary`, `primary_link`, `secondary`, `secondary_link`, `tertiary`, `tertiary_link`, `unclassified` (and `road`), `residential`, `living_street`, `service`, `pedestrian`, `footway` (and `steps`, `path`, `track`, `corridor`; `bridleway` for bikes and walkers), `cycleway`, `busway` (and `bus_guideway`), `ferry`. Other values (`construction`, `proposed`, `platform`, …) are not read. |
| Who may use a way, cars | `access`, `motorcar`, `motor_vehicle`, `vehicle`, `service` | A car class, not closed by `access=no` or `private`, nor by the most specific of the three vehicle keys being `no` or `private`, nor a `service` way tagged `parking_aisle`, `driveway`, `drive-through` or `emergency_access`. |
| Who may use a way, bikes | `bicycle`, `motorroad`, `access` | Never motorways and their slip roads. Trunk roads, their slip roads and busways only with `bicycle=yes`, `designated` or `permissive`; footways and pedestrian streets only with one of those or `dismount`; every other class unless `bicycle=no` or `use_sidepath`, `motorroad=yes`, or `access=no` or `private` without a `bicycle` permission. |
| Who may use a way, walkers | `foot`, `motorroad`, `access` | Every class but motorways and their slip roads, unless `foot=no` or `use_sidepath`, `motorroad=yes`, or `access=no` or `private` without a `foot` permission; a busway only with a `foot` permission. |
| Direction | `oneway`, `junction`, `oneway:bicycle`, `cycleway`, `cycleway:both`, `cycleway:left`, `cycleway:right`, `cycleway:<side>:oneway` | Vehicles follow `oneway`, else one way on roundabouts, motorways and their slip roads, else both ways. Bikes follow `oneway:bicycle`, else both ways where a cycleway tag opens the contraflow, else the street's direction. Walkers go both ways. |
| Lanes, per car direction | `lanes`, `lanes:forward`, `lanes:backward`, `lanes:both_ways` | The lanes of each direction; with no tag, the class's. |
| Free-flow speed of cars | `maxspeed:forward`, `maxspeed:backward`, `maxspeed` | A number alone or with `km/h`, `kmh`, `kph`, `mph` (× 1.609 344) or `knots` (× 1.852); `walk` (7 km/h); an implicit code (`urban` 50, `rural` 80, `motorway` 130, `trunk` 100, `living_street` 15, `zone30` 30, `zone20` 20 km/h); with no readable value, the class's. |
| Nodes that split every way | `highway=traffic_signals`, `stop`, `mini_roundabout`; any `barrier` | A junction of the layer. |
| Signals | `highway=traffic_signals` (not `traffic_signals=emergency`) | A mark on the node and on the links ending there. Stored, applied nowhere. |

**What each link gets.**

| Field | Where it comes from |
|---|---|
| Node ids | The OpenStreetMap node ids, shared by the three layers where their links meet |
| Link id | `MODE:WAY:PIECE:f` or `:b`: `MODE` is `car`, `bike` or `walk`; `WAY` the way's id; `PIECE` its number along the way on that layer, from 0; `f` along the way's node order, `b` against it |
| Length (m) | The sum of the WGS84 geodesics between the link's nodes |
| Lanes, car speed (km/h) | The tags, else the class's default below |
| Capacity (vehicles per hour) | Lanes × the class's capacity per lane below |
| Shape, road class | For drawing only: the positions of the nodes between the link's two ends, and the way's road class |

**The class table**: the free-flow speed and the lanes per direction where no tag gives them, and the capacity of one
lane, with the class's junctions counted in, as the reader models no junction. It is the one value of the parameter
`osm_class_defaults`; its sources are in
[Parameters](parameters.md#the-road-classes-of-an-openstreetmap-network) and its reasoning in
[Networks](../concepts/networks.md).

| `highway` | Car speed (km/h) | Car lanes per direction | Capacity per lane (vehicles per hour) |
|---|---|---|---|
| `motorway` | 110 | 2 | 2 000 |
| `motorway_link` | 60 | 1 | 1 550 |
| `trunk` | 90 | 2 | 1 800 |
| `trunk_link` | 50 | 1 | 1 550 |
| `primary` | 60 | 2 | 1 080 |
| `primary_link` | 40 | 1 | 900 |
| `secondary` | 50 | 1 | 900 |
| `secondary_link` | 40 | 1 | 900 |
| `tertiary` | 50 | 1 | 810 |
| `tertiary_link` | 30 | 1 | 720 |
| `unclassified`, `road` | 40 | 1 | 720 |
| `residential` | 30 | 1 | 600 |
| `living_street` | 20 | 1 | 450 |
| `service` | 20 | 1 | 450 |
| `pedestrian`, `footway`, `cycleway`, `busway`, `ferry` | no car link | no car link | no car link |

**The region.** A box or a polygon in WGS84 degrees keeps the nodes inside it. On every way but a main road, the
first node outside either end of each run inside is kept too, so the segment crossing the edge stays. On motorways,
trunk roads and their slip roads, every node within 2 000 m of the region is kept.

**`car_roads="main"`** (experimental) gives cars the main roads alone, motorways down to `unclassified`.

**What the reader reports.** `Run.osm_report` and the record's `[result.network.osm_report]` hold five counts
([the record](record.md#resultnetwork)). Tags that cannot be read are set right and counted
([Repairs](warnings.md#repairs)).

**Not read:** parkings, bus lanes, signal timing, turn restrictions, ferries' durations, and for bikes the kind of
cycling infrastructure and any speed limit.

**For bikes.** A bike has links of its own: a bike link runs where the bike rules above let a bike ride, in the
directions they give. A cycleway is a bike link and a walk link, and no car link.
