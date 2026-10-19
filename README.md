<p align="center"><img src="assets/openmobisim_lockup_paper.svg" alt="openmobisim" width="420"></p>

**An open-source, agent-based simulator for multimodal dynamic traffic assignment: car, bicycle, pedestrian and
public-transport trips, with park-and-ride and bike-and-ride, on one supernetwork. A Python interface on a Rust core.**

Every traveller is an individual agent. Mode, itinerary and route are chosen by discrete choice models (a nested logit,
a path-size logit over enumerated route choice sets, or your own model in Python); transit journeys are planned on GTFS
timetables with RAPTOR; cars and buses are loaded one vehicle at a time, event by event in continuous time, under the
link conditions of the link transmission model (Yperman, 2007; Raadsen, Bliemer and Bell, 2016); choice and loading are
iterated by the method of successive averages
towards a stochastic dynamic user equilibrium, with a verdict on convergence. Networks come from OpenStreetMap, GMNS or
your own tables; demand from trip tables or origin–destination matrices, with an optional persons table for traveller
attributes; results are Parquet tables, indicators and figures. Runs are deterministic and fingerprinted, and every parameter is named, documented and
overridable. Built for research on multimodal urban mobility and the infrastructure that carries it.

> **Status: 0.1, an alpha for testing by students and colleagues.** The tests are not complete and the interface may
> still change before 1.0 (each release says what changed). Feel free to use it, and please
> [report issues](https://github.com/bahmanmdd/openmobisim/issues): what goes wrong, and what is unclear.

## Start here

1. **Read the first tutorial**, [`notebooks/00_getting_started.ipynb`](notebooks/00_getting_started.ipynb), here on
   GitHub, before installing anything: it lists the ten tutorials and the order to read them in.
2. **Install and open the tutorials** (Python 3.11 or newer). Copy the block for your system, in order. One install
   is all there is: `openmobisim[all]` brings everything the tutorials use (JupyterLab, matplotlib, pandas,
   polars, pyarrow); the last command copies the ten notebooks into `./openmobisim-notebooks` and opens the first in
   JupyterLab.

   **conda** (any system):

   ```bash
   conda create -y -n openmobisim python=3.12
   conda activate openmobisim
   pip install "openmobisim[all]"
   openmobisim notebooks
   ```

   **macOS or Linux** (venv, in a terminal):

   ```bash
   python3 -m venv openmobisim
   source openmobisim/bin/activate
   pip install "openmobisim[all]"
   openmobisim notebooks
   ```

   **Windows** (venv, in the Command Prompt):

   ```bat
   py -m venv openmobisim
   openmobisim\Scripts\activate
   pip install "openmobisim[all]"
   openmobisim notebooks
   ```

   `openmobisim` alone says what else it can do; from Python, in any editor, `openmobisim.notebooks()` copies the
   tutorials and links the first.

## What it does

- **Every mode on one supernetwork.** Road, bike and walk networks (also referred to as layers) from OpenStreetMap
  (or GMNS, or your own tables); public transport from GTFS; park-and-ride and bike-and-ride through parkings with
  capacity. Transfers happen at transfer locations (also referred to as hubs): stops and parkings.
- **Bicycles are first-class.** Bike assignment on the bike network, with separated tracks, painted lanes and mixed
  traffic told apart, both in route choice and in what mode choice sees.
- **Dynamic network loading.** The link transmission model (Yperman 2007) applied to individual vehicles, event by
  event in continuous time, as Raadsen, Bliemer and Bell (2016) do for flows: every car and bus enters and leaves every
  link at its exact moment, and there is no time step. A vertical queue (also referred to as a point queue: a
  bottleneck in Vickrey's 1969 sense at the end of every link) by default, or the full model with queues that take
  road space and spill back; buses ride the roads they share with cars.
- **Choice.** Route, mode chain (also referred to as itinerary) and mode choice by a logit model (or your own model in
  Python), seeing each alternative's times by part, its money, the network along it and the traveller; user classes
  (also referred to as traveller classes) with their own modes and preferences.
- **Dynamic traffic assignment.** Choice and network loading are repeated (the method of successive averages) until the
  pattern settles,
  with a verdict on convergence.
- **Reproducible.** The same inputs give the same results and the same run fingerprint; every parameter has a name and
  an uncalibrated default you can override; units are metric throughout.

## Install

```bash
pip install openmobisim                  # the simulator alone (Python 3.11 or later; macOS, Linux, Windows)
pip install "openmobisim[viz]"           # with matplotlib: the figures, without Jupyter
pip install "openmobisim[all]"           # everything: JupyterLab, matplotlib, pandas, polars, pyarrow; the tutorial
                                         # notebooks (openmobisim notebooks) and the starter cases, ready to explore
```

## A first run

```python
import openmobisim as ms

net = ms.examples.toy_network()                     # sixteen links, every number checkable by hand
o, d = net.node_lonlat("W"), net.node_lonlat("D1")
trips = [ms.Trip(f"t{i}", 0, *o, *d, 7 * 3600 + 4 * i, "driver") for i in range(450)]

run = ms.Scenario.from_parts(net, trips, classes={"driver": (True, False, False)}).run("first-run")
print(run.completion)
print(f"mean trip {run.mean_travel_time_s / 60:.1f} min, verdict: {run.convergence_verdict}")
```

## Starter cases

Six cases to learn on, each a checked network with what runs on it: Delft, Amsterdam, Paris and Lyon (OpenStreetMap,
their transit feeds, and a synthetic morning built from open data and the cities' travel surveys, congested like a real
one), Sioux Falls and Nguyen–Dupuis (the benchmarks). They download once, checked against the checksums the package
carries.

```python
case = ms.examples.case("amsterdam")
run = case.scenario("base").run("amsterdam-base")
```

```bash
openmobisim cases            # what is there
openmobisim run amsterdam    # run one, and write its summary page and interactive map (each linked when written)
openmobisim check            # run the default scenarios and compare with this version's reference results
```

The starter cases' demand is **synthetic**: for learning the tool and comparing methods, not for studying the cities.
For a study, bring your own network, timetable and demand in the standard formats.

## Learn more

- **Documentation**: [openmobisim.github.io](https://openmobisim.github.io): the tutorials, guides to each task, how the models work, and the
  reference of every function, parameter, input and output file.
- **Tutorials**: ten notebooks in [`notebooks/`](notebooks/), from a first run to validation and applications; start
  with [`00_getting_started`](notebooks/00_getting_started.ipynb).
- **For AI agents**: [`AGENTS.md`](AGENTS.md).
- **Contributing**: [`CONTRIBUTING.md`](CONTRIBUTING.md). Issues are welcome; outside pull requests wait for a
  contributor agreement.

## Cite

If you use openmobisim in work you publish, cite the version you used — GitHub's "Cite this repository" gives the
reference from [`CITATION.cff`](CITATION.cff) with its DOI ⟨Zenodo DOI at the release⟩ — and keep the run fingerprint
of every result you report. If you use the starter cases, cite their data too [![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23472146.svg)](https://doi.org/10.5281/zenodo.23472146): Madadi, B. (2026). *openmobisim starter cases: multimodal networks and synthetic demand for four European cities and two benchmark networks*
(v1). Zenodo. [https://doi.org/10.5281/zenodo.23472146](https://doi.org/10.5281/zenodo.23472146).

## Licence

Apache-2.0. See [`LICENSE`](LICENSE). The starter cases' data keep their own licences (OpenStreetMap: ODbL; each
transit feed its operator's), listed with the cases.
