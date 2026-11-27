# What a run gives

% src: crates/io-parquet/README.md; crates/map/README.md; crates/summary/README.md; crates/binding/README.md ("Outputs"); crates/loop/README.md ("Outputs")

A run writes **five files** into one folder: four tables, as Parquet files, and a record of what was run. From a
finished run two **pages** can be written, a map and a summary, each one HTML file that opens from disk. The folder
alone tells what was run. The tables are Parquet files; reading them in Python needs a Parquet reader, for instance pandas with pyarrow
(`pip install pandas pyarrow`); the guide [Reading the results](../guides/results.md) explains it in full.

This page says in plain words what each of them holds, which stage of [the iteration](iteration.md) it comes from, and
how they relate. Every column is listed in [Output files](../reference/outputs.md), and every key of the record in
[The settings file and the record](../reference/record.md).

```python
import openmobisim as oms
import pyarrow.parquet as pq

run = oms.run_file("delft", out="delft-run")
for name in ("trips", "links", "iterations", "routes"):
    table = pq.read_table(getattr(run, name))
    print(name, table.num_rows, "rows,", table.num_columns, "columns")
print("ended by the stopping rule:", run.stopped_by_rule)
run.map("delft-run/map.html")
run.summary("delft-run/summary.html")
```

```text
trips 37649 rows, 9 columns
links 171941 rows, 8 columns
iterations 11 rows, 17 columns
routes 118304 rows, 12 columns
ended by the stopping rule: True
```

(`"delft"` is the folder of the Delft case, one of the [published cases](../reference/cases.md), unpacked from
`https://zenodo.org/records/24585967/files/delft.zip`. The run takes about 4 s on a laptop ([Run time](../guides/speed.md)). The other pages of this part read the
folder `delft-run` that this example makes.)

## The files and where they come from

% src: crates/io-parquet/README.md ("Outputs", "Inputs": the order of writing); crates/loop/README.md ("Outputs"); crates/binding/README.md (`Run`)

```{image} ../_static/figures/outputs_from_stages_paper.svg
:alt: Where each output comes from: the iterations table from every iteration's assessment, the trips, links and routes tables from the last iteration, the record from the inputs and settings, the two pages from the files
:class: only-light
:width: 100%
```
```{image} ../_static/figures/outputs_from_stages_night.svg
:alt: Where each output comes from: the iterations table from every iteration's assessment, the trips, links and routes tables from the last iteration, the record from the inputs and settings, the two pages from the files
:class: only-dark
:width: 100%
```

A run is a loop of five stages, search, choose, load, measure and assess, repeated until the stopping rule or the cap
on iterations ends it. **The results are the state of the last iteration**; only the iterations table looks back over
all of them.

| File | One row per | What it tells | Comes from |
|---|---|---|---|
| `trips.parquet` | trip | What became of each trip: the mode it took, when it arrived, how long it took, what it cost, how many alternatives it had | The last iteration: its choice (the mode), its loading (the arrival) and its measure (the cost) |
| `routes.parquet` | trip and leg | The way each trip went, leg by leg: the walks, the links driven, ridden or walked, the transit rides | The alternative each trip held at the last iteration, found by the search and taken in the choice |
| `links.parquet` | link and time bin | The traffic: how many cars, bikes and walkers entered each link in each time bin, and how long the cars took | The last iteration's loading |
| `iterations.parquet` | iteration | How the run went: the mean trip time, the unfinished trips, how many travellers were offered a new choice, switched and would still change, and the seconds each stage took | The assess stage of every iteration |
| `run_record.toml` | (one file) | What was run and what it found: every setting at the value used, the input files, the fingerprints and counts of the inputs, how the run ended, every repair | The inputs and settings as given, and the run's end |

In Python, `run`, `run_file` and `Run.open` return a `Run` whose attributes `trips`, `links`, `iterations`, `routes`
and `record` are the paths of these five files. A `Run` keeps nothing of the run in memory: every number is in the
files.

The tables relate through their identifiers: `trip_id` joins the trips table to the routes table and to your own
trips table; `link_id` joins the links table, and the lists of links in the routes table, to your links table. The
identifiers are the ones you gave, as text.

## The trips table

% src: crates/io-parquet/README.md (`trips.parquet`; "Null, never `NaN`"; "Units"); crates/loop/README.md ("Outputs": `trips`; "Repairs"); crates/summary/README.md ("Completed", "still travelling", "without an alternative")

One row per trip that was run. Every trip ends in exactly one of three states:

| State | In the table | Meaning |
|---|---|---|
| Finished | `finished` is true; `arrival_s` and `travel_time_s` hold values | The trip was at its door before the window's end |
| Unfinished | `finished` is false, `mode` holds a value, `arrival_s` and `travel_time_s` are empty | The trip took an alternative and was not at its door when the window ended; its arrival is unknown |
| Without an alternative | `mode` is empty | No mode open to the trip can make it (no route on the network, no journey); it is counted, not simulated |

The travel time is door to door: the walk from the door to the network or the stop, the time on the network or on
transit, and the walk to the door. The **cost** is the choice model's cost of the alternative taken, at the times of
the last iteration, in the model's unit; `alternatives` is the size of the trip's set.

A trip that the run dropped has no row. The run drops a trip that departs outside the window, and a trip whose walk to
the network would end after the window for every mode open to it. Each drop is counted as a repair in the record and
shown as a warning ([The simulation window](../guides/window.md)).

## The routes table

% src: crates/io-parquet/README.md (`routes.parquet`: the kinds, "A car, bike or walk trip is …")

What each trip took at the last iteration, one row per leg, in order. A trip without an alternative has no row, and
the routes nobody took are not written.

| Trip by | Its legs, in order |
|---|---|
| Car, bike or on foot | A walk from the door to the network (`walk_gap`); the route on the network (`car`, `bike` or `walk`), as the list of its link identifiers in the order travelled; a walk to the door (`walk_gap`) |
| Transit | A walk from the door to the first stop (`walk_gap`); each ride (`ride`: the run, the stop and second boarded, the stop and second left), with a walk between two stops (`transfer_walk`) where the next ride leaves from another stop; a walk from the last stop to the door (`walk_gap`) |

A walk leg carries its seconds. A route leg carries its links and no seconds: the run keeps the trip's realised
door-to-door time, in the trips table, not the time of each leg. A ride carries the two clock seconds of the
timetable. An example of a transit trip's legs, with its time and cost derived from them, is on the
[transit page](transit.md#the-cost-of-a-journey-and-the-wait-at-the-first-stop).

## The links table

% src: crates/io-parquet/README.md (`links.parquet`; "A crossing is filed under the bin …"); crates/loading/README.md ("Outputs")

The traffic of the last loading, one row per link and time bin in which someone entered the link (a link and bin that
nobody entered has no row). The bins are `link_bin` seconds long, 300 s by default. A vehicle is counted in the bin in
which it **entered** the link, once on every link of its route.

For cars the table gives the count, the sum of their crossing times (so the mean crossing time is the sum over the
count), and the exits of the first and the last car that entered in the bin. For bikes and walkers it gives the counts.
The delay of a link is not a column: it is the mean crossing time less the link's free-flow time, its length over its
car speed, from your links table. An example for one link is on the
[loading page](loading.md#what-the-loading-records).

## The iterations table

% src: crates/io-parquet/README.md (`iterations.parquet`); crates/loop/README.md ("Outputs": `iterations`)

One row per iteration, in order: the run's history. It is the table to read first, to see whether a result can be
trusted: whether every trip finished, whether the mean trip time settled, and how many travellers would still change.
Its columns and an example are explained on [The iteration](iteration.md#what-each-iteration-reports). Its last five
columns are the seconds each stage took on the machine that ran it, the only numbers of a run that differ from one run
to the next ([Reproducibility](reproducibility.md)).

## The run record

% src: crates/io-parquet/README.md (`run_record.toml`); crates/binding/README.md ("Two files with fixed names", `run_file`, `Run.open`); README.md

`run_record.toml` is a text file in TOML, in two parts.

- **Part one, the settings the run ran with**: the window, every parameter at the value used with the defaults filled
  in, each followed by its unit, the coefficients of each class, the input files and the classes. Part one is itself a
  valid settings file: copied into a file named `run.toml`, it runs the same run again
  ([Reproducibility](reproducibility.md)).
- **Part two, `[result]`, what the run found and did**: the version of openmobisim, the end of the window the run
  used, the number of iterations, whether the stopping rule or the cap ended it, the counts and fingerprints of the
  network, the demand and the timetable, the readers' reports, and every repair by kind with its count.

```python
import tomllib

with open("delft-run/run_record.toml", "rb") as f:
    record = tomllib.load(f)
print(record["window_start_s"], record["window_end_s"], record["parameters"]["link_bin"])
result = record["result"]
print(result["iterations"], result["stopped_by_rule"], result["window_end_used_s"])
print(result["network"]["links"], result["demand"]["trips"], result["transit"]["runs"])
```

```text
27000 34200 300
11 True 34200
88512 37649 1707
```

The record holds no time stamp, no machine name and no thread count, so the same run written twice gives the same
bytes. An input given as a file is named by its path relative to the record's folder; an input given as a table in
memory has no path, and a record without the files of its network and its travellers cannot run again by itself.

The settings file `run.toml` and the record `run_record.toml` are two files with fixed names. A run never writes
`run.toml`, so settings and results may share a folder.

## The map and the summary

% src: crates/map/README.md ("Outputs", "What the page lets a reader do", "Meanings and assumptions"); crates/summary/README.md ("Outputs", "Meanings and assumptions"); crates/binding/README.md (`Run.map`, `Run.summary`)

Both pages are written from a finished run, by `Run.map(path)` and `Run.summary(path)`. Each is one self-contained HTML
file: it holds its data, loads nothing from the internet, and opens from disk in a browser. Neither computes anything
new: each reads the run's files and inputs again and shows them.

| Page | What it shows | It reads |
|---|---|---|
| The map (`Run.map`) | An interactive map of the network: road traffic by time bin, coloured by volume over capacity, by delay over free flow or by volume; the cyclists, the walkers and transit as map layers; the routes and journeys between the busiest pairs of places; the pairs of places that use a link | The trips, routes and links tables; the network, the demand and the timetable; the record |
| The summary (`Run.summary`) | The run's numbers on one page: trips completed and not, the modes' shares and mean trip times, departures by quarter hour, vehicle-kilometres by delay band, the busiest links, boardings, kilometres cycled and walked, the convergence by iteration, the computation time, the record, and a static map per mode | The same, and the iterations table |

Two definitions are shared by both pages. A link's **delay over free flow** is its cars' mean crossing time over the
car free-flow time, less 1, never below 0; it is shown in four bands (0–15 %, 15–50 %, 50–100 %, above 100 %), and a
link whose cars lose under 2 s each is put in the first band whatever its percentage. A **pair of places** is the trips with
the same origin and destination doors; the map holds the 2 000 busiest pairs of places and says in its footer how many of the trips
they hold.

How to read the two pages is in [The map and the summary](../guides/pages.md).

## Opening a finished run

% src: crates/binding/README.md (`Run.open`; "The rule for a finished run")

`Run.open(folder)` gives the `Run` of a finished run without running anything: it reads the record, rebuilds each
input from the file the record names, and reads the tables. The pages of an opened run are the same bytes as those of
the fresh run.

```python
import openmobisim as oms

run = oms.Run.open("delft-run")
print(run.trips)
print(run.stopped_by_rule, run.repairs)
```

```text
delft-run/trips.parquet
True {'network': {}, 'demand': {}, 'transit': {}, 'run': {}}
```

An input that was given in memory when the run was made has no file in the record and must be passed
(`Run.open(folder, network=..., demand=...)`). An input whose fingerprint differs from the record's is named in a
warning, and the run is opened with it.

## Conventions of the tables

% src: crates/io-parquet/README.md ("Units", "Meanings and assumptions": "Null, never `NaN`")

- **Times of day are seconds of the clock**, seconds after midnight of the run's day (`departure_s`, `arrival_s`,
  `bin_start_s`, the exits, a ride's `board_s` and `alight_s`), so a file is read without knowing the window. For a
  finished trip, `arrival_s` is `departure_s` plus `travel_time_s`.
- **Durations are seconds** (`travel_time_s`, `car_time_s`, a walk's `seconds`, the stages' seconds). A cost is in the
  choice model's unit. Shares are fractions between 0 and 1.
- **A value that does not exist is empty (null), never NaN and never 0**: the arrival of an unfinished trip, the mode
  of a trip without an alternative, the crossing time on a link and bin without cars, the measure of the other choice
  model.
- **Rows are sorted**: trips by identifier; links by identifier, then bin; routes by trip, then leg; iterations in
  order. Text is sorted by its bytes.
- **A column that holds a time carries its unit in its name** (`_s` for seconds).

## For bikes

% src: crates/io-parquet/README.md ("Bikes"); crates/map/README.md ("Bikes"); crates/summary/README.md ("Bikes")

Bikes are in every output beside the other modes.

| Output | What it holds for bikes |
|---|---|
| Trips table | A trip by bike has `mode` `bike` and its door-to-door time like any other |
| Routes table | A leg of kind `bike` with the links ridden, between its two walks |
| Links table | The column `bikes`: the bikes that entered the link in the bin. There is no bike time, since a bike crosses a link at its free-flow time |
| Record | `bike_speed` under `[parameters]`; a link's bike permission and a trip's bike are in the fingerprints |
| Map | The cyclists are a map layer of their own, summed over the window; a bike route is listed in its pair of places beside the car's |
| Summary | Bikes are a row of every table by mode, the kilometres cycled a number of their own, and the cyclists a map of their own |

## Limits

% src: crates/io-parquet/README.md (`routes.parquet`: `seconds` is null for a route; "Undetectable, said once"); crates/map/README.md ("A pair", "Tests": the page's behaviour in a browser); crates/loop/README.md ("Money is 0 …": transit's walking legs are not on links); crates/binding/README.md ("No command line, …")

- **Only the last iteration is written.** The trips, routes and links of earlier iterations are not kept; the
  iterations table holds their summary numbers.
- **Only the routes taken are written.** The other alternatives of a traveller's set, and their costs, are not in any
  file; `alternatives` gives their number.
- **No time per leg of a route.** A car, bike or walk leg has no seconds of its own; the trip's door-to-door time is in
  the trips table and each link's crossing times by bin in the links table.
- **No single vehicle's passage of a link is written**, only the counts and sums per bin.
- **Walks off the network are not on links.** The walks between a door and the network, and all walks of a transit
  trip, are not counted in the links table: its walkers are those of walk trips only.
- **No transit vehicle's load is written**: the rides of each trip are in the routes table, and the map and the summary
  count them per stop and per pair of stops.
- **The map holds the 2 000 busiest pairs of places**, not all of them.
- **A value changed by hand in a table is not detected** when the run is opened again.
- There are no figure functions, no log and no progress output.

All limits of the model are gathered in [Limitations](limitations.md).

## References

This page describes files and cites no published work. The formats are Apache Parquet for the tables and TOML 1.0
for the record. The works behind the numbers in the tables are cited on the pages that explain them, and gathered in
the [bibliography](../reference/bibliography.md).
