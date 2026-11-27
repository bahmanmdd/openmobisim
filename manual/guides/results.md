# Reading the results

% src: crates/io-parquet/README.md ("Outputs", "Units", "Meanings and assumptions"); crates/binding/README.md ("Outputs")

A run writes four tables as Parquet files, and its record, into the folder given as `out`. The `Run` it returns holds
their paths, nothing else: the tables are read with any Parquet reader. The examples here use `pyarrow` on the run of
the published Delft case ([The cases](cases.md)).

| Attribute | File | One row per | It answers |
|---|---|---|---|
| `run.trips` | `trips.parquet` | trip | which mode each traveller took, when it arrived, whether it finished |
| `run.links` | `links.parquet` | link and time bin that someone entered | how many cars, bikes and walkers entered each link in each bin, and how long the cars took |
| `run.routes` | `routes.parquet` | trip and leg | which links, runs and stops each traveller used at the last iteration |
| `run.iterations` | `iterations.parquet` | iteration | how the run settled, and how long each stage took |
| `run.record` | `run_record.toml` | (a text file) | what was run ([Running a run again](reproducibility.md)) |

Rules that hold for all four: identifiers are text; every time of day is in seconds after midnight; a duration is in
seconds; a value that does not exist is null, never `NaN`. Every column is listed in
[Outputs](../reference/outputs.md), and what the numbers mean in [What a run gives](../concepts/outputs.md).

```python
import openmobisim as oms
import pyarrow.compute as pc
import pyarrow.parquet as pq

run = oms.Run.open("delft-run")        # or the Run that run_file returned
```

## Mode shares and travel times

% src: crates/io-parquet/README.md (`trips.parquet`); run by the writer

`trips.parquet` has the mode each traveller took (`mode`), its door-to-door time (`travel_time_s`) and whether it was
at its door before the window ended (`finished`).

```python
trips = pq.read_table(run.trips)
print(trips.num_rows, "trips;", pc.sum(trips["finished"]).as_py(), "finished")
by_mode = trips.group_by("mode").aggregate([("trip_id", "count"), ("travel_time_s", "mean")])
for row in by_mode.sort_by([("trip_id_count", "descending")]).to_pylist():
    share = 100 * row["trip_id_count"] / trips.num_rows
    print(f'{row["mode"]:8} {row["trip_id_count"]:6} {share:5.1f} % {row["travel_time_s_mean"] / 60:6.2f} min')
```

```text
37649 trips; 37649 finished
car       22475  59.7 %   8.31 min
transit    9196  24.4 %  28.26 min
bike       4364  11.6 %  12.85 min
walk       1614   4.3 %  24.81 min
```

The columns printed are the mode, its trips, its share of all trips and the mean door-to-door time of its finished
trips. An unfinished trip has no travel time (null), so a mean is over the finished trips only: read it beside the
count of unfinished trips ([The window and unfinished trips](window.md)). Here every trip finished. `mode` is null for
a trip without an alternative, one that no mode open to it can make.

## The traffic on a link

% src: crates/io-parquet/README.md (`links.parquet`, "Bikes"); crates/io-parquet/src/links.rs (the columns' meanings);
% run by the writer

`links.parquet` has one row per link and time bin in which someone entered the link. `cars`, `bikes` and `walkers`
count those who entered in the bin, each once; `car_time_s` is the sum of the crossing times of those cars, so the
mean crossing time is `car_time_s / cars`. The bin is 300 s long by default (`link_bin`). The example finds the link
most cars entered and prints its first six bins:

```python
links = pq.read_table(run.links)
totals = links.group_by("link_id").aggregate([("cars", "sum"), ("bikes", "sum"), ("walkers", "sum")])
busiest = totals.sort_by([("cars_sum", "descending")]).slice(0, 1).to_pylist()[0]
print(busiest)

rows = links.filter(pc.equal(links["link_id"], busiest["link_id"])).sort_by("bin_start_s")
for row in rows.slice(0, 6).to_pylist():
    clock = f'{row["bin_start_s"] // 3600:02}:{row["bin_start_s"] % 3600 // 60:02}'
    print(f'{clock} {row["cars"]:4} cars {row["car_time_s"] / row["cars"]:5.1f} s each '
          f'{row["bikes"]:3} bikes {row["walkers"]:3} walkers')
```

```text
{'link_id': 'car:488347925:00003309', 'cars_sum': 5245, 'bikes_sum': 0, 'walkers_sum': 0}
07:30  296 cars  24.4 s each   0 bikes   0 walkers
07:35  403 cars  24.4 s each   0 bikes   0 walkers
07:40  406 cars  24.4 s each   0 bikes   0 walkers
07:45  414 cars  24.4 s each   0 bikes   0 walkers
07:50  418 cars  24.4 s each   0 bikes   0 walkers
07:55  422 cars  24.4 s each   0 bikes   0 walkers
```

This link, 675 m of a four-lane motorway, takes about 400 cars per 5 minutes, and they cross it in 24.4 s on average,
0.1 s more than its free-flow time of 24.3 s (its length over its speed, from the case's `links.parquet`).
It has no bikes or walkers because each link of the published cases serves one mode; the cyclists of the same street
are on its bike link ([Cycling](cycling.md) reads those). A link and bin nobody entered has no row. Bikes and walkers
have no time column, since both cross a link in its length over their speed. A car's crossing is filed under the bin
in which it entered the link.

## A trip's legs

% src: crates/io-parquet/README.md (`routes.parquet`); run by the writer

`routes.parquet` has what each traveller took at the last iteration, one row per leg, in order (`leg` from 0). The
kinds of leg are:

| `kind` | What it is | Columns that describe it |
|---|---|---|
| `walk_gap` | a walk between a door and the network, in a straight line; or between a door and a stop: the straight lines at both ends and the walk links between them. The links are not listed, and the walker is counted on no link. | `seconds` |
| `car`, `bike`, `walk` | a route on the network | `links`, the link ids in the order travelled |
| `ride` | one transit run from stop to stop | `run_id`, `board_stop`, `board_s`, `alight_stop`, `alight_s` |
| `transfer_walk` | a walk between two stops, between two rides | `from_stop`, `to_stop`, `seconds` |

A car, bike or walk trip is a `walk_gap`, its route and a `walk_gap`; a transit trip is a `walk_gap`, its rides with
the transfer walks between them, and a `walk_gap`. The example prints the legs of the first trip by transit:

```python
routes = pq.read_table(run.routes)
trip = pc.filter(trips["trip_id"], pc.equal(trips["mode"], "transit"))[0].as_py()
print(trips.filter(pc.equal(trips["trip_id"], trip)).select(["trip_id", "departure_s", "arrival_s"]).to_pylist()[0])
for leg in routes.filter(pc.equal(routes["trip_id"], trip)).to_pylist():
    if leg["kind"] == "ride":
        print(leg["leg"], "ride", leg["run_id"], leg["board_stop"], leg["board_s"], "->", leg["alight_stop"], leg["alight_s"])
    elif leg["kind"] == "transfer_walk":
        print(leg["leg"], "transfer_walk", leg["from_stop"], "->", leg["to_stop"], round(leg["seconds"], 1), "s")
    elif leg["kind"] == "walk_gap":
        print(leg["leg"], "walk_gap", round(leg["seconds"], 1), "s")
    else:
        print(leg["leg"], leg["kind"], len(leg["links"]), "links")
```

```text
{'trip_id': 'od0000015', 'departure_s': 29818, 'arrival_s': 32371.082463634164}
0 walk_gap 1079.1 s
1 ride 387547703 3989647 31380 -> 3990662 31680
2 walk_gap 691.1 s
```

This traveller leaves at 08:16:58 (29 818 s), walks 1 079.1 s to stop `3989647`, waits there until the run
`387547703` leaves at 08:43:00 (31 380 s), rides 300 s, and walks 691.1 s to the door: 31 680 s + 691.1 s is the
arrival, 32 371.1 s. The stop and run identifiers are the GTFS feed's. A route's `seconds` is null: the table of trips
holds the time the trip took, door to door.

## The iterations

% src: crates/io-parquet/README.md (`iterations.parquet`); crates/loop/README.md ("Assess", "Stop"); run by the writer

`iterations.parquet` has one row per iteration: the mean trip time of the finished trips, the counts of finished and
unfinished trips and of trips without an alternative, the shares that say how settled the run is, and the seconds each
stage took.

```python
iterations = pq.read_table(run.iterations)
for row in iterations.to_pylist():
    print(f'{row["iteration"]:2} {row["mean_trip_time_s"] / 60:6.2f} min {row["unfinished"]:4} unfinished '
          f'{100 * row["switch_share"]:5.1f} % switched {100 * row["would_change_share"]:5.2f} % would change')
print(run.stopped_by_rule)
```

```text
 0  21.37 min 4689 unfinished   0.0 % switched  0.83 % would change
 1  15.99 min    0 unfinished  18.8 % switched 14.33 % would change
 2  15.10 min    0 unfinished   8.8 % switched 13.69 % would change
 3  14.64 min    0 unfinished   5.7 % switched 11.68 % would change
 4  14.48 min    0 unfinished   3.8 % switched  9.30 % would change
 5  14.43 min    0 unfinished   2.7 % switched  7.65 % would change
 6  14.44 min    0 unfinished   1.9 % switched  6.50 % would change
 7  14.41 min    0 unfinished   1.5 % switched  5.60 % would change
 8  14.42 min    0 unfinished   1.1 % switched  4.90 % would change
 9  14.42 min    0 unfinished   1.0 % switched  4.34 % would change
10  14.42 min    0 unfinished   0.7 % switched  3.89 % would change
True
```

At iteration 0 everyone chooses on free-flow times, and 4 689 trips do not finish; from iteration 1 on, every trip
finishes and the mean settles near 14.4 min. The stopping rule ended the run after 11
iterations. `would_change_share` is, under the logit, the share of travellers the model would put on another
alternative if they chose again at the costs the iteration measured; it is reported, never judged. Under the
least-cost choice that column is null and `gap` is reported instead. Both, and the stopping rule, are explained in
[The iteration](../concepts/iteration.md); the stage seconds in [Run time](speed.md).

## For bikes

% src: crates/io-parquet/README.md ("Bikes")

A trip by bike has `mode` equal to `bike` and its door-to-door time like any other trip. The cyclists who entered a
link in a bin are the column `bikes` of `links.parquet`, beside `cars` and `walkers`. A bike trip's route is a leg of
kind `bike` in `routes.parquet`, with its link ids. [Cycling](cycling.md) reads the busiest bike links of Delft.

## See also

- [The map and the summary](pages.md): two pages that show these tables without any code.
- [Outputs](../reference/outputs.md): every column with its type and unit.
