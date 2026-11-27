# Reading the results

% src: crates/io-parquet/README.md ("Outputs", "Units", "Meanings and assumptions"); crates/binding/README.md ("Outputs")

A run writes four tables as Parquet files, and its record, into the folder given as `out`. The `Run` it returns holds
their paths, nothing else. The examples here read the run of the published Delft case ([The cases](cases.md)).

## What you need

A run writes its four tables as Parquet files, and the package installs nothing else. To read them in Python, install
a Parquet reader. The easy choice is pandas with pyarrow:

```text
pip install pandas pyarrow
```

Then `pandas.read_parquet(run.trips)` gives a DataFrame. A DataFrame is shown as a table by VS Code's Data
Wrangler extension, which also opens a `.parquet` file, and by PyCharm's DataFrame viewer. Any other Parquet reader (polars, DuckDB, R's
arrow) reads the files as well.

## The four tables

| Attribute | File | One row per | It answers |
|---|---|---|---|
| `run.trips` | `trips.parquet` | trip | which mode each traveller took, when it arrived, whether it finished |
| `run.links` | `links.parquet` | link and time bin that someone entered | how many cars, bikes and walkers entered each link in each bin, and how long the cars took |
| `run.routes` | `routes.parquet` | trip and leg | which links, runs and stops each traveller used at the last iteration |
| `run.iterations` | `iterations.parquet` | iteration | how the run settled, and how long each stage took |
| `run.record` | `run_record.toml` | (a text file) | what was run ([Running a run again](reproducibility.md)) |

Rules that hold for all four: identifiers are text; every time of day is in seconds after midnight; a duration is in
seconds; a value that does not exist is null (pandas prints a null as `NaN`). Every column is listed in
[Outputs](../reference/outputs.md), and what the numbers mean in [What a run gives](../concepts/outputs.md).

```python
import openmobisim as oms
import pandas

run = oms.Run.open("delft-run")        # or the Run that run_file returned
```

## Mode shares and travel times

% src: crates/io-parquet/README.md (`trips.parquet`)

`trips.parquet` has the mode each traveller took (`mode`), its door-to-door time (`travel_time_s`) and whether it was
at its door before the window ended (`finished`). The example prints each mode's share of the trips, in %, and its
mean door-to-door time, in minutes:

```python
trips = pandas.read_parquet(run.trips)
print(len(trips), "trips;", trips["finished"].sum(), "finished")
print((100 * trips["mode"].value_counts(normalize=True)).round(1))
print((trips.groupby("mode")["travel_time_s"].mean() / 60).round(2))
```

```text
37649 trips; 37649 finished
mode
car        59.7
transit    24.4
bike       11.6
walk        4.3
Name: proportion, dtype: float64
mode
bike       12.85
car         8.31
transit    28.26
walk       24.81
Name: travel_time_s, dtype: float64
```

An unfinished trip has no travel time (null), so a mean is over the finished trips only: read it beside the count of
unfinished trips ([The window and unfinished trips](window.md)). Here every trip finished. `mode` is null for a trip
without an alternative, one that no mode open to it can make.

## The traffic on a link

% src: crates/io-parquet/README.md (`links.parquet`, "Bikes")

`links.parquet` has one row per link and time bin in which someone entered the link. `cars`, `bikes` and `walkers`
count those who entered in the bin, each once; `car_time_s` is the sum of the crossing times of those cars, so the
mean crossing time is `car_time_s / cars`. The bin is 300 s long by default (`link_bin`). The example finds the link
most cars entered and prints its first six bins, from 07:30 (27 000 s):

```python
links = pandas.read_parquet(run.links)
busiest = links.groupby("link_id")["cars"].sum().idxmax()
print(busiest)
one = links[links["link_id"] == busiest].sort_values("bin_start_s")
one = one.assign(car_mean_s=(one["car_time_s"] / one["cars"]).round(1))
print(one[["bin_start_s", "cars", "car_mean_s", "bikes", "walkers"]].head(6).to_string(index=False))
```

```text
car:488347925:00003309
 bin_start_s  cars  car_mean_s  bikes  walkers
       27000   296        24.4      0        0
       27300   403        24.4      0        0
       27600   406        24.4      0        0
       27900   414        24.4      0        0
       28200   418        24.4      0        0
       28500   422        24.4      0        0
```

This link, 675 m of a four-lane motorway, takes about 400 cars per 5 minutes, and they cross it in 24.4 s on average,
0.1 s more than its free-flow time of 24.3 s (its length over its speed, from the case's `links.parquet`).
It has no bikes or walkers because each link of the published cases serves one mode; the cyclists of the same street
are on its bike link ([Cycling](cycling.md) reads those). A link and bin nobody entered has no row. Bikes and walkers
have no time column, since both cross a link in its length over their speed. A car's crossing is filed under the bin
in which it entered the link.

## A trip's legs

% src: crates/io-parquet/README.md (`routes.parquet`)

`routes.parquet` has what each traveller took at the last iteration, one row per leg, in order (`leg` from 0). The
kinds of leg are:

| `kind` | What it is | Columns that describe it |
|---|---|---|
| `walk_gap` | a walk between a door and the network, in a straight line; or between a door and a stop: the straight lines at both ends and the walk links between them. The links are not listed, and the walker is counted on no link. | `seconds` |
| `car`, `bike`, `walk` | a route on the network | `links`, the link ids in the order travelled |
| `ride` | one transit run from stop to stop | `run_id`, `board_stop`, `board_s`, `alight_stop`, `alight_s` |
| `transfer_walk` | a walk between two stops, between two rides | `from_stop`, `to_stop`, `seconds` |

A car, bike or walk trip is a `walk_gap`, its route and a `walk_gap`; a transit trip is a `walk_gap`, its rides with
the transfer walks between them, and a `walk_gap`. The example joins the first trip by transit to its legs:

```python
routes = pandas.read_parquet(run.routes)
first = trips[trips["mode"] == "transit"].head(1)
legs = first[["trip_id", "departure_s", "arrival_s"]].merge(routes, on="trip_id")
print(legs[["trip_id", "departure_s", "arrival_s"]].head(1).to_string(index=False))
print(legs[["leg", "kind", "seconds", "run_id", "board_stop", "board_s", "alight_stop", "alight_s"]].to_string(index=False))
```

```text
  trip_id  departure_s    arrival_s
od0000015        29818 32371.082464
 leg     kind     seconds    run_id board_stop  board_s alight_stop  alight_s
   0 walk_gap 1079.097181       NaN        NaN      NaN         NaN       NaN
   1     ride         NaN 387547703    3989647  31380.0     3990662   31680.0
   2 walk_gap  691.082464       NaN        NaN      NaN         NaN       NaN
```

This traveller leaves at 08:16:58 (29 818 s), walks 1 079.1 s to stop `3989647`, waits there until the run
`387547703` leaves at 08:43:00 (31 380 s), rides 300 s, and walks 691.1 s to the door: 31 680 s + 691.1 s is the
arrival, 32 371.1 s. The stop and run identifiers are the GTFS feed's. A route's `seconds` is null: the table of trips
holds the time the trip took, door to door.

## The iterations

% src: crates/io-parquet/README.md (`iterations.parquet`); crates/loop/README.md ("Assess", "Stop")

`iterations.parquet` has one row per iteration: the mean trip time of the finished trips, the counts of finished and
unfinished trips and of trips without an alternative, the shares that say how settled the run is, and the seconds each
stage took.

```python
iterations = pandas.read_parquet(run.iterations)
columns = ["iteration", "mean_trip_time_s", "unfinished", "switch_share", "would_change_share"]
print(iterations[columns].round(4).to_string(index=False))
print(run.stopped_by_rule)
```

```text
 iteration  mean_trip_time_s  unfinished  switch_share  would_change_share
         0         1282.0379        4689        0.0000              0.0083
         1          959.4878           0        0.1876              0.1433
         2          906.1184           0        0.0875              0.1369
         3          878.2860           0        0.0574              0.1168
         4          868.7285           0        0.0378              0.0930
         5          865.9001           0        0.0271              0.0765
         6          866.2897           0        0.0194              0.0650
         7          864.3012           0        0.0147              0.0560
         8          865.3540           0        0.0108              0.0490
         9          865.4593           0        0.0096              0.0434
        10          865.1509           0        0.0068              0.0389
True
```

At iteration 0 everyone chooses on free-flow times, and 4 689 trips do not finish; from iteration 1 on, every trip
finishes and the mean settles near 865 s (14.4 min). The stopping rule ended the run after 11
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
