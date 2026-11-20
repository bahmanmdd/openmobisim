# openmobisim-transit

The timetable of one service day and the earliest-arrival journey over it, by RAPTOR (Delling,
Pajor and Werneck 2015), the earliest journey per number of rides, and the cheapest journey per
number of rides for a cost linear in a journey's parts (`cheapest::Cheapest::by_rides`: the
rounds of RAPTOR with a set of labels, arrival and cost, per stop, bounded from the start by
the costs of the same question's earliest-arrival journeys, which are journeys of it too; its
module documentation gives the cost, when a label makes another useless, why every catchable
run is boarded, why the bound changes no answer and why that needs the runs of a pattern left
in rank order; tested against an exhaustive enumeration on 300 random timetables, 12 000 questions, and by
hand). Depends on `openmobisim-types` and `rayon`, nothing else. It knows
stops by position only: linking stops to the walk network, and choosing which stops are within
reach of a trip's ends, is the caller's; the access and egress lists arrive already chosen.

## Inputs
- A `TimetableBuilder`: stops (external id, position x and y in metres); runs (external id,
  the calls in order, each a stop's external id, an arrival and a departure second); footpaths
  (from stop, to stop, seconds as a `Duration`; given, never derived). `build()` returns the
  `Timetable`.
- The boarding slack, a whole number of seconds applied at every boarding.
- A `Query` per journey wanted: `access` (stops and the second the traveller is at each),
  `egress` (stops and the walk from each to the door, seconds), `max_rides`.

## Outputs
- `Raptor::new(&timetable, slack)` is the search with its scratch, reusable across queries.
  `Raptor::earliest(&query)` returns `Option<Journey>`: `None` when no journey of at least one
  ride reaches an egress stop. `Raptor::by_rides(&query)` returns `Vec<Journey>`: for each
  number of rides whose door arrival is earlier than every smaller number's, the earliest
  journey with that many rides (by the same rule as `earliest`), fewest rides first; the last
  is `earliest`'s; empty when there is none. A slower journey of fewer rides is kept, a later
  one of more rides is not.
- `Journey { rides, footpaths, access, egress, arrival }`: each `Ride { run, board: (stop,
  second), alight: (stop, second) }`; between ride `i` and ride `i + 1`, `Some(Footpath { from,
  to, seconds })` or `None` (boarded where alighted); the access stop and second; the egress
  stop and walk; the arrival at the door. Its parts in seconds: `wait()` (over every boarding,
  departure less the instant the traveller is at the stop, the slack included), `in_vehicle()`,
  `transfer_walk()`, `egress_walk()`; and `ride_count()`. The access walk is the caller's. The
  access second plus the four parts is the door instant, the arrival before it is rounded up.
- `earliest_many(&timetable, slack, &queries)` and `by_rides_many` answer every query in
  parallel (rayon, one scratch per thread); answer `i` is query `i`'s, the same at any thread
  count. `cheapest_by_rides_many(&timetable, slack, &cost_queries, &earliest)` does the same
  for `Cheapest::by_rides_given`: the caller hands over, per question, the answer of
  `Raptor::by_rides` to it (the cheapest search's bound), so that a caller that has asked for
  it already does not have it searched for twice.
- `Timetable` accessors, read-only: stop and run counts and external ids, stop positions, a
  run's calls and pattern, a pattern's stops and runs, a stop's footpaths.
- `Timetable::fingerprint()`, a `u64` (the FNV-1a hash of `openmobisim-types`) over what the
  timetable was built from, in this order: the stop count, then per stop in id order its
  external id and its x and y in metres; the run count, then per run in id order its external
  id, its call count and per call the stop id, the arrival and the departure; the footpath
  count, then per footpath (by stop walked from, stop walked to, seconds) the two stop ids and
  the seconds as given. The patterns follow from the runs and are not fed. The same input, in
  any order, gives the same fingerprint; a change to any value of a stop, a call or a footpath
  changes it.

## Units
Seconds and metres. Times of the timetable and of the journey are whole seconds of the clock
(`Second`); walks are `Duration`s. A walk is put on the clock rounded up to the whole second:
the slack being whole and departures on whole seconds, a traveller at a stop at 1 400.5 s misses
the run leaving at 1 400 s and catches the one at 1 401 s exactly as the unrounded instant
says. The journey keeps the walks as given, so its parts are not rounded. The arrival is the
last ride's arrival plus the egress walk, rounded up to the second.

## Meanings and assumptions
- A journey starts at an access stop at its second; boards a run at a stop only if the
  traveller is there `slack` seconds before it leaves, at every boarding including the first;
  rides it to a later call; walks at most one footpath between two rides; takes at most
  `max_rides` rides; ends at an egress stop. No footpath before the first ride or after the
  last: those walks are the access and egress lists. Walking all the way is not a journey.
- The answer is the earliest arrival; among equal arrivals, the fewest rides; then the last
  ride on the run of lowest id; then the egress stop of lowest id; the last run boarded at the
  first stop where it can be caught, and what comes before it the earliest way there, by the
  fewest rides.
- A pattern is a set of runs on the same stops in the same order that never overtake one
  another. The runs of one stop sequence, in order of departure from its first stop, each join
  the first pattern of that sequence whose last run they do not overtake, else open a new one:
  an express and a local on the same stops are two patterns. Patterns are numbered by their
  lowest run id, so the same input in any order gives the same timetable. RAPTOR scans patterns round by round (round `k`: at most `k` rides), boarding the
  earliest run catchable, and walks footpaths once per round from the stops a ride improved;
  footpaths need not be closed under composition.
- A day's timetable, nothing else: no calendar, no service changes, no realised times, no buses
  on roads, no walking limit or window cut (the caller's lists carry those).

## Errors
`build()` refuses, naming the run or stop by external id: a stop or run given twice, a run of
fewer than two calls, a call at an unknown stop, a call out of order (departing before it
arrives, or arriving before the call before it departs), a footpath naming an unknown stop or
of negative or infinite seconds. Runs that overtake are not an error: they go to patterns of
their own.
`Raptor::new` panics for a slack that is not a whole, non-negative number of seconds;
`earliest` panics for a stop the timetable does not have and for a negative or infinite egress
walk.

## Bikes
Nothing here is a bike: a run carries passengers on its timetable, and the walks are the
caller's. A bike-and-ride journey, later, is an access by bike to the same query: the caller
rides to the stops and gives the seconds it reaches each as the access list. No bike value is
in the timetable's fingerprint.

## Tests
`cargo test -p openmobisim-transit`. Each expected value comes from outside the crate: RAPTOR
equal to a brute force that enumerates every journey by the definition (arrival, rides and last
run) on 300 random timetables, 1 500 queries, and `by_rides` equal on the same queries to the
brute force at each number of rides `k` (the earliest arrival with at most `k` rides); the wait for the first of two lines A to B at 600
and 870 s, against uniform independent phases (mean `H₁/2 − H₁²/(6H₂)` = 231.0 s within 10 s,
the share `1 − H₁/(2H₂)` = 0.655 within 0.02), and a fast and a slow line against
`E[min(U₁ + 900, U₂ + 1 500)]` integrated numerically in the test; one line at regular and
irregular headways against Osuna and Newell's `E[H²]/(2E[H])`; the network of Spiess and Florian
(1989) as timetables, every traveller equal to the brute force, the mean door time reported
against the paper's 27.75 min (a frequency-based strategy, not asserted); transfers, slack and
footpaths by hand; `by_rides` by hand (a slow direct run and a faster connection: two journeys;
a direct run as early or earlier: one); the cheapest journey unchanged by an access from which
every journey costs more (a property; two runs equal in cost, the dear access catching the later
one, then the earlier one); the refusals; `earliest_many`'s determinism over 1 and 8
threads and two runs (1 000 queries; a run does not call it). `by_rides_many`'s, the one a run
calls, is tested with the whole chain (`crates/loop/tests/determinism.rs`).
The fingerprint as a property (`tests/fingerprint.rs`): two builds of the same rows equal; one
value changed, all else equal, in each of the 10 input columns (stop id, x, y; run id; a call's
stop, arrival, departure; a footpath's from, to, seconds), each a named case, changes it; the
same rows in reverse order give the same fingerprint; two stops, one run and one footpath hash
to the byte stream above written out by hand.
The run time is reported by hand: `cargo test --release -p openmobisim-transit --test run_time
-- --ignored --nocapture`.
