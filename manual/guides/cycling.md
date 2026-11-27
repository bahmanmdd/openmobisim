# Cycling

% src: the "Bikes" section of every contract (crates/*/README.md); PARAMETERS.md (`bike_speed`, `coefficients`)

The bike is a mode beside the car, the walk and public transport in every part of a run. This page gathers what a
study of cycling needs: where bikes may ride, who may cycle, how fast, how the bike enters the choice, and where the
cyclists are in the results.

| Part of a run | What it means for bikes | Where it is set or read |
|---|---|---|
| The network | a bike rides the links that allow bikes | `bike` in a link's `modes` |
| The travellers | a traveller may cycle when the trip's modes hold the bike | `bike` in a trip's `modes` |
| The speed | one speed for every bike on every bike link | the parameter `bike_speed` |
| The route | the shortest route on the bike links, found once | nothing to set |
| The choice | the bike is an alternative with its own time coefficient and constant | `bike_ride_min` and `is_bike` of the class |
| The loading | a bike rides each link at the bike speed and interacts with nothing | nothing to set |
| The results | the trip's mode and time; the cyclists per link and time bin; the route | `trips.parquet`, `links.parquet`, `routes.parquet` |
| The pages | a cyclists' layer on the map; kilometres cycled and a cyclists' map in the summary | `Run.map`, `Run.summary` |

## Bike links

% src: crates/graph/README.md ("Bikes"); crates/binding/README.md ("Bikes"); crates/io-osm/README.md ("Bike ways",
% "Direction", "Bikes")

In a links table a link is open to bikes when its `modes` hold `bike`. A street for everyone is `car;bike;walk`; a
cycle path is `bike;walk` (or `bike`) with no car field: no lanes, no car speed, no capacity. A link has a direction,
from its upstream end to its downstream end, so a path ridden both ways is two links and a one-way cycle path is one
([A case from your own data](own_data.md)).

A network read from OpenStreetMap has bike links of their own, one mode per link, built where the reader's bike rules
let a bike ride. The main rules (all of them are in [the reader's contract](../reference/contracts/io-osm.md)):

- never on motorways and their slip roads, nor where `bicycle=no` or `bicycle=use_sidepath`;
- on trunk roads, busways, footways and pedestrian streets only where a `bicycle` tag allows it;
- on every other class of road, cycleways and car streets alike;
- against a one-way street where `oneway:bicycle=no` or a contraflow cycleway tag opens it.

The bike links are joined at their own junctions and keep their own largest part in which every link can be reached
from every other, so a street a car cannot use may stay for bikes. The published cases have one mode per link too:
Delft's has 20 652 bike links ([The cases](cases.md)).

## Who may cycle

% src: crates/binding/README.md ("Inputs": a mode set, "Bikes"); crates/demand/README.md ("Bikes", "Repairs")

A traveller may cycle when `bike` is among the `modes` of its row of the trips table (`car;bike;walk;transit`). Each
end of the trip is then joined, for the bike, to the nearest node with a link a bike may use, which may differ from
the car's node; the stretch between the door and that node is walked. A trip with `bike` on a network without any
bike link loses the bike from its set, which is counted (`trips_without_bike_network`).

A bike is not taken to a stop: the walks to and from public transport are on foot, and `bikes_allowed` of a GTFS
feed is not read.

## The bike speed

% src: PARAMETERS.md (`bike_speed`); crates/graph/README.md ("Bikes"); crates/binding/README.md ("Bikes");
% crates/search/README.md ("Bikes"); crates/loading/README.md ("Bikes")

Every bike rides every bike link at one speed, the parameter `bike_speed`, 15 km/h by default (an urban cycling speed
with stops included; the source is in [Parameters](../reference/parameters.md)). It is a keyword of the network's
constructors, `Network.from_tables(nodes, links, bike_speed=18)`, and a row of `[parameters]` in a settings file.

A bike's time on a link is the link's length over the bike speed, at every instant, whatever the cars do. Two things
follow. The fastest bike route is the shortest by length, so it is found once, at the first iteration, and never
changes. And a cyclist's travel time is the same in every iteration: 1 500 m take 360 s at 15 km/h, and 180 s at
30 km/h.

## The bike in the choice

% src: crates/loop/README.md ("Choose", "Bikes"); PARAMETERS.md (`coefficients`)

The bike alternative of a traveller is its one bike route, in a nest of its own beside the car's, the walk's and
public transport's. Its cost is

$$
\text{cost}_\text{bike} = \beta_\text{ride}\, t_\text{ride} + \beta_\text{walk}\, t_\text{walk} + \beta_\text{bike}
$$

with $t_\text{ride}$ the minutes on the bike links, $t_\text{walk}$ the minutes walked between the two doors and the
bike network, and $\beta_\text{ride}$, $\beta_\text{walk}$ and $\beta_\text{bike}$ the coefficients `bike_ride_min` (0.1 per
minute by default), `walk_min` (0.165 per minute) and `is_bike` (0) of the traveller's class. Raising `is_bike` makes cycling less attractive at every
distance; lowering `bike_ride_min` favours it more the longer the ride. [Traveller classes](classes.md) shows such a
change and its effect on the mode shares.

Since a bike's time never changes, a traveller's cost of cycling is fixed over the run. Its share still moves with
congestion, because the cost of the car it is compared with does.

## The cyclists in the results

% src: crates/io-parquet/README.md ("Bikes", `routes.parquet`); crates/map/README.md ("Bikes");
% crates/summary/README.md ("Bikes"); run by the writer on the Delft run

| Where | What it holds for bikes |
|---|---|
| `trips.parquet` | `mode` is `bike`; `travel_time_s` is the door-to-door time, the two walks included |
| `links.parquet` | the column `bikes`: the cyclists who entered the link in the bin, each counted once; there is no bike time column, since a bike crosses a link at its free-flow time |
| `routes.parquet` | a leg of kind `bike` with the link ids ridden, between two legs of kind `walk_gap` |
| the map | the cyclists' layer, a line per link whose width grows with the cyclists over the window; bike journeys listed in each pair, drawn dashed |
| the summary | bikes as a row of every table by mode; the kilometres cycled; a map of the cyclists |

The example reads the bike trips of the Delft run ([The cases](cases.md)), the kilometres cycled, and the five bike
links most cyclists entered. The lengths and road classes come from the case's own links table.

```python
import pyarrow.compute as pc
import pyarrow.parquet as pq

trips = pq.read_table("delft-run/trips.parquet")
bike = trips.filter(pc.equal(trips["mode"], "bike"))
print(bike.num_rows, "trips by bike,", round(100 * bike.num_rows / trips.num_rows, 1), "% of all;",
      round(pc.mean(bike["travel_time_s"]).as_py() / 60, 2), "min door to door on average")

counts = pq.read_table("delft-run/links.parquet", columns=["link_id", "bikes"])
per_link = counts.group_by("link_id").aggregate([("bikes", "sum")])
network = pq.read_table("delft/links.parquet", columns=["link_id", "length_m", "road_class"])
per_link = per_link.join(network, "link_id")

km = pc.sum(pc.multiply(per_link["bikes_sum"], per_link["length_m"])).as_py() / 1000
print(round(km), "km cycled on the links")

for row in per_link.sort_by([("bikes_sum", "descending")]).slice(0, 5).to_pylist():
    print(f'{row["link_id"]:26} {row["road_class"]:12} {row["length_m"]:6.0f} m {row["bikes_sum"]:5} cyclists')
```

```text
4364 trips by bike, 11.6 % of all; 12.85 min door to door on average
12612 km cycled on the links
bike:7537162:00003146      unclassified    475 m   438 cyclists
bike:7537159:00003248      unclassified     26 m   341 cyclists
bike:125410977:00016628    cycleway         17 m   323 cyclists
bike:7537159:00018963      unclassified     81 m   320 cyclists
bike:7537160:00003352      unclassified    322 m   317 cyclists
```

The busiest bike link carries 438 cyclists over the two hours. The 12 612 km are the number the summary page shows as
kilometres cycled. To see one link over time, filter `links.parquet` on its `link_id` and read `bikes` by
`bin_start_s` ([Reading the results](results.md)). The numbers describe a synthetic morning, not Delft's cyclists.

## Limits

% src: README.md ("Known limits": bikes and walkers travel at free flow); crates/loading/README.md ("Bikes");
% crates/demand/README.md ("Bikes"); crates/io-gtfs/README.md ("Bikes"); crates/io-osm/README.md ("Bikes")

- **No bike congestion.** Bikes ride at free flow and interact with nothing: not with cars, not with one another. A
  busy cycle path is as fast as an empty one.
- **One speed.** Every bike rides at `bike_speed` on every bike link: no difference by rider, by kind of bike, by
  surface or slope, and none between a cycleway and a street.
- **No signals, no delay at junctions** for bikes, as for every mode.
- **One route per cyclist**, the shortest by length. Route choice by comfort or safety is not modelled.
- **No bike-and-ride.** A bike is not ridden to a stop or carried on a vehicle.
- A walk between a door and the bike network is a straight line at the walking speed.

## See also

- [The network](../concepts/networks.md), [The route search](../concepts/search.md) and
  [The loading](../concepts/loading.md): each has a section "For bikes".
- [Traveller classes and choice coefficients](classes.md): the coefficients and how to give them.
