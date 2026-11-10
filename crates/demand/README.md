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
- `stop_walk_max`, the longest walk to or from a stop that the lists hold beside others, in
  metres (`PARAMETERS.md`): an end with no stop within it gets its nearest stop instead.

## Outputs
`Demand::build(&rows, &classes, &network, timetable, stop_walk_max)` returns a `Demand`, a
structure of arrays read through accessors:
- `trip_count`, `trip_ids`, `class_count`, `class_ids`; per trip its `class`, `departure`,
  `modes`, and `origin` and `destination` in metres in the network's plane.
- Per trip and mode, `origin_end` and `destination_end`: `Some(End { node, gap })`, the node
  the end is linked to and the straight line to it in metres; `None` for a mode not in the
  trip's set. Transit's end is the walk node.
- Per trip, `access` and `egress`: slices of `(StopId, Duration)`, by stop id, the walking
  seconds from the origin to the stop and from the stop to the destination: every stop within
  `stop_walk_max`, or, with none within it, the one stop with the least walk; empty for a trip
  without transit in its set, and at an end from which no stop can be reached on foot.
- `stop_count`, and per stop `stop_end`: its walk node and gap.
- `origins_without_stop`, `destinations_without_stop`: the trips with transit in their set
  whose list is empty at that end, that is from whose end no stop can be reached on foot at any
  distance; `trips_without_transit`: those with either list empty.

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
  the nearest node by straight line among the nodes with an out-link or an in-link that allows
  the mode, ties to the lower node id; the gap is that straight line. Each stop goes to its
  nearest walk node the same way. The nearest node is found on a grid of cells over the nodes,
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
- Nearest stop: an end with no stop within `stop_walk_max` gets, as its only entry, the stop
  with the least whole walk (end gap + network walk + stop gap), ties to the lower stop id,
  with its walking seconds; a stop beyond the limit is never in a list beside one within it.
  Origins walk from the end to the stop, destinations from the stop to the end. Found by one
  walk search per distinct node of such ends, in parallel, bounded first at twice the limit
  and doubled while a stop not yet reached could still be nearer, unbounded after 16 times the
  limit (`reach::nearest_stops`); the bound changes the work, never the stop.
- A trip with transit in its set and an empty list on either end (no stop reachable on foot
  from it) has no transit alternative: counted, not an error. Without a timetable every
  transit trip is such a trip.
- Every result is the same at any number of threads: each trip and each stop is computed
  from its own row alone, and the results are filed in id order.

## Errors
`build` refuses, never repairs, with a `DemandError` naming the trip, class or stop by
external id, the first fault in this order: `stop_walk_max` not a finite number at or above
zero; a class given twice; a trip given twice; then trip by trip in id order, a class not
given, an empty mode set, a position of another kind than the first trip's origin,
a position the network cannot take (not of the network's kind: longitude/latitude on a
network built in metres, or metres on a network built from longitude/latitude; out of the
projectable range; or a coordinate that is not finite), a mode of the set that no link allows
(transit: walking); then a stop when no link allows walking.

## Bikes
A bike's end is its own: the nearest node with a link a bike may use (its mode set holds
bike), which may differ from the car's and the walker's. On a cycle path the bike and the
walker link to the path's node while the car links to the nearest street; next to a motorway
the car may link to it and the bike may not. A bike is never in a transit list here: the
access and egress walks are on foot (a bike-and-ride access, later, would be given to the
transit query as its own list).

## Tests
`cargo test -p openmobisim-demand`. Each expected value comes from outside the crate:
linking equal to a full scan of every node for 20 000 points over a jittered 100 × 100 grid
with car-only, cycle-path and footway zones (60 000 ends, 0 mismatches, ties to the lower id
exercised on over 13 000 ends) and a bike's end on a cycle path by hand; the access and
egress lists equal to networkx's Dijkstra by length plus the gaps on a 20 × 20 grid
with one-way walk links, 6 stops off the nodes and 50 trip ends (`tests/networkx_access.py`
writes the fixture; networkx 3.7; seconds within 1e-9 of metres over 4.8 km/h); the gaps by
hand (30 m + 1 000 m + 40 m = 1 070 m in 802.5 s at 4.8 km/h; at `stop_walk_max` 1 500 m a
walk of 1 500 m in, 1 501 m out beside it); the nearest stop by hand: a lone stop at 1 501 m
offered in 1 125.75 s, of two beyond the limit the nearer by whole walk (2 130 m, not 2 180 m,
gaps counted), a tie to the lower stop id, a destination on a one-way walk (1 570 m, not the
1 060 m of the other way), an end with no walk to any stop left empty; networkx's unbounded
Dijkstra for the 26 of the 50 ends of the networkx fixture with no stop within 600 m; the
refusals, each naming its row. The demand's determinism over 1 and 8 threads and two runs is
tested with the whole chain (`crates/loop/tests/determinism.rs`: every end and both lists, with
entries within `stop_walk_max` and nearest stops beyond it). The run time on a 20 000-node grid with 300 stops and 13 000
trips is reported by hand: `cargo test --release -p openmobisim-demand --test run_time --
--ignored --nocapture`.
