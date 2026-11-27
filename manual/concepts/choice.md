# Choice

% src: crates/choice/README.md; crates/loop/README.md ("Choose", "Assess", "Who chooses again"); crates/types/README.md; PARAMETERS.md

Every traveller of a run makes one **choice**: which alternative of its set to take. Because the set holds
alternatives of every mode open to the trip, this one choice is the choice of mode and of route at once. A **choice
model** turns the cost of each alternative into the alternative taken.

This is the fourth of the seven concepts of this part in their reading order: the network, the demand, the routes,
the choice, transit, the loading and the iteration. The [network](networks.md) gives the links and their
times; the [demand](demand.md) gives the trips, their classes and their modes; the [search](search.md) gives each
traveller a set of alternatives, each with the times of its parts and, for a car route, its path size. The choice
model takes these sets and gives, per traveller, the alternative taken. What is taken is then loaded on the network
([The loading](loading.md)), the loading changes the car times, and the search and the choice are made again ([The
iteration](iteration.md)).

The choice model knows nothing of links, routes or timetables. It reads a table of numbers per alternative and a
table of coefficients per traveller class.

```{image} ../_static/figures/choice_nests_paper.svg
:alt: The nested logit: the traveller's set split into one nest per mode; a nest is drawn first, then an alternative within it
:class: only-light
:width: 100%
```
```{image} ../_static/figures/choice_nests_night.svg
:alt: The nested logit: the traveller's set split into one nest per mode; a nest is drawn first, then an alternative within it
:class: only-dark
:width: 100%
```

## The cost of an alternative

% src: crates/choice/README.md ("Outputs", "Units", "Meanings and assumptions"); crates/loop/README.md ("Units", "Choose"); crates/types/README.md ("Units")

Each alternative is described by sixteen **attributes**, the same sixteen for every mode: nine times in minutes, the
number of transfers, the money in euros, the logarithm of the path size, and four mode constants. A traveller class
has one **coefficient** per attribute. The **cost** of alternative $j$ for a traveller of class $c$ is

$$
C_j = \sum_{k=1}^{16} \beta_{c,k} \, x_{j,k}
$$

where $x_{j,k}$ is attribute $k$ of the alternative and $\beta_{c,k}$ the class's coefficient of it. The cost is the
**systematic utility** of the choice model with its sign turned: a positive coefficient makes an alternative worse,
and **the least cost is the best**. The cost has no unit of its own; with the default coefficients one unit is worth
ten minutes of driving.

| Attribute (column) | Unit | Default coefficient (cost per unit) | Filled for | What it holds |
|---|---|---|---|---|
| `car_drive_min` | min | 0.1 | car | the time of the car route on the network |
| `bike_ride_min` | min | 0.1 | bike | the time of the bike route on the network |
| `walk_min` | min | 0.165 | car, bike, walk | for a car or a bike, its two walks between door and network; for a walk, its whole time |
| `transit_in_vehicle_min` | min | 0.1 | transit | the time riding |
| `transit_access_min` | min | 0.165 | transit | the walk from the door to the first stop |
| `transit_egress_min` | min | 0.165 | transit | the walk from the last stop to the door |
| `transit_first_wait_min` | min | 0.145 | transit | the wait from the end of the access walk to the first boarding |
| `transit_transfer_wait_min` | min | 0.145 | transit | the waits at transfers |
| `transit_transfer_walk_min` | min | 0.165 | transit | the walks between rides |
| `transit_transfers` | transfers | 0.5 | transit | the number of transfers |
| `money_eur` | € | 0.6 | every mode | 0 for every alternative in this release |
| `car_ln_path_size` | none | −1 | car | the natural logarithm of the route's path size (below) |
| `is_car` | none | 0 | car | 1: the car's constant |
| `is_bike` | none | 0 | bike | 1: the bike's constant |
| `is_walk` | none | 0.5 | walk | 1: the walk's constant |
| `is_transit` | none | 0.25 | transit | 1: transit's constant |

An alternative fills only the columns of its own mode; the others are 0. The nine times of any alternative add up to
its door-to-door time.

**What the defaults mean.** A minute of walking weighs 1.65 times a minute of riding or driving, and a minute of
waiting 1.45 times. A transfer weighs as much as five minutes in a vehicle. The coefficient of money, 0.6 per euro
beside 0.1 per minute, is a value of time of 10 € per hour (KiM 2023); since every alternative costs 0 € in this
release, it changes nothing. The walk and transit constants add the cost of 5 and 2.5 minutes of driving. **The
defaults are starting values to be calibrated for each study, not estimates.**

**Per class.** The parameter `coefficients` gives every class its own sixteen; [Traveller
classes](../guides/classes.md) shows how. A **time-only run** has a coefficient of 1 on each of the nine times and 0
on the rest: its cost is the door-to-door time in minutes. The run reports, for each trip, the cost of the
alternative it took (the column `cost` of `trips.parquet`), formed by the same sum that the choice reads.

## The two models

% src: crates/choice/README.md ("Meanings and assumptions"); PARAMETERS.md (choice, nest_scale); crates/binding/README.md ("Inputs": keywords)

| `choice` | What it does | Draws random numbers | Reads `nest_scale` | Reads the path size |
|---|---|---|---|---|
| `"logit"` (the default) | each traveller takes an alternative with the probability of a nested logit with one nest per mode; at special values it is the plain logit and the path-size logit | yes | yes | yes |
| `"deterministic"` | each traveller takes the alternative of least cost | no | no | no |

Both read the same cost. There is no third model, and none can be supplied from Python.

### The least-cost choice

% src: crates/choice/README.md ("Meanings and assumptions": Deterministic); crates/loop/README.md ("Choose", "Assess")

Under `choice="deterministic"` a traveller takes the alternative of least cost in its set, with probability 1; of
several of equal least cost, the first in the set. At the equilibrium of this model no traveller can lower its cost
by changing its alternative. The coefficient of the path size is read as 0, whatever is given: the path size
corrects an error of the logit, and the least-cost choice has no such error. The run's record keeps the coefficient
as given.

### The logit family

% src: crates/choice/README.md ("Meanings and assumptions": Logit, path-size logit); crates/loop/README.md ("Choose")

Under `choice="logit"` the alternatives of a traveller are grouped in **nests**, one per mode: its car routes in one
nest, its bike route in another, its walk route in a third, its transit journeys in a fourth. Alternatives of one
nest are more alike, in what the model does not observe, than alternatives of different nests; the **nest scale**
$\mu$, a number above 0 and at most 1, says how much. The model is the nested logit with one nest scale (Ben-Akiva
and Lerman 1985, chapter 10).

Write $v_j = -C_j / \mu$ for alternative $j$. The **inclusive value** of nest $n$, also called its logsum, is

$$
I_n = \ln \sum_{j \in n} \exp(v_j)
$$

The probability that the traveller takes nest $n$ (a mode), and the probability that it takes alternative $j$ given
its nest, are

$$
P(n) = \frac{\exp(\mu I_n)}{\sum_m \exp(\mu I_m)}, \qquad
P(j \mid n) = \frac{\exp(v_j)}{\sum_{k \in n} \exp(v_k)}
$$

and the probability of alternative $j$ is $P(j) = P(n) \, P(j \mid n)$. Two special cases follow from these
equations:

| Case | What the equations become |
|---|---|
| $\mu = 1$ | the **plain logit** over all alternatives: $P(j) = \exp(-C_j) / \sum_k \exp(-C_k)$ |
| every nest holds one alternative | the plain logit too, whatever $\mu$: then $\mu I_n = -C_j$ |

**The path-size logit.** Car routes of one set that share links are not separate options (see [Routes and route
sets](search.md), which gives the path size $PS_j$ of a route: 1 for a route that shares nothing, $1/n$ for each of
$n$ identical routes). The path-size logit (Ben-Akiva and Bierlaire 1999) corrects the logit by adding
$\beta_{PS} \ln PS_j$ to the cost of a route. Inside a nest the whole cost is divided by $\mu$, so the simulator
reads the coefficient given times $\mu$: with $\tilde C_j$ the cost without the path-size term,

$$
C_j = \tilde C_j + \mu \, \beta_{PS} \ln PS_j
\qquad\Longrightarrow\qquad
\exp(v_j) = \exp\!\left(-\tilde C_j / \mu\right) \cdot PS_j^{\,-\beta_{PS}}
$$

The coefficient given, with its sign turned, is therefore the exponent of the path size in the weight of a route
inside the car nest, at every nest scale. At the default $\beta_{PS} = -1$ the weight of a route is multiplied by its
path size: $n$ identical copies of a route, each of path size $1/n$, count together as the one route, and the other
alternatives keep their probabilities. A coefficient of 0 switches the correction off. Bike routes, walk routes and
transit journeys have a path size of 1 and no such term.

| Parameter | Default | Unit | What it sets |
|---|---|---|---|
| `choice` | `"logit"` | name | the model: `"logit"` or `"deterministic"` |
| `nest_scale` | 0.5 | none | $\mu$, in (0, 1]; 1 is the plain logit |
| `coefficients` | the table above, for every class | cost per unit of each attribute | the cost |
| `seed` | 0 | none | the random numbers of the run (below) |

### A calculation by hand

% src: the equations above, computed by hand; not an output of the simulator

A traveller has two car routes of 10 and 12 minutes and a bike route of 20 minutes, with no walks. At the default
coefficients the costs are 1.0, 1.2 and 2.0. The table gives the probabilities of the three alternatives in four
cases:

| Alternative | Cost | Plain logit, $\mu = 1$ | Nested logit, $\mu = 0.5$, path sizes 1 | Nested logit, $\mu = 0.5$, the car routes' path sizes 0.6 |
|---|---|---|---|---|
| car route 1 | 1.0 | 0.457 | 0.466 | 0.438 |
| car route 2 | 1.2 | 0.374 | 0.312 | 0.294 |
| bike | 2.0 | 0.168 | 0.222 | 0.269 |

For the middle column: $\exp(v)$ is $e^{-2} = 0.1353$ and $e^{-2.4} = 0.0907$ for the car routes, so
$I_{\text{car}} = \ln 0.2261 = -1.487$ and $\exp(\mu I_{\text{car}}) = 0.4755$; the bike's nest gives
$\exp(\mu I_{\text{bike}}) = e^{-2} = 0.1353$. Then $P(\text{car}) = 0.4755 / 0.6108 = 0.778$, shared between the two
routes as $0.1353 / 0.2261 = 0.599$ and $0.401$. Nesting moves travellers from the car to the bike, because the two
car routes are treated as similar. When the two routes also overlap (the last column), each weight is multiplied by
0.6, the car nest's $\exp(\mu I)$ falls to 0.3683, and the bike's share rises again.

## How each traveller takes one alternative

% src: crates/choice/README.md ("Meanings and assumptions": sampling, draws); crates/types/README.md ("Outputs": rng, "Assumptions": determinism); crates/types/src/rng.rs (gumbel)

The probabilities say how often an alternative is taken; a simulation needs each traveller to take one. The logit is
a random-utility model: a traveller's utility is the systematic part plus a random part that the model does not
observe, the traveller's **taste**. The simulator draws the taste and takes the best, in two stages:

1. the nest $n$ with the greatest $\mu I_n + \varepsilon_n$;
2. within that nest, the alternative $j$ with the greatest $v_j + \varepsilon_j$;

where every $\varepsilon$ is an independent draw from the standard Gumbel distribution, $\varepsilon = -\ln(-\ln u)$
for a uniform number $u$ between 0 and 1. This sampling is exact: over many travellers each alternative is taken
with the probability $P(j)$ above.

**The draws are fixed for the whole run.** A draw is a function of three things only:

| A draw depends on | Why |
|---|---|
| the run's `seed` | another seed gives another population of tastes |
| the trip | each traveller has its own taste |
| the alternative's position in the traveller's set, or the nest (the mode) | each alternative and each mode has its own draw |

A draw does not depend on the iteration, on the thread that computes it, or on the order in which travellers are
taken. Three properties follow:

- **A traveller's choice changes only when costs change** (or when the search adds an alternative). A set only grows
  at its end, so an alternative keeps its position and its draw. Between two iterations a traveller does not flip
  between alternatives by chance.
- **Two runs with the same seed, inputs and program on the same platform give identical results**, at any number of
  threads. As measured, the default run of the Delft test (test 10) gives byte-identical trips on macOS (Apple silicon), Linux
  (x86_64) and Windows (x86_64). [Reproducibility](reproducibility.md) gives the whole account.
- **The share of travellers who would change is a count, not an estimate.** After an iteration, the simulator asks
  for each traveller whether its fixed taste, at the costs just measured, still picks the alternative it holds. When
  no traveller would change, the run is at the fixed point of its iterations, for the sets it has found (see [The
  iteration](iteration.md)).

**Who chooses at each iteration.** At iteration 0 every traveller chooses. At later iterations only a share of the
travellers is offered a new choice, a share that falls with the iteration number; a traveller who is offered takes
the model's choice over its whole set, its mode included, and the others keep what they hold. This averaging, its
parameter `step_exponent` and the stopping rule are on the page [The iteration](iteration.md).

## An example run

% src: run by the writer with openmobisim 0.2; the closed form is the plain logit of the section above

The example gives 20 000 travellers the same trip of 1 500 m with three modes and no congestion (the street's
capacity is unlimited). Each mode has one alternative, so each nest holds one alternative and the nested logit is the
plain logit of the three costs. The costs by hand: the car drives 108 s, 1.8 min × 0.1 = 0.18; the bike rides 360 s,
6 min × 0.1 = 0.6; the walk takes 1 125 s, 18.75 min × 0.165 + 0.5 = 3.59375.

```python
import collections, math
import openmobisim as oms
import pyarrow.parquet as pq

nodes = {"node_id": ["a", "b"], "x_m": [0.0, 1500.0], "y_m": [0.0, 0.0]}
links = {
    "link_id":        ["street", "path"],
    "upstream":       ["a", "a"],
    "downstream":     ["b", "b"],
    "modes":          ["car", "bike;walk"],
    "length_m":       [1500.0, 1500.0],
    "lanes":          [1, None],
    "car_speed_km_h": [50.0, None],
    "capacity_veh_h": [float("inf"), None],
}
network = oms.Network.from_tables(nodes, links)

n = 20000
trips = {
    "trip_id":         [f"t{i:05d}" for i in range(n)],
    "class_id":        ["adult"] * n,
    "departure_s":     [28800 + i % 3600 for i in range(n)],
    "modes":           ["car;bike;walk"] * n,
    "origin_x_m":      [0.0] * n,
    "origin_y_m":      [0.0] * n,
    "destination_x_m": [1500.0] * n,
    "destination_y_m": [0.0] * n,
}
demand = oms.Demand.from_table(trips, ["adult"], network)
run = oms.run(network, demand, window=(28800, 36000), out="choice-run")

table = pq.read_table(run.trips).to_pydict()
taken = collections.Counter(table["mode"])
cost = {mode: c for mode, c in zip(table["mode"], table["cost"])}
total = sum(math.exp(-c) for c in cost.values())
for mode in ("car", "bike", "walk"):
    print(mode, round(cost[mode], 5), round(math.exp(-cost[mode]) / total, 4), round(taken[mode] / n, 4))
```

```text
car 0.18 0.5917 0.5887
bike 0.6 0.3888 0.3913
walk 3.59375 0.0195 0.02
```

| Mode | Cost reported by the run | Share by the logit formula | Share of the 20 000 travellers in the run |
|---|---|---|---|
| car | 0.18 | 0.5917 | 0.5887 |
| bike | 0.6 | 0.3888 | 0.3913 |
| walk | 3.59375 | 0.0195 | 0.0200 |

The costs are the hand values. The shares of the run differ from the formula by sampling error: with 20 000
travellers the standard deviation of the car share is 0.0035. The run takes under a second.

## What is validated against closed forms

% src: crates/choice/README.md ("Tests"); crates/loop/README.md ("Tests"); README.md ("The ten tests", test 7)

Every expected value below comes from outside the simulator: a formula worked by hand or written out in the test.
These tests are test 7 of the ten tests of the release ([Validation](validation.md)).

| What is tested | Expected value | Judged by |
|---|---|---|
| the plain logit: six alternatives with costs $\ln(21/k)$ | probabilities $k/21$ | the probabilities within $10^{-12}$; 200 000 draws by chi-square |
| the nested logit: three nests of three, one and two alternatives, at $\mu$ of 0.2, 0.5, 0.9 and 1 | the equations above | the probabilities within $10^{-12}$; 20 000 draws each by chi-square |
| the path-size logit: three car routes of equal time, two of them sharing a fraction $\varphi$ of their length | $P = 1/(3 - \varphi)$ for the route that shares nothing: 0.4545 at $\varphi = 0.8$, 0.4 at $\varphi = 0.5$ | within $10^{-12}$ |
| the path size inside a nest, at $\mu$ of 0.5 and 0.25: three car routes of path sizes 0.5, 0.6 and 2/3 and a bike | the closed form with the path size at the exponent 1 | within $10^{-12}$ |
| one route split into two copies of half its path size | every other probability unchanged, each copy half | by hand |
| two classes over the same alternatives | $1/(1 + e^{3})$ and $1/(1 + e^{-3})$ | by hand |
| the least cost and its ties | the first of the least costs | by hand |
| in a run without congestion: car, bike, walk and transit | the closed-form logit shares | sampling error |
| in a run: two transit journeys in one nest against a bike, at $\mu$ of 0.5 and 1 | the nested logit's closed form | sampling error |
| in a run: the path sizes of three car routes of 800, 1 000 and 1 200 m sharing 600 m | 0.5, 0.6 and 2/3 | by hand |
| in a run: the cost reported for a trip | to the bit, the cost the choice reads | 200 alternatives of each mode, two classes |
| 10 000 travellers, twice, in a shuffled order and on eight threads | the same choices | identity (the one comparison of the simulator with itself) |

## For bikes

% src: crates/choice/README.md ("Bikes"); crates/loop/README.md ("Bikes", "Choose", "Measure")

- A bike route is an alternative like any other, in the same set and the same table as the car's and transit's. Its
  cost is its riding minutes times the coefficient of `bike_ride_min`, its two walks between door and network times
  the coefficient of `walk_min`, and the constant `is_bike`. By default a minute on the bike weighs the same as a
  minute in a car, and the bike has no constant.
- The bike is a nest of its own. A traveller's set holds one bike route, so within the nest there is nothing to
  choose, and the probability of the bike is $\exp(-C_{\text{bike}})$ over the sum of $\exp(\mu I_m)$ of all nests.
- A bike's cost does not change during a run, because its time does not. Its share changes all the same, when the
  car times change.
- A bike route has a path size of 1.
- The nested logit with a bike nest is tested against its closed form (the table above).

## Limits

% src: crates/choice/README.md; crates/loop/README.md ("Choose", "What the choice is over", "Money"); PARAMETERS.md (coefficients); README.md ("Known limits")

- **The default coefficients are not calibrated.** They are starting values in plausible proportions.
- **Money is 0 for every alternative** in this release: no fare, fuel cost, toll or parking charge enters a cost.
- **The cost is linear in sixteen fixed attributes.** No other attribute, no product of two attributes and no
  attribute of the traveller beyond its class can be added, and no model can be supplied from Python.
- **One nest scale for all nests, and the nests are the modes.** Other nesting structures cannot be chosen.
- **The choice is over the set the search found**, not over every route of the network: a logit share between two
  routes exists only once both were found (see [Routes and route sets](search.md)).
- **The path-size correction is for car routes only.** Transit journeys that share a ride are in one nest but are not
  corrected for their overlap.
- **A traveller chooses mode and route, not departure time or destination.**
- **A run stopped by its rule is near, not exactly at, the model's equilibrium** (see [The iteration](iteration.md)
  and [Limitations](limitations.md)).

## References

- Ben-Akiva, M. and Bierlaire, M. (1999). Discrete choice methods and their applications to short term travel
  decisions. In R. W. Hall (ed.), *Handbook of Transportation Science*, 5–33. Kluwer.
- Ben-Akiva, M. and Lerman, S. R. (1985). *Discrete Choice Analysis: Theory and Application to Travel Demand*. MIT
  Press. (Chapter 10: the nested logit.)
- KiM Netherlands Institute for Transport Policy Analysis (2023). *Values of time and reliability in passenger
  transport*. (The source of the default value of time, 10 € per hour.)

The full list of the manual is in the [Bibliography](../reference/bibliography.md).
