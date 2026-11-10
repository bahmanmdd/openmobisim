# openmobisim-io-parquet

Writes the three tables of a run, trips, links by time bin and iterations, as three Parquet
files in a folder, and nothing else. Depends on `openmobisim-types`, `-graph`, `-loading` (the
links table's type), `-demand`, `-loop`, and `parquet` with `arrow-array` and `arrow-schema`
(version 59, no default features, the `arrow` feature alone: no compression codec, no C).

## Inputs
`write(run, network, demand, folder)`, no options:
- `run`: the loop's `Run`.
- `network`, `demand`: the `Network` and the `Demand` the run ran on; they give the external
  ids of the links, the trips and the user classes.
- `folder`: where the files go, created if missing; a file of the same name is replaced.

## Outputs
`Written`, one `Table` (its `path` and its `rows`) per file. Three files, one row per entity,
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
  `without_transit`, `without_stop` (UInt64), `switch_share`, `found_share`, `gap` (Float64, nullable), `search_s`, `choose_s`,
  `load_s`, `measure_s`, `assess_s` (Float64).

Each column's meaning is the loop's (`crates/loop/README.md`) and is written beside its schema
in `src/trips.rs`, `src/links.rs` and `src/iterations.rs`. A column that cannot be null is
marked so in the file's schema.

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
  of a trip without an alternative; `mean_trip_time_s` when no trip finished; `switch_share`,
  `found_share` and `gap` when no trip has an alternative, and `gap` when the least costs sum
  to 0. In the links table `car_time_s`, `first_exit_s` and `last_exit_s` are null where `cars`
  is 0 (the loading holds 0 for the time and `NaN` for the exits there).
- `mode` is null exactly for a trip without an alternative.
- A crossing is filed under the bin its vehicle entered the link in, with its exact exit, even
  past the window's end (the loading crate's `LinkBins`).
- The writer's settings are the `parquet` crate's defaults: no compression, dictionary
  encoding, Parquet format 1.0, one row group up to 1 048 576 rows. The files' metadata holds
  the writer (`parquet-rs version 59.3.2`) and the Arrow schema, nothing else: no path, no
  name, no time stamp, so a second write of the same run is the same bytes.

## Errors
`WriteError`: a demand whose trip count is not the run's (`NotTheRunsDemand`), a network
lacking a link the run names (`NotTheRunsNetwork`), both checked before anything is written;
the folder or a file that cannot be created or written (`Io`); the Arrow or Parquet writer's
refusal (`Parquet`).

## Bikes
A bike's count is its own column, `bikes`, beside `cars` and `walkers` in each (link, bin) row:
the bikes that entered the link in the bin, each counted once. Bikes have no time column,
since a bike crosses a link at its free-flow time by definition (the loading crate). A trip by
bike has `mode` `bike` and its door-to-door time like any other mode.

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
- The refusals: a demand and a network not the run's, a folder that cannot be made.
- **Sioux Falls** (`tests/sioux_falls.rs`, ignored in the debug suite): the loop's regular test
  case (48 814 trips) run and written; the row counts must be the run's and the write under
  1 s in release (CI runs it):
  `cargo test --release -p openmobisim-io-parquet --test sioux_falls -- --ignored --nocapture`.
