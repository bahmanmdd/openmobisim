# Changelog

Each release lists what changed for a user. Versions follow [semantic versioning](https://semver.org/); before 1.0,
a minor version may change the interface, and says so here.

## Unreleased

**Python 3.13 or newer.** openmobisim now needs Python 3.13 or later, following the scientific Python ecosystem's
support policy ([SPEC 0](https://scientific-python.org/specs/spec-0000/)); one wheel per platform serves 3.13 and
every later version, tested on 3.13 and 3.14. 0.1.0 accepted Python 3.11 and 3.12; on those, pip installs 0.1.0.

**Door-to-door times for every mode.** Every run's car and bike trips pay the fixed times of their own vehicle, in
the loading as mode choice counts them: reaching it before the leg and leaving it after (`mode_options`
`car_access_s` 120 s, `car_egress_s` 300 s, `bike_access_s` 60 s, `bike_egress_s` 60 s). Setting them to 0 leaves the
drive or the ride alone, as a benchmark such as Sioux Falls counts it. The egress is also the time to park at a car or
bike park with room; a full parking adds its slope on top.
- `trip_results` has two new columns: `vehicle_s`, the completed trip's seconds in a vehicle of its own, and
  `access_egress_s`, its fixed seconds. `Run.total_vehicle_time_s` and `Run.mean_vehicle_time_s` sum and average
  the first.
- `parking_options` `floor_car_s` and `floor_bike_s` are gone (a scenario naming them is refused with the list of
  options); `slope_car_s` and `slope_bike_s` stay.
- Before this change the loading started a car at the trip's departure and added no egress, so a car trip's travel
  time was its drive while mode choice weighed the door-to-door time: the mean trip of every run is longer now, by
  the fixed times; a run of stated car trips sets them to 0 to keep the drives.

**Buses keep to their timetable.** By default every bus runs exactly on its timetable and is not a vehicle of the
loading: it takes no road capacity and traffic does not delay it. `transit_options` `buses_on_roads` 1 puts the buses
on the roads among the cars, where congestion makes them late, as 0.1.0 did. Travellers plan transit on the
timetable in every iteration, with buses on the roads too, and ride the vehicles that come; 0.1.0 planned on the
bus times of the last iteration.
- `convergence()` has two new columns: `gap_transit_plan_s`, how much later the transit trips' rides ended than on
  the timetable they planned on (0 with buses on the timetable), and `gap_transit_plan_share`, its share of the
  itineraries' gap with transit.
- The timetable searches are kept for the run and answer the same query again (the same results, sooner);
  `transit_summary` counts them (`timetable_searches_*`, `timetable_reused_*`, `timetable_kept`,
  `timetable_kept_bytes`).

## 0.1.0 — 2026-10-20

The first public release: **an alpha, for testing by students and colleagues.** The tests are not complete; please use
it and [report issues](https://github.com/bahmanmdd/openmobisim/issues).

**Networks and data**
- Road networks from OpenStreetMap (`network_read_osm`, clipped to a study area with a buffer of main roads), GMNS
  (`network_read_gmns`, read and written) or link tables (`network_read_table`, TNTP included); foreign units
  converted once on reading.
- Bike and walk networks (layers) from the same extract, with cycle tracks, painted lanes and mixed traffic told apart; ferries.
- Public transport from GTFS (one service day, clipped; `frequencies.txt` expanded); parkings from OpenStreetMap or a
  table.
- Demand from trip tables, person tables or origin–destination matrices (`demand_read_*`, `demand_from_od`);
  user classes (traveller classes) with their own modes, ownership and preferences (`classes=`, `demand_read_classes`,
  `demand_assign_classes`); `network_check` audits a network before a run.

**Simulation**
- Six modes: car, bike, walk, transit, park-and-ride, bike-and-ride; mode choice among the modes a class may use.
- Route sets and route choice for cars; mode chains (itineraries) for the multimodal modes, the transit journey found by RAPTOR and
  the parking chosen through the choice set.
- A nested logit by default, one nest per mode (the path-size logit where a run offers one mode), also the logit and
  a deterministic choice, or any model written in Python, registered by name, seeing
  times by part, money (no prices by default: give tolls, fees, fares or fuel as you need them), the network along
  each alternative (your own link values) and the traveller and trip.
- Dynamic network loading: the link transmission model applied to individual vehicles, event by event in continuous
  time, as a vertical queue (point queue; the default) or with spillback, turn pockets and en-route rerouting; buses ride the roads they share with cars, busways their own.
- Dynamic traffic assignment (equilibration) by the method of successive averages, stopping when converged, with a convergence verdict; a single
  quick loading (`equilibration="free_flow"`).
- Scenario edits (`network_edit`: closures, lanes, capacity, and cycle tracks or lanes added or taken away), timed
  disruptions (`disruptions=`), skims (`Run.skim`) and logsums (`Run.logsum_od`).

**Results**
- Per run: `kpis.parquet`, `trip_results.parquet` (every trip: who, from where to where, when, by which mode, what
  became of it, and where a car still travelling at the window's end was; `Run.trip_results()`),
  `diagnostics.parquet`, `events.parquet` (one trip in a hundred), the link bins of the road, bike and walk layers,
  `transit_calls.parquet`, `parking_bins.parquet`, `manifest.json` and `timings.csv`; or, for
  mass runs, `kpis.csv` and `manifest.json` alone (`outputs="minimal"`). A fingerprint identifies the inputs and
  settings; `Run.open` reads a finished run back.
- Figures in `openmobisim.viz` (link maps of the roads or the bike layer, routes, demand, transit by line, parkings,
  mode shares, a summary page) and an interactive map (`map_interactive`) with layers and PNG and SVG export.
- `parameters()` lists every parameter, its default and unit; every one is overridable by name.

**Starter cases and tutorials**
- Six starter cases (Delft, Amsterdam, Paris, Lyon, Sioux Falls, Nguyen–Dupuis), downloaded once and checked;
  `openmobisim check` compares an installation with this version's reference results; `openmobisim run <case>` runs
  one and writes its summary page and interactive map, each linked when written.
- Ten tutorial notebooks, in `notebooks/`, starting with `00_getting_started` (an overview of the others); in the
  package too, without outputs: `openmobisim notebooks` copies them into a folder and opens the first in JupyterLab
  (`openmobisim.notebooks()` from Python). `openmobisim` alone says what to run first.

**Known limitations** (stated with results)
- Urban free-flow speeds are the posted limits, with delay only at signals.
- Bikes: no signal delay, no interaction with cars, no gradients, one speed per facility.
- The vertical (point) queue has no spillback (`Run.report_spillback()` says where it would matter).
- The starter cases' demand is synthetic.
- The manual's [limitations](https://openmobisim.github.io/concepts/limitations.html) page lists them all.
