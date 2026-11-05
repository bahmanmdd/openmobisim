# openmobisim-graph

The one network every later module reads: built once from node and link rows, then immutable,
held as a structure of arrays. Depends on `openmobisim-types` only.

## Inputs
- Nodes (`NetworkBuilder::add_node`): an external id and a position, either longitude/latitude
  (WGS84 degrees) or metres on a plane, all nodes of one kind; optionally marked signalised.
- Links (`NetworkBuilder::add_link`): an external id, the upstream node, the downstream node and
  a `LinkRow`: the modes allowed (any of car, bike, walk), an optional length in metres, whether
  a signal stands at the downstream end, and three car fields: lanes, the car free-flow speed in
  km/h and the capacity in vehicles per hour. With car in the modes, lanes and speed are
  required and the capacity is optional; without car, all three are absent. Two-way: two links.
- Three build values (`BuildValues`), always given by the caller, never defaulted here:
  `capacity_per_lane` (veh/h per lane), `bike_speed` and `walk_speed` (km/h). See `PARAMETERS.md`.

## Outputs
`Network`: counts and external-id tables of nodes and links; per node its position in metres
and signalised flag; per link its two nodes, length, free-flow speed and time per mode,
capacity, lanes, modes and signal flag; out- and in-links per node; the projection, if any.

## Units
Metres, m/s, seconds and veh/s inside; km/h and veh/h only in the rows and build values.

## Meanings and assumptions
- Free-flow time of a link for a mode = length / that mode's free-flow speed, nothing else. A
  signal never changes it. There is no travel-time column.
- Capacity = the row's value when given, else lanes × `capacity_per_lane`: the flow the
  link's downstream end passes. A car row may give an unlimited capacity as infinity; its
  downstream end then passes any flow. A link without cars has 0 lanes, infinite capacity
  (nothing is loaded there) and no car speed. No storage, jam density, passenger-car unit or wave speed.
- Signals (a node's flag, a link's downstream-end flag) are stored and applied nowhere.
- Length absent = the straight line between the two nodes in the plane. For longitude/latitude
  that plane is the UTM zone of the nodes' bounding-box centre (Krüger series, WGS84; the
  Norway and Svalbard exceptions ignored), within 0.1 % of the ground distance in the zone.
- Ids follow sorted external id, whatever the input order; out- and in-links in link-id order.

## Errors
`build` refuses, never repairs, with a `BuildError` naming the node or link: no nodes; a mix of
position kinds; a position that cannot be projected or is not finite; a duplicate node or link
id; a signal on an undeclared node; a link to an undeclared node; a link with no mode or with
transit; a car link without lanes or car speed, or with zero lanes; a link without cars that
gives any car field; a car speed or length (given or straight line) that is not a finite
number above zero; a capacity that is not above zero (infinity is allowed); a build value that
is not a finite number above zero. The first fault in sorted order is named.

## Bikes
A bike is a mode on the same links as cars and walkers, with its own free-flow speed
(`bike_speed`, on every link); there is one network and no separate layer. A link a bike may
use has bike in its mode set; a cycle path is a link whose set is bike (and walk) alone, given
with no car field: no lanes, no car speed, no capacity limit. A bike rides 1 500 m at 15 km/h
in 360 s, on a street or a cycle path, signal or not.

## Tests
`cargo test -p openmobisim-graph`. Each expected value comes from outside the crate: hand
free-flow times (1 500 m at 15 km/h by bike 360 s, 1 000 m at 4.8 km/h on foot 750 s, 2 000 m
at 50 km/h by car 144 s, unchanged by a signal; a cycle path with no car fields, 360 s by bike,
infinite capacity, 0 lanes); hand capacities (1 200 veh/h is 1/3 veh/s; 2 lanes × 1 800 veh/h
is 1 veh/s; a car row's infinity kept as an unlimited capacity); Pythagoras (3 m and 4 m apart is 5 m); a hand count of a 3 × 3
grid (9 nodes, 24 links, out-degrees 2, 3, 4); the standard library's sort for the id order;
the UTM grid's definition and the WGS84 meridian and parallel arcs for the projection.
