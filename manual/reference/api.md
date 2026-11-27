# The Python interface

% src: crates/binding/README.md (the opening list and code block); checked by listing the module's names in Python

The module `openmobisim` has the names on this page and no other. There is no command line, no plotting
function, no log and no progress output.

| Name | Kind | What it does |
|---|---|---|
| [`Network.from_osm`](#networkfrom_osm) | static method | reads a network from an OpenStreetMap extract |
| [`Network.from_tables`](#networkfrom_tables) | static method | builds a network from a nodes table and a links table |
| [`Network.tables`](#networktables) | method | gives a network back as the two tables |
| [`Transit.from_gtfs`](#transitfrom_gtfs) | static method | reads the timetable of one service day from a GTFS feed |
| [`Demand.from_table`](#demandfrom_table) | static method | builds the trips of a run from a trips table |
| [`run`](#run) | function | runs the simulation on objects held in memory and writes its files |
| [`run_file`](#run_file) | function | runs the settings file `run.toml` of a folder |
| [`make_case`](#make_case) | function | experimental: writes a case folder from an OpenStreetMap extract |
| [`Run`](#run-the-result) | class | a finished run: the paths of its files and what was repaired |
| [`Run.map`](#runmap) | method | writes the run's interactive map as one HTML file |
| [`Run.summary`](#runsummary) | method | writes the run's summary as one HTML file |
| [`Run.open`](#runopen) | static method | opens a finished run from its folder without running it |

The objects `Network`, `Transit` and `Demand` are immutable. They are made by the static methods above, never by
calling the class. The whole chain, on the case `dotnet` of the [published cases](cases.md):

```python
import openmobisim as oms

network = oms.Network.from_tables("dotnet/nodes.parquet", "dotnet/links.parquet")
transit = oms.Transit.from_gtfs("dotnet/gtfs.zip", network)
classes = ["bike_only", "car_and_bike", "car_only", "no_vehicle"]
demand = oms.Demand.from_table("dotnet/trips.parquet", classes, network, transit=transit)
result = oms.run(network, demand, transit, window=(27000, 34200), out="dotnet-objects-run")

print(result.trips)
print(result.stopped_by_rule)
print(result.repairs)
print(result.map("dotnet-map.html"))
print(result.summary("dotnet-summary.html", title="DOTNET, the morning"))
```

```text
dotnet-objects-run/trips.parquet
True
{'network': {}, 'demand': {}, 'transit': {}, 'run': {}}
dotnet-map.html
dotnet-summary.html
```

**Conventions of this page.** A *path* is a `str` or an `os.PathLike`. A *table* is a table in memory or the path of a
Parquet file ([Input tables and files](inputs.md)). A *second of the clock* is a whole number of seconds after
midnight of the run's day. Every keyword named "a parameter" is a row of [Parameters](parameters.md), which gives its
unit, default and source; a parameter is accepted only by the entry point listed for it there. What each call sets
right instead of refusing, and every refusal, is listed in [Repairs, warnings and refusals](warnings.md).

**Exceptions common to every entry point.**

| Condition | Exception |
|---|---|
| A keyword that is not a parameter, or a parameter that another entry point reads | `TypeError` naming it and the entry point that reads it |
| A required argument missing (`window` or `out` among them) | `TypeError` |
| A parameter's value that cannot be read or is out of its range (a string for a speed, a negative speed) | `ValueError` naming it |
| A boolean given for a parameter or inside one (`True` would be read as 1) | `ValueError` naming it, the value and what to give instead |
| A file that cannot be opened, read or written, or a directory given as a file | `OSError` (`FileNotFoundError` where the entry says so) |
| A refusal of a module of the core | `ValueError` carrying the module's message unchanged |

## `Network.from_osm`

% src: crates/binding/README.md ("Inputs", "Experimental", "Errors"); crates/io-osm/README.md; signature read from the built module

```python
Network.from_osm(path, region=None, car_roads="all", **network_parameters)
```

Reads one OpenStreetMap extract into a network of car links, bike links and walk links. What is read from the extract
is in [Input tables and files](inputs.md#an-openstreetmap-extract); how the network is made is in
[Networks](../concepts/networks.md).

| Argument | Type | Meaning |
|---|---|---|
| `path` | path | One extract in the format `.osm.pbf`. |
| `region` | `None`, or four numbers, or a sequence of pairs of numbers | The part of the extract to read, in WGS84 degrees. `None`: the whole extract. Four numbers `(west, south, east, north)`: a box. A sequence of `(lon, lat)` points: a polygon. Lists and tuples are read alike. |
| `car_roads` | `"all"` or `"main"` | `"all"`: cars drive every road of a car class. `"main"` (**experimental**): cars drive motorways down to unclassified roads only. |
| `capacity_per_lane` | number, vehicles per hour per lane | A parameter. It changes nothing in a network read from an extract, because every car link takes its road class's capacity; it is accepted and recorded. |
| `bike_speed` | number, km/h | A parameter: the one speed of every bike. |
| `walk_speed` | number, km/h | A parameter: the one speed of every walker. |
| `osm_class_defaults` | `None` | A parameter with one value: the reader's own table of speeds, lanes and capacities by road class. |

**Returns** a `Network`. Its attribute `repairs` is a `dict` from repair key to count, what the reader set right.

| Refused | Exception |
|---|---|
| `region` that is no area: not numbers, a boolean among them, west not west of east, south not south of north, fewer than three points, or points that repeat or lie on one line | `ValueError` naming it |
| `car_roads` other than `"all"` or `"main"` | `ValueError` |
| `osm_class_defaults` other than `None` | `ValueError` |
| The file cannot be read | `OSError` |
| The file cannot be decoded, or holds no street of any mode | `ValueError` |

## `Network.from_tables`

% src: crates/binding/README.md ("Inputs": Tables, "Errors"); crates/graph/README.md ("Errors"); signature read from the built module

```python
Network.from_tables(nodes, links, **network_parameters)
```

Builds a network from a nodes table and a links table.

| Argument | Type | Meaning |
|---|---|---|
| `nodes` | table | The [nodes table](inputs.md#the-nodes-table). |
| `links` | table | The [links table](inputs.md#the-links-table). |
| `capacity_per_lane` | number, vehicles per hour per lane | A parameter: the capacity of a car link whose `capacity_veh_h` is null is its lanes times this value. |
| `bike_speed` | number, km/h | A parameter. |
| `walk_speed` | number, km/h | A parameter. |

**Returns** a `Network`, with `repairs` as above. A node or link read from a table carries no traffic signal.

| Refused | Exception |
|---|---|
| A column missing from a table | `ValueError` naming the table and the column |
| A Parquet file with a column of a type that is not read, a column name held twice, a compression other than none or snappy, or a file that is not Parquet | `ValueError` naming the table, the file and the column or the codec |
| A Parquet file that cannot be opened or read, or a directory | `OSError` |
| No node left; a build value that is not a finite number above zero | `ValueError` |

## `Network.tables`

% src: crates/binding/README.md ("Outputs": Network.tables()); crates/binding/tests/test_tables_back.py; called on the case dotnet

```python
network.tables()
```

Takes no argument. **Returns** a tuple `(nodes, links)`: the two tables `Network.from_tables` reads, each a `dict`
from column name to a `list`, in id order.

| Table | Columns, in order |
|---|---|
| `nodes` | `node_id`, then `lon_deg`, `lat_deg` for a network built from longitude and latitude (written to 1e-9 degree), or `x_m`, `y_m` for a network built in metres |
| `links` | `link_id`, `upstream`, `downstream`, `modes`, `length_m`, `lanes`, `car_speed_km_h`, `capacity_veh_h`, `shape`, `road_class` |

The car's speed and capacity are written to nine decimals. `lanes`, `car_speed_km_h` and `capacity_veh_h` are `None`
on a link without cars; `shape` and `road_class` are `None` where the link has none. A bike link comes back as a bike
link.

Two things are not in the tables. The signal marks that an extract gives are stored in the network and applied
nowhere; `from_tables` does not read them, so the network built from the tables has another fingerprint than the one
read from the extract. The values `bike_speed` and `walk_speed` are given again to `from_tables`.

## `Transit.from_gtfs`

% src: crates/binding/README.md ("Inputs": Files, Keywords); crates/io-gtfs/README.md ("Inputs", "Errors"); signature read from the built module

```python
Transit.from_gtfs(path, network, **transit_parameters)
```

Reads one GTFS feed into the timetable of one service day, the busiest weekday of the feed, and derives the footpaths
between its stops on the walk links of `network`. What is read is in
[Input tables and files](inputs.md#a-gtfs-feed); the method is in [Transit](../concepts/transit.md).

| Argument | Type | Meaning |
|---|---|---|
| `path` | path | One GTFS feed, a `.zip` with its files at its root or in one folder inside it. |
| `network` | `Network` | A network built from longitude and latitude. Its projection places the stops; its walk links and walk speed give the footpaths. |
| `transfer_walk_max` | number, m | A parameter: the longest footpath between two stops. |

**Returns** a `Transit`. Its attribute `repairs` is a `dict` from repair key to count.

| Refused | Exception |
|---|---|
| A network built in metres | `ValueError` |
| `transfer_walk_max` not a finite number at or above zero | `ValueError` |
| The file cannot be read | `OSError` |
| The file is no zip; a required file or a required column is missing; no day of the feed has a trip | `ValueError` |

## `Demand.from_table`

% src: crates/binding/README.md ("Inputs": Tables, Keywords; "Repairs"); crates/demand/README.md ("Errors"); signature read from the built module

```python
Demand.from_table(trips, classes, network, transit=None, **demand_parameters)
```

Builds the trips of a run: each end of each trip is linked to the network for each mode open to the trip, and, with
a timetable, to the stops within walking reach ([Demand](../concepts/demand.md)).

| Argument | Type | Meaning |
|---|---|---|
| `trips` | table | The [trips table](inputs.md#the-trips-table). |
| `classes` | list of `str` | The ids of the traveller classes. A trip names one of them in `class_id`. |
| `network` | `Network` | The network the trips travel on. The trips' positions are of its kind, degrees or metres. |
| `transit` | `Transit` or `None` | The timetable, read on the same network. `None`: no trip has a transit alternative. |
| `stop_walk_max` | number, m | A parameter: the longest walk between a door and a stop. |

**Returns** a `Demand`, with two attributes: `repairs`, a `dict` from repair key to count, and `trips_given`, the
number of rows of the trips table (`int`).

| Refused | Exception |
|---|---|
| A column missing; a Parquet file's refusals, as for `Network.from_tables` | `ValueError`, `OSError` |
| `stop_walk_max` not a finite number at or above zero | `ValueError` |
| Positions of another kind than the network's, or of both kinds | `ValueError` naming the trip |

## `run`

% src: crates/binding/README.md ("Inputs": Keywords, run; "Outputs"; "Errors"); crates/loop/README.md ("Errors"); signature read from the built module

```python
run(network, demand, transit=None, *, window, out, **parameters)
```

Runs the simulation and writes five files into `out`: `trips.parquet`, `links.parquet`, `iterations.parquet`,
`routes.parquet` ([Output files](outputs.md)) and `run_record.toml`
([The settings file and the run record](record.md)). How a run proceeds is in
[How a run works](../concepts/architecture.md).

| Argument | Type | Meaning |
|---|---|---|
| `network` | `Network` | The network. |
| `demand` | `Demand` | The trips, built on `network` and on the `transit` given here. |
| `transit` | `Transit` or `None` | The timetable the demand was built on, or `None` for a run without transit. |
| `window` | two `int`, required | `(start_s, end_s)`, two seconds of the clock: the period simulated. It has no default. An end that is not a whole number of `link_bin` after the start is moved up to one (the repair `window_end_moved`). |
| `out` | path, required | The folder the files go to, created if missing. A `run.toml` there is left as it is. A folder that already holds a finished run (its `run_record.toml`) is told in one `UserWarning` naming it, and its files are replaced. |

The parameters `run` reads, each a keyword:

| Keyword | Type | Unit |
|---|---|---|
| `boarding_slack` | `int` | s |
| `max_rides` | `int` | rides |
| `nearby_stops` | `int` | searches per trip end |
| `max_car_routes` | `int` | routes per origin-destination pair |
| `max_transit_journeys` | `int` | journeys per trip |
| `choice` | `"logit"` or `"deterministic"` | name |
| `coefficients` | `dict` from every class id to its sixteen numbers, in the order of the columns of the choice row ([the coefficient tables](record.md#the-coefficients-of-a-class)) | cost per minute, per transfer, per euro, per unit, per alternative |
| `nest_scale` | number | none |
| `step_exponent` | number | none |
| `iteration_cap` | `int` | iterations |
| `iteration_floor` | `int` | iterations |
| `link_bin` | `int` | s |
| `seed` | `int` from 0 to 2⁶³ − 1 | none |

**Returns** a [`Run`](#run-the-result).

| Refused | Exception |
|---|---|
| `window` or `out` missing | `TypeError` |
| A demand built on another network or on another transit than the ones given | `ValueError` |
| An empty or reversed window: `ValueError`. A second below 0 or above 2³² − 1: `OverflowError`. A window past 24 h is accepted, up to an end that, moved up to a whole number of `link_bin`, stays below 2³² − 1 s; beyond that: `ValueError`. | `ValueError` or `OverflowError`, as said |
| `choice` that is not one of the two names; a class without coefficients; a coefficient row for no class; a nest scale outside (0, 1]; a seed outside 0 to 2⁶³ − 1 | `ValueError` naming it |
| `link_bin`, `iteration_cap`, `max_rides`, `nearby_stops`, `max_car_routes` or `max_transit_journeys` zero | `ValueError` |
| With a timetable, a class whose coefficient of a transit time or of the transfers is negative or not finite | `ValueError` |
| An `out` in which one of the five output names is one of the run's input files (canonical paths compared) | `ValueError` naming the file, before the run starts; the input is left as it is |

A known limit of the last check: paths are compared, not files, so an input that is a hard link of one of the five
output names in `out` is not recognised and would be replaced.

## `run_file`

% src: crates/binding/README.md ("Two files with fixed names", "run_file", "Errors"); crates/binding/tests/test_run_file.py; signature read from the built module

```python
run_file(folder=None, *, out)
```

Runs the settings file `run.toml`. It builds the network, the timetable (when the file has `[transit]`) and the
demand with the functions above, gives each the parameters its entry point reads, and runs them with `run`. Every key
of the file is in [The settings file and the run record](record.md).

| Argument | Type | Meaning |
|---|---|---|
| `folder` | path or `None` | The folder holding `run.toml`, or the path of a file of that name. `None`: the current folder. The input files the settings name are joined to this folder. |
| `out` | path, required | The folder the run is written into, as for `run`. It may be the settings' own folder, since no run writes `run.toml`, unless an input there has one of the five output names (as in the published cases, whose inputs are `trips.parquet` and `links.parquet`). |

**Returns** a [`Run`](#run-the-result).

```python
import openmobisim as oms

run = oms.run_file("dotnet", out="dotnet-run")
```

| Refused | Exception |
|---|---|
| A path where nothing is; a folder without `run.toml` | `FileNotFoundError`, saying what to give |
| The path of a file of another name | `ValueError`, saying what to give |
| A folder that holds a run's record and no `run.toml`, or the record's own path | `ValueError`, saying how to run it again |
| A settings file that is not TOML | `tomllib.TOMLDecodeError` (a `ValueError`) |
| A key or table under `[parameters]` that a settings file never holds; a missing required key; a value of the wrong type; a boolean for any key read; a `format` other than 1; both `osm_file` and table files | `ValueError` naming it, before any input is built |
| A parameter's value out of range | the exception of the entry point that reads it, raised once the inputs before that entry point are built |
| Every refusal of `run` | as for `run` |

Warned, with one `UserWarning` each, and the run goes on: a key or table outside `[parameters]` that a settings file
never holds (ignored, all of them named); `stop_walk_max` or `transfer_walk_max` without `[transit]` (ignored); a
whole record renamed `run.toml` (its part one alone is read); an input of such a record whose fingerprint differs
from the one recorded (both fingerprints named).

## `make_case`

% src: crates/binding/README.md ("Experimental"); crates/io-osm/README.md ("Inputs": trips::draw); signature read from the built module

```python
make_case(folder, osm, population, region=None, gtfs=None, car_roads="all",
          trips_per_person=0.1, seed=0)
```

**Experimental.** It writes two files into `folder`, a `trips.parquet` of trips drawn from the streets of the network
by fixed rules that are not calibrated, and a `run.toml`, which `run_file(folder, out=...)` runs. The trips are a
start for trying things, not a model of the city.

| Argument | Type | Meaning |
|---|---|---|
| `folder` | path | The folder to write the case into. |
| `osm` | path | The extract, read as `Network.from_osm` reads it. It is named in `run.toml`, not copied. |
| `population` | number | The people of the area. |
| `region` | as for `Network.from_osm` | The part of the extract to read. |
| `gtfs` | path or `None` | A GTFS feed, named in `run.toml`, not copied. `None`: a case without transit. |
| `car_roads` | `"all"` or `"main"` | As for `Network.from_osm`. |
| `trips_per_person` | number | The share of the people who travel in the hour of the draw. The number of trips is `population × trips_per_person`, rounded to the nearest whole number. |
| `seed` | `int` | The seed of the draw. The same arguments give the same files. |

**Returns** `folder`, as a `pathlib.Path`.

| Refused | Exception |
|---|---|
| No trip (`population × trips_per_person` rounds to 0) | `ValueError` |
| Every refusal of `Network.from_osm` | as there |

## `Run`: the result

% src: crates/binding/README.md ("Outputs", "Repairs"); attribute types read from a run of the cases delft and dotnet

A `Run` is what `run`, `run_file` and `Run.open` return. It keeps nothing of the run in memory: it holds the paths of
the run's files, and each page is read from those files when it is written.

| Attribute | Type | Meaning |
|---|---|---|
| `trips` | `pathlib.Path` | The path of [`trips.parquet`](outputs.md#tripsparquet): one row per trip run. |
| `links` | `pathlib.Path` | The path of [`links.parquet`](outputs.md#linksparquet): one row per link and time bin that someone entered. |
| `iterations` | `pathlib.Path` | The path of [`iterations.parquet`](outputs.md#iterationsparquet): one row per iteration. |
| `routes` | `pathlib.Path` | The path of [`routes.parquet`](outputs.md#routesparquet): one row per trip and leg of what each trip took at the last iteration. |
| `record` | `pathlib.Path` | The path of [`run_record.toml`](record.md): the settings the run ran with and what it found and did. |
| `stopped_by_rule` | `bool` or `None` | `True`: the stopping rule ended the run. `False`: the iteration cap did. `None`: an opened run whose record does not say. |
| `repairs` | `dict` of `dict` | What was set right instead of refused, key to count, under `"network"`, `"demand"`, `"transit"` (with transit) and `"run"` ([Repairs](warnings.md#repairs)). |
| `osm_report` | `dict` or `None` | The five counts of the OpenStreetMap reader for the run's network ([the record](record.md#resultnetwork)); `None` for a network from tables. |
| `gtfs_report` | `dict` or `None` | The fifteen counts of the GTFS reader for the run's timetable ([the record](record.md#resulttransit)); `None` without transit. |

## `Run.map`

% src: crates/binding/README.md ("Outputs": Run.map); crates/map/README.md ("Outputs", "Errors"); signature read from the built module

```python
result.map(path, title=None)
```

Writes the run's interactive map: one HTML file that holds its own style, script and data, loads nothing and opens
from disk in a browser. What it shows is in [The pages of a run](../guides/pages.md).

| Argument | Type | Meaning |
|---|---|---|
| `path` | path | The file to write. A file there is replaced. |
| `title` | `str` or `None` | The page's title. `None`: the stem of the file's name. |

**Returns** `path`, as given. The same run written twice under the same title gives the same bytes.

| Refused | Exception |
|---|---|
| A `path` that is one of the run's five files or one of its input files (canonical paths compared) | `ValueError` naming it, before anything is written |
| A table of the run that is missing or cannot be opened | `OSError` naming the file; no page written |
| A table with a column the page needs missing or of a type that cannot be read; a row whose own trip or link names nothing the inputs hold | `ValueError` naming the file |
| A `path` that cannot be written (a folder, say) | `OSError`; no part of a page is left |

What the tables hold that does not agree with the run's inputs is set aside with a `UserWarning`
([Warnings of a finished run](warnings.md#warnings-of-a-finished-run)).

## `Run.summary`

% src: crates/binding/README.md ("Outputs": Run.summary); crates/summary/README.md ("Outputs", "Errors"); signature read from the built module

```python
result.summary(path, title=None)
```

Writes the run's summary: one HTML page of its numbers, charts and maps, which loads nothing. Its sections are in
[The pages of a run](../guides/pages.md). The arguments, the value returned, the refusals and the warnings are those
of `Run.map`.

## `Run.open`

% src: crates/binding/README.md ("Run.open", "The rule for a finished run"); crates/binding/tests/test_open.py; signature read from the built module

```python
Run.open(folder, network=None, demand=None, transit=None)
```

Opens a finished run from its folder without running anything, and returns a `Run` like the one `run` returned. The
map and the summary of an opened run are the same bytes as those of the fresh run.

| Argument | Type | Meaning |
|---|---|---|
| `folder` | path | The folder of a finished run, which holds `run_record.toml` and the four tables, or the path of the record itself. |
| `network` | `Network` or `None` | The network the run ran on. `None`: rebuilt from the file the record names. |
| `demand` | `Demand` or `None` | The demand the run ran on. `None`: rebuilt from the file the record names. |
| `transit` | `Transit` or `None` | The timetable the run ran on. `None`: rebuilt from the file the record names, when the run had one. |

What it reads from the record is listed in
[The settings file and the run record](record.md#what-run_file-and-runopen-read).

**Returns** a [`Run`](#run-the-result).

```python
import openmobisim as oms

result = oms.Run.open("dotnet-run")
print(result.stopped_by_rule)
```

```text
True
```

| Refused | Exception |
|---|---|
| A path where nothing is; a folder without `run_record.toml` | `FileNotFoundError`, saying what to give |
| The path of a file of another name, `run.toml` included | `ValueError`, saying what to give |
| A record that is not TOML | `tomllib.TOMLDecodeError` (a `ValueError`) |
| A key under `[parameters]` the record should not hold; `window_start_s`, `[result] window_end_used_s`, `[network]`, `[demand]` or `[demand] classes` missing; a `format` other than 1 | `ValueError` naming it |
| An input that was given in memory when the run was made (the record names no file for it) and is not passed | `ValueError`: `pass it: network=` (or `demand=`) |
| `[result] stopped_by_rule` that is not a boolean | `ValueError` naming what it is |
| A demand passed that was not built on the network and transit used | `ValueError` |
| A table missing or unreadable, a column the pages need missing | `OSError` or `ValueError` naming the file |

Warned, with a `UserWarning`, and the run is opened: an input whose fingerprint differs from the record's; a record
without an input's fingerprint; a transit passed to a run that had none (set aside); a key the record never holds
(ignored); and what the tables hold that does not agree with the inputs
([Warnings of a finished run](warnings.md#warnings-of-a-finished-run)).

## Threads

% src: crates/binding/README.md ("Meanings and assumptions")

The interpreter's lock is released while a file is read, while a network, timetable or demand is built, and while a
run runs and is written. The threads are those of one global pool; the environment variable `RAYON_NUM_THREADS` sets
their number. The results are the same at any number of threads
([Reproducibility](../concepts/reproducibility.md)).
