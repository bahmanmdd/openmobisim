# Delft as tables: the fixture of test 10

A starter network: realistic-looking, made so that a user without a network of their own can start
and so that test 10 has a city-sized case. It is not a calibrated case of Delft, and its values are
not vouched for.

- `nodes.csv.gz`: `node_id`, `lon_deg`, `lat_deg` (WGS84). 21 566 nodes; a node is a link end's position.
- `links.csv.gz`: the columns of `Network.from_tables` (`crates/binding/README.md`). 12 460 car links
  with their lanes, free-flow speed (km/h) and capacity (veh/h), 20 652 bike links and 55 400 walk
  links, each with one mode. The road network that openmobisim 0.1 builds from the OpenStreetMap
  extract in `crates/io-osm/tests/fixtures/delft/` (© OpenStreetMap contributors, ODbL), written out as
  tables. No signals. For drawing only, two more columns (no run reads them; the network's
  fingerprint does not see them): `shape`, the points between the link's two ends in its own
  direction, `lon lat;lon lat` (WGS84; empty: straight), and `road_class`, the OpenStreetMap
  `highway` class. Both come from the same derivation's own tables, openmobisim 0.1's Delft network
  written as GeoParquet (`road_links.parquet`, `bike_links.parquet`, `walk_links.parquet` of version 1's
  `delft-tables.zip`: each line's points between its ends, and its `facility_type`): within each mode
  the fixture's k-th link is the source's k-th (car: the road layer's `auto` links), every row's
  ends equal to the source line's first and last points and its length to the source's within
  1e-5 m. All 88 512 links have a class (19 names, every one of the map's table); 42 958 have inner
  points, 45 554 are straight. Every polyline's length (WGS84) is its `length_m` within 1.3e-9
  relative, but for 50 walk and bike links of 0.1 to 0.5 m that version 1 gave its least length,
  0.5 m. Of 42 598 pairs of links each way between two nodes, 42 523 have one's points the other's
  reversed; 75 (47 car, 28 bike) do not (read as two roads between the same junctions, such as
  two carriageways; not checked one by one). Adding the columns
  took the file from 1 349 448 to 2 165 625 bytes; less the two columns, its text is the old one
  byte for byte (SHA-256 6918508b…5ad5b of the 5 848 933 decompressed bytes).
- `trips.csv.gz`: `trip_id`, `origin_lon`, `origin_lat`, `destination_lon`, `destination_lat`,
  `departure_s` (a second of the clock), `class_id`, `mode` (empty: the test gives each class its mode
  set). 65 959 trips with both ends inside Delft, leaving 07:30 to 08:30: openmobisim 0.1's seeded demand
  recipe on its Delft zone matrix, the rows inside the city scaled up to the matrix's total. Four
  classes: `car_and_bike`, `car_only`, `bike_only`, `no_vehicle`.

Known facts about this network (found while writing test 10): the westbound carriageway of
Wippolderlaan between the A13 on-ramp and the junction east of it is missing, so cars detour there;
bus-only roads (`highway=busway`) carry no cars.
