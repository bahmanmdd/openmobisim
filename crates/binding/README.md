# openmobisim-binding

The Python module `openmobisim`: nine names, nothing else (`Network`, `Transit`, `Demand`,
`run`, `run_file`, `Run` and its methods `Run.map`, `Run.summary` and `Run.open`, a static
method). Built by maturin (the `pyproject.toml` at the top of the tree) as one
extension on Python's stable ABI from 3.11 (abi3). Depends on `openmobisim-types`, `-graph`,
`-transit`, `-demand`, `-loop`, `-io-osm`, `-io-gtfs`, `-io-parquet`, `-map`, `-summary` and `pyo3`; no Python
runtime dependency, no `numpy`. A wheel is built by `maturin build --release -m crates/binding/Cargo.toml`; the
release's wheels (Linux, macOS, Windows) and its source archive are built, tested and published by
`.github/workflows/wheels.yml`.

```python
import openmobisim as oms
network = oms.Network.from_osm(path, region=None, **network_parameters)
network = oms.Network.from_tables(nodes, links, **network_parameters)
transit = oms.Transit.from_gtfs(path, network, **transit_parameters)
demand  = oms.Demand.from_table(trips, classes, network, transit=None, **demand_parameters)
result  = oms.run(network, demand, transit=None, window=(start_s, end_s), out=folder, **parameters)
result  = oms.run_file(folder=None, out=folder)
result  = oms.Run.open(folder, network=None, demand=None, transit=None)
page    = result.map(path, title=None)
page    = result.summary(path, title=None)
```

## Inputs
- **Files.** `from_osm`: one `.osm.pbf` extract (`crates/io-osm`), cut to `region` when given: a
  `(west, south, east, north)` box, or a sequence of `(lon, lat)` points of a polygon, in WGS84
  degrees, lists and tuples alike (`[[lon, lat], …]`, as a record's region reads back from TOML;
  `None`: the whole extract); a region that is no area (not numbers, a boolean among them, west
  not west of east, fewer than three points, or points that repeat or lie on one line so that
  they enclose no area) is refused, `ValueError` naming it.
  `from_gtfs`: one GTFS `.zip`
  read on a network built from longitude/latitude (`crates/io-gtfs`).
- **Tables.** A table is in memory or the path of a Parquet file (a `str` or an
  `os.PathLike`, as `os.fspath` reads it). In memory it is anything giving a column by name,
  `table[name]`: a dict of lists, a pandas or polars frame, a pyarrow table. Its column names are read from `column_names`
  (pyarrow), else `columns` (pandas, polars), else `keys()` (a dict); each needed column is read
  once into a plain list, by `to_pylist()` when the column has one, else by iterating it (the
  tests give dicts of lists and pyarrow tables; pandas and polars are not installed in CI). The
  columns are fixed, in any order, with no aliases; a missing one is refused by name, any other
  ignored (`columns_ignored`, warned once naming it; the links table's two optional columns,
  for drawing only, below, are read):
  - `nodes`: `node_id` (str), and either `lon_deg`, `lat_deg` (WGS84 degrees) or `x_m`, `y_m`
    (metres on a plane). The kind is told by the names: longitude/latitude when either
    `lon_deg` or `lat_deg` is a column, else metres. One kind per network (the graph's rule).
  - `links`: `link_id`, `upstream`, `downstream` (str: node ids), `modes` (str), `length_m`
    (float or `None`: the straight line), `lanes` (int or `None`), `car_speed_km_h` (float or
    `None`), `capacity_veh_h` (float or `None`: lanes × `capacity_per_lane`; `inf` is no
    limit). The car fields are given on a car link and `None` on any other (the graph's rule).
    Optionally, for drawing only (the map and the summary; no run reads them, the fingerprints
    and the record do not see them): `shape` (str or `None`: the points between the link's two
    ends in its order of travel, `x y;x y` in metres, or `lon lat;lon lat` in degrees for a
    network of longitude/latitude: the nodes' kind, each a finite number; `None`, the empty
    string or no column: straight) and `road_class` (str or `None`: the road class's name,
    `motorway`, `residential`, `cycleway`; `None`, the empty string or no column: none). Either
    may be absent. A shape that is not points is drawn straight (the graph's `shapes_unread`).
  - `trips`: `trip_id`, `class_id` (str), `departure_s` (int, a second of the clock), `modes`
    (str), and the two ends of the network's kind: `origin_lon_deg`, `origin_lat_deg`,
    `destination_lon_deg`, `destination_lat_deg`, or `origin_x_m`, `origin_y_m`,
    `destination_x_m`, `destination_y_m`.
  - A mode set is one string: mode names joined by `;` (`car;bike`), each of `car`, `bike`,
    `walk`, `transit`, read trimmed and lower-cased (`Car; bike` is read); a name still unknown is
    left out of its set (`mode_names_unknown`), and a set left empty is the crates' (a link or a
    trip with no mode, left out there).
  - **A cell** is read as its field: text from a string, or from a whole number written in
    decimal (an integer id `101`, or a float `101.0`, is `"101"`); an integer (`lanes`,
    `departure_s`) from an integer in the field's range, a float with no fraction, or a string
    of one; a float from an integer, a float or a string of one; a boolean is no number. Where a
    column above allows a null, `None`, a float `NaN` and pandas' `NA` are null. A row with a
    cell that cannot be read otherwise (a fractional or negative second, 300 lanes, a null where
    none is allowed) is left out: `nodes_unread` (a node, with its links), `links_unread`,
    `trips_unread`, each naming the row and the column, warned once. A position that is a float
    `NaN` is a number: the graph leaves the node out, the demand the trip.
  - `classes`: a list of class ids (str).
  - **A Parquet file** holds the same columns, in any order, exactly those (the two optional
    columns of the links table optional there too), with the same null
    rules (a file's null is `None`), and gives the same rows, so a file and the same table in
    memory build the same network or demand (the same fingerprint): it is read whole by
    `crates/io-parquet` (`read_table`) and each column, in its own kind, becomes the same plain
    list of Python values a table in memory gives, read by the same cell rules. Column types:
    text (`Utf8`, `LargeUtf8`), integers (`Int8` to `Int64`, `UInt8` to `UInt64`, exact),
    floats (`Float32`, `Float64`), pyarrow's `null` type (all nulls); any other type (a boolean,
    a date) refused, naming the column and its type. Compression: none or snappy (pyarrow's
    default); any other codec refused, naming it.
- **Keywords.** Each row of `PARAMETERS.md`, at exactly one entry point, the one whose crate
  reads it; omitted, the default of `PARAMETERS.md`, held in Rust (the loop's
  `Parameters::defaults`; for the five values the graph, the GTFS reader and the demand crate
  take from their caller, the constants of `src/lib.rs`), never in Python:

  | Entry point | Keywords (unit) |
  |---|---|
  | `Network.from_osm`, `Network.from_tables` | `capacity_per_lane` (veh/h per lane), `bike_speed`, `walk_speed` (km/h) |
  | `Network.from_osm` alone | `osm_class_defaults`: only `None`, the reader's one table (`crates/io-osm/README.md`) |

  From an extract every car link has its class's capacity per lane × its lanes
  (`osm_class_defaults`), so `capacity_per_lane`, the graph's value for a car row without a
  capacity, changes nothing there; it is accepted and recorded at both entry points.
  | `Transit.from_gtfs` | `transfer_walk_max` (m) |
  | `Demand.from_table` | `stop_walk_max` (m) |
  | `run` | `boarding_slack` (s), `max_rides`, `nearby_stops`, `choice` (`"logit"` or `"deterministic"`), `coefficients` (a dict from every class id to its sixteen, in the order of the loop's choice row: cost per minute of car driving, bike riding, walking, transit in vehicle, access walk, egress walk, first wait, transfer wait, transfer walk; per transfer; per €; per unit of ln PS, kept as given in the run's record, read times `nest_scale` under `"logit"` and as 0 under `"deterministic"`; the car, bike, walk and transit constants), `nest_scale`, `step_exponent`, `iteration_cap`, `iteration_floor`, `link_bin` (s), `seed` (an integer from 0 to 2⁶³ − 1, the largest the run's record holds) |
- **`run`**: `window=(start_s, end_s)`, two seconds of the clock, required, never defaulted;
  `out`, the folder the files go to (created if missing). The demand must have been built on
  the network and the transit given to `run`.
- **Two files with fixed names.** `run.toml`, the settings, is written by the user and read by
  `run_file`; `run_record.toml`, the record, is written by a run beside its four tables and read
  by `Run.open`. A run never writes `run.toml`, so settings and results may share a folder. Each
  is found in the folder given; the path of a file of that name is taken as its folder. Refused,
  saying what to give instead, nothing written: a file of any other name (`ValueError`; a path
  where nothing is, `FileNotFoundError`), a folder without the file (`FileNotFoundError`), and,
  given to `run_file`, a folder that holds a run's record and no `run.toml` (or the record's own
  path). A run into a folder that already holds a
  finished run (its `run_record.toml`) is told in one `UserWarning` naming the folder, and its
  files are replaced.
- **`run_file(folder=None, *, out)`**: the settings file `run.toml` in `folder` (none: the
  current folder) run into the folder `out`, returning a `Run`. It is read with Python's
  standard `tomllib`; it holds the keys of the record's part one: `format` (only 1; may be
  absent), `window_start_s` and `window_end_s` (required; the end given, which the run moves up
  to a whole `link_bin` when it is not one, a repair the record counts), `[parameters]` (a row
  absent is the default of `PARAMETERS.md`; `coefficients` one table per class, by column name,
  all sixteen required), the input files, `[network] osm_file` (with `[network] region`, the
  region to cut it to, if any), or `nodes_file` and `links_file`, `[demand] trips_file`
  (required), `[transit] gtfs_file` (required when `[transit]` is there), and `[demand]
  classes` (required). A key or table under `[parameters]` that is not one of these is refused
  by name; one elsewhere is ignored, all of them named in one `UserWarning`
  (`openmobisim_io_parquet::KEYS`); `stop_walk_max` or `transfer_walk_max` without `[transit]`
  is ignored, warned. Each file's path is joined to the folder of the settings file's canonical
  path. The network, the transit (when `[transit]` is there) and the demand are built by the
  functions of `from_osm` or `from_tables`, `from_gtfs` and `from_table`, each given the rows of
  `[parameters]` its entry point reads, and run by `run` with the rest. Minimal settings hold
  the window, the files and the classes alone. A record's part one, every line before
  `[result]`, is a settings file: copied into a `run.toml`, it runs the same run again. A whole
  record renamed `run.toml` (known by its `[result]`) is told in one `UserWarning` (write a
  settings file instead), and the run reads its part one alone, but for the inputs'
  fingerprints in `[result.network]`, `[result.demand]` and `[result.transit]`: an input whose
  fingerprint differs (an input file changed) is named in one more `UserWarning` with both
  fingerprints, and the run goes on; one it lacks, or that is not text, is not compared. A
  `result` that is not a table is a stray key, ignored and named. A settings file holds no
  fingerprint; nothing is compared.
- **`Run.open(folder, network=None, demand=None, transit=None)`**: the finished run in `folder`
  (its record, `run_record.toml`, its four tables beside it) opened without running anything,
  returning a `Run` like the one `run` returns. Each input not passed is rebuilt from the file
  the record names, by the one function `run_file` uses (`src/record.rs`, `inputs`); read from
  the record besides: the window, `window_start_s` to `[result] window_end_used_s` (required;
  the end the run used; the end given, `window_end_s`, is not read), these first, before any
  input is rebuilt, `link_bin` (absent: its default), `[result] stopped_by_rule` (absent:
  `Run.stopped_by_rule` is `None` and the pages say the run's end is not recorded),
  `[result.repairs]` and the inputs' fingerprints. A key or table either part never holds is
  ignored, all of them named in one `UserWarning`. The tables the pages read are read once,
  here. An opened run lacks nothing a fresh one has: `osm_report` and `gtfs_report` are its
  network's and transit's (rebuilt from the same files, or passed).

**The rule for a finished run (the owner's): refuse only what cannot be done; warn on the
rest.** `Run.open`, `Run.map` and `Run.summary` meet each condition so:

| Condition | Met by |
|---|---|
| a path that is neither a folder nor a file named `run_record.toml` (`run.toml` included) | refused (`ValueError`, or `FileNotFoundError` when nothing is there), saying what to give |
| the record missing (`FileNotFoundError`, saying what to give) or not TOML; a key under `[parameters]` it should not hold; `window_start_s`, `[result] window_end_used_s`, `[network]`, `[demand]` or `[demand] classes` missing | refused (`OSError` or `ValueError` naming it) |
| a key or table elsewhere a record never holds | `UserWarning` naming them all; ignored |
| an input given in memory when the run was made (no file in the record) and not passed (`Run.open`) | refused: `pass it: network=` (or `demand=`) |
| `[result] stopped_by_rule` not a boolean (`Run.open`) | refused, naming what it is |
| `[result] stopped_by_rule` missing | opened; `stopped_by_rule` is `None`, the summary says the record does not say what ended the run, the map's provenance "its end not recorded" |
| a demand passed that was not built on the network and transit used | refused, as for `run` |
| a table missing or unreadable, a column the pages need missing or of a type that cannot be read | refused naming the file (`OSError` when it cannot be opened, else `ValueError`); no page written |
| a row of `trips.parquet`, `links.parquet` or `routes.parquet` whose own trip or link names nothing the inputs hold | refused naming the file and the row |
| an input, passed or rebuilt, whose fingerprint differs from the record's | `UserWarning` naming it and both fingerprints; goes on with it |
| the record without an input's fingerprint, or one that is not text | `UserWarning` that it cannot be compared; goes on |
| a transit passed to a run that had none | `UserWarning`; the transit set aside |
| a table's columns in another order, other columns beside them, a list's inner field named `element` | read as written, no warning |
| a trip without a row in `trips.parquet` | read as not run (the run drops the trips its window cannot hold and writes no row for them); `UserWarning` only when their number is not the record's count of trips the run dropped |
| a trip's second row; an unknown mode | `UserWarning`; the first row used; read as none |
| a `links.parquet` row off the record's grid or outside its window, or with cars on a link that does not allow cars; a (link, bin) given twice | `UserWarning` with a count; the row set aside; the first used |
| a trip whose legs are misnumbered, of an unknown kind, lack a value, name a link, run or stop not held, or do not agree with its mode | `UserWarning` with a count; the trip left out of the journeys, counted in both footers |
| a walk gap or transfer walk without its seconds | `UserWarning`; the journey shows its door-to-door time and no leg split |
| an `iterations.parquet` without `would_change_share` (a run written before that share was reported) | `UserWarning` naming the column; the summary shows the share as unknown (`—`), as it does for a least-cost run, which does not report it |

Each warning names the file and the first five trips or links with their count. A warning given
by `Run.open` is given again by each page, which reads the tables again.

## Outputs
`Network`, `Transit` and `Demand` objects (opaque, immutable; each keeps the values it was
built with, the three build values, `transfer_walk_max`, `stop_walk_max`, for the run's
record) and, from `run`, `run_file` and `Run.open`, a `Run`: `trips`, `links`, `iterations`,
`routes` (the paths of `trips.parquet`, `links.parquet`, `iterations.parquet` and
`routes.parquet`, written by `crates/io-parquet`, whose README gives every column; `routes`
one row per trip and leg of what each took at the last iteration), `record` (the path of
`run_record.toml`, the record of what was run: part one the settings, every parameter at the
value used, defaults included, the window given, the input files and the classes; part two,
`[result]`, the end of the window used, how the run ended, the counts and fingerprints of the
network, the demand and the transit, the readers' counts and every repair; its keys in
`crates/io-parquet/README.md`),
When an input was read from a file (an OSM extract, a GTFS feed, a nodes, links or trips table
given as a path), the record names it (`osm_file`, `gtfs_file`, `nodes_file`, `links_file`,
`trips_file`; an extract cut to a region, `region` beside `osm_file`, as the box's four numbers
or the polygon's points), relative to the record's folder from the canonical paths of both, with `/`
between components (on Windows, a file on another drive than the record is recorded by its
absolute path, `D:/…`, its canonical path without the verbatim prefix `\\?\`, and a file on a
network share by its canonical path as it is, `//?/UNC/server/share/…`, Windows' long form of
`\\server\share\…`; both measured to run again and open, "Systems and user rights" below); a table given in memory has no key, and a record without the files of its network and its demand cannot run again. `run_file` writes the record of the run it ran, with the
fingerprints of the inputs as they are now.
`stopped_by_rule` (bool: the stopping rule, else the cap, ended the run; `None` for an opened
run whose record does not say), `osm_report` (the OSM reader's `Report` of its network, as a
dict, `None` for a network from tables), `gtfs_report` (the GTFS reader's of its transit, `None`
without transit) and `repairs`.

**Repairs.** What a reader or the run set right instead of refusing is shown as one
`UserWarning` per kind and call, `"<entry point>: <count> <what was done> (<key>; first: <the
first five ids>)"`, and kept as dicts, key to count: `Network.repairs` (the OSM reader's, or this
crate's table reader's and the graph's), `Transit.repairs` (the GTFS reader's), `Demand.repairs`
(the table reader's and the demand crate's) with `Demand.trips_given` (the rows of the trips
table), and `Run.repairs`, `{"network": …, "demand": …, "transit": … (with transit), "run": …}`
(the loop's: the window). When nothing is left, one more warning says so plainly: "no trip
is left: N given, all dropped (by reason: …)" from `Demand.from_table` and from `run` (the
summary's first tile says the same); "no link is left" from `Network.from_*`; "the network has
no car link: N trips that ask for a car cannot drive" from `Demand.from_table`; "no run is
left" from `Transit.from_gtfs`. The work goes on: a run of no trip writes its files and pages.
The record writes each as `[result.network.repairs]`, `[result.demand.repairs]`,
`[result.transit.repairs]`, `[result.repairs]`, and `[result.demand] trips_given` (when the
demand dropped some); the summary's first tile
says how many trips were given and how many dropped, by reason. The keys: this crate's table
reader's (`columns_ignored`, `nodes_unread`, `links_unread`, `trips_unread`,
`mode_names_unknown`, `road_classes_unread`, and `shapes_unread` for a shape it cannot read,
the graph's key, `src/tables.rs`) and those of `crates/io-osm`, `crates/graph`,
`crates/io-gtfs`, `crates/demand` and `crates/loop` (each README's "Repairs").

`Run.map(path, title=None)` (a `str` or an `os.PathLike`) writes the run's interactive map, one
self-contained HTML file (`crates/map`, whose README gives what it shows and its data layout),
titled `title`, else with the file's stem (so the file's name enters its bytes), replacing a file there, and
returns `path`. A `path` that is one of the run's five files or one of its input files (canonical
paths compared) is refused, a `ValueError` naming it, before anything is written; the page is
written to a sibling `.<name>.partial` then renamed, so a write that fails (an `OSError`) leaves no
part of a page at `path`.

`Run.summary(path, title=None)` writes the run's summary, one self-contained HTML page of its
numbers (`crates/summary`, whose README gives every section), titled `title`, else with the
file's stem, with the same
refusal and the same partial write as `Run.map` (one check for both). A `Run` keeps nothing of
the run in memory: its paths, `stopped_by_rule`, its window and bin, and its network, demand
and transit by reference (the objects the caller holds, or those `Run.open` and `run_file`
rebuilt), none of them Python attributes. Each page reads what it needs from the run's files
and inputs when it is written: the doors, classes and departures from the demand; the mode, time
and finished from `trips.parquet`; the legs from `routes.parquet`; the links table from
`links.parquet`; the iterations from `iterations.parquet` (the summary); the record's text from
`run_record.toml` (both footers); and a `Transit`'s GTFS names beside its timetable (`lines`: the
stops' names, the runs' lines and kinds). The map does not read `iterations.parquet`. A table
that cannot be read is refused naming it, nothing written; what the readers set aside is shown
as warnings (the table above). The pages of an opened run and of the fresh run are the same
bytes.

## Units
Those of `PARAMETERS.md` and the crates: km/h, veh/h, metres, seconds; positions in WGS84
degrees or metres; times of day in seconds of the clock.

## Meanings and assumptions
- Nothing is computed here: every number is a crate's. The binding reads tables and keywords
  into the crates' rows and values, calls the crates, and writes the run with `io-parquet`.
- No signals from tables: a node or link read from a table is never signalised (the graph
  stores signals and applies them nowhere).
- The interpreter's lock is released while a file is read, a network, timetable or demand is
  built, and a run runs and is written; the threads are rayon's global pool
  (`RAYON_NUM_THREADS` sets it).
- No command line, configuration file, scenario object, plot, cache, log or progress output.

## Errors
- A keyword that is not a row of `PARAMETERS.md`, or that another entry point reads, and a
  missing `window` or `out`: `TypeError` naming it, as Python refuses an unexpected keyword.
- A keyword's value that cannot be read (a string for a speed, a seed above 2⁶³ − 1, a choice model that is not one,
  a class without coefficients or a coefficient row for no class, `osm_class_defaults` other
  than `None`), a missing column, a demand built on another network or transit: `ValueError`
  naming it.
- A boolean given for a keyword or inside one (a coefficient, a second of `window`): no keyword
  is a boolean, and Python would read `True` as 1; `ValueError` naming it, the value and what
  to give instead (`seed: True is a boolean; this parameter is a number, give 1 instead`).
- A table's Parquet file: a column of a type not read (not text or a number), a column
  name held twice, a codec other than snappy, a file that is not Parquet: `ValueError` naming
  the table, the file and the column or the codec; a file that cannot be opened or read, or a
  directory: `OSError`.
- `run_file`: a key or table under `[parameters]` a settings file never holds, a missing required key, a value of the wrong type, a boolean
  for any key read (TOML's `true` is never a number here), a `format` other than 1, both
  `osm_file` and table files: `ValueError` naming it. These are checked before any input is
  built; a parameter's value (a negative seed, say) is checked by the entry point that reads
  it, the same as for a keyword, so it is refused once the inputs read before that entry point
  are built. A settings file that is not TOML: `tomllib.TOMLDecodeError` (a `ValueError`).
- `run_file` and `Run.open`: a path of a file of another name than theirs, a folder holding a
  run's record and no `run.toml` given to `run_file` (see "Two files with fixed names").
- `run` and `run_file`: an `out` whose `trips.parquet`, `links.parquet`, `iterations.parquet`,
  `routes.parquet` or `run_record.toml` is one of the run's input files (canonical paths compared): `ValueError` naming
  the file, before the loop runs (`openmobisim_io_parquet::replaces_no_input`, the check
  `write` also makes), so an input is never replaced by a result and the mistake costs no run.
  `run_file` into the settings' own folder writes the record beside `run.toml`, which no run
  writes. A known limit:
  paths are compared, not files, so an input that is a hard link of one of the five output
  names in `out` is not recognised and would be replaced.
- A crate's refusal: `ValueError` carrying the crate's message unchanged; a file that cannot
  be read or written (the record or a file it names included): `OSError`. A panic is pyo3's
  `PanicException` (unwinding, never an abort).

## Bikes
A bike is a mode like any other at every entry point: `bike` in a link's `modes` lets bikes
ride it (a cycle path is a link `bike;walk` with no car field), `bike` in a trip's `modes` opens
the bike alternative, `bike_speed` sets the one speed of every bike. A bike's trip and its
counts per link and bin come back in the two files like a car's. Tested: bike trips equal
networkx's shortest lengths on a grid with streets one-way for bikes alone, and `bike_speed=30`
halves a ride. `run_record.toml` records the `bike_speed` the network was built with under
`[parameters]`, and `run_file` on its part one gives it to the network again. From a file, a link's or a
trip's `bike` is text in its `modes` cell like any other mode: a bike's ride of 1 500 m from
three files takes its hand 360 s, and a rerun from the record's part one repeats a run with
bikes byte for byte (on Delft, with a bike trip).

## Systems and user rights
Measured on GitHub's runners (`windows-latest`, `ubuntu-latest`, `macos-latest` on Apple
silicon), the wheel installed, Python 3.14:
- **Windows, a standard user**: a new local account in the group Users alone (`whoami /groups`:
  no Administrators, medium integrity), started by `Start-Process -Credential`, working in
  folders it owns. The binding's tests pass as they do for the administrator, but for the two
  symbolic-link cases of `test_run_file.py`, skipped with the reason when the user may not
  create a link (152 passed and 5 skipped, against 153 and 4). A run from three
  table files, run again by `run_file` from its record's part one (its `trips.parquet` and
  `links.parquet` the first run's bytes) and opened by `Run.open` (its map and summary the fresh run's bytes), works in one
  folder; with the inputs on one drive and the run on the other (C: and D:, both ways); with the
  inputs, the run or both on a network share named `\\localhost\<share>\…`; with the run's
  folder or the inputs reached through a directory junction (`mklink /J`, which needs no right);
  and through a symbolic link where the user may create one. Without developer mode (the
  default) a standard user may not create a symbolic link (`WinError 1314`); the package never
  creates one. `wheels.yml` runs the binding's tests as such a user, developer mode off, on
  every release.
- **Linux and macOS**: the tests of `ci.yml` and `wheels.yml` run as the runner's account
  (`runner`, not root, no `sudo`), which may use `sudo` (and on macOS is in the group admin).
  Once by hand as a new account without either: every test of the suite as it is now passed.
- **Not measured**: a domain account, a share on another machine, Windows on ARM, a path over
  260 characters.

## Tests
`pytest crates/binding/tests` on the extension built in release (`maturin develop --release -m
crates/binding/Cargo.toml` in a virtual environment with pytest, pyarrow 26.0.1, networkx 3.7).
Each expected value comes from outside the simulator. The pages (the map and the summary) are
written and checked as files; their behaviour in a browser is not tested automatically.
- `test_hand_run.py`: io-parquet's hand run rebuilt from Python tables; its three files pass
  `crates/io-parquet/tests/read_back.py` (pyarrow, the hand numbers) unchanged.
- `test_summary.py`: `Run.summary` on the hand run writes the file and returns its path, the same
  bytes on a second call and in another folder, no address, the hand numbers by their `data-k`
  names (7 trips, 5 finished, 1 still travelling, 1 without an alternative, 9.0 vehicle-km, 1.5 km
  cycled, 3.0 km walked, three iteration rows, the iteration cap), the record shown; the run's
  four files and an input file refused by name, unchanged; a folder an `OSError`, no partial file.
  A title given names both pages (escaped), else the file's stem;
  the refusal holds with a title.
- `test_map.py`: `Run.map` on the hand run writes the file and returns its path, the same bytes on
  a second call and in another folder, no `http://`, `https://` or `//host.domain` address, the
  stem as its title; a folder as the path is an `OSError`, no partial file left; the run's four
  files and an input file refused by name, unchanged.
- `test_drawing.py`: the links table's `shape` and `road_class` on the hand run (st1 bent
  through (750, 300) m, a primary road; ab a cycleway; st2 residential): the fingerprints and the
  trips the plain run's; the map's data hold the bend (7 500 and 3 000 dm) and the classes
  (version 1's order: 16, 4, 11), the same from a file as in memory; a point in
  longitude/latitude at a node's position held at that node's decimetres; the summary's car map
  along the bend, mitred, by hand; a shape not points (with a point not
  finite, or the number 3.5) drawn straight, a road class `True` none (`road_classes_unread`), its map data that of st1 without a shape, counted and warned once; a
  misspelt `shapes` column ignored, counted and warned once.
- `test_open.py`: `Run.open` as a property: the hand run from files opened with nothing passed
  (each input rebuilt) gives a map and a summary equal, byte for byte, to the fresh run's, its
  four tables unchanged; from tables in memory, passed; the tram run (a GTFS feed, its tables
  files) opened with nothing passed and with its network passed. Warned, going on: a network of
  another fingerprint (a bike speed of 16 km/h, both fingerprints named; the page then shows the
  tables' journeys, 7 trips in 2 pairs as the fresh page, on the network passed, its classes), a transit passed to a
  run without one (set aside), a record without the network's fingerprint (then the fresh run's
  pages), a key a record never holds under `[result.network]` (ignored, named; the pages the
  fresh run's). Opened, the pages saying so: a record without `stopped_by_rule`. Refused by name: a
  network or a demand given in memory and not passed, a `stopped_by_rule` that is not a
  boolean, `links.parquet` with a column renamed, a
  removed `routes.parquet` (`OSError`); a fresh run's map with its `trips.parquet` removed
  (`OSError`, no page). `routes.parquet` rebuilt by `run_file` from the record's part one is the
  same bytes; an `out` holding
  an input named `routes.parquet` is refused, the input unchanged.
- `test_read_tables.py`: the hand run's tables changed by pyarrow, each warned and drawn from
  what agrees, by hand: the bike said to have driven, its routes rows removed, or given twice:
  left out (the warning naming it, both footers 1, pair a->b 3 travellers); a walk gap without
  its seconds: the bike's legs' seconds unknown, its 360 s kept; links rows doubled: the first
  used, the pages unchanged; `link_bin` 60 made 120 with a bike entering ab at 25 260 s: that row
  set aside; the window moved 30 s: all 5 rows set aside; routes rewritten by pyarrow's defaults
  (inner field `element`) with the columns reversed: the same pages; an empty route: an empty
  list, both pages written.
- `test_files.py`: tables as Parquet files written by pyarrow from hand values: the hand run
  from three files passes `read_back.py` and has the fingerprints of the same tables in memory;
  `test_defaults.py`'s bike (360 s), walk (750 s) and car (100, 101, 102 s) cases from files;
  every column type read (`LargeUtf8`, `Int32`, `UInt32`, `UInt64`, integers for floats) gives
  the plain tables' fingerprints; snappy (checked in the file's metadata) and no compression
  read, zstd refused by name; a text column of numbers, a `Float32` and whole floats for an
  integer give the plain tables' fingerprints; one case each refused by name: a boolean column,
  a missing column, a column name held twice (refused in memory too, by pyarrow); a file not
  Parquet (`ValueError`), an absent file, a directory, an empty path, an unreadable file
  (`OSError`); one case each repaired, by hand: 300 lanes, a `UInt64` above 2⁶³ − 1, a null
  departure, a null mode set leave their row out (the record's links or trips one fewer,
  the trips given 7), an extra column ignored, a column of nulls alone where none is allowed
  leaves all three links out; each counted and warned once, naming the row and column; a node
  row with a null `x_m` leaves the node and its three links out. A column of nulls
  alone where nulls are allowed reads as in memory (same fingerprint, the bike's 360 s). Files
  of several row groups and batches (a 33 × 33 grid: 1 089 nodes, 4 224 links, 1 100 trips)
  give those counts and the fingerprints of the tables in memory.
- `test_run_file.py`: each recorded path, joined to the record's folder by `os.path.join`, is
  the file given (`os.path.samefile`), relative with `/`: inputs by absolute path, by relative
  path, through a symbolic link, `out` four levels down, `out` through a symbolic link to a
  folder at another depth; no key for a table in memory. On Windows, which removes a `..` before
  it follows a link, the two symbolic-link cases check instead that each path is relative with
  `/` and that the record's part one runs again by `run_file` (the tables the first run's
  bytes) and the run opens by `Run.open`; for a user who may not create a symbolic link they are skipped, the reason
  given. The case
  of `test_record.py` from files with transit and a non-default value for every row a record
  holds, its record's part one run again by `run_file` into a sibling folder: `trips.parquet`,
  `links.parquet` and `run_record.toml` byte-identical, `iterations.parquet` equal but for its five wall-clock columns; the
  same after the whole tree is moved; at another depth the tables identical and only the four
  file lines of the record different. The same rerun on Delft from its OSM extract cut to
  version 1's box and its feed, the record holding the region (about a second; not gated).
  Hand-written settings of the window, the files and the classes alone run `test_defaults.py`'s stop case (2 475 s; no journey at 1 501 m) and records every
  parameter at the default parsed from `PARAMETERS.md`. A record renamed `run.toml`: one
  `UserWarning` saying so; one link, one trip or one call changed in its file: one more, for
  that input alone, with both fingerprints, the run complete, only its fingerprint changed; the
  record's part one alone: no warning, warnings as errors. An `out` holding an input under each
  of the four output names (`run_record.toml` among them) refused naming it for `run` and
  `run_file`, the input byte-identical after, and before the loop (with a window the loop
  refuses, the overwrite refusal is the one raised); inputs of other names beside the run kept;
  `run_file` into the settings' own folder, `run.toml` unchanged. Each refusal of a settings
  file by name, one case each, a boolean for each kind of number included. A key a settings
  file never holds outside `[parameters]` (eight cases, `[result]` among them: the record
  renamed), and `stop_walk_max` or `transfer_walk_max` without `[transit]`: one warning naming
  it, the record the same bytes as the minimal settings' run. The folder, the path of its
  `run.toml` and no folder (the current one) give the same record; refused, saying what to
  give, nothing written: a folder holding a run's record and no `run.toml`, the record's path,
  a file of another name (both functions), `run.toml` given to `Run.open`; a run into a folder
  holding a finished run: one warning naming it, the record replaced; a folder holding
  `run.toml` alone: no warning. One window end, by hand: settings ending at 35 950 s in bins
  of 300 s from 28 800 s record `window_end_s = 35950` and `window_end_used_s = 36000` (24
  bins), the move counted; edited to 36 300 s (25 whole bins), the next run ends there, nothing
  moved.
- `test_repairs.py`: the run's repairs by hand: a trip departing after a 1 800 s window
  dropped (`trips.parquet` the other 6, the record's `trips` 7 (no `trips_given`: the demand
  dropped none) and
  `[result.repairs] trips_outside_window = 1`, the summary's "of 7 given, 1 dropped", `Run.open`
  without a warning); a window of 1 801 s in 60 s bins ending at 27 060 s, the record keeping
  27 001 s given as `window_end_s` and 27 060 s as `[result] window_end_used_s`, and `run_file`
  on its part one repeating the repair (the same record bytes); a door 5 000 m
  from the network, 3 750 s on foot, after a 3 600 s window: walking taken out and the trip
  dropped; nothing left, each warned plainly and run on: every trip outside the window (the tile
  "no trip is left"), every class unknown, every link without modes, no car link, a feed whose
  run calls at no stop held.
- `test_networkx.py`: 30 bike and 30 walk trips on a jittered 12 × 12 grid, 15 % of its streets
  one-way for bikes: each `travel_time_s` is `ceil(g_o / w) + L / v + g_d / w` with `L`
  networkx's shortest length, within 1e-6 s (the derivation in the test).
- `test_parameters.py`: every row of `PARAMETERS.md`, parsed, accepted at exactly its entry
  point and refused at the others; the refusals by name, a boolean's with what to give instead
  (for a keyword of three entry points, a coefficient and the window); an extra column ignored
  and a mode name read trimmed and lower-cased, an unknown one left out (`bicycle`: the trip
  dropped with no mode), each counted and warned; a crate's message unchanged; `bike_speed=30`
  gives 180 s for the 1 500 m that take 360 s at 15 km/h.
- `test_defaults.py`: the five defaults held in `src/lib.rs`, each by a hand case run with no
  keyword: a 1 500 m ride in 360 s (15 km/h); a 1 000 m walk in 750 s (4.8 km/h); three cars
  through a two-lane link 1 s apart (1 800 veh/h per lane); a stop at 1 499 m reached and one at
  1 501 m not (1 500 m); two lines joined by a 399 m walk and not by a 401 m one (400 m).
- `test_delft.py`: Delft's extract and feed read through Python, the feed's counts against
  gtfs-kit's (`crates/io-gtfs/tests/fixtures/delft/expected.csv`), the read times printed;
  the extract cut to version 1's box and to the same box as a polygon of its corners: one
  network (the same counts and fingerprint), smaller than the whole; the same polygon given as
  lists accepted, a run cut to it run again by `run_file` and opened by `Run.open` on the same
  link count; eight regions that are no area each refused by name (three of them polygons of
  three points that repeat or lie on one line).
- `test_10_delft.py`: Delft with signals ignored, the whole run through Python; release only,
  skipped unless `TEST_10=1` (about a minute; not in CI); the file's header says what it judges.
- `test_determinism.py`: a logit run of 400 trips twice and at `RAYON_NUM_THREADS` 1 and 8:
  `trips.parquet` and `links.parquet` byte-identical.
- `test_record.py`: `run_record.toml` read with Python's `tomllib`. The names under `[parameters]` are
  the rows of `PARAMETERS.md` (parsed) less `osm_class_defaults`, and less `transfer_walk_max`
  and `stop_walk_max` without transit; each unit comment is the row's Unit cell; with no
  keyword every value is the default written in the test from `PARAMETERS.md`. A run with
  transit and non-default keywords at every entry point, rebuilt and rerun from the input
  tables and values read from its record alone: `trips.parquet`, `links.parquet` and
  `run_record.toml` byte-identical, `iterations.parquet` equal but for its five wall-clock
  columns, the three fingerprints equal, `[result.transit.gtfs_report]` equal to
  `Run.gtfs_report`. One link, one trip or one call changed changes its own fingerprint alone.
  `run_record.toml` byte-identical
  twice and at `RAYON_NUM_THREADS` 1 and 8, on the run of `test_determinism.py` and on the case
  with transit. Class ids with a quote, a backslash, control characters and a letter outside
  ASCII read back unchanged. Delft from its OSM extract and feed: `source = "osm"`, and both
  readers' counts in the record equal to the `Run`'s.
- `test_record_values.py`: each keyword's value is the one used, read back from `run_record.toml`
  beside a hand number: `walk_speed` 5 km/h walks 1 000 m in 720 s; `capacity_per_lane` 900
  on two lanes lets cars out at 100, 102, 104 s; a 4 500 s walk unfinished in a one-hour
  window and at its door at 29 700 s in a two-hour one, the window recorded as given;
  `iteration_floor` 8 gives 8 rows, the rule alone 5, `iteration_cap` 3 gives 3, and
  `[result]` agrees with the rows and `Run.stopped_by_rule`; `transfer_walk_max` 300 against
  400 m keeps 0 against 2 footpaths of 375 m; `stop_walk_max` 1 000 against 1 200 m leaves a
  stop 1 100 m away out of reach (`without_stop` 1 against 0); `boarding_slack` 60 against
  120 s takes a run leaving 100 s after arrival or the next (400 against 1 300 s);
  `max_rides` 1 refuses a two-ride journey that 2 takes in 800 s; `nearby_stops` 1 against 2
  offers 1 against 2 journeys. `nest_scale` 0.5 against 1: of 2 000 identical travellers
  offered two transit journeys whose costs differ by 0.5, the share taking the cheaper within
  3 binomial standard deviations of the nested logit's `1 / (1 + exp(−0.5 / μ))`, 0.731
  against 0.622. `choice="deterministic"`: 1 000 travellers all take the cheaper bike over the
  walk, which the logit would do with probability about 1e-21. `seed` changes the logit
  choices of 400 travellers (the test says why it must). A seed of 2⁶³ refused before the run, 2⁶³ − 1 run
  and read back.
