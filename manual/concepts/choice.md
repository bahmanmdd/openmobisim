# Choice

% src: crates/core-choice/src/{lib.rs, model.rs, builtin.rs, batch.rs}; crates/core-sim/src/route_choice.rs;
% crates/core-sim/src/itinerary_choice.rs; python/openmobisim/choice.py; python/openmobisim/scenario.py

## What it is

Every trip in a run makes a choice. A trip that drives chooses a **route** among the routes of its
origin-destination pair. A trip by transit, park-and-ride or bike-and-ride, or a trip that chooses its mode, chooses an
**itinerary**: a door-to-door alternative such as "walk, ride tram 2, change to bus 15, walk", or "drive to car park X,
park, ride the metro, walk" (see [Modes and itineraries](modes.md)). A **choice model** decides which alternative each
trip takes.

Both kinds of choice use the same model and the same vocabulary of **attributes**: named numbers that every alternative
carries, such as its time in minutes or its cost in euros. So one set of coefficients describes every choice in a run.
A **situation** is one traveller's one trip at one iteration, with the alternatives offered to it.

Choices are made before each loading of the network: on free-flow times first, and later on the times the last loading
produced (see [Equilibration](equilibration.md)).

## The models

% src: core-choice/src/model.rs Registry::builtin; python/openmobisim/scenario.py Scenario.__init__ (choice_model unsaid:
% "nested_logit" when the run offers more than one mode, "logit" otherwise)

| Name | What it does | Draws random numbers |
|---|---|---|
| `logit` (the default of a `Scenario` that offers one mode) | A random-utility model, linear in named attributes, sampled exactly | yes |
| `nested_logit` (the default when the run offers more than one mode) | The same utility, with the alternatives of each mode in a nest of their own | yes |
| `deterministic` | Everyone takes the alternative with the least `time_min` (the first of a tie), with probability 1: for debugging or as an upper bound | no |
| your own | Any Python object with a `choose(batch)` method (below) | as it says |

{py:func}`openmobisim.choice_models` lists the built-in names. A model is chosen with `Scenario.from_parts(...,
choice_model=..., choice_options=...)`.

### The logit

% src: core-choice/src/builtin.rs Logit, Logit::utilities, Logit::default; model.rs logit_probabilities,
% sample_random_utility, gumbel

**The probabilities.** Each alternative $i$ offered in a situation is taken with the probability of the multinomial
logit {footcite}`mcfadden1974`:

$$
P_i = \frac{\exp V_i}{\sum_j \exp V_j}
$$

where $V_i$ is the alternative's **utility** and the sum runs over the alternatives offered. The probabilities sum to
one; an alternative with a higher utility is likelier, never certain. Only differences of utility matter: a term that
is the same for every alternative cancels out of $P_i$. Train's chapter 3 derives the model and its properties
{footcite}`train2009`. The code subtracts the largest utility from every $V_j$ before taking the exponentials, which
gives the same probabilities and keeps the exponentials from overflowing.

**The utility** is a weighted sum of the alternative's attributes, linear in its coefficients, the usual form of applied
choice models {footcite}`benakiva1985`:

$$
V_i = \sum_a \beta_a \, x_{a,i} + \sum_{(a,b)} \beta_{a*b} \, x_{a,i} \, x_{b,i}
$$

where $x_{a,i}$ is attribute $a$ of alternative $i$ and $\beta_a$ its **coefficient**, given by name as
`beta_<attribute>` in `choice_options`. The second sum holds the **product terms**, `beta_<a>*<b>`, also called
interaction terms: each weighs the product of two attributes, and the two names may come in either order. None is set
by default. A product term is how a characteristic of the traveller, which is the same for every alternative of a
situation and so cancels out on its own, enters the model. In Train's example (his section 3.3.1) {footcite}`train2009`, a
household's taste for a car's shoulder room grows with the household's size and the weight of its price falls with its
income: the utility holds shoulder room times size and price divided by income, a standard logit with two interaction
terms. Here, for instance:

- `beta_mode_car*person_age` weighs the car's constant (`mode_car`, 1 for a car alternative and 0 for the others)
  times the traveller's age, so the car's appeal changes with age;
- `beta_cost_eur*person_income_inv` weighs the cost times one over the traveller's income (a traveller value
  `income_inv` that you give), so a euro weighs less for a higher income. Train and McFadden derive, from the trade-off
  between goods and leisure, when the cost should be divided by the wage and when multiplied by it
  {footcite}`train1978`.

**How each traveller takes one alternative.** The probabilities say how often each alternative is taken; a simulation
needs each traveller to take one. The logit is a **random-utility model**: the traveller takes the alternative with the
largest $V_i + \varepsilon_i$, where $\varepsilon_i$ is the part of the utility the model does not observe. When the
$\varepsilon_i$ are independent standard Gumbel errors (type I extreme value), the probability that alternative $i$ has
the largest is exactly $P_i$: this is how McFadden derived the logit {footcite}`mcfadden1974`, and how Train's section 3.1
derives it {footcite}`train2009`. The simulator does just this for each traveller. It draws one error per alternative
from a uniform random number $U_i$ between 0 and 1, by the inverse of the Gumbel distribution (Train's section 9.2.3), and
takes the alternative with the largest sum:

$$
\varepsilon_i = -\ln(-\ln U_i), \qquad i^* = \arg\max_i \left( V_i + \varepsilon_i \right).
$$

So each traveller makes one definite choice, and over many travellers each alternative is taken with probability
$P_i$. The run records, for each trip, $P_{i^*}$: the probability of the alternative it took.

**An example.** Three routes between the same origin and destination share no link, so their path size is 1 and only
their times count. At the default `beta_time_min` of −0.2 per minute:

| Route | Time (min) | Utility $V$ | $\exp V$ | Probability $P$ |
|---|---|---|---|---|
| A | 20 | −4.0 | 0.0183 | 0.491 |
| B | 22 | −4.4 | 0.0123 | 0.329 |
| C | 25 | −5.0 | 0.0067 | 0.180 |
| All three | | | 0.0373 | 1 |

One traveller's draw:

| Route | Utility $V$ | Uniform $U$ | Error $\varepsilon = -\ln(-\ln U)$ | $V + \varepsilon$ |
|---|---|---|---|---|
| A | −4.0 | 0.30 | −0.19 | −4.19 |
| B | −4.4 | 0.80 | 1.50 | −2.90 |
| C | −5.0 | 0.55 | 0.51 | −4.49 |

This traveller takes route B, whose $V + \varepsilon$ is the largest, although A is the likelier route; the run records
0.329 for the trip. The next traveller draws numbers of their own and may take A or C. Of many travellers, 49 % take A,
33 % B and 18 % C.

The same choice could be drawn with one uniform number against the cumulative probabilities: route A below 0.491, B
below 0.820, C above. Both draws are exact. The simulator gives each alternative a random number of its own instead,
keyed on the alternative (see [The random numbers](#the-random-numbers)). A choice then does not depend on the order in
which the alternatives are listed, and two scenarios run with the same seed keep the same random numbers for every
alternative a change leaves alone, so they differ only where the change makes them differ: the *common random numbers*
of simulation experiments {footcite}`glasserman1992`.

**The default coefficients** of `logit` and `nested_logit`:

% src: core-choice/src/builtin.rs Logit::default; DEFAULT_ATTRIBUTES

| Coefficient | Default | Unit | What it weighs |
|---|---|---|---|
| `beta_time_min` | −0.2 | per minute | the time door to door (`time_min`) |
| `beta_ln_path_size` | 1 | per unit | the log of the path size (below) |
| `beta_walk_min` | −0.13 | per minute | minutes walking, **on top of** their share of `time_min` |
| `beta_wait_min` | −0.09 | per minute | minutes waiting at stops, on top of `time_min` |
| `beta_transfers` | −1 | per transfer | each change of vehicle |
| `beta_bike_mixed_km` | −0.16 | per km | each kilometre of a bike leg in mixed traffic, on top of its time |
| `beta_cost_eur` | −1.2 | per euro | what the alternative costs |

Every other attribute's coefficient is 0 unless you set it. Walking then weighs $(0.2 + 0.13)/0.2 = 1.65$ times a
minute in a vehicle and waiting $(0.2 + 0.09)/0.2 = 1.45$ times, close to the averages of Wardman's review of British
evidence (1.66 and 1.47) {footcite}`wardman2004`. A transfer weighs as much as five minutes in a vehicle; Garcia-Martinez
et al. estimate a transfer at 15.2 to 17.7 minutes in Madrid {footcite}`garciamartinez2018`, so the default is on the low
side. A coefficient of −1.2 per euro at −0.2 per minute is a value of time of 10 € per hour: in general
`beta_cost_eur = beta_time_min · 60 / V` for a value of time of `V` euros per hour. Every price is 0 by default (see
[Modes and itineraries](modes.md)), so money changes nothing until you give prices.

**These defaults are assumptions, not a calibration.** At −0.2 per minute, a route four minutes slower than another is
taken with $e^{-0.8} \approx 0.45$ times its weight, other things equal. Estimate your own coefficients and pass them;
[Parameters](../reference/parameters.md) lists every one.

A coefficient you give by name must have its attribute on offer: otherwise the run refuses to start and lists the
attributes that are. A default coefficient whose attribute a situation lacks counts that attribute as 0.

### The nested logit

% src: core-choice/src/builtin.rs NestedLogit (situation_probabilities, choose, logsums); NEST_KEY

The nested logit groups the alternatives in **nests** by the attribute `nest`, which is the alternative's mode. With
the nest's **logsum parameter** $\mu$ (`mu`, above 0 and at most 1), also called its dissimilarity parameter, the
**inclusive value** of nest $n$ is

$$
I_n = \ln \sum_{j \in n} \exp(V_j / \mu),
$$

a nest is taken with probability $\exp(\mu I_n) / \sum_m \exp(\mu I_m)$, and an alternative within it with
$\exp(V_j/\mu) / \sum_{k \in n} \exp(V_k/\mu)$. With $\mu = 1$ it is the plain logit. A smaller $\mu$ makes the
alternatives of one mode closer substitutes for each other than for other modes, so a mode with many routes does not
gain share from their number alone. The default is $\mu = 0.5$, an assumption. Within a nest the utilities are
divided by $\mu$, so the choice among a mode's own routes is sharper by $1/\mu$: at $\mu = 0.5$ a car trip in a run with
mode choice, a stated one too, chooses its route as if `beta_time_min` were −0.4, more concentrated on the best route
than in a run of one mode under `logit`. Compare route spreads between the two kinds of run with that in mind.

This is the nested logit of Williams and of Ben-Akiva and Lerman {footcite}`williams1977,benakiva1985`, written as
Train writes it with $\lambda$ for $\mu$ {footcite}`train2009`; texts that fix the scale of the upper level write the
nest's own scale, $1/\mu$, instead.

The draw is exact, in two steps: the nest by the Gumbel-max construction over $\mu I_n$, each nest with a random error of
its own; then the alternative by Gumbel-max over $V_j/\mu$.

### The logsum

% src: core-choice/src/model.rs ChoiceModel::logsums; builtin.rs Logit::logsums, NestedLogit::logsums

Both built-in random-utility models also give each situation's **logsum**: the expected utility of its best
alternative, up to a constant. For the logit it is $\ln \sum_j \exp V_j$; for the nested logit
$\ln \sum_n \exp(\mu I_n)$. It is in the units of the utility; divided by $-\beta_\text{time}$ it reads in minutes. It is
the standard measure of utility-based accessibility and user benefit {footcite}`williams1977,benakiva1985`. {py:meth}`openmobisim.Run.itinerary_choices` gives it per
itinerary trip, and {py:meth}`openmobisim.Run.logsum_od` by origin and destination.

## What a model sees

% src: core-sim/src/itinerary_choice.rs ATTRIBUTES, attribute(); route_choice.rs Chooser::fill; python/openmobisim/choice.py ROUTE_ATTRIBUTES

Every alternative, route or itinerary, carries the same 40 attributes ({py:data}`openmobisim.choice.ROUTE_ATTRIBUTES`).
An attribute an alternative has none of is 0: a car route has no walking, so `walk_min` is 0 and a coefficient on it
changes nothing for car routes.

| Attribute | Meaning |
|---|---|
| `time_min` | expected time door to door, in minutes |
| `length_km` | a route's length; an itinerary's car or bike leg's |
| `detour` | time over the best alternative's, minus one (0 for the best) |
| `overlap` | a route's largest share of its cost shared with a route found before it; 0 for an itinerary |
| `ln_path_size` | the log of the path size: 0 for an alternative that shares nothing, negative as it shares more |
| `n_links` | a route's links; an itinerary's car or bike leg's |
| `car_min`, `bike_min`, `walk_min`, `wait_min`, `ride_min` | the parts of the time: driving, cycling, walking, waiting at stops (boarding slack included), riding transit |
| `transfers` | changes of vehicle |
| `parking_min` | the time to park or to fetch a car or bike |
| `bike_separated_km`, `bike_lane_km`, `bike_mixed_km` | a bike leg's kilometres on separated tracks, painted lanes and in mixed traffic (a ferry crossing counts in none) |
| `walk_access_min`, `walk_egress_min`, `walk_transfer_min` | the walk to the first stop, from the last stop, and between stops (parts of `walk_min`) |
| `wait_first_min`, `wait_transfer_min` | the wait for the first vehicle and for the others (parts of `wait_min`) |
| `ride_rail_min`, `ride_metro_min`, `ride_tram_min`, `ride_bus_min`, `ride_ferry_min`, `ride_other_min` | minutes on board by kind of service (parts of `ride_min`) |
| `cost_eur` | what the alternative costs, in euros: the sum of the four parts below |
| `cost_running_eur`, `cost_toll_eur`, `cost_parking_eur`, `cost_fare_eur` | the car or bike leg's kilometres at the vehicle's price, the car leg's tolls, the parking fee, the fare |
| `trip_departure_h` | the trip's departure, in hours after midnight |
| `nest` | the alternative's mode as a number: the nest of the nested logit |
| `mode_walk`, `mode_bike`, `mode_car`, `mode_transit`, `mode_car_transit`, `mode_bike_transit` | 1 for the alternative's mode, 0 for the others: a coefficient on one is that mode's **constant** |

A part (such as `walk_access_min`) is weighed on top of its total (`walk_min`), or instead of it if you set the total's
coefficient to 0.

Three kinds of value of your own become attributes too:

- **Link values** (`Scenario(link_values=...)`): numbers per link of the road, bike or walk layer, such as greenery
  along a cycle route. Each becomes `<layer>_<name>_km` (the value times each link's length in kilometres, summed over
  the alternative's leg on that layer) and `<layer>_<name>_sum` (the plain sum).
- **Traveller values** (`person_values=`): `person_<name>`, such as income or age.
- **Trip values** (`trip_values=`): `trip_<name>`, such as a trip's purpose. The trip value `parking_eur` is added to
  `cost_parking_eur` of every alternative that arrives by car.

Traveller and trip values are the same for every alternative of a situation, so the built-in models weigh them only in
a product term.

**For a route**, `time_min` is its free-flow time (turns and signal delay included) at the first choice, and later its
time on the link times of the last loading, walked from the trip's departure. **For an itinerary**, every time is
expected: the last loading's link times, the timetable as the buses last ran, and how full each parking was. See
[Modes and itineraries](modes.md).

### The path size

% src: core-routes/src/attributes.rs RouteSets::attributes; core-sim/src/itinerary_choice.rs finalise

A plain logit treats two routes that share most of their links as two independent alternatives, which gives the pair
of them too much share. The **path size** {footcite}`benakiva1999` measures how much of a route is its own:

$$
PS_i = \sum_{a \in i} \frac{L_a}{L_i} \, \frac{1}{N_a}
$$

where $L_a$ is link $a$'s length, $L_i$ route $i$'s length and $N_a$ the number of routes of the pair's set that use
link $a$. A route that shares no link has $PS = 1$; two identical routes have $\frac{1}{2}$ each. The model adds
`beta_ln_path_size` $\cdot \ln PS_i$ to the utility. For a route, $N_a$ counts the routes **offered** to the traveller
(below), not the whole stored set.

For an itinerary the path size is measured by **time** instead of length. The elements alternatives can share are the
car or bike leg (the same links on the same mode) and each ride (the same line between the same two stops); walking,
waiting and parking are each alternative's own:

$$
PS_i = \frac{1}{T_i} \left( \sum_{e \in i} \frac{t_e}{N_e} + \Big(T_i - \sum_{e \in i} t_e\Big) \right)
$$

where $T_i$ is the itinerary's time door to door, $t_e$ the time of element $e$ and $N_e$ the number of the trip's
alternatives that hold it. A car route's path size (in route choice and as a car alternative in mode choice) is by
length over the **routes offered** to the trip, after the detour limit and `choice_max_routes`, not over the whole
stored set: two routes offered that share nothing both read 0, whatever else the set holds. The stored set's
path size, which the route panel and the reports show, is over the whole set.

## The choice set

% src: core-sim/src/route_choice.rs Chooser::fill; core-sim/src/run.rs DEFAULT_CHOICE_DETOUR_LIMIT, DEFAULT_CHOICE_MAX_ROUTES;
% core-sim/src/itinerary_choice.rs finalise

The **choice set** is the alternatives a traveller is offered at one choice. It depends on the time: a route that
congestion has made much slower than the best is not offered, and comes back when it is fast again.

**For a route**, two limits apply, both set on the `Scenario`:

| Option | Default | What it does |
|---|---|---|
| `choice_detour_limit` | 0.5 | a route is offered only if its expected time is at most `1 + limit` times the best route's in the pair's set |
| `choice_max_routes` | 5 | at most this many routes are offered: the fastest at the trip's departure, among those within the detour limit |

Setting either to 0 turns it off. The best route is always offered. A route not offered stays in the pair's set (which
may hold more routes: see [Route sets](route_sets.md)) and can be offered later. The limit of five routes is an
assumption on how many routes people consider; Prato reviews choice sets of a handful of routes {footcite}`prato2009`.

**For an itinerary**, the alternatives are first made unique (two itineraries with the same parking, car or bike leg and
rides are one), then:

- an alternative is offered only if its time is within `choice_detour_limit` of the best alternative **of its own
  mode** (a walk is always slower than a drive; whether it is a real alternative is for the model to weigh, not a time
  limit);
- the alternatives of at most `parking_options["candidates"]` parkings are kept per mode (see [Parking](parking.md));
- an alternative with transit that takes longer than `transit_options["journey_max_s"]` is dropped (see
  [Transit](transit.md)).

`choice_max_routes` does not apply to itineraries. In mode choice the car alternatives are the pair's routes under the
same two limits as in route choice: the fastest `choice_max_routes` of them, within the detour limit of the best car
route.

## The random numbers

% src: core-choice/src/model.rs gumbel (DrawAddress::from_quad); core-sim/src/route_choice.rs route_identity;
% core-sim/src/itinerary_choice.rs Alternative::identity_hash

Every random error is drawn from the run's **choice stream**, which `master_seed` starts, at an address made of the
traveller, the trip, the iteration and the alternative's **identity**. The identity does not depend on which other
alternatives are offered:

- a route's identity is a hash of its links;
- an itinerary's is a hash of its kind (transit, out through a parking, back to a parking, or a direct leg and its
  mode), its parking, its car or bike leg's links and its rides as (line, boarding stop, alighting stop), but **not**
  the vehicle runs, so a bus a minute late keeps its alternative and its draws.

So the results do not depend on the number of threads or on the order in which trips are processed. And adding or
removing an alternative leaves the random errors of every other alternative as they were: two scenarios differ only
where they really differ (*common random numbers* {footcite}`glasserman1992`). The same `master_seed` reproduces a run
exactly.

## Traveller classes

% src: core-choice/src/builtin.rs Logit::with_classes; core-choice/src/model.rs model_with_classes;
% python/openmobisim/scenario.py Scenario.__init__ (warning on choice_options replaced by a class)

A traveller class may give its own coefficients (any `beta_*`), such as its own mode constants or its own value of time.
A class's coefficient replaces the run's of the same name for that class's travellers; the rest are the run's. If you
set a coefficient in `choice_options` that a class also sets, the run warns that the class's value wins for that class.
Only `logit` and `nested_logit` weigh classes. See [Demand](demand.md) and [Modes and itineraries](modes.md).

## Writing your own model

% src: python/openmobisim/choice.py (module docs, sample_random_utility, segment_argmax, segment_softmax);
% crates/py-bindings/src/choice.rs PythonChoice

A model of your own is any Python object with a `choose(batch)` method. The run calls it once per batch of up to 32 768
situations. The batch holds NumPy arrays:

| Field | One value per | What it is |
|---|---|---|
| `offsets` | situation, plus one | situation `s` owns alternatives `offsets[s]:offsets[s+1]` |
| `attributes` | alternative | a dict of the attributes above, by name |
| `gumbel` | alternative | each alternative's standard Gumbel error, keyed as the built-in models' are |
| `identity` | alternative | its identity |
| `situation_of` | alternative | its situation |
| `traveller`, `trip` | situation | whose choice it is |
| `user_class` | situation | the traveller's class, as an index into `batch.class_names` |
| `iteration` | batch | the iteration |

`choose` returns, for each situation, the index of the alternative taken **within** the situation, or a pair
`(indices, probabilities)`. {py:func}`openmobisim.choice.sample_random_utility` does the bookkeeping for a random-utility
model, adding `batch.gumbel` to your utilities:

% Run on the code as built: prints "short_and_simple" and 60 of 60 trips completed.

```python
import openmobisim as ms
from openmobisim import choice


class ShortAndSimple:
    name = "short_and_simple"           # shown in the manifest
    descriptor = "short_and_simple;v1"  # identifies your settings; part of the fingerprint
    attributes = ["time_min", "length_km"]  # what you read (optional)

    def choose(self, batch):
        utility = -0.3 * batch.attributes["time_min"] - 0.1 * batch.attributes["length_km"]
        return choice.sample_random_utility(batch, utility)


net = ms.examples.manhattan_grid(6, 200.0, True)
trips = ms.examples.fixed_car_trips(
    net, [(f"p{i}", (0, i % 6), (5, (i + 3) % 6), 8.0, None) for i in range(60)]
)
run = ms.Scenario.from_parts(net, trips, choice_model=ShortAndSimple()).run("own", quiet=True)
```

Optional parts of a model:

- `probabilities(batch)`: the probability of **every** alternative, summing to one within each situation. Without it,
  an iterated run cannot measure how far it is from equilibrium, and those measures are `nan`.
- `logsum(batch)`: one number per situation, the expected utility of its best alternative; without it the logsum is
  `nan`.
- `name` and `descriptor`: shown in the manifest and part of the run's fingerprint. **Change the descriptor whenever the
  model's behaviour changes**, or two different models look like one run.
- `sampled`: `False` if the model never uses `batch.gumbel` (default `True`).
- `attributes`: the attributes it reads; the run then computes only those (default: all).

In Rust a model is a type that implements the `ChoiceModel` trait of the `openmobisim-core-choice` crate, added to a
`Registry` to be selected by name.

## Where the choices are

% src: python/openmobisim/scenario.py Run.route_choices, Run.itinerary_choices, Run.trip_modes, Run.choice_model

| Accessor | What it gives |
|---|---|
| {py:meth}`openmobisim.Run.route_choices` | per car trip: the route taken, its rank in the pair's set, how many alternatives, the probability, the weight |
| {py:meth}`openmobisim.Run.itinerary_choices` | per itinerary trip and per trip choosing its mode: the mode, the parking, the direction, the number of alternatives, the probability, the expected time, the rides and the logsum |
| {py:meth}`openmobisim.Run.trip_modes` | per trip: its departure and the mode it took |
| {py:attr}`openmobisim.Run.choice_model` | the model's descriptor |

## For bikes

% src: core-choice/src/builtin.rs Logit::default (bike_mixed_km); core-sim/src/itinerary_choice.rs measure_bike_legs, direct;
% core-sim/src/layers.rs StaticRoutes

- A bike alternative carries its kilometres by facility: `bike_separated_km`, `bike_lane_km` and `bike_mixed_km`. By
  default only mixed traffic is weighed (−0.16 per kilometre, on top of the time). At the default time weight and a bike
  speed of 15 km/h in mixed traffic this is the same 1.2 weight on a minute in mixed traffic that bike routes are chosen
  by (see [Networks](networks.md)), so a ride's route and its mode are judged alike. `bike_separated_km` and
  `bike_lane_km` weigh 0 until you set them.
- Link values on the bike layer (greenery, the car traffic beside a bike link) become `bike_<name>_km` and
  `bike_<name>_sum`, and any model can weigh them.
- `beta_mode_bike` is the bike's mode constant, 0 by default. A traveller class can give its own.
- **A cyclist has one route, not a choice set.** A bike trip with a stated mode takes the shortest route on the bike
  layer under the bike cost, without the choice model; in mode choice, the bike alternative is that one route. Route
  choice among several bike routes is not built.
- Bike-and-ride alternatives are itineraries like park-and-ride's; see [Parking](parking.md).

## Limits

- The defaults are uncalibrated. openmobisim does not estimate coefficients; estimate them elsewhere and pass them.
- The built-in models are the logit and the nested logit with one level of nests (the mode). Other forms (a mixed logit,
  a model that is not random-utility) can be written as your own model.
- `deterministic` reads `time_min` only.
- A Python model is called once per batch from the run, which is slower than a built-in model.
- `choice_max_routes` limits car routes, not itineraries.
- Bikes have one route per trip (above).

## References

```{footbibliography}
```
