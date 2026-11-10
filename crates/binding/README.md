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
  | `run` | `boarding_slack` (s), `max_rides`, `choice` (`"logit"` or `"deterministic"`), `coefficients` (a dict from every class id to its three: cost per s, per €, per unit of ln PS), `nest_scale`, `iteration_cap`, `iteration_floor`, `link_bin` (s), `seed` |
- **`run`**: `window=(start_s, end_s)`, two seconds of the clock, required, never defaulted;
  `out`, the folder the files go to (created if missing). The demand must have been built on
  the network and the transit given to `run`.

## Outputs
`Network`, `Transit` and `Demand` objects (opaque, immutable) and, from `run`, a `Run`:
`trips`, `links`, `iterations` (the paths of `trips.parquet`, `links.parquet`,
`iterations.parquet`, written by `crates/io-parquet`, whose README gives every column),
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
- A keyword's value that cannot be read (a string for a speed, a choice model that is not one,
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
halves a ride.

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
- `test_determinism.py`: a logit run of 400 trips twice and at `RAYON_NUM_THREADS` 1 and 8:
  `trips.parquet` and `links.parquet` byte-identical.
