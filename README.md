# openmobisim

**An open-source, agent-based traffic simulator for road networks, with a Python interface on a
fast Rust core.**

> **Status: 0.1, alpha.** This first release simulates **car traffic** on road networks. Walking,
> cycling, transit and multimodal hubs are planned but not here yet. The API may change between
> 0.x releases. External contributions are not accepted at this stage.

## Install

```
pip install openmobisim            # the simulator; needs only numpy
pip install "openmobisim[viz]"     # with figures (matplotlib)
```

Python 3.11 or later. Wheels for Linux (x86-64, ARM64), macOS (Intel, Apple silicon) and
Windows (x86-64).

## A first run

```python
import openmobisim as ms

net = ms.examples.manhattan_grid(10, 200.0, signals=True)        # 10 x 10 junctions, 200 m apart
trips = ms.examples.trips_random(net, 3000, seed=1, max_m=2000)  # 3 000 car trips in one hour

scenario = ms.Scenario.from_parts(
    net,
    trips,
    class_defaults={"commuter": (True, False, False)},  # commuters own a car
    window_hours=2,
    link_bin_s=300,  # record per-link results in 5-minute bins
)
run = scenario.run("first-run")

print(run.completion)           # how many trips completed, and why the others did not
print(run.convergence_verdict)  # "good", "acceptable" or "poor"
```

A real network comes from an OpenStreetMap extract (for example from Geofabrik) or from a link
table, such as the TNTP files of the transport research benchmarks:

```python
net = ms.network_read_osm("andorra-latest.osm.pbf")
net = ms.network_read_table("links.csv", "nodes.csv")
```

Your own demand is a table of trips, in memory or as a `trips.parquet` file, with one row per trip:
`(traveller_id, trip_seq, origin_lon, origin_lat, destination_lon, destination_lat,
departure_time_s, user_class, weight)`.

Figures, with `openmobisim[viz]`:

```python
from openmobisim import viz

viz.map_link(run).savefig("traffic.png")  # volume and delay on every link
viz.map_interactive(run, "traffic.html")  # one self-contained interactive map
```

## What 0.1 does

- **Networks** from OpenStreetMap `.osm.pbf` extracts, kept to the part where every junction a car
  can use reaches every other, or from a plain link table, including the TNTP format (Sioux Falls,
  Anaheim and the other benchmark networks).
- **Route sets** for every origin–destination pair: link penalties, the shortest path, or a Monte
  Carlo search biased towards the links the demand is likely to congest.
- **Route choice**: a path-size logit by default, or everyone on the best route. A choice model of
  your own is a Python class with one method.
- **Traffic flow**: a link transmission model with queues that take up road space and spill back
  (the default), or simpler levels: a spatial queue, a point queue, free flow.
- **Equilibration**: the method of successive averages, 10 iterations by default, with route sets
  that grow between iterations and a convergence report for each one.
- **Results**: Parquet files (KPIs, per-link time bins, diagnostics, sampled events) and a
  `manifest.json` recording every setting. A master seed and a run fingerprint make runs
  reproducible.
- **Figures**: maps of traffic and demand, and an interactive HTML map for inspecting route
  alternatives.

## Known limitations

- **Car traffic only.** No walking, cycling, transit, hubs, mode choice or departure-time choice.
- **Gridlock.** Under very heavy demand a jam can outlast the simulation window. Iterating then
  does not settle, and the times of trips that never finish are lower bounds. Growing the route
  sets between iterations can make a run close to gridlock worse.
- **Junctions.** Only roundabouts tagged as such in OpenStreetMap give way to circulating traffic;
  other priority rules (major and minor roads) are not modelled. Turn pockets and lane drops are
  not represented. Signals use default timings, not real signal plans.
- **Parameters are assumptions, not calibrations**: the road-class defaults and the route-choice
  coefficients.
- **Aggregated travellers** (a trip weight above 1) are faster but coarser, and can lock small
  roundabouts.
- **Figures** are laid out for 8 inches wide or more; narrower, the text crowds.

## Licence

Apache-2.0: see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE). No map data ships with this package;
OpenStreetMap data is © OpenStreetMap contributors, available under the ODbL.
