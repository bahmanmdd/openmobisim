# Networks

% src: crates/graph/README.md; crates/io-osm/README.md; crates/types/README.md; crates/binding/README.md; PARAMETERS.md

A **network** is what travellers move on: a set of **nodes** (points) and **links** (directed connections between
two nodes). openmobisim holds one network for the three modes that travel on links: the car, the bike and walking.
Scheduled public transport does not travel on links; it runs on its timetable (see [Transit](transit.md)) and is
reached on foot over the walk links.

The network is the first of the seven concepts of this part in their reading order: the network, the demand, the
routes, the choice, transit, the loading and the iteration. The table follows the first four:

| Page | What it takes | What it gives to the next |
|---|---|---|
| Networks (this page) | node and link tables, or an OpenStreetMap extract | per link: its two nodes, its length, the modes allowed, a free-flow time per mode and a capacity |
| [Demand](demand.md) | the network, a trips table, a timetable if any | per trip: the node each end is linked to per mode, and the stops in walking reach |
| [Routes and route sets](search.md) | the network, the demand, the current link times | per traveller: a set of alternatives (routes and transit journeys) |
| [Choice](choice.md) | the sets and their costs | per traveller: the alternative taken |

Journeys on the timetable are found beside the routes ([Transit](transit.md)). The alternatives taken are then loaded
on the network ([The loading](loading.md)), which gives new link times, and the search and the choice are repeated
([The iteration](iteration.md)).

```{image} ../_static/figures/network_modes_paper.svg
:alt: One network, modes per link: a street as a car link, a bike link and a walk link between the same two nodes, and a cycle path with a bike link and a walk link only
:class: only-light
:width: 100%
```
```{image} ../_static/figures/network_modes_night.svg
:alt: One network, modes per link: a street as a car link, a bike link and a walk link between the same two nodes, and a cycle path with a bike link and a walk link only
:class: only-dark
:width: 100%
```

## Nodes and links

% src: crates/graph/README.md ("Inputs", "Outputs", "Meanings and assumptions"); crates/types/README.md ("Outputs", "Assumptions")

A **node** has an identifier and a position. The positions of one network are all of one kind: either longitude and
latitude in WGS84 degrees, or metres on a plane. Positions in degrees are projected once to metres, on the UTM zone
(Universal Transverse Mercator) of the centre of the nodes' bounding box; a distance on that plane is within 0.1 % of
the distance on the ground inside the zone.

A **link** runs one way, from its **upstream** node to its **downstream** node. A street open in both directions is
two links. A link holds:

| Field | Unit | Meaning | Read by |
|---|---|---|---|
| upstream node, downstream node | none | where the link starts and ends | the search, the loading |
| modes | none | which of car, bike and walk may use the link | the demand's linking, the search |
| length | m | the distance travelled along the link | the free-flow times, the path size of a route |
| lanes | lanes | the number of car lanes in the link's direction (car links only) | the capacity, when none is given |
| car free-flow speed | km/h | the speed of a car that meets no queue (car links only) | the car's free-flow time |
| capacity | veh/h | the flow of cars the link's downstream end passes (car links only) | [the loading](loading.md) |
| signal at the downstream end | none | a mark, stored and applied nowhere (below) | nothing |
| shape, road class | m, none | for drawing only (below) | the map and the summary |

A link that allows cars must give its lanes and its car speed. A link that does not allow cars gives none of the
three car fields: it has 0 lanes, no car speed and no capacity limit, since nothing is queued on it.

The network is built once and never changed afterwards. Nodes and links are numbered by the sorted order of their
identifiers, so the order of the rows in the input changes nothing.

## One network for car, bike and walk

% src: crates/graph/README.md ("Bikes", "Meanings and assumptions"); crates/types/README.md ("Outputs": mode); crates/io-osm/README.md ("Outputs")

The four modes of travel are car, bike, walk and transit, and none is ranked above another. Car, bike and walk share
the one network: each link carries the set of modes that may use it, and a route by a mode uses only links whose set
holds that mode. There is no separate bike or walk network object.

Two ways of describing the same street are both valid:

- **One link with several modes**, such as `car;bike;walk`: the car, the bike and the walker use the same link, each
  at its own speed. A cycle path is a link `bike;walk` with no car field.
- **One link per mode**: a car link, a bike link and a walk link along the same street, between the same or
  different nodes. A network read from OpenStreetMap is of this kind: every one of its links has exactly one mode
  (below).

The modes do not interact on a link. Only cars queue; a bike and a walker cross a link in the same time whatever the
cars on it do (see [The loading](loading.md)).

## Free-flow time

% src: crates/graph/README.md ("Meanings and assumptions", "Tests"); PARAMETERS.md (bike_speed, walk_speed)

The **free-flow time** of a link for a mode is the time that mode needs to cross it when nothing is in its way:

$$
\tau_{a,m} = \frac{\ell_a}{v_{a,m}}
$$

where $\ell_a$ is the length of link $a$ in metres and $v_{a,m}$ the free-flow speed of mode $m$ on it in metres per
second. For the car, $v$ is the link's own car speed. For the bike and for walking, $v$ is one speed for the whole
network, a parameter given when the network is built:

| Parameter | Default | Unit | What it sets |
|---|---|---|---|
| `bike_speed` | 15 | km/h | the speed of every bike on every bike link |
| `walk_speed` | 4.8 | km/h | the speed of every walker on every walk link, and of every walk between a door and the network |

Nothing else enters a free-flow time: there is no travel-time column, and a signal does not change it. By hand: a
bike rides 1 500 m at 15 km/h in 360 s; a walker covers 1 000 m at 4.8 km/h in 750 s; a car drives 2 000 m at 50 km/h
in 144 s.

For bikes and walkers the free-flow time is the travel time of every run. For cars it is the lower bound: the
loading adds the time spent in a queue.

## Capacity

% src: crates/graph/README.md ("Meanings and assumptions", "Tests"); PARAMETERS.md (capacity_per_lane)

The **capacity** of a car link is the largest flow of cars its downstream end passes, in vehicles per hour. It is the
link's own value when one is given, and otherwise

$$
C_a = n_a \cdot c
$$

where $n_a$ is the number of lanes and $c$ the parameter `capacity_per_lane` (default 1 800 veh/h per lane): two
lanes at 1 800 veh/h per lane pass one vehicle per second. A car link may give an unlimited capacity (infinity); its
downstream end then passes any flow.

A flow counts vehicles: there is no passenger-car unit. A link has no storage, no jam density and no wave speed: how
the capacity turns into queues and delays is the subject of [The loading](loading.md).

## Where a network comes from

A network is built in one of two ways: from two tables, or from an OpenStreetMap extract.

### From tables

% src: crates/binding/README.md ("Inputs": tables, nodes, links; "Outputs": Network.tables)

`Network.from_tables(nodes, links)` reads a nodes table and a links table. A table is a dict of lists, a pandas or
polars frame, a pyarrow table, or the path of a Parquet file. The columns are fixed
([Input tables](../reference/inputs.md) gives every rule):

| Table | Column | Type | Meaning |
|---|---|---|---|
| nodes | `node_id` | text | the node's identifier |
| nodes | `lon_deg`, `lat_deg` | degrees (WGS84) | the position; or |
| nodes | `x_m`, `y_m` | m | the position on a plane |
| links | `link_id` | text | the link's identifier |
| links | `upstream`, `downstream` | text | the identifiers of its two nodes |
| links | `modes` | text | mode names joined by `;`, for example `car;bike;walk` |
| links | `length_m` | m, or empty | the length; empty: the straight line between the two nodes |
| links | `lanes` | whole number, or empty | car lanes; empty on a link without cars |
| links | `car_speed_km_h` | km/h, or empty | the car's free-flow speed; empty on a link without cars |
| links | `capacity_veh_h` | veh/h, or empty | the capacity; empty: lanes × `capacity_per_lane`; `inf`: no limit |
| links | `shape`, `road_class` | text, optional | for drawing only (below) |

The example builds a street and a cycle path between two nodes 1 500 m apart, and reads the network back with
`Network.tables()`, which returns the two tables that `from_tables` reads:

```python
import openmobisim as oms

nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1500.0], "y_m": [0.0, 0.0]}
links = {
    "link_id":        ["street", "street_back", "path", "path_back"],
    "upstream":       ["a", "b", "a", "b"],
    "downstream":     ["b", "a", "b", "a"],
    "modes":          ["car;bike;walk", "car;bike;walk", "bike;walk", "bike;walk"],
    "length_m":       [None, None, 1600.0, 1600.0],
    "lanes":          [2, 2, None, None],
    "car_speed_km_h": [50.0, 50.0, None, None],
    "capacity_veh_h": [None, None, None, None],
}
network = oms.Network.from_tables(nodes, links)
nodes_back, links_back = network.tables()
for name in ("link_id", "modes", "length_m", "lanes", "car_speed_km_h", "capacity_veh_h"):
    print(name, links_back[name])
```

```text
link_id ['path', 'path_back', 'street', 'street_back']
modes ['bike;walk', 'bike;walk', 'car;bike;walk', 'car;bike;walk']
length_m [1600.0, 1600.0, 1500.0, 1500.0]
lanes [None, None, 2, 2]
car_speed_km_h [None, None, 50.0, 50.0]
capacity_veh_h [None, None, 3600.0, 3600.0]
```

The street took the straight line between its nodes as its length (1 500 m) and two lanes at 1 800 veh/h per lane as
its capacity (3 600 veh/h); the cycle path kept the length given and has no car field. The links come back in the
order of their identifiers.

### From an OpenStreetMap extract

% src: crates/io-osm/README.md ("Inputs", "Outputs", "Meanings and assumptions"); crates/binding/README.md ("Inputs": files)

`Network.from_osm(path, region=None)` reads one OpenStreetMap extract (a `.osm.pbf` file), or the part of it inside
a region: a box `(west, south, east, north)` or a polygon of `(lon, lat)` points, in WGS84 degrees. The reader has
one behaviour and no options. It builds three **layers** of links, one per mode, and puts them in the one network:

1. **Which ways each mode may use.** A way is read when its `highway` value is one of the road classes of the table
   below (or it is a ferry route). The car layer takes the ways of a road class open to cars that are not closed to cars
   by their access tags. The bike layer takes every road class except motorways and their slip roads, unless the tags close the way
   to bikes; trunk roads, busways, footways and pedestrian streets only where a `bicycle` tag opens them. The walk
   layer takes every road class except motorways and their slip roads, unless the tags close the way to walkers.
2. **Directions.** Cars follow `oneway` and are one-way on roundabouts and motorways. Bikes follow `oneway:bicycle`
   and the cycleway tags that open a one-way street against its direction, and otherwise the street's direction.
   Walkers go both ways.
3. **Links from junction to junction.** Each layer splits its ways at its own junctions: the nodes its ways use twice
   or more, the nodes that carry a signal, a stop sign, a mini-roundabout or a barrier, and the ends of the ways. A
   link's length is the sum of the geodesics on the WGS84 ellipsoid between its nodes (Vincenty 1975), not a straight
   line.
4. **The largest strongly connected part**, per layer (Tarjan 1972): a link is kept when both its ends are in the
   largest set of nodes that all reach one another by that mode. Each layer has its own, so a street that a car
   cannot leave stays for walkers.
5. **Contraction**, per layer: a node that is no junction and joins two links of the same road class and the same
   car fields is merged away; the lengths add and the shapes join.

Every link of such a network has exactly one mode. The layers share their nodes where their links meet (a node's
identifier is its OpenStreetMap node id), and a link's identifier is `MODE:WAY:PIECE:f` or `MODE:WAY:PIECE:b`: its
mode (`car`, `bike` or `walk`), the id of its way, the number of its piece along the way (from 0), and `f` along the
way's node order or `b` against it.

When a region is given, ordinary streets are cut one node beyond its edge. Motorways, trunk roads and their slip
roads are kept up to 2 000 m outside it, and a one-way carriageway cut there is joined to the opposite carriageway by
a short turnaround link, so that the motorway stays connected in both directions.

**The car fields of a link** come from the way's tags and, where a tag is missing or unreadable, from its road class:
the lanes per direction from `lanes` and its variants, the free-flow speed from `maxspeed` and its variants, and the
capacity always from the road class, as lanes × the road class's capacity per lane.

### The road class defaults

% src: crates/io-osm/README.md ("The class table"); PARAMETERS.md (osm_class_defaults, capacity_per_lane)

| Road class (`highway` value) | Car free-flow speed (km/h) | Car lanes per direction (lanes) | Junctions | Green share (share of the signal cycle) | Capacity (veh/h per lane) |
|---|---|---|---|---|---|
| motorway | 110 | 2 | none at grade | — | 2 000 |
| motorway_link | 60 | 1 | ramp | — | 1 550 |
| trunk | 90 | 2 | none at grade | — | 1 800 |
| trunk_link | 50 | 1 | ramp | — | 1 550 |
| primary | 60 | 2 | at grade | 0.60 | 1 080 |
| primary_link | 40 | 1 | at grade | 0.50 | 900 |
| secondary | 50 | 1 | at grade | 0.50 | 900 |
| secondary_link | 40 | 1 | at grade | 0.50 | 900 |
| tertiary | 50 | 1 | at grade | 0.45 | 810 |
| tertiary_link | 30 | 1 | at grade | 0.40 | 720 |
| unclassified, road | 40 | 1 | at grade | 0.40 | 720 |
| residential | 30 | 1 | at grade | 1/3 | 600 |
| living_street | 20 | 1 | at grade | 0.25 | 450 |
| service | 20 | 1 | at grade | 0.25 | 450 |
| pedestrian, footway, cycleway, busway, ferry | no cars | no cars | — | — | no cars |

The reader models no junction, so the capacity of a lane **counts the road class's junctions in**. For roads without
at-grade junctions it is a carriageway lane's capacity: 2 000 veh/h on a motorway (ICTAVRU), 1 800 veh/h on a trunk
road, 1 550 veh/h on a ramp (Cerema, *Fondamentaux de la conception routière*). For roads with at-grade junctions it
is the capacity of a signalised approach lane,

$$
C = Q_s \cdot g
$$

with $Q_s$ the saturation flow, 1 800 veh/h per lane, and $g$ the green share, the green time over the cycle time
(Cerema 2016). The formula is the guideline's; **the green share given to each road class is an assumption of
this project**, not a value of the guideline. The speeds and lanes are common conventions for OpenStreetMap data, not a
sourced national table. The table is one parameter, `osm_class_defaults`, with one value; it cannot be changed
through `from_osm`. To use other values, read the network back with `Network.tables()`, change the columns and build
it again with `from_tables`.

The parameter `capacity_per_lane` applies to a car row of a link table that gives no capacity. A network read from an
extract gives every car link its road class's capacity, so `capacity_per_lane` changes nothing there.

### A lighter car network (experimental)

% src: crates/io-osm/README.md ("Inputs": network_for); crates/binding/README.md ("Experimental")

`Network.from_osm(path, region=None, car_roads="main")` is an **experimental** helper that may change. It gives cars
the main roads alone, from motorways down to `unclassified`, and no `residential`, `living_street` or `service`
road; a roundabout tagged as one of those three road classes is driven as the largest main road class that meets it. Bikes and
walkers are read as before. The default, `car_roads="all"`, is the reader described above.

## Signals: read and stored, applied nowhere

% src: crates/graph/README.md ("Meanings and assumptions": signals); crates/io-osm/README.md ("Signals"); crates/binding/README.md ("Meanings and assumptions", "Outputs")

The OpenStreetMap reader marks a node tagged `highway=traffic_signals` as signalised, and marks every link whose
downstream node is signalised. The network stores both marks. **Nothing reads them**: no free-flow time, capacity,
route or delay depends on a signal. The effect of junctions on a road's throughput is in the road class table's
capacities instead. A network built from tables has no signal marks, and `Network.tables()` does not write them.

## Shapes and road classes: for drawing only

% src: crates/graph/README.md ("Inputs": drawings, "Outputs"); crates/binding/README.md ("Inputs": links)

A link may carry a **shape**, the points between its two ends in its direction of travel, and a **road class**, a
name such as `motorway`, `residential` or `cycleway`. Both are used by the run's map and summary to draw the link
and nowhere else. A link's length is the value given (or the straight line between its nodes), never measured along
its shape. A network read from OpenStreetMap carries both for every link; in a links table they are the two optional
columns `shape` and `road_class`.

## What is repaired instead of refused

% src: crates/graph/README.md ("Errors", "Repairs"); crates/io-osm/README.md ("Errors", "Repairs"); crates/binding/README.md ("Repairs")

A network is refused only when none can be built: no node is left, the positions mix degrees and metres, or a build
value is not a positive number. Every other faulty row is set right or left out, counted by kind, and shown as one
warning per kind: a link naming a node that does not exist is left out, a car link without lanes or speed loses the
car and keeps its other modes, a length that is not above zero becomes the straight line, a link of 0 m is left out.
The counts are in `Network.repairs` and in the run's record. [Warnings](../reference/warnings.md) lists every kind.

## The Delft case

% src: the published case `delft` (doi:10.5281/zenodo.24585967), its `links.parquet` and `nodes.parquet`, counted with pyarrow

The published case `delft` gives its network as two tables, with one mode per link. It has 21 566 nodes and 88 512
links:

| Mode of the link | Links (count) | Total length (km) |
|---|---|---|
| car | 12 460 | 912.3 |
| bike | 20 652 | 1 240.5 |
| walk | 55 400 | 2 253.2 |

```python
import pyarrow.parquet as pq

links = pq.read_table("delft/links.parquet").to_pydict()
for mode in ("car", "bike", "walk"):
    lengths = [m for m, modes in zip(links["length_m"], links["modes"]) if modes == mode]
    print(mode, len(lengths), round(sum(lengths) / 1000, 1))
```

```text
car 12460 912.3
bike 20652 1240.5
walk 55400 2253.2
```

## For bikes

% src: crates/graph/README.md ("Bikes"); crates/io-osm/README.md ("Bikes", "Meanings and assumptions": bike ways, direction); crates/types/README.md ("Bikes")

- A bike is a mode on the one network, beside the car and the walker. A link that a bike may use has `bike` in its
  modes; a cycle path is a link for bikes (and walkers) with no car field.
- A bike rides every bike link at `bike_speed`, 15 km/h by default: 1 500 m in 360 s, on a street or on a cycle path,
  with or without a signal.
- From OpenStreetMap, bike links follow the bike rules: no motorways, the `bicycle` tags, and contraflow cycling on a
  one-way street where the tags open it. The bike layer has its own junctions and its own strongly connected part: a
  one-way street that a car cannot use may stay for bikes, joined by a cycleway.
- A car link without lanes or speed in a table stays a bike link when its modes hold `bike`.
- With `car_roads="main"` (experimental) the bike links are unchanged.

## Limits

% src: crates/graph/README.md ("Meanings and assumptions"); crates/io-osm/README.md ("Meanings and assumptions", "The class table"); README.md ("Known limits")

- **No junction is modelled.** Signals are stored and not applied; turn restrictions are not read, and a turn or a
  node costs no time. The road class capacities stand in for junctions, and their green shares are assumptions.
- **One bike speed and one walk speed** for the whole network: no level of cycling infrastructure and no speed
  limit changes them, and bikes and walkers never slow one another down.
- **A link has no storage.** A queue does not spill back onto the links upstream (see [The loading](loading.md)).
- **Access restrictions on through traffic are not modelled**: a way tagged `access=destination`, `delivery` or
  `customers` is driven like any other.
- **Transit does not use links.** No bus is delayed by road traffic, and a bus lane is not read.
- **Parkings, ferry durations and roundabout rules are not read** from OpenStreetMap.
- **The road class table is one fixed default**, not a calibrated or national table.

## References

- Cerema (2016). *Évaluation de la capacité aux feux, retour d'observations en France*. Centre d'études et
  d'expertise sur les risques, l'environnement, la mobilité et l'aménagement.
- Cerema. *Fondamentaux de la conception routière*, rapport de synthèse.
- de Hartog, J. J., Boogaard, H., Nijland, H. and Hoek, G. (2010). Do the health benefits of cycling outweigh the
  risks? *Environmental Health Perspectives* 118(8), 1109–1116.
- ICTAVRU: *Instruction sur les conditions techniques d'aménagement des voies rapides urbaines*, the French
  instruction for urban expressways.
- Knoblauch, R. L., Pietrucha, M. T. and Nitzburg, M. (1996). Field studies of pedestrian walking speed and start-up
  time. *Transportation Research Record* 1538, 27–38.
- OpenStreetMap contributors. OpenStreetMap wiki, the pages `Key:oneway`, `Key:lanes`, `Key:maxspeed` and "Default
  speed limits". The data are © OpenStreetMap contributors, Open Database License 1.0.
- Tarjan, R. (1972). Depth-first search and linear graph algorithms. *SIAM Journal on Computing* 1(2), 146–160.
- Transportation Research Board (2022). *Highway Capacity Manual*, 7th edition. Washington, DC.
- Vincenty, T. (1975). Direct and inverse solutions of geodesics on the ellipsoid with application of nested
  equations. *Survey Review* 23(176), 88–93.

The full list of the manual is in the [Bibliography](../reference/bibliography.md).
