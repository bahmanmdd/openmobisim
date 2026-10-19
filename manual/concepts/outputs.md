# Outputs

A run returns a {py:class}`openmobisim.Run` in Python and writes its results into its `output_dir`
(`./openmobisim-runs/<run_id>` under the current directory unless given) as files that pandas, polars or R read
directly. A folder that already holds a run is never replaced silently: `run(..., overwrite=True)` says so. This page explains what is recorded and how. Every column is listed in
[Output files](../reference/outputs.md).

## What becomes of each trip

% src: crates/core-sim/src/run.rs TripCompletionStats (fields, attempted, completion_rate); scenario.py Run.completion,
% Run.report_no_alternative
Every trip of the run ends in exactly one of these outcomes:

| Outcome | Meaning |
|---|---|
| completed | Arrived within the simulation window |
| truncated | Still travelling when the window ended |
| no vehicle available | Its stated mode needs a vehicle its traveller does not have at the origin |
| no feasible path | A car was available, but no road route reached the destination |
| mode not available | The run lacks what the mode needs (a layer, a timetable, parkings) |
| no alternative | It chose its mode, or was given transit, park-and-ride or bike-and-ride, and none reached the destination within the limits. A trip that chose is counted under the mode `none`, one given its mode under that mode |

Trips departing outside the window are not in the run at all; see [The simulation window](../guides/window.md).

The **completion rate** is the completed trips over the trips that entered the network (completed plus truncated). The
trips that never left are not in it. {py:attr}`openmobisim.Run.completion` and
{py:attr}`openmobisim.Run.completion_by_mode` give the counts.

% src: crates/core-sim/src/trip_results.rs (TripOutcomes, TripTable); crates/core-loading/src/ltm/mod.rs
% unfinished_links; crates/io-parquet/src/trip_results.rs; python/openmobisim/run.py Run.trip_results
**Every trip, one by one.** {py:meth}`openmobisim.Run.trip_results` gives one row per trip, also written as
`trip_results.parquet`: its traveller, class and weight, where it starts and ends, when it departs, the mode it took,
its outcome, and, if it completed, when it arrived and how long it took. For a trip still travelling when the window
ended whose car was on the roads, `link_at_end` is the road link it was on, or waited at its origin to enter: where
the unfinished cars are. To see the unfinished trips:

```python
t = run.trip_results().to_pandas()
t[t.outcome == "truncated"]
```

Counted by `outcome`, the trips agree with `completion`, with one difference: a trip with no alternative (those
{py:meth}`openmobisim.Run.report_no_alternative` lists, but for any that then went by car) is `no_alternative` here,
and counted in `completion` under the outcome it had, most often `no_feasible_path`. The
outcomes are those of the last loading. Building and writing the table adds one pass over the trips at the end of the
run. On the starter cases it adds 0.02–0.04 s to a city's run (under 0.4 %) and 0.07–0.10 s to Sioux Falls's runs of
two to four seconds (2–3 %); the file is 1–2 MB for a city.

**Two things to know when reading travel times.** A car trip's `travel_time_s` is its time on the roads, from its
departure to its arrival at the destination's node: the two minutes to reach the car and the three to park it that
mode choice weighs (`car_access_s`, `car_egress_s`; see [Modes and itineraries](modes.md)) are not in it, while a
transit trip's time holds its walks and waits, so a mean trip by mode sets a drive beside a door-to-door journey. And a
trip that states `car` whose origin and destination snap to the same road node completes at its departure, with a
travel time of 0 (in Delft, 3 of 65 956 trips, each a few hundred metres long); a trip choosing its mode is not
offered the car in that case.

## Weights: trips and people

% src: crates/io-parquet/src/kpis.rs (people, completed_people; total_travel_time_s weighted)
A simulated traveller may stand for several people (its weight; see `demand_sample`). Results that count people are
weighted. Among them: `people`, `completed_people`, the total travel time (`total_travel_time_s` sums weight × travel
time), boardings, and parking arrivals. `trips` and `completed_trips` count simulated trips. `manifest.json`'s
`kpi_weighting` records which form the travel time takes.

## The files

% src: crates/io-parquet/src/{kpis,link_bins,transit_calls,parking_bins,trip_results,events,diagnostics,manifest}.rs
| File | One row per | What it holds |
|---|---|---|
| `kpis.parquet` | iteration, mode and metric | The run's metrics in long format (`metric`, `value`). The trip metrics of the last loading are written for `all` and for each mode with trips; the convergence measures for every iteration; transit and parking totals |
| `link_bins.parquet` | time bin and road link with traffic | `crossings` (traversals that finished in the bin, unweighted), `pcu` (traffic that left the link, weight and vehicle size included), `pcu_seconds` (the time those traversals took). `pcu_seconds / pcu` is the mean traversal time; `pcu × 3600 / bin_seconds` is the flow per hour. Bins are counted from midnight |
| `link_bins_bike.parquet`, `link_bins_walk.parquet` | time bin and bike or walk link | The same, on the bike and walk layers; `pcu` counts travellers |
| `trip_results.parquet` | trip | Every trip and what became of it: traveller, class, weight, origin, destination, departure, mode, outcome, arrival and travel time if it completed, and for a car still travelling at the window's end the road link it was on; see [above](#what-becomes-of-each-trip) |
| `transit_calls.parquet` | call (a run at a stop) | Scheduled and realised times, boardings, alightings and passengers on board as the vehicle leaves. A bus that rode the roads keeps the loading's times; empty where it had not reached the call by the window's end |
| `parking_bins.parquet` | parking and time bin | Arrivals, departures, mean and peak occupancy, seconds full, and the most vehicles over capacity (a full parking refuses nobody). Bins are counted from midnight, like `link_bins` |
| `events.parquet` | trip event | A sample of the trips' events, one in a hundred: second, type, what it concerns |
| `diagnostics.parquet` | diagnostic | Counters of what the run met (category, code, severity, element, count) |
| `manifest.json` | — | What was run: fingerprint, versions, platform, seed, every method with its options, the window, the totals; see [Reproducibility](reproducibility.md) |
| `timings.csv` | stage | Wall-clock seconds by stage and iteration; see [Running faster](../guides/speed.md) |

The parquet files carry the run's fingerprint and master seed in their metadata, so a file read on its own can be
traced to its run.

## Reading results in Python

% src: python/openmobisim/scenario.py Run (methods)
| Question | `Run` |
|---|---|
| How did trips end? Which did not finish, and where were they? | `completion`, `completion_by_mode`, `trip_results()`, `report_no_alternative()` |
| Which mode did each trip take? | `trip_modes()`, `itinerary_choices()` |
| What happened on each link? | `link_bins(layer)`, `link_bins_table(layer)` |
| How did transit and parkings do? | `transit_calls()`, `transit_summary()`, `parking_bins()`, `parking_summary()` |
| Did the run settle? | `converged`, `convergence_gap`, `convergence_verdict`, `convergence()` |
| Where did queues outgrow the road, or traffic lock? | `report_spillback()`, `report_gridlock()` |
| How long between places, by mode? How good is the choice set? | `skim(...)`, `logsum_od(...)` |
| Which routes did cars take, and change on the way? | `route_choices`, `route_realised()`, `route_changes()` |

## For bikes

Bike trips count in every metric under the mode `bike`, and bike-and-ride under `bike_transit`. Their flows are in
`link_bins_bike.parquet`, per bike link. Bike parkings' occupancy is in
`parking_bins.parquet`.

## Limits

% src: crates/io-parquet/src/lib.rs (what is not written)
- `kpis.parquet`'s `design_id` and `replication` are the labels the run was given, `"default"` and 0 unless set.
- `events.parquet` has no payload column. Boardings, parking and reroutes have files or methods of their own.
- `diagnostics.parquet` has no free-text detail.
