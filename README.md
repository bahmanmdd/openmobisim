# openmobisim
A simulator of multimodal dynamic traffic assignment: individual travellers choose mode and route; cars are loaded on a point-queue network, bikes and walkers at free flow, transit on its timetable; choice and loading are iterated to a stable state. A Rust core with a Python interface.
Every module enters with a contract (its README: inputs, outputs, units, assumptions) and a test against an answer from outside the simulator (theory, a hand derivation, a published result or another tool).
Bikes are first-class in every module, test and output. Metric units throughout.

**The manual** is at [openmobisim.github.io](https://openmobisim.github.io): two worked examples (Delft and Amsterdam), the parameters table and the contract of every module. Its source is the folder `manual/`.

## Install
```bash
pip install openmobisim
```
Python 3.11 or newer, on Linux, macOS or Windows; no Python dependency. This page describes release 0.2. Until 0.2 is on PyPI, that command gives release 0.1, which has another interface; 0.2 is then built from this tree with a Rust toolchain: in a virtual environment, `pip install maturin`, then `maturin develop --release` in the repository.

## A first run
Five cases are ready to download, the morning of Delft, Amsterdam, Lyon and Paris and a small synthetic city ([doi:10.5281/zenodo.24585967](https://doi.org/10.5281/zenodo.24585967)): the cities' networks and timetables are real, the travellers synthetic. Each is a folder holding the network as two tables, the timetable as GTFS, one row per traveller and a settings file, `run.toml`.
```python
import urllib.request, zipfile
import openmobisim as oms

urllib.request.urlretrieve("https://zenodo.org/records/24585967/files/delft.zip", "delft.zip")
zipfile.ZipFile("delft.zip").extractall()          # the folder delft
run = oms.run_file("delft", out="delft-run")       # 37 649 travellers, 07:30 to 09:30
run.map("delft-map.html")                          # the interactive map, one HTML page
run.summary("delft-summary.html")                  # the run's numbers, one HTML page
```
The results are Parquet tables (`run.trips`, `run.links`, `run.iterations` and `run.routes` are their paths), read with `pyarrow`, pandas, polars or any other reader of Parquet.

## The interface
```python
network = oms.Network.from_tables(nodes, links)          # or oms.Network.from_osm("extract.osm.pbf", region=None)
transit = oms.Transit.from_gtfs("gtfs.zip", network)
demand  = oms.Demand.from_table(trips, classes, network, transit=transit)
run     = oms.run(network, demand, transit, window=(27000, 34200), out="folder")   # seconds after midnight
run     = oms.run_file("folder-with-run.toml", out="folder")                       # the same from a settings file
run     = oms.Run.open("folder")                                                   # a finished run, nothing run again
```
A table is in memory (a dict of lists, a pandas or polars frame, a pyarrow table) or the path of a Parquet file. `crates/binding/README.md` gives every argument and every input column, and `PARAMETERS.md` every parameter of a run with its unit, default and source. Two helpers are experimental: `Network.from_osm(..., car_roads="main")` (cars on the main roads alone) and `make_case(folder, osm, population)` (a case folder drawn from an extract). For bikes: `bike` in a link's `modes` lets bikes ride it, `bike` in a trip's `modes` opens the bike to that traveller, and `bike_speed` is the one speed of every bike.

## What a run writes
A run writes five files into one folder: `trips.parquet`, `links.parquet`, `iterations.parquet` and `routes.parquet`, its results, and `run_record.toml`, the record of what was run: part one the settings it ran with (the window, the input files, the classes, every parameter at the value used), part two what it found and did (the end of the window it used, how it ended, the counts and fingerprints of its inputs, every repair), so that the folder alone tells what was run (`crates/io-parquet/README.md`). A run can also be described in a settings file, `run.toml`, written by hand in a folder: the window, the input files (an OSM extract or nodes and links tables, a trips table, a GTFS feed; tables as Parquet files, paths relative to the folder), the classes and any parameter to change; `run_file(folder, out=folder)` runs it. The two files have fixed names and are never one file: a run never writes `run.toml`, and the record's part one, copied into a `run.toml`, runs the same run again (`crates/binding/README.md`). For bikes: `bike_speed` is in the record, and each link's and trip's bike in the files it names.
`Run.map(path)` writes the run's interactive map, one self-contained HTML file that opens from disk with no server or network: road traffic by time bin, cyclists, walkers and transit, the journeys between the busiest pairs of places, and the pairs that use a link (`crates/map/README.md`); cyclists are a layer of their own and a bike journey is listed beside the car's. `Run.summary(path)` writes its summary, one self-contained HTML page of its numbers: the headline tiles, the modes, when people left, the roads, transit, bikes and walking, the convergence, the computation time, the record and a map per mode (`crates/summary/README.md`); bikes are a mode of every table and a map of their own.

## The ten tests
Release 0.2 is done when these ten tests pass, each against an answer from outside the simulator. Where the code and its documents say "test N" or "test N of the specification", they mean a row of this table.

| # | Test | External answer |
|---|---|---|
| 1 | Bottleneck: one link, inflow above capacity for a period; and the same link with unlimited capacity | queue and delay by Vickrey's recursion within 1 s; free-flow times reproduced exactly |
| 2 | Merge: two links into one of lower capacity, the approaches' demands unequal | Daganzo's merge rule with demand-proportional priorities, which is the deterministic queue on the merged link: discharge at its capacity, delay by arrival order (Vickrey's recursion by hand), total delay λ(λ−μ)T²/(2μ), each approach's share of the discharge equal to its share of the demand |
| 3 | Two routes under steady demand | equal times on both used routes, split by capacity (Wardrop) |
| 4 | Braess network with capacities | the static equilibrium of Braess (1968) as the window grows |
| 5 | Nguyen–Dupuis | the capacity-constrained equilibrium (the route flows that minimise total free-flow time with no link above its capacity; Larsson and Patriksson 1995) solved as a linear program by an outside solver: total free-flow time, mean trip, origin–destination times within tolerance |
| 6 | Sioux Falls (TNTP) | the same capacity-constrained equilibrium from the TNTP files, solved by one outside solver and confirmed by a second: total free-flow time and the set of queued links tightly at a large population run to a fixed count, the mean trip at 1 %; a single link's queue delay is reported, never judged (Bar-Gera's BPR flows exceed capacity on 60 of 76 links and are not a queue model's answer) |
| 7 | Choice closed forms | logit, nested logit and path-size logit shares against hand-computed values, chi-square at sampling error; in a run, the path sizes of car routes at the last iteration at closed form |
| 8 | Transit | common lines at closed form (expected wait and each line's share under uniform independent phases); RAPTOR equal to a brute force on the network of Spiess and Florian (1989) as timetables and on random timetables; that network's mean time is reported beside the paper's 27.75 min, never judged |
| 9 | Bike and walk | trip times equal length over speed by hand; routes equal networkx |
| 10 | Delft, signals ignored | free-flow routes of 1 000 node pairs per mode (car, bike, walk) equal to networkx's shortest times on the same network; identical results at 1 and 8 threads and over two runs; every trip finishes; under 15 s; the stopped state within 1 % of a 60-iteration run |

## Known limits
- **The mean trip time is over finished trips only.** A trip not at its door when the window ends is unfinished and has no travel time, so it is not in the mean. Unfinished trips are always recorded: each trip's row in `trips.parquet` has `finished`, and each row of `iterations.parquet` has the `finished` and `unfinished` counts beside its mean. Read the mean together with the unfinished count; a window long enough for every trip to finish makes means comparable (`crates/loop/README.md`).
- **Of two routes of exactly equal time, one is offered.** The search's tie rule finds one of them, and a traveller chooses only among what the search found (`crates/loop/README.md`).
- **A car trip while a bottleneck's queue builds up is slightly above theory.** In the test of cars against bikes behind a bottleneck, the mean car trip in the build-up period was measured about 1.5 s above the theoretical value on a trip of about 692 s (0.2 %) with `step_exponent` 1, of which about 0.4 s comes from whole-second departures, the rest not explained; at the default 0.75 it is +0.9 ± 0.8 s over 20 seeds, which tells neither apart.
- **A run stopped at the default 50 iterations is near, not exactly at, the equilibrium.** In the test of two car routes, one behind a bottleneck, under the default model the 300 s departure group farthest from theory has its trip by the bottleneck route 2.3 ± 1.0 s from it over 20 seeds; with `step_exponent` 1, the plain method of successive averages, it is 9.5 ± 1.9 s, the queue delay rippling by about ± 7 s along departure time, the same in every seed, and halving when the iterations double (`crates/loop/tests/bottleneck_route_choice.rs`; the groups measured 2026-11-21 with a probe that prints them).
- **A single link's queue delay on Sioux Falls is reported, never judged** (test 6): the sum over links agrees with the outside solver, single links differ.
- **Bikes and walkers travel at free flow**: one bike speed and one walk speed, no bike congestion.

## Cite and licence
To cite openmobisim, use `CITATION.cff`, and name the version you used and the fingerprints of the inputs of every run you report (each run's record holds them). Licensed under Apache-2.0; see LICENSE and NOTICE.
