# Networks

% src: crates/core-graph/src/{network.rs,defaults.rs,network_options.rs,layers.rs,turns.rs}; crates/io-osm/src/{import.rs,tags.rs,region.rs}; crates/py-bindings/src/network.rs network_read_osm; python/openmobisim/{network.py,audit.py}

A **network** is what travellers move on. openmobisim keeps one road network, on which cars and buses are loaded and
congest, and two **layers** built over the same streets, on which bicycles and pedestrians move at fixed speeds: the
**bike layer** and the **walk layer**. Scheduled public transport is a fourth layer, read from a timetable (see
[Transit](transit.md)); it joins the others at its stops. This page explains how a network is built, from
OpenStreetMap or from a table of links, and what every link of it carries.

## The four layers

% src: crates/core-graph/src/layers.rs Layer, StaticLayer, StaticNetwork; crates/core-graph/src/network.rs RoadNetwork

| Layer | Who uses it | How its times are set | Built from |
|---|---|---|---|
| road | cars and buses | by the loading: queues form behind capacity (see [The loading](loading.md)) | the road classes that carry motor traffic, and busways for buses |
| bike | cyclists | fixed: a speed per link, by its cycling infrastructure | the same ways, by the cycling rules below |
| walk | pedestrians, and every walk to or from a stop or a parking | fixed: walking speed | the same ways, by the walking rules below |
| transit | riders of scheduled services | the timetable, with buses delayed by road traffic | a GTFS feed ([Transit](transit.md)) |

A layer with fixed times is called a **static layer**: nothing on it congests, so a route found once stays the best
route all morning. The bike and walk layers are their own directed graphs over the road network's nodes (the same
OpenStreetMap node ids, in the same projection), so a place where layers meet, such as a stop or a parking, joins them
by node.

## Reading OpenStreetMap

% src: crates/io-osm/src/import.rs import_detailed (steps 1–6); tags.rs classify, direction, lanes, maxspeed, node_signal, node_splits_way

{py:func}`openmobisim.network_read_osm` reads an `.osm.pbf` extract in six steps:

1. **Classify the ways.** A way with a `highway` value the table knows becomes a road of that **road class**
   (`motorway`, `primary`, `residential`, …; the list is on the [parameters page](../reference/parameters.md)).
   `highway=road` counts as `unclassified`; `footway`, `steps`, `path`, `track` and `corridor` count as `footway`;
   `busway` and `bus_guideway` as `busway`. A road closed to other motor traffic (`access=no` or `private`, or the
   most specific of `motorcar`, `motor_vehicle`, `vehicle` being `no` or `private`) but open to buses (`bus` or `psv`
   = `yes` or `designated`) is a **busway**: buses only; closed and not open to buses, it is no road link (it keeps
   its bike and walk links by their own rules). `access=destination|delivery|customers` stay drivable (the
   restriction is on through traffic, which routing does not represent). A `highway=service` way that is a parking
   aisle, a driveway, a drive-through or an emergency access (`service=…`) is no road link either; alleys and untyped
   service roads are. Ways that are areas (`area=yes`) or have fewer than two nodes are left out. Each kind of
   exclusion is counted (`osm_access_denied`, `osm_service_not_a_street`).
2. **Read the nodes the kept ways use**, and which of them carry a traffic signal, a stop sign, a mini-roundabout or a
   barrier.
3. **Split each way at its junctions** (nodes two or more kept ways share, the ends of every way, and the nodes of
   step 2), so that a link runs from one junction to the next. A link's length is measured along its geometry on the
   ellipsoid, not as a straight line.
4. **Keep the largest strongly connected part** of the network a car may use (`connectivity="strong"`, the default),
   so that every node a car can use reaches every other. Whole links are removed and none is changed; footways and
   other links a car may not use are untouched. `connectivity="keep"` keeps everything.
   {py:meth}`openmobisim.Network.report_import` counts what was removed.
5. **Contract**: merge chains of links joined at nodes that are not junctions, when the links agree on class, lanes
   and speed and the node has no signal, stop sign or barrier. With `contract_drivable=True` (the default), car links
   are also merged across nodes that only footways, cycleways or pedestrian streets touch.
6. **Build the bike and walk layers** from the same ways (`layers=True`, the default), each with its own junctions,
   connectivity trim and contraction.

Nothing in the data stops an import: an unreadable speed limit, a node the extract lacks, a one-node way each has a
fallback, and each is counted in the network's diagnostics.

**Direction.** A way is one-way when tagged `oneway=yes|true|1` (or `-1`/`reverse` against its node order), when it is
a roundabout (`junction=roundabout|circular`), or when it is a motorway or motorway slip road; `oneway=no` overrides
the last two. A road whose direction changes over the day (`oneway=reversible|alternating`) is read as two-way.

**Lanes.** `lanes:forward` and `lanes:backward` are read where tagged. Otherwise a `lanes` total goes all one way on a
one-way road and is halved on a two-way road, rounded down with at least one lane. Where nothing is tagged, the road
class's lanes apply.

**Speed limits.** `maxspeed` is read as km/h, or converted from `mph` or `knots`; `walk` is read as 7 km/h; a list
takes its first value. The usual implicit codes are read with their country ignored: `XX:urban` 50, `XX:rural` 80,
`XX:motorway` 130, `XX:trunk` 100, `XX:living_street` 15, `XX:zone30` 30 and `XX:zone20` 20 km/h (`DE:motorway` has
no limit: the class's speed applies). These are one table for every country, uncalibrated (the OpenStreetMap wiki's
*Default speed limits*); where a country differs, give the way a number. Any other value (`XX:signals`, free text) or
`none` falls back to the road class's speed, and is counted.

## What a road link carries

% src: crates/core-graph/src/defaults.rs DefaultRow, default_row, LinkParameters::from_defaults, WAVE_SPEED_BOUNDS_KM_H; network.rs RoadNetworkBuilder::build_with

Every road link has a **fundamental diagram**: the relation, in traffic flow theory, between how densely vehicles
occupy a road and how many pass a point per hour. openmobisim uses the triangular one: flow rises at the
**free-flow speed** $v$ until the **capacity** $q$ (the most vehicles a link passes per hour), then falls to zero at
the **jam density** $k_j$ (the most vehicles a link holds per kilometre, standing still), along a line whose slope is
the **backward wave speed** $w$, the speed at which a queue's back moves upstream {footcite}`newell1993b`. Its four
quantities are tied by one identity, so only three can be chosen:

$$
q = \frac{v\, w\, k_j}{v + w}
$$

The code chooses $v$, $q$ and $k_j$ and derives $w$, the least directly observed of the four. For a link of class
$c$ with $n$ lanes in its direction:

$$
v = v_{\text{tag}} \cdot f_v, \qquad
q = s_c \cdot n \cdot f_q, \qquad
k_j = k_{j,c} \cdot n \cdot f_k, \qquad
w = \frac{q}{k_j - q/v}
$$

where $v_{\text{tag}}$ is the tagged speed limit or else the class's free-flow speed, $s_c$ the class's
**saturation flow** per lane (the rate at which a lane discharges a standing queue), $k_{j,c}$ its jam density per
lane, and $f_v$, $f_q$, $f_k$ the global multipliers `free_flow_speed_factor`, `capacity_factor` and
`jam_density_factor` (1 by default). A table of links that states a capacity uses it in place of $s_c \cdot n$.

One guard keeps the triangle valid. The capacity and the free-flow speed are held (a table, a tag or a factor means
them); if the $w$ they imply falls outside 6 to 30 km/h (a capacity too high for the jam density, the free-flow branch
reaching capacity only at or past the jam density so that no triangle fits, or a capacity too low), $w$ is held at
the bound and $k_j$ derived again as $q/w + q/v$, so the storage is raised above the bound and lowered below it. The
link is recorded in the diagnostics either way (`jam_density_raised_to_fit_capacity`, `wave_speed_clamped`). No
shipped class row reaches the guard; a table that states a capacity its class row's jam density cannot carry at its
speed does (Sioux Falls, whose free-flow speeds are low: 20 links, their storage raised by 1 to 19 %), and
{py:func}`openmobisim.network_read_table` warns when the table gave no lanes or the raise passes half.

% src: residential row 30 km/h, 1 400 veh/h/lane, 140 veh/km/lane (defaults.rs default_row)

For example, a one-lane residential street with no speed tag has $v$ = 30 km/h, $q$ = 1 400 veh/h and
$k_j$ = 140 veh/km, so $w = 1400 / (140 - 46.7) \approx 15$ km/h.

A link's **free-flow time** is its length over $v$, plus a signal's control delay where it ends at one (below); its
**storage**, the vehicles it holds standing, is $k_j$ times its length. Links shorter than the shortest length the
builder accepts are lengthened to it and recorded (`degenerate_link_length`). The road classes' rows are on the
[parameters page](../reference/parameters.md) as `<class>.free_flow_km_h`, `<class>.lanes`,
`<class>.saturation_flow_veh_h_lane` and `<class>.jam_density_veh_km_lane`. Pedestrian, footway, cycleway and ferry
classes carry a residential row that the loading never reads: they are never on the road network.

## Signals

% src: crates/io-osm/src/tags.rs node_signal, SignalDirection; import.rs signal_stops, step 5 (signalised per link); defaults.rs SignalDefaults (merge_m), uniform_delay, SaturationBands; network.rs build_with, mark_signal_location; turns.rs TurnTable::build

OpenStreetMap gives where signals stand, never their timings. A node tagged `highway=traffic_signals` is a signal,
unless it is an emergency signal (`traffic_signals=emergency`). It governs some of the links that **end** at it, its
*approaches*:

- **Tagged with a direction** (`traffic_signals:direction=forward|backward`): the links travelling that way along the
  way (a stop line's signal on one side of the junction).
- **At a junction** (a node where three or more streets meet): every approach, except one that comes from a signal
  that already stopped its traffic within `signal_merge_m` (25 m): the junction's own stop line, or another signalised
  junction just before it. A crossing tagged twice delays once.
- **On a street, near a junction** (a *stop line*: a junction within `signal_merge_m` along the street, the nearest one
  if there are two): only the traffic heading into that junction. A car that has just crossed the junction passes the
  stop line of the opposite approach freely, though the signal's tag names no direction. Mappers in the Netherlands
  tag nearly every signal this way.
- **On a street, far from any junction** (a mid-block crossing): both directions.

`signal_merge_m` 0 restores the plain rule, every approach of every direction-less signal. Each signalised approach
gets three things:

1. **A control delay**, added to its free-flow time: Webster's uniform delay {footcite}`webster1958`, a cycle average
   that does not respond to the traffic of the run,

   $$
   d = f_d \cdot \frac{0.5\, C\, (1 - g)^2}{1 - \min(1, x)\, g}
   $$

   with cycle length $C$ (`signal_cycle_s`, 90 s), green fraction $g$ (`signal_green_fraction`, 0.48, times
   `green_fraction_factor`), the degree of saturation $x$ it is evaluated at (`signal_degree_of_saturation`, 0.85)
   and $f_d$ the `control_delay_factor`. At the shipped values $d \approx 20.6$ s.
2. **A discharge share**: the turns out of a signalised approach pass at most the fraction $g$ of its capacity
   ([The loading](loading.md)).
3. **A lower saturation flow on at-grade roads.** At a signal, a lane of a road other than a motorway, a trunk road or
   their slip roads discharges at most the saturation flow of its speed band, per hour of green:

   | Speed limit (km/h) | Parameter | Shipped (veh/h per lane) |
   |---|---|---|
   | 30 or less | `street_30.saturation_flow_veh_h_lane` | 1 000 |
   | above 30, at most 50 | `street_50.saturation_flow_veh_h_lane` | 1 200 |
   | above 50, at most 70 | `street_70.saturation_flow_veh_h_lane` | 1 500 |

   These are measured values for signalised junctions in France {footcite}`cerema2015`, lower than the class rows, which
   are values for a lane at its best. Above 70 km/h, and on links that do not end at a signal, the class row applies;
   a link whose capacity a table states is not capped. The cap applies before `capacity_factor`.

## Turns

% src: crates/core-graph/src/turns.rs

A **turn** is a pair of links, in and out, at a node; the loading moves vehicles turn by turn. Every pair is a turn,
except **U-turns, which exist only where the approach has no other exit** (a dead end). OpenStreetMap's turn
restrictions are **not read**: every other movement is allowed.

## Clipping a study area

% src: crates/io-osm/src/region.rs (Region, ClipOptions, TURNAROUND_MAX_M, turnarounds); crates/py-bindings/src/network.rs network_read_osm

`region=` cuts a study area from a larger extract, as a rectangle `(west, south, east, north)` or a polygon of
`(lon, lat)` points. A node inside is kept. A road leaving the area ends at the edge; a road that leaves and comes back
is two roads, never bridged across what lies outside. Three options keep more:

| Option | Default | What it keeps |
|---|---|---|
| `region_stubs` | on | each road's first node outside, so the segment crossing the edge stays |
| `region_buffer_m` | 2 000 m | motorways, trunk roads and their slip roads within this distance of the area, so a ring road just outside and the main approaches stay |
| turnarounds | with a buffer | where a one-way motorway or trunk carriageway is cut at the buffer's edge, a short one-way link at 30 km/h joins it to the nearest cut end of the opposite carriageway within 300 m (one with the same `ref` first), so neither carriageway ends in a dead end |

The turnarounds are the only links a clip adds; they are tagged `openmobisim:turnaround=yes`. Without them, the
strong-connectivity step would remove a cut carriageway back to its last interchange.

## The bike and walk layers

% src: crates/io-osm/src/tags.rs bike_way, walk_way, is_ferry, duration_seconds; crates/core-graph/src/layers.rs StaticLayerDefaults, BikeInfrastructure, BikeCost, StaticNetwork::link_costs, derive

**Who may cycle where.** Every road a car may use, other than a trunk road, is a bike link, in **mixed traffic**
unless its tags give it more. Trunk roads and their links carry bikes only with `bicycle=yes|designated|permissive`
(in the Netherlands and on a French *voie express* they are closed to bikes by law and the tag is often missing; a
trunk road that is open needs the tag). Never: motorways and their slip roads, `motorroad=yes`, `bicycle=no` or
`bicycle=use_sidepath`, areas, and roads closed by `access=no|private` unless `bicycle=yes|designated|permissive`.
Footways, paths, tracks, steps and pedestrian streets only with `bicycle=yes|permissive|designated|dismount`; busways
only with such a `bicycle` tag. Ferries (`route=ferry`) carry bikes unless `bicycle=no`.

**Infrastructure** is read in three levels:

| Level | Tags | Speed (km/h) |
|---|---|---|
| separated | `highway=cycleway`; `cycleway=track`; a footway or path with `bicycle=designated` | `bike_dedicated_km_h`, 18 |
| lane | `cycleway=lane`; a cycle street (`cyclestreet=yes`, `bicycle_road=yes`) | `bike_dedicated_km_h`, 18 |
| mixed | everything else | `bike_mixed_km_h`, 15 |

`cycleway:both`, `:right` and `:left` are read per side: on a two-way street the right side serves the way's own
direction and the left side the reverse, which assumes right-hand traffic. A bike's speed is never above the way's
speed limit, and is walking speed where it must be pushed (`bicycle=dismount`, steps). A bike follows the cars'
one-way rules unless the contraflow is opened: by `oneway:bicycle=no`, by `opposite…` in `cycleway` or
`cycleway:both|left|right`, or by a lane or track whose `cycleway:left:oneway` / `cycleway:right:oneway` runs against
the street. On a one-way street with the contraflow open, the left lane or track (the right one on `oneway=-1`)
serves the contraflow and the other side the cars' direction; with it closed, a lane on either side serves the one
direction.

**Who may walk where.** Every way but motorways and their slip roads, `motorroad=yes`, `foot=no` or
`foot=use_sidepath`, areas, and roads closed by `access` unless `foot` permits it; cycleways too, unless `foot=no`;
busways only with a permitting `foot` tag. Every walk link is walked both ways at `walk_km_h` (4.8 km/h).

**Ferries** are on the bike and walk layers only. A crossing takes its `duration` tag where given, else its length at
`ferry_km_h` (10 km/h), plus an expected wait of `ferry_wait_s` (300 s); the two are folded into the link's speed.

**A network with no tags** (a link table, the example grids) gets its layers derived from the road links: bikes on
every road link but motorways, footways, pedestrian streets and busways, in the road's direction, separated on a
cycleway and mixed elsewhere; pedestrians both ways on every link but motorways and cycleways.

## Other sources of a network

% src: python/openmobisim/network.py network_read_table, network_edit; gmns.py network_read_gmns

The same link model serves {py:func}`openmobisim.network_read_table` (a table of links, which may state each link's
capacity and speed), {py:func}`openmobisim.network_read_gmns` (a GMNS folder) and the example networks. See
[Bring your own network and demand](../guides/own_data.md). {py:func}`openmobisim.network_edit` makes a changed copy
of a network for a scenario ([Scenarios and edits](../guides/scenarios.md)).

## Defaults and options

% src: crates/core-graph/src/network_options.rs FIELDS, CLASS_FIELDS; python/openmobisim/network.py network_options

Every number a network is built with is a **network option**, named and overridable when the network is read:

```python
net = openmobisim.network_read_osm("city.osm.pbf", network_options={"residential.free_flow_km_h": 25})
```

{py:func}`openmobisim.network_options` lists them with their shipped values (or a network's own values). They are
the road-class rows (`<class>.free_flow_km_h`, `.lanes`, `.saturation_flow_veh_h_lane`, `.jam_density_veh_km_lane`),
the five global multipliers, the three signal settings, the three saturation bands, and the bike and walk layers'
speeds and costs. An unknown name is refused with the list of known ones. Each is described, with its source, on the
[parameters page](../reference/parameters.md). Every value is an uncalibrated default. The built network keeps the
values it was built with, so everything derived later uses them, and they are part of its fingerprint.

## Checking a network

% src: python/openmobisim/audit.py network_check

{py:func}`openmobisim.network_check` checks a network before a run. Each check passes, warns (worth a look on the
map) or fails (trips will fail):

| Layer | Checks |
|---|---|
| road (links a car may use) | strongly connected; no self-loops; every length and free-flow time above 0; under 0.5 % of links more than 1 % shorter than the straight line between their ends; under 3 % of links shorter than 5 m; under 1 % parallel links; each class's median free-flow speed in a plausible range (a mix-up of mph and km/h shows here); under 2 % of links outside 500–2 400 PCU/h per lane; no more than 6 lanes per direction; under 25 % dead ends; under 10 % of the signals on a two-way street within `signal_merge_m` of a junction stopping both directions (a stop line stops the traffic into its junction; many that stop both are a direction ignored by a reader or the data), with the signals per drivable kilometre |
| bike, walk | connected; no self-loops; lengths and times above 0; lengths against the straight line; a plausible median speed (bike 8–30 km/h, walk 2.5–7 km/h) |

It returns the worst status as its verdict, a row per check with the rule it applied, and each layer's size; for the
bike layer, the shares of its length that are separated or a painted lane.

## For bikes

% src: crates/core-graph/src/layers.rs BikeCost, StaticNetwork::link_costs, with_infrastructure

The bike layer is read from the same ways as the roads, with cycling's own access, direction and infrastructure rules,
so a cycle track, a contraflow lane or a bike-only bridge is in it even where no car may go. A cyclist's route is the
least-cost route on it under one of two costs, chosen by `Scenario(bike_cost=…)`:

$$
\text{cost}_\ell = \frac{L_\ell}{u_\ell} \times
\begin{cases}
m & \text{in mixed traffic}\\
a & \text{on a painted lane}\\
1 & \text{on a separated track, or a ferry}
\end{cases}
$$

for `"dedicated"` (the default), where $L_\ell$ is the link's length, $u_\ell$ its bike speed, $m$ the
`bike_mixed_cost_factor` (1.2) and $a$ the `bike_lane_cost_factor` (1); `"time"` is the time alone. So by default a route all on separated tracks is preferred to one all in mixed traffic as long as it takes less than 1.2 times as long; tracks and lanes are also faster (18 against 15 km/h). Cycling
infrastructure can be changed for a scenario ({py:func}`openmobisim.network_edit`): a changed link rides at its new
level's speed. Bikes do not congest, and do not slow cars: the bike layer has no capacity.

## Limits

- OpenStreetMap's turn restrictions are not applied; every movement but a U-turn away from a dead end is allowed.
- Signal timings are not read: every signal has the same cycle, green share and a delay that does not respond to the
  traffic of the run.
- A signalised mid-block crossing (no junction within `signal_merge_m`) delays both directions like a junction's
  approach: in the starter cities, 13–51 crossings each.
- Which junction a stop-line signal belongs to is read from distances along the street (`signal_merge_m`), not from
  the tags of the junction's approaches: a stop line between two junctions closer than that serves the nearer one.
- Lanes per movement (`turn:lanes`) are not read.
- A changing speed limit along a way takes its first value; implicit limits are one table for every country.
- Sides of a street (`cycleway:right|left`) assume right-hand traffic.
- Reversible lanes are read as two-way.
- Bikes and pedestrians do not congest, and do not interact with cars.
- Every default is uncalibrated; each one's source is on the [parameters page](../reference/parameters.md).

## References

```{footbibliography}
```
