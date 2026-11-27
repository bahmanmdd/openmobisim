# How a run works

openmobisim is a simulation core written in Rust with a Python module on top. You work in Python: you build a network,
a timetable and the demand from files or tables, and call `run`, which iterates choice and loading to a stable
state, writes the results as files and returns a `Run` that names them. Everything that is computed per traveller,
link or iteration happens in Rust; the Python module reads tables and keywords and passes them on.

```{image} ../_static/figures/architecture_paper.svg
:alt: The architecture: the Python module you use, the compiled bridge, and the Rust modules beneath, each using only modules below it or beside it in its row
:class: only-light
:width: 100%
```
```{image} ../_static/figures/architecture_night.svg
:alt: The architecture: the Python module you use, the compiled bridge, and the Rust modules beneath, each using only modules below it or beside it in its row
:class: only-dark
:width: 100%
```

## The two parts: Python and Rust

% src: crates/binding/README.md (the module's names; "Meanings and assumptions": nothing is computed here)
| Part | What it is | What it holds |
|---|---|---|
| The Python module `openmobisim` | The interface, eleven names and nothing else | `Network` (`from_tables`, `from_osm`, `tables`), `Transit.from_gtfs`, `Demand.from_table`, `run`, `run_file`, `make_case`, and `Run` with `Run.map`, `Run.summary` and `Run.open` |
| The module `binding` | The bridge to Rust, compiled into the Python module | It reads tables and keywords into the rows and values the modules below take, calls them and writes the run. It computes no number itself |
| The Rust modules | The simulator | The network, the demand, the timetable, the route search, the choice models, the loading, the iteration, the readers and the writers |

The package has no Python dependency. A run writes Parquet files; any reader of Parquet reads them (the tutorials use
`pyarrow`).

## Inside the simulator

% src: crates/*/README.md (each module's first paragraph and its "Depends on"); crates/*/Cargo.toml
The modules form a hierarchy: a module uses only modules below it in the figure or beside it in its row, and knows
nothing of those above. The modules of one row use one another in one direction only: `demand` uses `search`,
`loading` and `transit`; `search` uses `loading`; `summary` uses `map`. Three separations keep the parts independent:

- **The loading never chooses.** Every car arrives with its route already chosen; the loading only works out when it
  passes each link ([The loading](loading.md)).
- **The choice models do not know what they choose among.** A car route, a bike route, a walk and a transit journey
  are all offered as alternatives described by the same attributes; the model returns one of them ([Choice](choice.md)).
- **The iteration is the only part that connects them.** It asks the search for alternatives, the choice model for a
  choice, the loading for the times, and decides when to stop ([The iteration](iteration.md)).

| Module | What it holds | It uses | Explained in |
|---|---|---|---|
| `types` | Identifiers, the clock, metric units, the modes, sums in a fixed order, random streams | nothing | [Reproducibility](reproducibility.md) |
| `graph` | The network: nodes, links per mode, free-flow times, capacities | `types` | [Networks](networks.md) |
| `transit` | The timetable of one service day and the journey search on it | `types` | [Transit](transit.md) |
| `choice` | The least-cost choice and the logit family | `types` | [Choice](choice.md) |
| `loading` | Cars on the point queue, bikes and walkers at free flow; the time a link takes at an instant | `graph` | [The loading](loading.md) |
| `search` | The fastest route per mode at the current link times | `graph`, `loading` | [Routes and route sets](search.md) |
| `demand` | The trips, each end linked to the network per mode and to the stops in walking reach | `graph`, `loading`, `search`, `transit` | [Demand](demand.md) |
| `loop` | The iteration of search, choose, load, measure and assess; the stopping rule | all of the above | [The iteration](iteration.md) |
| `io-osm`, `io-gtfs` | The readers of an OpenStreetMap extract and of a GTFS feed | `graph`, `demand`, `transit` | [Networks](networks.md), [Transit](transit.md) |
| `io-parquet` | The four tables and the run record, written and read back | `loop` and the readers | [What a run gives](outputs.md) |
| `map`, `summary` | The two pages of a finished run | `map`: `io-parquet`; `summary`: `io-parquet` and `map` | [What a run gives](outputs.md) |

Each module has a contract, its README: inputs, outputs, units, assumptions and what it means for bikes. The contracts
and the graph of which module uses which are in [The modules](../reference/crates.md).

## Data flow: from files to results

% src: crates/binding/README.md ("Inputs", "Outputs", `run_file`, `Run.open`)
The inputs are read into three objects, a run turns them into results on disk, and the two pages are written from
the finished run.

```{image} ../_static/figures/dataflow_paper.svg
:alt: The data flow: input files are read into a network, a timetable and the travellers; a run turns them into four tables and a record; the map and the summary are written from the run
:class: only-light
:width: 100%
```
```{image} ../_static/figures/dataflow_night.svg
:alt: The data flow: input files are read into a network, a timetable and the travellers; a run turns them into four tables and a record; the map and the summary are written from the run
:class: only-dark
:width: 100%
```

| Object | Made by | Holds | The run uses it for |
|---|---|---|---|
| `Network` | `Network.from_tables` (a nodes table and a links table), `Network.from_osm` (an OpenStreetMap extract) | Nodes and links; each link carries one or more of car, bike and walk, its length and, for cars, its lanes, free-flow speed and capacity | Car, bike and walk routes; the loading of cars; linking trip ends and stops to the network |
| `Transit` | `Transit.from_gtfs`, read on a network | The stops, runs and footpaths of one service day | Transit journeys; the transit alternatives of each traveller |
| `Demand` | `Demand.from_table`, read on a network and, for transit, a `Transit` | One row per trip: its two ends, its departure second, its traveller class and the modes open to it | Everything: each trip is one traveller who chooses and travels |

`run(network, demand, transit, window=..., out=...)` runs them. `run_file(folder, out=...)` does the same from a
settings file, `run.toml`, which names the files, the window and the parameters. Either returns a `Run`; `Run.open`
gives the same object for a finished run on disk, without running anything
([The settings file and the record](../reference/record.md)).

## One trip, leg by leg

% src: crates/io-parquet/README.md (`routes.parquet`, the kinds of leg); crates/loop/README.md
A trip goes from door to door. The door is rarely on the network, so every trip starts and ends with a walk, and a
transit trip also waits and may change vehicles. Travel time is always door to door.

```{image} ../_static/figures/modes_paper.svg
:alt: The four modes as chains of legs from door to door
:class: only-light
:width: 100%
```
```{image} ../_static/figures/modes_night.svg
:alt: The four modes as chains of legs from door to door
:class: only-dark
:width: 100%
```

## Interactions in a run

% src: crates/loop/README.md ("The loop", "Search", "Choose", "Load", "Measure", "Assess", "Stop")
A run is a loop of five stages. Iteration 0 starts from free-flow link times and the timetable: everyone chooses as
if the roads were empty. Each later iteration starts from the times the last loading measured.

```{image} ../_static/figures/run_loop_paper.svg
:alt: A run: the inputs built once, then the loop of search, choose, load, measure and assess until the stopping rule or the cap, then the results written
:class: only-light
:width: 100%
```
```{image} ../_static/figures/run_loop_night.svg
:alt: A run: the inputs built once, then the loop of search, choose, load, measure and assess until the stopping rule or the cap, then the results written
:class: only-dark
:width: 100%
```

| Stage | Reads | Produces |
|---|---|---|
| Search | The network, the timetable, the link times of the last loading | For each traveller, the best alternative of each mode open to it, added to its set if it is new |
| Choose | Each traveller's set with the cost of every alternative; the traveller class's coefficients; the choice model | The alternative each traveller takes. Not everyone is offered a new choice: the share offered one shrinks with the iteration |
| Load | The car routes taken and their departures; the links' capacities | When each car passes each link (cars queue; bikes and walkers travel at free flow; transit runs as scheduled) |
| Measure | The loading | Each trip's door-to-door time; the cost of every alternative with a car route, taken or not, priced again at the new link times (the alternatives of the other modes keep their cost); the link times the next search reads |
| Assess | The iteration's choices and costs | How settled the run is (the shares of travellers who switched and who found something better, the unfinished trips, and a measure of what is left); whether to stop |

The run stops when the stopping rule holds or at the cap on iterations, and writes the last iteration
([The iteration](iteration.md), [What a run gives](outputs.md)).

## Parallel and repeatable

% src: crates/loop/README.md ("Determinism"); crates/types/README.md
The core uses all processor cores. Results do not depend on how many are used or on the order in which threads
finish: sums are taken in a fixed order, and every random draw comes from a stream addressed by what it is for (the
traveller, the alternative), never by a thread. The same inputs and settings give the same trips, links and routes
tables ([Reproducibility](reproducibility.md)).

## For bikes

% src: crates/graph/README.md; crates/loading/README.md; crates/binding/README.md ("Bikes")
A bike is a mode like any other in every part: a link says whether bikes may ride it, a trip says whether its
traveller may cycle, the search finds bike routes on the bike links, and the choice model weighs the bike beside the
car, the walk and transit. Bikes ride at one speed, `bike_speed`, and do not queue, so a bike route's time does not
change from one iteration to the next. Every output counts bikes: the trips table, the links table per link and time
bin, the map layer of cyclists and the summary.

## Limits

% src: README.md ("Known limits"); crates/loading/README.md; crates/graph/README.md
- Cars queue at the downstream end of a link and a queue takes no road space: there is no spillback.
- Traffic signals are read and stored, and applied nowhere.
- Bikes and walkers travel at free flow, and transit runs on its timetable whatever the road traffic.

All of them, with their reasons, are in [Limitations](limitations.md).
