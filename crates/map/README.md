# openmobisim-map

The interactive map of a finished run: one self-contained HTML file, and nothing else. Depends on
`openmobisim-types`, `-graph`, `-transit`, `-demand`, `-loop`, `-io-gtfs` (the names beside the
timetable, `Lines`), `-io-parquet` (the run's tables as read back: `Links`, `RouteLeg`), and `flate2` (no default features,
the pure-Rust `zlib-rs` backend: no C). Base64 and the page's JSON are written by hand.

## Inputs
- `Kept::of(demand, trips, routes, links, window)`: what the run's pages (this map and the
  summary, `crates/summary`) need, as plain data: the run's tables read back from its files by
  `openmobisim_io_parquet::read` (its `Trips`, its routes, its `Links`) and the `Demand` it ran
  on: per trip its two doors (the demand's origin and destination, metres in the network's
  plane), its departure and its user class (the demand's), the mode it took (none without an
  alternative), the legs it took (`routes.parquet`: walk gaps, a route's links, rides, transfer
  walks), its realised door-to-door time and whether it finished (`trips.parquet`); the classes'
  ids; the rows of the links table (`links.parquet`) and the window. Read through `trips()`,
  `classes()`, `rows()` and `window()`. Nothing here reads the loop's `Run`.
- Shared with the summary: `hops(kept, timetable, lines)`, the transit hops of every journey taken
  with their kind of service and their travellers (the transit layer's); `record::provenance` and
  `record::source`, the footer's lines read from the run's record; `theme::{PAPER, NIGHT}`, the two themes, one definition,
  with `Theme::bands()`, the delay bands' colours; `save(path, text)`, the write through a partial
  sibling renamed.
- `write(&Mapped, title, path)`: `Mapped` is the run: what the pages keep of it (`kept`), the
  `Network` and the `Timetable` (or none) it ran on, the GTFS reader's `Lines` beside the
  timetable (the stops' names, the runs' lines and kinds; or none) and its record, `run_record.toml`, as
  text; then the page's title, and the file to write (replaced if there). No options. The page is
  written to a sibling, `.<name>.partial`, then renamed to `path`: a write that fails leaves no
  part of a page at `path` and removes the sibling. Through the binding, `Run.map(path)` titles the
  page with the file's stem, so the file's name enters its bytes, and refuses a path that is one of
  the run's own files (`crates/binding/README.md`).

## Outputs
One HTML file holding its style, its script and its data: two `<script>` blocks, `meta` (JSON) and
`blob` (base64 of a zlib stream of typed arrays). It names no address and loads nothing: it opens
from disk in any current browser, with no server, network or library. A second write of the same
run under the same title gives the same bytes (no time stamp, no machine, no path; maps ordered by
id). The page's `<body>` reports what it read once drawn: `data-state="ready"` (or `"error"`, with
the message shown), `data-links`, `data-rows` (rows of car traffic), `data-pairs`,
`data-journeys`, `data-hops`.

What the page lets a reader do:
- switch the layers: road traffic, transit, cyclists, walkers;
- colour the road traffic by volume ÷ capacity (as it opens; delay when no car link has a capacity
  limit), delay over free flow or volume;
- hover or click a car link for its volume, its delay over free flow and the seconds each car loses
  there (times under 10 s to 0.01 s, from 10 s on to 0.1 s);
- show the whole window or one bin (the time slider), and play road traffic's bins in turn, one every
  0.45 s: Pause, the whole window ticked and road traffic hidden stop the play;
- choose a pair (every pair held is listed, the busiest first; ‹ › step through them), or pick its
  two places on the map; see its journeys, hover one to see it alone, click one to zoom to it,
  tick its modes on or off;
- click a link or a transit hop for its facts and the pairs that use it, then a pair to show it;
- zoom with the wheel, the buttons or the five levels (overview to detail), and fit the network;
- save the view as a PNG (twice the screen's resolution) or an SVG, with its title, subtitle and
  legend; the subtitle, on the page and in the exports, is road traffic's caption (the bins shown,
  the ribbons' width and colour), written only while road traffic is drawn, as each other layer's
  words are; a run without car traffic says "No car traffic in this run" there, its network drawn
  plain;
- switch to the night theme;
- keep the view (theme, colouring, bin, layers, pair, link, place and zoom) in the page's address
  after `#`, to come back to it or share it.

## Units
Positions in decimetres from the network's least x and least y (`meta.origin`, metres), as 32-bit
integers; lengths in metres; times in seconds (`meta.window_start` and the bins on the clock, a
journey's and a leg's time door to door or as a part of it); volumes in vehicles per hour, the
cars of the bins shown over their hours; capacities in vehicles per hour, all lanes; speeds in
km/h.

## Meanings and assumptions (the display rules, `src/rules.rs`)
- **A place** is a door position, exactly equal coordinates (−0 and +0 the same). **A pair** is the
  trips with the same origin and destination doors, numbered busiest first: most travellers, then
  the pair of the lowest trip id. The page holds the 2 000 busiest pairs; its footer says how many
  of how many, and how many of the trips they hold. A pair's travellers include those without an
  alternative, counted apart. A pair lists its six busiest journeys, all of them when asked.
- **A journey** within a pair is a mode with its exact link sequence, or, for transit, its rides as
  (pattern, boarding stop, alighting stop), numbered the same way within the pair. Its travellers
  are counted; its time is the mean realised door-to-door time of its travellers who finished
  (none: no time, and every leg 0 s). Its legs (a walk between a door and the network or a stop,
  a route, a wait, a ride, a walk between two stops) are those of its first traveller, each with
  its seconds averaged over the finished travellers: a car or bike leg is the realised time less
  the gap walks; the first wait runs from the end of the walk to the stop to the first departure,
  each later one from the end of the walk between rides, so the legs add up to the journey's time.
- **Road traffic** per bin, from the links table's cars: volume (cars over the bins' hours, veh/h,
  coloured by its square root up to the 99th percentile of the links' volumes, the middle of the
  legend's bar a quarter of that); the delay over free flow, `rules::delay`, the one definition
  (the script mirrors it, the summary reads it): the cars' mean crossing time ÷ the car free-flow
  time − 1, never below 0, banded 0–15 %, 15–50 %, 50–100 %, 100 %+ by `rules::delay_band`, a link
  whose cars lose under `rules::DELAY_FLOOR_S` (2 s) each in the first band whatever its delay (a
  link crossed in 0.433 s against 0.045 s free flow is 862 % over free flow and costs each car
  0.39 s), the legend saying so; bikes and walkers are drawn by their counts, never by delay; volume ÷ capacity from
  the busiest period's volume, banded 0–0.6, 0.6–0.8, 0.8–1.0, 1.0+, a car link without a capacity
  limit in the first band. The busiest period is fixed: the busiest run of `k = max(1, min(bins
  shown, round(900 s / bin)))` consecutive bins, named by its length `k` × bin (15 min for bins of
  300 s, 20 min for 600 s, 1 h for 3 600 s). Each link is drawn along its shape (the network's,
  straight between its two nodes without one): a ribbon and a cyclists' or walkers' line moved to
  the right of travel, mitred at a bend, a chevron at its middle segment, picked by its distance
  to the line; a journey's legs along their links' shapes. **One rule decides how roads are
  drawn: by road class when every link has one, else by capacity.** By road class: version 1's
  19 classes in order (motorway 0 … service 13, pedestrian 14, footway 15, cycleway 16, ferry,
  busway; `road` drawn as unclassified, steps, path, track and corridor as footway, any other name
  as unclassified); the page draws the classes up to a limit that grows as it zooms in (5 at an
  overview, then 9, 13, 16), the paths and cycleways (14 and above) only zoomed in, three classes
  fewer while moving; a base line's width by its road's tier, version 1's: 0 without cars, else 1
  to 4 by its capacity over the 99th percentile of the car links' capacities, from 0, 0.25, 0.45,
  0.75 on (widths 0.5, 0.8, 1.2, 2.0, 3.0); the hover reads "Class: primary · 2,000 m" then
  "Lanes, speed: 1 lane · 50 km/h", the panel "Link <id> · primary · 2,000 m". By capacity: the
  drawing class ≥ 3 600 veh/h (or no limit) 0, ≥ 2 400 1, ≥ 1 600 2, any other car link 3, a link
  without cars 4; drawn up to 1 at an overview, then 2, 3, 4, one fewer while moving; the base
  line 3.0, 2.0, 1.2, 0.8, 0.5 by class; the hover and panel name the link "car link · 1 lane ·
  50 km/h · 2,000 m". Every class is drawn at every zoom on a network under 5 000 links; the
  links beyond the limit are drawn faint. A ribbon's width shows the road hierarchy by capacity
  alone, whatever the traffic: its capacity's place on a log scale between the small roads' (the
  5th percentile of the car links' capacities, or an eighth of the large roads' when lower) and
  the large roads' (the 99th percentile; a car link without a limit too), from a fifth of the way
  between 1 px and the top to the top, 1 px more than the widest ribbon the room leaves, at most
  18 px; the room is measured over the road classes the zoom draws in full, never fewer than the
  second zoom's (an overview of the largest roads alone would read their spacing as room). The
  summary's static maps keep their own rule (2 + 6 √(capacity over the 99th percentile) units).
- **Bikes and walkers** are summed per link over the window: views of the same links as the cars.
  A line's width is one rule for both: 1 + 7 √(min(n, 3 000) / 3 000) px for n cyclists or
  walkers over the window, full (8 px) at 3 000 and more (`rules::FLOW_FULL`: Delft's busiest
  cyclists' link carries 3 006 in the morning peak); the legend says so.
- **Transit**: a hop is two consecutive stops of a pattern ridden, merged across patterns of the
  same kind of service (one hop per kind and ordered pair of stops: a tram and a bus between the
  same two stops are two hops); its kind is its run's line's (the GTFS route type; other without
  the side table), drawn in that kind's colour; its travellers are those whose journeys ride it, every journey taken
  over the window (not only the pairs held). A stop's boardings are the rides
  boarded there. Stops are named by their GTFS names, else their ids. A ride's line is its run's
  (its route's short name, else long name, else id); a transit journey is named by its lines in
  order ("1 → 19"), "Transit" when they are unknown.
- **Places** are named by release 0.1's rule: a place's nearest stop within 400 m by its name
  ("near <name>" beyond 60 m), else where it lies from the network's centre, the mean of the links'
  ends ("1.2 km SE of the centre"; "the centre" within 300 m), by the eight directions of the
  compass. A pair is shown as "#49 · origin → destination · 41 travellers".
- **The pointer** takes, of the things drawn under it in the layers ticked (a road ribbon, a
  cyclists' or walkers' line, a transit hop or stop), the one drawn on top: they are drawn ribbons,
  cyclists, walkers, hops, stops in that order, so a thin line over a wide ribbon wins where the
  pointer is on both; when it is on none, the nearest within 4 px of its edge; only when nothing
  drawn is in reach, the nearest link drawn as a base line within 7 px, else the nearest drawn
  faint. A click on a link, a line or a hop selects it (its panel). While a pair is open, a click on
  the empty map or on a stop closes nothing; with no pair open it closes the link panel. The open
  pair's row is marked in whatever link panel lists it; a row shows its pair's journeys where the
  view is, the link staying selected; the marked row closes the pair. ×, Clear and Esc close.
  A pair from the list, ‹ or › is shown fitted to the view. An address naming a pair, a link, a hop
  or a bin the page does not hold is ignored.
- **The footer**: the provenance line from the run's record (version, window, bin, choice model,
  iterations and what ended them, seed, the network's fingerprint's first eight digits), the
  OpenStreetMap credit when the record says the network was read from OpenStreetMap, the pairs and
  trips held, the links and the file's size in MB (10⁶ bytes, the whole file, measured), and,
  when the run's files left trips out of the journeys, "N trips left out: their routes do not
  agree with the trips table" (`left_out` in `meta`). A part of the provenance whose key the
  record lacks is left out, except what ended the run: without `stopped_by_rule` the line says
  "its end not recorded". `record::dropped` gives the trips given and, of them, those dropped
  by reason (the summary's first tile).
- **A trip the run dropped** (outside its window: no row in `trips.parquet`) is not kept: in no
  pair, count or layer.
- **A trip left out** (its routes disagree with `trips.parquet`, `crates/io-parquet`) is in no
  pair and no transit count. **A journey whose legs' seconds are unknown** (a walk without its
  seconds in `routes.parquet`) shows its door-to-door time and no strip of its legs.
- **Link analysis**: a link lists the held pairs whose journeys use it, by car, bike and walk
  together, each with its travellers on the link; a hop lists its riders' pairs.

## The data layout (the contract between the encoder and the script)
`meta`: `version`, `title`, `n_nodes`, `n_links`, `n_rows`, `n_bins`, `bin_seconds`,
`window_start`, `trips`, `left_out` (the trips left out of the journeys), `origin` (`[x, y]`, m), `n_pairs` (held), `pairs_total`, `stops` (external
ids of the stops held, in their order), `delay_bands`, `delay_band_labels`, `delay_floor_s` (2),
`vc_bands`,
`vc_band_labels`, `peak_seconds` (900), `road_classes` (the distinct road classes in sorted
order when every link has one, else `null`), `road_tiers` and `road_tier_bounds` (the base
line's widths and the tiers' bounds by road class), `flow_full` (3 000), `kinds` (the six kinds' names), `lines` (the lines
ridden by the held journeys, by name) and `line_kinds` (each one's kind), `place_names` (the
held pairs' places' distinct names, in the order first met; `qpo` and `qpd` index them), `provenance`, `source`, `megabytes`, `tokens` (the two themes, `src/theme.rs`: surface, inks,
the network's grey, the mark, glow, the journeys', the modes', the delay's ramp and its four
`bands`, the volume's ramp), `arrays` (per array `n` name, `t` type, `o` byte offset,
a multiple of 8, `c` count). The arrays, little-endian, `u8`, `u32`, `i32` or `f32`; `u32::MAX` is "none":

| Arrays | Per | What |
|---|---|---|
| `nx`, `ny` (i32) | node | position, dm from `origin` |
| `vs` (u32) | link, + 1 | first of the link's shape points in `dx`, `dy` |
| `dx`, `dy` (i32) | shape point, by link | dm from the point before it in the arrays (the first from `origin`); a link's vertices are its upstream node, its points, its downstream node |
| `nid` (u8) | — | the nodes' external ids, UTF-8, each followed by a 0 byte (an id holding a 0 byte would split: a known limit) |
| `lid` (u8) | — | the links' external ids, the same |
| `lu`, `ld` (u32) | link | upstream, downstream node |
| `lm` (u8) | link | modes: 1 car + 2 bike + 4 walk |
| `cls`, `lanes` (u8) | link | drawing class (0 to 18 by road class, else 0 to 4 by capacity); lanes (0 without cars) |
| `rc` (u32) | link | only with road classes: its class in `road_classes` |
| `len`, `ff`, `cap`, `spd` (f32) | link | length m; car free-flow s; capacity veh/h (0: none or no limit); car speed km/h (0 without cars) |
| `bs` (u32) | bin, + 1 | first row of the bin in `rl`, `rp`, `rs` |
| `rl`, `rp` (u32), `rs` (f32) | row with cars, by bin then link | link; cars that entered; their crossing times summed, s |
| `bv`, `wv` (u32) | link | bikes, walkers that entered it over the window |
| `tsx`, `tsy` (i32), `tsb` (u32) | stop held | position dm; boardings |
| `ta`, `tb`, `tl` (u32), `tk` (u8) | hop, least ridden first | from stop, to stop (held index); travellers; kind |
| `qox`, `qoy`, `qdx`, `qdy` (i32) | pair | origin and destination doors, dm |
| `qn`, `qr` (u32) | pair | travellers; of them without an alternative |
| `qpo`, `qpd` (u32) | pair | its origin's and its destination's name in `place_names` |
| `qs` (u32) | pair, + 1 | first journey of the pair |
| `om` (u8), `on`, `of` (u32), `ot` (f32) | journey | mode (0 car, 1 bike, 2 walk, 3 transit); travellers; finished; mean time s (NaN: none finished) |
| `ol` (u32) | journey, + 1 | first leg of the journey |
| `lline` (u32) | leg | a ride's line in `lines`, else `u32::MAX` |
| `lk` (u8), `lsec` (f32) | leg | kind (0 drive, 1 bike, 2 walk on links, 3 ride, 4 walk off links, 5 wait); mean seconds |
| `la`, `lb` (u32) | leg | a route: its range of `pl`; a ride: its range of `pst`; a walk between two stops: the two stops; else none |
| `pl`, `pst` (u32) | — | the routes' links; the rides' stops (held index) |

The page's script is `page/data.js` (the data and the view), `page/draw.js` (the drawing),
`page/panels.js` (pointer, panels, legend) and `page/start.js` (exports, controls, start), joined in
that order into one function; its style is `page/page.css`, its markup `page/page.html`.

## Errors
`MapError`, before anything is written: a link of the run that the network does not have
(`NotTheRunsNetwork`); a journey and no timetable (`NoTimetable`); a ride on a run, or calls, the
timetable does not have (`NotTheRunsTransit`); a position more than 2³¹ − 1 dm from the origin
(`TooFar`); the file that cannot be written (`Io`).

## Bikes
A bike's journey is a journey like any other, in its pair beside the car's, drawn dashed; its ride
is counted per link in `bv` and drawn as the cyclists' layer on the same links as the cars, its
time is its realised door-to-door time. A link's hover and its analysis count its cyclists, and a
clicked link lists the pairs whose bike journeys use it with those whose car and walk journeys do.
Play and the time slider step road traffic only: the cyclists' layer, counted over the window, is
drawn the same at every bin, and stays drawn when road traffic is hidden.

## Tests
`cargo test -p openmobisim-map`. Each expected value is a hand number, read back from the written
file by a decoder that knows only the layout above (`tests/common/mod.rs`). Each run's four tables
are written by `openmobisim_io_parquet::write` and read back, as the binding draws a page
(`tests/common/mod.rs`, `kept`):
- `tests/hand_run.rs`: io-parquet's hand run (seven trips, three links, derived in its header):
  node positions; each link's ends, modes, class, length, free-flow time, capacity, lanes and speed;
  cars per bin (`st1` 3 in bin 0 with 435 s, `st2` 3 in bin 2 with 306 s); bikes and walkers per
  link; the two pairs (a→b: 4 travellers, the walk journey of 2 of whom 1 finished at 1 125 s, the
  bike at 360 s, 1 without an alternative; a→c: the car journey `st1`, `st2` of 3 at 247 s), every
  leg; each link's pairs and travellers; two writes the same bytes; no address in the file.
- `tests/hand_run.rs` also: each held pair's places by the centre rule (1.2 km W, 0.3 km E and
  1.3 km E of the centre at 1 166.7 m); the links' and nodes' ids; from a hand record, the
  provenance line and the OpenStreetMap credit.
- `tests/transit.rs`: a hand timetable of two runs and a footpath: the hops' travellers (3, 4, 1),
  the boardings per stop, each journey's rides, waits and walk between stops in seconds; a walk of
  225 s to the first stop and two patterns sharing a hop (one hop, 2 travellers); a title and a
  stop id holding `"`, `<` and `</script>` read back as given (the file's three script elements
  intact, the title element escaped); a journey without its timetable refused, nothing written.
- `tests/transit.rs` also: a tram and a bus sharing a hop kept apart by kind, the stops' names, the
  lines held and each ride's line, places near a named stop and at one.
- `tests/rules.rs`: doors 300 m and 400 m off their nodes (225 s and 300 s walked) for a car (669 s
  door to door), a bike (885 s) and a walker (1 650 s), every leg; the classes at 3 600, 3 599,
  2 400, 2 399, 1 600 and 1 599 veh/h and without a limit; 2 003 pairs of 2 006 trips, of which the
  page holds 2 000 holding 2 003 trips, the lowest ids kept at equal travellers; three cars of 244,
  248 and 247 s whose journey's time is their mean, 246.33 s, not the middle value; the delay rule
  (1 / 144 at 145 s against 144 s, 0 below free flow and for a free-flow time of 0); the delay's
  band with its floor of 2 s per car (862 % at 0.39 s per car, 19 % at 1.9 s, 4 % at 6 s and a car
  faster than free flow in the first band, 20 % at exactly 2 s in 15–50 %, 60 % and 150 % in the next
  two, exactly 50 % and 100 % in the band above the edge; on a 144 s link 0.7 % at 1 s in the first,
  25 % at 36 s in 15–50 %; a free-flow time of 0 in the first); road classes on every link drawn
  in version 1's order (steps 15, primary 4, an unknown name 10) with their names, and `st1`'s bend
  at (750, 300) m held as 7 500 and 3 000 dm; one link unclassed, drawn by capacity; the cyclists'
  and walkers' width share at 0, 750, 3 000 and 6 000 travellers 0, 0.5, 1, 1 (1, 4.5, 8, 8 px).
- `tests/record.rs`: the provenance line from a whole hand record; each key missing leaves its part
  out (what ended the run says it is not recorded); an empty record gives empty lines; the trips
  given and dropped by reason from a hand record.
- `src/places.rs` (unit tests): the eight directions with y to the north, "the centre" within
  300 m, a stop's name within 60 m and "near" it within 400 m, a stop without a name by its id.
- Through the binding (`crates/binding/tests/test_map.py`): `Run.map` writes the file and returns
  its path, the same bytes twice, no address; a folder as the path an `OSError` with no partial
  file left; each of the run's four files and an input file (by another spelling of its path)
  refused, unchanged. A run with a hand GTFS feed (`test_names.py`): the stops' names, the tram's
  kind and line, the places and the journey's line reach the map and the summary.
- `tests/page_delay_band.mjs`, in Node (`node crates/map/tests/page_delay_band.mjs`; CI runs it as
  a step of its own): the page's own `band`, `delayBand` and `aggregate`, cut from `page/data.js`
  and run on one bin of hand traffic (3 cars per link), put the twelve links of `rules.rs`'s floor
  cases in the same bands by hand, so the page colours a car link as `rules::delay_band` does (the
  page's own edges, 15 %, 50 % and 100 %, and floor, 2 s, checked in `tests/hand_run.rs`).
- The page is otherwise checked as a file (its data, by hand); its behaviour in a browser (the
  clicks, the panels, Play, the exports) is not tested automatically.
