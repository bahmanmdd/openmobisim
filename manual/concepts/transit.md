# Transit

% src: crates/io-gtfs/src/{lib.rs, read.rs}; crates/core-transit/src/{timetable.rs, raptor.rs, kind.rs, defaults.rs};
% crates/core-sim/src/transit.rs; crates/core-sim/src/itinerary_choice.rs (plan_transit, transit_part, finalise, follow);
% crates/py-bindings/src/pipeline.rs (the window cut); python/openmobisim/transit.py; python/openmobisim/scenario.py

## What it is

**Transit** is scheduled public transport: trains, metros, trams, buses and ferries running to a timetable. A transit
trip walks to a stop, rides one or more vehicles, changing between them, and walks to its destination. Transit is also
the second half of park-and-ride and bike-and-ride (see [Parking](parking.md)).

openmobisim simulates **one service day** of a timetable, read from a GTFS feed, the standard format in which transit
agencies publish their timetables. Every scheduled vehicle trip is simulated as one vehicle: a **run**, which makes a
**call** at each stop (an arrival and a departure time). **Buses ride the roads among the cars**, so congestion delays
them and their passengers; trains, metros, trams and ferries run by the schedule.

## Reading a timetable

% src: io-gtfs/src/read.rs (module docs, steps 1–5); _core.pyi transit_read_gtfs

{py:func}`openmobisim.transit_read_gtfs` reads one service day of a feed (a `.zip` or a folder of its `.txt` files):

1. **Stops** (`stops.txt`): every stop or platform with a position. Given a `network`, only the stops within 300 m of
   its walk layer's nearest node are kept.
2. **The service day** (`calendar.txt`, `calendar_dates.txt`): the date given (`date="YYYY-MM-DD"`), or else the
   **busiest weekday**, the Monday to Friday with the most trips in service, the earliest on a tie, within the first
   year of the feed's dates.
3. **Runs** (`stop_times.txt`): the calls of every trip in service that day, in order. A blank time between two timed
   calls is interpolated, by the distance along the shape where the feed gives it (`shape_dist_traveled`) and by
   straight-line distance otherwise; a trip whose first or last call has no time is dropped. Each run then keeps only its
   calls at the kept stops; a run left with fewer than two is dropped.
4. **Transfers** (`transfers.txt`): minimum transfer times between two kept stops (`transfer_type` 2, and 1, timed, as
   zero). Rows for particular routes or trips, rows between stations, and types 0 and 3 are skipped.
5. **Frequencies** (`frequencies.txt`): a trip listed there is a template. It runs from each row's `start_time`, every
   `headway_secs`, while before its `end_time`, each time as a run of its own.

Rows with a bad number or an unknown stop are skipped and counted; only a missing file or column, or a day with no
service, is an error. **Not read:** trips of the previous day that run past midnight into this one, shapes and fares.

Times count in seconds from the service day's midnight, as every time in openmobisim does: departures, the window
and the results' bins. The simulation itself starts at the window's start ([Loading](loading.md)).

### The kind of service

% src: core-transit/src/kind.rs ServiceKind::from_route_type, rides_road

Each line's GTFS `route_type` gives its kind, which the choice model sees in the minutes on board by kind
(`ride_bus_min` …):

| Kind | GTFS `route_type` | Runs |
|---|---|---|
| rail | 2, 100–199, 300–399 | by the schedule |
| metro | 1, 12, 400–699 | by the schedule |
| tram | 0, 5, 900–999 | by the schedule |
| bus | 3, 11, 200–299, 700–899 | on the roads |
| ferry | 4, 1000–1099, 1200–1299 | by the schedule |
| other | any other | by the schedule |

Trams run by the schedule because GTFS does not say which run in the street and which on their own track.

## The timetable inside the window

% src: core-transit/src/timetable.rs Timetable::within; py-bindings/src/pipeline.rs (window_cut); scenario.py Run.window

A run keeps only the part of the day inside its **simulation window** (see [Demand](demand.md)). Each vehicle run keeps
the calls it makes within the window: arriving at or after its start, leaving at or before its end. A run already under
way when the window opens starts at its first stop inside it; one still going when it closes ends at its last stop
inside it; a run left with fewer than two calls is dropped. {py:attr}`openmobisim.Run.window` says how many runs were
kept, cut and dropped.

## Stops, walks and transfers

% src: core-sim/src/transit.rs (module docs, TransitSetup, access_memo); core-transit/src/defaults.rs TransitDefaults;
% core-graph/src/layers.rs StaticLayerDefaults (walk_km_h)

**Every stop is a hub** with a transit access point (the stop), a walk access point (the walk layer's nearest node, if
within `transit_options["stop_walk_snap_m"]`, 300 m) and a bike access point where the bike layer reaches it likewise.
A stop that no pedestrian can reach keeps its runs, but nobody boards or alights there.

Walking is on the walk layer, at its walking speed (4.8 km/h):

| Walk | Limit | Option |
|---|---|---|
| from the origin to a stop, and from a stop to the destination | 30 minutes (2.4 km) | `transit_options["access_walk_max_s"]`, or the traveller class's own |
| between two stops, in a change | 5 minutes (400 m) | `transfer_walk_max_s`; `transfers.txt`'s minimum times replace the walk where the feed gives them |
| a journey's last walks together: the change to the stop it alights at, and from that stop to the destination | the walk to and from stops (above) | the same `access_walk_max_s`: a journey may not end in two walks that together pass it; if the walk to the alighting stop is too long, the stop's own ride is taken instead |

A walk's time is counted in whole seconds. These limits bound the choice set; the minutes walked are weighed by the
choice model.

## Finding the journeys: RAPTOR

% src: core-transit/src/raptor.rs (module docs, RaptorData::new, Raptor::pareto); itinerary_choice.rs plan_transit, is_loop

The journeys are found by **RAPTOR**, round-based public transit routing {footcite}`delling2015`. Round $k$ finds the
earliest arrival at every stop with at most $k$ vehicles: it scans every line pattern that calls at a stop improved in
round $k-1$, boards there the earliest run a passenger can catch, and then follows the walking transfers from the stops
the scan improved.

- A **pattern** is a set of runs of one line that call at the same stops with the same boarding rules, split so that no
  run overtakes another; the split is made on the times in use (the schedule, or the buses' times from a loading).
- **Boarding slack:** a passenger boards a run only if they are at the stop `transit_options["board_slack_s"]` (60 s)
  before it leaves, at every boarding.
- A journey takes at least one vehicle (walking all the way is another mode) and at most `max_rides` (8).
- The walking transfers come from bounded searches on the walk layer, so a stop may be reachable in two walks and not in
  one. Each stop keeps its best arrival by a ride apart from its best arrival overall, and walks start from every stop a
  ride improved, so the result is exact for single walks between rides.

**The competitive journeys.** For a trip, RAPTOR returns the **Pareto set**: for each number of vehicles, the earliest
arrival, where it beats every journey with fewer vehicles. These are the trip's transit alternatives. A journey that
boards and finally alights at the same stop is dropped. A journey longer than `journey_max_s` (two hours door to door) is
no alternative; the test is made on the realised times of the last loading (the schedule at the first choice), so a
journey at the edge of the limit can be offered one iteration and not the next when the buses run later. The [choice model](choice.md) then picks one, weighing its walks, waits, rides and changes: transit
attributes split the minutes into the walk to the first stop, the walks between stops and from the last, the wait for
the first vehicle and for the others, and the minutes on board by kind of service.

## Choosing and riding

% src: itinerary_choice.rs (module docs, Planner::transit_part, follow); raptor.rs RaptorData::ride_line

A journey is **chosen on the times expected**: the schedule at the first choice, and later the times the buses ran in
the last loading. It is **ridden on the times the run makes**: at each stop of the plan the traveller boards the first
run of the chosen line they can catch, to the chosen stop. If the line cannot be followed (its run never comes), the
traveller takes the earliest journey from where they are, and the trip is counted (`itinerary_replanned_trips`).

## Buses on the roads

% src: core-sim/src/transit.rs (module docs, BusReport, BusLoad); core-transit/src/defaults.rs

Every bus pattern is routed from stop to stop at free flow on the roads cars use and on **busways**, which only buses
use. Each bus stop is placed at the nearest node of such a road within `bus_stop_snap_m` (300 m). A pattern is **run by
the schedule** instead when a stop is farther from the roads, when the roads do not connect a stop to the next, or when
its free-flow time on the roads is more than `bus_plausibility_ratio` (1.6) times its scheduled time.
{py:attr}`openmobisim.Run.transit_summary` counts the patterns of each kind.

In a loading, each bus run is a chain of vehicles of `bus_pcu` (2 passenger car units), one per leg from stop to stop:

- a leg leaves its stop `bus_dwell_s` (20 s) after the leg before it arrived, and not before its scheduled departure;
- a bus that falls behind stays behind, and one that is early waits;
- while it dwells at a stop it is off the road, so it does not block the traffic behind it.

The loading's times replace the schedule's for these runs, and passengers plan on them at the next choice.

## Scenario changes

% src: python/openmobisim/transit.py transit_edit; python/openmobisim/scenario.py (disruptions)

| Change | How |
|---|---|
| cancel a line for the day | {py:func}`openmobisim.transit_edit` `cancel=[line]`: passengers plan without it |
| run a line at another headway | `transit_edit` `headway={line: seconds}` through its day, or `{line: (seconds, from_s, to_s)}` within a time window: each of the line's stop patterns runs every `seconds` from its first departure in the window, at the times of its middle run there; a pattern with no run in the window gets none |
| delay or cancel a line's runs during the day | `Scenario(disruptions=[{"line": ..., "delay_s": ...}])` or `{"line": ..., "cancel": True}`, for the runs leaving their first stop between `from_s` and `to_s` |

A line is named by its GTFS `route_id`, or by its short name where that is unique; `transit.lines()` lists both.

## Results

% src: python/openmobisim/scenario.py Run.transit_calls, transit_calls_table, transit_summary; io-parquet transit_calls.rs, kpis.rs

| Accessor or file | What it gives |
|---|---|
| {py:meth}`openmobisim.Run.transit_calls`, `transit_calls.parquet` | every call: run, line, kind, stop, scheduled and realised arrival and departure, boardings, alightings and the load on board as the vehicle leaves (weighted by traveller weight) |
| {py:attr}`openmobisim.Run.transit_summary` | boardings; bus patterns on the roads and run by the schedule (and why); bus runs on the roads and arrived; their mean delay at the last stop |
| `kpis.parquet` (mode `transit`) | `boardings`, `bus_runs_on_roads`, `bus_runs_arrived`, `bus_delay_mean_s` |
| {py:func}`openmobisim.viz.map_transit` | the lines and their loads, drawn |

## Defaults and options

Every option of `transit_options`, with its default and source, is in [Parameters](../reference/parameters.md). They are
uncalibrated.

## For bikes

% src: core-sim/src/transit.rs (stop_bike, the bike access point); itinerary_choice.rs (bike_transit)

- Every stop has a bike access point where the bike layer reaches it, so a bike can be ridden to a station for
  bike-and-ride (see [Parking](parking.md)).
- Bikes are not carried on board: bike-and-ride leaves the bike at a bike parking.
- Buses share the roads with cars, not with bikes: the bike layer is separate (see [Networks](networks.md)), so a bus
  does not delay a cyclist, nor a cyclist a bus.

## Limits

- One service day; trips of the day before that run past midnight are not read.
- Vehicles have no capacity: nobody is left behind at a full vehicle, and crowding is not felt.
- Only buses ride the roads; trams, metros, trains and ferries keep the schedule (or a delay given as a disruption).
- A bus's dwell is fixed, not dependent on boardings.
- Fares are not read from the feed; give prices in `price_options` (see [Modes and itineraries](modes.md)).

## References

```{footbibliography}
```
