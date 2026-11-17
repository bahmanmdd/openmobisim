# openmobisim-demand

The trips of a run, each end linked to the network per mode, and the transit access and egress
lists of each trip: which stops a traveller can walk to from the origin, and from which stops
to the destination, with the walking seconds. Depends on `openmobisim-types`,
`openmobisim-graph`, `openmobisim-loading` (only for the free-flow link times a search is
built on), `openmobisim-search`, `openmobisim-transit` and `rayon`, nothing else.

## Inputs
- Trip rows, `TripRow { id, origin, destination, departure, class, modes }`: an external id;
  two positions, longitude/latitude or metres, all of the network's kind; the departure, a second of the
  clock; the external id of a user class; the mode set open to the trip (a given mode is a set
  of one).
- The user classes, a list of external ids.
- The `Network` (`openmobisim-graph`).
- The `Timetable` (`openmobisim-transit`) or none: its stops, by position in metres.
- `stop_walk_max`, the longest walk to or from a stop, in metres (`PARAMETERS.md`).

## Outputs
`Demand::build(&rows, &classes, &network, timetable, stop_walk_max)` returns a `Demand`, a
structure of arrays read through accessors:
- `trip_count`, `trip_ids`, `class_count`, `class_ids`; per trip its `class`, `departure`,
  `modes`, and `origin` and `destination` in metres in the network's plane.
- Per trip and mode, `origin_end` and `destination_end`: `Some(End { node, gap })`, the node
  the end is linked to and the straight line to it in metres; `None` for a mode not in the
  trip's set. Transit's end is the walk node.
- Per trip, `access` and `egress`: slices of `(StopId, Duration)`, by stop id, the walking
  seconds from the origin to the stop and from the stop to the destination; empty for a trip
  without transit in its set.
- `stop_count`, and per stop `stop_end`: its walk node and gap.
- `origins_without_stop`, `destinations_without_stop`: the trips with transit in their set
  whose list is empty at that end; `trips_without_transit`: those with either list empty.
- `fingerprint()`, a `u64` (the FNV-1a hash of `openmobisim-types`) over what the demand was
  built from, in this order: the class count, then each class's external id in id order; the
  trip count, then per trip in id order its external id, its class id, its departure, its
  modes, and its origin's and destination's x and y in metres in the network's plane. The joins
  to nodes and stops and the access and egress lists are not fed: they follow from the network,
  the timetable and `stop_walk_max`, each recorded beside it. The same rows, in any order, give
  the same fingerprint; a change to any value of a trip row or to the class list changes it.

Nothing here builds a transit query or a route: that is the loop's.

## Units
Metres and seconds. Positions are of the network's kind: on a network built from
longitude/latitude they are longitude/latitude, projected with its projection; on a network
built in metres they are metres in its plane, taken as given. The walk speed is the network's
`walk_speed()`, the same on every link. The walking seconds are
`Duration`s, never rounded.

## Meanings and assumptions
- Trips and classes get dense ids by sorted external id (`ExternalIdTableBuilder`), whatever
  the order of the rows.
- Linking: each end, for each mode of its set (car, bike, walk; transit by walking), goes to
a node by straight line, ties to the lower node id; the gap is that straight line. Where
  both ends are nearest the same node of the mode's network, both join it (no link is
  travelled); otherwise the origin joins the nearest node with an out-link that allows the mode
  and the destination the nearest with an in-link. Each stop goes to its nearest node
  of the walk network, one node for the walk to it and from it (the reach is searched both
  ways from it). Whether two nodes are
  connected is the search's to find, not the linking's. The nearest node is found on a grid of cells over the nodes,
  searched in rings until no unseen cell can hold a node as near (equal to a full scan).
- Reach: once per stop, in parallel by stop, the walk network is searched from the stop's
  node both ways (`Search::reach_backward`, node to stop, and `Search::reach`, stop to node),
  bounded by `stop_walk_max` over the walk speed; the results are kept per node by stop.
  The search is public (`reach::NearStops`, with `index::NodeIndex`): the GTFS reader derives
  the footpaths between stops with it, bounded by `transfer_walk_max`.
- Lists: for a trip with transit in its set, the access list holds each stop reached on foot
  from the origin's walk node, with the walking seconds = origin gap / walk speed + node to stop
  + stop gap / walk speed; a stop is kept when the whole walk, in metres at walking speed, is at
  most `stop_walk_max`, read to a part in a billion (1.5 µm at 1 500 m) so that the rounding of
  a sum of link times cannot drop a walk of exactly the limit. The egress list is the same from
  each stop to the destination. A gap is walked in a straight line at walking speed.
- A trip with transit in its set and an empty list on either end has no transit alternative:
  counted, not an error (the run reports the count as `without_stop`; a larger `stop_walk_max`
  reaches more stops). Without a timetable every transit trip is such a trip.
- Every result is the same at any number of threads: each trip and each stop is computed
  from its own row alone, and the results are filed in id order.

## Errors
`build` refuses only what no demand can be built from, with a `DemandError` naming the trip:
`stop_walk_max` not a finite number at or above zero; then trip by trip in id order, a
position of another kind than the first trip's origin (`MixedKinds`), or of another kind than
the network's (`PositionNotTaken`: longitude/latitude on a network built in metres, or metres
on a network built from longitude/latitude; no projection converts them).

## Repairs
Every other row is used, set right or dropped, and noted in `Demand::repairs` by kind, with its
count and the first five ids; `Demand::trips_given` counts the rows given. A trip dropped is not
in the demand (no id, in no output).

| Key | Rows | What is done |
|---|---|---|
| `classes_repeated` | a class given twice | read once |
| `trips_repeated` | a trip id given twice | the first row given kept, the others dropped |
| `trips_of_unknown_class` | a trip naming a class not given | dropped |
| `trips_unplaced` | a coordinate not finite, or a longitude/latitude not projectable | dropped |
| `trips_without_car_network`, `trips_without_bike_network`, `trips_without_walk_network`, `trips_without_transit_network` | a mode of the set that no link allows (transit: walking, by which its ends are linked) | the mode taken out of the set |
| `trips_without_mode` | no mode, given so or left so | dropped |

With a timetable and no link allowing walking, no stop is linked (every transit trip has lost
transit above); `stop_count` is still the timetable's.

## Bikes
A bike's end is its own: the nearest node with a link a bike may use (its mode set holds
bike), which may differ from the car's and the walker's. On a cycle path the bike and the
walker link to the path's node while the car links to the nearest street; next to a motorway
the car may link to it and the bike may not. A bike is never in a transit list here: the
access and egress walks are on foot (a bike-and-ride access, later, would be given to the
transit query as its own list). In the fingerprint a trip's bike is in its modes: a trip
offered the bike or no longer offered it changes the fingerprint.

## Tests
`cargo test -p openmobisim-demand`. Each expected value comes from outside the crate:
linking equal to a full scan of every node for 20 000 points over a jittered 100 × 100 grid
with car-only, cycle-path and footway zones (60 000 ends, 0 mismatches, ties to the lower id
exercised on over 13 000 ends) and a bike's end on a cycle path by hand; the access and
egress lists equal to networkx's bounded Dijkstra by length plus the gaps on a 20 × 20 grid
with one-way walk links, 6 stops off the nodes and 50 trip ends (`tests/networkx_access.py`
writes the fixture; networkx 3.7; seconds within 1e-9 of metres over 4.8 km/h); the gaps by
hand (30 m + 1 000 m + 40 m = 1 070 m in 802.5 s at 4.8 km/h; at `stop_walk_max` 1 500 m a
walk of 1 500 m in, 1 501 m out); each repair by hand, counted and naming its trip, and the
refusals; the fingerprint as a property (`tests/fingerprint.rs`): two
builds of the same rows equal, and one value changed, all else equal, in each of the 9 input
columns (class id; trip id, class, departure, modes, origin x and y, destination x and y), each
a named case, changes it; the same rows in reverse order give the same fingerprint; one class
and one trip hash to the byte stream above written out by hand. The demand's determinism
over 1 and 8 threads and two runs is tested with the whole chain
(`crates/loop/tests/determinism.rs`: every end and both lists). The run time on a 20 000-node grid with 300 stops
and 13 000 trips is reported by hand: `cargo test --release -p openmobisim-demand --test
run_time -- --ignored --nocapture`.
