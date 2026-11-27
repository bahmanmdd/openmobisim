# The cases

Five cases ready to run are published as a data record, apart from the package: *openmobisim example cases*,
version 2, <https://doi.org/10.5281/zenodo.24585967>. This page is the reference of that record: its cases, the files
of a case, how the cases were made, and the licence of each part. How to download and run a case is in
[Running a case](../guides/cases.md); cite the record as [How to cite](citing.md) says.

[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.24585967.svg)](https://doi.org/10.5281/zenodo.24585967)

The cases are made for the users of openmobisim, as convenient cases to start from. Their networks and timetables are
real. **Their demand is synthetic**: it is neither observed nor a forecast, and it is not calibrated. The cases serve
to learn the tool, to compare methods and settings, and as a realistic load for a network. For a study of a city,
bring that city's own trips.

## The five cases

% src: the record's README.md (the table of cases, "DOTNET")

| Case | Study area | Timetable | Service day of the timetable | Travellers (trips) | Archive size (MB) |
|---|---|---|---|---|---|
| `delft` | the municipality of Delft | the Dutch national feed | Friday 9 October 2026 | 37 649 | 4.5 |
| `amsterdam` | Amsterdam inside and around its A10 ring road | the Dutch national feed | Friday 9 October 2026 | 198 252 | 24.5 |
| `lyon` | Lyon and Villeurbanne | TCL | Friday 20 November 2026 | 117 073 | 19.1 |
| `paris` | Paris inside the Boulevard Périphérique, the ring included | Île-de-France Mobilités | Friday 16 October 2026 | 243 203 | 29.4 |
| `dotnet` | a synthetic city of 8 km by 8 km | 7 synthetic lines | not a day of a real feed | 39 919 | 0.4 |

Every case simulates the travellers who leave between 07:30 and 08:30, until 09:30: the window runs from 27 000 s to
34 200 s after midnight. Each traveller makes one trip.

Each case is one archive, `<case>.zip`, at `https://zenodo.org/records/24585967/files/<case>.zip` (for example
<https://zenodo.org/records/24585967/files/delft.zip>). An archive unpacks to a folder of the case's name, which
`run_file` runs:

```python
import openmobisim as oms

run = oms.run_file("delft", out="delft-run")
print(run.map("delft-map.html"))
print(run.summary("delft-summary.html"))
```

```text
delft-map.html
delft-summary.html
```

openmobisim 0.2 and later read the cases; openmobisim 0.1 does not.

## The files of the record

% src: the record's README.md (the paragraph under the table of files, "Remaking the cities", "Licences")

| File | What it holds |
|---|---|
| `delft.zip`, `amsterdam.zip`, `lyon.zip`, `paris.zip`, `dotnet.zip` | The five cases, one archive each. |
| `README.md` | The description of the record. |
| `CODEBOOK.md` | Every column of every file of a case, with its unit. |
| `SHA256SUMS` | The checksum of every file of the record and of every file inside the five archives. |
| `make.zip` | The scripts that remake the four cities' folders, each city's recipe inputs, and the list of every changed traveller with its cause. |
| `LICENCE_MOBILITES.pdf` | The text of the « Licence Mobilités », under which the timetables of Paris and Lyon are made available. |

## The files of a case

% src: the record's README.md (the table of files); the record's CODEBOOK.md

Every case is a folder with the same files. Positions are WGS84 longitude and latitude in degrees.

| File | What it holds | Described in |
|---|---|---|
| `nodes.parquet` | The network's nodes: an identifier and a position. | [The nodes table](inputs.md#the-nodes-table) |
| `links.parquet` | The network's links, each for one mode (car, bike or walk): its two ends, its length, and for a car link its lanes, free-flow speed and capacity; for drawing, its shape and road class. | [The links table](inputs.md#the-links-table) |
| `gtfs.zip` | The timetable: a GTFS feed of one service day. The cities' feeds hold the runs from 05:00 to 12:00. | [A GTFS feed](inputs.md#a-gtfs-feed) |
| `trips.parquet` | One row per traveller: the two ends, the departure time, the traveller class and the modes the traveller may use. | [The trips table](inputs.md#the-trips-table) |
| `run.toml` | The settings `run_file` reads: the window, the files, every parameter written out with its unit, and each class's choice coefficients. | [The settings file](record.md) |
| `LICENCE.md` | The sources of the case's files and their licences. `lyon` and `paris` also hold `LICENCE_MOBILITES.pdf`. | [Licences](#licences) |

### The columns

% src: the record's CODEBOOK.md

| File | Column | Type | Unit | Meaning |
|---|---|---|---|---|
| `nodes.parquet` | `node_id` | text | none | The node's identifier, unique in the case. |
| `nodes.parquet` | `lon_deg` | real | degree | Longitude. |
| `nodes.parquet` | `lat_deg` | real | degree | Latitude. |
| `links.parquet` | `link_id` | text | none | The link's identifier, unique in the case: the mode, a colon and the source's own identifier. |
| `links.parquet` | `upstream` | text | none | The `node_id` where the link starts. |
| `links.parquet` | `downstream` | text | none | The `node_id` where the link ends. |
| `links.parquet` | `modes` | text | none | The mode the link serves: `car`, `bike` or `walk`. |
| `links.parquet` | `length_m` | real | m | The link's length along its shape. |
| `links.parquet` | `lanes` | integer | lanes | A car link's lanes in its direction; empty for bike and walk links. |
| `links.parquet` | `car_speed_km_h` | real | km/h | A car link's free-flow speed; empty for bike and walk links. |
| `links.parquet` | `capacity_veh_h` | real | vehicles per hour | A car link's capacity over all its lanes; empty for bike and walk links. |
| `links.parquet` | `shape` | text | degree | The points between the link's two ends, in its direction, as `lon lat;lon lat`; empty: a straight line. For drawing only. |
| `links.parquet` | `road_class` | text | none | OpenStreetMap's `highway` class of the link's road; DOTNET's links take the same names. For drawing only. |
| `trips.parquet` | `trip_id` | text | none | The traveller's identifier, unique in the case. |
| `trips.parquet` | `origin_lon_deg`, `origin_lat_deg` | real | degree | Where the trip starts. |
| `trips.parquet` | `destination_lon_deg`, `destination_lat_deg` | real | degree | Where the trip ends. |
| `trips.parquet` | `departure_s` | integer | s | The departure time, seconds after midnight of the service day (27 000 is 07:30). |
| `trips.parquet` | `class_id` | text | none | The traveller class; its choice coefficients are in `run.toml`. |
| `trips.parquet` | `modes` | text | none | The modes the traveller may use, separated by `;`. |

A link is travelled from its upstream end to its downstream end; a two-way street is two links. Each link serves one
mode: a street open to cars, bikes and walkers is a car link, a bike link and a walk link.

### The traveller classes

% src: the record's CODEBOOK.md (the classes of the four cities); the record's README.md ("How the cities' cases were made": Choice)

| `class_id` | Who | `modes` | In |
|---|---|---|---|
| `car_and_bike` | residents with a car and a bike | `car;bike;walk;transit` | every case |
| `car_only` | residents with a car and no bike | `car;walk;transit` | every case |
| `bike_only` | residents with a bike and no car | `bike;walk;transit` | every case |
| `no_vehicle` | residents with neither | `walk;transit` | every case |
| `from_outside_car` | cars entering, leaving or crossing the study area at a road on its edge | `car` | the four cities |
| `from_outside_transit` | transit riders arriving at a rail station of the area | `transit`, or `transit;bike` for those given a station bike | the four cities |
| `goods` | goods vehicles | `car` | the four cities |

The classes differ in the modes each may use. Every class of every case takes the default choice coefficients of
openmobisim 0.2, written out in each `run.toml` ([the coefficients of a class](record.md#the-coefficients-of-a-class)).

**For bikes.** Two of the four resident classes own a bike, and bike links are a layer of every network. Some
travellers were given a bike so that they arrive within the window (below).

## How the cities' cases were made

% src: the record's README.md ("How the cities' cases were made", "Remaking the cities")

The four cities' cases were made from version 1 of the record (doi:10.5281/zenodo.23472146) by the scripts in
`make.zip`, which remake the four folders byte for byte.

| Part | How it was made |
|---|---|
| Networks | The road, bike and walk networks that openmobisim 0.1 read from the OpenStreetMap extracts of version 1 of the record, written as tables. |
| Car capacities | A car link's capacity is its lanes times a value per lane by its road class, in the table below. |
| Timetables | Those of version 1 of the record, unchanged. |
| Trips | The morning recipe of version 1 of the record: zones with their residents and built volume from the Global Human Settlement Layer, a gravity model fitted to the surveys' trip lengths, the cars and transit riders that enter from outside, goods vehicles, and seven traveller classes. The trips were drawn by openmobisim 0.1.0 with one change: each traveller's departure is drawn evenly over 06:30 to 09:30, and the travellers leaving between 07:30 and 08:30, about a third of the morning, are kept. |
| Choice | The default choice coefficients of openmobisim 0.2. The mode constants of version 1 of the record, fitted to the surveys' shares, are not carried over. |

The capacity per lane of the cities' car links, by road class:

| Road class | Capacity per lane (vehicles per hour) |
|---|---|
| motorway, motorway ramp | 2 100 |
| trunk | 1 800 |
| trunk ramp; a motorway ramp ending where another kind of road begins | 1 550 |
| primary | 1 080 |
| primary ramp, secondary, secondary ramp | 900 |
| tertiary | 810 |
| tertiary ramp, unclassified, road | 720 |
| residential | 600 |
| living street, service | 450 |

The record gives the source of the motorway value as HCIA v3 (Rijkswaterstaat 2011, Table 3.1). These capacities are
the cases' own: they are in the cases' `links.parquet`, and they differ on motorways (2 100 against 2 000 vehicles per hour per lane) and on
motorway ramps (2 100 against 1 550 vehicles per hour per lane) from the values openmobisim 0.2 gives a network it reads
from an OpenStreetMap extract
([Input tables and files](inputs.md#an-openstreetmap-extract)).

What was changed per city, for the three cities the record lists changes for:

| City | Factor on the recipe's travellers (none) | Other changes |
|---|---|---|
| Amsterdam | 1.2 | Two blocks count at a tenth of their residents and built volume: Westpoort's port and industrial area, and Weesp's western business park. 9 of the 39 gateways where cars enter or leave were moved 10 to 760 m onto the carriageway of their own direction. |
| Lyon | 0.7 | 15 of the 66 gateways moved. Each trip end at a 500 m block's point goes to that point or one of six around it at 200 m, drawn evenly. |
| Paris | 0.5 | 23 of the 86 gateways moved. Trip ends spread as for Lyon. |

The factors of Lyon and Paris were chosen for low congestion and short runs. A gateway is a road on the edge of the
study area where cars enter or leave.

**Every traveller arrives by 09:30.** A traveller who could not arrive by 09:30 was given a bike (a resident without a
vehicle, or a train passenger a station bike); one who still could not had the destination moved to the nearest place
a bike reaches in time; two leave earlier.

| Case | Travellers changed so |
|---|---|
| `delft` | 110 |
| `amsterdam` | 1 734 |
| `lyon` | 311 |
| `paris` | 18 |

`make.zip` lists every change with its cause.

## DOTNET

% src: the record's README.md ("DOTNET")

DOTNET is a synthetic city of 8 km by 8 km on round numbers, made for teaching network design and for testing.

| Part | What it holds |
|---|---|
| Network | 41 nodes and 136 links per mode: a ring road at 60 km/h, outer roads at 50 km/h, core streets at 30 km/h. |
| Timetable | 7 transit lines: two metro lines, a tram and four buses. |
| Travellers | 4 traveller classes and an origin-destination matrix of the morning peak. |

Car capacities (each direction) and the matrix's cells are varied at random by up to 15 % and 30 % (seeded), so that
routes of equal cost are rare once the roads load; lengths and speeds stay round. Travellers without a vehicle whose
only alternative was a walk of over an hour were given a bike (the four corner nodes have no stop within 1 500 m).
Unlike the cities' roads, DOTNET's roads are congested at the peak. Its generator is not part of the record: the
files are its source.

## Licences

% src: the record's README.md ("Licences"); each case's LICENCE.md

Each part keeps its own licence.

| What | Source | Licence |
|---|---|---|
| Networks of the four cities | OpenStreetMap, as read by openmobisim 0.1 | ODbL 1.0, © OpenStreetMap contributors |
| Timetables of Delft and Amsterdam | the Dutch national GTFS feed, published by OVapi from the NDOV data of the Dutch transit agencies, which do not endorse this use | CC0 1.0 |
| Timetable of Paris | Île-de-France Mobilités' GTFS Datahub | Licence Mobilités, February 2021 |
| Timetable of Lyon | SYTRAL Mobilités' TCL timetable, via data.gouv.fr | Licence Mobilités, February 2021 |
| Zones' residents and built volume, in the trips | the Global Human Settlement Layer, European Commission's Joint Research Centre | CC BY 4.0, © European Union, 1995–2026 |
| The Dutch motorway counts, in the trips of Delft and Amsterdam | Rijkswaterstaat, INWEVA 2025, morning peak | CC0 1.0 |
| The trips, the classes, the settings, DOTNET and the scripts | made for the record | CC BY 4.0 |

By case, as each case's `LICENCE.md` states:

| Case | Network | Timetable | Trips | Settings |
|---|---|---|---|---|
| `delft` | ODbL 1.0 | CC0 1.0 | CC BY 4.0, from sources under CC BY 4.0 and CC0 1.0 | CC BY 4.0 |
| `amsterdam` | ODbL 1.0 | CC0 1.0 | CC BY 4.0, from sources under CC BY 4.0 and CC0 1.0 | CC BY 4.0 |
| `lyon` | ODbL 1.0 | Licence Mobilités, February 2021 | CC BY 4.0, from a source under CC BY 4.0 | CC BY 4.0 |
| `paris` | ODbL 1.0 | Licence Mobilités, February 2021 | CC BY 4.0, from a source under CC BY 4.0 | CC BY 4.0 |
| `dotnet` | CC BY 4.0 | CC BY 4.0 | CC BY 4.0 | CC BY 4.0 |

Paris's case contains information from Île-de-France Mobilités' GTFS Datahub, and Lyon's from the TCL timetable, made
available under the « Licence Mobilités »; its text, `LICENCE_MOBILITES.pdf`, is in the record and in each of the two
cases' archives.

## Versions

% src: the record's README.md ("Versions and citation"); each case's LICENCE.md (the DOI of every version)

This page describes **version 2** of the record, doi:10.5281/zenodo.24585967, read by openmobisim 0.2. The DOI
10.5281/zenodo.23472145 names every version of the record. Version 2 changes from version 1
(doi:10.5281/zenodo.23472146):

- The cases are ready-to-run folders for `run_file`, not OpenStreetMap extracts and a recipe; the networks, timetables
  and recipe are those of version 1, with the changes above.
- New: `dotnet`.
- Not carried over: the Sioux Falls and Nguyen–Dupuis benchmarks, the OpenStreetMap extracts, the networks as GMNS
  and GeoParquet tables, and the notebook comparing the mornings with the surveys. They stay in version 1, which
  openmobisim 0.1 keeps reading.

A version of the record is never edited; a correction is a new version. Cite the version you used.
