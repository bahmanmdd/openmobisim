# How a run works

openmobisim is a simulation core written in Rust with a Python package on top. You work in Python: you build a
{py:class}`openmobisim.Scenario` from a network, the demand and settings, and call
{py:meth}`openmobisim.Scenario.run`, which returns a {py:class}`openmobisim.Run` holding the results and writes
the output files. Everything that is computed per vehicle, link or trip happens in Rust; Python reads inputs, checks
them, sets the defaults and shapes the results.

```{image} ../_static/figures/architecture_paper.svg
:alt: The architecture: the Python package you use, the compiled bridge, and the Rust crates beneath
:class: only-light
:width: 100%
```
```{image} ../_static/figures/architecture_night.svg
:alt: The architecture: the Python package you use, the compiled bridge, and the Rust crates beneath
:class: only-dark
:width: 100%
```

## The two layers

% src: python/openmobisim/__init__.py (module docstring, __all__); crates/py-bindings/src/lib.rs
| Layer | What it is | What it holds |
|---|---|---|
| The Python package `openmobisim` | The public interface | `Scenario`, `Run`, `Table`, the readers (`network_read_*`, `demand_read_*`, `transit_read_gtfs`, `parking_read_*`), `parameters()`, the figures (`openmobisim.viz`), choice models written in Python (`openmobisim.choice`), fixtures (`openmobisim.examples`) |
| The compiled module `openmobisim._core` | The bridge to Rust | Private: its shape may change between versions; everything you use is re-exported by `openmobisim` |
| The Rust crates | The simulator | Networks, demand, route sets, choice, the loading, transit, the run, the readers and writers |

The figures need `matplotlib` (`pip install "openmobisim[viz]"`) and are imported on demand, so `import openmobisim`
never needs it.

## Inside the simulator

The loading never computes a route: every vehicle arrives with its route already chosen. The choice models do not know
what they choose: routes, itineraries and modes are all offered to them as alternatives with named attributes. The run
is the only part that connects the two. Each part is a Rust crate that can be built and tested without Python; which
crate does what, and how to extend them, is in [The Rust crates](../reference/crates.md).

## Data flow: from files to results

% src: python/openmobisim/__init__.py (the readers); _core.pyi network_read_osm, Network.layer, transit_read_gtfs,
% parking_read_osm; gmns.py network_read_gmns; network.py network_read_table; demand.py demand_*; parking.py
% parking_read_table; scenario.py Scenario.__init__, Scenario.run, Run; crates/io-parquet/src/lib.rs
The inputs are read into four objects, a `Scenario` combines them with the settings, and a run turns them into results:
a `Run` in Python and the output files on disk.

```{image} ../_static/figures/dataflow_paper.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, a run turns them into a Run object and output files, and the figures are drawn from the Run
:class: only-light
:width: 100%
```
```{image} ../_static/figures/dataflow_night.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, a run turns them into a Run object and output files, and the figures are drawn from the Run
:class: only-dark
:width: 100%
```

| Object | Made by | Holds | Used by the run for |
|---|---|---|---|
| `Network` | `network_read_osm`, `network_read_gmns`, `network_read_table`; edited by `network_edit` | The road network (links, nodes, signals, turns, each link's traffic parameters) and its walk and bike layers (`network.layer("bike")`), read from OpenStreetMap or derived from the roads | Car routes and the loading (road); walking and cycling legs (layers); placing trips, stops and parkings |
| `Transit` | `transit_read_gtfs`; edited by `transit_edit` | Stops, routes, runs and their calls on one service day | Transit journeys (RAPTOR); buses loaded on the roads |
| `Parkings` | `parking_read_osm`, `parking_read_table` | Car and bike parkings: position, capacity, fees | Park-and-ride and bike-and-ride; parking occupancy |
| Trips | `demand_read_trips`, `demand_from_od`, `demand_sample`, `demand_assign_classes` | One row per trip: traveller, departure, origin, destination, mode (optional), class, weight | Everything: each trip becomes a vehicle, a walk, a ride or an itinerary |

A network can be written back out with `network_write_gmns` and trips with `demand_write_trips`, so a case built here
can be used elsewhere.

## Interactions in a run

% src: crates/core-sim/src/run.rs (Run::execute); crates/core-sim/src/equilibration.rs (msa: probability 1/(i+1));
% crates/core-sim/src/route_update.rs; crates/core-sim/src/link_times.rs
Inside a run, its driver calls the other parts in a loop. The figure shows one run with equilibration (`msa`, the
default): what is done once at the start, the loop of loadings, and the end. Iteration 0 is everyone's first choice,
and at iteration $i \geq 1$ each traveller chooses again with probability $1/(i+1)$, decided by a random draw keyed to
the traveller and the iteration.

```{image} ../_static/figures/run_loop_paper.svg
:alt: A run: the stages done once at the start, the loop of choice, loading, route update and gap until convergence, and the end
:class: only-light
:width: 100%
```
```{image} ../_static/figures/run_loop_night.svg
:alt: A run: the stages done once at the start, the loop of choice, loading, route update and gap until convergence, and the end
:class: only-dark
:width: 100%
```

| Component | Reads | Produces |
|---|---|---|
| Route sets | The road network; link times of the last loading (route update) | Routes per origin–destination pair, with their length and overlap (path size) |
| Itineraries | Layers, timetable, parkings, the last loading's times | Each itinerary trip's candidate chains of legs, and their expected times and costs |
| Choice model | Alternatives with named attributes (times, lengths, costs, transfers, your own values) and the class's coefficients | One chosen alternative per choosing trip |
| Loading | Vehicles with routes and departures; the network's traffic parameters; the loading rules | Link travel times by time of day, per-link results by time bin, trip outcomes, realised bus times |
| Equilibration | The chosen routes of two iterations; link times | Who chooses again; the gap; whether to stop |
| Writers | The run's results | The output files |

## What a run does, stage by stage

% src: crates/core-sim/src/run.rs (Run::execute, the timings.lap calls in order); crates/core-sim/src/timings.rs
A run goes through the stages below, in this order, the reading and setup first. Their names are the ones `timings.csv` and
{py:meth}`openmobisim.Run.timings` record, with the wall-clock time each took.

| Stage | When | What happens |
|---|---|---|
| `static_routes` | once | Bike and walk trips get their routes on the bike and walk layers. Their costs do not depend on traffic, so this happens once per run |
| `itineraries_setup` | once, with transit, parkings or mode choice | The trips that travel by transit, park-and-ride or bike-and-ride, or that choose their mode, get their candidate itineraries (chains of legs by mode, joined at hubs) |
| `route_sets` | once | A set of car routes for every origin–destination pair the car trips need |
| `route_choice` | each loading | Each car trip chooses a route from its pair's set, with the choice model |
| `itinerary_choice` | each loading, with itineraries | Each itinerary trip chooses among its itineraries (and, with mode choice, among modes) on the expected times |
| `partial_loading` | before the first full loading | The free-flow loading's groups: travellers loaded group by group, each group choosing on the congestion of the groups before it |
| `loading` | each iteration | Every vehicle through the network over the simulation window |
| `route_update` | between loadings | New routes found on the last loading's times are added to the route sets |
| `network_gap` | after the last loading | A sample of trips tested against the whole network: whether the choice sets missed a route traffic made worthwhile (the stopping rule reads the in-set gaps, not this) |
| `disrupted_loading` | once, with disruptions travellers do not know in advance | The day loaded once more with the disruptions, every choice as the run left it |
| `network_read`, `demand_read`, `setup` | once, first | The network's reading, if it recorded how long it took (read before the run, and not in the total); the demand read and checked; the layers, timetable and parkings prepared, and the fingerprint |
| `results_assembled` | once | The results gathered into the `Run` |
| `results_written` | once | The output files written |
| `total` | once | The run's whole time, as its one-line message says it |

How the loadings repeat is the subject of [Equilibration](equilibration.md); the loading itself, of
[The loading](loading.md).

## One setting for detail: the flow level

% src: crates/core-loading/src/lib.rs (the fidelity ladder); python/openmobisim/scenario.py FLOW_LEVELS, Scenario.__init__
How detailed the traffic model is depends on one setting, `flow_level`, not on separate models. The levels are the same
code with parameters changed: from level 2 up, the link transmission model (Yperman 2007) applied to individual
vehicles, event by event in continuous time, as Raadsen, Bliemer and Bell (2016) do for flows; at level 2 the end of
every link is a bottleneck in Vickrey's (1969) sense ([The loading](loading.md#where-the-model-comes-from)). It
has no time step: the 300-second settings are the resolution of the per-link outputs (`link_bin_s`) and of the costs
the next iteration chooses on (`cost_bin_s`), not of the traffic ([The loading](loading.md)).

| `flow_level` | Name | What it models |
|---|---|---|
| 0 | free flow | Every vehicle at free-flow speed; no capacity, no queues |
| 2 | point queue (the default) | Capacities at every link and junction; queues form but take no road space |
| 3 | spatial queue | Queues take road space and can fill a link, without the delay of the backward wave |
| 4 | full model | Spillback with the backward wave, rerouting on the way (turn pockets at every level) |

Level 1 does not exist. A `Scenario` refuses any other value.

## Defaults: the core's and the package's

% src: crates/core-sim/src/run.rs (module docs: Level0, deterministic, none); python/openmobisim/scenario.py Scenario.__init__
The Rust core's own defaults are the simplest: free flow, a deterministic choice and no equilibration. The Python
`Scenario` sets its own on top, and those are the defaults this manual describes: the point queue, the nested logit
when the run offers more than one mode and the path-size logit otherwise, and `msa` equilibration with the route update
`best_response`. Every value is listed in
[Parameters](../reference/parameters.md).

## Parallel and repeatable

% src: crates/core-types/src/lib.rs (reduce, rng); crates/core-choice/src/lib.rs (draws)
The core runs its parallel work on all processor cores. Results do not depend on how many cores are used, or in what
order threads finish:
- sums over many items are taken in a fixed order;
- every random draw comes from a stream addressed by what it is for (the traveller, the trip, the iteration and the
  alternative), never by a thread.

The same inputs on the same platform give the same results, bit for bit. See [Reproducibility](reproducibility.md).

## For bikes

% src: crates/core-graph/src/layers.rs; crates/core-sim/src/layers.rs
Bikes travel on their own layer, a network over the same geometry as the roads with its own costs, which take the kind
of cycling facility into account. Bike routes do not depend on traffic, so they are found once per run
(`static_routes`), not in every loading. A bike trip still takes part in mode choice, and in bike-and-ride, where the
bike leg ends at a parking.

## Limits

% src: crates/core-loading/src/lib.rs; crates/io-parquet/src/lib.rs
- Level 1 of the flow levels (a volume–delay model) is not built.
- `kpis.parquet`'s `design_id` and `replication` are labels a run is given (`Scenario.run(design_id=…,
  replication=…)`), `"default"` and 0 for a single run; a replication differs from another only through its
  `master_seed`. A design space is run as a loop of runs: there is no batch runner yet.
