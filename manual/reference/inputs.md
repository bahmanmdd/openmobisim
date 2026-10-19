# Input files

Every file openmobisim reads, with the columns its readers take. A column not listed is ignored. Names of columns are
matched without regard to case unless a section says otherwise. Coordinates are WGS84 longitude and latitude in
degrees; times of day are seconds after midnight; every other unit is stated in the table.

All tables on this page share one layout: **Column**, **Type**, **Unit**, **Required** (yes, or no), **Default** (what
an absent column or an empty cell means) and **Meaning**.

**Example files.** [openmobisim_example_inputs.zip](../_static/examples/openmobisim_example_inputs.zip) holds a small
file in each format below, made from the package's toy network and read back by its reader: a GMNS network,
trips, traveller classes, zones and an origin–destination matrix. Its `README.txt` gives the line that reads each.

## Demand

### Trips: `trips.parquet`, `trips.csv` or rows in memory

% src: crates/core-demand/src/trips.rs read_trips_parquet; crates/core-demand/src/columns.rs;
% python/openmobisim/demand.py demand_read_trips, TRIP_COLUMNS; python/openmobisim/scenario.py _trip_row, from_parts

One row per trip. {py:meth}`openmobisim.Scenario.from_parts` takes the path of a `trips.parquet` file, the path of a
`trips.csv` file (read by {py:func}`openmobisim.demand_read_trips`), or the rows themselves as tuples in the column order
below (nine fields, or ten with the mode). In a Parquet file the column names must be exactly these (lower case).

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `traveller_id` | string or integer | — | yes | — | Who travels. An integer and its text are the same id. |
| `trip_seq` | whole number ≥ 0 | — | yes | — | The trip's place in its traveller's day. |
| `origin_lon` | float | degrees | yes | — | Where the trip starts. |
| `origin_lat` | float | degrees | yes | — | |
| `destination_lon` | float | degrees | yes | — | Where it ends. |
| `destination_lat` | float | degrees | yes | — | |
| `departure_time_s` | whole number ≥ 0 | s after midnight | yes | — | When it leaves. A trip departing outside the simulation window is left out of the run, with a `WindowWarning`. |
| `user_class` | string or integer | — | yes | — | The traveller class (see the class table). |
| `weight` | whole number ≥ 1 | people | no | the scenario's `default_weight` | How many people the traveller stands for. A property of the traveller: the first row's value is kept. |
| `mode` | string | — | no | none stated | `car`, `bike`, `walk`, `transit`, `car_transit` (park-and-ride) or `bike_transit` (bike-and-ride); any other value is refused. A trip without one is a car trip, or chooses its mode when the scenario gives `modes`. |

% src: crates/core-demand/src/travellers.rs codes

**A traveller's day is a chain.** Trips are ordered by `trip_seq`; where trip *n* + 1's stated origin differs from trip
*n*'s destination, the chain wins (the origin is replaced) and the run records `trip_chain_origin_overridden` in
`diagnostics.parquet`. Two rows with the same `trip_seq` keep the first (`duplicate_trip_seq`); rows that disagree on
`weight` or `user_class` keep the first (`inconsistent_traveller_weight`, `inconsistent_user_class`). A required column
with an empty value (null) in a Parquet file is refused.

{py:func}`openmobisim.demand_write_trips` writes rows as a `trips.csv` with exactly these columns, an empty cell for a
missing weight or mode.

### Persons: `persons.parquet` or rows in memory (optional)

% src: crates/core-demand/src/persons.rs read_persons_parquet; crates/py-bindings/src/pipeline.rs PersonRow

One row per traveller whose ownership or class differs from their class's. `Scenario.from_parts(persons=...)` takes the
file's path or rows as tuples `(traveller_id, owns_car, owns_bike, has_transit_pass, user_class)`, `None` for "no
override".

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `traveller_id` | string or integer | — | yes | — | Must name a traveller of the trips; a row that names none is ignored (`person_unknown_traveller`). |
| `owns_car` | boolean | — | no | the class's | Whether the traveller has a car (needed for `car` and `car_transit` trips). |
| `owns_bike` | boolean | — | no | the class's | Whether the traveller has a bike (needed for `bike` and `bike_transit` trips). |
| `has_transit_pass` | boolean | — | no | the class's | Whether the traveller holds a transit pass. |
| `user_class` | string | — | no | the trips' | Replaces the class the trips state. |

A second row for the same traveller is dropped (`duplicate_person_row`); each traveller overridden is counted
(`ownership_overridden`).

### Zones: `zones.csv`

% src: python/openmobisim/demand.py demand_read_zones

Read by {py:func}`openmobisim.demand_read_zones`, for turning an OD matrix into trips.

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `zone_id` (or `zone`, `id`) | string | — | yes | — | The zone; an id listed twice is refused. |
| `x_coord` (or `lon`, `longitude`) | float | degrees | yes | — | The zone's point (GMNS's names first). |
| `y_coord` (or `lat`, `latitude`) | float | degrees | yes | — | |

### Zone points: `zone_points.csv` (optional)

% src: python/openmobisim/demand.py demand_read_zone_points, demand_from_od (zone_points)

Read by {py:func}`openmobisim.demand_read_zone_points`: where within a zone its trips start and end, so that trips are
placed where the buildings are (homes as origins, workplaces as destinations) instead of around the zone's point.
Given to {py:func}`openmobisim.demand_from_od` as `zone_points`, a zone that has points draws each trip end among them
by weight; a trip within such a zone ends at a point other than its origin's when the zone has more than one; a zone
without points keeps the rule of `spread_m` and `points_per_zone`. A starter case names the table in its `case.json`.

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `zone_id` (or `zone`, `id`) | string | — | yes | — | A zone of the zones table; several rows per zone. |
| `x_coord` (or `lon`, `longitude`) | float | degrees | yes | — | The point. |
| `y_coord` (or `lat`, `latitude`) | float | degrees | yes | — | |
| `weight_origin` (or `weight`) | float ≥ 0 | — | no | 1 | How likely a trip from the zone starts here, relative to the zone's other points (for example its residents). |
| `weight_destination` (or `weight`) | float ≥ 0 | — | no | 1 | How likely a trip to the zone ends here (for example its workplaces). A zone whose weights are all 0 draws uniformly. |

### OD matrix: `od.csv`

% src: python/openmobisim/demand.py demand_read_od, demand_from_od

Read by {py:func}`openmobisim.demand_read_od`; {py:func}`openmobisim.demand_from_od` turns it into trips (whole trips per
cell, one traveller per trip, departures drawn in the window, trip ends spread around the zone's point).

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `origin_zone` (or `origin`, `o`) | string | — | yes | — | A zone of the zones table. |
| `destination_zone` (or `destination`, `d`) | string | — | yes | — | |
| `trips` (or `demand`, `volume`) | float ≥ 0 | people | yes | — | How many trips; may be fractional. |
| `start_s` | whole number | s after midnight | no | `demand_from_od`'s `start_s` (07:00) | The departure window's start. |
| `end_s` | whole number | s after midnight | no | `demand_from_od`'s `end_s` (09:00) | Its end. |
| `user_class` | string | — | no | `demand_from_od`'s `user_class` (`default`) | The class of these trips. |
| `mode` | string | — | no | none stated | The mode, as in the trips table. |

### Traveller classes: `classes.csv` or a dict

% src: python/openmobisim/demand.py demand_read_classes, _class_entry, CLASS_KEYS, CLASS_LIMITS

Read by {py:func}`openmobisim.demand_read_classes`, or given to `Scenario(classes=...)` as `{class: {column: value}}`.
One row per class; a class must not repeat. A column not in this table and not a `beta_*` coefficient is refused, with
the list.

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `class` (or `user_class`, `name`) | string | — | yes | — | The class, as the trips' `user_class`. |
| `share` | float ≥ 0 | share of people | no | none | The share of the people in the class, read only by {py:func}`openmobisim.demand_assign_classes` (the shares are scaled to sum to one). |
| `modes` | list, `;`-separated in a file | — | no | every mode the run offers | The modes its travellers may use, among `car`, `bike`, `walk`, `transit`, `car_transit`, `bike_transit`. When the classes name their modes and the scenario gives no `modes`, the run offers every mode a class names. |
| `owns_car` | `1`/`0` (also `true`/`false`, `yes`/`no`) | — | no | as the modes imply | A car, unless a persons row says otherwise. Implied when the class may use `car` or `car_transit`, or names no modes. |
| `owns_bike` | `1`/`0` | — | no | as the modes imply | A bike; implied by `bike` or `bike_transit`, or no modes named. |
| `has_transit_pass` | `1`/`0` | — | no | `0` | A transit pass: a holder pays no fare. Never implied: a pass is money, not access (who may take transit is said by `modes`). |
| `beta_<attribute>` | float | per unit of the attribute | no | the choice model's | The class's own coefficient for the built-in logit and nested logit (for example `beta_mode_bike`, a mode constant, or `beta_time_min`); it replaces the run's by name for this class. See the [parameters](parameters.md). |
| `walk_max_s` | float > 0 | s | no | `mode_options` `walk_max_s` | The longest walk offered to the class's trips choosing their mode; `inf` offers every walk. |
| `bike_max_s` | float > 0 | s | no | `mode_options` `bike_max_s` | The longest bike ride offered. |
| `access_walk_max_s` | finite float > 0 | s | no | `transit_options` `access_walk_max_s` | The longest walk to or from a stop. |

A class not listed (all of them, without a class table) may use every mode, with the model's coefficients, and owns a
car and a bike, and holds no transit pass, unless a persons row says otherwise.

## Parkings: a parking table

% src: python/openmobisim/parking.py parking_read_table, PARKING_COLUMNS

Read by {py:func}`openmobisim.parking_read_table`: a CSV or TSV file (the delimiter found from the header), its text,
or rows as dicts. One row per parking. A run snaps each parking to its vehicle's layer and to the walk layer and keeps
it only if a stop is within `parking_options` `walk_max_s` on foot.

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `parking_id` | string | — | yes | — | Unique in the table. |
| `lon` | float | degrees | yes | — | Where it is. |
| `lat` | float | degrees | yes | — | |
| `vehicle` | string | — | yes | — | `car` or `bike`. |
| `capacity` | whole number ≥ 0 | vehicles | yes | — | How many vehicles it holds. A full parking refuses nobody; parking there takes longer. |
| `name` | string | — | no | none | A name, for figures and tables. |
| `hub_id` | string | — | no | its own hub | Rows sharing one join one hub (a station's car park and its bike parking). |
| `initial_occupancy` | whole number ≥ 0 | vehicles | no | 0 | Vehicles parked when the day starts. |
| `fee_eur` | float ≥ 0 | € per stay | no | `price_options` `parking_car_eur` or `parking_bike_eur` | What a stay costs, seen by choice models as `cost_parking_eur`. |

{py:func}`openmobisim.parking_read_osm` reads parkings from OpenStreetMap instead (see below).

## Networks

### A link table and a node table

% src: python/openmobisim/network.py network_read_table, _LINK_ALIASES, _NODE_ALIASES, _parse_delimited_text

Read by {py:func}`openmobisim.network_read_table`: a CSV or TSV file, a TNTP-style file (a `~`-headed, `;`-terminated
table, its `<…>` metadata lines skipped), its text, or rows as dicts. The first alias present in a row wins. One row is
one directed link: a two-way street is two rows. Fields may not contain the delimiter (no quoting). A value not given
comes from the road class's row of the [parameters](parameters.md) (`network` group), as an untagged OpenStreetMap way's
would.

**Links:**

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `from` (`init_node`, `from_node`, `source`, `tail`) | string | — | yes | — | The link's start node. |
| `to` (`term_node`, `to_node`, `target`, `head`) | string | — | yes | — | Its end node. |
| `id` (`link_id`) | string | — | no | `L0`, `L1`, … in row order | The link's id. |
| `length` (`length_m`, `dist`, `distance`) | float | `length_unit`: m (default), km or mi | no | the straight line between its nodes | Its length. |
| `lanes` | whole number | lanes | no | the class's | Lanes in this direction. |
| `free_flow_speed` (`free_flow_speed_km_h`) | float | `speed_unit`: km/h (default) or mi/h | no | from `free_flow_time`, else the class's | Its free-flow speed. A column named `speed` is deliberately not read. |
| `free_flow_time` (`fftt`, `free_flow_time_min`) | float | `time_unit`: s, min (default) or h | no | — | Read only without a speed, and only with a length: the speed is length over time. |
| `capacity` (`cap`, `capacity_veh_h`) | float | PCU/h, the whole link | no | the class's per lane × lanes | Its capacity; a link whose capacity is given is not capped at signals. |
| `class` (`type`, `link_type`, `highway`, `road_class`) | string | — | no | `unclassified`, with a diagnostic | A road class name (`primary`, `residential`, …, as `openmobisim.LINK_CLASSES` lists them); a number such as TNTP's `link_type` is not one. |
| `roundabout` | boolean (`1`, `true`, `yes`, `y`) | — | no | false | Part of a roundabout's circulating carriageway: entering traffic gives way. |
| `signalised` (`signal`) | boolean | — | no | false | A signal at the link's end that stops this link alone (one direction's stop line). |

**Nodes** (optional):

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `id` (`node`, `node_id`) | string | — | yes | — | The node, as the links name it; a row without one is skipped. |
| `x` (`lon`, `longitude`) | float | degrees, or the table's own unit with `coordinates="xy"` | no | 0 | Its position. With `"xy"` the positions are scaled by `coordinate_scale_m` and placed on a schematic layout that represents no real place. |
| `y` (`lat`, `latitude`) | float | as `x` | no | 0 | |
| `signalised` (`signal`, `signals`) | boolean | — | no | false | A signalised node: every approach gets a control delay and a green fraction. |
| `signal_location` | boolean | — | no | false | A signal stands here but stops only the links whose own `signalised` says so (a stop line's). Ignored where `signalised` is true. |

Without a node table, the nodes are laid out on a grid `layout_spacing_m` apart, for figures only.

### GMNS: `node.csv`, `link.csv`, `config.csv`

% src: python/openmobisim/gmns.py (module docstring, network_read_gmns, _road_rows, _attach_layer, _units, network_write_gmns)

[GMNS](https://github.com/zephyr-data-specs/GMNS), the General Modeling Network Specification, is a plain-CSV network
standard. {py:func}`openmobisim.network_read_gmns` reads either one GMNS folder (the road network; its bike and walk
layers are then derived from it) or a folder holding `road/`, `bike/` and `walk/` GMNS folders, as
{py:func}`openmobisim.network_write_gmns` writes them (each layer read as written). A link whose `directed` is false
becomes two links. Units are converted once, on reading, from `config.csv`; a link's `geometry` is not read.

**`config.csv`** (optional; metres and km/h without it):

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `long_length` | string | — | no | `m` | The length unit: `m`, `km`, `mi` or `ft` (and their long names). |
| `speed` | string | — | no | `kmh` | The speed unit: `kmh` (`km/h`, `kph`) or `mph`. |
| `crs` | string | — | no | WGS84 | Only `EPSG:4326` (longitude and latitude) is read; anything else is refused. |

**`node.csv`:**

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `node_id` | string | — | yes | — | The node. |
| `x_coord` | float | degrees | yes | — | Longitude. |
| `y_coord` | float | degrees | yes | — | Latitude. |
| `ctrl_type` | string | — | no | none | `signal` (also `signalized`, `signalised`, `traffic_signals`): a signalised node. Road layer only. |
| `signal_stops` | string | — | no | `all` | With `ctrl_type` `signal`: `all`, every link into the node stops at it; `links`, only the links whose `signal` column says so (a stop line's, as openmobisim writes it). Road layer only. |

**`link.csv`:**

| Column | Type | Unit | Required | Default | Meaning |
|---|---|---|---|---|---|
| `link_id` | string | — | yes | — | The link (`<link_id>_r` for the reverse of an undirected one). |
| `from_node_id` | string | — | yes | — | Its start node. |
| `to_node_id` | string | — | yes | — | Its end node. |
| `directed` | boolean | — | no | true | `false` (`0`, `no`, `n`) makes two links. |
| `length` | float | `long_length` | no | the straight line | Its length. |
| `free_speed` | float | `speed` | road: no; bike and walk: yes | the class's | Its free-flow speed (on the bike and walk layers, the speed ridden or walked). |
| `facility_type` | string | — | no | from `allowed_uses`, else the class defaults | The road class name. |
| `lanes` | whole number | lanes | no | the class's | Road layer. |
| `capacity` | float | PCU/h **per lane** (GMNS's convention) | no | the class's | Road layer: multiplied by the lanes on reading. |
| `allowed_uses` | list, `,`-separated | — | no | all | Road layer: a link that names uses but not `auto`, `car` or `all` is closed to cars (a busway if `bus` is among them; a cycleway if only `bike`; else a footway). |
| `roundabout` | boolean | — | no | false | Road layer (an openmobisim column). |
| `signal` | boolean | — | no | false | Road layer: a signal stopping this link alone (an openmobisim column). |
| `bike_facility` | string | — | no | `mixed` | Bike layer: `separated`, `lane` or `mixed`. |

**What {py:func}`openmobisim.network_write_gmns` writes:** one folder per layer (`road/`, and with `layers=True`
`bike/` and `walk/`), each with `node.csv` (`node_id`, `x_coord`, `y_coord`, and `ctrl_type` on the road layer),
`link.csv` (`link_id`, `from_node_id`, `to_node_id`, `directed` always `true`, `length` in m, `free_speed` in km/h,
`facility_type`; on the road layer `lanes`, `capacity` per lane, `allowed_uses`, `roundabout`, `signal`; on the bike
layer `bike_facility`; with `geometry=True` the link's polyline as WKT) and `config.csv` (`m`, `kmh`, `EPSG:4326`). Ids
are the network's own indices, zero-padded, so a network read back keeps its order but not its earlier ids.

### OpenStreetMap: `.osm.pbf`

% src: crates/io-osm/src/tags.rs (classify, direction, lanes, maxspeed, node_signal, node_splits_way, bike_way, walk_way,
% duration_seconds); crates/io-osm/src/import.rs road_spec; crates/core-graph/src/defaults.rs RoadClass::from_osm_highway;
% python/openmobisim/_core.pyi network_read_osm

{py:func}`openmobisim.network_read_osm` reads an extract into the road network and its bike and walk layers. A tag it
cannot interpret falls back to the road class's default and is counted in `Network.report_import()`; nothing stops the
import. These are the tags it reads.

**Roads (the car network):**

| Tag | Values read | Effect |
|---|---|---|
| `highway` | `motorway`, `motorway_link`, `trunk`, `trunk_link`, `primary`, `primary_link`, `secondary`, `secondary_link`, `tertiary`, `tertiary_link`, `unclassified` (and `road`), `residential`, `living_street`, `service`, `busway` (and `bus_guideway`) | The road class; a way of another value is not a road. |
| `area` | `yes` | Left out (a square, not a way). |
| `access` | `no`, `private` | Left out, unless `bus` or `psv` is `yes`/`designated`: then a busway (buses only). |
| `motor_vehicle` | `no` | With `bus` or `psv` `yes`/`designated`: a busway. |
| `oneway` | `yes`/`true`/`1`; `-1`/`reverse`; `no`/`false`/`0` | Direction. Without it, `junction=roundabout`/`circular` and motorways (and their slip roads) are one-way; everything else two-way. `reversible` and `alternating` are read as two-way. |
| `junction` | `roundabout`, `circular` | One-way; `roundabout` also marks the circulating carriageway, where entering traffic gives way. |
| `lanes`, `lanes:forward`, `lanes:backward` | whole numbers | Lanes per direction: the directional tags win; else the total, all of it on a one-way street, half (rounded down, at least 1) each way on a two-way one. Absent: the class's. |
| `maxspeed` | a number (km/h), or with `km/h`, `kph`, `kmh`, `mph`, `knots`; `walk` (7 km/h); `none` | The free-flow speed; a list takes its first value. `none` and unreadable values (such as `DE:urban`) take the class's speed, with a diagnostic. |

**Nodes:**

| Tag | Values read | Effect |
|---|---|---|
| `highway` | `traffic_signals` | A signal: the approaches it stops get a control delay and their turns a green fraction. Not when `traffic_signals=emergency`. |
| `traffic_signals:direction` | `forward`, `backward` (else both) | The signal stops only the traffic going that way along the way (a stop line). Without it, a signal within `signal_merge_m` of a junction stops only the traffic heading into that junction ([Networks](../concepts/networks.md#signals)). |
| `highway` | `traffic_signals`, `mini_roundabout`, `stop`; any `barrier` | The way is split at the node. |

**The bike layer:**

| Tag | Values read | Effect |
|---|---|---|
| (every road but motorways and `motorroad=yes`) | — | A bike link in mixed traffic, in the road's directions. |
| `bicycle` | `no`, `use_sidepath` | No bike link. |
| `bicycle` | `yes`, `permissive`, `designated`, `dismount` | Footways, paths, tracks, steps and pedestrian streets carry bikes (shared with pedestrians; separated if `designated`); also lets bikes through `access=no`/`private` and onto busways. |
| `bicycle` | `dismount` (and `highway=steps`) | The bike is walked: walking speed. |
| `highway` | `cycleway` | A separated track. |
| `cycleway`, `cycleway:both`, `cycleway:right`, `cycleway:left` | a lane or a track | Dedicated infrastructure on that side: on a two-way street the right side serves the way's direction and the left the reverse (right-hand traffic assumed); on a one-way street a lane on either side serves its direction. |
| `cycleway` | `opposite`, `opposite_lane`, `opposite_track` | Opens the contraflow on a one-way street (mixed, or the lane or track named). |
| `oneway:bicycle` | `no`; `yes`; `-1` | Overrides the road's direction for bikes. |
| `cyclestreet`, `bicycle_road` | `yes` | A cycle street: counted as a lane. |
| `route` | `ferry` | A ferry crossing, ridden unless `bicycle=no`; `duration` (`HH:MM`, `H:MM:SS`, minutes, or ISO 8601 such as `PT1H30M`) gives its time, else `ferry_km_h`; `ferry_wait_s` is added. |

**The walk layer:** every way with a `highway` tag is walked, both ways, except motorways and their slip roads,
`motorroad=yes`, `foot=no`, `foot=use_sidepath`, areas, ways with `access=no`/`private` unless `foot` is `yes`,
`designated` or `permissive`, and busways unless `foot` allows. Cycleways are walked unless `foot=no`. Ferries are walked
unless `foot=no`.

**Region:** with `region` (a rectangle or a polygon), nodes outside are dropped; with `region_stubs` each road keeps its
first node outside, and with `region_buffer_m` motorways, trunk roads and their slip roads within that distance are kept
(2 000 m by default; a main road cut by the buffer's edge is joined to its other carriageway by a turnaround).
`connectivity="strong"` keeps only the largest part of the car network in which every node reaches every other.

### OpenStreetMap parkings

% src: crates/io-osm/src/parkings.rs; python/openmobisim/_core.pyi parking_read_osm

{py:func}`openmobisim.parking_read_osm` reads park-and-ride car parks and bike parkings from an extract, from nodes and
ways (an area's centroid), inside the region if one is given.

| Tag | Values read | Effect |
|---|---|---|
| `amenity` | `parking` | A car park, kept only if it is a park-and-ride: `park_ride` present and not `no`, or a `name` containing `P+R`, `P&R`, `park and ride` or `transferium`. |
| `amenity` | `bicycle_parking` | A bike parking. |
| `access` | `private`, `no` | Left out. |
| `capacity` | a number | Its capacity; else, for an area, its ground area over `area_per_car_m2` (25 m²) or `area_per_bike_m2` (1.5 m²); else `capacity_car` (100) or `capacity_bike` (10). |

A node inside an area of the same kind is the same parking mapped twice and is left out; parkings of one kind closer
than `merge_m` (100 m) are merged into one site, capacities added. The five numbers are arguments of
`parking_read_osm`, uncalibrated defaults.

## Timetables: GTFS

% src: crates/io-gtfs/src/read.rs (module docs, read_stops, read_routes, read_calendars, stop times, transfers, frequencies);
% crates/core-transit/src/kind.rs ServiceKind::from_route_type; crates/py-bindings/src/transit.rs transit_read_gtfs

{py:func}`openmobisim.transit_read_gtfs` reads one service day of a [GTFS](https://gtfs.org/schedule/reference/) feed (a
zip, or a folder of its `.txt` files): the `date` given, else the busiest weekday (the Monday to Friday with the most
trips in service, the earliest on a tie, within a year of the feed's first date). With a `network`, only the stops
within 300 m of its walk layer's nearest node are kept, and each run keeps its calls at them; a run left with fewer than
two calls is dropped. A row that cannot be read (a bad number, an unknown stop or trip) is skipped and counted in
`Transit.report`; only a missing file or column, or a day with no service, is an error.

| File | Columns read (required in bold) | What is kept |
|---|---|---|
| `stops.txt` | **`stop_id`**, **`stop_lat`**, **`stop_lon`**, `stop_name`, `location_type`, `parent_station` | Stops and platforms (`location_type` 0 or empty) with a position. |
| `routes.txt` | **`route_id`**, **`route_type`**, `route_short_name`, `route_long_name` | The line's name (the short name, else the long one) and kind: `route_type` 0, 5 and 900–999 tram; 1, 12 and 400–699 metro; 2, 100–199 and 300–399 rail; 3, 11, 200–299 and 700–899 bus; 4, 1000–1099 and 1200–1299 ferry; anything else `other`. Only buses ride the roads. |
| `trips.txt` | **`trip_id`**, **`route_id`**, **`service_id`** | The trips in service on the day. |
| `calendar.txt` (optional) | **`service_id`**, **`start_date`**, **`end_date`**, `monday` … `sunday` | Which services run on the day. |
| `calendar_dates.txt` (optional) | **`service_id`**, **`date`**, **`exception_type`** (1 added, 2 removed) | Exceptions to the calendar. |
| `stop_times.txt` | **`trip_id`**, **`stop_id`**, **`stop_sequence`**, **`arrival_time`**, **`departure_time`**, `pickup_type`, `drop_off_type`, `shape_dist_traveled` | The calls of the day's trips, in sequence. `pickup_type` 1 forbids boarding and `drop_off_type` 1 alighting. A blank time is interpolated between the timed calls around it (by `shape_dist_traveled`, else by straight-line distance); a trip whose first or last call has no time is dropped. |
| `transfers.txt` (optional) | **`from_stop_id`**, **`to_stop_id`**, **`transfer_type`**, `min_transfer_time` | Stop-to-stop minimum transfer times: `transfer_type` 2 with its time, 1 (timed) as zero. Rows for particular routes or trips, between stations, and types 0 and 3 are skipped and counted. |
| `frequencies.txt` (optional) | **`trip_id`**, **`start_time`**, **`end_time`**, **`headway_secs`** | A listed trip is a template: it runs from each row's start, every headway, while before its end, one run each, named `<trip_id>@<seconds after midnight>` (`exact_times` 0 and 1 alike). |

**Not read:** `agency.txt`, `shapes.txt`, fares, and trips of the previous service day that run past midnight into this
one.
