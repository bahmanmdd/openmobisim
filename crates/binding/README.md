# openmobisim-binding

The Python module `openmobisim`: five names, nothing else. Built by maturin (`pyproject.toml`)
as one extension on Python's stable ABI from 3.11 (abi3). Depends on `openmobisim-types`,
`-graph`, `-transit`, `-demand`, `-loop`, `-io-osm`, `-io-gtfs`, `-io-parquet` and `pyo3`; no
Python runtime dependency, no `numpy`.

```python
import openmobisim as oms
network = oms.Network.from_osm(path, **network_parameters)
network = oms.Network.from_tables(nodes, links, **network_parameters)
transit = oms.Transit.from_gtfs(path, network, **transit_parameters)
demand  = oms.Demand.from_table(trips, classes, network, transit=None, **demand_parameters)
result  = oms.run(network, demand, transit=None, window=(start_s, end_s), out=folder, **parameters)
```

## Inputs
- **Files.** `from_osm`: one `.osm.pbf` extract (`crates/io-osm`). `from_gtfs`: one GTFS `.zip`
  read on a network built from longitude/latitude (`crates/io-gtfs`).
- **Tables.** A table is anything giving a column by name, `table[name]`: a dict of lists, a
  pandas or polars frame, a pyarrow table. Its column names are read from `column_names`
  (pyarrow), else `columns` (pandas, polars), else `keys()` (a dict); each needed column is read
  once into a plain list, by `to_pylist()` when the column has one, else by iterating it (the
  tests give dicts of lists and pyarrow tables; pandas and polars are not installed in CI). The
  columns are fixed, in any order, with no aliases; a table holds exactly these:
  - `nodes`: `node_id` (str), and either `lon_deg`, `lat_deg` (WGS84 degrees) or `x_m`, `y_m`
    (metres on a plane). The kind is told by the names: longitude/latitude when either
    `lon_deg` or `lat_deg` is a column, else metres. One kind per network (the graph's rule).
  - `links`: `link_id`, `upstream`, `downstream` (str: node ids), `modes` (str), `length_m`
    (float or `None`: the straight line), `lanes` (int or `None`), `car_speed_km_h` (float or
    `None`), `capacity_veh_h` (float or `None`: lanes × `capacity_per_lane`; `inf` is no
    limit). The car fields are given on a car link and `None` on any other (the graph's rule).
  - `trips`: `trip_id`, `class_id` (str), `departure_s` (int, a second of the clock), `modes`
    (str), and the two ends of the network's kind: `origin_lon_deg`, `origin_lat_deg`,
    `destination_lon_deg`, `destination_lat_deg`, or `origin_x_m`, `origin_y_m`,
    `destination_x_m`, `destination_y_m`.
  - A mode set is one string: mode names joined by `;`, no spaces (`car;bike`), each of `car`,
    `bike`, `walk`, `transit`. A null is Python's `None`, accepted only where a column above
    says so (a float `NaN` is a number, which the crates refuse).
  - `classes`: a list of class ids (str).
- **Keywords.** Each row of `PARAMETERS.md`, at exactly one entry point, the one whose crate
  reads it; omitted, the default of `PARAMETERS.md`, held in Rust (the loop's
  `Parameters::defaults`; for the five values the graph, the GTFS reader and the demand crate
  take from their caller, the constants of `src/lib.rs`), never in Python:

  | Entry point | Keywords (unit) |
  |---|---|
  | `Network.from_osm`, `Network.from_tables` | `capacity_per_lane` (veh/h per lane), `bike_speed`, `walk_speed` (km/h) |
  | `Network.from_osm` alone | `osm_class_defaults`: only `None`, the reader's one table (`crates/io-osm/README.md`) |
  | `Transit.from_gtfs` | `transfer_walk_max` (m) |
  | `Demand.from_table` | `stop_walk_max` (m) |
  | `run` | `boarding_slack` (s), `max_rides`, `nearby_stops`, `choice` (`"logit"` or `"deterministic"`), `coefficients` (a dict from every class id to its sixteen, in the order of the loop's choice row: cost per minute of car driving, bike riding, walking, transit in vehicle, access walk, egress walk, first wait, transfer wait, transfer walk; per transfer; per €; per unit of ln PS; the car, bike, walk and transit constants), `nest_scale`, `iteration_cap`, `iteration_floor`, `link_bin` (s), `seed` (an integer from 0 to 2⁶³ − 1, the largest the run's record holds) |
- **`run`**: `window=(start_s, end_s)`, two seconds of the clock, required, never defaulted;
  `out`, the folder the files go to (created if missing). The demand must have been built on
  the network and the transit given to `run`.

## Outputs
`Network`, `Transit` and `Demand` objects (opaque, immutable; each keeps the values it was
built with, the three build values, `transfer_walk_max`, `stop_walk_max`, for the run's
record) and, from `run`, a `Run`: `trips`, `links`, `iterations` (the paths of
`trips.parquet`, `links.parquet`, `iterations.parquet`, written by `crates/io-parquet`, whose
README gives every column), `record` (the path of `run.toml`, the record of what was run:
every parameter at the value used, defaults included, the window, the counts and fingerprints
of the network, the demand and the transit, the readers' counts; its keys in
`crates/io-parquet/README.md`),
`stopped_by_rule` (bool: the stopping rule, else the cap, ended the run), `osm_report` (the OSM
reader's `Report` as a dict, `None` for a network from tables) and `gtfs_report` (the GTFS
reader's, `None` without transit).

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
  than `None`), a missing or unknown column, a row that cannot be read, a mode name that is not
  one, a demand built on another network or transit: `ValueError` naming it.
- A crate's refusal: `ValueError` carrying the crate's message unchanged; a file that cannot
  be read or written: `OSError`. A panic is pyo3's `PanicException` (unwinding, never an abort).

## Bikes
A bike is a mode like any other at every entry point: `bike` in a link's `modes` lets bikes
ride it (a cycle path is a link `bike;walk` with no car field), `bike` in a trip's `modes` opens
the bike alternative, `bike_speed` sets the one speed of every bike. A bike's trip and its
counts per link and bin come back in the two files like a car's. Tested: bike trips equal
networkx's shortest lengths on a grid with streets one-way for bikes alone, and `bike_speed=30`
halves a ride. `run.toml` records the `bike_speed` the network was built with under
`[parameters]`.

## Tests
`pytest crates/binding/tests` on the extension built in release (`maturin develop --release -m
crates/binding/Cargo.toml` in a virtual environment with pytest, pyarrow 26.0.1, networkx 3.7).
Each expected value comes from outside the simulator:
- `test_hand_run.py`: io-parquet's hand run rebuilt from Python tables; its three files pass
  `crates/io-parquet/tests/read_back.py` (pyarrow, the hand numbers) unchanged.
- `test_networkx.py`: 30 bike and 30 walk trips on a jittered 12 × 12 grid, 15 % of its streets
  one-way for bikes: each `travel_time_s` is `ceil(g_o / w) + L / v + g_d / w` with `L`
  networkx's shortest length, within 1e-6 s (the derivation in the test).
- `test_parameters.py`: every row of `PARAMETERS.md`, parsed, accepted at exactly its entry
  point and refused at the others; the refusals by name; a crate's message unchanged;
  `bike_speed=30` gives 180 s for the 1 500 m that take 360 s at 15 km/h.
- `test_defaults.py`: the five defaults held in `src/lib.rs`, each by a hand case run with no
  keyword: a 1 500 m ride in 360 s (15 km/h); a 1 000 m walk in 750 s (4.8 km/h); three cars
  through a two-lane link 1 s apart (1 800 veh/h per lane); a stop at 1 499 m reached and one at
  1 501 m not (1 500 m); two lines joined by a 399 m walk and not by a 401 m one (400 m).
- `test_delft.py`: Delft's extract and feed read through Python, the feed's counts against
  gtfs-kit's (`crates/io-gtfs/tests/fixtures/delft/expected.csv`), the read times printed.
- `test_10_delft.py`: Delft with signals ignored, the whole run through Python; release only,
  skipped unless `TEST_10=1` (about a minute; not in CI); the file's header says what it judges.
- `test_determinism.py`: a logit run of 400 trips twice and at `RAYON_NUM_THREADS` 1 and 8:
  `trips.parquet` and `links.parquet` byte-identical.
- `test_record.py`: `run.toml` read with Python's `tomllib`. The names under `[parameters]` are
  the rows of `PARAMETERS.md` (parsed) less `osm_class_defaults`, and less `transfer_walk_max`
  and `stop_walk_max` without transit; each unit comment is the row's Unit cell; with no
  keyword every value is the default written in the test from `PARAMETERS.md`. A run with
  transit and non-default keywords at every entry point, rebuilt and rerun from the input
  tables and values read from its `run.toml` alone: `trips.parquet`, `links.parquet` and
  `run.toml` byte-identical, `iterations.parquet` equal but for its five wall-clock columns,
  the three fingerprints equal, `[transit.gtfs_report]` equal to `Run.gtfs_report`. One link,
  one trip or one call changed changes its own fingerprint alone. `run.toml` byte-identical
  twice and at `RAYON_NUM_THREADS` 1 and 8, on the run of `test_determinism.py` and on the case
  with transit. Class ids with a quote, a backslash, control characters and a letter outside
  ASCII read back unchanged. Delft from its OSM extract and feed: `source = "osm"`, and both
  readers' counts in the record equal to the `Run`'s.
- `test_record_values.py`: each keyword's value is the one used, read back from `run.toml`
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
