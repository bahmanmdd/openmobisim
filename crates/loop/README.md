# openmobisim-loop

The loop: five stages per iteration, search, choose, load, measure, assess, iterated to the
stopping rule; and the three tables a run returns. Depends on `openmobisim-types`, `-graph`,
`-loading`, `-search`, `-choice`, `-transit`, `-demand` and `rayon`, nothing else.

## Inputs
- The `Network` (`openmobisim-graph`), the `Demand` (`openmobisim-demand`, built on that
  network and on the timetable), and the `Timetable` (`openmobisim-transit`) or none.
- The window `(start, end)`, two seconds of the clock: the study period, the user's, never
  defaulted; a whole number of link bins long.
- `Parameters` (`PARAMETERS.md`, the rulings): `link_bin` (s), `iteration_cap`, `seed`,
  `boarding_slack` (s), `max_rides`, `choice` (deterministic or logit), `nest_scale`, and per
  user class the three coefficients on `time_s`, `money_eur` and `ln_path_size`.

## Outputs
`run::run` returns a `Run`:
- `trips`, one row per trip in id order: class, departure, the mode taken, the arrival at the
  door (seconds after the window's start; `NaN` for a trip without an alternative or
  unfinished, whose arrival is unknown), the door-to-door travel time (the same), whether it finished,
  the cost of the alternative taken, the size of its set. `set(trip)` and `taken(trip)` give the
  alternatives themselves (mode, legs, time, `ln PS`).
- `links`, the last loading's table (`LinkBins`): per link and bin of entry, cars with their
  crossing times, bikes, walkers.
- `iterations`, one row per iteration: the mean trip time of the finished trips, the finished,
  unfinished and without-alternative counts, the switch share, the found share, the relative
  gap, and the seconds of each stage. `stopped_by_rule` says whether the rule or the cap ended
  the run.

## Units
Seconds and metres; `f64` seconds after the window's start inside, `Second` on the clock at the
boundary; euros for money; a cost is `Σ β x` in the choice model's unit (seconds in a time-only
run).

## Meanings and assumptions
- **An alternative** is its mode and three legs: a walk off the network (the gap between the
  door and its node, or a transit access walk, priced by the demand's seconds and loaded
  nowhere), then a route on the network (car, bike or walk; empty when the origin's node is
  the destination's) or a journey on the timetable, then a walk. A set is append-only, so an
  alternative keeps its position and the traveller's taste (the rulings).
- **Search:** cars every iteration (`fastest_many` on the last loading's reading, free flow at
  iteration 0); bikes, walkers and transit at iteration 0 only, since their costs never change
  in 2.0, a property, not an option. A route not yet in the set is appended; it is *found* when
  its cost on time and money is strictly below the set's cheapest (a trip offered its first
  alternative counts as found). A journey comes from the demand's access and egress lists, the
  traveller at each access stop on the whole second its walk ends.
- **Choose:** rows `[time_s, money_eur, ln_path_size]`, nest = mode; at iteration 0 everyone
  chooses; at `k ≥ 1` a traveller re-chooses with probability `1 / (k + 1)` (the method of
  successive averages, Sheffi 1985), drawn from `Stream::MsaReselection` at (trip, k); the
  others keep their alternative.
- **Load:** the chosen route legs as vehicles on the point queue, each entered on the whole
  second its walk ends; an empty route and a journey load nothing.
- **Measure:** every alternative of every set, taken or not, priced again by walking its legs
  on the new reading, then the path sizes (Ben-Akiva and Bierlaire 1999) over each set's routes
  of one mode; a journey ends when its last ride arrives, so the transit crate's rounded
  `Journey::arrival` is never read.
- **Assess:** the mean door-to-door time over the finished trips; unfinished = finished before
  the window's end is false (a loaded trip by the loading, plus its last walk; an unloaded one by
  its departure plus its measured time); the switch share and the found share over the trips
  with an alternative; the gap `Σ (c_taken − c_least) / Σ c_least` over the sets at the measured
  costs, reported, never judged.
- **Stop** after an iteration when its found share is at most 0.5 % and the range of the last
  five mean trip times, greatest less least, is at most 0.3 % of the latest (never while one of
  the five has no finished trip); always at the cap. The three shares and the gap are `NaN`
  when no trip has an alternative.
- Money is 0 for every alternative until a source of money comes through the door; gaps are
  walked at the walking speed by every mode. Transit's walking legs are not on links: the
  links table counts the walkers of walk alternatives only.
- Determinism: every parallel stage files its result by trip, every sum is in fixed order and
  every draw is keyed by identity, so the three tables are the same at any thread count and
  over two runs.

## Errors
`run` refuses, never repairs, with a `RunError`: `link_bin`, `iteration_cap` or `max_rides`
zero; coefficient rows not one per class; a window that is not whole bins, is empty or runs
past the clock; a trip departing outside the window, or reaching the network by some mode at or
after its end (named by external id); the choice model's and the loading's refusals.

## Bikes
A bike alternative is a route on the links a bike may use at the bike's speed, in the same set
and the same rows as the car's and transit's, its nest its mode; it is loaded as a vehicle that
interacts with nothing and is counted per bin; its gaps are walked. A bike rides 1 500 m in
360 s through the loop, 405.5 s with 30 m gaps at each end (22.5 s walked, the ride from the
whole second 23).

## Tests
`cargo test -p openmobisim-loop`. Each expected value comes from outside the crate: test 3,
two routes with a bottleneck against Vickrey's closed form (`t* = 345.6 s`, mean 452.5 s, the
split `c1 / q = 2/3` after `t*`, equal times at `t2`); test 4, Braess with capacities against
the capacity-constrained steady state by hand (Cominetti, Correa and Olver 2022: 19 min in
thirds with 8 min queues, 11 min without the link, 3 min at 900 veh/h); tests 5 and 6,
Nguyen–Dupuis and Sioux Falls against the same linear program solved by scipy
(`tests/equilibrium_lp.py`, Larsson and Patriksson 1995) at a demand where its total time is
unique: the mean trip, `Σ τ x`, the OD times, and every link with a standing queue at its
capacity; by hand, the bike's 360 s and 405.5 s, the walker's 1 125 s, a journey at 922.5 s,
the cheaper mode taken, an empty route, an unfinished walker, the boarding slack and the most
rides, the refusals, the stopping rule on hand-made rows; determinism over 1 and 8 threads and
two runs (3 000 trips, three modes, the nested logit, 12 iterations).
