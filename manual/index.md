---
myst:
  html_meta:
    description: "openmobisim, an open-source, agent-based simulator for multimodal dynamic traffic assignment (car, bicycle, walking, public transport, park-and-ride and bike-and-ride on one supernetwork), developed by Bahman Madadi: an introduction and the complete documentation."
---

# openmobisim

**An open-source, agent-based simulator for multimodal dynamic traffic assignment: car, bicycle, pedestrian and
public-transport trips, with park-and-ride and bike-and-ride, on one supernetwork.**

openmobisim is built for research on multimodal urban mobility and the infrastructure that carries it, at the scale
of a city's morning peak: the effect of a cycle track, a change of transit frequency, a road closure or a price on
mode shares, on route flows, on where the network congests and on travel times. Every traveller is an individual
agent with a traveller class, a set of available modes and preferences. Mode, itinerary and route are chosen by
discrete choice models: a nested logit over the modes and itineraries offered, a path-size logit over enumerated route
choice sets, or a model of your own written in Python. Public-transport journeys are planned on GTFS timetables with
RAPTOR. Cars and buses are loaded onto the road network one vehicle at a time, event by event in continuous time,
under the link conditions of the link transmission model (Yperman, 2007), which Raadsen, Bliemer and Bell (2016) solve
the same way for flows: a point queue by default, or the full model with spatial queues and spillback. Choice and network loading are iterated by the method of successive averages
towards a stochastic dynamic user equilibrium, and every run reports how close it came. Networks are read from
OpenStreetMap, GMNS or your own tables; demand from trip tables or origin–destination matrices, with an optional persons table for
traveller attributes. Results are Parquet tables, indicators and figures. A run is deterministic: the same inputs give the same results and the same
fingerprint, and every parameter is named, carries a documented default and can be overridden.

It is developed by [Bahman Madadi](https://bahmanmadadi.com), assistant professor at Université Gustave Eiffel and
ENTPE, as an open tool for research and teaching, under the Apache-2.0 licence.

```{figure} _static/figures/home_amsterdam.webp
:alt: A map of Amsterdam's simulated morning traffic, with a ribbon along each street, wider where more vehicles pass and coloured by their delay
:width: 100%

Amsterdam's morning, simulated. Each ribbon's width is the traffic on the street and its colour the delay. Amsterdam is
one of the starter cases that come with the simulator.
```

## What it does

- **Every mode on one network.** Streets, cycle paths and footpaths come from OpenStreetMap, public transport from the
  cities' timetables, and parkings let travellers switch from a car or a bike to transit.
- **Bicycles are taken seriously.** Cycle tracks, painted lanes and mixed traffic are told apart, both in the choice of
  route and in the choice of mode.
- **Choices and congestion together.** Travellers choose a mode, a route and a chain of modes; cars and buses queue
  where the roads are full; the two are repeated until the pattern settles, and the run says how well it settled.
- **What-if studies.** Change the network, the timetable, the prices or the travellers, and compare the results: the
  tutorials show transit frequency, accessibility, emissions and disruptions.
- **Ready to try.** Starter cases for Amsterdam, Delft, Paris and Lyon, the Sioux Falls and Nguyen–Dupuis benchmark
  networks, and ten tutorials.
- **Python on top, Rust underneath.** You work in Python; the simulation runs in a fast compiled core.

**Status:** version 0.1, an early release for testing by students and colleagues. The interface may still change, and
every release says what changed.

## Get started

```bash
pip install "openmobisim[all]"
openmobisim notebooks
```

This installs the simulator with everything the tutorials use (JupyterLab, matplotlib, pandas, polars, pyarrow) and
opens the ten tutorials at the first one,
[Getting started](tutorials/00_getting_started.ipynb). It needs Python 3.11 or newer, on macOS, Linux or Windows.
The notebooks can also be downloaded from the [Tutorials](tutorials/index.md), each from its page or all ten in one
zip.

## The documentation

This site is openmobisim's complete documentation, in four parts:

- **[Tutorials](tutorials/index.md)**: ten notebooks, from a first run to small studies of a city. Start here if you
  are new.
- **[Guides](guides/index.md)**: how to do one task, such as building a case from your own data, calibrating it or
  running scenarios.
- **[Concepts and methods](concepts/index.md)**: how the model works, with its equations and the literature behind
  it. [How a run works](concepts/architecture.md) shows how the pieces fit together.
- **[Reference](reference/index.md)**: every function, parameter, input file and output, generated from the code.

## Cite, licence and contact

- **Cite** the version you use: [How to cite](reference/citing.md).
- **Licence:** Apache-2.0. The starter cases' data keep their own licences, listed with [the cases](reference/cases.md).
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
