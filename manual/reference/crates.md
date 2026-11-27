# The modules

The simulator's core is fourteen Rust crates in one workspace (`crates/` in the repository). You do not need them to
use openmobisim from Python. This page is for readers who want to know where something is computed and on what it
rests. How the modules work together in a run is in [How a run works](../concepts/architecture.md); the graph below,
drawn from the crates' own `Cargo.toml` files, shows which module uses which (an arrow points from a module to one it
uses, without the arrows another path already implies).

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

## What each module holds, and its contract

% src: crates/*/README.md (first paragraph of each); crates/*/Cargo.toml
Each module has a contract, its README in the repository: its inputs, its outputs, their units, its meanings and
assumptions, what it repairs or refuses, and what it means for bikes. A module is admitted with that contract and a
test against an answer from outside the simulator ([Validation](../concepts/validation.md)). The contracts are shown
here as they are in the repository; a path written in one, such as `crates/loop/README.md`, names another of these
pages. The table goes from the bottom of the hierarchy to its top.

| Module | What it holds | Contract |
|---|---|---|
| `types` | Identifiers, the clock, metric units, the modes, sums in a fixed order and random streams addressed by identity | [types](contracts/types.md) |
| `graph` | The network, built once from node and link rows: links per mode, free-flow times, capacities | [graph](contracts/graph.md) |
| `transit` | The timetable of one service day and the journey search on it (RAPTOR) | [transit](contracts/transit.md) |
| `choice` | Which alternative a traveller takes: the least cost, and the logit family | [choice](contracts/choice.md) |
| `loading` | Cars on the point queue, bikes and walkers at free flow; the time a link takes at an instant | [loading](contracts/loading.md) |
| `search` | The fastest route per mode at the current link times | [search](contracts/search.md) |
| `demand` | The trips, each end linked to the network per mode and to the stops within walking reach | [demand](contracts/demand.md) |
| `loop` | The iteration: search, choose, load, measure, assess; the stopping rule | [loop](contracts/loop.md) |
| `io-osm` | The reader of OpenStreetMap extracts | [io-osm](contracts/io-osm.md) |
| `io-gtfs` | The reader of GTFS feeds | [io-gtfs](contracts/io-gtfs.md) |
| `io-parquet` | The four tables and the run record, written and read back | [io-parquet](contracts/io-parquet.md) |
| `map` | The interactive map page of a finished run | [map](contracts/map.md) |
| `summary` | The summary page of a finished run | [summary](contracts/summary.md) |
| `binding` | The Python module: it reads tables and keywords, calls the modules above and computes nothing itself | [binding](contracts/binding.md) |

## Building and testing the core

% src: README.md; crates/binding/README.md (first paragraph); .github/workflows/ci.yml
Every module builds and is tested without Python: `cargo test --workspace` from a clone of the repository. The Python
module is built from the `binding` crate by maturin: in a virtual environment, `pip install maturin`, then
`maturin develop --release` in the repository.

```{toctree}
:hidden:

contracts/types
contracts/graph
contracts/transit
contracts/choice
contracts/loading
contracts/search
contracts/demand
contracts/loop
contracts/io-osm
contracts/io-gtfs
contracts/io-parquet
contracts/map
contracts/summary
contracts/binding
```
