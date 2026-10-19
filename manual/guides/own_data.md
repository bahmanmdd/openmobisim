# Bring your own network and demand

openmobisim can build a network from OpenStreetMap ([Build a case](build_a_case.md)), but a study usually has its own
network and demand. Both come in through standard files. Every column is described in
[Input files](../reference/inputs.md).

## A network in GMNS

% src: python/openmobisim/gmns.py (module docstring), network_read_gmns, network_write_gmns
GMNS (the General Modeling Network Specification) is a plain-CSV standard: a `node.csv`, a `link.csv` and an optional
`config.csv` with units. Other tools such as DTALite, osm2gmns and path4gmns read and write it.

```python
network = ms.network_read_gmns("my_network/")
ms.network_write_gmns(network, "exported/")   # road/, bike/ and walk/, one GMNS network each
```

- **One folder per layer.** A folder holding `road/`, `bike/` and `walk/` is read as the road network with its bike
  and walk layers, exactly as written. A single GMNS folder, as other tools write, is read as the road network. Its
  layers are then derived from it: bikes on every road but motorways, in the road's direction; walking on every road
  but motorways and cycleways, both ways.
- **Units are converted on reading:** lengths in `m`, `km`, `mi` or `ft`, and speeds in `kmh` or `mph`, as
  `config.csv` says. Capacities are per lane, GMNS's convention.
- **Ids are renumbered.** A network written by openmobisim has its ids as zero-padded indices. After a round trip,
  give link values and edits by the new ids.

## A network as a link table

% src: python/openmobisim/network.py network_read_table (Args)
```python
network = ms.network_read_table("links.csv", "nodes.csv", length_unit="km", speed_unit="km_h")
```

Any delimited file works: CSV, TSV, or the TNTP format of the classic benchmark networks. You can also pass rows from a
data frame (`df.to_dict("records")`). `from` and `to` are required; the length, lanes, free-flow speed or time,
capacity and class are optional. A link without a class or lanes takes the defaults of its class. A class the table
does not recognise is treated as unclassified, with a warning.

## Check it

```python
report = ms.network_check(network)
```

This finds what would make trips fail, such as pieces the cars cannot cross between, and what is worth a look on a map.
See [Build a case](build_a_case.md#1-the-network).

## Demand

% src: python/openmobisim/demand.py demand_read_trips, demand_read_zones, demand_read_od, demand_from_od,
% demand_write_trips
**Trips with their own times and places:**

```python
trips = ms.demand_read_trips("trips.csv")   # or a trips.parquet path given to Scenario.from_parts
```

The columns are `traveller_id`, `trip_seq`, `origin_lon`, `origin_lat`, `destination_lon`, `destination_lat`,
`departure_time_s` and `user_class`, and optionally `weight` and `mode`.

% src: python/openmobisim/demand.py Trip, _trips_checked; scenario.py from_parts (demand)
**In memory, in any of three forms**, the same columns:

```python
trips = [ms.Trip("t1", 0, 4.36, 52.01, 4.37, 52.00, 7 * 3600 + 900, "commuter")]   # rows; weight and mode optional
trips = frame                                     # a pandas or polars data frame with the columns
trips = {"traveller_id": [...], "trip_seq": [...], ...}   # a dict of columns
scenario = ms.Scenario.from_parts(network, trips, classes=classes)
```

`ms.Trip` is a named tuple of one row; a plain tuple of the same fields is the same row. Every row is checked before
the run: a latitude outside ±90, a departure that is not whole seconds after midnight or an unknown mode is refused
with the row's number and the field.

**An origin–destination matrix between zones:**

```python
zones = ms.demand_read_zones("zones.csv")      # each zone a point
od = ms.demand_read_od("od.csv")               # origin_zone, destination_zone, trips, …
trips = ms.demand_from_od(od, zones, start_s=7.5 * 3600, end_s=8.5 * 3600, profile="am_peak", seed=0)
```

{py:func}`openmobisim.demand_from_od` turns the matrix into whole trips and draws their departures over the period.
It spreads their start and end points around the zone's point. A matrix row may name its own class, mode and
departure period.

**Write the trips out** with `ms.demand_write_trips(trips, "trips.csv")` to share them, or to read them elsewhere.

## A worked example: a small town in GMNS

A network made by hand, as another tool would write it: a grid of 5 by 5 junctions 300 m apart, residential streets
inside and a secondary road around them. Each street is one row with `directed` false, read as one link each way;
`config.csv` gives the units.

```{include} _examples/own_data-write.md
```

Read as one GMNS network, it is the road network, and its bike and walk layers are derived from it:

```{include} _examples/own_data-read.md
```

Trips as rows of `ms.Trip`, between random junctions over an hour. Without a class table a trip's `user_class` is only
a label, and a scenario given no `modes` offers the car alone:

```{include} _examples/own_data-trips.md
```

```{include} _examples/own_data-run.md
```

```{include} _examples/own_data-map.md
```

## For bikes

A GMNS bike layer keeps its `bike_facility` (`separated`, `lane`, `mixed`), which bike routes and the choice model use.
A network without one gets a bike layer derived from its roads. Give the bike layer if you have it: separated tracks
and lanes change routes and mode choice.
