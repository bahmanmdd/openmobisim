# Delft as tables: the fixture of test 10

A starter network: realistic-looking, made so that a user without a network of their own can start
and so that test 10 has a city-sized case. It is not a calibrated case of Delft, and its values are
not vouched for.

- `nodes.csv.gz`: `node_id`, `lon_deg`, `lat_deg` (WGS84). 21 566 nodes; a node is a link end's position.
- `links.csv.gz`: the columns of `Network.from_tables` (`crates/binding/README.md`). 12 460 car links
  with their lanes, free-flow speed (km/h) and capacity (veh/h), 20 652 bike links and 55 400 walk
  links, each with one mode. The road network that openmobisim 0.1 builds from the OpenStreetMap
  extract in `crates/io-osm/tests/fixtures/delft/` (© OpenStreetMap contributors, ODbL), written out as
  tables. No signals.
- `trips.csv.gz`: `trip_id`, `origin_lon`, `origin_lat`, `destination_lon`, `destination_lat`,
  `departure_s` (a second of the clock), `class_id`, `mode` (empty: the test gives each class its mode
  set). 65 959 trips with both ends inside Delft, leaving 07:30 to 08:30: openmobisim 0.1's seeded demand
  recipe on its Delft zone matrix, the rows inside the city scaled up to the matrix's total. Four
  classes: `car_and_bike`, `car_only`, `bike_only`, `no_vehicle`.

Known facts about this network (found while writing test 10): the westbound carriageway of
Wippolderlaan between the A13 on-ramp and the junction east of it is missing, so cars detour there;
bus-only roads (`highway=busway`) carry no cars.
