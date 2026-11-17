# openmobisim-io-gtfs

Reads one GTFS feed into the transit crate's `Timetable` of one service day, the busiest
weekday, with the footpaths between its stops derived once on the walk network. Depends on
`openmobisim-types`, `openmobisim-graph`, `openmobisim-transit`, `openmobisim-demand` (the
nearest walk node and the bounded walk search, reused, not copied), `zip` (deflate through
flate2's pure-Rust zlib-rs backend, no C) and `csv-core`.

## Inputs
`read(path, &network, transfer_walk_max)`, no options:
- `path`: one GTFS feed, a `.zip` with the files at its root or in one folder inside it.
  Read: `stops.txt`, `trips.txt`, `stop_times.txt` (required); `calendar.txt`,
  `calendar_dates.txt` (either or both); `frequencies.txt`, `transfers.txt` (when present).
  Nothing else (no shapes, fares; routes only for the names below).
- `lines(path)`, beside the timetable, for the run's pages: `stops.txt`'s `stop_name`,
  `routes.txt`'s `route_short_name`, `route_long_name` and `route_type`, `trips.txt`'s
  `route_id`. The timetable, its fingerprint and the transit crate do not read them.
- `network`: the `Network` built from longitude/latitude: its projection places the stops in
  its plane, its walk links and walk speed give the footpaths.
- `transfer_walk_max`: the longest footpath between two stops, metres (`PARAMETERS.md`,
  400 m; this crate is its only reader).

## Outputs
`(Timetable, Report)`. The timetable's stops are the feed's stops by `stop_id`; its runs are
named by `trip_id`, `<trip_id>@<seconds>` for a frequency run, `<name>@previous-day` for a
night run; its footpaths are the derived walks. The `Report` counts: `date` (`YYYYMMDD`),
`trips` in service that day, `runs`, `calls`, `night_runs`, `frequency_runs`, `stops`,
`trips_untimed`, `trips_single_call`, `times_interpolated`, `pickup_drop_off_ignored`,
`frequency_rows_skipped`, `footpaths`, `transfers_applied`, `transfers_skipped`.

`Lines` from `lines(path)`: `stop_names` (each named stop's name by `stop_id`; a blank name is
none) and `trip_lines` (each trip's `Line`, by `trip_id`: its route's `route_short_name`, else
`route_long_name`, else `route_id`, and its `Kind`). `stop_name(stop)` reads a stop's name;
`line_of_run(run)` a run's line, the run named as `read` names it (the trip, less
`@previous-day` and `@<seconds>`). `Kind` is rail, metro, tram, bus, ferry or other, from
`route_type` as release 0.1 read it: 0, 5 and 900–999 tram; 1, 12 and 400–699 metro; 2, 100–199
and 300–399 rail; 3, 11, 200–299 and 700–899 bus; 4, 1000–1099 and 1200–1299 ferry; anything
else, or a type that is not a number, other. Nothing is refused there but a feed that cannot be
opened or read: a missing file or column, a trip of an unknown route, leave the names unknown.

## Units
Seconds after the service day's midnight, whole (`Second`); a time past 24:00:00 is kept
(`25:10:00` is 90 600 s). Metres in the network's plane. Footpaths in seconds (`Duration`),
metres over the network's walk speed, never rounded.

## Meanings and assumptions
- **The service day** is the busiest weekday: the Monday to Friday with the most trips in
  service (rows of `trips.txt` whose service runs that date by `calendar.txt` and every
  `calendar_dates.txt` exception), over the feed's whole range of dates, the earliest on a
  tie. A feed with no Monday to Friday in service (weekend-only) is read on its busiest day of
  all seven, the same rule, noted (`weekday_absent`). No other day can be asked for.
- **Stops**: every row of `stops.txt` with `location_type` 0 or blank and a position, called
  at or not, projected with the network's projection. None is dropped by area.
- **Runs**: every trip in service that day, its calls in `stop_sequence` order. A blank
  arrival or departure is the other; a call with neither is interpolated between the timed
  calls around it by `shape_dist_traveled` when every call between has one, else by
  straight-line distance in the plane, rounded to the second. A trip whose first or last
  call has no time, or of one call, is dropped and counted.
- **`pickup_type` and `drop_off_type` 1** are counted and ignored: the transit crate's `Call`
  has no boarding or alighting flag, so such a call is open to both.
- **Frequencies** (`frequencies.txt`, `exact_times` 0 and 1 alike): a listed trip is a
  template, its times read from its first departure; it runs from each row's `start_time`
  every `headway_secs` while before its `end_time`, one run each; the template is not a run.
- **Night runs**: the runs of the day before (its trips, frequency runs included), keeping
  the calls that depart at or after 24:00:00 of that day, shifted by −86 400 s (an arrival
  before 24:00:00 becomes 0); a run left with two calls or more is a run of the day. The
  part of such a run before midnight is not in the day.
- **Footpaths**: each stop is linked to its nearest walk node by straight line (the demand
  crate's `NodeIndex`); from each stop's node the demand crate's bounded walk search
  (`NearStops`) gives the walk to every node within `transfer_walk_max`. A footpath from stop
  `a` to stop `b` is `a`'s gap, the walk between their nodes and `b`'s gap, kept when at most
  `transfer_walk_max` metres (read to a part in a billion, as the demand crate reads its
  limit); its seconds are its metres over the walk speed. The bound is applied in the search:
  nothing is filtered afterwards.
- **`transfers.txt`**: a row with `transfer_type` 2 (`min_transfer_time`) or 1 (zero),
  between the two stops of a footpath, with no route or trip: the footpath's seconds become
  the larger of the walk's and the row's. Every other row is skipped and counted.
- One day, nothing else: no service changes, realised times, routes or vehicles.

## Errors
`GtfsError`, refusing only where no timetable can be read: `transfer_walk_max` not a finite
number at or above zero; a network built in metres (`NetworkInMetres`: no projection places a
stop); the file cannot be read (`Io`) or is no zip (`Zip`); a required file (`MissingFile`) or
a required column of `stops.txt`, `trips.txt`, `stop_times.txt`, `calendar.txt` or
`calendar_dates.txt` (`MissingColumn`) is missing; no day of the feed's dates has a trip
(`NoService`).

## Repairs
Every row that cannot be read is set right or left out and noted in `Report::repairs` by kind,
with its count and the first five ids (the Python binding warns once per kind; the record
writes the counts, `[transit.repairs]`). Every row is read, whichever day its trip or service
runs.

| Key | What | What is done |
|---|---|---|
| `stops_skipped` | a stop whose `location_type` is not blank or 0 to 4, whose id is empty, or whose position is not a number within ±180° and ±84° | skipped; its calls are then `calls_dropped` |
| `stops_repeated` | a stop id given twice | the first kept |
| `trips_repeated` | a trip id given twice in `trips.txt` | the first row (its service) kept; the calls of both are one trip, as `stop_times.txt` cannot tell them apart |
| `calendar_rows_skipped` | in `calendar.txt` a date that is not one (the row skipped), a weekday flag other than 0 or 1 (read as 0), a service given twice (the first kept); in `calendar_dates.txt` an exception type other than 1 or 2 or a date that is not one (the row skipped), a service's date given twice (the first kept) | as said |
| `weekday_absent` | no Monday to Friday has a trip (a weekend-only feed) | the busiest day of all seven read, its date the id |
| `stop_times_skipped` | a row of `stop_times.txt` whose trip `trips.txt` does not hold | skipped (such a trip is never in service) |
| `calls_dropped` | a call at a stop not held (absent, a station, or skipped above), or with a `stop_sequence` that is not a whole number | the call dropped; a run left with one call is dropped and counted as `trips_single_call` |
| `times_unread` | a time that is not `H:MM:SS` | read as blank: the other time, else interpolated |
| `distances_unread` | a `shape_dist_traveled` that is not a finite number | read as blank |
| `columns_absent` | `arrival_time` or `departure_time` absent as a column | read as blank in every row |
| `files_unread` | `frequencies.txt` or `transfers.txt` without one of its columns | not read |
| `runs_repeated` | two runs of one name (two overlapping `frequencies.txt` periods of a trip give `<trip>@<seconds>` twice) | the first kept |
| `trips_without_call` | a trip in service that day or the night before left with no call at all (none in `stop_times.txt`, or every one dropped above); the same trips `trips_single_call` counts the one-call ones of, but its own key | no run |
| `trips_out_of_order` | a run with a call departing before it arrives, or arriving before the call before it departs (also what a repeated `stop_sequence` leads to) | the run dropped |
| `footpaths_none` | no link of the network allows walking | no footpath between stops |

Rows that do not apply are not errors and are skipped and counted: a `frequencies.txt` row
with a bad time or headway or an end not after its start (`frequency_rows_skipped`), a
`transfers.txt` row that is not a stop-to-stop minimum time of a footpath
(`transfers_skipped`). A stop of `location_type` 1 to 4 is not a stop; a trip dropped for an
untimed end or a single call is counted (`trips_untimed`, `trips_single_call`).

## Bikes
Nothing here is a bike: a run carries passengers on its timetable, and a footpath is a walk on
the walk network. `bikes_allowed` is not read. A bike-and-ride journey, later, is an access by
bike to the transit query, not a footpath.

## Tests
`cargo test -p openmobisim-io-gtfs`. Each expected value comes from outside the crate:
- `tests/lines.rs`, a hand feed: a stop's name and a blank name; each line's name from the short
  name, the long name or the route id; the kinds of `route_type` 0, 3, 2, 715, 1200, 6 and a type
  that is not a number; a trip of an unknown route; a frequency run's and a night run's line read
  back from their names; a feed without `routes.txt` (stops named, no line).
- Hand feeds, each value derived from the GTFS reference: the busiest weekday from a calendar
  and its exceptions (2026-10-07: four trips) and on a tie (the earliest, 2026-10-01); the
  fixed bug, a calendar from 2020 to 2030 with three more trips from 2026-10-05, gives
  2026-10-05; a feed of exceptions alone; interpolation by shape distance 0, 1 000, 3 000,
  4 000 m over 08:00 to 08:10 at 08:02:30 and 08:07:30, by straight line at 08:01:00 and
  08:04:00; `25:10:00` = 90 600 s; frequencies 07:00 to 08:00 every 1 200 s then 08:00 to
  08:30 every 900 s, five runs and no template, a row ending before it starts skipped; the fixed bug, a trip of the day before at
  26:00 a run at 7 200 s, counted; the dropped trips counted; each kind of unreadable row set
  right or left out, counted under its key naming its id, the feed still read (20 cases); two
  overlapping frequency periods of one trip (08:00 to 09:00 and 08:30 to 09:30 every 10 min:
  three names given twice, 9 runs, not 12); a weekend-only feed read on its first Saturday,
  2026-10-03; a run going back in time dropped; a trip whose every call is at a stop not held
  (`trips_without_call`); the refusals.
- Footpaths on a hand network: stops on a walking street at 0, 300 and 800 m at 4.8 km/h:
  300 m is 225 s, 500 m out at 400 m, 375 s and 600 s at 1 000 m, exactly 300 m in at 300 m
  and out at 299 m (the fixed bug: the footpath set is bounded as it is built);
  `transfers.txt` raising 225 s to 300 s, the rows skipped; a `transfers.txt` without
  `transfer_type` not read (`files_unread`, A to B 225 s); a network with one car link and no
  walk link: no footpath (`footpaths_none`), the run read.
- Delft: the feed `tests/fixtures/delft/gtfs.zip` (614 735 bytes, a cut for Delft of
  OVapi's Dutch national feed) on the Delft network of `crates/io-osm/tests/fixtures/delft/`: the
  date, trips, runs, calls, night runs and stops against `fixtures/delft/expected.csv`, written
  by `tests/delft_gtfs.py` with gtfs-kit 14.1.1 (pandas 3.1.1): 2026-10-09, 1 707 trips and
  runs, 39 156 calls, 0 night runs, 1 340 stops. The read time is printed and must be under
  3 s in release. Release only (about 7 s in debug, skipped by `cargo test`; CI runs it):
  `cargo test --release -p openmobisim-io-gtfs --test delft -- --ignored --nocapture`.
