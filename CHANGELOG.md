# Changelog

Each release lists what changed for a user. Versions follow [semantic versioning](https://semver.org/); before 1.0,
a minor version may change the interface, and says so here.

## Unreleased

- The route update can stop adding routes after a given iteration: `route_update_options={"until_iteration": k}` (0, the default, adds in every iteration). After it the route sets stay as they are and the remaining loadings settle the choices among them, without the jumps a route added late causes.
**A run with a service counts its service trips as the service day gave them.** `completion`, `completion_by_mode`, `total_travel_time_s`, `mean_travel_time_s`, the line a run prints and the last iteration's `kpis.parquet` rows (`all` and the service's modes) follow `trip_results`' `service_outcome` and `service_arrival_s`: a cancelled taxi request is counted under a new `cancelled` (`cancelled_trips`), a traveller who found no shared bike arrives when the walk on ends (`service_arrival_s` now gives that second). A run without a service is unchanged.

**`parameters()` gives every parameter's meaning and source** (columns `meaning` and `source`), written once in the package's `data/parameters.csv`; `run.toml`'s comments show each meaning's first sentence from there.

**Three parameters their arguments took are now in `parameters()` and `run.toml`:** a service's `log_events` (`[services.<name>]`), the `noop` policy's `step_s` (`[policy]`), and each road class's own volume-delay pair, `<class>.bpr_alpha` and `<class>.bpr_beta` (`[loading]`, written as comments: a class's pair wins over the pair for every class, so it is passed only when stated).

**The run configuration file, `run.toml`: every parameter of a run in one file.** `config_write_defaults(path)`
writes the complete file at the shipped defaults: one table per group (`[inputs]`, `[run]`, `[equilibration]`,
`[choice]`, `[modes]`, `[prices]`, `[services.<name>]`, `[service_assumptions]`, `[policy]`, `[network]`, `[loading]`,
`[routes]`, `[route_update]`, `[transit]`, `[parking]`), a method's name and its options in the same table, every
line's comment giving its default, its meaning and its unit, and a header naming the package version, the defaults
version and the parameters that differ from the defaults. `Config.from_arguments(...)` makes one from a run's
arguments, `config_read(path)` reads one back, every name checked (a misspelt one is refused with its table's names).
The defaults are `parameters()`'s, so the file and the code cannot disagree.
- `Scenario.run(config="run.toml")` (or a `Config`) runs with the file's values as its arguments, in place of the
  scenario's own of the same names; an argument given both in the file and in the call is refused, and so are
  `[inputs]` or `[network]` values that are not the scenario's. `Config.scenario()` reads a file's `[inputs]` through
  the package's readers, relative paths against the file, and `Config.run()` runs it.
- The command line: `openmobisim run --config run.toml`, `openmobisim config --defaults` (the complete file at the
  defaults) and `openmobisim config --from <run folder>`.
- **Every run writes `run.toml`** beside its `manifest.json` (with `outputs="minimal"` too): the complete, effective
  configuration it ran with, about a millisecond to write. `Run.config()` reads it back; on the scenario it ran,
  `scenario.run(config=run.config())` gives the same fingerprint, and a run whose inputs came from a file is repeated
  from its folder alone (`openmobisim run --config <folder>/run.toml`). What a file cannot hold (inputs given in
  memory, link values, disruptions, a Python choice model or policy) is named in its header.

**The interactive map shows every mode's trips between two places.** `viz.map_interactive` groups a run's trips by
origin-destination pair (the car route sets' pairs) and shows, for a pair, how its travellers split by mode and each
way they went: car routes, transit journeys line by line, park-and-ride and bike-and-ride, bike routes, walks, shared
bikes and taxis, each with its travellers, its expected door-to-door time and a strip of its legs (walking, waiting,
riding, driving, parking), drawn leg by leg on the map. Pick a pair from the busiest, step through them, pick an
origin and a destination on the map, or click a road for the pairs whose trips use it. The legend shows only the
layers that are on, under the controls in the right-hand column with the compass and the scale. `max_route_pairs`
keeps the pairs with the most travellers.
- The two directions of a two-way road are drawn apart, each hovered and picked on its own side (the hover names its
  upstream and downstream nodes), and the widths of the road traffic and of the trips grow linearly with volume and
  travellers, up to a width set by the room between the roads on screen.
- `Run.itinerary_journeys()`: each trip's chosen journey (of the trips `itinerary_choices()` lists), leg by leg: its
  links on its mode's layer, its rides (line, stops, seconds), its walks, waits and parking times.
- `Transit.line_patterns()`: each line's stop sequences.
- An SVG export with the transit lines on no longer fails.

**Shared-mobility services in a run: chosen on fixed assumptions, played once after the run.** A run with a
docked bike service or a taxi fleet (`services=`) offers them to the trips that choose their mode. What travellers
assume when they choose is fixed for the whole run and set by name, `Scenario.run(service_assumptions={...})`:
`assumed_wait_s` (600 s, the waiting time before a taxi picks them up), `assumed_no_taxi_share` (0, the chance of
getting no taxi), `assumed_extra_walk_at_pickup_s` and `assumed_extra_walk_at_return_s` (the extra walk a shared-bike
user assumes if the pickup dock is empty or the return dock full, in seconds; `None` by default: from the docks'
start-of-day bikes). Every value is an assumption, uncalibrated, recorded in the fingerprint and `manifest.json`
(`service_assumptions`); a misspelt name is refused with the list. The loadings do not play the services: a service
trip arrives as planned. Once the run has stopped, its last loading's service trips are played in one day on the fleet
engine, the taxis on that loading's road times, and what the fleet actually did is reported beside what was assumed.
- A taxi alternative's planned time is the assumed wait, boarding and the drive, whatever the fleet's cancel limit.
  With an assumed chance `p` of no taxi above 0, a cancelled request is followed, after the cancel limit, by the
  traveller's best other alternative; under the random utility model this is the ride, fare in full, with the cancel
  limit × `p / (1 − p)` added to its waiting in the choice, so a higher assumed chance always makes a taxi less
  likely to be chosen.
- **An operator in a run:** `Scenario.run(policy=..., policy_options=...)` steers the run's service day with a
  policy, as `fleet_day(policy=...)` takes it (a built-in one by name or object, a registered name, or a Python
  object with `step(view)`); the day is the one `fleet_day(frozen=run.freeze(...), policy=...)` replays, decision for
  decision. `Run.operator_decisions()` returns its decisions, `Run.service_policy()` its name and parameters, which
  the fingerprint and `manifest.json` (`service_policy`) record. Travellers still choose on the assumptions. Refused
  without services.
- A run with an operator writes its decisions to `operator_tasks.parquet`, one row per decision, with the columns of
  `Run.operator_decisions()` (a number a decision does not have is null); `Run.operator_tasks_table()` reads it, and a
  run read back with `Run.open` gives `operator_decisions()` from it. A run without an operator writes no such file.
- `parameters()` lists the services too: a docked bike service's and a taxi fleet's `options`, the assumptions
  (`service_assumptions`), the services' prices, the built-in policies' parameters (`policy.<name>`), the bike route
  sets' options and the fleet environment's reward weights and zone grid.
- A taxi fleet's options: `request_cancel_s` (600 s: a request is cancelled if no taxi can pick it up within this
  time; the trip ends there, with no walk on, in a run, in `fleet_day` and in `fleet_environment`),
  `dispatch_delay_s` (120 s: from a request being matched to the taxi setting off; the waiting time counts it, and a
  `dispatch_delay_s` at or above `request_cancel_s` is refused) and `boarding_s` (60 s). Assumptions, uncalibrated.
- `taxi_trips.parquet` holds one row per request: `requested_s`, `assumed_wait_s`, `outcome` (`completed`,
  `cancelled`, `unfinished`, `not_requested`), `vehicle`, `pickup_s`, `dropoff_s`, `waiting_s`, `in_vehicle_s`,
  `ride_km`, `empty_km`. `taxi_events.parquet` ends a cancelled request with `request_cancelled`.
- `kpis.parquet` has one row set per service, under its name, for the last iteration. A taxi fleet: `requests`,
  `rides`, `cancelled`, `unfinished`, `waiting_mean_s`, `waiting_median_s`, `waiting_p90_s`, `assumed_wait_s`,
  `in_vehicle_mean_s`, `ride_km_mean`, `empty_km_per_ride`, `utilisation`, `rides_per_taxi`, `kwh_charged`. A docked
  bike service: `requests`, `rides`, `no_bike`, `unfinished`, `extra_walk_at_pickup_mean_s`,
  `extra_walk_at_return_mean_s`, `assumed_extra_walk_at_pickup_s`, `assumed_extra_walk_at_return_s`,
  `redirected_share`.
- Under a mode a service serves (`bike_share` and `bike_share_transit` with a docked bike service, `taxi` with a taxi
  fleet), `kpis.parquet` keeps the rows of the trips' choice (`trips`, `people`, `mode_share`, `gap`,
  `no_vehicle_available_trips`, `no_feasible_path_trips`, `mode_not_available_trips`) and has no `completed_trips`,
  `completed_people`, `truncated_trips`, `completion_rate` or `total_travel_time_s`: how those trips went is in the
  service's rows. The `all` rows count every trip as planned, a service's included. `Run.open` leaves those keys out
  of the mode's `completion_by_mode` entry.
- `bike_share_trips.parquet` has `outcome` (`completed`, `cancelled`: no bike at any dock tried, `unfinished`,
  `not_requested`) and splits `extra_s` into `extra_walk_at_pickup_s` and `extra_walk_at_return_s`.
- The docked service's counts of docks found empty or full, in plain words: `bike_share_trips.parquet`'s
  `docks_found_empty` and `docks_found_full` (the docks a trip found without a bike, and without a free slot);
  `bike_share_docks.parquet`'s `people_found_empty_went_on` (found the dock empty and walked on to another),
  `people_found_empty_gave_up` (found it empty, none near had a bike, and walked to the destination) and
  `people_found_full_went_on` (found it full and rode on to another).
- `trip_results` of a run with a service has two more columns: `service_outcome` (`completed`, `cancelled`,
  `unfinished`; empty for a trip without a service) and `service_arrival_s`, the second the service day actually
  brought the traveller to the destination. `manifest.json` has `service_assumptions` and `service_day`, which says
  how the day was played.
- `Run.service_assumptions()` returns what the travellers assumed; `Run.service_actual()` what the service day
  actually gave, in the same form (the taxis' mean waiting time and cancelled share, the docks' mean extra walks), so a
  next run can assume it: `scenario.run(service_assumptions=run.service_actual())`.
- `fleet_day`'s and `fleet_environment`'s trip tables have `outcome`, `pickup_s`, `dropoff_s` and `waiting_s`; the
  standard reward's weight per cancelled request is `w_cancelled`; a docked service's view and observation count
  `no_bike` per dock.
- The output schema is version 5 (`openmobisim.schema_version` in every file, `manifest_version`).
- `Run.freeze` keeps the day's taxi legs and road times too: `fleet_day(frozen=...)` replays the run's day, bikes and
  taxis, leg by leg.

**Python 3.13 or newer.** openmobisim now needs Python 3.13 or later, following the scientific Python ecosystem's
support policy ([SPEC 0](https://scientific-python.org/specs/spec-0000/)); one wheel per platform serves 3.13 and
every later version, tested on 3.13 and 3.14. 0.1.0 accepted Python 3.11 and 3.12; on those, pip installs 0.1.0.

**Door-to-door times for every mode.** Every run's car and bike trips pay the fixed times of their own vehicle, in
the loading as mode choice counts them: reaching it before the leg and leaving it after (`mode_options`
`car_access_s` 120 s, `car_egress_s` 300 s, `bike_access_s` 60 s, `bike_egress_s` 60 s). Setting them to 0 leaves the
drive or the ride alone, as a benchmark such as Sioux Falls counts it. The egress is also the time to park at a car or
bike park with room; a full parking adds its slope on top.
- `trip_results` has two new columns: `vehicle_s`, the completed trip's seconds in a vehicle of its own, and
  `access_egress_s`, its fixed seconds. `Run.total_vehicle_time_s` and `Run.mean_vehicle_time_s` sum and average
  the first.
- `parking_options` `floor_car_s` and `floor_bike_s` are gone (a scenario naming them is refused with the list of
  options); `slope_car_s` and `slope_bike_s` stay.
- Before this change the loading started a car at the trip's departure and added no egress, so a car trip's travel
  time was its drive while mode choice weighed the door-to-door time: the mean trip of every run is longer now, by
  the fixed times; a run of stated car trips sets them to 0 to keep the drives.

**One hub id in the results.** `transit_calls` and `parking_bins` carry one `hub_id`, the hub of the run: a stop's
station (GTFS `parent_station`) or the stop itself; a parking's is the hub its table names, else the nearest stop's
within `hub_join_m` (150 m), else its own. The output files' schema is version 2.
- `transit_calls`: `hub_group` is now `hub_id` (the same values).
- `parking_bins` and `Run.parking_places()`: `hub_id` holds what `hub_group` held, and `hub_group` is gone; the hub a
  parking table names is still in the table.
- `Scenario.run`'s `hub_group_m` is now `hub_join_m`.

**Buses keep to their timetable.** By default every bus runs exactly on its timetable and is not a vehicle of the
loading: it takes no road capacity and traffic does not delay it. `transit_options` `buses_on_roads` 1 puts the buses
on the roads among the cars, where congestion makes them late, as 0.1.0 did. Travellers plan transit on the
timetable in every iteration, with buses on the roads too, and ride the vehicles that come; 0.1.0 planned on the
bus times of the last iteration.
- `convergence()` has two new columns: `gap_transit_plan_s`, how much later the transit trips' rides ended than on
  the timetable they planned on (0 with buses on the timetable), and `gap_transit_plan_share`, its share of the
  itineraries' gap with transit.
- The timetable searches are kept for the run and answer the same query again (the same results, sooner);
  `transit_summary` counts them (`timetable_searches_*`, `timetable_reused_*`, `timetable_kept`,
  `timetable_kept_bytes`).

**Bus lanes from OpenStreetMap.** `network_read_osm` reads the lanes reserved for buses (`busway`,
`busway:both|left|right`, `lanes:bus`, `lanes:psv`, `bus:lanes`, `psv:lanes` and their `:forward` and `:backward`
forms), per direction. A lane also designated for carpools (`hov:lanes`, `lanes:hov`, `hov=designated`) is not a bus
lane: cars keep it, as the model does not tell a car's occupancy (Paris's Boulevard Périphérique keeps its four lanes).
OpenStreetMap's `lanes` counts them with the others, so a car link's bus lanes are taken out of
the lanes cars use, never below one (where no `lanes` is tagged, out of the road class's default). With buses on the
roads (`buses_on_roads` 1), a bus on a link with a bus lane rides at free flow, outside the cars' queue; a contraflow bus lane on a one-way street is a busway link
against it, for buses alone; on the bike network a bus lane bikes share (`cycleway=share_busway`) is a bike lane.
- `bus_lanes=False` reads none of them, every lane being the cars'.
- `Network.link_bus_lanes()` gives each link's bus lanes; `report_import()` counts the car links with one, their
  length, the lane length taken from cars and the contraflow busways; `network_read_table` reads a `bus_lanes` column.
- Networks of cities that map bus lanes (Lyon, Paris) have less car capacity than before, and their results change.

**Buses on the roads keep the feed's times at their stops.** A bus stays at a stop for the feed's own dwell there
(scheduled departure less scheduled arrival; nothing where the two are equal, the running times then holding it),
plus `transit_options` `bus_dwell_s`, now 0 by default (it was 20 s at every stop, counting the dwell twice in feeds
that hold it in their running times). Each stop snaps to a road node from which its bus can go on the way it travels,
so a stop beside a dual carriageway is on the bus's side; and a one-way street buses may ride both ways
(`oneway:bus=no`, `oneway:psv=no`) is a busway against it. Buses that ride the roads are less late, and fewer bus
lines are run by the schedule for an implausible route.

**Every route search by the choice model's utility.** Route generation, the route update, the routes a traveller is
offered, the gaps, park-and-ride's drives and en-route rerouting search by a route's **generalised cost**: its travel
time plus the seconds-equivalent of every per-link term the utility weighs (length, running cost, tolls, user link
values), each user class by its own coefficients. With the defaults, which weigh only time on a car route (every price
0), car routes are exactly as before. Classes with different coefficients get their own searches, up to the new
`Scenario` argument `max_cost_vectors` (2); beyond it a class is searched by the nearest by value of time, recorded
as a `cost_vectors_grouped` diagnostic. Where a class weighs more than time, the stopping rule reads the gap within
the choice set in generalised minutes (utility over the time coefficient), shown in the new `convergence()` column
`gap_generalised_excess` (also in `kpis.parquet`; `nan` where the route cost is time alone), and so do
`Run.convergence_gap` and `Run.convergence_verdict`; the gaps in time are still reported.
- **Bike routes from the utility**: `bike_cost="utility"` is the new default (travel time plus 48 s per kilometre in
  mixed traffic at the default coefficients; a ferry is not mixed traffic); `"dedicated"` (the 1.2 factor) and
  `"time"` stay. The bike route choice now fills `bike_lane_km` and `bike_separated_km`.
- **Renamed, the old names still accepted with a `DeprecationWarning` and the same fingerprint:** `route_method`
  `"penalty"` → `"link_penalty"`, `"montecarlo"` → `"simulation"`, `"sampled"` → `"doubly_stochastic"`;
  `route_update` `"best_response"` → `"least_cost"`, `"sampled"` → `"doubly_stochastic"`; `bike_route_choice`
  `"sampled"` → `"doubly_stochastic"`; `PenaltyOptions` → `LinkPenaltyOptions`,
  `MonteCarloOptions` → `SimulationOptions`, `BestResponseOptions` → `LeastCostOptions`. A run's manifest and route
  sets keep the internal names. `parameters()` lists the groups by the new names.
- The doubly stochastic method and update keep and order their routes by the generalised cost they were searched by,
  no longer by travel time.

**Disruptions as travellers meet them.** A closed road takes no vehicle in, a departure onto it included (it waits
at its origin), and the vehicles on it when it closes drive out. Under disruptions the travellers know of, a closure
is part of the network they plan on: every route cost (the route choice from the first iteration, the route update,
the gaps, the car legs of the itineraries, skims) reads the wait until it reopens from the closure itself, and a pair
whose every route crosses a closed road is given the least route round it. A known narrowing is read from the link
times the iterations record, so it must start and end on the cost bins (`equilibration_options` `cost_bin_s`; a run
that does not is refused with the reason). Under disruptions the travellers did not know of, at flow level 2, a
vehicle whose next road is closed is offered a new route where it meets it (or at its origin), taken if it beats
waiting for the reopening by `reroute_min_gain`; nobody else is offered one, and a vehicle with no way round waits
before the closed road. One search tree from each such junction answers its vehicles for `loading_options`
`reroute_tree_s` (60 s). `loading_options` `reroute` 0 turns these offers off.
- `loading_options` `reroute_queue_ahead` is gone (the offer for a queue seen further ahead), and `route_changes`
  reasons are `"closed_ahead"` and `"stuck"` (levels 3 and 4).

**msa makes at most 15 iterations** (`equilibration_options` `iterations`; 10 before), still stopping as soon as
the disequilibrium is below `gap_tolerance` (0.05). Delft's base reached its tenth iteration above the tolerance and
now stops by it at the 13th; runs that converged within 10 are unchanged.

**The last iteration's gap on the same footing as every other.** The route sets grow after the last iteration too,
with the routes its travel times reveal, before its gaps are measured: the gaps take each set's best route, so the last
iteration of a run now reads what the same iteration of a longer run reads (0.1.0 measured it without them, about 1 %
lower on Delft). The traffic of every iteration is unchanged; `routes_added` of the last row counts those routes,
which the run's route sets keep.

## 0.1.0 — 2026-10-20

The first public release: **an alpha, for testing by students and colleagues.** The tests are not complete; please use
it and [report issues](https://github.com/bahmanmdd/openmobisim/issues).

**Networks and data**
- Road networks from OpenStreetMap (`network_read_osm`, clipped to a study area with a buffer of main roads), GMNS
  (`network_read_gmns`, read and written) or link tables (`network_read_table`, TNTP included); foreign units
  converted once on reading.
- Bike and walk networks (layers) from the same extract, with cycle tracks, painted lanes and mixed traffic told apart; ferries.
- Public transport from GTFS (one service day, clipped; `frequencies.txt` expanded); parkings from OpenStreetMap or a
  table.
- Demand from trip tables, person tables or origin–destination matrices (`demand_read_*`, `demand_from_od`);
  user classes (traveller classes) with their own modes, ownership and preferences (`classes=`, `demand_read_classes`,
  `demand_assign_classes`); `network_check` audits a network before a run.

**Simulation**
- Six modes: car, bike, walk, transit, park-and-ride, bike-and-ride; mode choice among the modes a class may use.
- Route sets and route choice for cars; mode chains (itineraries) for the multimodal modes, the transit journey found by RAPTOR and
  the parking chosen through the choice set.
- A nested logit by default, one nest per mode (the path-size logit where a run offers one mode), also the logit and
  a deterministic choice, or any model written in Python, registered by name, seeing
  times by part, money (no prices by default: give tolls, fees, fares or fuel as you need them), the network along
  each alternative (your own link values) and the traveller and trip.
- Dynamic network loading: the link transmission model applied to individual vehicles, event by event in continuous
  time, as a vertical queue (point queue; the default) or with spillback, turn pockets and en-route rerouting; buses ride the roads they share with cars, busways their own.
- Dynamic traffic assignment (equilibration) by the method of successive averages, stopping when converged, with a convergence verdict; a single
  quick loading (`equilibration="free_flow"`).
- Scenario edits (`network_edit`: closures, lanes, capacity, and cycle tracks or lanes added or taken away), timed
  disruptions (`disruptions=`), skims (`Run.skim`) and logsums (`Run.logsum_od`).

**Results**
- Per run: `kpis.parquet`, `trip_results.parquet` (every trip: who, from where to where, when, by which mode, what
  became of it, and where a car still travelling at the window's end was; `Run.trip_results()`),
  `diagnostics.parquet`, `events.parquet` (one trip in a hundred), the link bins of the road, bike and walk layers,
  `transit_calls.parquet`, `parking_bins.parquet`, `manifest.json` and `timings.csv`; or, for
  mass runs, `kpis.csv` and `manifest.json` alone (`outputs="minimal"`). A fingerprint identifies the inputs and
  settings; `Run.open` reads a finished run back.
- Figures in `openmobisim.viz` (link maps of the roads or the bike layer, routes, demand, transit by line, parkings,
  mode shares, a summary page) and an interactive map (`map_interactive`) with layers and PNG and SVG export.
- `parameters()` lists every parameter, its default and unit; every one is overridable by name.

**Starter cases and tutorials**
- Six starter cases (Delft, Amsterdam, Paris, Lyon, Sioux Falls, Nguyen–Dupuis), downloaded once and checked;
  `openmobisim check` compares an installation with this version's reference results; `openmobisim run <case>` runs
  one and writes its summary page and interactive map, each linked when written.
- Ten tutorial notebooks, in `notebooks/`, starting with `00_getting_started` (an overview of the others); in the
  package too, without outputs: `openmobisim notebooks` copies them into a folder and opens the first in JupyterLab
  (`openmobisim.notebooks()` from Python). `openmobisim` alone says what to run first.

**Known limitations** (stated with results)
- Urban free-flow speeds are the posted limits, with delay only at signals.
- Bikes: no signal delay, no interaction with cars, no gradients, one speed per facility.
- The vertical (point) queue has no spillback (`Run.report_spillback()` says where it would matter).
- The starter cases' demand is synthetic.
- The manual's [limitations](https://openmobisim.github.io/concepts/limitations.html) page lists them all.
