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
  Nothing else (no routes, shapes, fares).
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

## Units
Seconds after the service day's midnight, whole (`Second`); a time past 24:00:00 is kept
(`25:10:00` is 90 600 s). Metres in the network's plane. Footpaths in seconds (`Duration`),
metres over the network's walk speed, never rounded.

## Meanings and assumptions
- **The service day** is the busiest weekday: the Monday to Friday with the most trips in
  service (rows of `trips.txt` whose service runs that date by `calendar.txt` and every
  `calendar_dates.txt` exception), over the feed's whole range of dates, the earliest on a
  tie. No other day can be asked for.
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
`GtfsError`, refusing, never repairing: `transfer_walk_max` not a finite number at or above
zero; a network built in metres (`NetworkInMetres`); the file cannot be read (`Io`) or is no
zip (`Zip`); a required file (`MissingFile`) or column (`MissingColumn`) is missing; a row
that cannot be read, naming its file, its id and the value, as the OSM reader refuses an
unreadable tag: a bad value (`BadValue`: in `stops.txt` a position that is not a number or
outside longitude ±180° and latitude ±84°, an empty stop id, an undefined `location_type`; in
`calendar.txt` a date that is not one or a weekday flag other than 0 or 1; in
`calendar_dates.txt` a date that is not one or an exception type other than 1 or 2; in
`stop_times.txt` a time that is not `H:MM:SS`, a sequence or shape distance that is not a
number), an id given twice (`RepeatedId`: a stop, a trip, a service's calendar, a service's
exception on one date), a call at a stop that is not a stop with a position (`UnknownStop`),
a row of `stop_times.txt` whose trip `trips.txt` does not hold (`UnknownTrip`, naming the trip
and the row's `stop_sequence`; the reference requires every stop time's trip to exist).
Every row of these files is read, whichever day its trip or service runs. No weekday has a
trip (`NoWeekdayService`); no link allows walking (`NoWalkLinks`); the transit crate's builder
refuses a run (`Timetable`, naming the run: a call departing before it arrives or before the
call before it).

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
- Hand feeds, each value derived from the GTFS reference: the busiest weekday from a calendar
  and its exceptions (2026-10-07: four trips) and on a tie (the earliest, 2026-10-01); the
  fixed bug, a calendar from 2020 to 2030 with three more trips from 2026-10-05, gives
  2026-10-05; a feed of exceptions alone; interpolation by shape distance 0, 1 000, 3 000,
  4 000 m over 08:00 to 08:10 at 08:02:30 and 08:07:30, by straight line at 08:01:00 and
  08:04:00; `25:10:00` = 90 600 s; frequencies 07:00 to 08:00 every 1 200 s then 08:00 to
  08:30 every 900 s, five runs and no template, a row ending before it starts skipped; the fixed bug, a trip of the day before at
  26:00 a run at 7 200 s, counted; the dropped trips counted; each kind of unreadable row
  refused with the error naming its file, id and value (20 cases); the other refusals.
- Footpaths on a hand network: stops on a walking street at 0, 300 and 800 m at 4.8 km/h:
  300 m is 225 s, 500 m out at 400 m, 375 s and 600 s at 1 000 m, exactly 300 m in at 300 m
  and out at 299 m (the fixed bug: the footpath set is bounded as it is built);
  `transfers.txt` raising 225 s to 300 s, the rows skipped.
- Delft: the feed `tests/fixtures/delft/gtfs.zip` (614 735 bytes, a cut for Delft of
  OVapi's Dutch national feed) on the Delft network of `crates/io-osm/tests/fixtures/delft/`: the
  date, trips, runs, calls, night runs and stops against `fixtures/delft/expected.csv`, written
  by `tests/delft_gtfs.py` with gtfs-kit 14.1.1 (pandas 3.1.1): 2026-10-09, 1 707 trips and
  runs, 39 156 calls, 0 night runs, 1 340 stops. The read time is printed and must be under
  3 s in release: `cargo test --release -p openmobisim-io-gtfs --test delft -- --nocapture`.
