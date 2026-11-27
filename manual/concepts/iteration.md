# The iteration and the stopping rule

% src: crates/loop/README.md; crates/loop/src/assess.rs (the stopping rule and its three constants); PARAMETERS.md; README.md ("Known limits")

Travellers choose their mode and route by what a trip costs, and what a car trip costs depends on what everyone else
chose: a road that many take is slow. The state the simulator looks for is one where the two agree, where nobody would
choose otherwise at the travel times that the choices themselves produce. The simulator approaches that state by
iterating: it lets travellers choose, loads the roads with what they chose, measures the
times, offers a part of the travellers a new choice at those times, and repeats.

**The iteration** (the module `loop`) is the part of the simulator that does this, and the only part that connects the
others. It calls them in turn, in five stages:

| Stage | What it does | The part it calls | What it hands on |
|---|---|---|---|
| 1. Search | Finds for each traveller the best alternative of each mode at the current times and adds it to the traveller's set when it is new | [The route search](search.md); [the journey search](transit.md) | Each traveller's **set** of alternatives |
| 2. Choose | Offers a part of the travellers a new choice over their set | [The choice model](choice.md) | The alternative each traveller takes |
| 3. Load | Moves every chosen car, bike and walk route through the window | [The loading](loading.md) | Each trip's arrival; the link time reading |
| 4. Measure | Prices again every car route of every set at the new times | The loading's link time reading | The cost of every alternative, taken or not |
| 5. Assess | Computes what the iteration reports, and decides whether to stop | Itself | One row of the iterations table |

When the iteration stops, the last iteration's state is the run's result ([What a run gives](outputs.md)). The loop is
drawn in [How a run works](architecture.md#interactions-in-a-run); this page explains each stage, who is offered a
new choice, and when the run stops.

## Terms

% src: crates/loop/README.md ("An alternative", "What the choice is over", "Units")

- An **alternative** is a mode with three legs: a walk from the door, then a route on the network (by car, bike or on
  foot) or a journey on the timetable, then a walk to the door.
- A traveller's **set** holds the alternatives the search has found for it so far. A set only grows: an alternative,
  once in, stays in its place for the whole run.
- A **cost** is the choice model's sum of coefficient times attribute over the attributes of an alternative (minutes of each
  kind of time, transfers, euros, the mode's constant); lower is better.
- Iterations are numbered from 0. The letter $k$ is the iteration's number.

## The five stages

### Search

% src: crates/loop/README.md ("Search", "Car routes per origin-destination pair", "What the choice is over")

Cars are searched at every iteration: for each traveller who may drive, the fastest route at the link times of the last
loading, from its own departure. Bikes, walkers and transit are searched at iteration 0 only, because their times never
change during a run: a bike and a walker travel at free flow and transit runs on its timetable.

A route that is not yet in the traveller's set is added to it. It counts as **found** when its cost is strictly below
that of every alternative of the same mode already in the set (the first alternative of a mode always counts). The cost
is compared without the path-size term, which is explained on the [choice page](choice.md).

So a set holds, per mode, the fastest route at iteration 0; for cars also every route that was the fastest at some
later iteration's times; and the transit journeys offered at iteration 0. It is not every route of the network: a car
route that is never the fastest at any iteration is never offered. The number of distinct car routes that the trips
of one origin-destination pair (the same car origin node and the same car destination node) may gather is limited by
`max_car_routes` (6 by default); a route new to an origin-destination pair at its limit is not added, and none is replaced ([Routes and route sets](search.md)).

### Choose

% src: crates/loop/README.md ("Choose", "Who chooses again"); crates/choice/README.md ("The draws are the traveller's taste")

At iteration 0 every traveller chooses. At every later iteration only a part of the travellers is **offered a new
choice** ([below](#who-is-offered-a-new-choice)); the others keep the alternative they hold. A traveller who is offered
takes the choice model's choice over its whole set, its mode included, whatever it held before.

Two choice models exist, set by the parameter `choice` ([Choice](choice.md)):

- `"logit"` (the default): a nested logit with the modes as nests. Each traveller has its own taste, a random term per
  alternative and per mode that is drawn once and kept for the whole run. So a traveller's choice changes only when the
  costs change, never because the dice are thrown again.
- `"deterministic"`: the alternative of least cost.

### Load

% src: crates/loop/README.md ("Load"); crates/loading/README.md

Every chosen car, bike and walk route is given to [the loading](loading.md) as one vehicle. Every vehicle
enters the first link of its route at the instant its walk from the door to the network ends: the departure second
plus the walking time, not rounded. Cars queue; bikes and walkers travel at free flow. A transit journey, and a trip
whose two ends are linked to the network at the same node, load nothing.

### Measure

% src: crates/loop/README.md ("Measure")

Every alternative with a car route, taken or not, is priced again by reading its links one after the other on the new
loading, as its own traveller. The alternative a traveller took therefore costs exactly the time its trip took. A
bike's, a walker's and a transit journey's time is the one the search gave it: none of them reads a car's crossing of
a link, so pricing it again would give the same number.

### Assess

% src: crates/loop/README.md ("Assess", "Stop")

The last stage computes the numbers the iteration reports ([below](#what-each-iteration-reports)) and reads the
[stopping rule](#the-stopping-rule).

## Iteration 0

% src: crates/loop/README.md ("Search": "free flow at iteration 0"; "Choose": "at iteration 0 everyone chooses")

Iteration 0 starts from an empty network. The search reads free-flow times on every link, so every traveller's set
holds, for each mode open to it, the fastest route of the empty network, and its transit journeys. Everyone chooses,
as if the roads were empty; the first loading then shows what that costs.

## Who is offered a new choice

% src: crates/loop/README.md ("Choose", "Who chooses again"); PARAMETERS.md (`step_exponent`)

Travellers behind one queue read the same cost, and if all of them were offered a new choice at once they could all move together.
The classical remedy is the **method of successive
averages** (Sheffi 1985): at iteration $k$ move only a share $1/(k+1)$ of the flow, a share that shrinks so that the
moves die out. openmobisim applies it to individual travellers: at iteration $k \ge 1$ each traveller is offered a new
choice with a probability, drawn independently per traveller and iteration.

### Under the logit

```{image} ../_static/figures/offer_share_paper.svg
:alt: The share of travellers offered a new choice at iteration k, from 0 to 50: 1/(k+1)^0.75 by default and 1/(k+1) for the method of successive averages
:class: only-light
:width: 100%
```
```{image} ../_static/figures/offer_share_night.svg
:alt: The share of travellers offered a new choice at iteration k, from 0 to 50: 1/(k+1)^0.75 by default and 1/(k+1) for the method of successive averages
:class: only-dark
:width: 100%
```

Every traveller is offered at the same rate,

$$
p_k = \frac{1}{(k+1)^{a}},
$$

where $a$ is the **step exponent**, the parameter `step_exponent` (0.75 by default). At $a = 1$ this is the step of
the plain method of successive averages. The averaging settles when the steps sum to infinity and their squares do not
(Robbins and Monro 1951), which holds for $a$ in $(0.5, 1]$. A smaller $a$ offers a new choice to more travellers at every
iteration after the first, so a run comes nearer its settled state in fewer iterations.

| Iteration $k$ | Share offered at $a = 0.75$ (the default) | Share offered at $a = 1$ |
|---|---|---|
| 1 | 0.595 | 0.500 |
| 2 | 0.439 | 0.333 |
| 3 | 0.354 | 0.250 |
| 5 | 0.261 | 0.167 |
| 10 | 0.166 | 0.091 |
| 20 | 0.102 | 0.048 |
| 49 | 0.053 | 0.020 |

The rate is the same for everyone on purpose. The logit puts a share of travellers on an alternative that is not the
cheapest of its mode, and when costs change, travellers have to move both towards the cheaper and towards the dearer
alternative. A rate that rose with what a traveller would save would ask for the first kind of move often and the
second rarely, and would leave too few travellers on the dearer alternatives.

A choice once made is held until the traveller is offered again. With the default exponent the choices held are recent:
after 50 iterations 0.08 % of travellers still hold their choice of iteration 0 (2 % at $a = 1$), and 45 % made their
last choice in the final ten iterations (20 % at $a = 1$).

### Under the least-cost choice

Under the least-cost choice every traveller belongs on the cheapest alternative of its set, so a traveller far from it
can be offered a new choice more often. Each traveller is offered with the probability

$$
p_k = \max\left(\frac{1}{k+1},\; s\right), \qquad
s = \begin{cases} r & \text{if } r > 0.2 \\ 0 & \text{otherwise,} \end{cases} \qquad
r = \frac{c - b}{c},
$$

where $c$ is the cost of the alternative the traveller holds and $b$ the least cost among the alternatives of the same
mode in its own set, both at the times the last iteration measured ($s = 0$ when $c$ is not positive). In words: $r$
is the share of its cost the traveller would save by moving to its best alternative of the same mode. Everyone is
offered a new choice at least at the plain rate $1/(k+1)$; a traveller who would save more than 20 % is offered one
at the rate of the share it would save. This is after Smith's proportional switching (1984), in which flow moves between routes in proportion to
their cost difference, outside a band of indifference (Mahmassani and Chang 1987).

An example where cost is proportional to time: a traveller whose car route reads 30 min where its best car route reads
20 min would save a third. It is offered with probability 1/2 at $k = 1$ (the plain rate is larger there) and 1/3 from
$k = 2$ on. A traveller whose route reads 22 min would save 9 %, is inside the band, and is offered at the plain rate.

Only alternatives of the mode held enter $b$, but a traveller who is offered chooses over its whole set, so it also
reconsiders its mode. The band, 0.2, is a fixed constant of the rule, not a parameter: inside it only the plain step
acts, so that travellers do not keep moving between near-equal routes at a rate that does not fall with $k$.

## What each iteration reports

% src: crates/loop/README.md ("Outputs": `iterations`; "Assess"); crates/loop/src/assess.rs (the fields of `Iteration`); crates/io-parquet/README.md (`iterations.parquet`)

Each iteration writes one row of the iterations table (`iterations.parquet`; every column is listed in
[Output files](../reference/outputs.md)). The shares are over the trips that have an alternative.

| Reported | Column | Meaning |
|---|---|---|
| The mean trip time | `mean_trip_time_s` | The mean door-to-door time of the trips that finished within the window, in seconds |
| The trip counts | `finished`, `unfinished`, `without_alternative` | Trips at their door before the window's end; trips unfinished then; trips that no mode open to them can make |
| Transit out of reach | `without_transit`, `without_stop` | Trips with transit open to them and no journey offered; of those, the trips with no stop in walking reach at one end |
| The offered share | `offered_share` | The share of trips offered a new choice in this iteration (1 at iteration 0) |
| The switch share | `switch_share` | The share of trips that changed alternative in this iteration's choice |
| The found share | `found_share` | The share of trips for which this iteration's search found an alternative cheaper than every one of its mode in their set (1 at iteration 0, where every first alternative counts) |
| The share who would change | `would_change_share` | Under the logit only: the share of trips for which the logit, choosing again over their own set at the costs this iteration measured, would take another alternative than the one they hold |
| The duality gap | `gap` | Under the least-cost choice only: how much more the travellers pay than they would on the cheapest alternative of their sets, as a share (below) |
| The computation time | `search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s` | The seconds each stage took |

**One measure of the distance to equilibrium per choice model.** Each model has the one measure that can reach 0 under
it. It is reported, never judged: no run is refused or flagged for its value.

- Under the **least-cost choice** the measure is the **duality gap** (relative: over the sum of the least costs),

  $$
  \text{gap} = \frac{\sum_i \left(c_i^\text{taken} - c_i^\text{least}\right)}{\sum_i c_i^\text{least}},
  $$

  summed over the travellers $i$ with an alternative, with $c_i^\text{taken}$ the measured cost of the alternative
  traveller $i$ holds and $c_i^\text{least}$ the least measured cost in its set. It is 0 when everyone is on the
  cheapest alternative of its set. The share who would change is not the measure here: at the equilibrium the routes in
  use cost the same, so which of them a traveller holds does not settle.
- Under the **logit** the measure is the **share who would change**. The model puts a share of travellers on an
  alternative that is not their cheapest, so the duality gap does not reach zero. Because each traveller's taste is fixed for
  the run, its choice at given costs is fixed too, and the share is an exact count made with the choose stage's own
  choice, not an estimate. When it is 0, nobody would change, and the run is at the fixed point of its iterations for
  the sets it has found.

The column of the other model is empty (null).

## The stopping rule

% src: crates/loop/README.md ("Stop"); crates/loop/src/assess.rs (`stops`, `STOP_WINDOW`, `STOP_FOUND_SHARE`, `STOP_MEAN_RANGE`); PARAMETERS.md (`iteration_cap`, `iteration_floor`)

After each iteration the run stops when both of these hold:

1. **The search has little left to find**: the iteration's found share is at most 0.5 %.
2. **The mean trip time is flat**: over the last five iterations, this one included, the greatest mean trip time less
   the least is at most 0.3 % of this iteration's mean. (The condition never holds while one of the five iterations has
   no finished trip.)

The two numbers and the five iterations are fixed constants of the rule, not parameters. Two parameters bound the rule:

| Parameter | Unit | Default | Meaning |
|---|---|---|---|
| `iteration_cap` | iterations | 50 | The run always stops after this many iterations, whatever the rule says |
| `iteration_floor` | iterations | 0 | The rule is not read before this many iterations have run |

Since the rule needs five mean trip times, no run stops by the rule before five iterations. Setting the floor equal to
the cap runs a fixed number of iterations. `Run.stopped_by_rule`, and `stopped_by_rule` in the run's record, say whether
the rule or the cap ended the run.

**What the rule does and does not say.** The rule reads the slope of the mean trip time. Because only a share of the
travellers is offered a new choice at each iteration, that slope is about the remaining distance times the step, and the step shrinks.
A run that the rule stops can therefore still be short of equilibrium. How far is what the share who would change
(logit) or the duality gap (least-cost choice) of the last row reports.

## Defaults

% src: PARAMETERS.md; crates/binding/README.md ("Keywords")

| Parameter | Unit | Default | What the iteration does with it |
|---|---|---|---|
| `choice` | name | `"logit"` | The choice model: `"logit"` (nests by mode) or `"deterministic"` (the least-cost choice) |
| `step_exponent` | none | 0.75 | The exponent $a$ of the share offered a new choice under the logit; not read under the least-cost choice |
| `iteration_cap` | iterations | 50 | The most iterations |
| `iteration_floor` | iterations | 0 | The iterations before which the stopping rule is not read |
| `max_car_routes` | routes per origin-destination pair | 6 | The distinct car routes the trips of one origin-destination pair may gather |
| `seed` | none | 0 | The tastes of the travellers and who is offered a new choice ([Reproducibility](reproducibility.md)) |

All are keywords of `run`, or keys under `[parameters]` of a settings file. Their sources are on the
[parameters page](../reference/parameters.md).

## An example: the Delft case

% src: a run of the Delft case (doi:10.5281/zenodo.24585967) with every parameter at its default; the rule above applied by hand; the same case run with `iteration_cap = 1` (the 18 049 trips); crates/loop/src/assess.rs (the share who would change)

The iterations table of the run of the Delft case made on [What a run gives](outputs.md):

```python
import pyarrow.parquet as pq

rows = pq.read_table("delft-run/iterations.parquet").to_pylist()
for r in rows:
    print(r["iteration"], f"{r['mean_trip_time_s']:.1f}", r["unfinished"],
          f"{100 * r['offered_share']:.1f}", f"{100 * r['switch_share']:.2f}",
          f"{100 * r['found_share']:.2f}", f"{100 * r['would_change_share']:.2f}")
```

```text
0 1282.0 4689 100.0 0.00 100.00 0.83
1 959.5 0 59.2 18.76 34.52 14.33
2 906.1 0 43.5 8.75 15.73 13.69
3 878.3 0 35.4 5.74 9.88 11.68
4 868.7 0 30.2 3.78 2.49 9.30
5 865.9 0 26.3 2.71 1.81 7.65
6 866.3 0 23.2 1.94 1.84 6.50
7 864.3 0 21.3 1.47 0.73 5.60
8 865.4 0 19.5 1.08 0.52 4.90
9 865.5 0 17.7 0.96 0.55 4.34
10 865.2 0 16.6 0.68 0.36 3.89
```

| Iteration | Mean trip time of finished trips (s) | Unfinished trips | Offered a new choice (% of trips) | Switched (% of trips) | Found a cheaper alternative (% of trips) | Would change (% of trips) |
|---|---|---|---|---|---|---|
| 0 | 1 282.0 | 4 689 | 100.0 | 0.00 | 100.00 | 0.83 |
| 1 | 959.5 | 0 | 59.2 | 18.76 | 34.52 | 14.33 |
| 2 | 906.1 | 0 | 43.5 | 8.75 | 15.73 | 13.69 |
| 3 | 878.3 | 0 | 35.4 | 5.74 | 9.88 | 11.68 |
| 4 | 868.7 | 0 | 30.2 | 3.78 | 2.49 | 9.30 |
| 5 | 865.9 | 0 | 26.3 | 2.71 | 1.81 | 7.65 |
| 6 | 866.3 | 0 | 23.2 | 1.94 | 1.84 | 6.50 |
| 7 | 864.3 | 0 | 21.3 | 1.47 | 0.73 | 5.60 |
| 8 | 865.4 | 0 | 19.5 | 1.08 | 0.52 | 4.90 |
| 9 | 865.5 | 0 | 17.7 | 0.96 | 0.55 | 4.34 |
| 10 | 865.2 | 0 | 16.6 | 0.68 | 0.36 | 3.89 |

How to read it:

- **Iteration 0** is the choice on empty roads: 4 689 of the 37 649 trips do not finish within the window, and the mean
  of the others is 1 282 s. Its mean is over fewer trips than the later means, so it is not comparable with them
  ([The loading](loading.md#the-window-and-unfinished-trips)).
- **The 0.83 % of iteration 0 does not mean the run is near its fixed point.** The share is counted over the sets
  found so far, and after iteration 0 a set holds one car route, so a traveller can only change mode; 18 049 trips
  with the car as their only alternative cannot change at all. From iteration 1 the sets hold second car routes and
  the share shows what is left.
- **The offered share follows the formula**: 59.2 % at iteration 1 and 16.6 % at iteration 10, against
  $1/2^{0.75} = 59.5\ \%$ and $1/11^{0.75} = 16.6\ \%$. Each traveller is drawn independently, so the share
  offered scatters around the formula's value.
- **The rule stops the run after iteration 10.** Its found share is 0.36 %, at most 0.5 %; and the means of iterations
  6 to 10 lie between 864.3 s and 866.3 s, a range of 2.0 s, which is at most 0.3 % of 865.2 s (2.6 s). After iteration
  9 the first condition failed: its found share was 0.55 %.
- **The run is near, not at, its fixed point**: at the end 3.89 % of the travellers would still take another alternative
  if offered a new choice.

## For bikes

% src: crates/loop/README.md ("Bikes"; "Search": bikes at iteration 0 only; "Measure")

A bike alternative is a route on the links a bike may use, at the bike's speed, in the same set and the same choice as
the car's and transit's, with the bike mode as its nest. It is searched once, at iteration 0, and its time never
changes, since a bike does not queue. A bike traveller is offered a new choice at the same rate as every other
traveller, and chooses over its whole set, so a change in the car's cost changes how the bike compares with it. The
counts and shares of every iteration include the bike trips. A trip whose door is so far from the bike network that
its walk there would end after the window loses the bike for the run.

## Limits

% src: README.md ("Known limits"); crates/loop/README.md ("Stop", "What the choice is over", "The window and the mean", "Who chooses again": the limits of the least cost's rule, the sets grown at another step exponent); PARAMETERS.md (`step_exponent`, `iteration_floor`)

- **A run the rule stops can be short of equilibrium** (above). In the test of two car routes, one behind a
  bottleneck, a run of the default 50 iterations under the default model has the trip by the bottleneck route within
  2.3 ± 1.0 s of theory (over 20 seeds) in the group of departures farthest from it; with the step exponent at 1 it is
  9.5 ± 1.9 s, and halves when the iterations double.
- **The choice is over the sets found, not over every route.** A route that is never the fastest is never offered, and
  of two routes of exactly equal time the search's tie rule finds one. A logit share between two routes exists only
  once both were found.
- **The settled state depends on the path taken to it.** The sets grow by the search at the costs each iteration meets,
  so another step exponent gives other sets and can settle at another state: on Sioux Falls the sets grown at 0.75
  settle about 9 s of mean trip time (1.2 %) above those grown at 1, in every seed, more than the spread between
  seeds.
- **The mean trip time is over finished trips only**, and the stopping rule reads it. When the unfinished count differs
  between two iterations, their means are over different trips.
- **The distance to equilibrium is reported, never judged.** No value of the duality gap or of the share who would change
  stops or fails a run.
- **The least-cost rule has its own limits.** The saving $r$ is the same in any unit of cost but shrinks when a constant
  is added to every cost (a long shared leg, a mode constant); it favours short trips (5 min lost beside a 10 min best
  is $r = 0.33$, beside a 60 min best 0.077); above the band the rate does not fall with $k$, so travellers behind one
  queue who read the same cost can move together; and the chance jumps at the band.
- **Old choices linger at step exponent 1 and under the least-cost choice inside the band**: after $K$ iterations about
  one traveller in $K$ still holds the choice it made at each earlier iteration, on that iteration's costs.
- **The departure time is not chosen.** The iteration settles modes and routes for the departures given.

All limits of the model are gathered in [Limitations](limitations.md).

## References

- Sheffi, Y. (1985). *Urban Transportation Networks: Equilibrium Analysis with Mathematical Programming Methods*.
  Prentice-Hall, Englewood Cliffs.
- Robbins, H. and Monro, S. (1951). A stochastic approximation method. *The Annals of Mathematical Statistics* 22(3),
  400–407.
- Smith, M. J. (1984). The stability of a dynamic model of traffic assignment: an application of a method of Lyapunov.
  *Transportation Science* 18(3), 245–252.
- Mahmassani, H. S. and Chang, G.-L. (1987). On boundedly rational user equilibrium in transportation systems.
  *Transportation Science* 21(2), 89–99.
- Ben-Akiva, M. and Bierlaire, M. (1999). Discrete choice methods and their applications to short term travel
  decisions. In R. W. Hall (ed.), *Handbook of Transportation Science*, Kluwer, 5–33. (The path size.)

The full list is in the [bibliography](../reference/bibliography.md).
