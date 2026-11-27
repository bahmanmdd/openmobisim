# The modules

The simulator's core is fourteen modules written in Rust. You do not need them to use openmobisim from Python. This
page is for readers who want to know where something is computed and on what it rests. How the modules work together
in a run is in [How a run works](../concepts/architecture.md); the graph below shows which module uses which (an
arrow points from a module to one it uses, without the arrows another path already implies).

```{image} ../_static/figures/crates_paper.svg
:alt: Which module uses which: an arrow points from a module to one it uses directly
:class: only-light
:width: 100%
```
```{image} ../_static/figures/crates_night.svg
:alt: Which module uses which: an arrow points from a module to one it uses directly
:class: only-dark
:width: 100%
```

## What each module holds

% src: crates/*/README.md (first paragraph of each); crates/*/Cargo.toml
The table goes from the bottom of the hierarchy to its top: a module uses only modules above its row.

| Module | What it holds | Explained in |
|---|---|---|
| `types` | Identifiers, the clock, metric units, the modes, sums in a fixed order and random streams addressed by identity | [Reproducibility](../concepts/reproducibility.md) |
| `graph` | The network, built once from node and link rows: links per mode, free-flow times, capacities | [Networks](../concepts/networks.md) |
| `transit` | The timetable of one service day and the journey search on it (RAPTOR) | [Transit](../concepts/transit.md) |
| `choice` | Which alternative a traveller takes: the least cost, and the logit family | [Choice](../concepts/choice.md) |
| `loading` | Cars on the point queue, bikes and walkers at free flow; the time a link takes at an instant | [The loading](../concepts/loading.md) |
| `search` | The fastest route per mode at the current link times | [Routes and route sets](../concepts/search.md) |
| `demand` | The trips, each end linked to the network per mode and to the stops within walking reach | [Demand](../concepts/demand.md) |
| `loop` | The iteration: search, choose, load, measure, assess; the stopping rule | [The iteration](../concepts/iteration.md) |
| `io-osm` | The reader of OpenStreetMap extracts | [Networks](../concepts/networks.md) |
| `io-gtfs` | The reader of GTFS feeds | [Transit](../concepts/transit.md) |
| `io-parquet` | The four tables and the run record, written and read back | [What a run gives](../concepts/outputs.md) |
| `map` | The interactive map page of a finished run | [What a run gives](../concepts/outputs.md) |
| `summary` | The summary page of a finished run | [What a run gives](../concepts/outputs.md) |
| `binding` | The Python module: it reads tables and keywords, calls the modules above and computes nothing itself | [The Python interface](api.md) |

**For bikes.** No module is a bike module: the bike is a mode in each of them, from the bike links of `graph` to the
bike layer of `map`.
