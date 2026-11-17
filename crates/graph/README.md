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
- Drawings (`NetworkBuilder::add_drawing`, at most one per declared link, for drawing only): a
  `LinkDrawing` of the link's shape, the points between its two ends in its order of travel, of
  the nodes' kind (longitude/latitude projected as the nodes are, or metres), and its road class
  (a name, or `None`).
- Three build values (`BuildValues`), always given by the caller, never defaulted here:
  `capacity_per_lane` (veh/h per lane), `bike_speed` and `walk_speed` (km/h). See `PARAMETERS.md`.

## Outputs
`Network`: counts and external-id tables of nodes and links; per node its position in metres
and signalised flag; per link its two nodes, length, free-flow speed and time per mode,
capacity, lanes, modes and signal flag; out- and in-links per node; the projection, if any;
`bike_speed()` and `walk_speed()`, the build values in m/s, the same on every link;
`fingerprint()`, below; for drawing only, per link `shape()` (its points between the ends, in
metres; empty: straight) and `road_class()`. No crate that runs anything reads these two; they
are not in the fingerprint, and a link's length is the row's (or the straight line between its
ends), never measured along its shape here (`crates/io-osm` sums the geodesics along the same
points itself).

## Fingerprint
`Network::fingerprint()`, a `u64` (the FNV-1a hash of `openmobisim-types`), over the network as
built, in this order: the projection (0 for none, else 1, the UTM zone number and the
hemisphere, 0 north and 1 south); the bike and walk speeds in m/s; the node count, then per
node in id order its external id, its x and y in metres (projected, for longitude/latitude) and
its signal mark; the link count, then per link in id order its external id, its upstream and
downstream node ids, its length in metres (given, or the straight line), its lanes, its modes,
its car free-flow speed in m/s (`NaN` without cars), its capacity in veh/s (given, or lanes ×
`capacity_per_lane`; infinite without cars) and its downstream-end signal mark. Nothing derived
from these (free-flow times, out- and in-links) is fed, nor any link's drawing (shape, road
class). The same rows, in any order, give the
same fingerprint; a change to any value of a node or link row or to a build value changes it,
except a `capacity_per_lane` no car link reads (every car link gives its capacity) and a length
or capacity given equal to the one it would take.

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
`build` refuses only what no network can be built from, with a `BuildError`: no node left; a mix
of position kinds (longitude/latitude and metres: no projection converts them); a build value
that is not a finite number above zero.

## Repairs
Every other row is used as given, set right or left out, and noted in `Network::repairs` by
kind, with its count and the first five ids (`openmobisim_types::repairs`); rows are sorted by
id stably, so "the first" is the first given. Not in the fingerprint.

| Key | Rows | What is done |
|---|---|---|
| `nodes_repeated` | a node id given twice | the first usable row kept (one that can be placed); the others left out, each noted once |
| `nodes_unplaced` | a node whose position is not finite, or in longitude/latitude not projectable (beyond ±180°, ±84°) | left out, and every link naming it (`links_to_no_node`) |
| `signals_unplaced` | a signal on a node not held | ignored |
| `links_repeated` | a link id given twice | the first usable row kept (one no fault below leaves out), with its own drawing; the others left out, each noted once (of an id with no usable row, the first row's fault, the others as repeated) |
| `links_to_no_node` | a link naming a node not held | left out |
| `links_transit_removed` | a link allowing transit | transit taken out (it runs on its timetable) |
| `car_mode_removed` | a car link without lanes, with 0 lanes, or without a car speed that is a finite number above zero | the car mode taken off, no speed guessed; kept for its other modes |
| `car_fields_ignored` | a link without cars giving lanes, a car speed or a capacity | the field ignored |
| `links_without_mode` | a link left with no mode | left out |
| `lengths_defaulted` | a length given that is not finite or not above zero | the straight line between its nodes |
| `links_of_no_length` | a link of 0 m (a node to itself, or two nodes at one position, without a length) | left out; no length is invented (`LINKS_OF_NO_LENGTH`, the key the OSM reader also uses) |
| `capacities_defaulted` | a car link's capacity that is NaN or not above zero | lanes × `capacity_per_lane` |
| `drawings_unplaced` | a drawing given before any row of its link | ignored |
| `drawings_repeated` | a second drawing for one link row | ignored, the first kept |
| `shapes_unread` | a shape with a point of the other kind, not finite or not projectable | drawn straight, its class kept (`SHAPES_UNREAD`, the key and phrase a table reader also uses for a shape text it cannot read) |

A drawing belongs to the link row last added with its id (a row is added before its drawing), so the row kept of a repeated id keeps its own drawing and the drawings of the rows left out go with them.

For bikes: a bike link that is also a car link without lanes or speed stays a bike link
(`car_mode_removed`); a link left out is lost to bikes too, and counted.

## Bikes
A bike is a mode on the same links as cars and walkers, with its own free-flow speed
(`bike_speed`, on every link); there is one network and no separate layer. A link a bike may
use has bike in its mode set; a cycle path is a link whose set is bike (and walk) alone, given
with no car field: no lanes, no car speed, no capacity limit. A bike rides 1 500 m at 15 km/h
in 360 s, on a street or a cycle path, signal or not. In the fingerprint a link's bike
permission is in its modes and the bike speed is fed once: a link opened to or closed to bikes,
or another `bike_speed`, changes the fingerprint.

## Tests
`cargo test -p openmobisim-graph`. Each expected value comes from outside the crate: hand
free-flow times (1 500 m at 15 km/h by bike 360 s, 1 000 m at 4.8 km/h on foot 750 s, 2 000 m
at 50 km/h by car 144 s, unchanged by a signal; a cycle path with no car fields, 360 s by bike,
infinite capacity, 0 lanes; the bike and walk speeds read back, 36 km/h as 10 m/s and 7.2 km/h
as 2 m/s); hand capacities (1 200 veh/h is 1/3 veh/s; 2 lanes × 1 800 veh/h
is 1 veh/s; a car row's infinity kept as an unlimited capacity); Pythagoras (3 m and 4 m apart is 5 m); a hand count of a 3 × 3
grid (9 nodes, 24 links, out-degrees 2, 3, 4); the standard library's sort for the id order;
the UTM grid's definition and the WGS84 meridian and parallel arcs for the projection. Drawings
(`tests/drawing.rs`): a shape and a class kept as given; a link without one straight and
unclassed; the length (1 000 m, not the 1 166.2 m along the bend) and the fingerprint unchanged
by a shape and a class, on a street and a cycle path; a point in longitude/latitude projected as
a node at that point; each repair counted and naming its link. The fingerprint as a property (`tests/fingerprint.rs`): two builds of the same rows equal; one
value changed, all else equal, in each of the 18 input columns (node id, x, y, longitude,
latitude, signal mark; link id, upstream, downstream, modes, `length_m`, `lanes`,
`car_speed_km_h`, `capacity_veh_h`, `signal_at_downstream_end`; `capacity_per_lane`,
`bike_speed`, `walk_speed`), each a named case, changes it; the same rows in reverse order
give the same fingerprint; two nodes and one link hash to the byte stream above written out by
hand, in metres and in longitude/latitude south of the equator (zone 56, south, by the UTM
grid's definition).
