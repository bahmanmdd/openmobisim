# Validation

% src: README.md ("The ten tests"); the tests named under each section, read for their cases and tolerances; crates/*/README.md ("Tests")

How do we know the simulator computes what it says? Release 0.2 is built against **ten tests**, each of which compares
the simulator with **an answer from outside the simulator**: a closed form of theory, a derivation by hand, a published
result, or another tool. The simulator's own output, old or new, is never the expected value of a test, so a test
cannot pass merely because the program agrees with itself.

Two comparisons of the simulator with itself are made, each for a property that has no outside answer:

- **determinism**: identical results at 1 and 8 threads and over two runs ([Reproducibility](reproducibility.md));
- **the link time reading** on a route a traveller did not take, against the loading run again with that one car moved
  ([The loading](loading.md#the-link-time-reading)).

This page has one section per test: what is tested, what the outside answer is and where it comes from, and the
tolerance, as it is written in the test itself. A tolerance stated here was read in the test's code.

## The ten tests at a glance

% src: README.md ("The ten tests")

| # | Test | Part of the model | The outside answer |
|---|---|---|---|
| 1 | A bottleneck | [The loading](loading.md) | Vickrey's deterministic queue, by hand |
| 2 | A merge | The loading | Daganzo's merge with demand-proportional priorities, as a queue by hand |
| 3 | Two routes under steady demand | [The iteration](iteration.md) with the loading | Equal times and the split by capacity (Wardrop), by a closed form |
| 4 | The Braess network with capacities | The iteration | The capacity-constrained equilibrium, by hand |
| 5 | Nguyen–Dupuis | The iteration | The capacity-constrained equilibrium, by a linear program in an outside solver |
| 6 | Sioux Falls | The iteration | The same, from the published network files |
| 7 | Choice closed forms | [Choice](choice.md) | Logit, nested logit and path-size logit shares, by hand |
| 8 | Transit | [Transit](transit.md) | Waiting times at closed form; every journey enumerated by brute force |
| 9 | Bike and walk | The loading, [the route search](search.md) | Length over speed by hand; shortest paths by networkx |
| 10 | Delft | The whole run through Python | Shortest times by networkx; determinism; a longer run |

The tests are part of the source tree, under `crates/<module>/tests/`. With a Rust toolchain, each module's tests run
with one command from the top of the tree:

```bash
cargo test -p openmobisim-loading
cargo test -p openmobisim-choice
cargo test -p openmobisim-transit
cargo test -p openmobisim-search
cargo test -p openmobisim-loop
```

| Command | Tests of the ten it holds | Time of the command on a laptop, without optimisation, its own tests compiled in that time (s) |
|---|---|---|
| `cargo test -p openmobisim-loading` | 1, 2, and the trip times of 9 | 6 |
| `cargo test -p openmobisim-choice` | 7 | 4 |
| `cargo test -p openmobisim-transit` | 8 | 9 |
| `cargo test -p openmobisim-search` | the routes of 9 | 2 |
| `cargo test -p openmobisim-loop` | 3, 4, 5, 6, and the path sizes of 7 | 43 |

The longer cases (test 4 with the added link, test 5, the
larger cases of tests 4 and 6) are skipped by these commands and run with optimisation; each contract gives its
command ([The modules](../reference/crates.md)). Test 10 runs through Python, from
`crates/binding/tests/test_10_delft.py`.

## Test 1: a bottleneck

% src: crates/loading/tests/point_queue.rs (`a_bottleneck_of_1200_veh_h_fed_a_car_every_2_s_delays_car_k_by_k_seconds`, `the_bottleneck_queue_clears_at_lambda_t_over_mu_900_s`, `with_unlimited_capacity_every_exit_is_exactly_entry_plus_free_flow`)

**What is tested.** The point queue on one link whose inflow exceeds its capacity for a period, and the same link with
unlimited capacity. The link is 1 000 m at 50 km/h (72 s at free flow) with a capacity of 1 200 vehicles per hour; a
car enters every 2 s for 600 s, 300 cars in all.

**The outside answer.** Vickrey's (1969) deterministic bottleneck, by hand: with arrivals at λ = 0.5 vehicles per
second and a capacity of μ = 1/3 vehicle per second, car $k$ leaves at $72 + 3k$ s and is delayed by $k$ seconds; the
total delay is 44 850 vehicle-seconds; the queue is gone at $\lambda T / \mu = 900$ s. With unlimited capacity every
car leaves exactly one free-flow time after it enters.

| Quantity | Tolerance |
|---|---|
| Each car's exit and delay | The specification allows 1 s; the test asserts 10⁻⁹ s |
| The total delay | 10⁻⁶ vehicle-seconds |
| With unlimited capacity, each exit and each travel time | Exact, to the last bit |

## Test 2: a merge

% src: crates/loading/tests/point_queue.rs (`a_merge_of_1800_and_600_veh_h_into_1800_veh_h_queues_both_approaches_alike`)

**What is tested.** Two links feeding one of lower capacity, with unequal demands: approach A sends 1 800 vehicles per
hour and approach B 600, both for 900 s, into a link of 1 800 vehicles per hour.

**The outside answer.** Daganzo's (1995) merge rule with priorities in proportion to the demands is the deterministic
queue on the merged link: it discharges at its capacity, and cars are delayed by their order of arrival. Car by car by
Vickrey's recursion, by hand, the total delay is 90 300 vehicle-seconds (67 650 on approach A's cars and 22 650 on
B's); the fluid formula $\lambda(\lambda-\mu)T^2/(2\mu)$ gives 90 000 vehicle-seconds and a mean delay of 150 s on each
approach. The unequal demands are what tell this rule from priorities in proportion to capacity, under which B would
pass free.

| Quantity | Tolerance |
|---|---|
| Each car's arrival against the recursion by hand | 10⁻⁹ s |
| The total delay, and each approach's, against the hand values | 10⁻³ vehicle-seconds |
| The total delay against the fluid 90 000 vehicle-seconds | 0.5 % |
| Each approach's mean delay against the fluid 150 s | 1 s |

## Test 3: two routes under steady demand

% src: crates/loop/tests/two_routes.rs (the header; `equal_times_on_both_routes_and_the_split_by_capacity_at_1800_veh_h`; `equilibrium_at`); crates/loop/tests/common/mod.rs (`time_only`, `PART_1_S`)

**What is tested.** The iteration with the loading, on the smallest case with a route choice: a short route of 288 s
with a bottleneck of 1 200 vehicles per hour on its first link, and a long route of 460.8 s with ample capacity, under
a constant inflow of 1 800 vehicles per hour for one hour. Travellers choose the least time.

**The outside answer.** The dynamic user equilibrium of Vickrey's (1969) bottleneck with a second route, in closed
form: everyone takes the short route until its queue costs the difference of the two free-flow times, at
$t^* = (t_2 - t_1)\,c_1/(q - c_1) = 345.6$ s; from then on both routes are used at equal times (Wardrop 1952), the short
one at its capacity, so the trips leaving after $t^*$ split by capacity, $c_1/q = 2/3$ on the short route. The mean trip
over the hour is 452.5 s.

| Quantity | Judged on | Tolerance |
|---|---|---|
| Every trip finishes; the long route runs at free flow | every seed | 10⁻⁶ s on the long route's time |
| Each car's exit against the point queue replayed on the recorded entries | every seed | 10⁻⁶ s |
| The short route's share after $t^*$ against 2/3 | the mean over seeds 0 to 4 | 0.03 |
| $t^*$ against 345.6 s | the mean over seeds 0 to 4 | 60 s |
| The mean trip against 452.5 s | the mean over seeds 0 to 4 | 3 % |
| Equal times on the two routes after $t^*$ | the mean over seeds 0 to 4 | 3 % of the long route's time |

The quantities that depend on who was asked to choose again are judged on the mean over five seeds, because one seed
scatters about as much as the tolerances.

## Test 4: the Braess network with capacities

% src: crates/loop/tests/braess.rs (the header; `judge_nineteen_minutes`; the four tests)

**What is tested.** The iteration on the network of Braess's paradox (Braess 1968), where adding a link makes every
trip longer. Four nodes s, a, b, t; the links s→a and b→t take 1 min and pass 1 000 vehicles per hour; a→t and s→b take
10 min; the added link a→b takes 1 min. The demand is 1 500 vehicles per hour for three hours, and the departures of
the second hour are read as the steady state.

**The outside answer.** The steady state of the point queue is the static equilibrium with capacity constraints
(Cominetti, Correa and Olver 2022), a linear program solved by hand: with the added link, a third of the travellers on
each of the three routes, a queue of 8 min at s→a and at b→t, and every trip 19 min (1 140 s); without the added link,
half on each route, no queue, 11 min; at a demand of 900 vehicles per hour with the added link, everyone on the route
through it, 3 min.

| Case | Quantity | Tolerance |
|---|---|---|
| With the link, 4 500 travellers, at the stopping rule, the mean over seeds 0 to 4 | The mean trip against 1 140 s | 2 % |
| | Each route's share against 1/3 | 0.03 |
| | Each queue's delay against 480 s | 8 % |
| With the link, 72 000 travellers, 1 000 iterations, seed 0 | The mean trip | 0.5 % |
| | Each route's share | 0.005 |
| | Each queue's delay | 1 % |
| Without the link, the mean over seeds 0 to 4 | The mean trip against 660 s | 2 % |
| | The crossing time of s→a against 60 s | 5 % |
| At 900 vehicles per hour, the mean over seeds 0 to 4 | The mean trip against 180 s | 2 % |
| | The share on the route through the added link | At least 0.97 |

The two tiers show that the distance to theory vanishes with the number of travellers, not with the number of
iterations: a finite population leaves a residual, which the larger case shrinks. The test's own notes record that the
larger case, judged on seed 0, depends on its seed: over seeds 0 to 5, two seeds exceed the 1 % limit on a queue's
delay.

## Test 5: Nguyen–Dupuis

% src: crates/loop/tests/nguyen_dupuis.rs (the header; the two tests); crates/loop/tests/equilibrium_lp.py (the header)

**What is tested.** The iteration on the network of Nguyen and Dupuis (1984): 13 nodes, 19 links and four
origin–destination pairs, each at a constant rate for three hours, the second hour read as the steady state.

**The outside answer.** The capacity-constrained equilibrium: the route flows that minimise the total free-flow time
with no link above its capacity (Larsson and Patriksson 1995). The queue delays are the dual prices of the capacity
constraints. It is solved as a linear program by an outside solver (HiGHS, through scipy), at 0.9 of the case's demand,
where the total time is unique: a mean trip of 36.10 min, a total free-flow time of 60 610 vehicle-minutes per hour of
demand, and origin–destination times of 32, 38, 37 and 34 min. Two links must queue and two may.

| Quantity | Judged on | Tolerance |
|---|---|---|
| The total free-flow time of the routes taken | the mean over seeds 0 to 4 | 3 % |
| The mean trip | the mean over seeds 0 to 4 | 3 % |
| Each origin–destination time | the mean over seeds 0 to 4 | 3 % outside the range the program allows |
| The entries of each link with a standing queue, against its capacity | the mean over seeds 0 to 19 | 3 % of the capacity |
| Which links queue: none that the program never queues | every seed | Exact |
| Each car's exit against the point queue replayed, and its entries along its route | every seed | 10⁻⁶ s |

No published equilibrium of this network is the equilibrium of a queue model, which is why the answer is computed by
the linear program.

## Test 6: Sioux Falls

% src: crates/loop/tests/sioux_falls.rs (the header; `steady_hour`; the two tests); crates/loop/tests/equilibrium_lp.py; README.md (row 6; "Known limits")

**What is tested.** The iteration on the Sioux Falls network of the Transportation Networks for Research collection
(TNTP): 24 nodes, 76 links and 528 origin–destination pairs, three hours of constant demand, the second hour read as
the steady state.

**The outside answer.** The same capacity-constrained equilibrium, solved by the same linear program from the TNTP
files, at 0.045 of the TNTP demand per hour with the capacities at a tenth (the program does not change when flows and
capacities are scaled together): a mean trip of 11.18 min, a total free-flow time of 151 080 vehicle-minutes per hour,
and 20 links that must queue, each at its capacity.

| Case | Quantity | Tolerance |
|---|---|---|
| 48 814 trips, at the stopping rule | The total free-flow time of the routes taken | 3 % |
| | The mean trip | 3 % |
| | The entries of each link with a standing queue, against its capacity | 3 % of the capacity |
| 97 362 trips, 400 iterations | The total free-flow time of the routes taken | 0.1 % |
| | The mean trip | 1 % |
| | The entries of each link with a standing queue | 2 % of the capacity |

Two things are reported and not judged. A single link's queue delay: the sum over the links agrees with the solver, but
single links differ. And the published user-equilibrium flows of this network: they are the equilibrium of
volume–delay functions, a model without capacity or queue, in which the flow exceeds the capacity on 60 of the 76
links, so they are not the answer of a queue model. The mean trip is judged at 1 % and not tighter in the larger case
because the queues of a point queue under constant demand drift along the window, a residual that vanishes neither with
iterations nor with the population.

## Test 7: choice closed forms

% src: crates/choice/tests/logit.rs; crates/choice/tests/nested.rs; crates/choice/tests/path_size_in_a_nest.rs; crates/choice/tests/common/mod.rs (`CHI_SQUARE_999_DF5`); crates/loop/tests/path_size_in_a_run.rs

**What is tested.** The choice models alone, and the path sizes inside a run.

**The outside answer.** The formulas of the logit, of the nested logit (Ben-Akiva and Lerman 1985, chapter 10) and of
the path-size logit (Ben-Akiva and Bierlaire 1999), written out by hand in the tests.

| Case | Outside answer | Tolerance |
|---|---|---|
| Six alternatives with costs $\ln(21/k)$ | Probabilities $k/21$ | 10⁻¹² on each probability |
| The same, 200 000 draws | The same shares | The chi-square statistic below 20.52, the 0.999 quantile at 5 degrees of freedom |
| Three nests of three, one and two alternatives, at nest scales 0.2, 0.5, 0.9 and 1 | The nested logit's closed form | 10⁻¹² on each probability; chi-square below 20.52 on 20 000 draws each |
| Three car routes of equal time, two of them sharing a fraction $\varphi$ of their length | The route that shares nothing takes $1/(3-\varphi)$: 0.4545 at $\varphi = 0.8$, 0.4 at $\varphi = 0.5$ | 10⁻¹² |
| Three car routes with path sizes 0.5, 0.6 and 2/3 in one nest, a bike in another, at nest scales 0.5 and 0.25 | The closed form with the path size at the exponent 1 | 10⁻¹² |
| A route split into two copies of half its path size | Every other probability unchanged; each copy takes half | 10⁻¹² |
| In a run: three car routes of 800, 1 000 and 1 200 m sharing their first 600 m | The path sizes 0.5, 0.6 and 2/3 by hand, for every car route of every traveller's set at the last iteration | 10⁻¹² on the logarithm of each path size |

## Test 8: transit

% src: crates/transit/tests/common_lines.rs; crates/transit/tests/brute_force.rs; crates/transit/tests/spiess_florian.rs; crates/transit/README.md ("Tests")

**What is tested.** The journey search, in two ways: the waiting times and line shares it produces when travellers
arrive at random, and its journeys one by one.

**The outside answers.**

- *Common lines.* Two lines from A to B, every 600 s and every 870 s, with travellers arriving every 10 s. Under
  uniform and independent phases the mean wait for the first vehicle is $H_1/2 - H_1^2/(6H_2) = 231.0$ s and the first
  line's share is $1 - H_1/(2H_2) = 0.655$. A fast and a slow line are compared with the same expectation integrated
  numerically in the test. One line at regular and at irregular headways is compared with the mean wait of Osuna and
  Newell (1972), $E[H^2]/(2E[H])$.
- *Brute force.* On 300 random timetables, with five questions each, a program in the test enumerates every journey by
  the definition and keeps the earliest arrival, then the fewest rides, then the lowest last run. The search must give
  the same, for the earliest journey and for the journey at each number of rides.
- *The network of Spiess and Florian (1989)*, written as timetables: four stops and four lines, a traveller every 10 s
  for two hours. Every traveller's journey must equal the brute force's.

| Quantity | Tolerance |
|---|---|
| The mean wait for the first of two lines against 231.0 s | 10 s |
| The first line's share against 0.655 | 0.02 |
| A fast and a slow line: the mean time to the door, and the fast line's share | 10 s; 0.02 |
| One line, regular and irregular headways: the mean wait | 10 s |
| The search against the brute force, on the random timetables and on the network of Spiess and Florian | Exact: the same arrival, rides and last run |

The mean time on the network of Spiess and Florian is reported beside the paper's 27.75 min and is not judged: the
paper's travellers follow a strategy knowing only the frequencies, while these know the timetable. The tolerance of
10 s on the waits is the timetable's, not the code's: the lines are discrete and the period finite, and an independent
computation on the same timetables gives 227.23 s and 0.6650.

## Test 9: bike and walk

% src: crates/loading/tests/bikes.rs; crates/loading/tests/common/mod.rs (`close`); crates/search/tests/networkx_routes.rs

**What is tested.** That a bike and a walker take exactly the length of their route over their speed, whatever the
cars do, and that their routes are the shortest.

**The outside answers.** By hand: a bike rides 1 500 m at 15 km/h in 360 s; a walker covers 1 000 m at 4.8 km/h in
750 s; a route of three links takes the sum of its links (720 s by bike and 2 250 s on foot in the test's case). By
another tool: the shortest paths by length of networkx (Hagberg, Schult and Swart 2008; version 3.7) on a grid of
12 × 12 nodes moved off their places, for 20 pairs of nodes that each have exactly one shortest path.

| Quantity | Tolerance |
|---|---|
| Each trip time against length over speed | A relative 10⁻¹² |
| A bike's and a walker's times and counts with 1 000 cars queued on their link | Identical to those without the cars |
| The nodes of each bike route against networkx's path; the walker's and the free-flow car's links against the bike's | Exact |
| Each arrival against networkx's length over the mode's speed | A relative 10⁻⁹ |

## Test 10: Delft

% src: crates/binding/tests/test_10_delft.py (the header and the five tests); README.md (row 10)

**What is tested.** A whole run of a real city through the Python interface, with every parameter at its default:
Delft's network as tables (21 566 nodes; 12 460 car links, 20 652 bike links and 55 400 walk links; no signals), 65 959
trips leaving between 07:30 and 08:30 in four classes, the city's transit feed, and a window from 07:30 to 09:30.

| Part | Outside answer or property | Tolerance |
|---|---|---|
| 1. Free-flow routes of 1 000 pairs of nodes per mode (car, bike and walk), with car capacities unlimited | The shortest times of networkx on the same tables | A relative 10⁻⁶ on each trip's time |
| 2. Every trip has an alternative and finishes by 09:30 | A property | None: every trip |
| 3. The run alone takes under 15 s | A budget | Judged on the developer's machine only; in the automated checks the time is printed and not judged |
| 4. The trips and links files over two runs and at 1 and 8 threads | Determinism, the self-comparison | Identical bytes |
| 5. The state at which the stopping rule stops the run | A run of 60 iterations | 1 %, relative, on each mode's mean trip time and on each mode's share |

Part 5 does not compare with an outside answer: it checks that stopping early loses little against running on.

## Beyond the ten

% src: crates/*/README.md ("Tests")

Each module's contract lists further tests, each again against a value from outside the module: the readers against
hand-made files and against other tools (gtfs-kit for the Delft feed, pyarrow for the tables written), the fingerprint
against the published test vectors of its hash, the random draws against their distributions, the run's record against
Python's standard TOML reader. They are listed in [The modules](../reference/crates.md) and in each contract.

## For bikes

% src: README.md ("Bikes are first-class in every module, test and output"); the tests above

Bikes are in the tests as a mode beside the others: test 9 is theirs and the walkers'; test 10 judges their routes
against networkx, counts their trips in the finishing and determinism parts, and judges their mean trip time and share
in part 5; test 7's formulas do not depend on the mode, and a bike stands in its own nest beside the car routes in its
nested cases. Tests 1 to 6 are about queues, which a bike does not have: test 9 checks that 1 000 queued cars leave a
bike's time unchanged.

## Limits

% src: README.md ("Known limits"); the headers of crates/loop/tests/braess.rs, sioux_falls.rs, nguyen_dupuis.rs; crates/map/README.md and crates/summary/README.md ("Tests": the page's behaviour in a browser)

- **The tests are of the model's arithmetic and logic, not of its realism.** They show that the queue, the search, the
  choice and the iteration compute what their definitions say. No test compares a run with counts or travel times
  observed on a real network: the Delft test compares the simulator with networkx, with itself and with a longer run.
- **The equilibrium tests have tolerances of a few percent**, because a finite number of travellers leaves a residual;
  the larger cases show it shrinking.
- **Some quantities are reported and never judged**: a single link's queue delay on Sioux Falls, the mean time on the
  network of Spiess and Florian, each traveller's distance from the best of its own set in tests 3 to 6.
- **Some cases depend on their seed** (test 4's larger case, above).
- **The run-time budget of test 10 is judged on one machine.**
- **The behaviour of the map and the summary in a browser** (clicks, panels, exports) is not tested automatically; the
  numbers they hold are.
- **Signals are ignored in every test**, as everywhere in release 0.2.

All limits of the model are gathered in [Limitations](limitations.md).

## References

- Ben-Akiva, M. and Bierlaire, M. (1999). Discrete choice methods and their applications to short term travel
  decisions. In R. W. Hall (ed.), *Handbook of Transportation Science*, Kluwer, 5–33.
- Ben-Akiva, M. and Lerman, S. R. (1985). *Discrete Choice Analysis: Theory and Application to Travel Demand*. MIT
  Press, Cambridge.
- Braess, D. (1968). Über ein Paradoxon aus der Verkehrsplanung. *Unternehmensforschung* 12, 258–268.
- Cominetti, R., Correa, J. and Olver, N. (2022). Long-term behavior of dynamic equilibria in fluid queuing networks.
  *Operations Research* 70(1), 516–526.
- Daganzo, C. F. (1995). The cell transmission model, part II: network traffic. *Transportation Research Part B*
  29(2), 79–93.
- Hagberg, A. A., Schult, D. A. and Swart, P. J. (2008). Exploring network structure, dynamics, and function using
  NetworkX. In *Proceedings of the 7th Python in Science Conference (SciPy 2008)*, 11–15.
- Larsson, T. and Patriksson, M. (1995). An augmented Lagrangean dual algorithm for link capacity side constrained
  traffic assignment problems. *Transportation Research Part B* 29(6), 433–455.
- Nguyen, S. and Dupuis, C. (1984). An efficient method for computing traffic equilibria in networks with asymmetric
  transportation costs. *Transportation Science* 18(2), 185–202.
- Osuna, E. E. and Newell, G. F. (1972). Control strategies for an idealized public transportation system.
  *Transportation Science* 6(1), 52–72.
- Spiess, H. and Florian, M. (1989). Optimal strategies: a new assignment model for transit networks. *Transportation
  Research Part B* 23(2), 83–102.
- Transportation Networks for Research Core Team. *Transportation Networks for Research*.
  https://github.com/bstabler/TransportationNetworks (the Sioux Falls files).
- Vickrey, W. S. (1969). Congestion theory and transport investment. *The American Economic Review* 59(2), 251–260.
- Wardrop, J. G. (1952). Some theoretical aspects of road traffic research. *Proceedings of the Institution of Civil
  Engineers* 1(3), 325–362.

The full list is in the [bibliography](../reference/bibliography.md).
