# The cases

% src: the published record doi:10.5281/zenodo.24585967 (its README.md and CODEBOOK.md); README.md ("The cases")

Five cases are published with openmobisim, each a folder of plain files that one call runs: the morning of four
European cities and a small synthetic city. They are meant for learning the tool and for comparing methods and
settings. This page shows how to get one, what its folder holds and how to run it.

| Case | Study area | Timetable and its service day | Travellers | Archive (MB) |
|---|---|---|---|---|
| `delft` | the municipality of Delft | the Dutch national feed, Friday 9 October 2026 | 37 649 | 4.5 |
| `amsterdam` | Amsterdam inside and around its A10 ring road | the Dutch national feed, Friday 9 October 2026 | 198 252 | 24.5 |
| `lyon` | Lyon and Villeurbanne | TCL, Friday 20 November 2026 | 117 073 | 19.1 |
| `paris` | Paris inside the Boulevard Périphérique, the ring included | Île-de-France Mobilités, Friday 16 October 2026 | 243 203 | 29.4 |
| `dotnet` | a synthetic city of 8 km by 8 km | 7 synthetic lines | 39 919 | 0.4 |

Every case simulates the travellers who leave between 07:30 and 08:30, until 09:30.

## Download and unpack one

% src: the published record (README.md); run by the writer

The record is at [doi:10.5281/zenodo.24585967](https://doi.org/10.5281/zenodo.24585967). Each case is one archive,
`<case>.zip`, which unpacks to a folder of the case's name. Python's standard library is enough:

```python
import urllib.request, zipfile

urllib.request.urlretrieve("https://zenodo.org/records/24585967/files/delft.zip", "delft.zip")
zipfile.ZipFile("delft.zip").extractall()          # makes the folder delft
```

The record also holds `SHA256SUMS`, the checksum of every archive and of every file inside the archives, for a reader
who wants to check a download.

## What a case folder holds

% src: the published record (README.md, CODEBOOK.md); the folder delft, read by the writer

| File | What it holds |
|---|---|
| `nodes.parquet` | the network's nodes: an identifier and a position (longitude and latitude, WGS 84) |
| `links.parquet` | the network's links, each for one mode (car, bike or walk): its two ends, its length, and for a car link its lanes, free-flow speed and capacity; for drawing, its shape and road class |
| `gtfs.zip` | the timetable, a GTFS feed of one service day |
| `trips.parquet` | one row per traveller: the two ends, the departure time, the traveller class and the modes the traveller may use |
| `run.toml` | the settings: the window, the files, every parameter and each class's choice coefficients |
| `LICENCE.md` | the sources of the case's files and their licences |

A link is travelled from its upstream end to its downstream end, so a two-way street is two links; and each link of a
case serves one mode, so a street open to cars, bikes and walkers is a car link, a bike link and a walk link. A case's
tables and a run's tables are Parquet files; reading them in Python needs a Parquet reader, for instance pandas with
pyarrow (`pip install pandas pyarrow`): see [Reading the results](results.md). The example counts the links of Delft by
mode:

```python
import pyarrow.parquet as pq

links = pq.read_table("delft/links.parquet")
print(pq.read_table("delft/nodes.parquet").num_rows, "nodes;", links.num_rows, "links")
for row in links.group_by("modes").aggregate([("link_id", "count")]).sort_by("modes").to_pylist():
    print(row["modes"], row["link_id_count"])
print(pq.read_table("delft/trips.parquet").num_rows, "travellers")
```

```text
21566 nodes; 88512 links
bike 20652
car 12460
walk 55400
37649 travellers
```

Every column of the three tables, with its unit, is in [The cases (reference)](../reference/cases.md) and
[Inputs](../reference/inputs.md); the settings file is explained in [The settings file](settings.md).

## Run one

% src: crates/binding/README.md (`run_file`, "Outputs"); run by the writer

`run_file` reads the folder's `run.toml`, builds the network, the timetable and the travellers from the files it names,
runs, and writes the results into the folder given as `out`:

```python
import openmobisim as oms

run = oms.run_file("delft", out="delft-run")
print(run.trips, run.links, run.iterations, run.routes, run.record, sep="\n")
print(run.stopped_by_rule)
```

```text
delft-run/trips.parquet
delft-run/links.parquet
delft-run/iterations.parquet
delft-run/routes.parquet
delft-run/run_record.toml
True
```

The run writes four tables and its record, and returns their paths. `stopped_by_rule` says that the stopping rule, not
the cap on the iterations, ended the run ([The iteration](../concepts/iteration.md)). Reading the tables is in
[Reading the results](results.md), the two pages a run can write in [The map and the summary](pages.md), and the record
in [Running a run again](reproducibility.md). The case folder is not written to: a run never writes `run.toml`, and it
refuses an `out` where one of its result files would replace an input.

On a laptop, Delft's run takes 4.1 s and Paris's 77.2 s ([Run time](speed.md)).

## What the runs give

% src: the published record (README.md, "What a run gives"); Delft's and DOTNET's rows reproduced by the writer

`run_file` on each folder gives these results. Another version of the simulator may give other numbers; the
files of a version of the record never change.

| Case | Car (% of travellers) | Bike (% of travellers) | Walk (% of travellers) | Transit (% of travellers) | Mean car trip (min) | Car delay (% of the cars' time on links) |
|---|---|---|---|---|---|---|
| `delft` | 59.7 | 11.6 | 4.3 | 24.4 | 8.31 | 13.8 |
| `amsterdam` | 47.3 | 21.0 | 3.2 | 28.5 | 11.66 | 6.9 |
| `lyon` | 57.3 | 13.0 | 3.6 | 26.2 | 9.83 | 3.9 |
| `paris` | 25.7 | 24.8 | 3.5 | 46.1 | 9.44 | 1.3 |
| `dotnet` | 41.8 | 30.9 | 0.5 | 26.7 | 14.03 | 43.4 |

The shares are of all travellers, those entering from outside the study area included.

## The demand is synthetic

% src: the published record (README.md: the opening, "How the cities' cases were made", "DOTNET", "What a run gives")

The networks and the timetables of the four cities are real: the networks were read from OpenStreetMap and the
timetables are published GTFS feeds. **The travellers are not.** They are neither observed nor a forecast, and nothing
is calibrated:

- The trips were drawn from zones with their residents and built volume and a gravity model, with each traveller's
  departure drawn evenly over the morning, not peaked.
- Every class takes the simulator's default choice coefficients ([Traveller classes](classes.md)).
- The cases were made so that every traveller arrives by 09:30: a traveller who could not was given a bike, or had the
  destination moved (Delft 110 travellers, Amsterdam 1 734, Lyon 311, Paris 18).
- The cities' congestion is low next to a real morning: the simulator applies no traffic signals, and the departures
  are spread evenly.
- `dotnet` is synthetic throughout (41 nodes, 136 links per mode, 7 transit lines, 4 classes), and unlike the cities
  its roads are congested at the peak.

Use the cases to learn the tool, to compare methods and settings, and as a realistic load for a network. For a study
of a city, bring that city's own trips ([A case from your own data](own_data.md)).

## For bikes

% src: the published record (CODEBOOK.md, the classes; README.md, "How the cities' cases were made")

Bike links are rows of `links.parquet` with `modes` equal to `bike`, 20 652 of them in Delft. Who may cycle is written
per traveller in the `modes` column of `trips.parquet`. In the four cities the classes `car_and_bike` and `bike_only`
may cycle, and so may the transit riders of `from_outside_transit` who were given a station bike; `car_only`,
`no_vehicle`, `from_outside_car` and `goods` may not. `dotnet` has the four resident classes alone: `car_and_bike`, `car_only`,
`bike_only` and `no_vehicle`.

## Licences and citing

% src: the published record (README.md, "Licences"); README.md ("Cite")

The networks are © OpenStreetMap contributors (ODbL 1.0); each timetable keeps its publisher's licence; the trips, the
classes and the settings are CC BY 4.0. Each folder's `LICENCE.md` lists them, and
[The cases (reference)](../reference/cases.md) gives the whole table. If you use the cases in work you publish, cite
the record: Madadi, B. (2026). *openmobisim example cases: multimodal networks and synthetic morning demand for four
European cities and a synthetic test city* (v2). Zenodo. <https://doi.org/10.5281/zenodo.24585967>. See also
[Citing](../reference/citing.md).
