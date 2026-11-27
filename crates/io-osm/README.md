# openmobisim-io-osm

Reads one OpenStreetMap extract, or the part of it a region holds, into the one network of
`openmobisim-graph`, as release 0.1's reader did: three layers of one mode each, the car links, the
bike links and the walk links, each built from the ways its own rules admit, split at its own
junctions, trimmed to its largest strongly connected component and contracted. A car link carries
its lanes, free-flow speed and capacity, from its tags and its road class. Depends on
`openmobisim-types`, `openmobisim-graph` and `osmpbf` (pure Rust, no C); the experimental trips
(`trips::draw`) also on `openmobisim-demand`, for the trip row they are given as, and `libm`.

## Inputs
- `read(path, values, region)`: one `.osm.pbf` file, the graph's three build values
  (`capacity_per_lane`, `bike_speed`, `walk_speed`; `PARAMETERS.md`), always the caller's, and an
  optional `Region`: `Region::bbox(west, south, east, north)` or `Region::polygon(points)`, WGS84
  degrees (`None`: the whole extract).
- `network(&osm, values, region)`: the same from an `Osm` held in memory (nodes: id, longitude and
  latitude in WGS84 degrees, tags; ways: id, node ids in order, tags). `read_pbf(path)` fills an
  `Osm` with the ways tagged `highway` or `route=ferry` and the nodes they name. No options: one
  behaviour, release 0.1's Python default (`contract`, `connectivity="strong"`,
  `contract_drivable`, `layers`, `region_stubs`, `region_buffer_m=2000`).
- **Experimental**, to be refined: `network_for(&osm, values, region, car_roads)`, the same with
  the roads cars drive chosen. `CarRoads::All` is `network` itself. `CarRoads::Main` gives cars
  the main roads alone, motorways down to `unclassified`, and no `residential`,
  `living_street` or `service` road: a lighter car network whose main roads run past the side
  streets as one link. A roundabout tagged as one of those three classes
  (`junction=roundabout|circular`, a road, not closed to cars) is driven as the largest main
  class meeting it, with that class's defaults, so that it does not cut the roads through it; a
  ring drawn as several ways is one ring. Every road still counts in the car links' junctions
  before the merging; bikes and walkers are read the same either way. The report holds
  `car_roads`.
- **Experimental**: `trips::draw(&network, trips, start, with_transit, seed)`, a morning's trips
  drawn from the streets of a network read here, as the demand crate's `TripRow`s. Homes by
  the length of residential and living streets per cell of 500 m, activities by the length of
  the other streets (primary to tertiary and their links, unclassified, service, pedestrian);
  the activity cell by activities × exp(−distance / 4 km); four classes at 40, 20, 20 and 20 %
  (`car_and_bike`, `car_only`, `bike_only`, `no_vehicle`); 5 % of the trips cars of the class
  `through` between two cut ends of motorways or trunk roads (the turnarounds) at least 1 km
  apart, by capacity; departures evenly over one hour. The numbers are constants of
  `src/trips.rs`, none calibrated: a start for trying ideas, not a model of the city. The same
  network, count and seed give the same trips (`Stream::Population`).

## Outputs
The `Network`, and a `Report`: `nodes`, `links` (of the three layers), `links_unreachable` (links
dropped outside their layer's strong component, counted before contraction), `loops_dropped`
(pieces that start and end at one node), `ways_cut` (street ways holding some of their nodes and
missing others: cut by the edge of the extract or of the region, or at a node not placeable or a
segment whose geodesic does not converge), `repairs` (below, "Repairs"; the graph builder's among
them) and `region`. Every link has exactly one mode. Node external ids are OSM node ids, shared by
the layers where their links meet; a link's is `MODE:WAY:PIECE:f` (or `:b`), its first piece's:
`MODE` `car`, `bike` or `walk`, `WAY` the way, `PIECE` its number along the way on that layer
(from 0), `f` along the way's node order, `b` against it. For drawing only, each link's shape (the
positions of the nodes between its two ends, in its own direction) and its road class (below);
neither is in the fingerprint.

## Units
Metres (lengths), km/h (speeds; `mph` × 1.609 344, `knots` × 1.852), lanes as whole numbers,
vehicles per hour (capacities).

## Meanings and assumptions
- **The road classes** (release 0.1's): the `highway` values `motorway`, `motorway_link`, `trunk`,
  `trunk_link`, `primary`, `primary_link`, `secondary`, `secondary_link`, `tertiary`,
  `tertiary_link`, `unclassified` (and `road`), `residential`, `living_street`, `service`,
  `pedestrian`, `footway` (and `steps`, `path`, `track`, `corridor`; `bridleway` for bikes and
  walkers), `cycleway`, `busway` (and `bus_guideway`); and `ferry` (`route=ferry`). A link's road
  class is its way's. Other `highway` values (`construction`, `proposed`, `platform`, …) are not
  read. An `area=yes` way and a way of fewer than two nodes are no street.
- **Car ways** (release 0.1's `classify`): a car class, not closed by `access=no|private`, nor by
  the most specific of `motorcar`, `motor_vehicle`, `vehicle` it carries being `no|private`
  (`motor_vehicle=yes` reopens `vehicle=no`), nor a `service` way tagged
  `service=parking_aisle|driveway|drive-through|emergency_access`. `access=destination`,
  `delivery`, `customers` stay drivable (a restriction on through traffic, not modelled).
- **Bike ways** (release 0.1's `bike_way`): never motorways and their slip roads,
  `bicycle=no|use_sidepath`, `motorroad=yes`, `access=no|private` without
  `bicycle=yes|designated|permissive`. Trunk roads, their slip roads and busways only with
  `bicycle=yes|designated|permissive`; footways (paths, tracks, steps, bridleways among them) and
  pedestrian streets only with `bicycle=yes|designated|permissive|dismount`; a ferry unless
  `bicycle=no`; every other class (cycleways, car streets, parking aisles). Only `bicycle` is
  read for bikes (not `vehicle`).
- **Walk ways** (release 0.1's `walk_way`): every class but motorways and their slip roads, unless
  `foot=no|use_sidepath`, `motorroad=yes`, or `access=no|private` without
  `foot=yes|designated|permissive`; a busway only with `foot=yes|designated|permissive`; a
  cycleway too (a walker may use it where no footway runs beside it); a ferry unless `foot=no`.
- **Direction** (wiki, `Key:oneway`): vehicles follow `oneway` (`yes`, `true`, `1`; `-1`,
  `reverse`; `no`, `false`, `0`), else one-way along the node order on `junction=roundabout` or
  `circular` and on motorways and their slip roads, else both ways; `reversible` and
  `alternating` are read as no `oneway`. Bikes follow `oneway:bicycle` (`no`: both ways; `yes`:
  the street's one way; `-1`: against the node order), else both ways on a one-way street whose
  contraflow a cycleway tag opens (an `opposite…` value of `cycleway`, `cycleway:both|left|right`,
  or a lane, track or shared bus lane on a side whose `cycleway:<side>:oneway` runs against the
  street), else the street's direction. Walkers go both ways (a ferry: its own direction).
- **Lanes** (wiki, `Key:lanes`), per car direction: one-way, `lanes:<side>` else `lanes`;
  two-way, a tagged side is its own; total and one side: the other is total − given −
  `lanes:both_ways`; total alone: (total − `lanes:both_ways`) / 2 rounded down, at least 1; no
  tag: the class's. A count that is not a whole number is read as absent (`tags_unread`); lane
  tags that contradict are repaired (`lanes_repaired`): a total that is not the sum of the sides
  tagged, the sides as tagged; a total and one side leaving the other none, 1; a
  `lanes:both_ways` above the total (or equal to it: `lanes=0` on a two-way road), 1 each way; a
  direction cars drive given 0 lanes, 1.
- **Free-flow speed** (wiki, `Key:maxspeed`): `maxspeed:forward` (or `:backward`), then
  `maxspeed`, then the class's. A value is read as release 0.1 read it: a number, alone or with
  `km/h`, `kmh`, `kph`, `mph` or `knots`; `walk` (7 km/h); an implicit code `XX:urban` (50),
  `rural` (80), `motorway` (130), `trunk` (100), `living_street` (15), `zone30` (30), `zone20`
  (20), the country ignored (the wiki's "Default speed limits"); of a `;` list, the first. `none`,
  `DE:motorway` (no limit) and anything else are read as absent, noted (`speeds_defaulted`, once
  per way cars drive): the next key, else the class's speed.
- **Capacity**: lanes × the class's capacity per lane (the table below, junctions counted in):
  the flow a link's downstream end passes. `capacity_per_lane` (`PARAMETERS.md`) is the graph's value for a car row
  without a capacity, so it applies to a link table; every car link read here has its own.
- **Junctions**, per layer: a node its ways use twice or more (the car layer counts every road of
  `classify`, footways and cycleways among them), a node that splits every way (a signal, `stop`,
  `mini_roundabout`, any `barrier`), and the ends of each run of held nodes. A link runs from one
  junction to the next along one way; its **length** is the sum of the WGS84 geodesics between its
  nodes (Vincenty 1975, iterated to 1e-14 rad). A piece from a node to itself is dropped
  (`loops_dropped`); a piece of 0 m is left out (`links_of_no_length`).
- **The strong component**, per layer (Tarjan 1972; most nodes, the first found on a tie): a link
  whose two ends are not both in it is dropped (`links_unreachable`). Each layer has its own: a
  street a car cannot leave stays for walkers.
- **Contraction**, per layer (release 0.1's): a node is merged away when it splits no way and has
  one link in and one out, or two in and two out that are one street's two directions, the links
  merged having the same road class and car fields (lanes, speed, capacity), and the merge making
  no link from a node to itself. Lengths add, shapes join, the link keeps its first piece's id.
  The car layer is contracted as if no footway or cycleway were there. (Release 0.1 paired two links
  in and two out by the order it met them; where a one-way street met a two-way one it could merge
  the node away and lose the one-way street's continuation. Here such a node stays.)
- **Signals.** A node tagged `highway=traffic_signals` (not `traffic_signals=emergency`) is
  signalised; a link whose downstream node is signalised has the downstream-end flag. Stored,
  applied nowhere.
- **The region** (release 0.1's clip, its one behaviour): the nodes inside the box (edges included)
  or polygon (even–odd rule on the plane of degrees); on every way but a main road, the first node
  outside either end of each run inside (a stub: the segment crossing the edge stays); on
  motorways, trunk roads and their slip roads, every node within 2 000 m of the region; a way
  leaving and coming back is two pieces, nothing bridged. A one-way main-road carriageway cut at
  the buffer's edge that no kept road leaves (or enters) is joined to the nearest cut end of the
  other kind within 300 m (one with the same `ref` first) by a turnaround: a one-way
  `motorway_link` (a `trunk_link` between trunk roads), `maxspeed=30`, way id from
  −4 611 686 018 427 387 904 up. Without it the strong component took both carriageways back to
  their last interchange (Delft's A13 and A4).

## The class table (a default: a row of `PARAMETERS.md`, `osm_class_defaults`)
Free-flow speed and lanes per direction where no tag gives them: release 0.1's table ("common OSM
speed conventions"). Capacity of one lane, the flow a link's downstream end passes, **with the
class's junctions counted in**, as the reader models no junction (no signal, no saturation band):

- Roads without at-grade junctions: a motorway 2 000 veh/h per lane (ICTAVRU, the French
  instruction for urban expressways, about 2 000 uvp/h per lane); a trunk road 1 800, the design
  capacity of a carriageway lane; a motorway's or trunk road's ramp 1 550 (Cerema, *Fondamentaux
  de la conception routière*, rapport de synthèse).
- Roads with at-grade junctions: a signalised approach lane's capacity, Ca = Qs × V / Ty (Cerema,
  *Évaluation de la capacité aux feux, retour d'observations en France*, October 2016), with Qs
  the saturation flow, 1 800 uvp/h per lane, and V / Ty the green time over the cycle. **The green
  share of each class is this project's assumption, not the guideline's** (the guideline gives the
  formula, not a table by class).

Read by `tests/tags.rs` (`the_class_table_reads_as_its_readme_states`).

| `highway` | car km/h | car lanes per direction | green share | car veh/h per lane |
|---|---|---|---|---|
| motorway | 110 | 2 | no junction | 2 000 |
| motorway_link | 60 | 1 | ramp | 1 550 |
| trunk | 90 | 2 | no junction | 1 800 |
| trunk_link | 50 | 1 | ramp | 1 550 |
| primary | 60 | 2 | 0.60 | 1 080 |
| primary_link | 40 | 1 | 0.50 | 900 |
| secondary | 50 | 1 | 0.50 | 900 |
| secondary_link | 40 | 1 | 0.50 | 900 |
| tertiary | 50 | 1 | 0.45 | 810 |
| tertiary_link | 30 | 1 | 0.40 | 720 |
| unclassified, road | 40 | 1 | 0.40 | 720 |
| residential | 30 | 1 | 1/3 | 600 |
| living_street | 20 | 1 | 0.25 | 450 |
| service | 20 | 1 | 0.25 | 450 |
| pedestrian, footway, cycleway, busway, ferry | — | — | — | — |

## Errors
`OsmError`, each naming what it refuses, only where no network can be read: the file cannot be
read (`Io`) or decoded (`Decode`); no street of any mode (`NoStreets`); the graph's builder refuses
(`Build`: no node left, a build value not a finite number above zero). A region that is no area
(`RegionError`: not finite, out of range, west not west of east, fewer than three vertices) is
refused where it is made, `Region::bbox` or `Region::polygon`.

## Repairs
Everything else is set right and noted in `Report::repairs`, one kind per rule, with its count
and the first five ways (`way N`) or links; the Python binding warns once per kind and the run's
record writes the counts (`[network.repairs]`). A way's tags are counted only when the way gives a
piece (two nodes in a row held): a way outside the region, or the extract, is not counted:

| Key | What | What is done |
|---|---|---|
| `ways_repeated` | a way id given twice | the first kept |
| `tags_unread` | a car way's `oneway` or lane value of no known meaning | read as absent |
| `speeds_defaulted` | a way cars drive with a `maxspeed`, `maxspeed:forward` or `:backward` that is not a speed (counted once per way) | read as absent: the next key, else the class's speed |
| `lanes_repaired` | lane tags that contradict or leave a driven direction no lane | above, "Lanes" |
| `links_of_no_length` (the graph's key) | a piece of 0 m (two nodes at one position), each of its links on each layer | left out before the strong components are taken, so it joins nothing; no length is invented |

and the graph builder's other kinds (`crates/graph/README.md`), none of which an extract read
here should meet.

## Trimmed from release 0.1
Parkings; bus lanes (release 0.1 took them from the cars' lanes) and busways for buses (it
contracted cars and buses together, and kept roads closed to cars but open to buses); signal
timing, defaults and saturation bands (the class table's capacities count junctions in instead;
release 0.1 capped a signalised approach's lane at 1 000,
1 200 or 1 500 veh/h up to 30, 50, 70 km/h); the bike layer's infrastructure levels, dismounting
and speed limit (a bike rides at `bike_speed` on every bike link here); roundabout flags; ferries'
durations; turn restrictions; every option and feature flag; release 0.1's diagnostics (this crate
counts its repairs).

## Bikes
A bike has links of its own: a bike link runs where the bike rules above let a bike ride, in the
directions they give; a cycleway is a bike link and a walk link, no car link. A bike rides at
`bike_speed` on every bike link. The bike layer has its own junctions, its own strong component
(a one-way street a car cannot use may stay for bikes, joined by a cycleway) and its own
contraction (merged where the road class is the same). Delft's region: 20 664 bike links,
1 240.497 km. `CarRoads::Main` leaves
the bike links as they are, and the drawn trips give 60 % of the travellers a bike. Tested in `tests/tags.rs` (`what_every_rule_means_for_a_bike`, the bike rules case
by case), `tests/connectivity.rs` and `tests/delft_tables.rs`.

## Tests
`cargo test -p openmobisim-io-osm`. Each expected value comes from outside the crate:
- The two experimental parts by hand: `tests/main_roads.rs` (`CarRoads::Main`: a main road
  past a side street is one car link while bikes and walkers split there; a roundabout tagged
  residential between secondary roads is driven as secondary; a ring of two ways is raised
  whole, whichever way meets the larger road; one no main road meets is not driven) and
  `tests/trips.rs` (trips start on the residential street and end on the tertiary road of a
  two-street extract; ids, the departure hour, the classes' shares within four standard
  deviations, the modes with and without a timetable, the same seed the same trips; no street
  of the classes, no trips).
- The class table against its sources (the capacities as the table above states them, end to
  end), tags by hand from the wiki pages named above and release 0.1's rules (`tests/tags.rs`, release 0.1's
  tag and bike and walk rule tests, those of kept rules): the road class and refusals, ways closed
  to cars, service roads, directions, lanes (the fixed bug: `lanes=4`, `lanes:forward=1` gives
  backward 3), speeds (the fixed bug: `maxspeed:forward=30`, `:backward=70` give 30 and 70 km/h;
  30 mph = 48.280 32 km/h; implicit codes), nodes that split, every bike and walk rule, the class
  table end to end, the repairs (an unreadable tag read as absent and noted; `lanes=0` on a
  two-way road, 1 each way, repaired once).
- One hand extract end to end (`tests/repairs.rs`): a way given twice, an unreadable `maxspeed`,
  lane and one-way values, contradicting lanes, a link of 0 m and a cut way, each counted; an
  island joined only by a piece of 0 m dropped on each layer; a segment whose geodesic does not
  converge cuts its way.
- Splitting, signals, cut ways, closed ways, contraction (release 0.1's: two ways that are one street
  merged, not across a change, a signal or into a loop; a chain collapsed, the length conserved; a
  footway crossing a street splits the walk layer only; a one-way street meeting a two-way one
  keeps its node), the same network whatever the input order (`tests/import.rs`); each layer's
  strong component (`tests/connectivity.rs`); the region by hand (`tests/region.rs`: a box and a
  concave polygon, the distance to them, stubs, a road leaving and coming back, the main roads'
  buffer, the turnaround of a cut dual carriageway, a one-way avenue cut short, a way with an
  unreadable `maxspeed` outside the region not counted, one inside counted).
- Drawings (`tests/drawing.rs`): the zigzag of the length test gives its three inner nodes,
  projected, as the `f` link's shape and in reverse as the `b` link's, its length pyproj's; the
  class by release 0.1's names, the fingerprint the same for two classes.
- Length: a five-node zigzag against pyproj 3.8.0 (`Geod(ellps="WGS84").line_length`,
  542.861 119 846 913 m, within 1 µm) on each layer; single geodesics from 1.3 km to 5 853 km.
- Delft (`tests/fixtures/delft/network.osm.pbf`, 17 976 211 bytes, © OpenStreetMap
  contributors, ODbL 1.0), release only (ignored by `cargo test`; CI runs both):
  - `tests/delft.rs`: read in full (under 3 s in release; 1.2 s measured); 20 ways' modes, lanes
    and speeds against `fixtures/delft/ways.csv` (libosmium and pyproj, `tests/delft_ways.py`,
    its rules release 0.1's), the lengths where a way's links hold nothing else (18 of 40
    directions), a mode missing only where its layer dropped the whole neighbourhood.
  - `tests/delft_tables.rs`: the region (4.315, 51.965, 4.410, 52.035) against release 0.1's own
    output, `crates/binding/tests/fixtures/delft-tables/`, link by link (by the OSM nodes each
    link passes): lanes and speeds as they are, a capacity from release 0.1's class table scaled by
    the ratio of the two tables' capacities per lane; every difference counted and traced to a
    trimmed feature or a fixed bug (its header lists them), none unexplained: a lane difference
    only where release 0.1's rule for that cause, read from the way's tags, gives both counts
    exactly; a capacity lowered at a signal only where the link's downstream node is tagged
    `highway=traffic_signals`.
  `cargo test --release -p openmobisim-io-osm --test delft --test delft_tables -- --ignored --nocapture`.
