# openmobisim-io-parquet

Writes the three tables of a run, trips, links by time bin and iterations, as three Parquet
files in a folder, and the run's record, `run.toml`, beside them; and reads an input table
(nodes, links or trips) from a Parquet file; nothing else: the folder alone tells what was run,
and, when every input was read from a file, the record alone runs again (the binding's
`run_file`). Depends on `openmobisim-types`, `-graph`, `-loading` (the links table's type),
`-transit`, `-demand`, `-loop`, `-io-osm` and `-io-gtfs` (their `Report`s, for the record), and
`parquet` with `arrow-array` and `arrow-schema` (version 59, no default features, the `arrow`
feature and the `snap` feature, the pure-Rust snappy codec, to read pyarrow's default
compression: no other codec, no C). The record is TOML written by hand: no TOML or
serialisation library.

## Inputs
`write(run, network, demand, given, folder)`, no options:
- `run`: the loop's `Run`.
- `network`, `demand`: the `Network` and the `Demand` the run ran on; they give the external
  ids of the links, the trips and the user classes, their counts and their fingerprints.
- `given`: `Given { source, files, build_values, stop_walk_max, transit, parameters }`, what
  else the run was given, every value as it was used, defaults included: `Source::Osm(&report)`
  or `Source::Tables`; `Files { osm, nodes, links, trips, gtfs }`, the canonical path
  (absolute, symbolic links resolved) of each input read from a file, `None` for one given in
  memory; the network's `BuildValues`; the demand's `stop_walk_max` (m); `None` or
  `Some(Transit { timetable, report, transfer_walk_max })` (the GTFS reader's timetable, its
  counts and the `transfer_walk_max` it was read with, m); the loop's `Parameters`. The bike
  and walk speeds of `build_values` must be the network's (it keeps them in m/s, from which
  km/h is not recovered exactly, hence the second copy); `capacity_per_lane` is not kept by the
  network and cannot be checked.
- `folder`: where the files go, created if missing; a file of the same name is replaced. A
  `run.toml` already there is removed after the record and the three tables are built (so
  every refusal comes before it) and before they are written, and the new one is written last, so a folder holding a record holds the complete run it describes: a write that
  fails part way leaves no record.

`read_table(path)`: one Parquet file read whole, an `InputTable`: `names()`, its column names in
the file's order, and `column(name, kind)`, the column's cells in row order, `None` for a null,
as `Cells::Text` (from `Utf8` or `LargeUtf8`), `Cells::Integer` (`i64`, from `Int32`, `Int64`,
`UInt32` or `UInt64`; a `UInt64` above 2⁶³ − 1 is refused as out of range) or `Cells::Float`
(`f64`, from `Float64` or one of those integer types, rounded to the nearest above 2⁵³), as
`kind` (`Kind::Text`, `Integer`, `Float`) asks; a column of Arrow's `Null` type (pyarrow's type
for a column of nulls alone) is all nulls of any kind, as the same column in memory is. Which columns a table holds, which may be null
and what a row means is the caller's contract, the same for a file as for a table in memory
(`crates/binding/README.md`); this crate decodes and checks each column's type against the kind
asked for. Compression read: none and snappy; any other codec is refused by name before a page
is decoded.

## Outputs
`Written`: one `Table` (its `path` and its `rows`) per Parquet file, and `record`, the path of
`run.toml`. Three tables, one row per entity,
external ids as strings, every column named with its unit, every time of day in seconds of the
clock, null where there is no value:
- `trips.parquet`, one row per trip in trip id order (the external ids sorted bytewise):
  `trip_id` (Utf8), `class_id` (Utf8), `departure_s` (UInt32), `mode` (Utf8, nullable: `car`,
  `bike`, `walk`, `transit`), `arrival_s`, `travel_time_s` (Float64, nullable), `finished`
  (Boolean), `cost` (Float64, nullable), `alternatives` (UInt32).
- `links.parquet`, one row per (link, bin of entry) that someone entered, in link id order
  then bin: `link_id` (Utf8), `bin_start_s` (UInt32), `cars` (UInt32), `car_time_s`,
  `first_exit_s`, `last_exit_s` (Float64, nullable), `bikes`, `walkers` (UInt32).
- `iterations.parquet`, one row per iteration in order: `iteration` (UInt32),
  `mean_trip_time_s` (Float64, nullable), `finished`, `unfinished`, `without_alternative`,
  `without_transit`, `without_stop` (UInt64), `offered_share`, `switch_share`, `found_share`,
  `gap` (Float64, nullable), `search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s`
  (Float64).

Each column's meaning is the loop's (`crates/loop/README.md`) and is written beside its schema
in `src/trips.rs`, `src/links.rs` and `src/iterations.rs`. A column that cannot be null is
marked so in the file's schema.

`run.toml` (TOML 1.0; its layout also in `src/record.rs`), keys in this fixed order:
- A first line `# ...` saying what the file is; `format = 1`; `openmobisim_version` (the
  package's version; a development version, ending `-dev`, does not identify the commit it was
  built from); `window_start_s`, `window_end_s` (seconds of the clock).
- `[parameters]`: each row of `PARAMETERS.md` the run read, by its name, at the value it used,
  defaults included, in the table's order, each followed by its unit as a comment, the Unit
  column of `PARAMETERS.md` (`capacity_per_lane = 1800.0  # veh/h per lane`): the three build
  values, `stop_walk_max` and `transfer_walk_max` (both absent without transit, where nothing
  reads them), `boarding_slack`,
  `max_rides`, `nearby_stops`, `choice` (`"logit"` or `"deterministic"`), `nest_scale`,
  `iteration_cap`, `iteration_floor`, `link_bin`, `seed`. Then a comment and one table per class
  in class id order (neither without a class), `[parameters.coefficients."<class id>"]`, a key per column of the choice
  row (`openmobisim_loop::parameters::COLUMNS`). `osm_class_defaults` is never written: its one
  value is the OSM reader's one table (`crates/io-osm/README.md`).
- `[network]`: `source` (`"osm"` or `"tables"`), `nodes`, `links`, `fingerprint`, then
  `osm_file`, or `nodes_file` and `links_file`, each absent for a table given in memory;
  `[network.osm_report]`, the OSM reader's five counts, absent for a network from tables.
- `[demand]`: `trips`, `classes` (the class ids in id order), `fingerprint`, `trips_file`
  (absent for a table given in memory).
- `[transit]`, absent without transit: `stops`, `runs`, `fingerprint`, `gtfs_file`;
  `[transit.gtfs_report]`, the GTFS reader's fifteen counts (`date` as GTFS writes it,
  `YYYYMMDD`).
- `[result]`: `iterations` (the rows of the iterations table), `stopped_by_rule`.

A fingerprint is the 64-bit FNV-1a hash of what the network, the demand or the timetable was
built from (each crate's README says exactly what it feeds), as 16 lower-case hexadecimal
digits. An input file's path is written relative to the folder the record is written in,
computed from the canonical paths of both, with `/` between components
(`../inputs/trips.parquet`); absolute only when no relative path exists (the file on another
drive; on Windows, a known limit, untested); a path that is not UTF-8 is refused
(`NotWritable`, naming its key), before anything is written. So the record's folder and its inputs move together, and a run written into a folder
at the same depth records the same paths. Not written: a time stamp, the machine, the thread
count; so a second write of the same run, and a run at any number of threads, give the same
bytes. `KEYS` lists every table of the record and the keys the writer may write in it, for a
reader to refuse any other. `format` stays 1: the file keys joined tables that already existed.

## Units
Seconds and counts; metric only. Every time of day, `departure_s`, `arrival_s`, `bin_start_s`,
`first_exit_s` and `last_exit_s`, is in seconds of the clock (seconds after midnight of the
run's day), so a file reads without the run's window: the writer adds the window's start to the
loop's seconds after it, once, here. For a finished trip `arrival_s = departure_s +
travel_time_s` on the clock. `travel_time_s`, `car_time_s` and the stage seconds are durations.
`cost` is in the choice model's unit: minutes in a time-only run. The shares and the gap are
fractions.

## Meanings and assumptions
- **Null, never `NaN`.** The loop's `NaN` means "no value" in exactly these places, written as
  null: `arrival_s` and `travel_time_s` of a trip unfinished or without an alternative; `cost`
  of a trip without an alternative; `mean_trip_time_s` when no trip finished; `offered_share`,
  `switch_share`, `found_share` and `gap` when no trip has an alternative, and `gap` when the least costs sum
  to 0. In the links table `car_time_s`, `first_exit_s` and `last_exit_s` are null where `cars`
  is 0 (the loading holds 0 for the time and `NaN` for the exits there).
- `mode` is null exactly for a trip without an alternative.
- A crossing is filed under the bin its vehicle entered the link in, with its exact exit, even
  past the window's end (the loading crate's `LinkBins`).
- In `run.toml` a float is written in Rust's shortest round-trip form with a point or an
  exponent (`1800.0`, `0.1`, `1e-7`), so TOML reads it back as a float of the same bits; an
  integer as an integer; a string or a quoted key as a basic string, the quote, the backslash
  and every control character escaped (`\uXXXX`). TOML has no null: a value that does not
  exist is an absent key or table.
- The writer's settings are the `parquet` crate's defaults: no compression (the `snap` feature,
  compiled for the reader, changes no default), dictionary
  encoding, Parquet format 1.0, one row group up to 1 048 576 rows. The files' metadata holds
  the writer (`parquet-rs version 59.3.2`) and the Arrow schema, nothing else: no path, no
  name, no time stamp, so a second write of the same run is the same bytes.

## Errors
`ReadError` from `read_table` and `column`: a file that cannot be opened or read, or a
directory (`Io`), a codec other than snappy (`Codec`, naming it), a column name held twice
(`TwoColumns`), a column the file lacks (`NoColumn`), a type not
read as the kind asked for (`Type`, naming the column and its Arrow type), a `UInt64` above
2⁶³ − 1 (`OutOfRange`, naming the column and the row), a file that is not Parquet or is damaged
(`Parquet`).

`WriteError`, every refusal checked before anything is written: a demand whose trip count is
not the run's (`NotTheRunsDemand`), a network lacking a link the run names
(`NotTheRunsNetwork`), a transit whose stop count is not the demand's (`NotTheRunsTransit`),
parameters with a coefficient row count other than the classes' or a link bin other than the
run's (`NotTheRunsParameters`), build values whose bike or walk speed is not the network's
(`NotTheNetworksSpeeds`), a value of the record that is a float not finite or an integer
above 2⁶³ − 1 (`NotWritable`, naming its key); a `trips.parquet`, `links.parquet`, `iterations.parquet` or
`run.toml` in the folder that is one of `given.files` (canonical paths compared; a file or
folder not there yet is none), which the write would destroy (`WouldReplaceInput`, naming it;
the same check is public, `replaces_no_input(files, folder)`, for a caller to make before the
run, as the Python binding does; a known limit: paths are compared, not files, so an input
that is a hard link of one of the four output names is not recognised and would be replaced);
the folder or a file that cannot be created or written (`Io`); the Arrow or Parquet writer's
refusal (`Parquet`).

## Bikes
A bike's count is its own column, `bikes`, beside `cars` and `walkers` in each (link, bin) row:
the bikes that entered the link in the bin, each counted once. Bikes have no time column,
since a bike crosses a link at its free-flow time by definition (the loading crate). A trip by
bike has `mode` `bike` and its door-to-door time like any other mode. In `run.toml` the bike's
one speed is `bike_speed` under `[parameters]`, and a link's bike permission and a trip's bike
are in the network's and the demand's fingerprints. Read from a file, a link's or a trip's bike
is text in its `modes` cell, read like any other text: the reader knows no mode.

## Tests
`cargo test -p openmobisim-io-parquet`. Each expected value comes from outside the simulator:
- **The hand run** (`tests/hand_run.rs`): seven trips on three links in a window from 07:00:00
  (25 200 s of the clock), so the clock and the window's seconds differ in every time column;
  every value derived by hand in the test's header: a bike that rides 1 500 m in 360 s, a
  walker at 1 125 s, a walker still walking at the window's end (arrival null, cost 1 125 s / 60 = 18.75 min), a
  trip without an alternative, three cars through two streets by the point queue (`st1` exits
  at 25 344, 25 346, 25 348 s; `st2` at 25 444, 25 448, 25 452 s, the bin from 25 320 s), in
  three iterations. The test writes the three files, checks they exist with 7, 4 and 3 rows,
  and that a second write is byte-identical; then
  `tests/read_back.py` reads them with **pyarrow 26.0.1** and checks every column's name,
  Arrow type and nullability and every value against the hand numbers written in the script,
  that no float is `NaN`, and that every finished trip's `arrival_s` is `departure_s +
  travel_time_s` exactly; it exits 1 on any difference (CI runs it):
  `python3 crates/io-parquet/tests/read_back.py target/tmp/io-parquet-hand-run`.
  The same folder's `run.toml` is read by `tests/read_record.py` with Python's standard
  `tomllib` (Python 3.11 or later): every key in its order and every value with its type
  against the numbers written in the script, the keywords the hand run gives, the defaults of
  `PARAMETERS.md` for the rest, the hand counts (3 nodes, 3 links, 7 trips), the version of the
  workspace's `Cargo.toml`, each fingerprint 16 lower-case hexadecimal digits; it exits 1 on any
  difference (CI runs it):
  `python3 crates/io-parquet/tests/read_record.py target/tmp/io-parquet-hand-run`.
  A second write of `run.toml` is byte-identical (`hand_run.rs`).
- The record's refusals: a coefficient that is not a number and a seed of 2⁶⁴ − 1, each named
  by its key; parameters, a transit, a bike speed and a walk speed not the run's; nothing
  written on any of them, and a refused write into a folder holding a complete earlier run
  leaves its four files byte-identical. A used folder whose `links.parquet` cannot be written (a folder of that name):
  the write fails and the earlier `run.toml` is gone. A run without a class: no coefficients
  comment or table.
- The refusals: a demand and a network not the run's, a folder that cannot be made.
- **The reader** is tested through the binding, on files written by pyarrow 26.0.1 from hand
  values (`crates/binding/tests/test_files.py`): the hand run above from three files passes
  `read_back.py`; every type read gives the plain table's fingerprints; snappy and no
  compression read, zstd refused by name; a wrong type, an integer out of range, a missing or
  extra column and a forbidden null refused by name. The paths of the record are tested there
  too (`test_run_file.py`): each, joined to the record's folder by Python's `os.path`, is the
  file given.
- **Sioux Falls** (`tests/sioux_falls.rs`, ignored in the debug suite): the loop's regular test
  case (48 814 trips) run and written, the record included; the row counts must be the run's
  and the write under 1 s in release (CI runs it):
  `cargo test --release -p openmobisim-io-parquet --test sioux_falls -- --ignored --nocapture`.
