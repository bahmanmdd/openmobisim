# Cycling

Bikes are a mode in their own right in openmobisim, not an afterthought of a car model. This guide gathers everything
that concerns cycling: the network bikes ride on, how they choose routes and modes, bike-and-ride, the parameters and
the results.

## The bike layer

% src: _core.pyi Network.layer; crates/core-graph/src/layers.rs; network_options.rs (bike_* rows); gmns.py
Every network has a **bike layer**: a network of its own over the same streets, read with the roads.
- **Which streets:** from OpenStreetMap, the cycleways, cycle lanes and streets bikes may use. One-way streets are
  ridden both ways where contraflow cycling is allowed. A network from another source derives its bike layer from the
  roads: bikes on every road but motorways.
- **Facility:** each bike link carries one, either `separated` (a track), `lane` (a painted lane or a cycle street)
  or `mixed` (in traffic). A GMNS bike layer gives it in `bike_facility`.
- **Speeds:** 15 km/h in mixed traffic (`bike_mixed_km_h`) and 18 km/h on dedicated infrastructure
  (`bike_dedicated_km_h`), never above the street's speed limit. Where the tags say to dismount, bikes move at walking
  speed (`walk_km_h`).

```python
bike = network.layer("bike")
ms.network_check(network)["summary"]["bike"]   # links, nodes, km, separated_share, lane_share
```

## Routes

% src: scenario.py Scenario.from_parts(bike_cost=), BIKE_COSTS; network_options.rs bike_mixed_cost_factor,
% bike_lane_cost_factor
Bike routes do not depend on traffic, so each bike trip gets its route once per run, the least-cost route on the bike
layer. How "least cost" is measured is `bike_cost`:

| `bike_cost` | A route's cost |
|---|---|
| `"dedicated"` (default) | Its time, with every minute in mixed traffic counted 1.2 times (`bike_mixed_cost_factor`) and on a painted lane 1.0 times (`bike_lane_cost_factor`): cyclists accept a small detour for a track or a lane |
| `"time"` | Its time |

## Choosing to cycle

% src: core-choice attributes (bike_min, bike_separated_km, bike_lane_km, bike_mixed_km); layers.rs ModeDefaults
% bike_max_s; demand.py (classes)
A trip that chooses its mode is offered a ride if its class may cycle and owns a bike, and if the ride takes at most
`bike_max_s` (60 minutes by default; a class may set its own). The choice model sees:
- the ride's time (`bike_min`);
- its kilometres by facility (`bike_separated_km`, `bike_lane_km`, `bike_mixed_km`);
- the bike's constant (`beta_mode_bike`).

By default each kilometre in mixed traffic weighs `beta_bike_mixed_km`. A bike trip needs the traveller's bike at its
origin, as a car trip needs their car. See [Choice](../concepts/choice.md) and [Traveller classes](classes.md).

## Bike-and-ride

% src: core-sim parking.rs, itinerary_choice.rs; parking.py; _core.pyi parking_read_osm
A bike-and-ride trip rides to a bike parking near a stop, parks and continues by transit. It needs `bike_transit`
among the class's modes, a timetable and bike parkings (`parking_read_osm` keeps every public bike parking). The
station is chosen by the choice model among candidates within the bike's reach (`parking_options["reach_bike_s"]`).
See [Parking](../concepts/parking.md).

## Results

% src: scenario.py Run.link_bins(layer="bike"), link_bins_table; io-parquet link_bins (link_bins_bike.parquet)
- **Flows on the bike layer:** `run.link_bins("bike")` and `link_bins_bike.parquet`, per bike link and time bin.
  There, `pcu` counts travellers.
- **Mode shares and trips:** the `bike` rows of `kpis.parquet`, and `run.completion_by_mode`.
- **Parkings:** bike parkings' occupancy in `parking_bins.parquet`.

## Changing the cycling network

% src: network.py network_edit (bike_facility); writer T's finding on the speed limit
`ms.network_edit(network, …, bike_facility=…)` changes a bike link's facility, for example to test a new cycle track.
In this version, an edited link rides at the full facility speed, without the street's speed limit applied.

## Limits

- Bike routes do not depend on traffic or on other cyclists: there is no bike congestion.
- Each bike trip takes its single least-cost route; there is no stochastic bike route choice.
- No bike sharing in this version.
