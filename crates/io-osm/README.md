# openmobisim-io-osm

Reads one OpenStreetMap extract into the one network of `openmobisim-graph`: every street way
becomes directed links between junction nodes, each with the modes (car, bike, walk) the
wiki's access rules give it in its own direction and, for a car link, its lanes and free-flow
speed. Depends on `openmobisim-types`, `openmobisim-graph` and `osmpbf` (pure Rust, no C).

## Inputs
- `read(path, values)`: one `.osm.pbf` file and the graph's three build values
  (`capacity_per_lane`, `bike_speed`, `walk_speed`; `PARAMETERS.md`), always the caller's.
- `network(&osm, values)`: the same from an `Osm` held in memory (nodes: id, longitude and
  latitude in WGS84 degrees, tags; ways: id, node ids in order, tags). `read_pbf(path)` fills
  an `Osm` with the ways tagged `highway` and the nodes they name. No options.

## Outputs
The `Network`, and a `Report` of counts: `nodes`, `links`, `links_unreachable` (links that
lost their car mode, below), `loops_dropped` (pieces of closed ways that start and end at one node),
`ways_cut` (street ways naming nodes the extract does not hold, whose position cannot be
placed, or with a segment whose geodesic cannot be measured: each run of nodes is read), and
`repairs`, what the reader set right instead of refusing (below, "Repairs"), the graph
builder's among them. Node external ids are OSM node
ids; a link's is `WAY:PIECE:f` along the way's node order or `WAY:PIECE:b` against it, `PIECE`
counting from 0 along the way. For drawing only, each link's drawing: its shape, the positions of
the way's nodes between its two junctions in the link's own direction (the `b` link's reversed),
and its road class, the first `highway` value of its way's row in the class table below
(`road` is `unclassified`); neither is in the fingerprint.

## Units
Metres (lengths), km/h (speeds, as the graph's rows take them; `mph` converted at
1.609 344 km per mile), lanes as whole numbers. Capacity is the graph's: lanes ×
`capacity_per_lane`, the flow a link's downstream end passes; this crate sets none.

## Meanings and assumptions
- **A street** is a way with a `highway` value of the class table below, at least two nodes,
  and no `area=yes`. Other `highway` values (`construction`, `proposed`, `platform`, …) and
  ferries are not read.
- **Links.** A node is a junction when it ends a street way, when street ways pass it twice or
  more, or when it is signalised. A link runs from one junction to the next along one way; the
  nodes between are its shape (for drawing). Its **length** is the sum of the WGS84 geodesics between its
  nodes (Vincenty 1975, iterated to 1e-14 rad). A way used both ways is two links per piece,
  each with its own direction's modes. A way cut by the extract's edge keeps each run of nodes
  the extract holds; nothing is bridged. No merging across ways.
- **Modes** (wiki: "OSM tags for routing/Access restrictions", Netherlands row; `Key:access`):
  the class's default, then the most specific key present decides: car `motorcar` >
  `motor_vehicle` > `vehicle` > `access`; bike `bicycle` > `vehicle` > `access`; walk `foot` >
  `access`. The values:

  | Reads as | Values | From |
  |---|---|---|
  | admits | `yes`, `designated`, `permissive`, `destination`, `customers`, `delivery`, `official`, `discouraged`, `optional_sidepath` | the wiki's access values (`Key:access`); `optional_sidepath` from `bicycle=*` |
  | admits, as `customers` | `visitors` | the owner's ruling (the wiki lists no such value; read as those a place admits) |
  | refuses | `no`, `private`, `permit`, `use_sidepath`, `dismount`, `agricultural`, `forestry` | the wiki's access values (`agricultural` and `forestry`: only those vehicles, none of this reader's modes) |
  | refuses, as `private` (a restricted group of people) | `emergency`, `restricted`, `residents`, `employees`, `staff`, `members`, `military`, `police`, `disabled`, `school`, `students` | the owner's ruling; `emergency` is the wiki's `access=emergency` (emergency vehicles only) |
  | refuses, as `use_sidepath` | `separate`, on `bicycle` and `foot` only | the owner's ruling (the wiki's `separate` says the path is mapped as a way of its own) |
  | "only these": admits the modes named, closes every other | `foot`, `pedestrian` (walkers); `bicycle` (bikes); `motorcar`, `motor_vehicle` (cars); `vehicle` (cars and bikes); `motorcycle`, `moped`, `mofa`, `horse`, `bus`, `psv`, `taxi`, `hgv`, `coach`, `goods`, `hov`, `carpool` (none of this reader's modes) | the owner's ruling: the common reading of `access=<mode>` (the wiki writes it `access=no` with `<mode>=yes`); the groups are the wiki's transport-mode keys |
  | says nothing (the next key decides, else the class's default) | `unknown`; and any other value (`construction`, `service`, `separate` on another key, …), noted `tags_unread` | `unknown`: the wiki; the rest by the rule "only what is known is read" |

  **A `;` list** (`destination;private`) is read part by part, each part trimmed: the groups it
  names are one group (`foot;bicycle`: walkers and bikes); then the most
  restrictive known part decides: any part that refuses refuses; else any part that admits
  admits; else the list says nothing. `motorroad=yes` refuses bikes and walkers by default;
  `service=parking_aisle|driveway|drive-through|emergency_access` refuses cars by default
  (version 1's rule). A class with no car row is never a car link, and its car keys are not
  read. Restrictions to a destination are not modelled.
- **Direction** (wiki, `Key:oneway`): cars follow `oneway` (`yes`/`1`, `-1`, `no`/`0`;
  `reversible` and `alternating` read as both ways; any other value read as absent, noted
  `tags_unread`), else one-way for motorways and
  `junction=roundabout|circular`. Bikes follow `oneway:bicycle`, else the car's rule. Walkers
  go both ways unless `oneway:foot`.
- **Lanes** (wiki, `Key:lanes`), per car direction: one-way, `lanes:<side>` else `lanes`; two-way,
  a tagged side is its own; total and one side: the other is total − given − `lanes:both_ways`;
  total alone: (total − `lanes:both_ways`) / 2 rounded down, at least 1; no tag: the class
  default. A count that is not a whole number is read as absent (`tags_unread`). Lane tags that
  contradict each other are repaired, noted `lanes_repaired`: a total that is not the sum of
  the sides tagged, the sides as tagged; a total and one side leaving the other none, 1; a
  `lanes:both_ways` above the total, 1 each way; a direction cars drive given 0 lanes, 1.
- **Free-flow speed** (wiki, `Key:maxspeed`): `maxspeed:forward` (or `:backward`) first, then
  `maxspeed`, then the class default. Read only for directions cars use. A value that is not a
  number of km/h or of ` mph` (`none`, `walk`, `NL:urban`, `50;30`) is read as absent, noted
  `speeds_defaulted`: the next key, else the class default (no table of country codes).
  `speeds_defaulted` counts ways (not links), once per way whatever how many of its speed keys
  were not read, and only ways cars drive; the speed then taken may be another key's, not the
  class default's. A way cars do not drive has none of its car tags read or noted.
- **Signals.** A node tagged `highway=traffic_signals` is signalised; a link whose downstream
  node is signalised has the downstream-end flag. Stored, applied nowhere.
- **The car network's largest strongly connected component** (Tarjan 1972; most nodes, first
  found on a tie): a link with cars whose ends are not both in it loses its car mode and its
  car fields (lanes, speed) and is counted; it is kept for its bike and walk modes, and dropped
  only when no mode is left. Links without cars are kept as they are. Delft: 2 197 links lose
  their car mode, 1 797 of them are kept for bikes or walkers (1 670 for bikes), 400 car-only
  links are dropped.

## The class table (a default: a row of `PARAMETERS.md`, `osm_class_defaults`)
Car speed (km/h) and lanes per direction when untagged: version 1's table
(`crates/core-graph/src/defaults.rs`, `free_flow_km_h`, `lanes_per_direction`, sourced there as
"common OSM speed conventions"). Modes: the wiki's Netherlands row; `track` is not a car street
(the wiki says yes; the table has no car speed for it). Read by `tests/tags.rs`
(`the_class_table_reads_as_its_readme_states`).

| `highway` | car km/h | car lanes | car | bike | walk |
|---|---|---|---|---|---|
| motorway (one-way) | 110 | 2 | yes | no | no |
| motorway_link (one-way) | 60 | 1 | yes | no | no |
| trunk / trunk_link | 90 / 50 | 2 / 1 | yes | yes | yes |
| primary / primary_link | 60 / 40 | 2 / 1 | yes | yes | yes |
| secondary / secondary_link | 50 / 40 | 1 / 1 | yes | yes | yes |
| tertiary / tertiary_link | 50 / 30 | 1 / 1 | yes | yes | yes |
| unclassified, road | 40 | 1 | yes | yes | yes |
| residential | 30 | 1 | yes | yes | yes |
| living_street | 20 | 1 | yes | yes | yes |
| service | 20 | 1 | yes | yes | yes |
| path, track, cycleway | — | — | no | yes | yes |
| pedestrian, footway, steps, corridor | — | — | no | no | yes |
| busway, bridleway | — | — | no | no | no |

## Errors
`OsmError`, each naming what it refuses, only where no network can be read: the file cannot be
read (`Io`) or decoded (`Decode`); no street at all (`NoStreets`); the graph's builder refuses
(`Build`: a build value not a finite number above zero).

## Repairs
Everything else is set right and noted in `Report::repairs`, one kind per rule, with its count
and the first five ways (`way N`) or links; the Python binding warns once per kind and the
run's record writes the counts (`[network.repairs]`):

| Key | What | What is done |
|---|---|---|
| `ways_repeated` | a way id given twice | the first kept |
| `tags_unread` | an access, one-way or lane value of no known meaning | read as absent |
| `speeds_defaulted` | a way cars drive with a `maxspeed`, `maxspeed:forward` or `:backward` that is not a speed (counted once per way) | read as absent: the next key, else the class default |
| `lanes_repaired` | lane tags that contradict or leave a driven direction no lane | above, "Lanes" |
| `links_of_no_length` (the graph's key) | a piece of 0 m (two nodes at one position), both its links | left out here, before the car network's strong component is taken, so it joins nothing; no length is invented |

and the graph builder's other kinds (`crates/graph/README.md`), none of which an extract read
here should meet.

## Bikes
A bike is a mode on the same links as cars and walkers. A link carries bike when the class
lets bikes through (streets, trunk roads, cycleways, paths, tracks; not motorways, footways,
pedestrian streets, steps) and no more specific key refuses it (`bicycle=no|use_sidepath|
dismount`, `vehicle=no`, `access=no` without `bicycle=yes`, `motorroad=yes`); `bicycle=yes`
opens a footway. A bike follows `oneway` unless `oneway:bicycle` says otherwise (a one-way
street with `oneway:bicycle=no` is a bike-and-walk link against the cars). A cycleway is a bike
and walk link with no car field. Car keys, lanes and speeds bind no bike: a bike rides at
`bike_speed` on every link. A street outside the car network's strongly connected component
loses its cars and stays a bike and walk link (Delft: 1 670 such links, 71.7 km, kept for
bikes). Delft has 80 504 links a bike may ride (3 337.6 km), 34 848 of them without cars.
Tested in `tests/tags.rs` (`what_every_rule_means_for_a_bike`) and `tests/connectivity.rs`.

## Tests
`cargo test -p openmobisim-io-osm`. Each expected value comes from outside the crate:
- Tags by hand from the wiki pages named above: modes per class and per key, directions,
  lanes (the fixed bug: `lanes=4`, `lanes:forward=1` gives backward 3), speeds (the fixed bug:
  `maxspeed:forward=30`, `:backward=70` give 30 and 70 km/h; 30 mph = 48.280 32 km/h), the
  repairs (an unreadable tag read as absent and noted, naming the way: residential 30 km/h and
  1 lane where its `maxspeed` or `lanes` cannot be read; the `;` lists; the restricted groups
  closed; footways and cycleways with car keys not noted). One hand extract end to end
  (`tests/repairs.rs`): a way given twice, an unreadable `maxspeed`, an unknown access and
  one-way value, contradicting lanes, a link of 0 m and a cut way, each counted once; an island
  joined to a ring only by a piece of 0 m loses its cars (its 4 links counted in
  `links_unreachable`, the piece's 2 in `links_of_no_length`); a segment whose geodesic does not
  converge (0°, 0° to 179.5°E, 0.5°N) cuts its way. Mode and group values by hand
  (`a_mode_or_vehicle_group_admits_it_alone`): each row of the table above.
- Drawings (`tests/drawing.rs`): the zigzag of the length test gives its three inner nodes,
  projected, as the `f` link's shape and in reverse as the `b` link's, its length pyproj's as
  before; one bend on the way back; the class by the way's row (`road` as `unclassified`), the
  fingerprint the same for two classes.
- Splitting, signals, cut ways, closed ways and the strong component (a spur and an island lose
  their cars and keep bikes and walkers), by drawing; the same network whatever the input order.
- Length: a five-node zigzag against pyproj 3.8.0 (`Geod(ellps="WGS84").line_length`,
  542.861 119 846 913 m, within 1 µm); single geodesics from 1.3 km to 5 853 km against pyproj.
- Delft (`tests/fixtures/delft/network.osm.pbf`, 17 976 211 bytes, © OpenStreetMap
  contributors, ODbL 1.0) read in full; 20 ways' modes, lanes, speeds and lengths against
  `fixtures/delft/ways.csv`, written once by `tests/delft_ways.py` from libosmium (pyosmium
  4.3.2) and pyproj; the read time printed, under 3 s in release. Release only (about 7 s in
  debug, skipped by `cargo test`; CI runs it):
  `cargo test --release -p openmobisim-io-osm --test delft -- --ignored --nocapture`.
