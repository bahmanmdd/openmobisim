# Troubleshooting

% src: the "Repairs" and "Errors" sections of crates/binding, graph, demand, io-osm, io-gtfs, loop and io-parquet
% (README.md of each); every message shown was raised by the writer on a small hand-made case

openmobisim follows one rule with its inputs: **it refuses only what no run can be made of, and repairs and reports
the rest.** So there are two kinds of message, and they ask for different things.

| Kind | What happened | How it shows | What to do |
|---|---|---|---|
| A **repair** | a row or value could not be used as given; it was set right or left out, and the work went on | one `UserWarning` per kind and call, with a count and the first five ids; the counts in `.repairs` and in the run's record | read the count; decide whether the rows matter; fix the input if they do |
| A **refusal** | nothing can be built or run from what was given | an exception (`TypeError`, `ValueError`, `OSError`) naming what is wrong; nothing is written | fix what the message names |

Every repair and refusal is listed in [Warnings](../reference/warnings.md). This page shows the ones most often met.

## Reading a repair

% src: crates/binding/README.md ("Repairs")

A repair's warning has one form: the entry point, the count, what was done, then in brackets the repair's key and the
first five ids.

```text
Demand.from_table: 1 trips naming a class not given: dropped (trips_of_unknown_class; first: by_bike)
```

The same counts are kept on the objects, as dicts from key to count: `Network.repairs`, `Transit.repairs`,
`Demand.repairs` (with `Demand.trips_given`, the rows of the trips table), and `Run.repairs`, which gathers the four:
`{"network": …, "demand": …, "transit": …, "run": …}`. The record writes them under `[result.network.repairs]`,
`[result.demand.repairs]`, `[result.transit.repairs]` and `[result.repairs]`, and the summary page's first tile says
how many trips were given and how many were dropped, by reason.

Python shows a given warning once per place by default. To see every one, or to stop at the first:

```python
import warnings
warnings.simplefilter("always")     # or "error": a repair then raises
```

## Repairs of the network

% src: crates/binding/README.md ("Inputs": a cell, "Repairs"); crates/graph/README.md ("Repairs")

| Message (shortened) | Key | Cause | What to do |
|---|---|---|---|
| `columns a table does not read: ignored` | `columns_ignored` | a column that is not one of the table's columns; often a misspelt name, such as `shapes` for `shape` | nothing, or correct the name |
| `car links without lanes or a car speed above zero: the car mode taken off` | `car_mode_removed` | a link with `car` in its modes and no `lanes` or no `car_speed_km_h` | give both on every car link; the link is kept for its other modes |
| a link naming a node that is not there: left out | `links_to_no_node` | an `upstream` or `downstream` that is no `node_id` of the nodes table, or names a node that was itself left out | compare the ids of the two tables; look for a node repair above it |
| a row with a cell that cannot be read: left out | `nodes_unread`, `links_unread` | a null where none is allowed, 300 lanes, text where a number is expected | the warning names the row and the column |
| a link left with no mode: left out | `links_without_mode` | an empty `modes`, or only unknown mode names | write the modes as `car;bike;walk` |
| a link of 0 m: left out | `links_of_no_length` | two nodes at one position and no length | merge the nodes, or give the length |

When no link is left the warning says so plainly ("no link is left"), and the work still goes on.

## Repairs of the travellers

% src: crates/demand/README.md ("Repairs"); crates/binding/README.md ("Inputs": a mode set, "Repairs")

| Message (shortened) | Key | Cause | What to do |
|---|---|---|---|
| `rows naming a mode that is not car, bike, walk or transit: the name left out` | `mode_names_unknown` | `bicycle`, `pt`, `drive` and the like | use the four names; they are read trimmed and in any case |
| `trips with no mode (given, or left): dropped` | `trips_without_mode` | every mode of the trip was unknown or has no link in the network | as above; or the network lacks links of that mode |
| `trips naming a class not given: dropped` | `trips_of_unknown_class` | a `class_id` that is not in the list `classes` | add the class to the list, and its coefficients if you give any |
| `rows with a cell that cannot be read as its field: left out` | `trips_unread` | a departure with a fraction of a second or below zero, a null | `departure_s` is a whole second after midnight |
| a mode that no link allows: taken out of the trip's set | `trips_without_car_network` and the like | `car` in a trip's modes on a network with no car link | check the links' `modes` |
| a coordinate that cannot be placed: dropped | `trips_unplaced` | a position that is not a finite number, or a longitude beyond ±180° or a latitude beyond ±84° | check the four position columns |

When every trip is dropped the warning reads "no trip is left: N given, all dropped (by reason: …)". The run still
writes its files and pages, which show an empty run.

## Trips dropped or unfinished because of the window

% src: crates/loop/README.md ("Repairs", "The window and the mean")

```text
run: 1 trips departing before the window's start or at or after its end: dropped (trips_outside_window; first: on_foot)
```

A trip that departs outside the window is not run and has no row in `trips.parquet`. If the count is large, the
window and the departures do not match: check that both are in seconds after midnight of the same day.

A trip that departs inside the window and is not at its door when it ends is **unfinished**: it is run, its row has
`finished` false and no travel time, and it is in no mean. Many unfinished trips mean the window ends before the
trips do. [The window and unfinished trips](window.md) shows how to read results with them and how to choose the
window.

## Trips without an alternative, without a journey, without a stop

% src: crates/loop/README.md ("Outputs": the iterations table); crates/demand/README.md ("Meanings and assumptions");
% PARAMETERS.md (`stop_walk_max`)

Three columns of the iterations table count travellers the model could not serve as asked. None of them is an error.

| Column | Who is counted | Common cause | What to do |
|---|---|---|---|
| `without_alternative` | trips that no mode open to them can make; `mode` is null in `trips.parquet` | the only mode is `transit` and no journey exists; the origin and the destination are not connected for the trip's modes | open another mode to the trip; check the network |
| `without_transit` | trips with `transit` in their modes that were offered no journey | see the next row; or stops at both ends and no journey between them in the timetable | look at the timetable: the reader takes one service day of the feed, its busiest weekday |
| `without_stop` | of those, the trips with no stop within `stop_walk_max` of the origin or the destination | a trip end far from any stop; a run with `transit` in the trips' modes and no timetable given | raise `stop_walk_max` (1 500 m by default); give the timetable to `Demand.from_table` and to `run` |

A traveller counted in `without_transit` still travels when another of its modes can make the trip; only one with no
alternative at all is in `without_alternative`.

## Refusals of a keyword or a value

% src: crates/binding/README.md ("Errors")

| Message | Cause | What to do |
|---|---|---|
| `TypeError: Network.from_tables: bikespeed is not a parameter; the parameters are the rows of PARAMETERS.md` | a misspelt keyword | every keyword is a row of [Parameters](../reference/parameters.md) |
| `TypeError: Network.from_tables: seed is read by run, not here` | a parameter given to the wrong function | give it where the message says |
| `TypeError: run() missing 1 required keyword argument: 'window'` | no window; it is never defaulted | `window=(start_s, end_s)` |
| `ValueError: seed: True is a boolean; this parameter is a number, give 1 instead` | `True` or `False` for a number | give the number |
| `ValueError: coefficients: class bike_only has no row` | `coefficients` given for some classes only | give the sixteen for every class ([Traveller classes](classes.md)) |
| `ValueError: the window: the window must be longer than zero seconds` | the end at or before the start | two seconds after midnight, the start first |
| `ValueError: the demand was built on another network` | the network or timetable given to `run` is not the one given to `Demand.from_table` | build the demand again on the network you run |

## Refusals of a table or a file

% src: crates/binding/README.md ("Inputs": a Parquet file; "Two files with fixed names"; "Errors")

| Message (shortened) | Cause | What to do |
|---|---|---|
| `ValueError: links: missing column lanes; the columns are link_id, upstream, …` | a required column is absent; names are fixed, with no aliases | rename or add the column ([Inputs](../reference/inputs.md)) |
| `ValueError: trip …: a position of another kind than the network's` | trips in longitude and latitude on a network in metres, or the reverse | one kind of position for the network and the trips |
| a Parquet column of a type not read, or a codec other than snappy | a boolean or date column; a file compressed with zstd or gzip | text and numbers only; write with snappy (pyarrow's default) or no compression |
| `FileNotFoundError: …: run_file reads a settings file named run.toml, or the folder holding it` | nothing is at the path given | give the folder that holds `run.toml`, or the file's path |
| `ValueError: …: run_file reads a settings file named run.toml, or the folder holding it` | the path is an existing file of another name | the file must be named `run.toml` |
| `ValueError: …: the folder holds a run's record, run_record.toml, and no settings file, run.toml` | `run_file` given the folder of a finished run | write a `run.toml` from the record ([Running a run again](reproducibility.md)), or open the run with `Run.open` |
| `FileNotFoundError: …: no run_record.toml there` | `Run.open` given a folder that holds no finished run | give the folder that was the run's `out` |
| `ValueError: run_record.toml: the network was given in memory when the run was made …; pass it: network=` | `Run.open` on a run whose tables were not files | `Run.open(folder, network=network, demand=demand)` |
| `ValueError: … is one of the run's input files; writing the run there would replace it` | `out` is the folder of the inputs, and a result would take an input's name | write the run into another folder |
| `UserWarning: …: the folder holds a finished run (its run_record.toml); this run replaces its files` | `out` already holds a run | nothing, if replacing it was meant |

A timetable on a network in metres is refused: a stop has a longitude and a latitude, and only a network built from
longitude and latitude can place it.

## For bikes

% src: crates/graph/README.md ("Repairs": for bikes); crates/demand/README.md ("Repairs"); crates/loop/README.md
% ("Repairs": for bikes)

The repairs that most often concern bikes: the mode name `bicycle`, which is not read (the name is `bike`), so the
trip loses the bike and is dropped if it had no other mode; `trips_without_bike_network`, when no link of the network
allows bikes; and a link that is both a car link and a bike link without lanes or a car speed, which loses the car
(`car_mode_removed`) and stays a bike link. A bike trip whose door is so far from the bike network that the walk there
would end after the window loses the bike for the run (`modes_after_window`).

## If the result looks wrong

% src: README.md ("Known limits", the issues link); crates/binding/README.md ("Outputs": `repairs`, reports)

Look first at `run.repairs`, at the three counts above in the last row of the iterations table, and at the summary
page, whose first tiles give the trips given, dropped, completed and not completed. Then compare with
[Limitations](../concepts/limitations.md): what release 0.2 does not model. If a result is still implausible, please
report it at <https://github.com/bahmanmdd/openmobisim/issues>, with the run's record.
