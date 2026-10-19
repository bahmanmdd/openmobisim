# Scripts that rebuild the starter cases

These scripts rebuild the four city cases of the starter bundle (Delft, Amsterdam, Paris and Lyon) from public
downloads: they fetch the raw files, clip them to the study area, make the morning's demand, and pack the bundle. The
demand is **synthetic, not a forecast**: it is for starter cases and benchmarks. Every number in it comes from a source
named in `morning2.py` (with its URL) or is stated there as an assumption.

The scripts are deterministic: on the same inputs and the same openmobisim release they write the same files.

## What is in this folder

| File | What it is |
|---|---|
| `cases.py` | The cases of the collection: what each is, where its raw files come from (URL, provider, date, licence), its study areas. Every other script reads it. Also defines the data root (below). |
| `regions.py` | The study areas (polygons and boxes) and the test networks' table. `cases.py` reads it. |
| `fetch.py` | Downloads the raw files listed in `cases.py` that are missing. |
| `bundle.py` | The bundle: finds the service day, clips the OpenStreetMap extract and the timetable to the study area, checks the clip, packs the cases, and writes the network as tables. |
| `ghsl.py` | Cuts each city's residents and non-residential building volume out of the Global Human Settlement Layer (GHSL) grids. |
| `morning2.py` | Makes the morning's demand: zones, the origin-destination matrix, trips from outside the study area, goods vehicles, the traveller classes. |

## What they need

- Python 3.11 or newer, with `openmobisim` and `numpy` installed. `morning2.py` uses a helper of openmobisim's
  internals (`openmobisim._cases`), so use the openmobisim release these scripts were published with.
- `osmium` (pyosmium, `pip install osmium`) for `bundle.py clip`.
- `tifffile` and `imagecodecs` (`pip install tifffile imagecodecs`) for `ghsl.py`.
- `pyarrow` for `bundle.py tables`.

## The data root

Every script finds its files under one folder, the **data root**: the folder named by the environment variable
`OPENMOBISIM_RECIPE_DATA`, or, if the variable is not set, a folder `data` beside the scripts.

```
OPENMOBISIM_RECIPE_DATA/
  cases/<case>/raw/              the files as downloaded, and ghsl_2020.npz (made by ghsl.py)
  cases/<case>/processed/        tables derived from the raw files (the scripts here do not read them)
  cases/<case>/demand/morning2/  the morning's demand (morning2.py writes here unless given --out)
  bundle/v1/<case>/              case.json, network.osm.pbf, gtfs.zip, clip.json and demand/
  bundle/v1/<case>.zip           each case's archive, with MANIFEST.json beside them (bundle.py pack)
  sources/ghsl/                  the GHSL tiles, as downloaded (zip files)
  sources/tmja/tmja-2019.csv     France's national road counts; read only for a city set to the counts "tmja"
                                 (none of the four is)
  licences/                      licence_mobilites_2021-02.pdf, the text of the Licence Mobilités, packed with Paris and Lyon
  MANIFEST.csv                   optional: columns `path` and `sha256`; fetch.py says when a download differs from it
```

`<case>` is `delft`, `amsterdam`, `paris` or `lyon`.

## In which order

Name the cases on the command line. Each script keeps its command line as in its first docstring; the table shows the
order and what each step reads and writes (paths are under the data root).

| Step | Command | Needs besides openmobisim | Reads | Writes |
|---|---|---|---|---|
| 1. Fetch | `python fetch.py delft` | internet | the URLs in `cases.py` | `cases/<case>/raw/`: the OpenStreetMap extract, the timetable feed, and for Delft and Amsterdam the Rijkswaterstaat counts |
| 2. Service day | `python bundle.py date delft` | none | `cases/<case>/raw/` | `bundle/v1/<case>/case.json`: the busiest weekday of the full feed and the study area |
| 3. Clip | `python bundle.py clip delft` | `osmium` | `cases/<case>/raw/`, `case.json` | `bundle/v1/<case>/network.osm.pbf`, `gtfs.zip`, `clip.json` |
| 4. Check | `python bundle.py check delft` | none | the raw files and the clipped ones | a `checked` entry in `case.json`; stops with an error if the clipped files read differently from the full ones |
| 5. Zones from the GHSL | `python ghsl.py delft` | `tifffile`, `imagecodecs` | `sources/ghsl/`, `case.json` | `cases/<case>/raw/ghsl_2020.npz` |
| 6. The morning | `python morning2.py delft` | none | `case.json`, `network.osm.pbf`, `gtfs.zip`, `demand/classes.csv` of the case's bundle folder, `ghsl_2020.npz`, and for Delft and Amsterdam the counts in `raw/` | `cases/<case>/demand/morning2/`: `zones.csv`, `zone_points.csv`, `od.csv`, `classes.csv`, `gateway_counts.csv`, `morning.json` |
| 7. Pack | `python bundle.py pack delft amsterdam paris lyon` | none | the morning's files, the clipped files, the licence file for Paris and Lyon | the demand files and the sources and licences in each `case.json`, `<case>.zip`, `MANIFEST.json` |
| 8. Tables | `python bundle.py tables delft` | `pyarrow` | the packed case | `bundle/v1/<case>-tables.zip`: the network as GMNS tables and GeoParquet |

Notes:

- **Step 2 comes before step 5**: `ghsl.py` reads the study area from `case.json`.
- **Step 6 needs a `demand/classes.csv`** in the case's bundle folder: the traveller classes (columns `class`, `share`,
  `modes`, `owns_car`, `owns_bike`, `has_transit_pass`, `beta_mode_bike`). Take it from the published starter bundle.
  A city without its own file takes Amsterdam's, so Delft needs Amsterdam's in place. The script adds the rows for
  travellers from outside and goods vehicles itself.
- **`fetch.py` with no case downloads every case listed in `cases.py`**, including large extracts of cities that are
  not starter cases: name the cases.
- **`fetch.py` does not download** the GHSL tiles or the French road counts. The tiles' addresses are at the head of
  `ghsl.py`; the French counts are not needed by the four cities.
- **`bundle.py pack` without case names** also packs Sioux Falls and Nguyen–Dupuis, which are made from tables that
  these scripts do not produce: name the four cities.
- **`morning2.py` options**: `--model radiation|gravity` (the default is each city's own choice), `--out DIR` (where to
  write; the default is `cases/<case>/demand/morning2`), `--transfer-ratio` (for tests).

## Terms

- **Study area**: the polygon or box of a case; the clip keeps it widened by 3 km (by 300 m on Delft's south side).
- **GTFS**: the standard format of public transport timetables; a **feed** is one such file.
- **GHSL**: the Global Human Settlement Layer of the European Commission's Joint Research Centre; its grids give
  residents and building volume per cell of 3 arc-seconds (about 60 × 90 m here).
- **Zone**: a 1 km grid cell of the study area with at least five drivable nodes; the demand is made between zones.
- **Gateway**: a motorway, trunk, primary or secondary road that crosses the study area's edge, where traffic from
  outside enters and leaves.

## Sources and licences

The licences are those the bundle's `case.json` states for each source; the row on France's road counts, which no
`case.json` of the four cities lists, is from `morning2.py`.

| What | Source | Licence |
|---|---|---|
| Road, bike and walk networks, parkings | OpenStreetMap, from Geofabrik's and BBBike's extracts | ODbL 1.0: © OpenStreetMap contributors (https://www.openstreetmap.org/copyright) |
| Timetable of Delft and Amsterdam | The Dutch national GTFS feed (gtfs.ovapi.nl, published by OVapi from the NDOV data of the Dutch transit agencies; the agencies do not endorse this use) | CC0 1.0 (OVapi) |
| Timetable of Paris | Île-de-France Mobilités' GTFS feed, "Horaires prévus sur les lignes de transport en commun d'Ile-de-France (GTFS Datahub)" (data.iledefrance-mobilites.fr), one service day, clipped | Licence Mobilités (February 2021, https://cloud.fabmob.io/s/eYWWJBdM3fQiFNm): contains information from Île-de-France Mobilités' GTFS Datahub, made available under the « Licence Mobilités » |
| Timetable of Lyon | SYTRAL Mobilités' TCL timetable, "Horaires théoriques du réseau Transports en Commun Lyonnais" (data.gouv.fr dataset 5de42c4d8b4c417a10e62ec9, its "GTFS modifié" resource), one service day, clipped | Licence Mobilités (February 2021, https://cloud.fabmob.io/s/eYWWJBdM3fQiFNm), as SYTRAL Mobilités states it: contains information from the TCL timetable, made available under the « Licence Mobilités » |
| Residents and non-residential built volume of each zone, and the points within a zone where trips start and end | GHSL R2023A, epoch 2020: GHS-POP and GHS-BUILT-V NRES at 3 arc-seconds (European Commission, Joint Research Centre; https://human-settlement.emergency.copernicus.eu/) | CC BY 4.0, © European Union, 1995-2026 (https://creativecommons.org/licenses/by/4.0/); changed: summed by 1 km zone |
| Motorway gateways' morning counts (Delft, Amsterdam) | Rijkswaterstaat, INWEVA 2025, morning peak (https://geo.rijkswaterstaat.nl/services/ogc/gdr/inweva/ows, layer ochtendspits_intensiteiten_2025) | CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/) |
| Motorway gateways' morning flows (Paris, Lyon) | Synthetic: 0.35 of each carriageway's capacity, an assumption tuned so that the morning's congestion stays slightly below TomTom's; from Rijkswaterstaat's INWEVA 2025 counts at Amsterdam's gateways | CC BY 4.0 |
| Annual average daily traffic of France's national roads (optional, not used by the four cities) | Ministère de la Transition écologique, TMJA 2019 (https://www.data.gouv.fr/datasets/trafic-moyen-journalier-annuel-sur-le-reseau-routier-national) | Licence Ouverte 2.0 |
| The morning's demand (zones, matrix, trips from outside, goods vehicles) and the traveller classes | Synthetic: these scripts' second morning recipe; totals from each city's travel survey (quoted in `morning2.py`) | CC BY 4.0 |

## Licence of the scripts

The scripts in this folder are released under the **Creative Commons Attribution 4.0 International licence (CC BY
4.0)**, https://creativecommons.org/licenses/by/4.0/, as the demand they make. The data they download and read keeps
the licences above.
