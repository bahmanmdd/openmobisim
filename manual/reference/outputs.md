# Output files

A run writes five files into its output folder: four tables as Parquet files, described here, and the run's record,
`run_record.toml`, which has [its own page](record.md). The run's two pages, the map and the summary, are written
from these files on request ([`Run.map`](api.md#runmap), [`Run.summary`](api.md#runsummary)). How to work with the
tables is in [Reading the results](../guides/results.md); what the numbers mean is in
[Outputs](../concepts/outputs.md).

| File | One row per | Order of the rows |
|---|---|---|
| [`trips.parquet`](#tripsparquet) | trip run | trip id |
| [`links.parquet`](#linksparquet) | link and time bin that someone entered | link id, then bin |
| [`iterations.parquet`](#iterationsparquet) | iteration | iteration |
| [`routes.parquet`](#routesparquet) | trip and leg of what the trip took at the last iteration | trip id, then leg |

Ids are sorted bytewise, as text. All tables on this page share one layout: **Column**, **Parquet type** (as pyarrow
names it), **Unit**, **Null when** (never, or the condition) and **Meaning**.

## Conventions

% src: crates/io-parquet/README.md ("Outputs", "Units", "Meanings and assumptions"); crates/binding/README.md ("Tests": test_run_file.py, test_open.py, test_determinism.py, for the bytes of a second run); the file format read with pyarrow from a run of delft

| Convention | Rule |
|---|---|
| Ids | The external ids of the inputs (a trip's `trip_id`, a link's `link_id`, a class's id, a GTFS `stop_id` or `trip_id`), as text. |
| Times of day | `departure_s`, `arrival_s`, `bin_start_s`, `first_exit_s`, `last_exit_s`, `board_s`, `alight_s` are seconds of the clock: seconds after midnight of the run's day. A file is read without knowing the run's window. |
| Durations | `travel_time_s`, `car_time_s`, `seconds` and the five stage columns are in seconds. |
| No value | A missing value is a null, never `NaN`. A column that cannot be null is marked so in the file's schema. |
| Shares | The shares and the gap are fractions between 0 and 1, not percentages. |
| File format | Parquet format 1.0, no compression, dictionary encoding, one row group up to 1 048 576 rows. The metadata hold the writer and the schema and nothing else: no path, no name, no time stamp. |

The same run written twice gives the same bytes in `trips.parquet`, `links.parquet` and `routes.parquet`;
`iterations.parquet` differs in its five wall-clock columns ([Reproducibility](../concepts/reproducibility.md)).

The four files of a run of the case `delft` (a run of about 4 s on a laptop):

```python
import openmobisim as oms
import pyarrow.parquet as pq

run = oms.run_file("delft", out="delft-run")
for path in (run.trips, run.links, run.iterations, run.routes):
    table = pq.read_table(path)
    print(path.name, table.num_rows, "rows,", table.num_columns, "columns")
print(pq.read_table(run.trips).schema)
```

```text
trips.parquet 37649 rows, 9 columns
links.parquet 171941 rows, 8 columns
iterations.parquet 11 rows, 17 columns
routes.parquet 118304 rows, 12 columns
trip_id: string not null
class_id: string not null
departure_s: uint32 not null
mode: string
arrival_s: double
travel_time_s: double
finished: bool not null
cost: double
alternatives: uint32 not null
```

## `trips.parquet`

% src: crates/io-parquet/README.md ("Outputs", "Meanings and assumptions"); crates/io-parquet/src/trips.rs (the column table); schema read with pyarrow from a run of delft

One row per trip run, what the run's last iteration made of it. A trip the run dropped, because its window could not
hold it, has no row ([Repairs](warnings.md#repairs)).

| Column | Parquet type | Unit | Null when | Meaning |
|---|---|---|---|---|
| `trip_id` | `string` | none | never | The trip's id. |
| `class_id` | `string` | none | never | The id of its traveller class. |
| `departure_s` | `uint32` | s of the clock | never | Its departure. |
| `mode` | `string` | none | the trip has no alternative | The mode taken: `car`, `bike`, `walk` or `transit`. |
| `arrival_s` | `double` | s of the clock | the trip is unfinished or has no alternative | The arrival at the door. For a finished trip it is `departure_s + travel_time_s`. |
| `travel_time_s` | `double` | s | the trip is unfinished or has no alternative | The time from door to door, as realised. |
| `finished` | `bool` | none | never | Whether the trip arrived before the window's end. |
| `cost` | `double` | the choice model's unit | the trip has no alternative | The cost of the alternative taken, at the last measure. With time coefficients alone it is in minutes ([Choice](../concepts/choice.md)). |
| `alternatives` | `uint32` | alternatives | never | How many alternatives the trip's set holds. |

A trip *without an alternative* is one for which no mode open to it reaches its destination. An *unfinished* trip has
an alternative and is still travelling at the window's end; it has no travel time, so the mean trip time is over
finished trips only.

**For bikes.** A trip by bike has `mode` `bike` and its door-to-door time like any other mode.

## `links.parquet`

% src: crates/io-parquet/README.md ("Outputs", "Meanings and assumptions", "Bikes"); crates/io-parquet/src/links.rs (the column table); schema read with pyarrow from a run of delft

The traffic of the last iteration. One row per link and time bin in which someone entered the link; a link and bin
nobody entered has no row. A bin is `link_bin` seconds long (300 s by default) and is named by its first second. A
crossing is filed under the bin in which its vehicle entered the link, with its exact exit, even when the exit is
past the window's end.

| Column | Parquet type | Unit | Null when | Meaning |
|---|---|---|---|---|
| `link_id` | `string` | none | never | The link's id. |
| `bin_start_s` | `uint32` | s of the clock | never | The bin's first second. |
| `cars` | `uint32` | cars | never | The cars that entered the link in the bin. |
| `car_time_s` | `double` | s | `cars` is 0 | Their crossing times, exit less entry, summed. The mean crossing time of the bin is `car_time_s / cars`. |
| `first_exit_s` | `double` | s of the clock | `cars` is 0 | When the first of them to enter left the link. |
| `last_exit_s` | `double` | s of the clock | `cars` is 0 | When the last of them to enter left the link. |
| `bikes` | `uint32` | bikes | never | The bikes that entered the link in the bin, each counted once. |
| `walkers` | `uint32` | walkers | never | The walkers that entered the link in the bin. |

**For bikes.** Bikes have a count and no time column: a bike crosses a link at its free-flow time, the link's length
over `bike_speed` ([The loading](../concepts/loading.md)). The same holds for walkers.

## `iterations.parquet`

% src: crates/io-parquet/README.md ("Outputs", "Meanings and assumptions"); crates/io-parquet/src/iterations.rs (the column table); crates/loop/README.md ("Outputs", "Assess"); schema read with pyarrow from a run of delft

One row per iteration, from 0. The iteration and its measures are explained in
[The iteration](../concepts/iteration.md).

| Column | Parquet type | Unit | Null when | Meaning |
|---|---|---|---|---|
| `iteration` | `uint32` | none | never | The iteration, from 0. |
| `mean_trip_time_s` | `double` | s | no trip finished | The mean door-to-door time of the finished trips. |
| `finished` | `uint64` | trips | never | The trips that arrived before the window's end. |
| `unfinished` | `uint64` | trips | never | The trips with an alternative still travelling at the window's end. |
| `without_alternative` | `uint64` | trips | never | The trips with no alternative in any mode open to them. |
| `without_transit` | `uint64` | trips | never | The trips with transit open to them and no journey offered. |
| `without_stop` | `uint64` | trips | never | Of those, the trips with no stop within `stop_walk_max` of their origin or of their destination. The rest have stops at both ends and no journey between them. |
| `offered_share` | `double` | fraction | no trip has an alternative | The share of the trips with an alternative that were offered a new choice in this iteration; 1 at iteration 0, where everyone chooses. |
| `switch_share` | `double` | fraction | no trip has an alternative | The share of the trips with an alternative that changed it in this iteration's choice. |
| `found_share` | `double` | fraction | no trip has an alternative | The share of the trips with an alternative whose search found one cheaper than every alternative of its mode in their set. |
| `would_change_share` | `double` | fraction | the choice is `"deterministic"`; no trip has an alternative | Under the logit: the share of the trips with an alternative that the logit, choosing again over their own set at the costs this iteration measured, would put on another alternative than the one they hold. 0 when the run is at the fixed point of its iterations. |
| `gap` | `double` | fraction | the choice is `"logit"`; no trip has an alternative; the least costs sum to 0 | Under the least-cost choice: the relative duality gap, the sum over trips of the cost taken less the least cost of the set, over the sum of the least costs. |
| `search_s` | `double` | s | never | The wall-clock time of the search stage. |
| `choose_s` | `double` | s | never | The wall-clock time of the choose stage. |
| `load_s` | `double` | s | never | The wall-clock time of the load stage. |
| `measure_s` | `double` | s | never | The wall-clock time of the measure stage. |
| `assess_s` | `double` | s | never | The wall-clock time of the assess stage. |

Each choice model reports one of `would_change_share` and `gap`, the measure that can reach 0 under it, and leaves
the other null. Both are reported, never judged.

## `routes.parquet`

% src: crates/io-parquet/README.md ("Outputs": routes.parquet and the kinds); crates/io-parquet/src/routes.rs (the column table); schema read with pyarrow from a run of delft

What each trip took at the last iteration, one row per leg. A trip without an alternative has no row. Routes that
nobody took are not written.

| Column | Parquet type | Unit | Null when | Meaning |
|---|---|---|---|---|
| `trip_id` | `string` | none | never | The trip's id. |
| `leg` | `uint32` | none | never | The leg's place in the trip, from 0. |
| `kind` | `string` | none | never | The kind of leg, one of the six below. |
| `seconds` | `double` | s | the kind is `car`, `bike`, `walk` or `ride` | The seconds of a `walk_gap` or a `transfer_walk`, constant over the run. |
| `links` | `list<item: string>` | none | the kind is not `car`, `bike` or `walk` | The link ids of the route in the order travelled; an empty list when the trip's two nodes are one. |
| `run_id` | `string` | none | the kind is not `ride` | The transit run ridden. |
| `board_stop` | `string` | none | the kind is not `ride` | The stop boarded at. |
| `alight_stop` | `string` | none | the kind is not `ride` | The stop left at. |
| `board_s` | `uint32` | s of the clock | the kind is not `ride` | The run's departure from the board stop. |
| `alight_s` | `uint32` | s of the clock | the kind is not `ride` | The run's arrival at the alight stop. |
| `from_stop` | `string` | none | the kind is not `transfer_walk` | The stop walked from. |
| `to_stop` | `string` | none | the kind is not `transfer_walk` | The stop walked to. |

A transit run is named by the feed's `trip_id`; a run made from `frequencies.txt` is `<trip_id>@<seconds>`, a run of
the day before that continues past midnight `<name>@previous-day`. A stop is named by the feed's `stop_id`.

### The kinds of leg

| `kind` | What it is | Columns filled |
|---|---|---|
| `walk_gap` | A walk between a door and the network, in a straight line; or between a door and a stop: the straight lines at both ends and the walk links between them. The links are not listed, and the walker is counted on no link. | `seconds` (0 where the door is at the node or stop) |
| `car` | A route on the network's links by car. | `links` |
| `bike` | A route on the network's links by bike. | `links` |
| `walk` | A route on the network's links on foot. | `links` |
| `ride` | One transit run from stop to stop. | `run_id`, `board_stop`, `alight_stop`, `board_s`, `alight_s` |
| `transfer_walk` | A footpath of the timetable between two rides. | `from_stop`, `to_stop`, `seconds` |

| Trip by | Its legs, in order |
|---|---|
| car, bike or on foot | a `walk_gap`, the route (`car`, `bike` or `walk`), a `walk_gap` |
| transit | a `walk_gap`, its rides, with a `transfer_walk` between two rides where the traveller changes stop, a `walk_gap` |

Where the next ride leaves from the stop the last one was left at, there is no `transfer_walk` row. The seconds of a
route are not written: the run keeps each trip's realised door-to-door time, in `trips.parquet`. The seconds of a
ride are its two clock seconds.

The legs of one transit trip of the same run of Delft, the first one with a transfer walk, each row shown without its
null columns:

```python
import pyarrow.parquet as pq

routes = pq.read_table("delft-run/routes.parquet").to_pylist()
trip = next(r["trip_id"] for r in routes if r["kind"] == "transfer_walk")
for row in routes:
    if row["trip_id"] == trip:
        print({name: value for name, value in row.items() if value is not None})
```

```text
{'trip_id': 'od0000016', 'leg': 0, 'kind': 'walk_gap', 'seconds': 66.64282271155085}
{'trip_id': 'od0000016', 'leg': 1, 'kind': 'ride', 'run_id': '387547693', 'board_stop': '3989648', 'alight_stop': '3990650', 'board_s': 27768, 'alight_s': 28620}
{'trip_id': 'od0000016', 'leg': 2, 'kind': 'transfer_walk', 'seconds': 15.14197700218636, 'from_stop': '3990650', 'to_stop': '3428643'}
{'trip_id': 'od0000016', 'leg': 3, 'kind': 'ride', 'run_id': '370521830', 'board_stop': '3428643', 'alight_stop': '3621208', 'board_s': 28800, 'alight_s': 29640}
{'trip_id': 'od0000016', 'leg': 4, 'kind': 'walk_gap', 'seconds': 942.7743718115216}
```

**For bikes.** A bike trip is a `walk_gap` from the door to the bike network, a `bike` leg with its links, and a
`walk_gap` to the door. A bike is never part of a transit trip: the walks to and from a stop are on foot.
