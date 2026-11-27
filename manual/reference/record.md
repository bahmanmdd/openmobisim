# The settings file and the run record

Two files in the format TOML 1.0, with fixed names, share one layout.

| File | Written by | Read by | What it holds |
|---|---|---|---|
| `run.toml`, the settings file | the user (or the experimental [`make_case`](api.md#make_case)) | [`run_file`](api.md#run_file) | The window, the parameters, the input files and the classes of a run. |
| `run_record.toml`, the run record | every run, beside its four tables | [`Run.open`](api.md#runopen), and the run's two pages | Part one: the settings the run ran with, every default filled in. Part two, from `[result]` on: what the run found and did. |

**Part one of a record is a settings file.** Every line of a record before `[result]`, copied into a `run.toml`, runs
the same run again, provided every input of the run was read from a file. A run never writes `run.toml`, so settings
and results may share a folder. How to use the two files is in [Settings](../guides/settings.md) and
[Reproducing a run](../guides/reproducibility.md).

All tables of keys on this page share one layout: **Key**, **Type** (the TOML type), **Unit**, **In `run.toml`**
(required, or what its absence means) and **Meaning**. In a record every key of part one that applies is written.

## A minimal settings file

% src: crates/binding/README.md ("run_file": "Minimal settings hold the window, the files and the classes alone"); run for this page on the case dotnet

The window, the files and the classes are enough: every parameter left out takes its default. This file, put in place
of the `run.toml` of the case `dotnet` (in a copy of its folder named `minimal`), runs the case:

```toml
window_start_s = 27000
window_end_s = 34200

[network]
nodes_file = "nodes.parquet"
links_file = "links.parquet"

[demand]
trips_file = "trips.parquet"
classes = ["bike_only", "car_and_bike", "car_only", "no_vehicle"]

[transit]
gtfs_file = "gtfs.zip"
```

```python
import tomllib
import openmobisim as oms

run = oms.run_file("minimal", out="minimal-run")
with open(run.record, "rb") as file:
    record = tomllib.load(file)
print(sorted(record))
print(record["parameters"]["bike_speed"], record["parameters"]["link_bin"])
print(record["network"])
print(record["result"]["iterations"], record["result"]["stopped_by_rule"])
print(record["result"]["network"])
```

```text
['demand', 'format', 'network', 'parameters', 'result', 'transit', 'window_end_s', 'window_start_s']
15.0 300
{'nodes_file': '../minimal/nodes.parquet', 'links_file': '../minimal/links.parquet'}
17 True
{'source': 'tables', 'nodes': 41, 'links': 408, 'fingerprint': '396cdc682fcf83a4'}
```

The record of this run holds every parameter at its default, and the input files by their paths relative to the
record's folder.

## Part one: the settings

### The top of the file

% src: crates/io-parquet/README.md ("run_record.toml": first line, format, window); crates/binding/README.md ("run_file")

| Key | Type | Unit | In `run.toml` | Meaning |
|---|---|---|---|---|
| `format` | integer | none | may be absent | The format of the file. Only 1 is read. |
| `window_start_s` | integer | s after midnight | required | The start of the period simulated, a second of the clock. |
| `window_end_s` | integer | s after midnight | required | The end of the period simulated, as given. When it is not a whole number of `link_bin` after the start, the run moves it up to one and counts the repair `window_end_moved`; the record keeps the end given here and the end used in `[result]`. |

A record starts with one comment line that says what the file is and how its two parts divide.

### `[parameters]`

% src: crates/io-parquet/README.md ("[parameters]"); crates/binding/README.md ("Keywords", "run_file"); PARAMETERS.md; a record written by a run of dotnet

One key per parameter of [Parameters](parameters.md), by its name, which gives each one's default, source and
meaning. In a settings file a key left out takes its default. In a record every parameter the run read is written at
the value it used, in this order, each followed by its unit as a comment (`capacity_per_lane = 1800.0  # veh/h per
lane`). A key or table under `[parameters]` that is not in this table is refused by name.

| Key | Type | Unit | Given to |
|---|---|---|---|
| `capacity_per_lane` | float | vehicles per hour per lane | the network |
| `bike_speed` | float | km/h | the network |
| `walk_speed` | float | km/h | the network |
| `stop_walk_max` | float | m | the demand; ignored with a warning in settings without `[transit]`, and absent from the record of a run without transit |
| `transfer_walk_max` | float | m | the timetable; ignored with a warning in settings without `[transit]`, and absent from the record of a run without transit |
| `boarding_slack` | integer | s | the run |
| `max_rides` | integer | rides | the run |
| `nearby_stops` | integer | searches per trip end | the run |
| `max_car_routes` | integer | routes per origin-destination pair | the run |
| `max_transit_journeys` | integer | journeys per trip | the run |
| `choice` | string | name | the run: `"logit"` or `"deterministic"` |
| `nest_scale` | float | none | the run |
| `step_exponent` | float | none | the run |
| `iteration_cap` | integer | iterations | the run |
| `iteration_floor` | integer | iterations | the run |
| `link_bin` | integer | s | the run |
| `seed` | integer, 0 to 2⁶³ − 1 | none | the run |
| `coefficients` | one table per class | see below | the run |

The parameter `osm_class_defaults` is never written and is not a key: its one value is the OpenStreetMap reader's own
table ([Input tables and files](inputs.md#an-openstreetmap-extract)).

### The coefficients of a class

% src: crates/io-parquet/README.md ("[parameters.coefficients]"); crates/loop/README.md ("Choose"); PARAMETERS.md (coefficients); crates/binding/README.md ("Keywords", "run_file")

One table per class, `[parameters.coefficients."<class id>"]`, in class id order, with one key per column of the
choice row: sixteen floats. A coefficient is a cost, so a positive value is worse. In a settings file, the tables may
be left out altogether (every class then takes the defaults); when one is given, every class needs its table, and each table holds all
sixteen keys. As a keyword of `run`, `coefficients` is a `dict` from every class id to its sixteen numbers in the order of
this table. How the costs enter the choice is in [Choice](../concepts/choice.md).

| Key | Unit | Default | Multiplies |
|---|---|---|---|
| `car_drive_min` | cost per minute | 0.1 | The minutes of a car trip on the network. |
| `bike_ride_min` | cost per minute | 0.1 | The minutes of a bike trip on the network. |
| `walk_min` | cost per minute | 0.165 | The minutes walked: the whole of a walk trip, and the walks between the doors and the network of a car or bike trip. |
| `transit_in_vehicle_min` | cost per minute | 0.1 | The minutes in transit vehicles. |
| `transit_access_min` | cost per minute | 0.165 | The minutes of the walk from the door to the first stop. |
| `transit_egress_min` | cost per minute | 0.165 | The minutes of the walk from the last stop to the door. |
| `transit_first_wait_min` | cost per minute | 0.145 | The minutes from the end of the access walk to the first boarding. |
| `transit_transfer_wait_min` | cost per minute | 0.145 | The minutes waited at the later boardings. |
| `transit_transfer_walk_min` | cost per minute | 0.165 | The minutes walked between two rides. |
| `transit_transfers` | cost per transfer | 0.5 | The number of transfers. |
| `money_eur` | cost per euro | 0.6 | The money paid, in euros. |
| `car_ln_path_size` | cost per unit of the logarithm of the path size | −1 | The logarithm of a car route's path size. Recorded as given; the logit reads it times `nest_scale`, the least-cost choice reads it as 0. |
| `is_car` | cost per car alternative | 0 | 1 for a car alternative: the constant of the mode. |
| `is_bike` | cost per bike alternative | 0 | 1 for a bike alternative. |
| `is_walk` | cost per walk alternative | 0.5 | 1 for a walk alternative. |
| `is_transit` | cost per transit alternative | 0.25 | 1 for a transit alternative. |

With a timetable, a class whose coefficient of a transit time or of the transfers is negative or not finite is
refused.

### `[network]`

% src: crates/io-parquet/README.md ("[network]", the paths of input files); crates/binding/README.md ("run_file", "Experimental", "Outputs": the record's file keys)

Either an OpenStreetMap extract or the two tables; a settings file with both is refused.

| Key | Type | Unit | In `run.toml` | Meaning |
|---|---|---|---|---|
| `osm_file` | string | none | this, or the two table files | The path of an OpenStreetMap extract, read by `Network.from_osm`. |
| `region` | array of four floats, or array of pairs of floats | degree (WGS84) | absent: the whole extract | The region the extract is cut to: `[west, south, east, north]` for a box, `[[lon, lat], …]` for a polygon. |
| `car_roads` | string | none | absent: cars on every car road | `"main"`: cars on the main roads alone. **Experimental.** |
| `nodes_file` | string | none | with `links_file`, in place of `osm_file` | The path of the [nodes table](inputs.md#the-nodes-table), a Parquet file. |
| `links_file` | string | none | with `nodes_file` | The path of the [links table](inputs.md#the-links-table), a Parquet file. |

### `[demand]`

% src: crates/io-parquet/README.md ("[demand]"); crates/binding/README.md ("run_file")

| Key | Type | Unit | In `run.toml` | Meaning |
|---|---|---|---|---|
| `trips_file` | string | none | required | The path of the [trips table](inputs.md#the-trips-table), a Parquet file. |
| `classes` | array of strings | none | required | The ids of the user classes. In a record they are in id order. |

### `[transit]`

% src: crates/io-parquet/README.md ("[transit]"); crates/binding/README.md ("run_file")

The table is absent for a run without transit.

| Key | Type | Unit | In `run.toml` | Meaning |
|---|---|---|---|---|
| `gtfs_file` | string | none | required when `[transit]` is there | The path of the GTFS feed, a `.zip`. |

### The paths of the input files

% src: crates/io-parquet/README.md (the paragraph on fingerprints and paths); crates/binding/README.md ("run_file", "Outputs", "Systems and user rights")

| File | A path is read or written as |
|---|---|
| `run.toml` | Joined to the folder of the settings file. A relative path is relative to that folder. |
| `run_record.toml` | Written relative to the folder of the record, with `/` between components (`../inputs/trips.parquet`), from the canonical paths of both. It is absolute only when no relative path exists (on Windows, a file on another drive, or on a network share). |

So a record's folder and its inputs move together, and a run written into a folder at the same depth records the
same paths. An input given in memory has no key in the record: the record's `[network]` or `[demand]` then names no
file, such a run cannot be run again from its record, and `Run.open` needs the input passed.

## Part two: `[result]`

### `[result]`

% src: crates/io-parquet/README.md ("Part two"); a record written by a run of dotnet

Part two starts with a comment line, `# Part two: what the run found and did.`

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `openmobisim_version` | string | none | The version of the package that ran. A development version, ending `-dev`, does not identify the commit it was built from. |
| `window_end_used_s` | integer | s after midnight | The end of the window the run used: `window_end_s`, moved up to a whole `link_bin` when it was not one. Always written. |
| `iterations` | integer | iterations | The number of iterations run: the rows of `iterations.parquet`. |
| `stopped_by_rule` | boolean | none | `true`: the stopping rule ended the run. `false`: the iteration cap did. |

### `[result.repairs]`

% src: crates/io-parquet/README.md ("[result.repairs]"); crates/loop/README.md ("Repairs")

What the run set right about its window, one integer per kind of repair: its count. The table is written only when a
repair of its kind was made. Its keys are `window_end_moved`, `trips_outside_window`, `modes_after_window` and
`trips_after_window` ([Repairs](warnings.md#repairs)).

### `[result.network]`

% src: crates/io-parquet/README.md ("[result.network]"); crates/io-osm/README.md ("Outputs": Report); crates/graph/README.md ("Fingerprint"); a record written from Delft's extract

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `source` | string | none | `"osm"` or `"tables"`: where the network was read from. |
| `nodes` | integer | nodes | The nodes of the network. |
| `links` | integer | links | The links of the network, of all modes. |
| `fingerprint` | string | none | The network's fingerprint, 16 lower-case hexadecimal digits ([below](#fingerprints)). |

`[result.network.osm_report]`, absent for a network from tables, holds the five counts of the OpenStreetMap reader,
which are also `Run.osm_report`:

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `nodes` | integer | nodes | The nodes of the network. |
| `links` | integer | links | The links of the three layers (car, bike, walk). |
| `links_unreachable` | integer | links | The links dropped outside their layer's largest strongly connected part, counted before contraction. |
| `loops_dropped` | integer | pieces | The pieces of ways that start and end at one node, dropped. |
| `ways_cut` | integer | ways | The street ways that hold some of their nodes and miss others: cut by the edge of the extract or of the region, or at a node that cannot be placed. |

`[result.network.repairs]`: each kind of repair the network's readers made, with its count; written only when there
is one ([Repairs](warnings.md#repairs)).

### `[result.demand]`

% src: crates/io-parquet/README.md ("[result.demand]"); crates/demand/README.md ("Outputs": fingerprint)

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `trips` | integer | trips | The trips of the demand. |
| `trips_given` | integer | rows | The rows of the trips table. Written only when it differs from `trips`, that is, when the demand dropped some. |
| `fingerprint` | string | none | The demand's fingerprint. |

`[result.demand.repairs]`: each kind of repair the demand's readers made, with its count; written only when there is
one.

### `[result.transit]`

% src: crates/io-parquet/README.md ("[result.transit]"); crates/io-gtfs/README.md ("Outputs": Report); a record written by a run of delft

Absent for a run without transit.

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `stops` | integer | stops | The stops of the timetable. |
| `runs` | integer | runs | The runs of the timetable. A run is one vehicle's journey along its calls on the service day. |
| `fingerprint` | string | none | The timetable's fingerprint. |

`[result.transit.gtfs_report]` holds the fifteen counts of the GTFS reader, which are also `Run.gtfs_report`:

| Key | Type | Unit | Meaning |
|---|---|---|---|
| `date` | integer | none | The service day read, as GTFS writes a date, `YYYYMMDD`. |
| `trips` | integer | trips | The feed's trips in service that day. |
| `runs` | integer | runs | The runs of the timetable. |
| `calls` | integer | calls | The calls of those runs at stops. |
| `night_runs` | integer | runs | The runs of the day before that continue past midnight, kept from midnight on. |
| `frequency_runs` | integer | runs | The runs made from `frequencies.txt`. |
| `stops` | integer | stops | The stops read. |
| `trips_untimed` | integer | trips | The trips dropped because their first or last call has no time. |
| `trips_single_call` | integer | trips | The trips dropped because they have one call. |
| `times_interpolated` | integer | calls | The calls without a time, interpolated between the timed calls around them. |
| `pickup_drop_off_ignored` | integer | calls | The calls with `pickup_type` or `drop_off_type` 1, read as open to boarding and alighting. |
| `frequency_rows_skipped` | integer | rows | The rows of `frequencies.txt` with a bad time or headway, or an end not after its start. |
| `footpaths` | integer | footpaths | The footpaths derived between stops. |
| `transfers_applied` | integer | rows | The rows of `transfers.txt` that raised a footpath's seconds. |
| `transfers_skipped` | integer | rows | The rows of `transfers.txt` that are not a stop-to-stop minimum time of a footpath. |

`[result.transit.repairs]`: each kind of repair the GTFS reader made, with its count; written only when there is one.

## Fingerprints

% src: crates/io-parquet/README.md ("A fingerprint"); crates/types/README.md ("Outputs": fingerprint); crates/graph/README.md, crates/demand/README.md, crates/transit/README.md ("Fingerprint")

A fingerprint is the 64-bit FNV-1a hash of what the network, the demand or the timetable was built from, written as
16 lower-case hexadecimal digits. It is not a cryptographic hash. The same rows in any order give the same
fingerprint; a change to any value that is fed changes it.

| Fingerprint | Fed | Not fed |
|---|---|---|
| network | The projection; the bike and walk speeds; each node's id, position in metres and signal mark; each link's id, two nodes, length, lanes, modes, car free-flow speed, capacity and signal mark | The links' shapes and road classes; a `capacity_per_lane` that no car link reads |
| demand | The class ids; each trip's id, class, departure, modes and two positions in metres in the network's plane | The joins to nodes and stops, which follow from the network, the timetable and `stop_walk_max` |
| timetable | Each stop's id and position; each run's id and calls (stop, arrival, departure); each footpath's two stops and seconds | The names of stops and lines |

How fingerprints tell whether two runs had the same inputs is in
[Reproducibility](../concepts/reproducibility.md).

## What is not in a record

% src: crates/io-parquet/README.md ("Not written", "Meanings and assumptions")

No time stamp, no machine name, no thread count: a second write of the same run, and a run at any number of threads,
give the same bytes. TOML has no null: a value that does not exist is an absent key or table. A float is written in
its shortest form that reads back as the same number, with a point or an exponent (`1800.0`, `0.1`, `1e-7`).

## What `run_file` and `Run.open` read

% src: crates/binding/README.md ("run_file", "Run.open", "The rule for a finished run")

| | `run_file` reads `run.toml` | `Run.open` reads `run_record.toml` |
|---|---|---|
| `format` | only 1; may be absent | as `run_file` |
| The window | `window_start_s` and `window_end_s`, both required | `window_start_s` and `[result] window_end_used_s`, both required; `window_end_s` is not read |
| `[parameters]` | every key; an absent one takes its default; a key it never holds is refused | `link_bin` (absent: its default), and the keys the inputs it rebuilds are given; a key it never holds is refused |
| `[network]`, `[demand]`, `[transit]` | the input files, built by `Network.from_osm` or `Network.from_tables`, `Transit.from_gtfs` and `Demand.from_table` | the same files, by the same function, for each input not passed |
| `[result]` | not read. A whole record renamed `run.toml` is told in one `UserWarning`, and its part one alone is run | `stopped_by_rule` (absent: `Run.stopped_by_rule` is `None`), `[result.repairs]` |
| Fingerprints | a settings file holds none. For a whole record renamed `run.toml`, an input whose fingerprint differs from the recorded one is named in one `UserWarning` with both, and the run goes on | each input, passed or rebuilt, is compared with the record's; a difference is named in one `UserWarning` with both, and the run is opened |
| A key or table the file never holds, outside `[parameters]` | ignored, all of them named in one `UserWarning` | ignored, all of them named in one `UserWarning` |

`run_file` checks the keys, the window, the files, the classes and the booleans before any input is built; each
parameter's value is checked by the entry point that reads it. `run_file` writes the record of the run it ran, with
the fingerprints of the inputs as they are now.
