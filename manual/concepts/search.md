# Routes and route sets

% src: crates/search/README.md; crates/loop/README.md ("Search", "Car routes per origin-destination pair", "Measure", "What the choice is over"); crates/choice/README.md; PARAMETERS.md

A traveller chooses among **alternatives**: ways of making the trip, each by one mode. This page explains where the
alternatives come from. For the car, the bike and walking, an alternative is a **route**, a chain of links, found by
a search for the fastest route. The alternatives found for a traveller are kept in that traveller's **set**, which
grows over the iterations of a run as the car traffic changes the fastest car route.

This is the third of four concepts that build on one another. It takes the [network](networks.md) (the links, the
modes allowed on each, the free-flow times) and the [demand](demand.md) (for every trip and mode, the node at each
end; for transit, the stops in walking reach). It gives the [choice](choice.md) one set of alternatives per
traveller, each alternative with its travel time and, for a car route, its path size. The choice model takes one
alternative of the set; it never looks for a route itself.

The search is one of the five stages of an iteration (search, choose, load, measure, assess); [The
iteration](iteration.md) describes the loop and its stopping rule, [The loading](loading.md) how the link times
arise, and [Transit](transit.md) how journeys on the timetable are found.

```{image} ../_static/figures/route_set_growth_paper.svg
:alt: A traveller's set of alternatives over three iterations: each search adds what is new, and nothing is removed
:class: only-light
:width: 100%
```
```{image} ../_static/figures/route_set_growth_night.svg
:alt: A traveller's set of alternatives over three iterations: each search adds what is new, and nothing is removed
:class: only-dark
:width: 100%
```

## An alternative

% src: crates/loop/README.md ("An alternative", "Money", "Measure")

An **alternative** is a mode and three legs:

| Leg | What it is | Where its time comes from |
|---|---|---|
| a walk | the gap between the origin door and its node, or the walk to a stop | the demand ([Demand](demand.md)) |
| a route or a journey | a route on the network by car, by bike or on foot, or a journey on the timetable | the search (this page), or the transit search ([Transit](transit.md)) |
| a walk | the gap between the last node and the destination door, or the walk from a stop | the demand |

A route is empty when the origin's node is the destination's node: the trip is then its two walks. The travel time of
an alternative is the sum of its legs, door to door.

## The fastest route

% src: crates/search/README.md ("Inputs", "Outputs", "Meanings and assumptions")

A **route** from an origin node to a destination node is a chain of links, each allowed to the mode, the downstream
node of one being the upstream node of the next. The search finds the **fastest** route for a traveller who sets out
at a given instant, where fastest means **earliest arrival**.

The time of a route is walked link by link. A traveller who enters link $a_k$ at instant $t_k$ leaves it at the exit
instant the link times give for that entry, and enters the next link at once:

$$
t_{k+1} = \operatorname{exit}_{a_k}(t_k), \qquad t_0 = \text{the departure}, \qquad \text{arrival} = t_n
$$

for a route of $n$ links. Nothing else costs time: no turn, no delay at a node, no waiting at a node. The **link
times** are those of the last loading of the network; before any loading they are the free-flow times:

| Mode | Time to cross a link entered at instant $t$ | Does it depend on $t$? |
|---|---|---|
| car | the time the last loading gives a car entering at $t$: the free-flow time plus any time in the queue; never below the free-flow time | yes |
| bike | length / `bike_speed` | no |
| walk | length / `walk_speed` | no |

So a car's fastest route depends on when it sets out and on the queues of the last loading, and it can change from
one iteration to the next. A bike's or a walker's fastest route is its shortest route by length, from any departure,
whatever the cars do.

**The method.** The search is Dijkstra's label-setting algorithm (Dijkstra 1959) on the arrival instant, with one
label per node. It is exact on link times that are first-in-first-out, meaning that a traveller who enters a link
later never leaves it earlier, and that depend on the instant of entry alone (Kaufman and Smith 1993). The link times
of the loading are first-in-first-out by construction. The search is directed towards its goal in the manner of A\*
(Hart, Nilsson and Raphael 1968): it adds to each label a lower bound on the rest of the way, the mode's free-flow
time from the node to the destination. The bound is valid because no link time is below free flow; it changes how
much of the network is explored, never the route found.

**Ties.** Two routes that reach a node at exactly the same instant are decided by the shorter in metres, then the
fewer links, then the lower number of the link arriving at the node. Of several routes of exactly equal time, the
search therefore finds one, the same in every run.

**No route.** When the destination cannot be reached on links the mode may use, there is no route, and the traveller
has no alternative by that mode.

## How a set grows

% src: crates/loop/README.md ("Search", "An alternative", "What the choice is over", "Measure")

Each traveller has a **set** of alternatives. The search stage of each iteration adds to it; nothing is ever removed
or replaced.

| Iteration | Car | Bike | Walk | Transit |
|---|---|---|---|---|
| 0 | the fastest route at free flow | the fastest route | the fastest route | the journeys of the transit search |
| 1 and later | the fastest route at the link times of the last loading, added when it is not yet in the set | nothing new | nothing new | nothing new |

Only the modes open to the trip are searched. Bikes, walkers and transit are searched at iteration 0 only, because
their times never change during a run: a bike and a walker do not queue, and transit runs on its timetable. This is a
property of the model, not a setting. Cars are searched at every iteration, each at the instant its walk to the
network ends, on the link times of the last loading.

So the set of a traveller holds, per mode, the fastest route at iteration 0; for the car, in addition, every route
that was the fastest at the link times of some later iteration; and the transit journeys. It is **not every route of
the network**: a car route that is never the fastest at any iteration is never offered. The shares of a run are
those of a choice over these sets.

**An alternative keeps its position.** Because a set only grows at its end, an alternative keeps its position in the
set for the whole run. The choice model ties each traveller's random taste for an alternative to that position, so the
taste does not change when the set grows (see [Choice](choice.md)).

**Found.** A new alternative counts as *found* when its cost, without the path-size term, is strictly below that of
every alternative of the same mode already in the set; the first alternative of a mode always counts. The **found
share** of an iteration is the share of trips, among those with an alternative, for which the search found such an
alternative. It falls as the sets fill up, and the stopping rule reads it (see [The iteration](iteration.md)).

**The travel time of a car alternative is measured again after every loading**, for every car route in every set,
taken or not, by walking it on the new link times as its traveller. The choice of the next iteration therefore
compares all car routes of a set at the same, latest link times.

**Transit journeys.** A trip with transit open to it is offered, at iteration 0, the journeys the transit search
finds between the stops of its access and egress lists: the earliest journey for each number of rides, the cheapest
journey for its class for each number of rides, and the same from the stops nearest each trip end tried alone
(`nearby_stops`). Of these, at most `max_transit_journeys`, the cheapest for the traveller, are kept. [Transit](transit.md)
gives the method.

## The limit on car routes per origin-destination pair

% src: crates/loop/README.md ("Car routes per origin-destination pair"); PARAMETERS.md (max_car_routes)

An **origin-destination pair** is here the trips with the same car origin node and the same car destination node. The
pair *holds* the distinct car routes in the sets of its travellers. Without a limit, a pair on a large network
gathers many near copies of one route (the [parameters table](../reference/parameters.md) reports up to 22 on a
city case, differing only on parallel motorway carriageways). The parameter `max_car_routes` limits them:

- A car route that is new to its traveller's set and new to the pair is added only while the pair holds fewer than
  `max_car_routes` routes. The trips are taken in the order of their identifiers, so the result does not depend on
  the number of threads.
- Otherwise the route is not offered and not counted as found. It replaces no route: a faster route found by a pair
  at its limit is lost.
- A route that the pair already holds can still be added to the set of another traveller of the pair.
- A traveller's first car route is always added, so no car driver is left without a route. A pair whose travellers'
  first routes differ can therefore hold more routes than the limit.
- A limit at least as large as the number of routes any pair finds changes nothing.

| Parameter | Default | Unit | What it limits | Measured effect reported in the parameters table |
|---|---|---|---|---|
| `max_car_routes` | 6 | routes per origin-destination pair | the distinct car routes of a pair | on the Delft case at 6, the mean car trip is 1.3 % longer than without a limit (8.31 min against 8.21 min) |
| `max_transit_journeys` | 10 | journeys per trip | the transit journeys offered to a traveller | 10 is the smallest limit that left no trip unfinished on three example cases |

## The path size of overlapping car routes

% src: crates/loop/src/alternative.rs (path_sizes); crates/loop/README.md ("Measure", "Search", "Tests"); crates/choice/README.md ("Meanings and assumptions")

Car routes of one set often share links. Two routes that differ on one short link are nearly one route, and a choice
model that treated them as two separate options would send too many travellers along their shared part. The **path
size** of a route measures how much of it is its own (Ben-Akiva and Bierlaire 1999):

$$
PS_i = \sum_{a \in i} \frac{\ell_a}{L_i} \cdot \frac{1}{n_a}
$$

where the sum runs over the links $a$ of route $i$, $\ell_a$ is the length of link $a$ in metres, $L_i$ the length of
the route (the sum of its links' lengths), and $n_a$ the number of car routes in the traveller's set that use link
$a$. A route that shares no link has a path size of 1. Of $n$ identical routes each has $1/n$. The path size lies
between $1/n$ and 1 for a set of $n$ car routes.

The path sizes are computed over the car routes of each traveller's own set, and rewritten at once whenever that set
grows, so the choice of an iteration reads the path sizes of the set it chooses from. They depend on which routes are
in the set and on the links' lengths, not on the link times. A lone car route, an empty route, a bike route, a walk
route and a transit journey have a path size of 1.

The search hands the choice model the natural logarithm, $\ln PS_i$, which is 0 for a path size of 1 and negative
otherwise. How it enters the cost of a car route is on the [Choice](choice.md) page.

**By hand.** Three car routes of 800 m, 1 000 m and 1 200 m share their first 600 m and nothing else. The shared
part is used by three routes, the rest of each route by one:

| Route | Length (m) | Shared part: (600 / length) / 3 | Own part: (length − 600) / length | Path size |
|---|---|---|---|---|
| 1 | 800 | 0.25 | 0.25 | 0.5 |
| 2 | 1 000 | 0.2 | 0.4 | 0.6 |
| 3 | 1 200 | 0.1667 | 0.5 | 0.6667 |

A test of the simulator checks these three values in a run (`crates/loop/tests/path_size_in_a_run.rs`).

## The sets of the Delft case

% src: a run of the published case `delft` (doi:10.5281/zenodo.24585967) with its own settings, read with pyarrow

The run of the published case `delft` shows how the sets grow. The trips of the classes `from_outside_car` and
`goods` have the car as their only mode, so the size of their set is their number of car routes:

```python
import collections
import openmobisim as oms
import pyarrow.parquet as pq

run = oms.run_file("delft", out="delft-run")

trips = pq.read_table(run.trips).to_pydict()
drivers = [n for n, c in zip(trips["alternatives"], trips["class_id"]) if c in ("from_outside_car", "goods")]
sizes = collections.Counter(drivers)
for size in sorted(sizes):
    print(size, sizes[size], round(100 * sizes[size] / len(drivers), 1))

iterations = pq.read_table(run.iterations).to_pydict()
for k, found in zip(iterations["iteration"], iterations["found_share"]):
    print(k, round(100 * found, 2))
print(run.stopped_by_rule)
```

```text
1 4722 26.2
2 5728 31.7
3 4276 23.7
4 2170 12.0
5 812 4.5
6 341 1.9
0 100.0
1 34.52
2 15.73
3 9.88
4 2.49
5 1.81
6 1.84
7 0.73
8 0.52
9 0.55
10 0.36
True
```

| Car routes in the set at the end of the run (count) | Trips with the car as their only mode (count) | Share of those trips (%) |
|---|---|---|
| 1 | 4 722 | 26.2 |
| 2 | 5 728 | 31.7 |
| 3 | 4 276 | 23.7 |
| 4 | 2 170 | 12.0 |
| 5 | 812 | 4.5 |
| 6 | 341 | 1.9 |

| Iteration | Found share (% of the trips with an alternative) |
|---|---|
| 0 | 100.00 |
| 1 | 34.52 |
| 2 | 15.73 |
| 3 | 9.88 |
| 4 | 2.49 |
| 5 | 1.81 |
| 6 | 1.84 |
| 7 | 0.73 |
| 8 | 0.52 |
| 9 | 0.55 |
| 10 | 0.36 |

At iteration 0 every trip finds its first alternatives. From then on only car routes are new, and fewer at each
iteration; the run stopped by its rule after the iteration at which 0.36 % of the trips found a cheaper alternative.
The run took about 4 s on the laptop that built this page.

## For bikes

% src: crates/search/README.md ("Bikes"); crates/loop/README.md ("Bikes", "Measure")

- A bike's fastest route is its shortest route by length over the links that allow bikes, in their own direction: a
  one-way cycle path is ridden one way only. Its travel time is the route's length over `bike_speed`, from any
  departure. A street with a car queue costs a bike nothing more.
- A bike route is found once, at iteration 0, and the set of a traveller holds one bike route. Its path size is 1.
- The limit `max_car_routes` does not concern bikes.
- The bike routes of the simulator are tested against the shortest paths by length of another tool, networkx, on a
  grid, and on 1 000 node pairs of the Delft network (tests 9 and 10 of [Validation](validation.md)).

## Limits

% src: crates/search/README.md ("Meanings and assumptions": ties); crates/loop/README.md ("What the choice is over", "Car routes per origin-destination pair"); README.md ("Known limits")

- **A set holds what the search found, not every reasonable route.** A car route enters a set only by being the
  fastest for that traveller at some iteration. A route that is a little slower than the fastest at every iteration
  is never offered, even where a logit model would give it a share.
- **Of two routes of exactly equal time, one is offered.**
- **One route per mode for bikes and walkers.** A cyclist has no choice between a short route and a pleasant one: the
  route is the shortest by length.
- **A pair at its limit loses faster routes** found later (`max_car_routes`).
- **A known limit of the search at a tie.** The link time a car reads depends on the instant of entry and, besides,
  on which recorded car's place the traveller takes on the link before. The tie rule does not look at that place.
  When two routes reach a node at exactly the same instant having taken different places, the route kept can arrive
  later than the route dropped; in the hand case of the test, 152 s against 148 s.
- **No turn or node costs time**, and turn restrictions are not known to the network.
- **Fastest means least time.** The route search reads times alone: the coefficients of a traveller's class do not
  change which car, bike or walk route is found for it.

## References

- Ben-Akiva, M. and Bierlaire, M. (1999). Discrete choice methods and their applications to short term travel
  decisions. In R. W. Hall (ed.), *Handbook of Transportation Science*, 5–33. Kluwer.
- Dijkstra, E. W. (1959). A note on two problems in connexion with graphs. *Numerische Mathematik* 1, 269–271.
- Hart, P. E., Nilsson, N. J. and Raphael, B. (1968). A formal basis for the heuristic determination of minimum cost
  paths. *IEEE Transactions on Systems Science and Cybernetics* 4(2), 100–107.
- Kaufman, D. E. and Smith, R. L. (1993). Fastest paths in time-dependent networks for intelligent vehicle-highway
  systems application. *IVHS Journal* 1(1), 1–11.

The full list of the manual is in the [Bibliography](../reference/bibliography.md).
