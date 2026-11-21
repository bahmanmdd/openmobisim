# openmobisim-loading

The loading: cars on the point queue, bikes and walkers at free flow, over one window; and the
reading of what it recorded, the time a traveller takes to cross a link entering it at an
instant. Depends on `openmobisim-types` and `openmobisim-graph` only. Transit is not loaded here
yet.

## Inputs
- The `Network` (`openmobisim-graph`): per link its two nodes, length, free-flow time per mode,
  capacity and modes allowed.
- The vehicles (`input::Vehicle`), one per trip: the trip id, the mode (car, bike or walk), the
  route (the links in order, at least one) and the departure (seconds since the window's
  start: any instant inside the window, not only a whole second).
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

The table also keeps every car's crossing (entry, exit, trip) per link, 20 bytes each, for the
reading; `LinkBins::crossings()` hands them out, per link in the order the cars entered it.

`link_times::LinkTimes::new(network, links)` reads them for a `Reader`, a traveller
(`Reader::of(trip)`) or one with no car in the loading (`Reader::stranger()`):
`exit(link, mode, t, reader)` is when it leaves the link and the reader for the next link,
`time(link, mode, t, reader)` the time that takes, `route_time(route, mode, departure, reader)`
a route walked link by link; `since_start(second)` places a second of the clock on the
reading's clock, seconds since the window's start.

## Units
Seconds and metres; vehicles per second inside (the capacity's headway is 1 / capacity).
Instants are `f64` seconds since the window's start (a `Duration`), never rounded: departures,
entries, exits and arrivals alike. Bins are steps of the window (`StepIndex`).

## Meanings and assumptions
- Point queue (Vickrey 1969): a car entering link `a` at `t` leaves at
  `max(t + τ_a, x_prev + 1 / capacity_a)`, `τ_a` the car free-flow time (length / speed) and
  `x_prev` the exit of the car that entered before it; it enters the next link then. Storage is
  infinite; no node rule, merge rule or signal holds a car. First-in-first-out by construction.
- Events: a car reaching a link's downstream end, in the order of (instant, trip id).
- The window is half-open. Events run while their instant is before the end. A crossing that
  starts before the end is recorded at its entry with its exact exit, even one past the end;
  none starts at or after the end. A trip arriving at or after the end is unfinished.
  So a traveller pricing a route reads a link it would enter at or after the end without the
  cars that would enter it at or after the end: only those that entered before the end are ahead
  of it, and the costs of trips still travelling at the end read too low. Where every car
  finishes within the window (the unfinished count 0, the condition that makes means
  comparable), no crossing is missing and this does not arise.
- Link time reading: a car entering a link at time `t` leaves at the later of two instants:
  `t` plus the free-flow time, and one headway after the car ahead of it. The headway is
  3 600 s divided by the link's capacity in vehicles per hour. Which recorded car is ahead of
  a traveller: (1) the cars that entered before `t`; (2) of the cars that entered at `t`,
  those of a lower trip, the loading's own order, and all of them for a traveller with no car
  in the loading; (3) never the traveller's own recorded car: where it is among the cars
  ahead, the cars after it are replayed without it; (4) not a recorded car whose place the
  traveller took on the link before (it left that link at the very instant that car did):
  that car is behind it, whether or not the traveller's own recorded car entered at the same
  instant. Free flow before the first car and on a link no car entered. Nothing is
  interpolated and no bin is read. A traveller walking its own route from its own departure
  reads the time it took, to the bit.
- The reading's limits, on a route the traveller did not take. The reading holds every other car
  as the loading recorded it. When the traveller's car is moved to another route, the cars
  behind its own car on the route it left move up, and the cars behind it on its new route move
  back; the reading can see neither. Where no other car's entry on a link of the route changes,
  the reading is exact. Where one does, it is off by one headway of that link for each car it
  holds on the wrong side of the traveller, early or late, and the queues further on can add to
  that: along a route the error is not bounded by one headway. Only loading again with the
  traveller moved settles it. Three ways it happens, the first two shown by hand
  (`tests/reload.rs`). A car held at a bottleneck further upstream moves up into the place the
  traveller's own car left and is ahead of the traveller, though the record has it entering
  after (452 s read, 456 s taken). The traveller took car A's place on the link before, and A
  had entered this link at the same instant as a car B whose trip lies between A's and the
  traveller's: B moves up, the traveller leaves when B was recorded to, and the reading takes B
  for the car behind it, though B is ahead and A behind; where A and B then go on by different
  links, the next link is read one car out (228 s read, 224 s taken, and 224 s read, 228 s taken
  when the two swap links). A car the traveller delayed without taking its place meets it again
  by another way: recorded ahead of it, in truth behind.

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
car); the merge of two approaches into a narrower link (test 2: Daganzo's rule with demand-proportional
priorities is the deterministic queue on the merged link, 1 800 and 600 veh/h into 1 800 veh/h, 90 300 veh·s by
Vickrey's recursion against the fluid 90 000, both approaches delayed alike); the window's end (each bin as with a
window four times longer, by hand τ + t/2); the reading
by hand at each of its pieces, a burst inside one bin read at any bin length and window start,
cars entering at one instant each reading its own time, the headway behind a car, and its
properties over random arrivals against the recursion replayed in the test (every car its own
time, FIFO, above free flow; a route's time equal to the loaded one, to the bit); the reading
on a route not taken against the loading run again with that one car moved (`tests/reload.rs`:
equal to the bit on three hand-built cases, one of them a traveller reaching a link at the
instant its own car did, ahead of the car whose place it took, 148 s over two links and 222 s
over three; that case is exact because no other car moves in it, though the first of the
limits can apply at that very point, where a car queued behind the traveller's own car on the
approach it left moves up, enters the link at that instant with a lower trip and is ahead of
it; and the first two ways of the limits by hand); test 9 by hand (360 s, 750 s, a three-link
sum, unchanged by 1 000 queued cars); conservation on a 20 × 20 grid; determinism over two
runs and a shuffled input (nothing runs in parallel). The run time on
the 100 × 100 grid is reported by hand: `cargo test --release -p openmobisim-loading --test
run_time -- --ignored --nocapture`.
