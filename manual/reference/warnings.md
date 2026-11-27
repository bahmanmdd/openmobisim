# Repairs, warnings and refusals

openmobisim follows one rule at every entry point: **it refuses only what cannot be done, and sets right, counts and
warns on the rest.** A row that cannot be used is left out or set right, never silently: each kind of repair has a
key and a count. This page lists every repair key, every other warning, and every refusal with its exception. What to
do about each is in [Troubleshooting](../guides/troubleshooting.md).

## How a repair is shown and where it is counted

% src: crates/binding/README.md ("Repairs"); crates/io-parquet/README.md ("[result.*.repairs]"); the example run for this page

Each kind of repair is shown once per call as one `UserWarning`, in this form:

```text
<entry point>: <count> <what was done> (<key>; first: <the first five ids>)
```

and is kept, key to count, in four places:

| Counted in | Attribute, a `dict` from key to count | In `Run.repairs` under | In the record `run_record.toml` |
|---|---|---|---|
| the network | `Network.repairs` | `"network"` | `[result.network.repairs]` |
| the timetable | `Transit.repairs` | `"transit"` (with transit) | `[result.transit.repairs]` |
| the demand | `Demand.repairs`, with `Demand.trips_given`, the rows of the trips table | `"demand"` | `[result.demand.repairs]`, and `[result.demand] trips_given` when the demand dropped some |
| the run | `Run.repairs["run"]` | `"run"` | `[result.repairs]` |

A table of repairs is written in the record only when a repair of its kind was made. The summary's first tile says
how many trips were given and how many were dropped, by reason.

A trips table with a mode name that is not one, and a trip of a class that was not given:

```python
import warnings
import openmobisim as oms

nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1500.0], "y_m": [0.0, 0.0]}
links = {
    "link_id": ["ab"],
    "upstream": ["a"],
    "downstream": ["b"],
    "modes": ["bike;walk"],
    "length_m": [None],
    "lanes": [None],
    "car_speed_km_h": [None],
    "capacity_veh_h": [None],
}
trips = {
    "trip_id": ["t1", "t2", "t3"],
    "class_id": ["everyone", "everyone", "visitors"],
    "departure_s": [25200, 25260, 25320],
    "modes": ["bike", "Walk; scooter", "walk"],
    "origin_x_m": [0.0, 0.0, 0.0],
    "origin_y_m": [0.0, 0.0, 0.0],
    "destination_x_m": [1500.0, 1500.0, 1500.0],
    "destination_y_m": [0.0, 0.0, 0.0],
}

network = oms.Network.from_tables(nodes, links)
with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    demand = oms.Demand.from_table(trips, ["everyone"], network)
for warning in caught:
    print(warning.category.__name__ + ":", warning.message)
print(demand.repairs, demand.trips_given)
```

```text
UserWarning: Demand.from_table: 1 rows naming a mode that is not car, bike, walk or transit: the name left out (mode_names_unknown; first: t2 (scooter))
UserWarning: Demand.from_table: 1 trips naming a class not given: dropped (trips_of_unknown_class; first: t3)
{'mode_names_unknown': 1, 'trips_of_unknown_class': 1} 3
```

## Repairs

% src: the "Repairs" sections of crates/graph/README.md, crates/io-osm/README.md, crates/io-gtfs/README.md, crates/demand/README.md, crates/loop/README.md; crates/binding/README.md ("Inputs": Tables, A cell; "Repairs") and crates/binding/src/tables.rs for the table reader's keys

Every repair key, by the module that makes it. The modules: the *table reader* reads the nodes, links and trips tables
([Input tables and files](inputs.md)); the *graph* builds the network from rows; the *OpenStreetMap reader* and the
*GTFS reader* read their files; the *demand* builds the trips; the *loop* runs the iterations. The last column says
which of the four places above counts the key.

| Key | What was found | What is done | Module | Counted in |
|---|---|---|---|---|
| `columns_ignored` | A column that a table does not read | Ignored | table reader | the network (nodes and links tables); the demand (trips table) |
| `nodes_unread` | A row of the nodes table with a cell that cannot be read as its field | The node left out, with its links | table reader | the network |
| `links_unread` | A row of the links table with a cell that cannot be read as its field (300 lanes, a null where none is allowed) | The link left out | table reader | the network |
| `trips_unread` | A row of the trips table with a cell that cannot be read as its field (a fractional or negative second, a null) | The trip left out | table reader | the demand |
| `mode_names_unknown` | A mode name that is not `car`, `bike`, `walk` or `transit` after trimming and lower-casing | The name left out of its set | table reader | the network (links); the demand (trips) |
| `road_classes_unread` | A `road_class` that is not text | No class | table reader | the network |
| `shapes_unread` | A `shape` that cannot be read as points, or with a point of the other kind of position, not finite or not projectable | Drawn straight; its road class kept | table reader, graph | the network |
| `nodes_repeated` | A node id given twice | The first row that can be placed kept; the others left out | graph | the network |
| `nodes_unplaced` | A node whose position is not finite, or in longitude and latitude beyond ±180° or ±84° | Left out, and every link naming it (`links_to_no_node`) | graph | the network |
| `signals_unplaced` | A signal on a node not held | Ignored | graph | the network |
| `links_repeated` | A link id given twice | The first usable row kept, with its own drawing; the others left out | graph | the network |
| `links_to_no_node` | A link naming a node not held | Left out | graph | the network |
| `links_transit_removed` | A link allowing transit | Transit taken out of its modes (transit runs on its timetable) | graph | the network |
| `car_mode_removed` | A car link without lanes, with 0 lanes, or without a car speed that is a finite number above zero | The car mode taken off, no speed guessed; the link kept for its other modes | graph | the network |
| `car_fields_ignored` | A link without cars giving lanes, a car speed or a capacity | The field ignored | graph | the network |
| `links_without_mode` | A link left with no mode | Left out | graph | the network |
| `lengths_defaulted` | A length given that is not finite or not above zero | The straight line between its nodes | graph | the network |
| `links_of_no_length` | A link of 0 m: a node to itself, or two nodes at one position without a length; from an extract, a piece of 0 m, each of its links on each layer | Left out; no length is invented | graph, OpenStreetMap reader | the network |
| `capacities_defaulted` | A car link's capacity that is `NaN` or not above zero | Lanes × `capacity_per_lane` | graph | the network |
| `drawings_unplaced` | A drawing given before any row of its link | Ignored | graph | the network |
| `drawings_repeated` | A second drawing for one link row | Ignored; the first kept | graph | the network |
| `ways_repeated` | A way id given twice in the extract | The first kept | OpenStreetMap reader | the network |
| `tags_unread` | A car way's `oneway` or lane value of no known meaning | Read as absent | OpenStreetMap reader | the network |
| `speeds_defaulted` | A way cars drive with a `maxspeed`, `maxspeed:forward` or `maxspeed:backward` that is not a speed (counted once per way) | Read as absent: the next key, else the class's speed | OpenStreetMap reader | the network |
| `lanes_repaired` | Lane tags that contradict, or that leave a driven direction no lane | The sides as tagged, or 1 lane ([Networks](../concepts/networks.md)) | OpenStreetMap reader | the network |
| `stops_skipped` | A stop whose `location_type` is not blank or 0 to 4, whose id is empty, or whose position is not a number within ±180° and ±84° | Skipped; its calls are then `calls_dropped` | GTFS reader | the timetable |
| `stops_repeated` | A stop id given twice | The first kept | GTFS reader | the timetable |
| `trips_repeated` | A trip id given twice in `trips.txt` | The first row (its service) kept; the calls of both are one trip | GTFS reader | the timetable |
| `calendar_rows_skipped` | In `calendar.txt`: a date that is not one, a weekday flag other than 0 or 1, a service given twice. In `calendar_dates.txt`: an exception type other than 1 or 2, a date that is not one, a service's date given twice | The row skipped; the flag read as 0; the first kept | GTFS reader | the timetable |
| `weekday_absent` | No Monday to Friday has a trip (a weekend-only feed) | The busiest day of all seven read | GTFS reader | the timetable |
| `stop_times_skipped` | A row of `stop_times.txt` whose trip `trips.txt` does not hold | Skipped | GTFS reader | the timetable |
| `calls_dropped` | A call at a stop not held (absent, a station, or skipped), or with a `stop_sequence` that is not a whole number | The call dropped; a run left with one call is dropped too | GTFS reader | the timetable |
| `times_unread` | A time that is not `H:MM:SS` | Read as blank: the other time, else interpolated | GTFS reader | the timetable |
| `distances_unread` | A `shape_dist_traveled` that is not a finite number | Read as blank | GTFS reader | the timetable |
| `columns_absent` | `arrival_time` or `departure_time` absent as a column | Read as blank in every row | GTFS reader | the timetable |
| `files_unread` | `frequencies.txt` or `transfers.txt` without one of its columns | The file not read | GTFS reader | the timetable |
| `runs_repeated` | Two runs of one name (two overlapping periods of one trip in `frequencies.txt`) | The first kept | GTFS reader | the timetable |
| `trips_without_call` | A trip in service that day, or the night before, left with no call at all | No run | GTFS reader | the timetable |
| `trips_out_of_order` | A run with a call departing before it arrives, or arriving before the call before it departs | The run dropped | GTFS reader | the timetable |
| `footpaths_none` | No link of the network allows walking | No footpath between stops | GTFS reader | the timetable |
| `classes_repeated` | A class given twice | Read once | demand | the demand |
| `trips_repeated` | A trip id given twice in the trips table | The first row given kept; the others dropped | demand | the demand |
| `trips_of_unknown_class` | A trip naming a class not given | Dropped | demand | the demand |
| `trips_unplaced` | A coordinate not finite, or a longitude and latitude not projectable | Dropped | demand | the demand |
| `trips_without_car_network` | A trip with `car` in its modes on a network where no link allows cars | `car` taken out of its set | demand | the demand |
| `trips_without_bike_network` | A trip with `bike` in its modes on a network where no link allows bikes | `bike` taken out of its set | demand | the demand |
| `trips_without_walk_network` | A trip with `walk` in its modes on a network where no link allows walking | `walk` taken out of its set | demand | the demand |
| `trips_without_transit_network` | A trip with `transit` in its modes on a network where no link allows walking, by which its ends are linked | `transit` taken out of its set | demand | the demand |
| `trips_without_mode` | A trip with no mode, given so or left so | Dropped | demand | the demand |
| `window_end_moved` | A window that is not a whole number of `link_bin` | Its end moved up to the next whole bin; the id shown is the end used | loop | the run |
| `trips_outside_window` | A trip departing before the window's start, or at or after its end | Dropped | loop | the run |
| `modes_after_window` | A trip whose walk to the network by car, bike or on foot would end at or after the window's end (once per trip) | Each such mode taken out of its set for the run | loop | the run |
| `trips_after_window` | A trip left with no mode by that | Dropped | loop | the run |

A trip the demand dropped is not in the demand: it has no id and is in no output. A trip the run dropped is not run:
it has no alternative, is in no count of the iterations table and has no row in `trips.parquet`.

**Counts that are not repairs.** The two readers also count what they left out by rule: the OpenStreetMap reader's
five counts (`links_unreachable`, `loops_dropped`, `ways_cut` among them) and the GTFS reader's fifteen
(`trips_untimed`, `trips_single_call`, `pickup_drop_off_ignored`, `frequency_rows_skipped`, `transfers_skipped` among
them). They are in `Run.osm_report`, `Run.gtfs_report` and the record
([the record](record.md#resultnetwork)). A trip with transit in its modes and no stop within `stop_walk_max` of
either end is not an error either: the iterations table counts it as `without_stop`
([Output files](outputs.md#iterationsparquet)).

**For bikes.** A link left out is lost to bikes too, and counted. A bike link that is also a car link without lanes
or speed stays a bike link (`car_mode_removed`). A bike trip whose door is so far from the bike network that its walk
there would end after the window loses the bike (`modes_after_window`).

## When nothing is left

% src: crates/binding/README.md ("Repairs": "When nothing is left")

When the repairs leave nothing, one more `UserWarning` says so plainly, and the work goes on: a run of no trip writes
its files and its pages.

| Warning | From |
|---|---|
| "no link is left" | `Network.from_osm`, `Network.from_tables` |
| "no run is left" | `Transit.from_gtfs` |
| "no trip is left: N given, all dropped (by reason: …)" | `Demand.from_table`, `run` |
| "the network has no car link: N trips that ask for a car cannot drive" | `Demand.from_table` |

## Other warnings of `run` and `run_file`

% src: crates/binding/README.md ("Two files with fixed names", "run_file"); messages read from runs for this page

Each is one `UserWarning`; the run goes on.

| When | What it says | From |
|---|---|---|
| The output folder already holds a finished run (its `run_record.toml`) | The folder, and that this run replaces its files | `run`, `run_file` |
| A key or table of `run.toml`, outside `[parameters]`, that a settings file never holds | The keys, all named; they are ignored | `run_file` |
| `stop_walk_max` or `transfer_walk_max` in settings without `[transit]` | That it is ignored | `run_file` |
| A whole record renamed `run.toml` | That its part one alone is read; write a settings file instead | `run_file` |
| An input of such a record whose fingerprint differs from the recorded one | The input and both fingerprints | `run_file` |

## Warnings of a finished run

% src: crates/binding/README.md ("The rule for a finished run"); crates/io-parquet/README.md (the table under "The rule")

`Run.open`, `Run.map` and `Run.summary` read the run's tables back. What they find that does not agree with the run's
inputs is set aside with a `UserWarning` that names the file and the first five trips or links with their count. A
warning given by `Run.open` is given again by each page, which reads the tables again.

| Found | What is done |
|---|---|
| An input, passed or rebuilt, whose fingerprint differs from the record's | Warned, naming it and both fingerprints; the run is opened with it |
| A record without an input's fingerprint, or with one that is not text | Warned that it cannot be compared; goes on |
| A transit passed to a run that had none | Warned; the transit set aside |
| A key or table a record never holds, outside `[parameters]` | Warned, all of them named; ignored |
| `[result] stopped_by_rule` missing | No warning; `Run.stopped_by_rule` is `None`, and the pages say the run's end is not recorded |
| A table's columns in another order, or other columns beside them | Read as written; no warning |
| A trip of the demand without a row in `trips.parquet` | Read as not run. Warned only when the number of such trips is not the record's count of trips the run dropped |
| A trip's second row in `trips.parquet` | Warned; the first row used |
| A `mode` that is not `car`, `bike`, `walk` or `transit` | Warned; read as none |
| A row of `links.parquet` whose bin is off the record's grid or outside its window, or with cars on a link that does not allow cars | Warned with a count; the row set aside |
| A link and bin given twice in `links.parquet` | Warned with a count; the first row used |
| A trip whose legs in `routes.parquet` are misnumbered, of an unknown kind, lack a value their kind needs, name a link, run or stop not held, or do not agree with its mode | Warned with a count; the trip left out of the journeys, and counted in the footer of both pages |
| A `walk_gap` or `transfer_walk` without its seconds | Warned; the journey shows its door-to-door time and no split by leg |
| An `iterations.parquet` without the column `would_change_share` | Warned, naming the column; the summary shows the share as unknown |

Two things cannot be detected: a value changed in place in a table (a time, a count, a stop), and a `link_bin` replaced
by a divisor of it.

## Refusals

% src: the "Errors" sections of crates/binding/README.md, crates/graph/README.md, crates/io-osm/README.md, crates/io-gtfs/README.md, crates/demand/README.md, crates/loop/README.md, crates/choice/README.md, crates/io-parquet/README.md, crates/map/README.md, crates/summary/README.md; each exception type confirmed by a call

A refusal is a Python exception. Its message names what is refused; a refusal of a module of the core carries that
module's message unchanged.

### At every entry point

| Refused | Exception |
|---|---|
| A keyword that is not a parameter, or a parameter that another entry point reads | `TypeError` naming it and where it is read |
| A missing required argument (`window` or `out` among them) | `TypeError` |
| A parameter's value that cannot be read (a string for a speed) or is outside its range | `ValueError` naming it |
| A boolean given for a parameter or inside one (a coefficient, a second of `window`) | `ValueError` naming it, the value and what to give instead |
| A file that cannot be opened, read or written; a directory given as a file | `OSError` |

### By entry point

| Entry point | Refused | Exception |
|---|---|---|
| `Network.from_osm` | A `region` that is no area: not numbers, a boolean among them, west not west of east, fewer than three points, or points that repeat or lie on one line | `ValueError` |
| `Network.from_osm` | `car_roads` other than `"all"` or `"main"`; `osm_class_defaults` other than `None` | `ValueError` |
| `Network.from_osm` | An extract that cannot be decoded; an extract with no street of any mode | `ValueError` |
| `Network.from_osm`, `Network.from_tables` | No node left; a build value (`capacity_per_lane`, `bike_speed`, `walk_speed`) that is not a finite number above zero | `ValueError` |
| `Network.from_tables`, `Demand.from_table` | A required column missing from a table | `ValueError` naming the table and the column |
| `Network.from_tables`, `Demand.from_table` | A Parquet file with a column of a type not read (not text or a number), a column name held twice, a codec other than none or snappy, or a file that is not Parquet | `ValueError` naming the table, the file and the column or the codec |
| `Transit.from_gtfs` | A network built in metres | `ValueError` |
| `Transit.from_gtfs` | `transfer_walk_max` not a finite number at or above zero | `ValueError` |
| `Transit.from_gtfs` | A file that is no zip; a required file missing (`stops.txt`, `trips.txt`, `stop_times.txt`); a required column missing; no day of the feed's dates has a trip | `ValueError` |
| `Demand.from_table` | `stop_walk_max` not a finite number at or above zero | `ValueError` |
| `Demand.from_table` | A trip's position of another kind than the network's, or trips of both kinds | `ValueError` naming the trip |
| `run` | A demand built on another network or another transit than those given | `ValueError` |
| `run` | An empty or reversed window: `ValueError`. A second below 0 or above 2³² − 1: `OverflowError`. A window past 24 h is accepted, up to an end that, moved up to a whole number of `link_bin`, stays below 2³² − 1 s; beyond that: `ValueError`. | `ValueError` or `OverflowError`, as said |
| `run` | `link_bin`, `iteration_cap`, `max_rides`, `nearby_stops`, `max_car_routes` or `max_transit_journeys` zero | `ValueError` |
| `run` | A `choice` that is not `"logit"` or `"deterministic"`; a nest scale outside (0, 1]; a seed outside 0 to 2⁶³ − 1; a class without coefficients, or a coefficient row for no class; a coefficient that is not finite | `ValueError` |
| `run` | With a timetable, a class whose coefficient of a transit time or of the transfers is negative or not finite | `ValueError` |
| `run`, `run_file` | An output folder in which `trips.parquet`, `links.parquet`, `iterations.parquet`, `routes.parquet` or `run_record.toml` is one of the run's input files | `ValueError` naming the file, before the run starts |
| `run_file` | A path where nothing is; a folder without `run.toml` | `FileNotFoundError` |
| `run_file` | The path of a file of another name; a folder that holds a run's record and no `run.toml`, or the record's own path | `ValueError`, saying what to give |
| `run_file` | A settings file that is not TOML | `tomllib.TOMLDecodeError` (a `ValueError`) |
| `run_file` | A key or table under `[parameters]` a settings file never holds; a missing required key; a value of the wrong type; a boolean for any key read; a `format` other than 1; both `osm_file` and table files | `ValueError` naming it, before any input is built |
| `make_case` | No trip (`population × trips_per_person` rounds to 0) | `ValueError` |
| `Run.open` | A path where nothing is; a folder without `run_record.toml` | `FileNotFoundError` |
| `Run.open` | The path of a file of another name, `run.toml` included | `ValueError`, saying what to give |
| `Run.open` | A record that is not TOML | `tomllib.TOMLDecodeError` (a `ValueError`) |
| `Run.open` | A key under `[parameters]` a record should not hold; `window_start_s`, `[result] window_end_used_s`, `[network]`, `[demand]` or `[demand] classes` missing; a `format` other than 1 | `ValueError` naming it |
| `Run.open` | An input given in memory when the run was made and not passed | `ValueError`: `pass it: network=` (or `demand=`) |
| `Run.open` | `[result] stopped_by_rule` that is not a boolean | `ValueError` naming what it is |
| `Run.open` | A demand passed that was not built on the network and transit used | `ValueError` |
| `Run.open`, `Run.map`, `Run.summary` | A table of the run missing or unreadable | `OSError` naming the file; no page written |
| `Run.open`, `Run.map`, `Run.summary` | A column the pages need missing or of a type that cannot be read; a row of `trips.parquet`, `links.parquet` or `routes.parquet` whose own trip or link names nothing the inputs hold | `ValueError` naming the file, and the row |
| `Run.map`, `Run.summary` | A `path` that is one of the run's five files or one of its input files | `ValueError` naming it, before anything is written |
| `Run.map`, `Run.summary` | A `path` that cannot be written | `OSError`; no part of a page is left |

A known limit of the check on the output folder of `run` and `run_file`: paths are compared, not files, so an input that is a hard link
of one of the five output names in the output folder is not recognised and would be replaced.
