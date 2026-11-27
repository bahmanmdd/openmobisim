# Reproducibility

% src: crates/loop/README.md ("Determinism"); crates/types/README.md ("Outputs": `reduce`, `rng`, `fingerprint`; "Assumptions": determinism); crates/io-parquet/README.md (`run_record.toml`; "Not written: a time stamp, the machine, the thread count"); crates/binding/README.md (`run_file`, `Run.open`, "Tests")

A run of openmobisim can be repeated exactly. The same inputs and the same settings give the same trips, routes and
links tables and the same record, byte for byte: over two runs, and at any number of processor threads. The run's
record holds what is needed to run it again, and a fingerprint of each input, so that a change to an input is seen.

| Question | Answer |
|---|---|
| Does the number of threads change a result? | No. The tables are the same at any thread count |
| Do two runs of the same inputs and settings differ? | Only in the five columns of the iterations table that hold the seconds each stage took |
| Do two machines give the same result? | Measured, not guaranteed: a run of Delft's network gives byte-identical trips on the three platforms tested ([below](#across-platforms)) |
| What makes two runs differ on purpose? | The `seed`, under the logit |
| How do I run a run again? | Copy part one of its record into a `run.toml` and run it ([below](#the-record-runs-a-run-again)) |

## What is identical, and what is not

% src: crates/io-parquet/README.md ("Outputs"; "The writer's settings …"); crates/binding/README.md ("Tests": `test_determinism.py`, `test_record.py`, `test_run_file.py`, `test_open.py`); a run of the Delft case at 1 and 8 threads

| File | Over two runs and at any thread count |
|---|---|
| `trips.parquet` | Identical bytes |
| `links.parquet` | Identical bytes |
| `routes.parquet` | Identical bytes |
| `run_record.toml` | Identical bytes, when the inputs are files at the same place relative to the run's folder |
| `iterations.parquet` | Equal but for its five columns of computation time (`search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s`) |
| The map and the summary | Identical bytes when written twice from the same run under the same title, and from the run opened again with `Run.open`. The summary shows the computation time, which differs between two runs |

The files carry nothing that would make them differ: no time stamp, no machine name, no path of the machine, no thread
count. The Parquet files' metadata hold the writer's name and the table's schema, nothing else.

**Checking it yourself.** The number of threads is set by the environment variable `RAYON_NUM_THREADS`, read when the
program starts. With a script `run_delft.py` that runs the Delft case into a folder named on the command line,

```python
import sys
import openmobisim as oms

oms.run_file("delft", out=sys.argv[1])
```

two runs at 1 and at 8 threads,

```bash
RAYON_NUM_THREADS=1 python run_delft.py delft-1-thread
RAYON_NUM_THREADS=8 python run_delft.py delft-8-threads
```

compare as follows:

```python
import hashlib
from pathlib import Path

for name in ("trips.parquet", "links.parquet", "routes.parquet", "iterations.parquet", "run_record.toml"):
    digests = [hashlib.sha256(Path(folder, name).read_bytes()).hexdigest()[:12]
               for folder in ("delft-1-thread", "delft-8-threads")]
    print(name, *digests, digests[0] == digests[1])
```

```text
trips.parquet 44ba15acf8fb 44ba15acf8fb True
links.parquet ba6b66dc2fa4 ba6b66dc2fa4 True
routes.parquet e362b45c77c1 e362b45c77c1 True
iterations.parquet 8a076d27c7c9 785cedd1fd93 False
run_record.toml 27e9e2ad7efa 27e9e2ad7efa True
```

The iterations tables differ in the seconds each stage took, which no two runs share (here the whole script took
about 21 s at 1 thread and about 5 s at 8, the reading of the inputs included, on a laptop;
[Run time](../guides/speed.md)).

## How it is achieved

% src: crates/types/README.md ("Assumptions"); crates/loop/README.md ("Determinism"); crates/loading/README.md ("Events"; "Tests": nothing runs in parallel); crates/demand/README.md ("Every result is the same at any number of threads"); crates/search/README.md (`fastest_many`); crates/transit/README.md (`by_rides_many`)

A parallel program can give slightly different results from run to run, because its threads finish in another order
each time: sums are then added in another order, and random numbers handed out in another order. openmobisim removes
each of these causes.

| Cause of differences | What is done |
|---|---|
| The order of the input rows | Every node, link, stop, run, class and trip gets its internal number from the sorted order of its identifier, whatever the order of the rows given |
| The order in which threads finish | Every parallel stage (the route search, the journey search, linking the trips and the stops to the network, the parallel stages of the iteration) computes each trip from its own data alone and files the result under the trip's number: answer *i* is question *i*'s |
| The order of a sum | A sum is cut into chunks of 4 096 elements, each chunk summed in order, and the chunks combined in order, the same at any thread count |
| Random numbers | Every draw is a pure function of the seed, the stream it belongs to and the identity of what it is for (the trip, the alternative, the iteration). No draw depends on a thread or on how many draws came before |
| Events at the same instant | The loading runs on one thread and takes events in the order of the instant, then the trip's number |
| Ties | Each has a fixed rule that does not depend on the threads: two routes of equal time, two journeys of equal arrival, two alternatives of equal least cost are each decided the same way in every run |

## The seed and the random streams

% src: crates/types/README.md (`rng`); crates/choice/README.md ("Sampling is exact", "The draws come from the choice stream", "The draws are the traveller's taste"); crates/loop/README.md ("Choose": `Stream::MsaReselection` at (trip, k)); PARAMETERS.md (`seed`)

A run has one **seed**, the parameter `seed`, a whole number from 0 to 2⁶³ − 1, 0 by default. Random numbers are used
in two places of a run, each from a stream of its own:

| Stream | Used for | Addressed by |
|---|---|---|
| Choice | Under the logit, each traveller's taste: a random term (a standard Gumbel draw) per alternative and per mode | The trip, and the alternative's place in the traveller's set or the mode |
| Reselection | Whether a traveller is offered a new choice at an iteration | The trip and the iteration |

The taste draws do not depend on the iteration: a traveller keeps its taste for the whole run, so its choice changes
only when the costs change. Under the least-cost choice nothing of the choice is random; the seed then only decides
who is offered a new choice at each iteration.

**What another seed means.** Two runs that differ only in their seed are two samples of the same model: other tastes,
and other travellers offered a new choice at each iteration. Their difference shows how much of a result is chance.

## Across platforms

% src: crates/types/README.md ("Determinism: the same seed and the same binary on the same platform …"); crates/loop/README.md ("Across platforms, as measured"); crates/graph/README.md (`libm`); crates/choice/README.md (`libm`)

What is promised is: the same seed and the same program on the same platform give bit-identical results at any number
of threads. Across platforms the usual obstacle is the mathematics library, whose logarithm or exponential can differ in
the last bit from one system to another. openmobisim therefore takes every such function from one library written in
Rust (`libm`), not from the system: the logarithms of the Gumbel draws, of the logit's sum and of the path size, the
exponential, the power in the step, and the distances and the map projection.

As measured, a run of Delft's network with every parameter at its default (test 10 of [Validation](validation.md))
gives byte-identical trips on macOS (Apple silicon), Linux (x86_64) and Windows (x86_64). This is a measurement on
three platforms, not a guarantee for every platform.

## The fingerprints of the inputs

% src: crates/types/README.md (`fingerprint`); crates/graph/README.md ("Fingerprint"); crates/demand/README.md (`fingerprint()`); crates/transit/README.md (`Timetable::fingerprint()`); crates/io-parquet/README.md ("A fingerprint is …"); crates/binding/README.md (`run_file`, `Run.open`)

A **fingerprint** is a short number computed from everything an input was built from, so that two inputs with the same
fingerprint are, for practical purposes, the same input. The record holds three, each as 16 hexadecimal digits:

```python
import tomllib

with open("delft-run/run_record.toml", "rb") as f:
    result = tomllib.load(f)["result"]
for part in ("network", "demand", "transit"):
    print(part, result[part]["fingerprint"])
```

```text
network 3f87b0d8e1693b38
demand 4ba8ec884eb6e048
transit b78ffca832c14a3c
```

| Fingerprint of | What it covers | What it does not cover |
|---|---|---|
| The network | The projection; the bike and walk speeds; every node (identifier, position, signal mark); every link (identifier, its two nodes, length, lanes, modes, car free-flow speed, capacity, signal mark) | A link's drawing (its shape and road class), which no run reads |
| The demand | The classes; every trip (identifier, class, departure, modes, origin and destination) | The links to nodes and stops, which follow from the network, the timetable and `stop_walk_max`, each recorded beside it |
| The timetable | Every stop (identifier, position); every run with each call (stop, arrival, departure); every footpath (the two stops, the seconds) | The names of stops and lines, which only the pages read |

The same rows in any order give the same fingerprint, and a change to any value covered changes it. The hash is the
64-bit FNV-1a hash; it tells inputs apart and is not a cryptographic hash.

The fingerprints are compared when a finished run is used again. `Run.open` rebuilds each input from the file the
record names; when a rebuilt or passed input has another fingerprint than the record's, a warning names the input and
both fingerprints, and the run is opened with it. A settings file holds no fingerprint, so `run_file` compares nothing.

## The record runs a run again

% src: crates/io-parquet/README.md (`run_record.toml`: part one; the paths of the input files); crates/binding/README.md (`run_file`: "A record's part one … is a settings file"; "Tests": `test_run_file.py`, `test_record.py`); a rerun of the Delft case

The record `run_record.toml` is in two parts ([What a run gives](outputs.md#the-run-record)). Part one, every line
before `[result]`, is the settings the run ran with, with every default filled in: the window, every parameter, the
coefficients of every class, the input files and the classes. It is a valid settings file. Copied into a file named
`run.toml`, it runs the same run again:

```python
from pathlib import Path
import openmobisim as oms

record = Path("delft-run/run_record.toml").read_text()
Path("delft-again").mkdir()
Path("delft-again/run.toml").write_text(record.split("# Part two")[0])
again = oms.run_file("delft-again", out="delft-again")
for name in ("trips.parquet", "links.parquet", "routes.parquet", "run_record.toml"):
    print(name, Path("delft-run", name).read_bytes() == Path("delft-again", name).read_bytes())
```

```text
trips.parquet True
links.parquet True
routes.parquet True
run_record.toml True
```

Three things to know:

- **The input files are named by their path relative to the record's folder** (here `../delft/nodes.parquet`). The new
  folder above stands beside the old one, so the same relative paths lead to the same files. When a run's folder is
  moved, move its inputs with it, or edit the paths.
- **The record names the files; it does not hold them.** If an input file has changed since, the rerun reads the changed
  file. The fingerprints of the new record then differ from the old one's.
- **A run made from tables in memory cannot be run again from its record alone**, since the record has no file to name.
  Give the tables as Parquet files to have a record that runs again.

Because every default is written out in part one, a record also protects a result against a later change of a default.

The practical steps are in the guide [Reproducing a run](../guides/reproducibility.md).

## For bikes

% src: crates/types/README.md ("Bikes"); crates/graph/README.md ("Bikes": the fingerprint); crates/demand/README.md ("Bikes": the fingerprint); crates/binding/README.md ("Bikes")

Bikes are covered like every other mode. A link opened to or closed to bikes, another `bike_speed`, or a trip that
gains or loses the bike changes the fingerprint of the network or of the demand. The record holds the `bike_speed`
the network was built with, and a rerun from part one gives it to the network again; a rerun of a run with bike trips
repeats it byte for byte. Bikes use no random number of their own: a bike traveller's taste and its turn to be offered a new
choice come from the same two streams as everyone's.

## Limits

% src: crates/types/README.md ("Determinism"); crates/io-parquet/README.md ("a development version, ending `-dev`, does not identify the commit"; "Undetectable, said once"); crates/binding/README.md (`Network.tables()`: "the network of the tables has another fingerprint"; "a record without the files of its network and its demand cannot run again")

- **Identical results are promised on one platform with one program.** Across platforms they were measured on three
  (above), not proven. The promise is for the same program: the record holds the version that made it.
- **The seconds each stage took differ from run to run**, and with them `iterations.parquet` as a file.
- **A record made from tables in memory does not run again by itself** (above).
- **The fingerprints cover the inputs, not the results.** A value changed by hand inside a results table is not
  detected when the run is opened.
- **A fingerprint is not a cryptographic hash**: it guards against accident, not against intent.
- **A network read from OpenStreetMap and the same network given back as tables (`Network.tables()`) have different
  fingerprints**, because the tables carry no signal marks.
- **Not everything is in the record**: the table of road classes that the OpenStreetMap reader uses for missing speeds,
  lanes and capacities is fixed in the program and is not written.

All limits of the model are gathered in [Limitations](limitations.md).

## References

This page cites no published work. The fingerprint is the 64-bit FNV-1a hash of Fowler, Noll and Vo, with the offset
basis `cbf29ce484222325` and the prime `100000001b3` in hexadecimal; the random streams are ChaCha8 keystreams. The
works behind the model are in the [bibliography](../reference/bibliography.md).
