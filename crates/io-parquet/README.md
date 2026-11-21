# openmobisim-io-parquet

Writes the four tables of a run, trips, links by time bin, iterations and routes, as four
Parquet files in a folder, and the run's record, `run.toml`, beside them; reads the four back,
each by the inverse of its writer; and reads an input table (nodes, links or trips) from a
Parquet file; nothing else: the folder alone tells what was run, the folder and the run's
inputs give its pages (the binding's `Run.map`, `Run.summary`, `Run.open`), and, when every
input was read from a file, the record alone runs again (the binding's `run_file`). Depends on `openmobisim-types`, `-graph`, `-loading` (the links table's type),
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
- `given`: `Given { source, files, build_values, stop_walk_max, transit, parameters,
  network_repairs, trips_given, demand_repairs }`, what
  else the run was given, every value as it was used, defaults included: `Source::Osm(&report)`
  or `Source::Tables`; `Files { osm, nodes, links, trips, gtfs }`, the canonical path
  (absolute, symbolic links resolved) of each input read from a file, `None` for one given in
  memory; the network's `BuildValues`; the demand's `stop_walk_max` (m); `None` or
  `Some(Transit { timetable, report, transfer_walk_max })` (the GTFS reader's timetable, its
  counts and the `transfer_walk_max` it was read with, m); the loop's `Parameters`; what the
  network's readers repaired (the OSM reader's, or the tables' reader's and the build's), the
  trip rows given, and what the demand's readers repaired or dropped, for the record. The bike
  and walk speeds of `build_values` must be the network's (it keeps them in m/s, from which
  km/h is not recovered exactly, hence the second copy); `capacity_per_lane` is not kept by the
  network and cannot be checked.
- `folder`: where the files go, created if missing; a file of the same name is replaced. Every
  refusal comes before anything is written: the inputs checked against the run, every journey
  taken checked against the timetable given (`NotTheRunsJourney`), the record and the trips,
  links and iterations tables built. Then a `run.toml` already there is removed, the three
  tables written, the routes table streamed from the run in batches of 8 192 trips (it refuses
  nothing; only an input or output failure can stop it), and the new record written last, so a
  folder holding a record holds the complete run it describes: a write that fails part way
  leaves no record.

`read_table(path)`: one Parquet file read whole, an `InputTable`: `names()`, its column names in
the file's order, and `column(name)`, the column's cells in row order, of the column's own
kind, `None` for a null: `Cells::Text` (from `Utf8` or `LargeUtf8`), `Cells::Integer` (`i128`,
exact, from `Int8` to `Int64` and `UInt8` to `UInt64`) or `Cells::Float` (`f64`, from `Float32`,
widened exactly, or `Float64`); a column of Arrow's `Null` type (pyarrow's type for a column of
nulls alone) is all nulls, as the same column in memory is. Any other type (a boolean, a date, a
list) is refused naming the column and its type. Which columns a table holds, which may be null,
how a cell of one kind is read as a field of another and what a row means is the caller's
contract, the same for a file as for a table in memory (`crates/binding/README.md`); this crate
decodes. Compression read: none and snappy; any other codec is refused by name before a page is
decoded.

`read(folder, demand, network, timetable, window, dropped)`: what the run's pages read of the tables
`write` wrote into `folder`, for the run of `demand` on `network` and `timetable` over `window`
(a `StepGrid`, the bins of the links table), as `Tables { trips, links, routes, warnings }`.
`read_iterations(path)` reads the fourth (the summary's alone). One reader per file, each
public: `read_trips`, `read_links`, `read_routes`, `read_iterations`; each finds its columns
by name (another order, or other columns beside them, read the same) and reads only those the
pages need:
- `TripRows`, per trip of the demand in its order: `mode` (none without an alternative),
  `travel_time` (s, `NaN` for null), `finished`, `present` (false for a trip without a row). Not read: `class_id`, `departure_s` (the
  demand's), `arrival_s`, `cost`, `alternatives`.
- `Links`, per (link, bin) row in link then bin order: the link, the bin's index, `cars`,
  `car_time` (s; a null, no car, as 0 s), `bikes`, `walkers`. Not read: the exit times.
- `Routes`, per trip of the demand in its order: its legs (`RouteLeg`: `WalkGap(seconds)`,
  `Route { mode, links }`, `Ride(Ride)`, `TransferWalk(Footpath)`, the transit crate's two
  types); none without an alternative; `None` for a trip left out of the journeys (below). Each
  distinct link id is looked up once.
- `Iteration`s, the loop's rows, in the file's order, and the warnings. A file without
  `would_change_share`, written by a run before that share was reported, opens with the share
  unknown (`NaN`) and a warning naming the column.
- `warnings`: what was found and set aside, each naming its file and the first five trips or
  links with their count, for the caller to show.

**The rule (the owner's): refuse only what cannot be read; warn on the rest.** A reader
refuses (`OutputError`, naming the file) only when the file is missing or not a readable Parquet
file, a column it needs is missing or of a type it cannot read (a list of text whatever its
inner field's name, `item` as written here or `element` as pyarrow writes it), or a row's own id
(its trip in `trips.parquet` and `routes.parquet`, its link in `links.parquet`) names nothing
the inputs hold, so the row cannot be placed. Everything else is a warning and the reading goes
on with what agrees:

| Found | What is done |
|---|---|
| a trip of the demand without a row in `trips.parquet` | read as not run (`present` false): the run writes no row for a trip it dropped; warned only when their number is not the record's count of trips the run dropped (`dropped`, the sum of `[result.repairs]` `trips_outside_window` and `trips_after_window`) |
| a trip's second row in `trips.parquet` | set aside, the first used |
| a `mode` that is not car, bike, walk or transit | read as none |
| a `links.parquet` row whose bin is off the record's grid or outside its window | set aside |
| a `links.parquet` row with cars on a link that does not allow cars (a run writes none) | set aside |
| a (link, bin) given twice | the first row used |
| a trip whose legs are misnumbered (not 0 to n − 1 once each), of an unknown kind, without a value their kind needs, or naming a link, run or stop the inputs do not hold | left out of the journeys |
| a `walk_gap` or `transfer_walk` without its seconds | its seconds unknown (`NaN`): the trip's legs show no split, its door-to-door time stays |
| a trip whose legs do not agree with its mode in `trips.parquet` (`agrees`: without a mode, no leg; car, bike or walk, exactly one route of that mode and no ride; transit, at least one ride and no route) | left out of the journeys |

A trip left out is in no pair of the map and in no transit count; both pages' footers say how
many ("N trips left out: their routes do not agree with the trips table"). Undetectable, said
once: a value changed in place (a time, a count, a stop, `stopped_by_rule`), and a link bin
replaced by a divisor of it (every bin of 60 s also starts on a grid of 30 s; 60 s replaced by
120 s is seen only where a bin starts on an odd minute). Bikes or walkers recorded on a link that
does not allow them are read as written: nothing in the pages depends on their speed there.

## Outputs
`Written`: one `Table` (its `path` and its `rows`) per Parquet file, and `record`, the path of
`run.toml`. Four tables, one row per entity,
external ids as strings, every column named with its unit, every time of day in seconds of the
clock, null where there is no value:
- `trips.parquet`, one row per trip run (a trip the run dropped, outside its window, has none), in trip id order (the external ids sorted bytewise):
  `trip_id` (Utf8), `class_id` (Utf8), `departure_s` (UInt32), `mode` (Utf8, nullable: `car`,
  `bike`, `walk`, `transit`), `arrival_s`, `travel_time_s` (Float64, nullable), `finished`
  (Boolean), `cost` (Float64, nullable), `alternatives` (UInt32).
- `links.parquet`, one row per (link, bin of entry) that someone entered, in link id order
  then bin: `link_id` (Utf8), `bin_start_s` (UInt32), `cars` (UInt32), `car_time_s`,
  `first_exit_s`, `last_exit_s` (Float64, nullable), `bikes`, `walkers` (UInt32).
- `iterations.parquet`, one row per iteration in order: `iteration` (UInt32), `mean_trip_time_s`
  (Float64, nullable), `finished`, `unfinished`, `without_alternative`, `without_transit`,
  `without_stop` (UInt64), `offered_share`, `switch_share`, `found_share`, `would_change_share`,
  `gap` (Float64, nullable), `search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s`
  (Float64). `would_change_share` is, under the logit, the share of trips with an alternative
  that the logit, choosing again over their own set at the costs the iteration measured, would
  put on another alternative than the one they hold (0 at the fixed point of the iterations),
  null under the least-cost choice; `gap` is the relative duality gap under the least-cost
  choice, null under the logit. Under the least-cost choice the routes in use cost the same at
  the equilibrium, so the share of travellers not on their lowest-cost alternative does not
  settle, and the gap, which weighs each traveller by what it would save, is the measure. Under
  the logit the model puts a share of travellers on an alternative that is not their lowest-cost
  one, so the gap does not reach zero, and the share who would change is the measure.
  (`crates/loop/README.md`, "Assess".)
- `routes.parquet`, what each trip took at the last iteration, one row per trip and leg, in
  trip id order then leg order; a trip without an alternative has no row, and routes nobody took
  are not written: `trip_id` (Utf8), `leg` (UInt32, from 0), `kind` (Utf8), `seconds` (Float64,
  nullable), `links` (List<Utf8>, nullable: the link ids in the order travelled), `run_id`,
  `board_stop`, `alight_stop` (Utf8, nullable), `board_s`, `alight_s` (UInt32, nullable,
  seconds of the clock), `from_stop`, `to_stop` (Utf8, nullable). The kinds, one plain list:
  - `walk_gap`: a walk between a door and the network or a stop, on no link, priced by its
    seconds: `seconds` holds them (constant over the run; 0 where the door is at the node or
    stop).
  - `car`, `bike`, `walk`: a route on the network: `links` holds its link ids (empty when the
    trip's two nodes are one); `seconds` is null: the loop keeps a trip's realised door-to-door
    time (`trips.parquet`), not each route's.
  - `ride`: one transit run from stop to stop: `run_id`, `board_stop`, `alight_stop`, `board_s`
    (the run's departure from its board stop), `alight_s` (its arrival at its alight stop);
    `seconds` null (the two clock seconds give it).
  - `transfer_walk`: a footpath of the timetable between two rides: `from_stop` (the stop left),
    `to_stop` (the stop reached) and `seconds`, the timetable's. Two columns of its own, so a
    ride's stops and a walk's never share a column. Where the next ride leaves from the stop the
    last one was left at there is no row.

  A car, bike or walk trip is a `walk_gap`, its route and a `walk_gap`; a transit trip a
  `walk_gap`, its rides (with the transfer walks between them) and a `walk_gap`.

Each column's meaning is the loop's (`crates/loop/README.md`) and is written beside its schema
in `src/trips.rs`, `src/links.rs`, `src/iterations.rs` and `src/routes.rs`; the writer and the
reader of a file share its one schema. A column that cannot be null is
marked so in the file's schema.

`run.toml` (TOML 1.0; its layout also in `src/record.rs`), keys in this fixed order:
- A first line `# ...` saying what the file is; `format = 1`; `openmobisim_version` (the
  package's version; a development version, ending `-dev`, does not identify the commit it was
  built from); `window_start_s`, `window_end_s` (seconds of the clock: the window the run
  used), and `window_end_given_s` only when the run moved the end given up to a whole bin (the
  end given, which `run_file` runs from, so it repeats the repair).
- `[parameters]`: each row of `PARAMETERS.md` the run read, by its name, at the value it used
  (one exception: the coefficient of `car_ln_path_size` is recorded as given; the logit reads it
  times `nest_scale`, the least-cost choice as 0, `crates/loop/README.md`), defaults included, in the table's order, each followed by its unit as a comment, the Unit
  column of `PARAMETERS.md` (`capacity_per_lane = 1800.0  # veh/h per lane`): the three build
  values, `stop_walk_max` and `transfer_walk_max` (both absent without transit, where nothing
  reads them), `boarding_slack`,
  `max_rides`, `nearby_stops`, `choice` (`"logit"` or `"deterministic"`), `nest_scale`,
  `step_exponent`, `iteration_cap`, `iteration_floor`, `link_bin`, `seed`. Then a comment and one table per class
  in class id order (neither without a class), `[parameters.coefficients."<class id>"]`, a key per column of the choice
  row (`openmobisim_loop::parameters::COLUMNS`). `osm_class_defaults` is never written: its one
  value is the OSM reader's one table (`crates/io-osm/README.md`).
- `[network]`: `source` (`"osm"` or `"tables"`), `nodes`, `links`, `fingerprint`, then
  `osm_file` and `region` (the region the extract was cut to: `[west, south, east, north]` for a
  box, `[[lon, lat], …]` for a polygon, in degrees; absent for the whole extract), or
  `nodes_file` and `links_file`, each absent for a table given in memory;
  `[network.osm_report]`, the OSM reader's five counts, absent for a network from tables;
  `[network.repairs]`, each kind of the network's readers' repairs with its count.
- `[demand]`: `trips`, `trips_given` (the rows of the trips table, only when the demand dropped
  some), `classes` (the class ids in id order), `fingerprint`, `trips_file` (absent for a table
  given in memory); `[demand.repairs]`.
- `[transit]`, absent without transit: `stops`, `runs`, `fingerprint`, `gtfs_file`;
  `[transit.gtfs_report]`, the GTFS reader's fifteen counts (`date` as GTFS writes it,
  `YYYYMMDD`); `[transit.repairs]`.
- `[result]`: `iterations` (the rows of the iterations table), `stopped_by_rule`;
  `[result.repairs]`, the run's (the window's).

A `repairs` table is written only when a repair of its kind was made, `trips_given` and
`window_end_given_s` only when they differ from `trips` and `window_end_s`: a run that needed
no repair has the record it had before repairs were counted.

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
reader to refuse any other. `format` stays 1 (nothing has been released):
the file keys joined tables that already existed, and the first line names the fourth table;
no key changed.

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
  `switch_share`, `found_share`, `would_change_share` and `gap` when no trip has an alternative;
  `would_change_share` under the least-cost choice; `gap` under the logit and when the least
  costs sum to 0. In the links table `car_time_s`, `first_exit_s` and `last_exit_s` are null where `cars`
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
`OutputError` from the four readers, naming the file: `io` when it cannot be opened or read
(missing, say); else a column it needs is missing or of another type, or a row's own trip or
link is not of the inputs given. Everything else a reader finds is a warning (the table above).

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
(`NotTheNetworksSpeeds`), a journey taken that rides a run or calls at a stop the transit given
does not have (`NotTheRunsJourney`, naming the trip), a value of the record that is a float not finite or an integer
above 2⁶³ − 1 (`NotWritable`, naming its key); a `trips.parquet`, `links.parquet`,
`iterations.parquet`, `routes.parquet` or `run.toml` in the folder that is one of `given.files` (canonical paths compared; a file or
folder not there yet is none), which the write would destroy (`WouldReplaceInput`, naming it;
the same check is public, `replaces_no_input(files, folder)`, for a caller to make before the
run, as the Python binding does; a known limit: paths are compared, not files, so an input
that is a hard link of one of the five output names is not recognised and would be replaced);
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
  three iterations. The test writes the four files, checks they exist with 7, 4, 3 and 18 rows,
  and that a second write is byte-identical, and reads each back by the crate's readers as
  written (the trips' modes, times and finished, the links rows, the iterations, each trip's
  legs agreeing with its mode), with no warning; then
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
- **Who would change** (`tests/hand_run.rs`): the hand run under the logit, where each set holds
  one alternative and nobody would change: `would_change_share` 0 at each of the three
  iterations, right after `found_share`, and `gap` null; under the least cost,
  `would_change_share` null and `gap` 0; each read with the input reader and read back by
  `read_iterations` as written, without a warning; the logit's file with the column taken out
  opens, every other value as written, the share unknown (`NaN`), one warning naming the column.
- **Cars on a link without cars** (`tests/hand_run.rs`): the hand run's links table with the row
  of `st2` (3 cars entering from 25 320 s) given to `ab`, which does not allow cars, and given a
  bike: it opens, that row set aside whole (its bike with its cars: 1 bike read in all, the hand
  run's), the three others read as written, and exactly one warning names it (`ab at 25320`).
- **Routes on a timetable** (`tests/routes.rs`): a trip walks 100 m to its stop (75 s), rides
  r0 from s1 (200 s) to s3 (400 s), walks the 30 s footpath to s3b, rides r1 from s3b (500 s)
  to s4 (600 s), walks 0 m: five rows read with the Arrow reader alone (kinds, seconds 75, 30
  and 0, the run ids, the four stops and the four clock seconds, the from and to stops), and
  every leg read back by `read_routes` by hand; the same 86 100 s later (a window not starting
  at 0, rides past midnight: 86 300 to 86 700 s of the clock); a journey on a timetable lacking
  its second run refused before anything is written (the folder not made); 8 191, 8 192 and
  8 193 walkers with two trips without an alternative at each side of the batch boundary: three
  rows per walker in trip id order, read back whole.
- The record's refusals: a coefficient that is not a number and a seed of 2⁶⁴ − 1, each named
  by its key; parameters, a transit, a bike speed and a walk speed not the run's; nothing
  written on any of them, and a refused write into a folder holding a complete earlier run
  leaves its five files byte-identical. A used folder whose `links.parquet` cannot be written (a folder of that name):
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
