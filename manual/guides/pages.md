# The map and the summary

% src: crates/binding/README.md (`Run.map`, `Run.summary`, `Run.open`); crates/map/README.md; crates/summary/README.md

A finished run can write two pages: an interactive **map** of its traffic and journeys, and a **summary** of its
numbers. Each is one self-contained HTML file. It holds its own style, script and data, names no web address and loads
nothing, so it opens from disk in any current browser, with no server and no connection, and can be sent to a
colleague as it is.

## Writing the two pages

% src: crates/binding/README.md (`Run.map`, `Run.summary`); run by the writer on the Delft run

```python
import os
import openmobisim as oms

run = oms.Run.open("delft-run")        # or the Run that run_file returned
run.map("delft-map.html", title="Delft, a synthetic morning")
run.summary("delft-summary.html", title="Delft, a synthetic morning")
for page in ("delft-map.html", "delft-summary.html"):
    print(page, round(os.path.getsize(page) / 1e6, 1), "MB")
```

```text
delft-map.html 3.2 MB
delft-summary.html 2.1 MB
```

Each call writes the file, replacing one already there, and returns its path. The rules:

- **The title** is the `title` given, else the file's name without its extension.
- **The same run under the same title gives the same file**, byte for byte: a page holds no time stamp, machine name
  or path.
- **A path that is one of the run's own files or one of its input files is refused**, before anything is written.
- **A page is written whole or not at all**: a write that fails leaves no part of a page.
- Each page reads the run's four tables, its record and its inputs when it is written. A table that cannot be read is
  refused by name; what the reader sets aside (a trip's second row, a link row outside the window) is told in a
  warning.

## Opening a finished run

% src: crates/binding/README.md (`Run.open`, the table "The rule for a finished run")

`Run.open(folder)` opens the folder of a run made earlier, without running anything, and returns a `Run` like the one
`run` returns. It rebuilds each input from the file the record names, so the pages of a run can be written at any
time, in another session, and they are the same bytes as those of the fresh run.

An input that was given in memory when the run was made has no file in the record and must be passed:
`Run.open(folder, network=network, demand=demand)`. An input whose fingerprint differs from the record's (a file that
changed since the run) is named in a warning with both fingerprints, and the pages are written from it all the same
([Running a run again](reproducibility.md)).

## What the map shows

% src: crates/map/README.md ("Outputs", "Meanings and assumptions", "Bikes")

```{figure} ../_static/figures/delft_map.png
:alt: The map page of a Delft run: the legend on the left, the roads as ribbons coloured by volume over capacity, the layers and the controls on the right
:width: 100%

The map page of the Delft case as it opens (the network is from OpenStreetMap, © OpenStreetMap contributors).
```

The map draws the network and four layers that can be switched on and off: road traffic, transit, cyclists and
walkers. A reader can:

- colour the road traffic by volume over capacity (as the page opens), by delay over free flow, or by volume;
- show the whole window or one time bin, and play the bins in turn;
- hover or click a car link for its volume, its delay over free flow and the seconds each car loses there;
- choose a pair of places (an origin and a destination), listed busiest first, and see the journeys its travellers
  took, by every mode, with each journey's travellers and time;
- click a link or a transit hop to see which pairs use it;
- zoom, switch to a night theme, and save the view as a PNG or an SVG;
- keep the view in the page's address after `#`, to come back to it or share it.

How the page computes what it draws:

| Quantity | Definition |
|---|---|
| Volume of a car link (vehicles per hour) | the cars that entered in the bins shown, over the hours of those bins |
| Delay over free flow | the cars' mean crossing time over the car free-flow time, less 1, never below 0; shown in four bands: 0–15 %, 15–50 %, 50–100 %, 100 % and more. A link whose cars lose under 2 s each is in the first band whatever its ratio |
| Volume over capacity | the volume of the busiest period (15 minutes for bins of 300 s) over the link's capacity; four bands: 0–0.6, 0.6–0.8, 0.8–1.0, 1.0 and more |
| A pair | the trips with the same origin and destination positions; the page holds the 2 000 busiest pairs and its footer says how many of how many |
| A transit hop | two consecutive stops ridden, with the travellers whose journeys ride between them, coloured by the kind of service (rail, metro, tram, bus, ferry, other) |

Roads are drawn by road class when every link has one (the optional column `road_class`, or an OpenStreetMap
network), else by capacity, and each link along its shape when it has one, else straight. The footer gives the run's
provenance from its record: the version, the window, the bin, the choice model, the iterations and what ended them,
the seed and the start of the network's fingerprint.

## What the summary shows

% src: crates/summary/README.md ("Outputs", "Meanings and assumptions")

```{figure} ../_static/figures/delft_summary.png
:alt: The top of the summary page of a Delft run: the headline tiles and the table of the modes
:width: 100%

The top of the summary page of the Delft case.
```

The summary is a page of numbers, charts and static maps, in this order:

| Section | What it holds |
|---|---|
| Tiles | the trips, the share completed and not completed, the mean trip and the total travel time of the finished trips, the vehicle-kilometres and mean speed on the roads, the run time |
| Modes | per mode: trips, share, share completed, mean trip, travel time; a row for the trips without an alternative |
| When people left | departures by quarter hour and mode |
| On the roads | vehicle-kilometres by delay band; the cars entering links per bin; the five busiest links |
| Transit, bikes and walking | boardings, rides per transit trip, kilometres cycled, kilometres walked on the network |
| Convergence | the mean trip by iteration, the iterations table, and what ended the run |
| Computation time | the seconds of the five stages; the network's links, nodes and kilometres by mode |
| Settings | the run's record, whole |
| Maps | one per mode with travellers: car traffic by delay band, cyclists, walkers, transit |

Three words of the page: a trip is **completed** when it is at its door before the window's end, **still travelling**
when it has an alternative and is not, and **without an alternative** when no mode open to it can make the trip. The
three add up to the trips. The mean trip and the travel time are over the completed trips
([The window and unfinished trips](window.md)).

For the Delft run the page reads 37,649 trips, 100.00 % completed, a mean trip of 14.4 min and 12,612.2 km cycled (the
page writes a comma between the thousands).

## For bikes

% src: crates/map/README.md ("Bikes", "Bikes and walkers"); crates/summary/README.md ("Bikes")

On the map the cyclists are a layer of their own on the same links as the cars: a line per link whose width grows with
the cyclists who entered it over the window, full width at 3 000 and more. A bike journey is listed in its pair beside
the car's and drawn dashed, and a clicked link lists the pairs whose bike journeys use it. The time slider and the
play button step the road traffic only; the cyclists' layer shows the whole window. In the summary bikes are a row of
every table by mode, the kilometres cycled a number of their own, and the cyclists a map of their own.

## Limits

% src: crates/binding/README.md ("Tests": the pages); crates/map/README.md ("Meanings and assumptions")

- The map holds the 2 000 busiest pairs, not every pair; the layers of traffic, cyclists, walkers and transit count
  every trip.
- A trip the run dropped (one departing outside the window) is in no count of either page; the summary's first tile
  says how many trips were given and how many dropped.

## See also

- [Reading the results](results.md): the tables behind the pages.
