# openmobisim

**A simulator of multimodal dynamic traffic assignment: cars, bikes, walkers and scheduled transit.**

Individual travellers choose a mode and a route. Cars are loaded on a point-queue network, bikes and walkers travel
at free flow, and transit runs on its timetable. Choice and loading are iterated to a stable state. The core is
written in Rust; it is used from Python. Every module was admitted with a test against an answer from outside the
simulator (theory, a hand derivation, a published result or another tool); the
[code repository](https://github.com/bahmanmdd/openmobisim) lists the ten tests. Bikes are a mode of every module,
table and page. All units are metric.

```{figure} _static/figures/delft_map.png
:alt: The map of a simulated morning in Delft, one ribbon per direction of each road, wider where the road's capacity is higher and coloured by its volume over its capacity
:width: 100%

Delft's morning from 07:30 to 09:30, as the run's own map page shows it: one ribbon per direction of a road, its
width the road's capacity, its colour the cars' volume over that capacity in the busiest quarter hour. The network is
from OpenStreetMap (© OpenStreetMap contributors). [The first worked example](examples/delft.ipynb) makes this page.
```

## Install

```bash
pip install openmobisim
```

Python 3.11 or newer, on Linux, macOS or Windows. The package has no Python dependency. A run writes its results as
Parquet tables; the examples of this manual read them with `pyarrow` (`pip install pyarrow`), and any other reader of
Parquet does as well.

## A first run

A case is a folder: the network as two tables, a timetable as a GTFS feed, one row per traveller, and a settings
file, `run.toml`. Five cases are ready to download ([the cases](examples/index.md)). With Delft's folder unpacked
beside the script:

```python
import openmobisim as oms

run = oms.run_file("delft", out="delft-run")   # runs the settings file delft/run.toml
run.map("delft-map.html")                      # the interactive map, one HTML page
run.summary("delft-summary.html")              # the run's numbers, one HTML page
```

The run writes five files into `delft-run`: `trips.parquet` (one row per traveller: the mode taken and the
door-to-door time), `links.parquet` (cars, bikes and walkers per link and time bin), `iterations.parquet` (one row per
iteration), `routes.parquet` (what each traveller took, leg by leg) and `run_record.toml` (the settings the run ran
with, then what it found).

## This manual

- **[Worked examples](examples/index.md).** The five cases and where to download them, then two notebooks:
  [Delft](examples/delft.ipynb), a first run and how to read its results, and
  [Amsterdam](examples/amsterdam.ipynb), a case built step by step in Python and run a second time with faster bikes.
- **[Parameters](parameters.md).** Every parameter of a run: its unit, its default, the source of that default and
  the test that reads it. A parameter that is not in the table does not exist.
- **[Contracts](contracts/index.md).** The simulator is a chain of modules. Each has a contract: its inputs, its
  outputs, their units, its assumptions and what it means for bikes. The contract of the Python module lists every
  function and every column of the input tables; the contract of the output files lists every column a run writes.

## Cite and licence

openmobisim is developed by [Bahman Madadi](https://bahmanmadadi.com) and licensed under Apache-2.0. To cite it, use
the repository's `CITATION.cff` and name the version you used; each run's record holds the fingerprints of its
inputs.

```{toctree}
:hidden:
:maxdepth: 2

examples/index
parameters
contracts/index
```
