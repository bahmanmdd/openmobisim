# openmobisim-loading

The loading: cars on the point queue, bikes and walkers at free flow, over one window; and the
first-in-first-out reading of what it recorded, the time to cross a link when entering it at an
instant. Depends on `openmobisim-types` and `openmobisim-graph` only. Transit is not loaded here
yet.

## Inputs
- The `Network` (`openmobisim-graph`): per link its two nodes, length, free-flow time per mode,
  capacity and modes allowed.
- The vehicles (`input::Vehicle`), one per trip: the trip id, the mode (car, bike or walk), the
  route (the links in order, at least one) and the departure (a whole second of the clock).
- The window as a `StepGrid`: its start, its end, and its step, which is the length of a bin.

## Outputs
`point_queue::load` returns a `Loading`, three tables held as structures of arrays:
- `finished`: per trip that arrived before the window's end, sorted by trip: mode, departure,
  arrival; `travel_time(i)` is door to door, arrival less departure.
- `unfinished`: per trip still travelling at the end: mode, departure, the link it is on and
  when it entered it.
- `links` (`LinkBins`): per (link, bin of entry) someone entered, sorted by link then bin: cars
  (count, sum of crossing times, first and last car to enter, each its entry and exit), bikes
  (count), walkers (count). NaN for the entrants of a row without cars.

`link_times::LinkTimes::new(network, links)` reads the table: `time(link, mode, t)` and
`route_time(route, mode, departure)`.

## Units
Seconds and metres; vehicles per second inside (the capacity's headway is 1 / capacity).
Instants inside a loading are `f64` seconds since the window's start (a `Duration`), never
floored; departures and bins are whole seconds and steps of the window (`Second`, `StepIndex`).

## Meanings and assumptions
- Point queue (Vickrey 1969): a car entering link `a` at `t` leaves at
  `max(t + τ_a, x_prev + 1 / capacity_a)`, `τ_a` the car free-flow time (length / speed) and
  `x_prev` the exit of the car that entered before it; it enters the next link then. Storage is
  infinite; no node rule, merge rule or signal holds a car. First-in-first-out by construction.
- Events: a car reaching a link's downstream end, in the order of (instant, trip id).
- The window is half-open. Events run while their instant is before the end. A crossing that
  starts before the end is recorded at its entry with its exact exit, even one past the end;
  none starts at or after the end. A trip arriving at or after the end is unfinished.
- Link time reading (the meanings table): the exit `e(t)` of an entry at `t` is interpolated on
  straight lines between the recorded first and last entrants of each bin and across a bin
  boundary; after a bin's last entrant with the next bin empty, `max(x_last, t + τ)`; before a
  bin's first entrant with the bin before empty, `max(x_before, t + τ_first)`; before the first
  recorded bin and on a link no car entered, free flow. The time is `max(e(t) − t, τ)`. It never
  falls below free flow and the exit it reads never falls as `t` grows.

## Errors
`load` refuses, never repairs, with a `LoadError` naming the trip: a trip given twice; transit;
an empty route; a departure before the window's start or at or after its end; a link the network
does not have; a link that forbids the mode; two links of a route that do not follow one another.
The first fault in trip order is named, whatever the order of the vehicles.

## Bikes
A bike and a walker ride or walk each link of their route in its length over their mode's speed
(the graph's `bike_speed` and `walk_speed`), entering the next link when they leave one. They
interact with nothing: their times and counts are the same, to the bit, with or without cars on
their links. A bike rides 1 500 m at 15 km/h in 360 s, on a street or a cycle path; the table
counts it at entry on every link of its route.

## Tests
`cargo test -p openmobisim-loading`. Each expected value comes from outside the crate: Vickrey's
bottleneck by hand (1 200 veh/h fed a car every 2 s: car k delayed k s, 44 850 veh·s, the queue
clear at λT/μ = 900 s); the same link with unlimited capacity (every exit t + τ to the bit,
72 s door to door); Newell's tandem (83.3 and 166.7 veh·h, reversed 250 and 0, and Vickrey's recursion car by
car); the window's end (each bin as with a window four times longer, by hand τ + t/2); the reading
by hand at each of its rules, and its properties (own time at an entrant, linear between two,
FIFO and above free flow over random arrivals, a route's time equal to the loaded one); test 9 by
hand (360 s, 750 s, a three-link sum, unchanged by 1 000 queued cars); conservation on a 20 × 20
grid; determinism over two runs and a shuffled input (nothing runs in parallel). The run time on
the 100 × 100 grid is reported by hand: `cargo test --release -p openmobisim-loading --test
run_time -- --ignored --nocapture`.
