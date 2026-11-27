# Worked examples

Two notebooks, each run from start to end as shown, on cases anyone can download:

- **[Delft: a first run](delft.ipynb).** Download a case, run it with one command, and read its results: the mode
  shares and door-to-door times, the iterations, the traffic per link for cars, bikes and walkers, one traveller's
  journey leg by leg, the map and the summary page.
- **[Amsterdam: a case built in Python](amsterdam.ipynb).** The same steps written out (the network from tables, the
  timetable from GTFS, the travellers from a table), then a second run with a higher bike speed, compared with the
  first.

```{toctree}
:hidden:

delft
amsterdam
```

## The cases

Five cases are published as one data record, *openmobisim example cases*, version 2
([doi:10.5281/zenodo.24585967](https://doi.org/10.5281/zenodo.24585967)): the morning of four European cities and a
small synthetic city. They are convenient cases to start from. The networks and timetables are real; **the demand is
synthetic**: neither observed nor a forecast, and not calibrated. Use the cases to learn the tool, to compare methods
and settings, and as a realistic load for a network; for a study of a city, bring that city's own trips.

| Case | Study area | Travellers | Archive (MB) |
|---|---|---|---|
| `delft` | the municipality of Delft | 37 649 | 4.5 |
| `amsterdam` | Amsterdam inside and around its A10 ring road | 198 252 | 24.5 |
| `lyon` | Lyon and Villeurbanne | 117 073 | 19.1 |
| `paris` | Paris inside the Boulevard Périphérique, the ring included | 243 203 | 29.4 |
| `dotnet` | a synthetic city of 8 km by 8 km | 39 919 | 0.4 |

Every case simulates the travellers who leave between 07:30 and 08:30, until 09:30. A case is downloaded and unpacked
by a few lines of plain Python, and then run:

```python
import urllib.request, zipfile
import openmobisim as oms

urllib.request.urlretrieve("https://zenodo.org/records/24585967/files/delft.zip", "delft.zip")
zipfile.ZipFile("delft.zip").extractall()          # the folder delft
run = oms.run_file("delft", out="delft-run")
```

Each folder holds `nodes.parquet` and `links.parquet` (the network; each link is for one mode: car, bike or walk),
`gtfs.zip` (the timetable), `trips.parquet` (one row per traveller), `run.toml` (the settings, every parameter written
out) and `LICENCE.md` (the sources and their licences). The record's `README.md` says how the cases were made, and its
`CODEBOOK.md` gives every column with its unit.

## A case of your own

A run needs a network, travellers and, for transit, a timetable. Each has one entry point; the
[contract of the Python module](../contracts/binding.md) gives every argument and every column.

| Input | Entry point | Read from |
|---|---|---|
| Network | `Network.from_tables(nodes, links)` | two tables, in memory or as Parquet files |
| Network | `Network.from_osm(path, region=None)` | an OpenStreetMap extract (`.osm.pbf`); see [its contract](../contracts/io-osm.md) |
| Timetable | `Transit.from_gtfs(path, network)` | a GTFS feed (`.zip`) |
| Travellers | `Demand.from_table(trips, classes, network, transit=None)` | a table, in memory or as a Parquet file |

`run(network, demand, transit=None, window=(start_s, end_s), out=folder)` runs them, and `run_file(folder, out=...)`
runs the same from a settings file, `run.toml`. `Run.open(folder)` opens a finished run without running anything.
Two helpers are experimental: `Network.from_osm(..., car_roads="main")` gives cars the main roads alone, and
`make_case(folder, osm, population)` writes a case folder from an extract.
