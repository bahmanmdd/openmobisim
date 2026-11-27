# The window and unfinished trips

% src: crates/loop/README.md ("Inputs", "The window and the mean", "Repairs"); crates/loading/README.md ("Meanings and
% assumptions": the window is half-open); README.md ("Known limits")

A run simulates one stretch of the day, the **window**: it starts at the window's start with an empty network and
stops at the window's end. A trip that is not at its door when the window ends is **unfinished** and has no travel
time. Choosing the window, and reading the results with the unfinished trips in mind, is the subject of this page.

## Setting the window

% src: crates/binding/README.md (`run`: `window`; `run_file`: `window_start_s`, `window_end_s`)

The window is two times of day in seconds after midnight. It is required and never defaulted:

| Entry point | How the window is given |
|---|---|
| `run` | the keyword `window=(start_s, end_s)`, for example `window=(27000, 34200)` for 07:30 to 09:30 |
| a settings file | the keys `window_start_s` and `window_end_s` ([The settings file](settings.md)) |

The window is half-open: it holds its start and not its end. An empty or reversed window is refused; a window
may run past 24 h.

## What happens at its edges

% src: crates/loop/README.md ("Repairs"); crates/loading/README.md ("Meanings and assumptions")

| What | What the run does | Where it is counted |
|---|---|---|
| A window that is not a whole number of time bins long | its end is moved up to the next whole bin | `window_end_moved` |
| A trip departing before the start, or at or after the end | dropped: not run, in no result | `trips_outside_window` |
| A trip whose walk from its door to the network would end at or after the end, for a mode | that mode is taken out of its set | `modes_after_window` |
| A trip left with no mode by that | dropped | `trips_after_window` |
| A trip that departs in the window and is not at its door before the end | run, and marked unfinished | the column `finished` of `trips.parquet`; `unfinished` of `iterations.parquet` |

Each of the first four is told in one warning and counted in `Run.repairs["run"]` and in the record under
`[result.repairs]`. A dropped trip has no row in `trips.parquet`. The record keeps both ends: `window_end_s` as given
and `[result] window_end_used_s` as used.

The example runs the synthetic city `dotnet`, whose 39 919 travellers leave between 07:30 and 08:30, on a window from
07:45 to 08:28:20, which is not a whole number of 300 s bins:

```python
import pathlib, tomllib
import openmobisim as oms
import pyarrow.compute as pc
import pyarrow.parquet as pq

classes = ["bike_only", "car_and_bike", "car_only", "no_vehicle"]
network = oms.Network.from_tables("dotnet/nodes.parquet", "dotnet/links.parquet")
transit = oms.Transit.from_gtfs("dotnet/gtfs.zip", network)
demand = oms.Demand.from_table("dotnet/trips.parquet", classes, network, transit=transit)

run = oms.run(network, demand, transit, window=(27900, 30500), out="cut-run")
print(run.repairs["run"])
print(tomllib.loads(pathlib.Path(run.record).read_text())["result"]["window_end_used_s"])
print(pq.read_table(run.trips).num_rows, "trips run")
```

```text
{'window_end_moved': 1, 'trips_outside_window': 9986}
30600
29933 trips run
```

The run also gives two warnings (`UserWarning`), one per kind of repair:

```text
run: 1 windows not a whole number of link_bin: the end moved up to the next whole bin (window_end_moved; first: 30600)
run: 9986 trips departing before the window's start or at or after its end: dropped (trips_outside_window; first: t00000, t00004, t00009, t00010, t00016, …)
```

The end moved from 08:28:20 to 08:30:00 (30 600 s), and the 9 986 travellers who leave before 07:45 were dropped.

## Unfinished trips and the mean

% src: crates/loop/README.md ("The window and the mean", "Assess"); README.md ("Known limits"); run by the writer

**The mean trip time is over the finished trips only.** An unfinished trip has no arrival and no travel time (both
null in `trips.parquet`), so it is in no mean. The example runs the same travellers on a window that ends at 08:30,
when the last of them has just left:

```python
run = oms.run(network, demand, transit, window=(27000, 30600), out="hour-run")
trips = pq.read_table(run.trips)
finished = pc.sum(trips["finished"]).as_py()
print(trips.num_rows, "trips run;", finished, "finished;", trips.num_rows - finished, "unfinished")
print(round(pc.mean(trips["travel_time_s"]).as_py() / 60, 2), "min: the mean over the finished trips")

unfinished = trips.filter(pc.invert(trips["finished"]))
print(unfinished.group_by("mode").aggregate([("trip_id", "count")]).sort_by("mode").to_pylist())
print(unfinished.slice(0, 1).to_pylist()[0])
```

```text
39919 trips run; 26982 finished; 12937 unfinished
15.62 min: the mean over the finished trips
[{'mode': 'bike', 'trip_id_count': 5304}, {'mode': 'car', 'trip_id_count': 3848}, {'mode': 'transit', 'trip_id_count': 3676}, {'mode': 'walk', 'trip_id_count': 109}]
{'trip_id': 't00007', 'class_id': 'no_vehicle', 'departure_s': 29699, 'mode': 'walk', 'arrival_s': None, 'travel_time_s': None, 'finished': False, 'cost': 8.75, 'alternatives': 1}
```

| Window | Trips run | Finished trips | Unfinished trips | Mean trip time of the finished trips (min) |
|---|---|---|---|---|
| 07:30 to 08:30 | 39 919 | 26 982 | 12 937 | 15.62 |
| 07:30 to 09:30 (the published case) | 39 919 | 39 919 | 0 | 18.94 |

A third of the travellers are unfinished in the short window, and the mean of the others is 3.3 min below the mean of
all. The 26 982 trips that finish in the short window have a mean of 15.62 min in the long window too, although the two
runs are not identical trip by trip (they stop after 12 and 17 iterations): almost all of the difference between the
two rows comes from which trips are in the mean.

How to read results with this in mind:

- Read every mean beside the unfinished count. Each row of `iterations.parquet` has `finished` and `unfinished` beside
  its mean, and the summary page's tiles give the share not completed.
- **A window long enough for every trip to finish makes means comparable**, between iterations and between runs. One
  way is to let the travellers depart in the first part of the window, as the published cases do: departures from
  07:30 to 08:30, the window to 09:30.
- The stopping rule reads the mean of the finished trips. When the unfinished count differs between two iterations,
  their means are over different trips, and a queue that pushes trips past the end removes the longest of them from
  the mean.
- An unfinished trip still has its `mode`, its `cost` and its legs in `routes.parquet`: it chose and it travelled. A
  trip **without an alternative** is another thing: no mode open to it can make the trip, its `mode` is null, and it
  is counted in `without_alternative` ([Troubleshooting](troubleshooting.md)).

## The time bin

% src: crates/loop/README.md ("Inputs"); PARAMETERS.md (`link_bin`); crates/io-parquet/README.md ("Meanings and
% assumptions": a crossing is filed under the bin of entry); crates/loading/README.md ("Meanings and assumptions")

The window is cut into **time bins** of `link_bin` seconds (300 s by default), counted from the window's start. The
links table has one row per link and bin that someone entered: a window of 2 h gives up to 24 bins per link. A bin
only groups the results. The loading itself has no time step: every car enters and leaves every link at its own
instant.

A car's crossing is filed under the bin in which it entered the link, with its exact exit, even an exit past the
window's end. No crossing starts at or after the end. Changing `link_bin` changes the rows of `links.parquet`, the
bins of the map's time slider and, when the window is not a whole number of the new bins, the window's end.

## For bikes

% src: crates/loop/README.md ("Repairs": for bikes; "Bikes"); crates/loading/README.md ("Bikes")

The window applies to every mode alike. A bike trip departing outside it is dropped, and one still riding at its end
is unfinished: in the one-hour example above, 5 304 of the unfinished trips are by bike. A bike trip whose door is so
far from the bike network that the walk there would end after the window loses the bike from its set. A bike rides at
one speed whatever the cars do, so whether a bike trip finishes depends on its departure, the length of its route,
`bike_speed` and its walks between the doors and the bike network, never on congestion.

## Limits

% src: crates/loading/README.md ("Meanings and assumptions": the window); crates/loop/README.md ("Search": a limit can
% drop the earliest journey)

- The network is empty at the window's start: a trip departing before it is dropped, so no traveller is on the road
  when the window opens.
- A traveller pricing a route reads a link it would enter at or after the window's end without the cars that would
  enter it then, so the costs of trips still travelling at the end read too low. Where every trip finishes within the
  window this does not arise.
- The limit `max_transit_journeys` can drop a traveller's earliest transit journey, which may be the only one arriving
  within the window.

## See also

- [The iteration](../concepts/iteration.md): the stopping rule and what it reads.
- [The loading](../concepts/loading.md): how vehicles move from the window's start.
- [Limitations](../concepts/limitations.md): the known limits in one place.
