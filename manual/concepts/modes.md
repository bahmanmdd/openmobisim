# Modes and itineraries

% src: crates/core-demand/src/mode.rs; crates/core-sim/src/itinerary_choice.rs (Itineraries::new, offer_of, Planner::plan,
% finalise); crates/core-sim/src/layers.rs; crates/core-sim/src/run.rs; crates/py-bindings/src/pipeline.rs;
% python/openmobisim/scenario.py; python/openmobisim/demand.py

## What it is

A trip is made by one of six **modes**:

| Mode | Name in the data | How the trip travels |
|---|---|---|
| car | `car` | by the traveller's own car on the roads, among the other cars |
| bike | `bike` | by the traveller's own bike on the bike layer |
| walk | `walk` | on foot on the walk layer |
| transit | `transit` | walk to a stop, ride one or more vehicles, walk on |
| park-and-ride | `car_transit` | drive to a car park, park, walk to a stop, ride, walk on; or the way back to the car |
| bike-and-ride | `bike_transit` | the same with the bike and a bike parking |

```{image} ../_static/figures/modes_paper.svg
:alt: The six modes as chains of legs from origin to destination, each leg on its layer, with the stops and parkings where a trip changes layer
:class: only-light
:width: 100%
```
```{image} ../_static/figures/modes_night.svg
:alt: The six modes as chains of legs from origin to destination, each leg on its layer, with the stops and parkings where a trip changes layer
:class: only-dark
:width: 100%
```

A trip either **states** its mode (the `mode` column of the demand) or **chooses** it. A trip that states a mode keeps it.
A trip without a stated mode is a car trip, unless the run offers **mode choice**: then it chooses among the modes it
may use, together with the route, by the run's [choice model](choice.md).

An **itinerary** is a door-to-door alternative of a transit, park-and-ride or bike-and-ride trip: for example "walk
6 minutes to stop A, ride tram 2 to stop B, change to bus 15, ride to stop C, walk 3 minutes". In mode choice a car
route, a bike route and a walk are alternatives too, each a single leg. Every alternative carries the same attributes
(see [Choice](choice.md)), so they are weighed alike.

## Which trips choose their mode

% src: python/openmobisim/scenario.py Scenario.__init__ (modes from the classes); pipeline.rs; itinerary_choice.rs Itineraries::new

Mode choice is on when the scenario gives `modes`, or when its traveller classes say which modes their travellers may
use:

- `Scenario.from_parts(..., modes=["walk", "bike", "car", "transit"])`: a trip without a stated mode chooses among these.
- `classes=` with each class's `modes`, and no `modes`: the run offers every mode some class names, and each trip
  chooses among those its class may use.
- One mode is no choice: `modes=["car"]` is exactly the run without `modes`.

A trip with a stated mode never chooses, whatever `modes` says.

## What each mode needs

% src: itinerary_choice.rs Itineraries::new (simulated); pipeline.rs (the layers prepared); run.rs local_codes

| Mode | The run needs | The traveller needs |
|---|---|---|
| car | the road network | their car at the trip's origin |
| bike | the network's bike layer | their bike at the trip's origin |
| walk | the network's walk layer | — |
| transit | a timetable (`transit=`) and the walk layer | — |
| park-and-ride | a timetable and parkings (`parkings=`) | their car at the origin (out), or at the parking (back) |
| bike-and-ride | a timetable, parkings and the bike layer | their bike at the origin (out), or at the parking (back) |

The bike and walk layers are part of a network read from OpenStreetMap (see [Networks](networks.md)); a run prepares a
layer only if some trip uses it or may choose it, and always prepares the walk layer when it has a timetable.

A trip whose **stated** mode the run cannot simulate (transit without a timetable, say) is counted as
`mode_not_available` and does not travel. A stated car or bike trip whose vehicle is not at its origin is counted as
`no_vehicle_available`. Neither is turned into another mode.

## Where the vehicles are

% src: crates/core-demand/src/vehicles.rs (VehicleLocations, the base rule); itinerary_choice.rs Itineraries::vehicle_at

Every car and bike a traveller owns starts the day at the origin of their first trip. The run follows it through the
traveller's day: a car trip moves the car to the trip's destination; a park-and-ride trip out leaves it at the car park;
the trip back to that car park takes it on. So a traveller who drove to work drives home, and one who left their car at
a car park in the morning must fetch it.

**A vehicle at a parking is fetched.** When a traveller's car or bike stands at a parking, their next trip is offered
only the ways back to it (transit to the parking, then the vehicle onward), whatever its mode or the modes offered. Only
if there is no way back is the trip offered its other modes.

A traveller's trips choose **in order**, one after the other, since each may move a vehicle the next one needs. Trips
that state `transit`, which move no vehicle, all choose in the first round.

## What a choosing trip is offered

% src: itinerary_choice.rs Itineraries::offer_of, Planner::plan, Planner::far_enough; layers.rs StaticRoutes::leg, ClassLimits;
% core-sim/src/parking.rs ParkingDefaults (pr_min_km)

A trip that chooses is offered each mode that the run offers, that its class may use, and that it can use where it is:

| Mode | Offered when |
|---|---|
| walk | the shortest walk takes at most `mode_options["walk_max_s"]` (30 minutes by default) |
| bike | the traveller's bike is at the origin and the ride takes at most `mode_options["bike_max_s"]` (60 minutes) |
| car | the traveller's car is at the origin and the origin and destination are not the same road node |
| transit | always (it may still find no journey) |
| park-and-ride, bike-and-ride, out | the vehicle is at the origin and the trip is at least `parking_options["pr_min_km"]` long (3 km as the crow flies) |
| park-and-ride, bike-and-ride, back | the vehicle is at a parking (then nothing else is offered) |

The walk and ride limits bound the choice set; they are not preferences. A trip that states `walk` or `bike` takes it at
any length. A traveller class may give its own limits (below). The minutes within a limit are weighed by the model.

The alternatives of each mode, then, are:

| Mode | The alternatives |
|---|---|
| walk | one: the shortest walk on the walk layer, at walking speed |
| bike | one: the route on the bike layer under the bike cost (see [Networks](networks.md)) |
| car | the routes of the origin-destination pair's set, each timed on the expected link times |
| transit | the competitive journeys: for each number of vehicles, the earliest arrival that beats every journey with fewer, found by RAPTOR {footcite}`delling2015` (see [Transit](transit.md)) |
| park-and-ride, bike-and-ride | parking × the drive or ride to it × the journey from it (see [Parking](parking.md)) |

The choice set is then trimmed as [Choice](choice.md) says: within `choice_detour_limit` of the best of each mode, at
most `parking_options["candidates"]` parkings per mode, and no journey with transit over
`transit_options["journey_max_s"]`.

## Choosing and executing an itinerary

% src: itinerary_choice.rs (module docs: choosing and executing; follow); run.rs

An itinerary is **chosen on expected costs**: the last loading's link times for the car and the buses, the timetable as
the buses last ran, and how full each parking was in the iterations so far; at the first choice, free-flow times, the
schedule and the parkings as they start the day.

It is then **executed on the times of the loading**: the car of a park-and-ride trip drives among the other cars and
parks at the car park chosen; the traveller boards, at each stop of the plan, the first run of the chosen line they can
catch. If a line cannot be followed (its run never comes), the traveller takes the earliest journey from where they
are; such trips are counted (`itinerary_replanned_trips` in `kpis.parquet`).

Bike and walk trips with a stated mode are routed once per run on their layer and traversed at the layer's speeds:
their costs depend on nobody else, so the route is the same in every iteration, and they are not part of the car's
route choice or of the equilibrium gap.

## Traveller classes

% src: python/openmobisim/demand.py _class_entry, demand_read_classes, CLASS_KEYS; pipeline.rs (ClassDefaults with_fallback Ownership::ALL);
% core-sim/src/layers.rs ClassLimits

A **traveller class** groups travellers who own the same things, may use the same modes and weigh alike. A trip's class
is its `user_class`. A class may say:

| Column | What it says | If not said |
|---|---|---|
| `modes` | the modes its travellers may use | every mode the run offers |
| `owns_car`, `owns_bike` | what they own, unless a `persons` row says otherwise | as the modes imply: a car if it may drive or park and ride, a bike if it may cycle |
| `has_transit_pass` | whether they hold a pass (a holder pays no fare; never access) | false |
| `beta_*` | its own coefficients (mode constants, its own value of time) | the run's `choice_options` |
| `walk_max_s`, `bike_max_s` | the longest walk and ride offered to its choosing trips, in seconds | `mode_options`' |
| `access_walk_max_s` | the longest walk to or from a stop, in seconds | `transit_options`' |
| `share` | its share of the people, read only by {py:func}`openmobisim.demand_assign_classes` | — |

{py:func}`openmobisim.demand_read_classes` reads a class table from a CSV file;
{py:func}`openmobisim.examples.traveller_classes` is an illustrative table of four classes. A class that the scenario
does not list (every class, without `classes=`) may use every mode and owns a car and a bike. A class with none
of the run's modes leaves its choosing trips without an alternative.

`has_transit_pass` is recorded in the run's fingerprint and read by the money attributes only: a pass holder pays **no
fare** (`cost_fare_eur` is 0 for them; the pass's own price is not counted), whatever `fare_base_eur`, `fare_km_eur`
and `fare_transfer_eur` say. Who may take transit is said by the class's `modes`, never by the pass, and no class or
person holds one unless they say so (the example classes do not). Prices are 0 by default, so a pass changes nothing
until a study sets a fare; with a fare, a class that holds a pass is one that says `has_transit_pass: 1`.

## Money

% src: crates/core-sim/src/prices.rs (Prices, COST_ATTRIBUTES); itinerary_choice.rs cost(); route_choice.rs Chooser::fill

Every alternative carries what it costs, in euros, as the attribute `cost_eur` and its parts:

| Attribute | What it is | Set by |
|---|---|---|
| `cost_running_eur` | the car or bike leg's kilometres times the price per kilometre | `price_options` `car_eur_km`, `bike_eur_km` |
| `cost_toll_eur` | the car leg's tolls, paid at each passage | a road link value named `toll_eur` (`link_values=`) |
| `cost_parking_eur` | the fee of the parking a park-and-ride or bike-and-ride trip leaves its vehicle at (on the trip that parks, not the one that fetches); or, for an alternative arriving by car, the trip value `parking_eur` | `price_options` `parking_car_eur`, `parking_bike_eur`; a parking table's `fee_eur`; `trip_values` |
| `cost_fare_eur` | a journey's fare: a base fare once, a price per kilometre from each boarding stop to its alighting stop as the crow flies, and a price per transfer | `price_options` `fare_base_eur`, `fare_km_eur`, `fare_transfer_eur` |

**Every price is 0 by default.** Owning a car, a bike or a pass is access, which the classes give, and no money is
counted for it until you give prices. The built-in models weigh money by `beta_cost_eur` (−1.2 per euro by default), so
prices matter as soon as you set them.

## A trip with no alternative

% src: itinerary_choice.rs (Itineraries::choose: the fallback), Planner::fallback_car; core-sim/src/layers.rs ModeDefaults::fallback_car;
% run.rs RunResult::no_mode, fallback_car; python/openmobisim/scenario.py NoAlternativeWarning, NO_ALTERNATIVE_CHANGES,
% Run.report_no_alternative, FallbackWarning, Run.fallback_car

A transit, park-and-ride or bike-and-ride trip, or a trip choosing its mode, may have no alternative at all: no mode it
may use reaches its destination within the limits (a walk too long, no transit journey, no vehicle where it is).

By default such a trip **is not simulated, and is reported**. Every run with such trips warns with a
`NoAlternativeWarning` (once per place it is raised from under Python's default filter; set
`warnings.simplefilter("always", …)` to see it at every run). It says:
- how many trips and people;
- by class and stated mode;
- why, by mode offered;
- the places most of them are between;
- which option to change for each reason.

{py:meth}`openmobisim.Run.report_no_alternative` lists every such trip: traveller, trip, class, departure, weight,
origin, destination, stated mode, and for each mode why it gave nothing:

| Reason | Meaning | To change |
|---|---|---|
| `walk_too_long` | no walk within the walk limit | `mode_options["walk_max_s"]`, or the class's `walk_max_s` |
| `ride_too_long` | no ride within the ride limit | `mode_options["bike_max_s"]`, or the class's `bike_max_s` |
| `no_vehicle` | the traveller has no such vehicle, or left it elsewhere that day | the class's `owns_car` or `owns_bike`, or its modes |
| `no_road_route` | no road route between origin and destination | the network (`network_check`) |
| `no_stop_in_reach` | no stop within the walk to a stop, at the origin or the destination | `transit_options["access_walk_max_s"]`, or the class's `access_walk_max_s` |
| `no_transit_journey` | stops in reach, but no journey within the longest offered, on the window's timetable | `transit_options["journey_max_s"]`, the window, the timetable |
| `too_short` | shorter than the shortest park-and-ride or bike-and-ride trip | `parking_options["pr_min_km"]` |
| `unexplained` | none of these | nothing: a defect, to report |

These trips count under the mode `none` in {py:attr}`openmobisim.Run.completion_by_mode`.

**The car fallback, an option.** With `mode_options={"fallback_car": 1}` such a trip travels by car on its pair's
routes instead, if the roads connect its origin and destination. For a traveller without a car this stands for a lift
or a taxi. These trips are counted under the car, listed by {py:attr}`openmobisim.Run.fallback_car`, and announced
by a `FallbackWarning`.

## Hubs

% src: crates/core-graph/src/hubs.rs; crates/core-sim/src/transit.rs (stops as hubs); crates/core-sim/src/parking.rs (parkings as hubs)

A **hub** is a place where a traveller changes layer: from the pavement to a bus, from a car to the pavement. A hub has
**access points** (a layer and a node of it) and transfers between them. In a run:

- **every transit stop is a hub**, with a transit access point (the stop), a walk access point (the walk layer's nearest
  node, if within `transit_options["stop_walk_snap_m"]`) and a bike access point where the bike layer reaches it alike;
- **every parking is a hub**, with its car or bike access point, linked to the stops within walking reach (see
  [Parking](parking.md)).

A car or bike trip starts and ends at the road or bike layer's node nearest its origin and destination, with no walk
leg. In mode choice a car alternative also counts the time to reach the car and to park it (`mode_options`
`car_access_s` and `car_egress_s`, 2 and 3 minutes by default; see [Parking](parking.md)).

% src: crates/core-sim/src/hub_groups.rs
**Hub groups in the results.** `transit_calls` and `parking_bins` carry a `hub_group`, a label that gathers a
station's stops (by their GTFS `parent_station`) and the parkings within `hub_group_m` of one of them (an argument of
`Scenario.run`, 150 m by default), so that a station's boardings and parked bikes add up under one name. The label
changes no hub of the run and no result.

## Results

% src: python/openmobisim/scenario.py Run.trip_modes, Run.completion_by_mode, Run.itinerary_choices; io-parquet kpis (per-mode rows)

| Accessor | What it gives |
|---|---|
| {py:meth}`openmobisim.Run.trip_modes` | every trip's departure and the mode it took |
| {py:attr}`openmobisim.Run.completion_by_mode` | per mode (and `none`): trips completed and not, people, travel time |
| {py:meth}`openmobisim.Run.itinerary_choices` | per itinerary or choosing trip: mode, parking, direction, alternatives, probability, expected time, rides, logsum |
| `kpis.parquet` | the trip metrics of the last loading, also per mode (see [Outputs](outputs.md)) |
| {py:func}`openmobisim.viz.chart_mode_share` | the mode shares, drawn |

## For bikes

% src: itinerary_choice.rs offer_of (bike needs the traveller's bike at the origin), measure_bike_legs; layers.rs; vehicles.rs

- The bike is a mode of its own, and bike-and-ride a mode combining it with transit. Both are offered in mode choice
  like any other mode, with a mode constant `beta_mode_bike` (0 by default) and, for the example classes, a class's own.
- A bike trip needs the traveller's own bike at its origin, followed through the day as the car is: a traveller who
  rode to work has their bike there to ride home; one who left it at a station's bike parking must fetch it.
- A ride is offered in mode choice up to `bike_max_s` (60 minutes by default), or the class's own limit, so a class of
  keen cyclists may be offered longer rides.
- The bike alternative is one route, the best under the bike cost; its kilometres on separated tracks, painted lanes
  and in mixed traffic are attributes the model can weigh.
- Bike-and-ride trips are offered only from 3 km (`pr_min_km`), as park-and-ride ones are.
- Shared bikes are not part of this version: every bike is a traveller's own.

## Limits

- Every vehicle is the traveller's own, starting at their first trip's origin. There is no car sharing, bike sharing or
  taxi, except the car a trip with no alternative falls back to.
- A vehicle left at a parking at the end of the day is not weighed: nothing penalises not fetching it.
- A transit pass is free: its price is not among the money attributes; it waives the fare and does nothing else.
- A car or bike trip has no walk to or from the vehicle in the loading; mode choice counts two and three minutes
  for a car (`car_access_s`, `car_egress_s`), which its travel time in the results does not hold.
- A cyclist or walker has one route per trip, not a choice among routes.

## References

```{footbibliography}
```
