# The Rust crates

The simulator's core is twelve Rust crates in one workspace (`crates/` in the repository). You do not need them to use
openmobisim from Python. This page is for readers who want to know where something is computed, or to extend the core.
How they fit together is in [How a run works](../concepts/architecture.md); the graph below, drawn from the crates' own
`Cargo.toml` at every build, shows which crate uses which (an arrow points from a crate to one it depends on, without
the arrows another path already implies).

```{image} ../_static/figures/crates_paper.svg
:alt: Which crate uses which: an arrow points from a crate to one it depends on
:class: only-light
:width: 100%
```
```{image} ../_static/figures/crates_night.svg
:alt: Which crate uses which: an arrow points from a crate to one it depends on
:class: only-dark
:width: 100%
```

% Until the crates are published on crates.io, their API is read from the source: `cargo doc --workspace --no-deps --open`.
Each crate's API, every public type and function, is documented in its doc comments. Build it locally with
`cargo doc --workspace --no-deps --open` from a clone of the repository.

## What each crate holds

% src: crates/*/src/lib.rs (module tables and `pub mod` lists)
| Crate | Modules | What they hold |
|---|---|---|
| `core-types` | `ids`, `time`, `units`, `reduce`, `rng`, `diagnostics`, `registry`, `hash` | Dense integer identities with a null value; the clock in whole seconds and the loading's time grid; SI units as types; parallel sums in a fixed order; random draws addressed by identity; run diagnostics; registries of named methods; the FNV-1a hash |
| `core-graph` | `network`, `defaults`, `network_options`, `layers`, `hubs`, `turns`, `csr`, `connectivity`, `geometry`, `link_geometry`, `examples` | The road network (built once, then never changed); the table that turns OpenStreetMap tags into traffic parameters; the walk and bike layers; hubs and their transfers; turns; connectivity checks; projection to metres; street shapes; the test networks |
| `core-demand` | `trips`, `persons`, `travellers`, `mode`, `vehicles` | Reading and checking trips and persons; travellers and their trips in sorted arrays; a trip's mode; who owns which vehicle and where it is |
| `core-routes` | `search`, `generate`, `store`, `attributes`, `snap`, `reach` | Turn-aware shortest-path search; the route-set methods; the route-set store; each route's length and path size; the nearest node to a point; bounded searches from a point |
| `core-choice` | `batch`, `model`, `builtin` | A batch of choice situations with their alternatives and attributes; the choice-model interface and its registry; the built-in models |
| `core-loading` | `vehicle`, `level0`, `ltm`, `curves`, `node_model`, `link_bins` | Vehicles; the free-flow loading; the link transmission model (levels 2–4); cumulative counts per link; the node model; per-link results by time bin |
| `core-transit` | `timetable`, `raptor`, `kind`, `date`, `defaults`, `examples` | A day's stops, routes, runs and calls; RAPTOR; the kind of service and whether it uses the roads; service days; the transit defaults; the test timetables |
| `core-sim` | `run`, `equilibration`, `route_choice`, `route_update`, `route_cache`, `layers`, `itinerary`, `itinerary_choice`, `transit`, `parking`, `prices`, `loading_rules`, `link_times`, `link_values`, `demand_values`, `disruptions`, `skims`, `events`, `identity`, `timings` | The run and everything it brings together |
| `io-osm` | `source`, `region`, `tags`, `import`, `pbf`, `parkings` | Reading OpenStreetMap: elements, cutting a study area, interpreting tags, building the network, parkings |
| `io-gtfs` | `source`, `read` | Reading a GTFS feed (a zip or a folder) into one day's timetable |
| `io-parquet` | `kpis`, `link_bins`, `events`, `diagnostics`, `transit_calls`, `parking_bins`, `manifest` | Writing the output files |
| `py-bindings` | — | The Python module `openmobisim._core` |

## Extending the core

% src: core-choice/src/model.rs ChoiceModel, Registry::builtin; core-routes/src/generate.rs RouteSetGenerator,
% Registry::builtin; core-sim/src/equilibration.rs Equilibration, Registry::builtin; core-sim/src/route_update.rs
% RouteUpdate, Registry::builtin; io-osm/src/source.rs OsmSource
The methods selected by name in a `Scenario` are entries in registries. A new method is a Rust type that implements the
interface, added to the registry; the run, the fingerprint, the manifest and the Python interface need no other
change. This means building openmobisim from source.

| To add | Implement | In | Built in today |
|---|---|---|---|
| A choice model | `ChoiceModel` | `core-choice` | `deterministic`, `logit`, `nested_logit` |
| A route-set method | `RouteSetGenerator` | `core-routes` | `shortest`, `penalty`, `montecarlo` |
| An equilibration strategy | `Equilibration` | `core-sim` | `free_flow`, `msa` |
| A route update | `RouteUpdate` | `core-sim` | `none`, `best_response` |
| A source of OpenStreetMap data | `OsmSource` | `io-osm` | the `.osm.pbf` reader, an in-memory source |

**A choice model can also be written in Python**, without building anything; see [Choice](../concepts/choice.md).

## Rules the code follows

% src: crates/core-types/src/lib.rs ("The three rules", the module table's `registry` row)
Three rules keep the core repeatable and lean; a contribution follows them:
- Parallel sums go through `core-types`' fixed-order reductions, never a plain parallel `sum()`.
- Stored arrays of identities use each identity type's null value, never `Option`, which would double their size.
- Units are converted once, where a scenario is built (capacities arrive in vehicles per hour), never inside a loop.

Methods selected by name are also looked up once, when a run is built, never inside a loop.
