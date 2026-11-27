<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/bahmanmdd/openmobisim/main/assets/openmobisim_lockup_night.svg"><img src="https://raw.githubusercontent.com/bahmanmdd/openmobisim/main/assets/openmobisim_lockup_paper.svg" alt="openmobisim" width="420"></picture></p>

**An open-source simulator of multimodal dynamic traffic assignment: car, bicycle, pedestrian and public-transport
trips of individual travellers, on one network. A Python interface on a Rust core.**

Every traveller is an individual with a class, a set of modes and preferences. Mode and route are chosen by a discrete
choice model (a nested logit over the modes with a path-size logit over the car routes, or the least-cost choice);
transit journeys are found on GTFS timetables with RAPTOR; cars are loaded one by one on a point-queue network, where
the downstream end of every link is a bottleneck in Vickrey's sense, while bikes and walkers travel at their own
speeds; choice and loading are iterated until the pattern settles, and every run reports how settled it is. Networks
come from OpenStreetMap or your own tables, travellers from a trips table; results are Parquet tables, an interactive
map and a summary page. Runs are deterministic and recorded, and every parameter is named, sourced and can be changed.
Built for research on multimodal urban mobility and the infrastructure that carries it.

## Start here

1. **Read the first tutorial**, [Delft: a first run](https://openmobisim.github.io/tutorials/delft.html), before
   installing anything: it downloads a case, runs it and reads every result.
2. **Install** (Python 3.11 or newer; macOS, Linux, Windows). The package has no Python dependency. The tables a
   run writes are Parquet files; reading them in Python needs a Parquet reader, for instance pandas with pyarrow
   (`pip install pandas pyarrow`), as the guide
   [Reading the results](https://openmobisim.github.io/guides/results.html) explains.

   **macOS or Linux** (in a terminal):

   ```bash
   python3 -m venv openmobisim-env
   source openmobisim-env/bin/activate
   pip install openmobisim
   ```

   **Windows** (in the Command Prompt):

   ```bat
   py -m venv openmobisim-env
   openmobisim-env\Scripts\activate
   pip install openmobisim
   ```

## What it does

- **Every mode on one network.** Road, bike and walk links from OpenStreetMap or your own tables; public transport
  from GTFS. A trip goes from door to door: the walk to the network or to a stop, the wait and the transfers are part
  of it.
- **Bicycles are first-class.** Bike routes on the bike links, the bike in the mode choice, cyclists per link and
  time bin in the results, a layer of their own on the map: every module, test and output says what it means for
  bikes.
- **Dynamic network loading.** Every car enters and leaves every link at its own moment. A link is a point queue: a
  car leaves it after its free-flow time, and no sooner than one headway after the car ahead, the headway being
  3 600 s divided by the link's capacity in vehicles per hour.
- **Choice.** Mode and route by the logit family (logit, nested logit with nests by mode, path-size logit over car
  routes) or the least-cost choice, on each alternative's times by part and its money (money is 0 for every
  alternative); traveller classes with their own coefficients; each traveller has the modes open to
  it.
- **Dynamic traffic assignment.** Five stages per iteration (search, choose, load, measure, assess), repeated until
  a stopping rule holds; each iteration reports how many travellers switched and how many would still change.
- **Checked against outside answers.** Ten tests, each against theory, a hand derivation, a published result or
  another tool ([the ten tests](#the-ten-tests)).
- **Reproducible.** The same inputs and settings give the same results at any number of threads; each run writes a
  record of every setting and the fingerprints of its inputs; units are metric throughout.

## A first run

```python
import urllib.request, zipfile
import openmobisim as oms

urllib.request.urlretrieve("https://zenodo.org/records/24585967/files/delft.zip", "delft.zip")
zipfile.ZipFile("delft.zip").extractall()          # the folder delft: network, timetable, travellers, settings

run = oms.run_file("delft", out="delft-run")       # 37 649 travellers, 07:30 to 09:30, a few seconds
run.map("delft-map.html")                          # the interactive map, one self-contained HTML page
run.summary("delft-summary.html")                  # the run's numbers, one self-contained HTML page
print(run.trips, run.links, run.iterations, run.routes, run.record)   # the files the run wrote
```

With data of your own, the same run is built step by step:

```python
network = oms.Network.from_tables(nodes, links)          # or oms.Network.from_osm("extract.osm.pbf")
transit = oms.Transit.from_gtfs("gtfs.zip", network)
demand  = oms.Demand.from_table(trips, classes, network, transit=transit)
run     = oms.run(network, demand, transit, window=(27000, 34200), out="my-run")   # seconds after midnight
```

## The cases

Five cases to learn on, each a folder ready for `run_file`: the morning of Delft, Amsterdam, Lyon and Paris
(networks from OpenStreetMap, the cities' transit timetables and a synthetic morning) and DOTNET, a small synthetic
city. They are published as a data record:
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.24585967.svg)](https://doi.org/10.5281/zenodo.24585967)

| Case | Study area | Travellers | Archive (MB) |
|---|---|---|---|
| `delft` | the municipality of Delft | 37 649 | 4.5 |
| `amsterdam` | Amsterdam inside and around its A10 ring road | 198 252 | 24.5 |
| `lyon` | Lyon and Villeurbanne | 117 073 | 19.1 |
| `paris` | Paris inside the Boulevard Périphérique, the ring included | 243 203 | 29.4 |
| `dotnet` | a synthetic city of 8 km by 8 km | 39 919 | 0.4 |

The cases' demand is **synthetic**: for learning the tool and comparing methods, not for studying the cities. For a
study, bring your own network, timetable and trips.

## Learn more

- **Documentation**: [openmobisim.github.io](https://openmobisim.github.io): the tutorials, guides to each task, how
  the models work with their equations and sources, and the reference of every function, parameter, input column and
  output column.
- **Parameters**: [every parameter of a run](https://openmobisim.github.io/reference/parameters.html) with its unit,
  default and source.
- **Questions and problems**: [open an issue](https://github.com/bahmanmdd/openmobisim/issues).

## The ten tests
openmobisim is checked by these ten tests, each against an answer from outside the simulator.

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
| 10 | Delft, signals ignored | free-flow routes of 1 000 node pairs per mode (car, bike, walk) equal to networkx's shortest times on the same network; identical results at 1 and 8 threads and over two runs; every trip finishes; the stopped state within 1 % of a 60-iteration run |

## Known limits
- **The mean trip time is over finished trips only.** A trip not at its door when the window ends is unfinished and has no travel time, so it is not in the mean. Every unfinished trip is recorded (`finished` in `trips.parquet`, the counts `finished` and `unfinished` in `iterations.parquet`): read the mean together with the unfinished count, and choose a window long enough for every trip to finish when means are compared.
- **Of two routes of exactly equal time, one is offered.** The search finds one of them, and a traveller chooses only among what the search found.
- **A car trip while a bottleneck's queue builds up is slightly above theory**: by about 1 s on a trip of about 692 s (0.1 to 0.2 %).
- **A run stopped at the default 50 iterations is near, not exactly at, the equilibrium.** With two car routes, one behind a bottleneck, the trip time of the departure period farthest from theory is about 2 s from it.
- **A single link's queue delay on Sioux Falls is reported, never judged** (test 6): the sum over links agrees with the outside solver, single links differ.
- **Bikes and walkers travel at free flow**: one bike speed and one walk speed, no bike congestion.

## Cite

If you use openmobisim in work you publish, cite the version you used (GitHub's "Cite this repository" gives the
reference from [`CITATION.cff`](CITATION.cff)) and keep the fingerprints of the inputs of every run you report (each
run's record, `run_record.toml`, holds them). If you use the cases, cite their data too: Madadi, B. (2026).
*openmobisim example cases: multimodal networks and synthetic morning demand for four European cities and a synthetic
test city* (v2). Zenodo. [https://doi.org/10.5281/zenodo.24585967](https://doi.org/10.5281/zenodo.24585967).

## Licence

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE). The cases' data keep their own licences (OpenStreetMap:
ODbL; each transit feed its operator's), listed with the cases.
