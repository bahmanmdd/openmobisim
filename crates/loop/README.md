# openmobisim-loop

The loop: five stages per iteration, search, choose, load, measure, assess, iterated to the
stopping rule; and the three tables a run returns. Depends on `openmobisim-types`, `-graph`,
`-loading`, `-search`, `-choice`, `-transit`, `-demand` and `rayon`, nothing else.

## Inputs
- The `Network` (`openmobisim-graph`), the `Demand` (`openmobisim-demand`, built on that
  network and on the timetable), and the `Timetable` (`openmobisim-transit`) or none.
- The window `(start, end)`, two seconds of the clock: the study period, the user's, never
  defaulted; a whole number of link bins long (else its end is moved up to one, below).
- `Parameters` (defaults and their sources in `PARAMETERS.md`): `link_bin` (s), `iteration_cap`,
  `iteration_floor`, `seed`, `boarding_slack` (s), `max_rides`, `nearby_stops`, `choice`
  (deterministic or logit), `nest_scale`, and per user class the sixteen coefficients of the
  choice row (below).

## Outputs
`run::run` returns a `Run`:
- `trips`, one row per trip in id order: whether it was run (`kept`), class, departure, the mode taken, the arrival at the
  door (seconds after the window's start; `NaN` for a trip without an alternative or
  unfinished, whose arrival is unknown), the door-to-door travel time (the same), whether it finished,
  the cost of the alternative taken, the size of its set. `set(trip)` and `taken(trip)` give the
  alternatives themselves (mode, legs, time, `ln PS`).
- `links`, the last loading's table (`LinkBins`): per link and bin of entry, cars with their
  crossing times, bikes, walkers; and `links.crossings()`, per link every car of the last
  loading in the order it entered, its entry and exit instants (seconds after the window's
  start) and its trip.
- `iterations`, one row per iteration: the mean trip time of the finished trips, the finished,
  unfinished and without-alternative counts, `without_transit` (trips with transit open to them
  and no journey offered) and `without_stop` (of those, the trips with no stop within
  `stop_walk_max` of their origin or destination, which a larger `stop_walk_max` can reach; the
  rest have stops at both ends and no journey between them), the offered share (trips offered a
  new choice; 1 at iteration 0, where everyone chooses), the switch share, the found share, the
  relative gap, and the seconds of each stage. `stopped_by_rule` says whether the rule or the cap ended
  the run.

## Units
Seconds and metres; `f64` seconds after the window's start inside, `Second` on the clock at the
boundary; euros for money; the choice row's times in minutes; a cost is `Σ β x` in the choice
model's unit (minutes in a time-only run: 1 on each of the nine time columns, 0 on the rest).

## Meanings and assumptions
- **An alternative** is its mode and three legs: a walk off the network (the gap between the
  door and its node, or a transit access walk, priced by the demand's seconds and loaded
  nowhere), then a route on the network (car, bike or walk; empty when the origin's node is
  the destination's) or a journey on the timetable, then a walk. A set is append-only, so an
  alternative keeps its position and the traveller's taste (its random draw, fixed for the run:
  `crates/choice/README.md`).
- **Search:** cars every iteration (`fastest_many` on the last loading's reading, free flow at
  iteration 0); bikes, walkers and transit at iteration 0 only, since their costs never change
  in release 0.2, a property, not an option. A route not yet in the set is appended; it is *found* when
  its cost without the path-size term is strictly below that of every alternative of its mode
  in the set (the first alternative of a mode counts as found): a faster car route counts
  for a traveller whose bike is cheaper still. A set that grew has its path sizes rewritten at once, so
  the logit's choice of the same iteration reads the path sizes of the set it chooses from. Transit offers, to a set without transit, every journey of
  `Raptor::by_rides`: for each number of rides that reaches the door earlier than every smaller
  number, the earliest journey with that many rides, fewest rides first (the last is the
  earliest journey). A journey comes from the demand's access and egress lists, the traveller
  at each access stop on the whole second its walk ends. `nearby_stops` counts the searches for the earliest journeys per trip end: at 1 that
  search alone; above 1 the same is asked again from each of the `nearby_stops − 1` stops
  nearest the origin alone (to any stop of the egress list), and to each of the
  `nearby_stops − 1` stops nearest the destination alone (from any stop of the access list); nearest is the least walk, ties to the lower stop id; a list of one stop
  is not asked again. The earliest journey is not always the cheapest (a slow run boarded at
  once against a fast one waited for; a nearer stop with a later run), so transit also offers
  every journey of `Cheapest::by_rides` between any stops of the two lists: for each number of
  rides that makes a journey cheaper than every smaller number, the cheapest for the
  traveller's class, by its coefficients of the walks to and from the stops, the waits, the
  rides, the walks between rides and the transfers (each at or above zero, or the run is
  refused). A journey found twice is offered once, and of the journeys that ride the
  same runs in the same order (they differ in the stop boarded or left at) only the cheapest
  for the traveller is offered, the first found at equal cost.
- **Choose:** one row per alternative of sixteen columns, a utility per mode, times in
  minutes: `car_drive_min`, `bike_ride_min`, `walk_min`, `transit_in_vehicle_min`,
  `transit_access_min`, `transit_egress_min`, `transit_first_wait_min`,
  `transit_transfer_wait_min`, `transit_transfer_walk_min`, `transit_transfers`, `money_eur`,
  `car_ln_path_size`, `is_car`, `is_bike`, `is_walk`, `is_transit`. An alternative fills only
  its mode's columns: a car or bike its time on the network (door to door less its two gap
  walks; the route is entered at the instant the first walk ends) and its gap walks in
  `walk_min`; a walk
  its whole time in `walk_min`; a journey its parts, the first wait from the end of the access
  walk to the first boarding (the slack included), so the nine times of any alternative add up
  to its door-to-door time; its own mode's constant 1. The default coefficients (cost, positive
  is worse) are 0.1, 0.1, 0.165, 0.1, 0.165, 0.165, 0.145, 0.145, 0.165, 0.5, 0.6, −1, 0, 0,
  0.5, 0.25 (`PARAMETERS.md`). The coefficient of `car_ln_path_size`, its sign turned, is the
  exponent of the path size in a car route's weight inside its nest: −1 is the path-size logit
  (Ben-Akiva and Bierlaire 1999) at every nest scale, in which n identical copies of a route,
  each of path size 1/n, count as the one route. The nested logit divides a row's whole cost by
  the nest scale μ inside its nest, so the logit reads the coefficient times μ (−0.5 at the
  default 0.5). Under the least-cost choice it is read as 0, whatever is given: the path size
  corrects the logit for routes that overlap, and the least cost has no such error to correct.
  The path sizes are computed and reported all the same, and the run's record keeps the
  coefficient as given. The nest is the mode; at iteration 0 everyone chooses. At `k ≥ 1` a
  traveller is offered a new choice with probability `max(1 / (k + 1), s)`: the step of the
  method of successive averages (Sheffi 1985), or `s = r = (c − b) / c` when `r` is above 0.2
  (else `s = 0`; never above 1), with `c` the cost of the alternative it holds and `b` the
  least cost among the alternatives of the same mode in its own set, as the choice reads them
  (systematic costs, the path-size term included under the logit, at the times of the last
  measure); `s = 0`
  when `c` is not positive. Drawn from `Stream::MsaReselection` at (trip, k). A traveller that
  is offered takes the model's choice over its whole set, its mode included, whatever it held;
  the others keep their alternative.
- **Load:** the chosen route legs as vehicles on the point queue, each entered on the whole
  second its walk ends; an empty route and a journey load nothing.
- **Measure:** every alternative with a car route, taken or not, priced again by walking its
  legs on the new reading as its traveller (a route leg entered at the instant it is reached),
  so the alternative taken costs the time its trip took (`tests/taken_is_the_trip.rs`). A
  bike's, a walker's and a journey's time is the one the search priced it at: none of them
  reads a car's crossing, so pricing it again would give the same number, a property, not an
  option (a journey ends when its last ride arrives, so the transit crate's rounded
  `Journey::arrival` is never read). The path sizes (Ben-Akiva and Bierlaire 1999) over each
  set's car routes are the search's, rewritten whenever a set grows: they depend on who is in
  the set, not on the reading (a bike or walk route's `ln PS` is 0: their sets hold one route
  each, found at iteration 0).
- **Assess:** the mean door-to-door time over the finished trips; unfinished = finished before
  the window's end is false (a loaded trip by the loading, plus its last walk; an unloaded one by
  its departure plus its measured time); the offered share, the switch share and the found share
  over the trips with an alternative; the gap `Σ (c_taken − c_least) / Σ c_least` over the sets at the measured
  costs, reported, never judged.
- **Stop** after an iteration when its found share is at most 0.5 % and the range of the last
  five mean trip times, greatest less least, is at most 0.3 % of the latest (never while one of
  the five has no finished trip); not before `iteration_floor` iterations have run; always at
  the cap. The rule reads the slope of the mean, which the method of successive averages makes
  about the residual over the iteration count, so a run it stops early can still be short of
  equilibrium; the floor lets a test run to a fixed count. The offered, switch and found shares and the gap are `NaN`
  when no trip has an alternative.
- **What the choice is over.** A traveller chooses among the alternatives in its set, and the
  set holds what the search found: per mode the fastest route at iteration 0, for cars also
  every route that was the fastest at a later iteration's reading, and the transit journeys
  above. It is not every route of the network: a car route that is never the fastest at any
  reading is never offered, and of two routes of exactly equal time the search's tie rule
  (`openmobisim-search`) finds one. Shares and the costs of modes are those of a choice over
  these sets, and a logit share between two routes exists only once both were found.
- **The window and the mean.** The loading stops at the window's end; a trip not at its door
  by then is unfinished and has no travel time. The mean trip time, which the stopping rule
  reads, is over the finished trips of each iteration: when the unfinished count differs
  between two iterations their means are over different trips, and a queue that pushes trips
  past the end removes the longest of them from the mean. The iterations table gives the
  finished and unfinished counts beside every mean; the gap is over every trip with an
  alternative, at measured costs. A window long enough for every trip to finish (the
  unfinished count 0) makes the means comparable.
- **Who chooses again.** Everyone is offered a new choice at least at the plain method's rate
  `1 / (k + 1)` (under the deterministic model a traveller holding its cheapest keeps it; under
  the logit its taste decides). A traveller whose best alternative of the same mode would save
  more than 20 % of the cost of the one it holds (one more than 25 % dearer than that best) is
  offered at the larger rate of the share it would save, after Smith's proportional switching
  (1984; his flows move between pairs of routes in proportion to their cost difference, by a
  small step), outside a band of indifference (Mahmassani and Chang 1987). When cost is
  proportional to time, one whose car route reads 30 min where its best car route reads 20 min
  would save a third: it is offered with probability 1/2 at `k = 1` and 1/3 from `k = 2` on;
  one reading 22 min there would save 9 % and is offered at the plain rate. The chance depends
  only on the traveller's own two costs, for its origin, destination and departure time, from
  the last iteration. Only alternatives of the mode held enter `b`, but a traveller that is
  offered chooses over its whole set: one above the band also reconsiders its mode, at the
  rate `r`, and travellers of a mode with several alternatives (car routes) are asked more
  often than those of a mode with one. With one alternative per mode the rule is the plain
  method. The band 0.2 is a fixed constant of the rule, not a parameter: within it only the
  plain step acts, so travellers do not keep moving between near-equal routes at a rate that
  does not fall with `k`. Measured without a band (Braess below, 4 500 travellers, 20 seeds,
  the mean trip less 1 140 s, mean ± standard error): −8.5 ± 2.1 s under the plain method;
  −13.9 ± 2.5, −17.5 ± 2.7 and −24.3 ± 3.3 s at constant rates 0.25, 0.5 and 0.75. Near
  equilibrium only the plain step acts for the least-cost choice; under the logit a traveller
  may hold, by its taste, an alternative more than the band dearer than the cheapest of its
  mode, and is then offered at the rate `r` every iteration and, at unchanged costs, repeats
  its choice. Limits: `r` is the same in any unit of cost but not after a constant is added to
  every cost (a long shared leg, a mode constant or a fare shrinks it, and below the band the
  rule is the plain method); it favours short trips (5 min lost beside a 10 min best is
  `r = 0.33`, beside a 60 min best 0.077); above the band the rate does not fall with `k`, so
  travellers behind one queue who read the same cost can move together (an early swing on
  heavily oversaturated small networks); and the chance jumps at the band. Within the band the
  plain method's limit stays: after `K` iterations about one such traveller in `K` still holds
  the choice it made at each earlier iteration, on that iteration's costs. The gap reports what
  is left.
- Money is 0 for every alternative until a source of money comes through the door; gaps are
  walked at the walking speed by every mode. Transit's walking legs are not on links: the
  links table counts the walkers of walk alternatives only.
- Determinism: every parallel stage files its result by trip, every sum is in fixed order and
  every draw is keyed by identity, so the three tables are the same at any thread count and
  over two runs.

## Errors
`run` refuses with a `RunError` only what no run can be made of: `link_bin`, `iteration_cap`,
`max_rides` or `nearby_stops` zero; coefficient rows not one per class; an empty window or one
running past the clock; with a timetable, a class whose coefficient of a transit time or of the
transfers is negative or not finite; the choice model's and the loading's refusals.

## Repairs
What the window cannot hold is set right and noted in `Run::repairs` by kind (count and first
five ids); `Run::window` is the window run (`Run::window_given` the window given), `Run::dropped`
the trips dropped.

| Key | What | What is done |
|---|---|---|
| `window_end_moved` | a window that is not a whole number of `link_bin` | its end moved up to the next whole bin (the id: the end used, a second of the clock) |
| `trips_outside_window` | a trip departing before the window's start or at or after its end | dropped |
| `modes_after_window` | a trip whose walk to the network by car, bike or walk would end at or after the window's end (once per trip) | each such mode taken out of its set for the run |
| `trips_after_window` | a trip left with no mode by that | dropped |

A trip dropped is not run: no alternative, in no count of the iterations table, and its row of
`trips` says so (`kept` false), so the outputs leave it out. For bikes: a bike trip whose door
is so far from the bike network that its walk there would end after the window loses the bike.

## Bikes
A bike alternative is a route on the links a bike may use at the bike's speed, in the same set
and the same rows as the car's and transit's, its nest its mode; it is loaded as a vehicle that
interacts with nothing and is counted per bin; its gaps are walked. A bike rides 1 500 m in
360 s through the loop, 405 s with 30 m gaps at each end (22.5 s walked, the ride from then
on).

## Tests
`cargo test -p openmobisim-loop`. Each expected value comes from outside the crate: test 3,
two routes with a bottleneck against Vickrey's closed form (`t* = 345.6 s`, mean 452.5 s, the
split `c1 / q = 2/3` after `t*`, equal times at `t2`); test 4, Braess with capacities against
the capacity-constrained steady state by hand (Cominetti, Correa and Olver 2022: 19 min in
thirds with 8 min queues, 11 min without the link, 3 min at 900 veh/h); tests 5 and 6,
Nguyen–Dupuis and Sioux Falls against the same linear program solved by scipy
(`tests/equilibrium_lp.py`, Larsson and Patriksson 1995) at a demand where its total time is
unique: the mean trip, `Σ τ x`, the OD times, and every link with a standing queue at its
capacity (for test 5 the mean over seeds 0 to 19, release only; CI runs it by name). Tests 3
and 4 at 4 500 travellers or fewer and test 5's steady hour are judged against theory on the
mean over seeds 0 to 4, one seed scattering about as much as their tolerances, with each
seed's values and the standard error printed; part 1 and the per-traveller checks (the long
route at free flow, which links queue) in every seed. Of them, Braess's 19-minute case and
Nguyen–Dupuis's steady hour are release only (about 20 s each in debug); CI runs them by name.
Test 3's iteration-1 hand test, test 4's convergence tier (72 000 travellers) and test 6 are
judged on seed 0. Each of
tests 3 to 6 and the bottleneck below reports in order: part 1, the queue and the record (each
car's exit against the point queue replayed in the test on the recorded entries,
`links.crossings()`; each car's entries along its route, the first at its departure plus the
walk before its route, each next at its exit from the link before, each link once; both at
1e-6 s, with cars recorded and queued: a pass means each link's queue and the hand-over between
links are right for the cars recorded); part 2, the flows; part 3, the choices (each traveller against the best of its own set,
or the car share against the logit share at the times met; printed, not judged); then the
mixed numbers that follow from the three (the mean trip, a queue's delay); by hand, the bike's 360 s and 405 s, the walker's 1 125 s, a journey at 922.5 s,
the cheaper mode taken, the nearest stop tried alone and two journeys on one run
(`tests/nearby_stops.rs`), an empty route, an unfinished walker, the boarding slack and the most
rides, the refusals and the window's repairs, the stopping rule on hand-made rows; the choice row of each mode by hand
(`tests/choice_row.rs`: a car and a bike with gap walks, a walk, a journey with one transfer,
every part in minutes; the path size of two overlapping car routes, 0 for bike and walk
routes; and of three car routes sharing one link among three and one among two, one link
slower than the rest, by length: 7/12, 14/27 and 17/30); the path size of every car route of
every set at the end of a run, three routes of 800, 1 000 and 1 200 m sharing their first
600 m, by hand (`tests/path_size_in_a_run.rs`); on the same three routes a least-cost run the
same whatever the coefficient of the path size (0, −1 or −5), each trip's cost the minutes of
the alternative it takes, and every coefficient read as given but the path size's, read as 0
(`tests/least_cost_reads_no_path_size.rs`); under the logit the coefficient of the path size
read as −μ at μ of 0.5, 0.25 and 1, the rest as given, and three car routes with path sizes
0.5, 0.6 and 2/3 in one nest and a bike in another, a logit built in the test from the
parameters as read with a nest per mode, against the closed form with the path size at the exponent 1 by hand within 1e-12, and in a
logit run on the three routes above at μ of 0.5, 0.25 and 1, each trip's cost the minutes of
the route it takes less μ times the logarithm of its path size by hand, within 1e-9
(`tests/path_size_in_the_car_nest.rs`);
the default run against outside answers: without congestion, the closed-form logit
shares of car, bike, walk and transit (`tests/no_congestion.rs`), and car against bike behind
a bottleneck, a hand-derived trajectory solved by scipy (`tests/bottleneck_mode_choice.rs`,
`tests/fixtures/bottleneck/`; release only, skipped by `cargo test`; CI
runs it: `cargo test --release -p openmobisim-loop --test bottleneck_mode_choice --test braess
--test nguyen_dupuis -- --ignored --exact
one_run_follows_the_trajectory_and_reaches_the_steady_state_within_sampling_error
with_the_added_link_every_trip_takes_19_min_split_in_thirds_with_8_min_queues
the_steady_hour_is_the_capacity_constrained_equilibrium
over_twenty_seeds_each_queued_link_enters_at_its_capacity_within_3_percent`);
determinism of the whole chain over 1 and 8 threads and two runs (`tests/determinism.rs`: 3 000
trips, four modes, three transit lines, the nested logit; the demand, the sets and the three
tables compared to the bit), the one thread-count comparison of the search, the demand and
the loop.
