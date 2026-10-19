# Transit, parking, park-and-ride and bike-and-ride

This guide adds public transport and parkings to a scenario, and changes the service for a study. How journeys,
parkings and the station choice work is explained in [Transit](../concepts/transit.md),
[Parking](../concepts/parking.md) and [Modes](../concepts/modes.md).

## Give the scenario a timetable and parkings

% src: _core.pyi transit_read_gtfs, parking_read_osm; parking.py parking_read_table; scenario.py from_parts
```python
transit = ms.transit_read_gtfs("gtfs.zip", network, date="2026-10-09")
parkings = ms.parking_read_osm("city.osm.pbf", region=region)     # or ms.parking_read_table("parkings.csv")
scenario = ms.Scenario.from_parts(network, trips, classes=classes, transit=transit, parkings=parkings)
```

With these, a trip can:
- **take transit:** walk to a stop, ride (with transfers) and walk on;
- **park and ride:** drive to a car park, then continue by transit;
- **bike and ride:** cycle to a bike parking, then continue by transit.

A trip may state one of these modes (`transit`, `car_transit`, `bike_transit`), or choose among them when its class
allows. For park-and-ride and bike-and-ride, the station is chosen by the choice model among candidate parkings, not
fixed in advance.

**Buses ride the roads.** Buses are loaded among the cars, take their delays, and their realised times replace the
schedule's. Trams, metros, trains and ferries keep to the timetable. Results are in `transit_calls.parquet` and
{py:attr}`openmobisim.Run.transit_summary`.

## Change the service

% src: python/openmobisim/transit.py transit_edit
{py:func}`openmobisim.transit_edit` makes a scenario's timetable from the original. The original is left as it was,
and stops and lines keep their ids:

```python
more_frequent = ms.transit_edit(transit, headway={"T1": (300, 7 * 3600, 9 * 3600)})  # every 5 min, 07:00–09:00
without = ms.transit_edit(transit, cancel=["B1"])                                     # line out of service
```

A new headway applies to each of the line's stop patterns that runs in the window; it makes a line more or less
frequent, but adds no new route. `transit.lines()` lists the lines with their first and last departures.

Feeds that describe service by frequency (`frequencies.txt`) are read as runs at those frequencies.

## Hub groups in the results

% src: crates/core-sim/src/hub_groups.rs; scenario.py Scenario.run (hub_group_m=)
`transit_calls` and `parking_bins` (the files and the tables) carry a `hub_group`: a stop's station (GTFS
`parent_station`), or the stop itself; a parking takes the group of the nearest stop within `hub_group_m` (an
argument of `Scenario.run`, 150 m in a straight line), unless its table named a hub, which it keeps. A station's
platforms, its bike racks and its car park then add up under one name:

```python
calls = run.transit_calls().to_pandas()
racks = run.parking_bins().to_pandas()
boardings = calls.groupby("hub_group")["boardings"].sum()
parked = racks[racks.vehicle == "bike"].groupby("hub_group")["arrivals"].sum()
```

A label of the outputs only: the model's own hubs (each parking with the stops its travellers reach on foot) are not
changed by it, and neither is any result. In Amsterdam, 547 of the 922 parkings join a station's group, nearly all of
them bike parkings.

## The options that matter most

% src: parameters (transit and parking groups); reference/parameters.md
| Option | Default | What it limits |
|---|---|---|
| `transit_options["access_walk_max_s"]` | 1800 s | The walk to and from a stop |
| `transit_options["journey_max_s"]` | see [Parameters](../reference/parameters.md) | The longest journey offered |
| `transit_options["transfer_walk_max_s"]` | 300 s | A walk between two stops to transfer |
| `parking_options["walk_max_s"]` | 300 s | The walk from a parking to its stop |
| `parking_options["pr_min_km"]` | see [Parameters](../reference/parameters.md) | The shortest trip offered park-and-ride or bike-and-ride |

A trip none of whose modes reaches its destination within these limits has no alternative, and is reported (see
[Modes](../concepts/modes.md#a-trip-with-no-alternative)).

## A worked example on Delft

Delft's starter case carries its timetable and parkings, and its default scenario uses them:

```{include} _examples/transit_parking-parts.md
```

The mode each person took, in the default morning:

```{include} _examples/transit_parking-run.md
```

The busiest line every five minutes through the morning peak, the rest unchanged:

```{include} _examples/transit_parking-headway.md
```

The boardings by hub group, each named by one of its stops. The groups are the feed's own stations: where the feed
gives a station's platforms and its bus stops different stations, they stay apart, as Delft's railway station and the
stops beside it do here.

```{include} _examples/transit_parking-hubs.md
```

## For bikes

Bike-and-ride needs `bike_transit` among the class's modes, an owned bike, and bike parkings near stops. Its
candidates are the bike parkings within the bike's reach (`parking_options["reach_bike_s"]`). See
[Cycling](cycling.md).
