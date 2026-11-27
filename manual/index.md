---
myst:
  html_meta:
    description: "openmobisim, an open-source simulator of multimodal dynamic traffic assignment (car, bicycle, walking and scheduled public transport), developed by Bahman Madadi: an introduction and the complete documentation."
---

# openmobisim

**An open-source simulator of multimodal dynamic traffic assignment: car, bicycle, pedestrian and public-transport
trips of individual travellers, on one network. A Python interface on a Rust core.**

openmobisim is built for research on multimodal urban mobility at the scale of a city's morning peak. Every traveller
is an individual with a class, a set of modes open to it and preferences. Mode and route are chosen by a discrete
choice model: a nested logit over the modes, with a path-size logit over the car routes, or the least-cost choice.
Public-transport journeys are found on GTFS timetables with RAPTOR. Cars are loaded one by one on a point-queue
network, where the downstream end of every link is a bottleneck in Vickrey's sense; bikes and walkers travel at their
own speeds. Choice and loading are iterated until the pattern settles, and every run reports how settled it is.
Networks are read from OpenStreetMap or from your own tables, travellers from a trips table. Results are Parquet
tables, an interactive map and a summary page. A run is deterministic: the same inputs give the same results, and
every parameter is named, has a documented default with its source, and can be changed.

It is developed by [Bahman Madadi](https://bahmanmadadi.com), assistant professor at Université Gustave Eiffel and
ENTPE, as an open tool for research and teaching, under the Apache-2.0 licence.

```{figure} _static/figures/amsterdam_map.png
:alt: A map of Amsterdam's simulated morning traffic, with a ribbon along each direction of each road, wider where the road's capacity is higher and coloured by its volume over capacity
:width: 100%

Amsterdam's morning, simulated: the map page a run writes. Each ribbon is one direction of a road, its width the
road's capacity and its colour the cars' volume over that capacity in the busiest quarter hour. Amsterdam is one of
the cases ready to download (the network is from OpenStreetMap, © OpenStreetMap contributors).
```

## What it does

- **Every mode on one network.** Streets, cycle paths and footpaths from OpenStreetMap or from tables, each link
  open to cars, bikes or walkers; public transport from a GTFS timetable.
- **Bicycles are a mode like any other.** Every module, table and page says what it does for bikes: bike routes on
  the bike links, the bike in the mode choice, cyclists per link and time bin, a layer of their own on the map.
- **Choices and congestion together.** Travellers choose a mode and a route; cars queue where a road is full; the
  two are repeated until the pattern settles, and the run says how well it settled.
- **Checked against outside answers.** Each part is tested against theory, a hand derivation, a
  published result or another tool: ten tests, from Vickrey's bottleneck to Sioux Falls
  ([Validation](concepts/validation.md)).
- **Ready to try.** Five cases to download and run with one command: Delft, Amsterdam, Lyon, Paris and a small
  synthetic city ([The cases](guides/cases.md)), and two tutorials.
- **Reproducible.** The same inputs and settings give the same results at any number of threads; each run writes a
  record of its settings and the fingerprints of its inputs.
- **Python on top, Rust underneath.** You work in Python; the simulation runs in a compiled core, and the package
  has no Python dependency.

## Get started

```bash
pip install openmobisim
```

Python 3.11 or newer, on macOS, Linux or Windows. The tables a run writes are Parquet files; reading them in Python
needs a Parquet reader, for instance pandas with pyarrow (`pip install pandas pyarrow`), as
[Reading the results](guides/results.md) explains. Then download a case and run it:

```python
import urllib.request, zipfile
import openmobisim as oms

urllib.request.urlretrieve("https://zenodo.org/records/24585967/files/delft.zip", "delft.zip")
zipfile.ZipFile("delft.zip").extractall()          # the folder delft
run = oms.run_file("delft", out="delft-run")       # 37 649 travellers, 07:30 to 09:30
run.map("delft-map.html")                          # the interactive map, one HTML page
run.summary("delft-summary.html")                  # the run's numbers, one HTML page
```

The first tutorial, [Delft: a first run](tutorials/delft.ipynb), goes through these lines and reads the results.

## The documentation

This site is openmobisim's complete documentation, in four parts:

- **[Tutorials](tutorials/index.md)**: two notebooks, a first run and a case built step by step in Python. Start
  here if you are new.
- **[Guides](guides/index.md)**: how to do one task, such as building a case from your own data, writing the
  settings file or reading the results.
- **[Concepts and methods](concepts/index.md)**: how the model works, with its equations and the literature behind
  it. [How a run works](concepts/architecture.md) shows how the parts fit together.
- **[Reference](reference/index.md)**: every function, parameter, input column and output column, the published
  cases and the modules.

## Cite, licence and contact

- **Cite** the version you use: [How to cite](reference/citing.md).
- **Licence:** Apache-2.0. The cases' data keep their own licences, listed with [the cases](reference/cases.md).
- **Source code and issues:** [github.com/bahmanmdd/openmobisim](https://github.com/bahmanmdd/openmobisim).
- **Contact:** [Bahman Madadi](https://bahmanmadadi.com).

```{toctree}
:hidden:
:maxdepth: 2

tutorials/index
guides/index
concepts/index
reference/index
```
