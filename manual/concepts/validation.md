# Validation

% src: README.md ("The ten tests"); the tests under crates/*/tests/, read for their cases and tolerances; crates/*/README.md ("Tests")

How do we know the simulator computes what it says? Each part of the simulator is tested against **an
answer from outside the simulator**: a closed form of theory, a derivation by hand, a published result, or another
tool. The simulator's own output is never the expected value of a test, so a test cannot pass merely because the
program agrees with itself. Two comparisons of the simulator with itself are made, each for a property that has no
outside answer: determinism ([Reproducibility](reproducibility.md)), and the link time reading on a route a traveller
did not take, against the loading run again with that one car moved
([The loading](loading.md#the-link-time-reading)).

There are ten tests. The last column of the table gives how close the simulator comes to the outside answer: the
largest difference each test accepts. Where a quantity depends on chance (who is offered a new choice at an
iteration), it is judged on the mean of several runs.

| Test | What is tested | The outside answer, with its reference | How close the simulator comes |
|---|---|---|---|
| 1. A bottleneck | [The loading](loading.md): the point queue on one link of 1 000 m at 50 km/h with a capacity of 1 200 vehicles per hour, entered by a car every 2 s for 600 s; and the same link with unlimited capacity | The deterministic bottleneck of Vickrey (1969), by hand: car $k$ leaves at $72 + 3k$ s, the total delay is 44 850 vehicle-seconds and the queue is gone at 900 s. With unlimited capacity every car leaves one free-flow time after it enters | Each car's exit and delay within 10⁻⁹ s; the total delay within 10⁻⁶ vehicle-seconds; with unlimited capacity every exit is exact, to the last bit |
| 2. A merge | The loading: two links, sending 1 800 and 600 vehicles per hour for 900 s, feed one link of 1 800 vehicles per hour | The merge of Daganzo (1995) with priorities in proportion to the demands, which is the deterministic queue on the merged link. Car by car, by hand: a total delay of 90 300 vehicle-seconds. By the fluid formula: 90 000 vehicle-seconds and a mean delay of 150 s on each approach | Each car's arrival within 10⁻⁹ s; the total delay and each approach's within 10⁻³ vehicle-seconds of the hand values; the total delay within 0.5 % of the fluid value; each approach's mean delay within 1 s of 150 s |
| 3. Two routes under steady demand | [The iteration](iteration.md) with the loading: a short route of 288 s behind a bottleneck of 1 200 vehicles per hour and a long route of 460.8 s, under 1 800 vehicles per hour for one hour; travellers choose the least time | The dynamic user equilibrium of Vickrey's (1969) bottleneck with a second route, in closed form: from $t^* = 345.6$ s on, both routes are used at equal times (Wardrop 1952), two thirds of the trips on the short one; the mean trip is 452.5 s | The short route's share after $t^*$ within 0.03 of 2/3; $t^*$ within 60 s; the mean trip within 3 %; the times of the two routes after $t^*$ equal within 3 % of the long route's time; the long route's time within 10⁻⁶ s of free flow; each car's exit within 10⁻⁶ s of the point queue's rule applied to the recorded entries |
| 4. The Braess network with capacities | The iteration, on the network of the paradox of Braess (1968), at 1 500 vehicles per hour for three hours | The capacity-constrained equilibrium (Cominetti, Correa and Olver 2022), by hand: with the added link, a third of the travellers on each of three routes, two queues of 8 min, every trip 19 min; without it, every trip 11 min; at 900 vehicles per hour, every trip 3 min | With 4 500 travellers: the mean trip within 2 %, each route's share within 0.03, each queue's delay within 8 %. With 72 000 travellers and 1 000 iterations: within 0.5 %, 0.005 and 1 %. Without the added link: the mean trip within 2 % of 660 s, and the crossing time of the first link within 5 % of 60 s. At 900 vehicles per hour: the mean trip within 2 % of 180 s, and at least 0.97 of the travellers on the route through the added link |
| 5. Nguyen–Dupuis | The iteration, on the network of Nguyen and Dupuis (1984): 13 nodes, 19 links, four origin–destination pairs | The capacity-constrained equilibrium (Larsson and Patriksson 1995), a linear program solved by another tool (the solver HiGHS, through scipy): a mean trip of 36.10 min, a total free-flow time of 60 610 vehicle-minutes per hour of demand, and origin–destination times of 32, 38, 37 and 34 min | The total free-flow time of the routes taken within 3 %; the mean trip within 3 %; each origin–destination time within 3 % of the range the program allows; the entries of each link with a standing queue within 3 % of its capacity; no link queues that the program never queues; each car's exit within 10⁻⁶ s of the point queue's rule applied to the recorded entries |
| 6. Sioux Falls | The iteration, on the Sioux Falls network of the Transportation Networks for Research collection: 24 nodes, 76 links, 528 origin–destination pairs | The same linear program, solved from the published network files: a mean trip of 11.18 min, a total free-flow time of 151 080 vehicle-minutes per hour, and 20 links that queue, each at its capacity | With 48 814 trips: the total free-flow time within 3 %, the mean trip within 3 %, the entries of each queued link within 3 % of its capacity. With 97 362 trips and 400 iterations: within 0.1 %, 1 % and 2 % |
| 7. Choice closed forms | [Choice](choice.md): the logit, the nested logit at nest scales 0.2, 0.5, 0.9 and 1, and the path-size logit; and the path sizes of three overlapping car routes inside a run | The formulas of the logit, of the nested logit (Ben-Akiva and Lerman 1985, chapter 10) and of the path-size logit (Ben-Akiva and Bierlaire 1999), worked by hand | Each probability within 10⁻¹²; the shares of 200 000 and of 20 000 drawn choices by a chi-square test (the statistic below 20.52, the 0.999 quantile at 5 degrees of freedom); the logarithm of each path size of three overlapping routes within 10⁻¹²; in a run, each trip's cost with its path-size term within 10⁻⁹ |
| 8. Transit | [Transit](transit.md): the journey search, by the waiting times and line shares it gives travellers who arrive at random, and journey by journey | The closed forms for two lines serving the same pair of stops (a mean wait of 231.0 s and a share of 0.655 for the first line) and the mean wait of Osuna and Newell (1972) for one line; every journey enumerated by brute force on 300 random timetables and on the network of Spiess and Florian (1989) | Each mean wait, and the mean time to the door with a fast and a slow line, within 10 s; each line share within 0.02; every journey identical to the brute force's: the same arrival, rides and last run |
| 9. Bike and walk | The loading and [the route search](search.md): a bike and a walker take the length of their route over their speed, whatever the cars do, and their routes are the shortest | By hand: 1 500 m by bike at 15 km/h in 360 s, 1 000 m on foot at 4.8 km/h in 750 s. By another tool: the shortest paths of networkx (Hagberg, Schult and Swart 2008) on a grid of 12 × 12 nodes, for 20 pairs of nodes | Each trip time within a relative 10⁻¹²; times and counts identical with 1 000 cars queued on the same link; each route identical to the path of networkx; each arrival within a relative 10⁻⁹ |
| 10. Delft | A whole run of a real city through the Python interface, with every parameter at its default: 21 566 nodes, 88 512 links, 65 959 trips in four classes and the city's transit feed | The shortest times of networkx on the same tables, for 1 000 pairs of nodes per mode at free flow; the properties that every trip has an alternative and finishes, and that the results are identical over two runs and at 1 and 8 threads; a run of 60 iterations | Each free-flow trip time within a relative 10⁻⁶; every trip finishes; the trips and links files identical, byte for byte; the state at which the stopping rule stops within 1 % of the run of 60 iterations, on each mode's mean trip time and share |

## The tests in more detail

**Test 2: a merge.** The loading has no rule at a node: where two links feed a narrower one, the cars queue on the
narrower link's end in the order they reach it. That is what a merge with priorities in proportion to the demands
gives, and the unequal demands of the test are what tell this rule from priorities in proportion to capacity, under
which the smaller approach would pass free.

**Test 3: two routes.** In the closed form everyone takes the short route until its queue costs the difference of the
two free-flow times, at $t^* = (t_2 - t_1)\,c_1/(q - c_1) = 345.6$ s, with $t_1$ and $t_2$ the free-flow times of the
short and the long route, $c_1$ the capacity of the bottleneck and $q$ the inflow. From then on the short route runs at
its capacity, so the trips split by capacity, $c_1/q = 2/3$ on the short route.

**Tests 4 to 6: the capacity-constrained equilibrium.** Under a constant demand the point queue settles to a steady
state, and that state is a static equilibrium with capacity constraints (Cominetti, Correa and Olver 2022): the route
flows that minimise the total free-flow time with no link above its capacity (Larsson and Patriksson 1995). It is a
linear program, and the queue delay of a link is the price of its capacity constraint. On the Braess network the
program is solved by hand; on the two larger networks by an outside solver. Each case runs three hours of constant
demand, and the departures of the second hour are read as the steady state. A finite number of travellers leaves a
residual of a few percent, which the larger cases of tests 4 and 6 show shrinking. The published user-equilibrium
flows of Sioux Falls are not the outside answer: they are the equilibrium of volume–delay functions, a model without
capacity or queue, in which the flow exceeds the capacity on 60 of the 76 links. The linear program is solved at 0.9
of the demand of the Nguyen–Dupuis case, where its total time is unique, and at 0.045 of the published demand of Sioux
Falls per hour with the capacities at a tenth.

**Test 8: transit.** For two lines from A to B, every $H_1 = 600$ s and every $H_2 = 870$ s, with uniform and
independent phases, the mean wait for the first vehicle is $H_1/2 - H_1^2/(6H_2) = 231.0$ s and the first line's share
is $1 - H_1/(2H_2) = 0.655$. For one line the mean wait is $E[H^2]/(2E[H])$, with $H$ the interval between two runs
(Osuna and Newell 1972). A fast and a slow line are compared with the same expectation, integrated numerically. The 10 s allowed on a mean wait is the timetable's and not the search's: the lines are
discrete and the period finite, and an exact computation on the same timetables gives 227.23 s and 0.6650. The brute
force enumerates every journey by the definition and keeps the earliest arrival, then the fewest rides, then the lowest last run. On the network
of Spiess and Florian the mean time is not compared with the paper's 27.75 min: the paper's travellers follow a
strategy knowing only the frequencies, while these know the timetable.

**Test 10: Delft.** The network is that of the published case `delft` (12 460 car links, 20 652 bike links and
55 400 walk links); the trips are the test's own, leaving between 07:30 and 08:30 in a window from 07:30 to 09:30. The
comparison with a run of 60 iterations has no outside answer: it shows that stopping by the rule loses little against
running on.

## For bikes

% src: README.md ("Bikes are first-class in every module, test and output"); the tests above

Bikes are in the tests as a mode beside the others. Test 9 is theirs and the walkers', and checks that 1 000 queued
cars leave a bike's time unchanged. Test 10 compares their routes with networkx, counts their trips among those that
must finish and be identical, and compares their mean trip time and share with the longer run. In test 7 a bike stands
in its own nest beside the car routes. Tests 1 to 6 are about queues, which a bike does not have.

## Limits

% src: README.md ("Known limits"); the headers of crates/loop/tests/braess.rs, sioux_falls.rs, nguyen_dupuis.rs

- **The tests are of the model's arithmetic and logic, not of its realism.** They show that the queue, the search, the
  choice and the iteration compute what their definitions say. No test compares a run with counts or travel times
  observed on a real network.
- **The equilibrium tests agree within a few percent**, because a finite number of travellers leaves a residual.
- **A single link's queue delay on Sioux Falls is not compared**: the sum over the links agrees with the solver, single
  links differ.
- **Signals are ignored in every test**, as everywhere in the simulator.

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
  Engineers* 1(3), 325–378.

The full list is in the [bibliography](../reference/bibliography.md).
