# Running a run again

% src: crates/io-parquet/README.md (`run_record.toml`); crates/binding/README.md (`run_file`, `Run.open`, "Outputs");
% crates/loop/README.md ("Determinism")

Every run writes a **record**, `run_record.toml`, beside its tables: a text file that says what was run. With the
record and the input files, the same run can be made again and gives the same tables, byte for byte. This page shows
how, and what "the same" covers.

## The record

% src: crates/io-parquet/README.md (`run_record.toml`)

The record has two parts.

| Part | Lines | What it holds |
|---|---|---|
| One, the settings | every line before `[result]` | the window given, every parameter at the value used (defaults included), each class's choice coefficients, the input files, the classes |
| Two, the result | `[result]` and the tables under it | the simulator's version, the end of the window used, the number of iterations, whether the stopping rule ended the run, the counts and fingerprints of the network, the demand and the timetable, the readers' counts, every repair |

The record holds no time stamp, no machine name and no thread count, so the same run writes the same record. An input
file's path is written relative to the record's folder. Every key is listed in
[The run's record](../reference/record.md).

A **fingerprint** is a 64-bit hash of what an input was built from, written as 16 hexadecimal digits: one for the
network (its nodes, links and speeds), one for the demand (its classes and trips) and one for the timetable. A change
to any value of a node, a link, a trip or a call changes the fingerprint of that input; a link's shape and road class,
which are for drawing only, are not in it. Two runs of one version of the simulator with the same three fingerprints and the
same part one ran the same thing.

## Running again from the record

% src: crates/binding/README.md (`run_file`: "A record's part one ... is a settings file"); run by the writer on dotnet

Part one of a record is a settings file. Copied into a `run.toml`, it runs the same run again. The example runs the
synthetic city `dotnet` ([The cases](cases.md)), makes a settings file from the record, runs it, and compares the
files:

```python
import filecmp, pathlib, tomllib
import openmobisim as oms

first = oms.run_file("dotnet", out="dotnet-run")

record = pathlib.Path(first.record).read_text()
settings = record.split("\n[result]")[0]            # part one: every line before [result]
pathlib.Path("dotnet-again").mkdir()
pathlib.Path("dotnet-again/run.toml").write_text(settings)

second = oms.run_file("dotnet-again", out="dotnet-again")
for name in ("trips.parquet", "links.parquet", "routes.parquet", "run_record.toml", "iterations.parquet"):
    same = filecmp.cmp(f"dotnet-run/{name}", f"dotnet-again/{name}", shallow=False)
    print(name, "identical" if same else "different")

result = tomllib.loads(record)["result"]
print(result["openmobisim_version"], result["iterations"], result["stopped_by_rule"])
for part in ("network", "demand", "transit"):
    print(part, result[part]["fingerprint"])
```

```text
trips.parquet identical
links.parquet identical
routes.parquet identical
run_record.toml identical
iterations.parquet different
...
network 396cdc682fcf83a4
demand d6b6f37006351472
transit df08a0ecc5f55fb2
```

The trips, the links, the routes and the record are the same bytes. The iterations table differs only in its five
columns of stage seconds (`search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s`), which are wall-clock times of
the machine at that moment ([Run time](speed.md)).

Rules to know:

- The paths in a record are relative to its folder. The new `run.toml` above works because `dotnet-again` is beside
  `dotnet-run`, at the same depth; in a folder elsewhere, the four file paths have to be adjusted.
- A whole record renamed `run.toml` also runs: the run reads its part one, says so in a warning, and compares the
  inputs' fingerprints with those in the record, naming in a warning any input that changed.
- **A run whose network or trips were given in memory cannot be run again from its record**, which has no file to
  name. Give tables as Parquet files for a run that should be repeatable from its folder
  ([A case from your own data](own_data.md)).

## Opening a finished run

% src: crates/binding/README.md (`Run.open`)

`Run.open(folder)` opens a finished run without running anything, and names in a warning an input whose fingerprint
differs from the record's: see [The map and the summary](pages.md#opening-a-finished-run).

## The seed

% src: PARAMETERS.md (`seed`); crates/choice/README.md ("Meanings and assumptions": the draws);
% crates/loop/README.md ("Choose"); run by the writer on dotnet

Under the logit a traveller's choice has a random part, and which travellers choose again at each iteration is drawn
at random. Every draw comes from the parameter `seed` (0 by default; any whole number from 0 to 2⁶³ − 1) and from
what it is for (the trip, the alternative, the iteration), never from the order of the work. The same seed gives the
same run; another seed gives another run of the same model. For `dotnet`:

| `seed` | Car (trips) | Bike (trips) | Walk (trips) | Transit (trips) | Mean trip time of the finished trips (min) | Iterations |
|---|---|---|---|---|---|---|
| 0 | 16 688 | 12 344 | 210 | 10 677 | 18.94 | 17 |
| 1 | 16 781 | 12 228 | 206 | 10 704 | 18.89 | 18 |

The two runs differ by up to 1 % of the car, bike or transit trips, and by 4 of about 210 walk trips. A difference of this size between two scenarios could be the
seed's: compare scenarios over several seeds.

## Threads

% src: crates/binding/README.md ("Meanings and assumptions": rayon's global pool); crates/loop/README.md
% ("Determinism"); run by the writer on dotnet

A run uses several processor threads, by default as many as the machine offers. The environment variable
`RAYON_NUM_THREADS`, set before Python starts, sets the number:

```text
RAYON_NUM_THREADS=1 python my_script.py
```

**The number of threads changes the run time and nothing else.** Every parallel stage files its results by trip,
every sum is made in a fixed order and every random draw is keyed by what it is for, so the tables are the same at
any thread count: `dotnet` at 1 thread gives the same `trips.parquet`, `links.parquet` and
`run_record.toml`, byte for byte, as at 12.

## For bikes

% src: crates/graph/README.md ("Bikes"); crates/demand/README.md ("Bikes"); crates/binding/README.md ("Bikes")

What a run means for bikes is in its record and its fingerprints. The record writes `bike_speed` under
`[parameters]`, and a run from the record's first part gives it to the network again. A link opened or closed to
bikes, or another bike speed, changes the network's fingerprint; a trip offered the bike or no longer offered it
changes the demand's.

## Limits

% src: crates/loop/README.md ("Determinism": across platforms, as measured); crates/io-parquet/README.md
% (`openmobisim_version`); crates/binding/README.md ("Errors": the hard link)

- The same results are promised for the same version of the simulator. Another version may give other results from
  the same inputs; the record says which version ran.
- Across operating systems the equality of results is observed, not guaranteed: Delft's default run gives
  byte-identical trips on macOS (Apple silicon), Linux (x86_64) and Windows (x86_64).
- The record holds the fingerprints of the inputs, not the inputs. Keeping the input files with the run is the user's
  part.

## See also

- [Reproducibility](../concepts/reproducibility.md): how the determinism is built.
- [The settings file](settings.md): the form of part one.
- [Citing](../reference/citing.md): what to keep of a run you report.
