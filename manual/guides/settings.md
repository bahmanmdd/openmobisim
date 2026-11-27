# The settings file

% src: crates/binding/README.md ("Two files with fixed names", `run_file`); crates/io-parquet/README.md (the record)

A run can be described in one text file, `run.toml`, and run with one call, `run_file`. The file names the input
files, the window, the traveller classes and any parameter that should differ from its default. It is written in
[TOML](https://toml.io). Every published case holds one ([The cases](cases.md)).

`run.toml` is always written by the user and read by `run_file`. A run never writes it: what a run writes is its
record, `run_record.toml`, whose first part has the same form ([below](#a-records-first-part-is-a-settings-file)).

## A minimal file

% src: crates/binding/README.md (`run_file`: "Minimal settings hold the window, the files and the classes alone");
% run by the writer on the published case dotnet

The window, the files and the classes are enough. This file, saved as `my-case/run.toml` beside the folder `dotnet`
of the published cases, runs that case:

```toml
window_start_s = 27000   # 07:30, in seconds after midnight
window_end_s = 34200     # 09:30

[network]
nodes_file = "../dotnet/nodes.parquet"
links_file = "../dotnet/links.parquet"

[demand]
trips_file = "../dotnet/trips.parquet"
classes = ["bike_only", "car_and_bike", "car_only", "no_vehicle"]

[transit]
gtfs_file = "../dotnet/gtfs.zip"
```

```python
import pathlib, tomllib
import openmobisim as oms

run = oms.run_file("my-case", out="my-run")
record = tomllib.loads(pathlib.Path(run.record).read_text())
print(record["parameters"]["iteration_cap"], record["result"]["iterations"], record["result"]["stopped_by_rule"])
```

```text
50 17 True
```

Every parameter the file leaves out took its default, and the record wrote each at the value used: the cap on the
iterations is 50, the run made 17 and the stopping rule ended it. `run_file` takes the folder holding `run.toml`, or
the path of the file itself, or nothing (the current folder). `out` is the folder the results go to; it is required.

## The keys

% src: crates/binding/README.md (`run_file`); crates/io-parquet/README.md (`run_record.toml`, part one)

| Key | Required | Meaning |
|---|---|---|
| `format` | no | the file's format version; only 1 |
| `window_start_s`, `window_end_s` | yes | the window, in seconds after midnight ([The window](window.md)) |
| `[parameters]` | no | any row of [Parameters](../reference/parameters.md) by its name; a row left out takes its default |
| `[parameters.coefficients."<class id>"]` | no | one table per class: the sixteen choice coefficients by name, all sixteen ([Traveller classes](classes.md)) |
| `[network] nodes_file`, `links_file` | one of the two forms | the network as two Parquet tables |
| `[network] osm_file`, `region` | one of the two forms | the network as an OpenStreetMap extract, cut to `region` when given: `[west, south, east, north]` or a list of `[lon, lat]` points, in degrees |
| `[demand] trips_file` | yes | the trips table, a Parquet file |
| `[demand] classes` | yes | the list of traveller class ids |
| `[transit] gtfs_file` | when `[transit]` is there | the timetable, a GTFS `.zip`; without the table `[transit]` the run has no public transport |

A file's path is read relative to the folder of the settings file. One more key under `[network]`,
`car_roads = "main"`, belongs to an experimental helper ([A case from your own data](own_data.md)).

## Changing a parameter

% src: crates/binding/README.md (`run_file`); PARAMETERS.md (`iteration_cap`); run by the writer

A parameter is changed by writing it under `[parameters]`. Adding these two lines to the file above, before
`[network]`, caps the run at 5 iterations:

```toml
[parameters]
iteration_cap = 5
```

```python
run = oms.run_file("my-case", out="my-run-5")
record = tomllib.loads(pathlib.Path(run.record).read_text())
print(record["parameters"]["iteration_cap"], record["result"]["iterations"], record["result"]["stopped_by_rule"])
```

```text
5 5 False
```

Three lines of the record changed: `iteration_cap` under `[parameters]`, and under `[result]` the number of
iterations and `stopped_by_rule`, now false because the cap ended the run before the stopping rule held. The results
changed with them. The last row of each run's iterations table reads:

| Run | Iterations | Mean trip time of the finished trips (min) | Unfinished trips | Travellers who would change their choice (%) |
|---|---|---|---|---|
| `my-run` (cap 50) | 17 | 18.94 | 0 | 3.4 |
| `my-run-5` (cap 5) | 5 | 19.68 | 256 | 10.5 |

A run stopped this early is far from settled: one traveller in ten would still change, and 256 travellers do not reach
their door within the window. What these columns measure is in [The iteration](../concepts/iteration.md).

## The rules

% src: crates/binding/README.md (`run_file`, "Errors")

- A key under `[parameters]` that is not a parameter is refused by name, before any input is built. A key the file never
  holds elsewhere is ignored, and one warning names all of them.
- `osm_file` together with `nodes_file` or `links_file` is refused: a network comes from one source.
- No value is a boolean. TOML's `true` is never read as the number 1; it is refused, naming the key.
- `stop_walk_max` and `transfer_walk_max` without `[transit]` are ignored, with a warning: nothing reads them.
- A window end that is not a whole number of link bins after the start is moved up to one, and the record counts it
  ([The window](window.md)).
- A file that is not TOML raises `tomllib.TOMLDecodeError`. The file is read with Python's standard `tomllib`.

## A record's first part is a settings file

% src: crates/io-parquet/README.md (`run_record.toml`); crates/binding/README.md (`run_file`: "A record's part one")

Every run writes its record, `run_record.toml`, beside its tables. Part one of the record, every line before
`[result]`, is the settings the run ran with, every default filled in. Copied into a `run.toml`, it runs the same run
again ([Running a run again](reproducibility.md)). The `run.toml` of each published case was written this way, with
every parameter written out.

Since the two files never share a name, settings and results may share a folder: `run_file("my-case", out="my-case")`
writes the record and the tables beside `run.toml` and leaves it as it is.

## For bikes

% src: crates/binding/README.md ("Bikes"); PARAMETERS.md (`bike_speed`)

The one speed of every bike is the parameter `bike_speed` (15 km/h by default), written under `[parameters]`; the
record writes the value the network was built with, and a run from the record's first part gives it to the network
again. The bike's choice coefficients are `bike_ride_min` and `is_bike` in each class's table
([Cycling](cycling.md)).

## See also

- [Parameters](../reference/parameters.md): every parameter with its unit, default and source.
- [The run's record](../reference/record.md): every key of `run_record.toml`.
- [Troubleshooting](troubleshooting.md): the refusals of a settings file.
