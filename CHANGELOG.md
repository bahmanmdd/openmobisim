# Changelog

Each release lists what changed for a user. Versions follow [semantic versioning](https://semver.org/); before 1.0,
a minor version may change the interface, and says so here.

## Unreleased

**Shared-mobility services in a run: planned on fixed expectations, played once after the run.** A run with a
docked bike service or a taxi fleet (`services=`) offers them to the trips that choose their mode, priced on what
travellers expect, fixed for the whole run: `Scenario.run(service_expectations=...)`, a preset (`"default"`,
`"optimistic"`, `"pessimistic"`) or values by name (`taxi_wait_s` 600, `taxi_fail_share` 0, `dock_pickup_extra_s`
and `dock_return_extra_s`, the extra walk expected at a pickup and a return dock in seconds, `None` by default for the
docks' start-of-day counts, `dock_worst_case` 0; every value an assumption, uncalibrated), recorded in the fingerprint and
`manifest.json`. The loadings do not play the services: a service trip arrives as planned. Once the run has stopped,
its last loading's service trips are played in one day on the fleet engine, the taxis on that loading's road times,
and what they met is reported beside what they expected.
- `trip_results` of a run with a service has two more columns: `service_outcome` (`served`, `failed`, `truncated`;
  empty for a trip without a service) and `service_met_arrival_s`.
- `kpis.parquet` has one row set per service, under its name: `service_trips`, `served`, `failed`, `truncated`,
  `expected_door_to_door_s`, `met_door_to_door_s`, `mismatch_mean_s`, `mismatch_abs_mean_s` and, for a taxi fleet,
  `wait_mean_s`; `manifest.json` has `service_expectations` and `service_day`, which says how the day was played.
- `Run.service_expectations()` returns what travellers planned on; `Run.service_met()` what the day gave, in the same
  form (the taxis' mean wait and failed share, the docks' mean extra seconds at the pickup and at the return), so a
  next run can plan on it: `scenario.run(service_expectations=run.service_met())`. A docked service's KPI rows add
  `dock_pickup_extra_s` and `dock_return_extra_s`, and `bike_share_trips` splits `extra_s` into `pickup_extra_s` and
  `return_extra_s`.
- A taxi request that gets no vehicle within the fleet's longest wait ends there, with no walk on: in a run, in
  `fleet_day` and in `fleet_environment`.
- A taxi sets off after a hailing time: `service_taxi` option `hail_s` (120 s, an assumption), the seconds from a
  request being matched to a vehicle to that vehicle driving to the pickup. The wait counts it, and a vehicle serves a
  request only if the hailing time and its drive fit within `max_wait_s`; a `hail_s` at or above `max_wait_s` is
  refused.
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
