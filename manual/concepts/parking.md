# Parking

% src: crates/core-sim/src/parking.rs (ParkingDefaults, ParkingSetup::new, tally, availability, ExpectedAvailability);
% crates/core-sim/src/itinerary_choice.rs (Planner::plan_out, plan_back, car_candidates, prune_reach); crates/core-sim/src/run.rs;
% python/openmobisim/parking.py; crates/py-bindings (parking_read_osm); python/openmobisim/scenario.py (Run.parking_*)

## What it is

**Parkings** are the car parks and bike parkings where park-and-ride and bike-and-ride trips leave their vehicle and go
on by transit. In this version parkings serve only those two modes: a car trip to its destination does not look for a
parking, and a car parked at home is not in any parking.

Each parking holds one kind of vehicle, car or bike, has a **capacity** (how many vehicles it holds) and may start the
day with some vehicles already in it. It is a **hub**: a place where the traveller changes from the car or bike to the
pavement and from there to a stop (see [Modes and itineraries](modes.md)).

## Where parkings come from

% src: crates/core-graph (parking_read_osm via py-bindings; _core.pyi parking_read_osm); python/openmobisim/parking.py parking_read_table

| Reader | What it keeps |
|---|---|
| {py:func}`openmobisim.parking_read_osm` | from an OpenStreetMap extract: car parks that are park-and-ride (a `park_ride` tag other than `no`, or a name that says so) and every bike parking, private ones left out. The capacity is the `capacity` tag, else estimated from the area (25 m² per car, 1.5 m² per bike), else a default for a point (100 cars, 10 bikes). Parkings of one kind closer than 100 m are merged into one. All these numbers are arguments with defaults. |
| {py:func}`openmobisim.parking_read_table` | a table, one row per parking: `parking_id`, `lon`, `lat`, `vehicle` (`car` or `bike`), `capacity`; optionally `name`, `hub_id` (rows sharing one form one hub, such as a station's car park and its bike parking), `initial_occupancy` (vehicles parked at the start of the day; 0) and `fee_eur` (what a stay costs) |

A run uses parkings only with a timetable (`Scenario(transit=..., parkings=...)`).

## How a run sets them up

% src: parking.rs ParkingSetup::new

Each parking is snapped to the nearest node of its vehicle's layer (the roads for a car park, the bike layer for a bike
parking) and of the walk layer. It is **left out** if either node is farther than `parking_options["snap_m"]` (300 m),
or if no stop can be reached on foot within `parking_options["walk_max_s"]` (300 s). A parking need not be at a stop:
the traveller walks from it to the stops within that walk. {py:attr}`openmobisim.Run.parking_summary` counts the
parkings kept and those left out, and why.

## Park-and-ride and bike-and-ride: the way out

% src: itinerary_choice.rs Planner::plan_out, car_candidates, fill_car_reach, bike_reach_search, prune_reach

A trip whose car (or bike) is at its origin drives (rides) to a parking, parks, walks to a stop and rides on. Its
alternatives are **parking × the route to it × the journey from it**. They are found in four steps:

1. **The parkings within reach**: those of the vehicle's kind the car reaches within `parking_options["reach_car_s"]`
   (30 minutes) or the bike within `reach_bike_s` (20 minutes of the bike layer's cost).
2. **The candidates**: of those, the `K` nearest by the drive or ride, and the `K` best by a rough estimate of the
   whole trip (the drive or ride, the time to park, the walk to the nearest stop, and the straight-line distance on to
   the destination at `rank_speed_km_h`, 30 km/h). `K` is `parking_options["candidates"]` (3), so up to `2K` parkings
   are tried.
3. **The journeys**: for each candidate, the transit journeys from its stops, starting when the vehicle is parked (the
   drive, then the time to park), as for a transit trip (see [Transit](transit.md)).
4. **The choice set**: the alternatives within `choice_detour_limit` of the best park-and-ride (or bike-and-ride)
   alternative, of at most `K` parkings.

Car parks are ranked on the **free-flow** drive to each, searched once per run; only the candidates' drives are then
searched on the expected link times. A bike ride costs the same in every iteration, so each trip's bike parkings in
reach are found once per run and kept.

Mode choice offers these modes only to trips at least `pr_min_km` long (3 km as the crow flies). A trip that states
`car_transit` or `bike_transit` takes it at any length.

## The way back

% src: itinerary_choice.rs Planner::plan_back; vehicles.rs

A trip whose vehicle stands at a parking goes back to it: it walks to a stop, rides to that parking's stops, walks to
the parking, fetches the vehicle and drives (rides) on to its destination. **The parking is fixed**: it is where the
vehicle is. The alternatives are the journeys to it, each followed by the drive or ride onward on the expected times. A
trip whose vehicle is at a parking is offered only these ways back, whatever its mode (see
[Modes and itineraries](modes.md)).

## The time to park: soft capacity

% src: parking.rs ParkingDefaults::parking_seconds, fetch_seconds, tally, availability

A full parking refuses nobody. Parking there takes longer instead, as finding a space does. The time to park is

$$
t_\text{park} = t_\text{floor} + t_\text{slope} \, (1 - a)
$$

where $a$, the **availability**, is the share of the time bin (`parking_options["bin_s"]`, 15 minutes) in which the
parking had a free space. Fetching a vehicle takes $t_\text{floor}$ alone.

| Option | Car | Bike | What it is |
|---|---|---|---|
| `floor_car_s`, `floor_bike_s` | 120 s | 30 s | the time to park and walk out when there is room; also the time to fetch |
| `slope_car_s`, `slope_bike_s` | 600 s | 180 s | what a parking full for a whole bin adds |

Above capacity the parked vehicles simply exceed it, and the **overflow** is counted.

**Choice reads the expected availability; a traveller pays the realised one.** The **expected availability** of a
parking in a bin is the average of the availabilities realised in the loadings so far; before the first loading it is
the availability from the vehicles parked at the start of the day. After a loading, the car and bike arrivals at each
parking are tallied bin by bin, in time order (a vehicle fetched in a second frees its space before one arriving in that
second takes it), and the time each traveller pays is read from the realised availability of the bin they arrived in.
The difference between the time expected at the choice and the time paid, per traveller who parked, is the **hub
expectation mismatch** (`hub_mismatch_s` in `kpis.parquet`).

## Prices

% src: crates/core-sim/src/prices.rs; itinerary_choice.rs cost()

A parking's fee is its table's `fee_eur`, or the run's `price_options` `parking_car_eur` or `parking_bike_eur` (0 by
default). It is part of `cost_parking_eur` on the trip that parks, not on the trip that fetches. See
[Modes and itineraries](modes.md) for money.

## Results

% src: python/openmobisim/scenario.py Run.parking_bins, parking_bins_table, parking_places, parking_summary;
% crates/io-parquet/src/parking_bins.rs; io-parquet/src/kpis.rs

| Accessor or file | What it gives |
|---|---|
| {py:meth}`openmobisim.Run.parking_bins`, `parking_bins.parquet` | per parking and bin: arrivals and departures (weighted by traveller weight), mean and largest occupancy, seconds full, largest overflow |
| {py:meth}`openmobisim.Run.parking_places` | each parking kept: id, hub, vehicle, capacity, position, name |
| {py:attr}`openmobisim.Run.parking_summary` | parkings given, kept and left out; per kind: vehicles parked, those that found it full, the mismatch, vehicles left at the end of the window |
| `kpis.parquet` | `parking_arrivals`, `parking_overflow_arrivals`, `parking_full_share`, `hub_mismatch_s`, `vehicles_left_at_parkings` |
| {py:func}`openmobisim.viz.map_parking` | the parkings and how full they were, drawn |

## Defaults and options

Every option of `parking_options`, with its default and source, is in [Parameters](../reference/parameters.md). They
are uncalibrated. `reach_car_step_s` (0 by default) is for speed, not behaviour: above 0, the park-and-ride trips leaving
one road node within one step of that many seconds share one search for their drive to the car parks, made at the step's
middle, and each trip's drive along the routes found is timed at its own departure.
The mode options `car_access_s` and `car_egress_s` (the walk to the car and the search for a space, listed in
[Parameters](../reference/parameters.md)) are added to the *planned* drive to a car park; the loading itself starts the car at the trip's
departure, so a car reaches its car park about `car_access_s` earlier than its plan said. The connection is kept
(the traveller is early), and the parking's cost is unaffected.

## For bikes

% src: parking.rs (ParkingKind::Bike throughout); itinerary_choice.rs (ParkingKind::Bike in plan_out, plan_back, bike_reach_search)

- Bike parkings are parkings of their own kind, read from OpenStreetMap (every public bike parking) or a table, with
  their own capacity, soft capacity (30 s to park, up to 180 s more when full) and fee.
- Bike-and-ride is planned as park-and-ride is: the ride to the bike parkings within 20 minutes of the bike layer's cost,
  `K` nearest and `K` best tried, the journey from each.
- A station's car park and bike parking can share one hub (`hub_id`).
- The bike leg of a bike-and-ride alternative carries its kilometres by facility, as any ride does (see
  [Choice](choice.md)).
- The traveller's own bike is followed through the day: left at a station in the morning, it is fetched on the way back.

## Limits

- Parkings serve only park-and-ride and bike-and-ride. Parking at the destination of a car trip is not simulated; its
  price can be given per trip (`trip_values` `parking_eur`).
- Capacity is soft: nobody is turned away from a full parking.
- A vehicle left at a parking at the end of the window is counted (`vehicles_left_at_parkings`) but not weighed in any
  choice.
- Shared bikes and their docks are not part of this version.
