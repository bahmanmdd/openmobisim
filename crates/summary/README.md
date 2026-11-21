# openmobisim-summary

The summary of a finished run: one self-contained HTML page of its numbers, and nothing else.
Depends on `openmobisim-types`, `-graph`, `-transit`, `-loop` (the iterations table's row),
`-io-gtfs` (the names beside the timetable), `-io-parquet` (a leg as read back, `RouteLeg`) and
`-map` (what the pages keep of a run, the transit
hops and boardings, the themes, the display rules and the delay rule, the mark, the clock, the
escape, the partial-then-rename write); no external crate. The page of release 0.1's summary, harvested onto
release 0.2's data.

## Inputs
`write(&Summarised, title, path)`, no options. `Summarised` is the run, as plain data read back
from its files: `kept` (the map's `Kept`: per trip its doors, departure, user class, the mode
and the legs it took, its realised time and whether it finished; the links table's rows; the
window), the `network` and the `timetable` (or none) it ran on, the GTFS reader's `lines` beside
the timetable (or none), its `iterations` table (`iterations.parquet` read back), whether the stopping rule ended it (`stopped_by_rule`), and its
record, `run.toml`, as text. The page is written to a sibling `.<name>.partial`, then renamed to
`path` (`openmobisim_map::save`): a write that fails leaves no part of a page at `path`. Through the
binding, `Run.summary(path)` titles it with the file's stem.

## Outputs
One HTML file: its style, its numbers, its charts and maps as inline SVG, and one short script, the
Paper / Night button. It names no address and loads nothing; the same run written twice under the
same title gives the same bytes. Every number a test reads carries a `data-k` name (`trips`,
`mode.car.share`, `dep.07:00.all`, `band.0.km`, `busy.0.link`, `it.3.gap`, `km-cycled`, ...). The
`<body>` says `data-state="ready"` once the script has run. Its sections, in order:
- **Header**: the mark and the title, as on the map; the version, the window and the trips.
- **Tiles**: trips (with the window; when the record says trips were dropped, "of N given, M
  dropped: …" by reason, `data-k` `given` and `dropped`, led by "no trip is left:" when none is); completed (%); not completed (%), naming the trips still
  travelling at the window's end and those without an alternative; the mean trip (min) and the total
  travel time (h) of the finished trips; on the roads, vehicle-km and the mean speed (vehicle-km over
  the vehicle-hours of the cars' crossings); the run time (s, the five stages summed over the
  iterations; reading the inputs and writing the files left out, as the tile says).
- **Modes**: the share bar and the table (trips, share %, completed %, mean trip min, travel time h),
  a row for the trips without an alternative; then, when there are any, those trips by user class.
- **When people left**: departures by quarter hour of the clock and mode, stacked bars and the table;
  a quarter hour the window cuts (the window starting or ending inside it) is marked `*` and said
  under the table.
- **On the roads**: vehicle-km by delay band, share bar and table; the entries of cars into links
  per bin of the window, as a line (a car counts once on every link it enters, as the caption says); the five busiest links by volume (external id, length m, volume veh/h,
  delay %).
- **Transit, bikes and walking**: boardings; rides per transit trip, with the share that change
  vehicle; kilometres cycled; kilometres walked on the network.
- **Convergence**: the mean trip by iteration and the iterations table (mean trip min,
  switched %, offered %, found cheaper %, would change %, unfinished trips, duality gap %), and which of
  the stopping rule and the iteration cap ended the run (or that the record does not say, for a
  record without `stopped_by_rule`). Would change is, under the logit, the share of the trips
  with an alternative that the logit, choosing again at the costs the iteration measured, would
  put on another alternative than the one they hold; the duality gap is reported under the
  least-cost choice. Each is reported, never judged, and the one a model does not report shows
  as `—`, as every missing value of the page does. Under the least-cost choice the routes in use
  cost the same at the equilibrium, so the share of travellers not on their lowest-cost
  alternative does not settle, and the gap, which weighs each traveller by what it would save,
  is the measure. Under the logit the model puts a share of travellers on an alternative that is
  not their lowest-cost one, so the gap does not reach zero, and the share who would change is
  the measure.
- **Computation time**: the five stages (search, choose, load, measure, assess), seconds summed over
  the iterations; the network's links, nodes and kilometres by mode, and its timetable's stops and
  runs.
- **Settings**: the record, `run.toml`, whole, as text.
- **Maps**, one per mode with travellers: car traffic by delay band, as wide as the road's capacity;
  cyclists, dashed; walkers, dotted, each 1.5 + 8 √(min(n, 3 000) / 3 000) units wide for n
  over the window (the map's rule, `FLOW_FULL`: full at 3 000 and more, the same for both);
  transit hops by travellers on board, in the map's colour of
  their kind of service (rail, metro, tram, bus, ferry, other), a legend entry per kind. Static SVG, the base
  network (every line once whatever its direction) defined once and reused by every map; links
  along their shapes (straight without one), each direction on the right of its travel, mitred at
  a bend as on the map; hops straight between their stops; whole units of a frame 2 000 wide; one
  path per colour, width and dash.
- **Footer**: where the numbers come from; the OpenStreetMap credit when the run's record says the
  network was read from OpenStreetMap, as on the map; and, when the run's files left trips out of
  the journeys, how many (`data-k` `left-out`: "N trips left out of the journeys: their routes
  do not agree with the trips table").

## Units
Trips and counts; minutes for trip times, hours for travel time and vehicle-hours, seconds for the
run's stages; kilometres and metres; veh/h for volumes; km/h for speeds; percent where a column
says so. Numbers in the map's format: commas between thousands (`65,959`).

## Meanings and assumptions
- **Completed** is a trip at its door before the window's end; **still travelling** one with an
  alternative that is not; **without an alternative** one no mode open to it reaches (counted, not
  simulated). The three add up to the trips. Mean trip and travel time are over the finished trips.
- **Delay over free flow** is the map's one rule (`openmobisim_map::rules::delay`), applied per link
  and bin for the bands and over the window for the busiest links and the car map, in the map's
  four bands and colours; the time beyond free flow is the cars' crossing times times it. A link's vehicle-km in a
  bin are its cars times its length. The busiest links: the cars that entered over the window over
  its hours, and their delay over the whole window.
- **Boardings** are the map's count (`openmobisim_map::ridden`): the rides boarded over the window
  by every journey taken; **rides per transit trip** the boardings over the trips that took transit;
  the share that change vehicle, the trips whose journey has two rides or more. Kilometres cycled and walked are the bikes
  and walkers on links times the links' lengths: a walk between a door and the network or a stop is
  not on a link, so transit's walks are not in them.
- **Charts**: each value axis runs from 0 to the least of 1, 2 or 5 times a power of ten at or above
  the largest value, with grid lines at 0, half and the top, each labelled with its value exactly.
- **Car map widths**: a ribbon is as wide as its capacity over the 99th percentile of the car
  links' capacities, the rule the map's script applies (the same rule in two places).
- **Themes**: the map's tokens (`openmobisim_map::theme`, one definition): surface, inks, the
  network's grey, the mark, the modes' colours, the delay bands; the cards tinted with the
  network's grey, 12 px radius, uppercase muted headings, the map's fonts. Light and dark follow
  the reader's system until the button chooses one. The charts' line and bars are drawn in the
  transit colour, the accent of the old page.

## Errors
`SummaryError`: a journey without a timetable or not of it (`Map`, the map's refusal); a file that
cannot be written (`Io`).

## Bikes
Bikes are a mode of every table (share, completion, mean trip, departures), their kilometres a row
of their own, their links a map of their own (dashed, as on the map), and a bike link counts in the
network's table.

## Tests
`cargo test -p openmobisim-summary`, every value read by its `data-k` name against a hand number,
each run's four tables written by `openmobisim_io_parquet::write` and read back first, as the
binding draws a page (`tests/common/mod.rs`, `kept`):
- `tests/hand_run.rs`: io-parquet's hand run: 7 trips, 5 finished (71.43 %), 1 still travelling,
  1 without an alternative; the mean trip 7.4 min (2 226 s over 5) and 0.6 h; 9.0 vehicle-km at
  43.7 km/h (741 s of crossings); the modes table and the trips without an alternative by class;
  the departures by quarter hour; all 9.0 vehicle-km in the band 0–15 %; the busiest links `st1` and
  `st2` (3 veh/h, 0.7 % and 2.0 %); 1.5 km cycled and 3.0 km walked; three iteration rows (7.42 min,
  offered and found 100 % at iteration 0, nobody switching, gap 0) ended by the iteration cap; the
  network's links, nodes and kilometres by mode; the maps present; two writes the same bytes; no
  address; the record shown escaped.
- `tests/hand_run.rs` also: the charts' grid lines at 10, 5, 0 (departures, 6 at most), 5, 2.5, 0
  (cars entering links, 3 a bin at most) and 10, 5, 0 (the mean trip, 7.42 min), each label its
  line's value; the static maps' coordinates, sides of travel and widths by hand (the base network
  once, st1 8 units wide 5 below the line, st2 6 wide 4 below, the bike's line, of one cyclist,
  1.5 wide dashed 3.6 and 2.25, the walkers' (two) 1.5 wide dotted). `src/maps.rs` (unit tests):
  a line along its points, and moved to the right of travel, mitred at a right-angle bend by hand.
  Through the binding (`test_drawing.py`): the car map along a bent link, by hand.
- `tests/hand_run.rs` also: the hand run's links table with the row of `st2` (3 cars) given to
  `ab`, which does not allow cars: the reader sets it aside with one warning and the page is built
  from the rest, 3 × 2 km = 6.0 vehicle-km on `st1`, all in the band 0–15 %, `st1` the one
  busiest link.
- `tests/transit.rs`: a hand timetable: 5 boardings, 1.25 rides per transit trip, 25.0 % changing
  vehicle; 5 stops, 2 runs; the transit map, and no kilometres walked on links.
- `tests/transit.rs` also: with a tram and a bus line, the transit map's hops in the tram's and the
  bus's colours with their legend; the OpenStreetMap credit for a network from OpenStreetMap, none
  for one from tables.
- Through the binding (`crates/binding/tests/test_summary.py`): `Run.summary` writes the file and
  returns its path, the same bytes twice, the hand numbers; the run's files refused (the check of
  `Run.map`); in headless Chrome or Chromium where one is found, the page is ready (its script ran)
  and its button switches the surface between the two themes.
