# Starter cases and the data bundle

openmobisim comes with starter cases: real cities and two classic benchmark networks, ready to run. Their data is
kept in a **data bundle**, outside the package, which is downloaded once and checked against checksums the package
carries.

[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.23472146.svg)](https://doi.org/10.5281/zenodo.23472146)

Its version 1 is a dataset on Zenodo, *openmobisim starter cases: multimodal networks and synthetic demand for four European cities and two benchmark networks*, <https://doi.org/10.5281/zenodo.23472146>, with the notebook
`starter_cases.ipynb` and each city's networks as tables; cite it when you use the cases ([How to cite](citing.md)).

## The cases

% src: python/openmobisim/data/bundle_v1.json; each case's case.json in bundle v1; _cases.py
| Case | Kind | Transit day | People per simulated traveller (the case's default) | Scenarios | Archive |
|---|---|---|---|---|---|
| `amsterdam` | city (OpenStreetMap, GTFS) | 2026-10-09 | 5 | `base` | 59.7 MB |
| `delft` | city (OpenStreetMap, GTFS) | 2026-10-09 | 1 (every traveller) | `base` | 18.5 MB |
| `lyon` | city (OpenStreetMap, GTFS) | 2026-11-20 (the feed, published 2026-09-23 and valid to 2027-08-28, has its busiest weekday there) | 5 | `base` | 36.8 MB |
| `paris` | city (OpenStreetMap, GTFS) | 2026-10-16 | 10 | `base` | 91.4 MB |
| `siouxfalls` | benchmark (link table, OD matrix, cars) | — | 1 | `base`, `congested` | < 0.1 MB |
| `nguyendupuis` | benchmark (link table, OD matrix, cars) | — | 1 | `base` | < 0.1 MB |

Each city case holds:
- its OpenStreetMap extract, clipped to the study area;
- a GTFS feed cut to one service day;
- a morning's demand as zones, the points within them where trips start and end, and an origin–destination matrix
  (`demand/zones.csv`, `demand/zone_points.csv`, `demand/od.csv`);
- traveller classes (`demand/classes.csv`);
- the motorway counts used to size through traffic (`demand/gateway_counts.csv`);
- a description of how the morning was made (`demand/morning.json`);
- `case.json`: the region, the sources, the licences, and the settings of each scenario.

The French cities carry their transit data's licence (`LICENCE_MOBILITES.pdf`). The benchmarks hold a link table and
a node table.

## Using a case

% src: _cases.py case, Case.network/transit/parkings/zones/classes/demand/scenario; __main__.py
```python
case = ms.examples.case("delft")
scenario = case.scenario("base")             # the case's settings, the demand drawn from its matrix
run = scenario.run("delft")
```

`ms.examples.case(name)` looks for the case in three places, in order:
1. the folder you give as `root`;
2. the folder `OPENMOBISIM_DATA` names;
3. the cache, `~/.cache/openmobisim/bundle/<version>`, where a case is downloaded the first time.

Every file is checked against its checksum; a changed or broken file is refused. A case gives its parts one by one
(`network()`, `transit()`, `parkings()`, `zones()`, `classes()`, `demand()`) or as a ready `scenario()`, and its
`info` is its `case.json`.

`case.demand(weight=…)` draws the trips from the matrix. Pass `weight=1` for one simulated traveller per person, the
complete run, which is the slowest.

From the command line: `openmobisim cases`, `openmobisim run <case>`, `openmobisim check`. See
[The command line](../guides/command_line.md).

## Reference results

% src: python/openmobisim/data/reference_v1.json; _cases.py case_check, case_summary, CHECK_TOLERANCE
The package carries each scenario's reference results:
- its fingerprint;
- trips completed;
- mean trip time;
- the modes' shares.

`openmobisim check` compares a new run with them. On the platform that made the reference, the numbers must be
identical; on another, within tolerances (see [Reproducibility](../concepts/reproducibility.md#across-platforms)).

## How the data was made

% src: the recipe's scripts (cases/); each case's case.json and demand/morning.json
Each city is cut from public data and given a synthetic morning by a recipe whose scripts are published with
openmobisim, in its repository's `cases/` folder. Every number of the recipe and its source is in the case's
`demand/morning.json`; every input with its date and licence in its `case.json`.

| What | Source | Retrieved | Licence |
|---|---|---|---|
| Delft's road, bike and walk networks and parkings | OpenStreetMap, Geofabrik's Zuid-Holland extract, <https://download.geofabrik.de/europe/netherlands/> | 2026-10-11 | ODbL 1.0 |
| Amsterdam's | OpenStreetMap, Geofabrik's Noord-Holland extract | 2026-09-24 | ODbL 1.0 |
| Paris's and Lyon's | OpenStreetMap, BBBike's city extracts, <https://download.bbbike.org/osm/bbbike/> | 2026-09-26 | ODbL 1.0 |
| Delft's and Amsterdam's timetables | the Dutch national GTFS feed, <http://gtfs.ovapi.nl/> | 2026-09-28 | CC0 1.0 |
| Paris's timetable | Île-de-France Mobilités' GTFS feed | 2026-09-30 | Licence Mobilités |
| Lyon's timetable | the TCL timetable, via <https://www.data.gouv.fr/> | 2026-09-30 | Licence Mobilités |
| Residents and non-residential built volume | the Global Human Settlement Layer R2023A, epoch 2020, <https://human-settlement.emergency.copernicus.eu/> | 2026-10-11 | CC BY 4.0 |
| The Dutch motorway counts | Rijkswaterstaat, INWEVA 2025, morning peak | 2026-10-12 | CC0 1.0 |

Each timetable is cut to one service day, a Friday, and to the runs that call in the study area between 05:00 and
12:00.

**The morning** is made in four steps:
1. **Zones** are the cells of a 1 km grid, each with its residents and its non-residential built volume. Each zone is
   split into 500 m blocks, and trips start where residents live and end where the built volume and residents are,
   never at a zone's centre.
2. **Trips inside the city** are the travel survey's trips a day, times the morning's share of the day, times the study
   area's share of the survey's residents. A gravity model spreads them between zones, its distance decay fitted so
   that the mean straight-line trip meets a target: Paris 2.8 km and Amsterdam 4.5 km from their surveys, Lyon 3.0 km
   and Delft 2.0 km assumed.
3. **What enters from outside** follows the survey's trips between the city and its surroundings. Cars enter at the
   roads that cross the study area's edge; a motorway's flow is Rijkswaterstaat's count in the Dutch cities and an
   assumed 0.35 of the road's capacity in the French ones. Transit riders start at the rail stations; through traffic
   and goods vehicles are added.
4. **Traveller classes** are four by what people own, a car and a bike, a bike only, a car only, or neither, and three
   with fixed modes for what enters from outside and for goods. A mode constant or two per city is fitted to the
   survey's shares. No prices are charged.

How close the mornings come to their sources, the surveys' mode shares, TomTom's congestion index and the motorway
counts, is shown in the notebook published with the bundle, `starter_cases.ipynb`. In short: the car share sits near
its target, walking falls far short because the recipe lacks most short trips, and congestion sits slightly below
TomTom's.

For tools that do not read OpenStreetMap, the bundle's record also holds each city's networks as GMNS tables and as
GeoParquet, `<case>-tables.zip`; the package does not download them.

The two benchmarks come from their publications: Sioux Falls from TNTP {footcite}`tntp`, its base scenario half of the
published matrix over one hour; Nguyen–Dupuis as tabled in *Sensors* 25(7) 2262, 2025, with ten times the reference
demand over one hour. Nguyen–Dupuis's numbers are under CC BY 4.0, as that article; Sioux Falls is under TNTP's terms, for academic research only and citing the source, and not under the bundle's other licences.

## Limits

The demand is synthetic: a plausible morning built from public totals, for learning and for testing methods. It is
not a calibrated model of any city. Bring your own demand for a study; see
[Bring your own network and demand](../guides/own_data.md).

## References

```{footbibliography}
```
