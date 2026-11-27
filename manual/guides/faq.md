# Questions

Short answers, each with the page that holds the full one.

## Installing and running

% src: README.md ("Start here"); crates/binding/README.md (the opening, "Meanings and assumptions", "Systems and user
% rights")

**Which Python does it need?**
Python 3.11 or newer.

**Which operating systems?**
macOS, Linux and Windows: a wheel is published for each. On Windows it runs as a standard user, without administrator
rights.

**Does it need other Python packages?**
None to run: `pip install openmobisim` installs the simulator and nothing else. To read the tables a run writes you
need a Parquet reader, for instance pandas with pyarrow, `pip install pandas pyarrow`
([Reading the results](results.md)). Input tables may be dicts of lists, pyarrow tables, pandas or polars frames, or
Parquet files ([A case from your own data](own_data.md)).

**Is there a command line, or a progress bar?**
No. openmobisim is a Python module of a few functions ([The Python interface](../reference/api.md)), and a run prints
nothing while it runs: no log and no progress output. A run described in a settings file is one call, `run_file`
([The settings file](settings.md)).

**How long does a run take, and how much memory?**
From under a second for the synthetic city to 77 s and 8.4 GB of memory for Paris with 243 203 travellers, on a laptop
([Run time](speed.md)).

## Results

% src: crates/loop/README.md ("Determinism", "The window and the mean", "Assess"); the published record's README.md;
% README.md ("Known limits")

**Do two runs give the same results?**
Yes: the same inputs and settings give the same tables, byte for byte, at any number of threads. Another `seed` gives
another run of the same model ([Running a run again](reproducibility.md)).

**Can I study Delft, Amsterdam, Lyon or Paris with the published cases?**
No. Their networks and timetables are real, but their travellers are synthetic: neither observed nor a forecast, and
not calibrated. They are for learning the tool and comparing methods ([The cases](cases.md)).

**Why is the mean trip time lower when I shorten the window?**
Because the mean is over the finished trips only, and a shorter window leaves the long trips unfinished
([The window and unfinished trips](window.md)).

**Has my run converged?**
A run reports how settled it is and does not judge it: under the logit, the share of travellers who would change their
choice; under the least-cost choice, the relative gap. A run the stopping rule ended can still be short of
equilibrium ([The iteration](../concepts/iteration.md)).

**Are the default choice coefficients estimated for my city?**
No. They are starting values, to be calibrated for a study ([Traveller classes](classes.md)).

## What openmobisim does not do

% src: crates/graph/README.md ("Meanings and assumptions"); crates/loading/README.md ("Meanings and assumptions",
% "Bikes"); crates/transit/README.md ("Meanings and assumptions", "Bikes"); crates/loop/README.md ("An alternative",
% "Money"); crates/io-gtfs/README.md; crates/io-osm/README.md ("Signals", "The class table"); README.md ("Known limits")

Each answer here is a plain no. [Limitations](../concepts/limitations.md) has the full list and what each one means
for results.

**Traffic signals?**
Not applied. A signal in an OpenStreetMap extract is stored and changes nothing: no car waits at a signal. For a
network read from OpenStreetMap, each road class's capacity per lane is set with its junctions counted in, since no
junction is modelled ([The network](../concepts/networks.md)).

**Spillback?**
No. A car link is a point queue: cars wait at its downstream end in a queue that takes no room on the road. A queue
never fills its link and never blocks the links upstream ([The loading](../concepts/loading.md)).

**Buses in traffic?**
No. Public transport runs on its timetable. A bus is not on the road: cars do not delay it, and it does not delay
cars ([Transit](../concepts/transit.md)).

**Park-and-ride, bike-and-ride, parking?**
No. An alternative is one mode from door to door: a car route, a bike route, a walk, or a transit journey reached and
left on foot. Car and transit, or bike and transit, are not combined in one trip, and parking is not modelled.

**Fares, tolls, fuel costs?**
No. The choice has a coefficient per euro, but no input carries a price, so the money of every alternative is 0.

**Bike or pedestrian congestion?**
No. Bikes and walkers travel at one speed each, whatever the traffic ([Cycling](cycling.md)).

**Choice of departure time or destination?**
No. Each trip's ends and departure are given in the trips table; a traveller chooses its mode and its route.

**A choice model of my own, written in Python?**
No. There are two models, the nested logit and the least-cost choice, and their coefficients are yours to set
([Traveller classes](classes.md)).

**Another day of the GTFS feed than the busiest weekday?**
No. The reader takes the feed's busiest weekday, and no other day can be asked for
([A case from your own data](own_data.md)).

**Other network formats, such as GMNS?**
No. A network is two tables with fixed columns, or an OpenStreetMap extract ([Inputs](../reference/inputs.md)).

**Imperial units?**
No. Metres, seconds, km/h and vehicles per hour throughout.

## Help

% src: README.md ("Cite")

**Something is refused, or a warning appears.**
See [Troubleshooting](troubleshooting.md).

**A result looks wrong.**
Please report it, with the run's record, at <https://github.com/bahmanmdd/openmobisim/issues>.

**How do I cite it?**
See [Citing](../reference/citing.md).
