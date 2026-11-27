# Contracts

The simulator is a chain of modules (Rust crates). Each depends only on those above it in this table, and each has a
contract: its inputs, its outputs, their units, its meanings and assumptions, what it repairs or refuses, and what
it means for bikes. The pages below are those contracts, as the code repository holds them beside each module's
code. A path written in a contract, such as `crates/loop/README.md`, names another of these pages.

| Module | What it holds |
|---|---|
| [types](types.md) | the identifiers, the modes and the units shared by all modules |
| [graph](graph.md) | the network: nodes, links per mode, free-flow times and capacities |
| [loading](loading.md) | the loading: cars on the point queue, bikes and walkers at free flow |
| [search](search.md) | the route search on the network |
| [choice](choice.md) | the choice models: the least-cost choice and the logit family |
| [transit](transit.md) | the timetable and the journey search on it |
| [demand](demand.md) | the travellers: trips, classes and the modes open to each |
| [loop](loop.md) | the iteration: search, choose, load, measure, assess; the stopping rule |
| [io-osm](io-osm.md) | the reader of OpenStreetMap extracts |
| [io-gtfs](io-gtfs.md) | the reader of GTFS feeds |
| [io-parquet](io-parquet.md) | the files a run writes: the four tables, column by column, and the run record |
| [map](map.md) | the interactive map page |
| [summary](summary.md) | the summary page |
| [binding](binding.md) | the Python module: every function, argument and input column |

```{toctree}
:hidden:

types
graph
loading
search
choice
transit
demand
loop
io-osm
io-gtfs
io-parquet
map
summary
binding
```
