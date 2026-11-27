# Run time

% src: crates/loop/README.md (the five stages); crates/io-parquet/README.md (`iterations.parquet`: the stage seconds);
% measurements of the five published cases

How long a run takes depends on the size of the network, the number of travellers and the number of iterations. This
page says what the time is made of, gives the measured times of the five published cases, and shows what the number of
threads changes.

**The machine behind every time on this page**: a laptop with an Apple M2 Pro processor (12 cores) and 32 GB of
memory. The times are wall-clock times of single runs; the same run takes another time on another machine or another
day. Times are in no result but the stage seconds of the iterations table, and never in a fingerprint.

## What a run's time is made of

% src: crates/loop/README.md ("Search", "Choose", "Load", "Measure", "Assess"); crates/summary/README.md ("Tiles",
% "Computation time"); run by the writer on the Delft run

A call of `run_file` reads the inputs, iterates, and writes the files. Each iteration has five stages, and the
iterations table gives the seconds of each, per iteration:

| Column | Stage | What it does |
|---|---|---|
| `search_s` | search | finds routes: for cars at every iteration, on the travel times of the last loading; for bikes, walkers and public transport at the first iteration only, since their times never change |
| `choose_s` | choose | each traveller offered a new choice takes one alternative of its set |
| `load_s` | load | moves every car, bike and walker along its route, link by link |
| `measure_s` | measure | prices every car route of every set again on the new travel times |
| `assess_s` | assess | computes the iteration's row: the mean, the counts, the shares |

```python
import pyarrow.compute as pc
import pyarrow.parquet as pq

iterations = pq.read_table("delft-run/iterations.parquet")
for stage in ("search_s", "choose_s", "load_s", "measure_s", "assess_s"):
    first = iterations[stage][0].as_py()
    total = pc.sum(iterations[stage]).as_py()
    print(f"{stage:10} {first:5.2f} s at iteration 0, {total:5.2f} s over the {iterations.num_rows} iterations")
```

```text
search_s    0.56 s at iteration 0,  2.38 s over the 11 iterations
choose_s    0.00 s at iteration 0,  0.03 s over the 11 iterations
load_s      0.10 s at iteration 0,  1.19 s over the 11 iterations
measure_s   0.02 s at iteration 0,  0.44 s over the 11 iterations
assess_s    0.00 s at iteration 0,  0.05 s over the 11 iterations
```

In this run of Delft the search takes most of the time, then the loading. The summary page shows the same five sums
under "Computation time", and its tile "run time" is their total; reading the inputs and writing the files are not in
it.

## The five cases

% src: measurements of `run_file` on each published case (doi:10.5281/zenodo.24585967), one fresh process per run, on
% the machine above

Each row is one call of `run_file` on the case as published. The run time is the wall-clock time of the whole call,
reading the inputs and writing the files included.

| Case | Travellers | Links, all modes | Iterations | Run time (s) | Search at iteration 0 (s) | Search, later iterations together (s) | Load, all iterations (s) | Measure, all iterations (s) | Peak memory (GB) |
|---|---|---|---|---|---|---|---|---|---|
| `dotnet` | 39 919 | 408 | 17 | 0.8 | 0.1 | 0.2 | 0.2 | 0.1 | 0.26 |
| `delft` | 37 649 | 88 512 | 11 | 4.1 | 0.5 | 1.5 | 1.1 | 0.4 | 1.05 |
| `lyon` | 117 073 | 413 832 | 11 | 45.3 | 18.7 | 13.7 | 6.4 | 2.6 | 4.37 |
| `amsterdam` | 198 252 | 489 167 | 9 | 53.6 | 22.8 | 15.9 | 7.5 | 2.8 | 5.01 |
| `paris` | 243 203 | 520 684 | 6 | 77.2 | 58.8 | 4.1 | 3.6 | 0.9 | 8.39 |

Lyon and Paris were run a second time: 41.1 s and 76.9 s, with peak memory of 4.92 GB and 9.34 GB. Two runs of one
case differ by a few seconds and by about half a gigabyte.

What these five rows show, and no more:

- **Run time and memory rise with the size of the case.** The five cases are in the same order by links, by run time
  and by peak memory. The largest, Paris, used 8.4 and 9.3 GB of memory in its two runs, more than the memory of a machine with 8 GB.
- **The first search is the largest single part in the three large cities**: 41 % of Lyon's run time, 43 % of
  Amsterdam's and 76 % of Paris's. It is the one search that covers bikes, walkers and public transport as well as
  cars. Paris has the most walk links of the five (426 740) and the most travellers.
- **Each further iteration costs a car search, a loading and a measure.** In Lyon the ten later iterations took
  13.7 s of search, about 1.4 s each; in Paris the five later iterations took 4.1 s, about 0.8 s each. Paris also has
  the fewest cars: 25.7 % of its travellers drive, against 57.3 % in Lyon ([The cases](cases.md)).
- **The number of travellers alone does not set the time**: `dotnet` has more travellers than Delft and runs in a
  fifth of the time, on a network of 408 links against 88 512.

Five cases cannot separate the effect of the links from that of the travellers, the modes or the timetable: the cases
differ in all of them at once. For a case of your own, the stage seconds of a short trial run
(`iteration_cap` set to 2 or 3, [The settings file](settings.md)) show what one iteration costs.

## Iterations

% src: crates/loop/README.md ("Stop"); PARAMETERS.md (`iteration_cap`, `iteration_floor`)

A run iterates until its stopping rule holds, and at most `iteration_cap` times (50 by default). The five cases stop
by the rule after 6 to 17 iterations; a run the rule does not stop runs to the cap. Lowering the cap shortens a run
and can leave it less settled: the iterations table says by how much ([The settings file](settings.md) shows a run
capped at 5 iterations, [The iteration](../concepts/iteration.md) the rule).

## Threads

% src: crates/binding/README.md ("Meanings and assumptions": `RAYON_NUM_THREADS`); crates/loop/README.md
% ("Determinism"); measured by the writer on Delft, one run per row, while other work ran on the machine

A run uses the machine's processor threads; the environment variable `RAYON_NUM_THREADS`, set before Python starts,
sets how many ([Running a run again](reproducibility.md)). The results are the same at any number. Delft, one run per
row:

| Threads | Run time (s) | Search, all iterations (s) | Load, all iterations (s) | Measure, all iterations (s) |
|---|---|---|---|---|
| 1 | 18.6 | 13.7 | 1.1 | 2.5 |
| 2 | 11.1 | 7.6 | 1.1 | 1.4 |
| 4 | 6.7 | 4.0 | 1.2 | 0.8 |
| 8 | 4.7 | 2.3 | 1.2 | 0.4 |
| 12 | 4.5 | 2.2 | 1.2 | 0.4 |

The search and the measure take less time with more threads; the loading takes the same time at every count. From 8
to 12 threads little changes on this machine. These rows were measured once each, while other programs ran, so they
are less exact than the table of cases, which has 4.1 s for the same run at the default number of threads.

## For bikes

% src: crates/loop/README.md ("Search": bikes, walkers and transit at iteration 0 only); crates/search/README.md
% ("Bikes")

Bike routes are searched once, at the first iteration: a bike's time on a link never changes, so its shortest route
does not either. Cyclists add to the first search and to every loading, where each is moved along its route and
counted per link and bin, and to nothing else: they are not searched or priced again at later iterations.

## See also

- [The iteration](../concepts/iteration.md): the five stages and the stopping rule.
- [The cases (reference)](../reference/cases.md): the cases' sizes.
