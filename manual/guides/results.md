# Results and figures

A run's results come three ways: as tables in Python, as files on disk, and as figures. What each result means is in
[Outputs](../concepts/outputs.md), and every column in [Output files](../reference/outputs.md).

The examples on this page are one run of Delft's default scenario, a morning from 07:30, with the outputs it gives:

```{include} _examples/results-run.md
```

## Tables

% src: python/openmobisim/scenario.py Run, Table
```python
kpis = run.kpis().to_pandas()                         # or .to_polars()
links = run.link_bins_table().to_pandas()             # road links by time bin
bikes = run.link_bins_table(layer="bike").to_pandas() # the bike layer
calls = run.transit_calls_table().to_pandas()
run.completion                                        # how the trips ended
run.trip_results()                                    # every trip and what became of it
run.trip_modes()                                      # the mode each trip took
```

A `Table` wraps a Parquet file and reads it on demand with pandas, polars or pyarrow (`.to_pandas()`, `.to_polars()`,
`.to_arrow()`). The same files are in the run's `output_dir` for R or any other tool.

% src: python/openmobisim/table.py Columns; run.py (the results that return one)
The results held in memory as columns (`run.trip_modes()`, `run.itinerary_choices()`, `run.transit_calls()`,
`run.parking_bins()`, `run.convergence()`, `run.timings()`, `run.logsum_od(...)`, `ms.parameters()` and the others
described "as columns") are `Columns`: a dict from each column's name to its values, so `modes["mode"]` reads one
column, with `.to_pandas()`, `.to_polars()` and `.to_arrow()` for the whole table and `.row_count` for its length. In a
notebook a `Columns` shows as a table.

```{include} _examples/results-modes.md
```

**Every trip, and those that did not finish.** `run.trip_results()` has one row per trip (also written as
`trip_results.parquet`): who, from where to where, when, by which mode, its outcome, its arrival and travel time if it
completed, and, for a car still on the roads when the window ended, the link it was on. Filter on `outcome` to find
the trips that did not finish and where they were:

```{include} _examples/results-trips.md
```

`kpis.parquet` holds every metric by iteration and mode, in long form: one row per value. The relative gap after each
loading shows the run settling towards its equilibrium:

```{include} _examples/results-kpis.md
```

## A run read back from its folder

% src: python/openmobisim/run.py Run.open, _Files
```python
run = ms.Run.open("runs/delft-base")        # a finished run's output_dir
run.completion, run.mean_travel_time_s, run.convergence_verdict
```

`Run.open` reads what the files hold: the run's identity and settings (`manifest.json`), the completion by mode, the
mean trip, the convergence with its verdict (`kpis.parquet`), every trip's result (`trip_results.parquet`), the timings
and every table, the same as the run gave
when it ended. What only the run in memory holds (the network, the route sets and choices, the itinerary choices)
is refused with a message saying so: run the scenario again for those.

## Labels for a study

`scenario.run(..., design_id="tracks", replication=2)` writes both into every row of `kpis.parquet`, so the results of
many runs stack into one table. They are labels: the fingerprint does not change with them.

## Many runs: the minimal outputs

% src: python/openmobisim/scenario.py Scenario.run (outputs=); crates/io-parquet/src/kpis.rs write_kpis_csv
A study of thousands of runs (random disruptions, a design space) needs a few numbers from each, not their link
tables. `outputs="minimal"` writes two small text files per run, `kpis.csv` (the rows of `kpis.parquet`) and
`manifest.json` (every setting), a few kilobytes in all; nothing else, and no page or figure:

```python
import random
from pathlib import Path

import pandas as pd

base = ms.examples.case("delft").scenario("base")
links = base.parts["network"].link_ids()
rng = random.Random(1)
for k in range(2000):
    closed = rng.sample(links, 3)
    base.replace(
        disruptions=[{"links": closed, "capacity_factor": 0.0, "from_s": 7.5 * 3600, "to_s": 9.5 * 3600}]
    ).run(f"disruption-{k:04d}", output_dir=f"runs/disruption-{k:04d}", outputs="minimal", quiet=True,
          design_id="closed:" + ";".join(closed))

kpis = pd.concat(
    pd.read_csv(f, float_precision="round_trip") for f in sorted(Path("runs").glob("disruption-*/kpis.csv"))
)
```

Name the runs so their folders sort (`disruption-0001`, …), and put what distinguishes them in `design_id`: every row
of `kpis.csv` carries it. The run's results in memory, its fingerprint and `ms.Run.open(folder)` are the same as with
the default `outputs="all"`; the files left out (`events`, `diagnostics`, the link bins, `transit_calls`,
`parking_bins`, `timings.csv`) are refused by name if asked for.

## Figures

% src: python/openmobisim/viz/__init__.py (__all__, the module docstring)
The figures need `pip install "openmobisim[viz]"`:

| Function | Draws |
|---|---|
| `viz.map_link(run, …)` | The run's traffic on its network, one ribbon per direction, coloured by delay or volume, for a time bin or a period; `layer="bike"` for the bike layer |
| `viz.map_demand(…)` | The demand as origin–destination desire lines |
| `viz.map_route(…)` | The alternative routes of one origin–destination pair |
| `viz.map_transit(run)` | Transit lines by kind, their width the passengers on board |
| `viz.map_parking(run)` | Car parks and bike parkings, sized by their use |
| `viz.chart_demand_matrix(…)` | The origin–destination matrix as a heat map |
| `viz.chart_mode_share(run)` | Each mode's share of the trips, by departure time |
| `viz.summary(run)` | One self-contained page: headline numbers, travel statistics by mode, convergence, the computation time with the networks' sizes, and a map per mode at the end (cars, transit, bikes, walking, each when the run has its results); `summary.html` in the run's folder unless a path is given; returns the path |
| `viz.map_interactive(run)` | One interactive HTML file: the traffic by time bin, the routes, transit, with exports; `map.html` in the run's folder unless a path is given |

```python
from openmobisim import viz

viz.map_link(run)                                # the busiest period; bins= picks another
viz.summary(run)                                 # <run folder>/summary.html; a path= puts it elsewhere
viz.map_interactive(run)                         # <run folder>/map.html
```

Every figure carries the run's fingerprint in its footer, so it can be traced to the run that made it. Sizes are in
centimetres. Each takes `theme="paper"` (the default) or `"night"`, a `size_cm`, and a `path` to save it to, as PNG,
PDF or SVG by the extension.

### The traffic

`map_link` draws the busiest part of the run unless `bins` names a time bin, a pair of them or two clock times. On the
road, the colour is the delay over free flow by default; `colour="volume"` or `"volume_capacity"` colour by the flow
instead.

```{include} _examples/results-map.md
```

On the bike layer the colour is the flow, as there is no capacity to compare it with:

```{include} _examples/results-map-bike.md
```

### Modes, transit and parkings

```{include} _examples/results-mode-share.md
```

```{include} _examples/results-transit.md
```

```{include} _examples/results-parking.md
```

### The demand and the routes

`map_demand` and `chart_demand_matrix` take the trips, before or without a run; `cell_m` sets the size of the cells
the trips are gathered into, and `top` how many pairs are drawn.

```{include} _examples/results-demand.md
```

```{include} _examples/results-demand-matrix.md
```

`map_route` draws one pair's routes from the run's route sets, side by side where they share a street; `pair` is the
pair's position among the sets, or two points.

```{include} _examples/results-route.md
```

Notebook [05 · Results and figures](../tutorials/05_results_and_figures.ipynb) draws each of them on a starter case.

## For bikes

`viz.map_link(run, layer="bike")` maps cycling flows, and `link_bins_table(layer="bike")` gives them as a table. Bike
parkings appear in `viz.map_parking`, and bike shares in `viz.chart_mode_share`.
