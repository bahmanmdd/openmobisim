# Running faster

Before changing anything, look where a run's time goes. Then use the levers below, most effective first. Every time
on this page was measured on a MacBook Pro with an Apple M2 Pro (8 performance and 4 efficiency cores), 32 GB of memory, macOS 26, wall-clock, nothing else running; your machine's will differ.

## Where the time goes

% src: crates/core-sim/src/timings.rs; scenario.py Run.timings, _timings_text
A run says one line when it ends: its time, its window, how many trips completed, the mean trip and the verdict.
`run.timings()` gives, and `timings.csv` records, the wall-clock time of every stage of the run, iteration by
iteration; the same as a table is logged at the `DEBUG` level
(`logging.getLogger("openmobisim").setLevel(logging.DEBUG)`), and `quiet=True` silences the line. For Delft's default
scenario, summed over the iterations:

```{include} _examples/speed-timings.md
```

`route_update` is the search for new routes on each loading's times, `loading` the traffic simulation itself,
`itinerary_choice` the choice among modes and itineraries, and `static_routes` the bike and walk routes, found once.

## The levers

% src: demand.py demand_sample; scenario.py (flow_level, route_cache, equilibration_options, threads);
% route_update options (tree_step_s); parking_options (reach_car_step_s)
| Lever | What it changes | Effect on results |
|---|---|---|
| `ms.demand_sample(trips, weight=k)` | One simulated traveller per `k` people, travelling as a group | Totals kept. Queues form and clear in blocks, so delays read a little higher |
| `flow_level=2` (the default) rather than 3 or 4 | Queues take no road space; nothing spills back | No spillback; `run.report_spillback()` shows where it would have mattered |
| `equilibration_options={"gap_tolerance": …}` | Stops iterating earlier or later | A looser equilibrium |
| `route_update_options={"tree_step_s": 1800}` | Searches new routes from shared search trees, refreshed every 30 minutes of the day | Each route the fastest at the half hour's middle. With the next row, on the four starter cities: mean trips within 0.3 min and mode shares within 0.5 points of a search per pair, links' flows no farther apart than two random seeds' (6.7–20 % against 10–20 %), the run 1.4 to 2.2 times faster (measured below). The starter cases use it |
| `parking_options={"reach_car_step_s": 1800}` | Shares the drives to park-and-ride candidates in the same way | Measured with the row above. The starter cases use it |
| `preset="fast"` | Both rows above at once (`ms.PRESETS`); an option given by name wins over the preset | As the two rows. `preset="complete"`, like no preset, keeps every search on its own |
| `route_cache=True` | Keeps generated route sets for the next run of the same scenario in the same process | None: results do not depend on it |
| `threads=` | The number of cores used | None: results do not depend on it |

### Each lever, measured

Each lever against the starter case as it ships, on Delft and on Amsterdam, one run each:

```{include} _examples/speed-levers.md
```

```{include} _examples/speed-machine.md
```

A run's time varies from one run to the next by a few per cent on the same machine; the results do not, for a given
seed and settings.

For thousands of runs, `outputs="minimal"` writes a few kilobytes per run instead of megabytes (see
[Results](results.md#many-runs-the-minimal-outputs)); the results are the same, and the writing is shorter.

The starter cities' default runs share their searches, and all but Delft simulate one traveller for every few people
(Delft, the smallest, simulates everyone). That is how they run in well under two minutes; see
[Starter cases](../reference/cases.md).

## For bikes

Bike and walk routes are found once per run (`static_routes` in the timings), not in every iteration, so cycling adds
little to a run's time.
