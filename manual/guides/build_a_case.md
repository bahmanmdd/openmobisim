# Build a case from OpenStreetMap and GTFS

This guide builds a complete multimodal case for a city from open data, and runs it:
- the street network from an OpenStreetMap extract;
- public transport from a GTFS feed;
- parkings from the same extract;
- a morning's trips from an origin–destination matrix.

The example is Delft, in the Netherlands, with the files of the starter data bundle. The same steps work for any city
you have an extract and a feed for. The figure shows the way the data takes: each file is read into an object, a
`Scenario` combines them with the settings, and a run turns them into results.

```{image} ../_static/figures/dataflow_paper.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, and a run turns them into a Run object and output files
:class: only-light
:width: 100%
```
```{image} ../_static/figures/dataflow_night.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, and a run turns them into a Run object and output files
:class: only-dark
:width: 100%
```

% src: tested with python/openmobisim at the manual's build, on the Delft case's files:
% read 3.2 s; run 16.0 s at weight 4 on an Apple M2 Pro laptop; the outputs quoted below are that run's.

## 1. The network

% src: _core.pyi network_read_osm; audit.py network_check
```python
import openmobisim as ms

region = (4.315, 51.965, 4.41, 52.035)  # west, south, east, north, in degrees
network = ms.network_read_osm("network.osm.pbf", region=region)
print(network)  # Network(21663 nodes, 46951 links, with street geometry)
```

`region` cuts the study area out of a larger extract, as a box or as a polygon (a list of `(longitude, latitude)`
points). The network you get holds:
- the roads, with each link's traffic parameters derived from its OpenStreetMap tags;
- a walk layer and a bike layer over the same streets (`network.layer("walk")`, `network.layer("bike")`), with
  cycleways, cycle lanes and one-way streets open to bikes as the tags say.

Check the network before you run it:

```python
report = ms.network_check(network)
report["verdict"]  # "pass", "warn" or "fail"
[c for c in report["checks"] if c["status"] != "pass"]
```

Each check names its layer, what it tests, its status, the value found and the rule it applies. For Delft the verdict
is `warn`, with one finding: 2.1 % of the road links run parallel to another between the same two nodes. This is
worth a look on the map, but it does not stop a run. A `fail`, such as a road network in several pieces that cannot
reach one another, means some car trips will find no route. How the network is built, and every option that shapes it
(`network_options`), is in [Networks](../concepts/networks.md).

## 2. Public transport

% src: _core.pyi transit_read_gtfs (docstring: date, network)
```python
transit = ms.transit_read_gtfs("gtfs.zip", network, date="2026-10-09")
print(transit)  # Transit(2026-10-09: 869 runs, 255 stops, 24 lines)
```

A feed covers many days; a run uses **one service day**, `date` (without it, the feed's busiest weekday). Given the
network, the reader keeps only the stops a pedestrian can reach on its walk layer (within 300 m of its nearest node),
and every run keeps its calls at them. The buses among them will ride the roads with the cars; trams, metros,
trains and ferries keep to their timetable. See [Transit](../concepts/transit.md).

## 3. Parkings

% src: _core.pyi parking_read_osm
```python
parkings = ms.parking_read_osm("network.osm.pbf", region=region)
print(parkings)  # Parkings(2 car, 190 bike)
```

These are the car and bike parkings that park-and-ride and bike-and-ride trips can use. From OpenStreetMap,
openmobisim keeps the car parks tagged or named as park-and-ride and every public bike parking. A capacity comes
from the `capacity` tag, else from the parking's area, else a default; parkings of one kind closer than 100 m are
merged into one site. A table of your own works too, read with {py:func}`openmobisim.parking_read_table`. See [Parking](../concepts/parking.md).

## 4. The demand

% src: demand.py demand_read_zones, demand_read_od, demand_read_classes, demand_from_od, demand_assign_classes,
% demand_sample; _cases.py Case.demand (the same pattern)
A morning's trips are drawn from an origin–destination matrix between zones. Each traveller is given a **class**,
which says the modes they may use, what they own and how they weigh time and money:

```python
zones = ms.demand_read_zones("demand/zones.csv")      # 65 zones, each a point
od = ms.demand_read_od("demand/od.csv")               # 5093 rows: origin_zone, destination_zone, trips, …
classes = ms.demand_read_classes("demand/classes.csv")

trips = ms.demand_from_od(
    od, zones,
    start_s=7.5 * 3600, end_s=8.5 * 3600, profile="am_peak",  # departures 07:30–08:30, peaked
    spread_m=500.0,             # trips start and end up to 500 m around their zone's point
    user_class="drawn",         # the class of rows that do not name one
    seed=0,
)
```

Some rows of Delft's matrix name their own class and mode: through traffic entering at the edge of the study area,
and goods vehicles. Draw classes only for the other trips, so those keep theirs:

```python
drawn = ms.demand_assign_classes([t for t in trips if t[7] == "drawn"], classes, seed=0)
given = iter(drawn)
trips = [next(given) if t[7] == "drawn" else t for t in trips]
```

`t[7]` is the trip's class. A trip is a row of `(traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
destination_lat, departure_time_s, user_class, weight, mode)`, `trip_seq` being the trip's place in the traveller's
day and `departure_time_s` seconds after midnight; see
[Input files](../reference/inputs.md). {py:func}`openmobisim.demand_assign_classes` gives each traveller a class by
the classes' shares: who gets which class is random, but how many people each class gets is met to within one
traveller.

**For a faster run**, simulate one traveller for every few people:

```python
trips = ms.demand_sample(trips, weight=4, seed=0)   # 106 596 trips → 26 651, each standing for 4 people
```

Each kept traveller then stands for four people, and travels as a group of four on the real network. Totals in the
results stay those of the whole demand. See [Running faster](speed.md).

## 5. The scenario and the run

% src: scenario.py Scenario.from_parts, Scenario.run (the window, classes turning mode choice on)
```python
scenario = ms.Scenario.from_parts(
    network, trips,
    classes=classes, transit=transit, parkings=parkings,
    window=("07:30", "09:30"),   # the simulation window
    link_bin_s=900,              # per-link results in 15-minute bins
)
run = scenario.run("delft", output_dir="runs/delft")
```

Because the classes say which modes their travellers may use, every trip without a stated mode **chooses its mode**
among walking, cycling, driving, transit, park-and-ride and bike-and-ride, as its class allows. The run iterates
towards an equilibrium (`msa`) and stops when it has converged:

```python
run.fingerprint          # '1108c37f732cbe5d': what was run, see Reproducibility
run.converged            # True
run.convergence_gap      # 0.034
run.completion           # trips completed, unfinished at the window's end, without a route, …
```

The run writes its files into `runs/delft`:
- `kpis.parquet` and `link_bins.parquet`;
- `link_bins_bike.parquet` and `link_bins_walk.parquet`, for the cycling and walking layers;
- `transit_calls.parquet` and `parking_bins.parquet`;
- `events.parquet` and `diagnostics.parquet`;
- `manifest.json` and `timings.csv`.

Each file is described in [Output files](../reference/outputs.md), and the figures in [Results and
figures](results.md).

## What to watch

- **Trips outside the window are left out, with a warning.** A trip departing before the window's start or after its
  end is not simulated. Each run warns how many trips and people that is and what to change; see
  [The simulation window](window.md).
- **Trips with no alternative are reported.** The warning says how many, and between which places most of them are.
  See [When something goes wrong](troubleshooting.md).

## For bikes

The bike layer comes with the network: cycleways, cycle lanes and contraflow cycling are read from the tags, and each
bike link carries its facility (separated, lane or mixed), which the choice model can weigh. `network_check` reports the bike layer's
length and its share of separated cycleways and cycle lanes. A class that may cycle needs `bike` among its modes (and
owns a bike); bike-and-ride needs bike parkings near stops, which `parking_read_osm` reads.
