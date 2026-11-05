# openmobisim-search

The fastest route per mode at the current link times: the route of earliest arrival from one
node to another, setting out at a given second, each link's time read from the last loading
at the instant the route enters it. Depends on `openmobisim-types`, `openmobisim-graph`,
`openmobisim-loading` and `rayon`, nothing else.

## Inputs
- The `Network` (`openmobisim-graph`): per link its upstream and downstream node, length,
  free-flow time per mode and modes allowed; per node its position in metres.
- The link times, `LinkTimes` (`openmobisim-loading`), the first-in-first-out reading of a
  loading on that network; an empty loading reads free flow everywhere.
- A `Query` per route wanted: origin node, destination node, mode (car, bike or walk) and
  departure (a whole second of the clock).

## Outputs
- `Search::new(&network, &times)` is the search with its scratch, reusable across queries.
  `Search::fastest(&query)` returns `Option<Route>`: `None` when the destination cannot be
  reached on links the mode may use; the empty route arriving at the departure when the origin
  is the destination; otherwise the route of earliest arrival.
- `Route { links, arrival }`: the links in the order travelled, and the arrival at the
  destination in seconds since the window's start.
- `fastest_many(&network, &times, &queries)` answers every query in parallel (rayon, one
  scratch per thread); answer `i` is query `i`'s, the same bit for bit at any thread count.

## Units
Seconds and metres. The departure is a whole second of the clock (`Second`); the arrival is
`f64` seconds since the window's start (a `Duration`), never floored, on the clock of
`LinkTimes::time`. The search's bound is in seconds per metre of straight line.

## Meanings and assumptions
- A route's time is walked link by link: each link entered when the one before it ends, its
  time `LinkTimes::time(link, mode, t)` at that entry `t`. Nothing else costs time: no turn,
  no node delay, no waiting at a node (on first-in-first-out times waiting never helps). The
  arrival is `LinkTimes::route_time` of the route added to the departure, to the bit.
- Fastest means earliest arrival. The search is label-setting Dijkstra over nodes on the
  arrival, exact on first-in-first-out link times (Kaufman and Smith 1993), which the reading
  is by construction. It is goal-directed (A\*, Hart, Nilsson and Raphael 1968) by a lower bound
  on the rest of the way: the straight-line distance to the destination times the least, over
  the links the mode may use, of free-flow time over the straight-line distance between the
  link's ends (links whose ends coincide skipped), shrunk by one part in a million against
  rounding. It holds because no link time is below free flow (checked in debug builds); it
  changes how much is explored, never the route.
- Ties: two routes reaching a node at exactly the same instant are decided by the shorter in
  metres, then fewer links, then the lower id of the link arriving at the node.
- Links that forbid the mode are never taken.

## Errors
Nothing is refused by a return value: an unreachable destination is `None`. `fastest` panics,
as the reading does, for transit (it runs on its timetable, not on links), and for a node the
network does not have. In debug builds it panics if a link time read is below the link's
free-flow time (the reading would be broken).

## Bikes
A bike and a walker cross each link in its length over their mode's speed (the graph's
`bike_speed` and `walk_speed`), at every instant, whatever the cars do: their fastest route is
their shortest by length, from any departure, and its arrival is the departure plus the
route's length over the speed. A bike rides every link its mode set allows, a cycle path
included; a street with a car queue costs it nothing more. Test 9 checks these routes against
networkx's shortest paths by length.

## Tests
`cargo test -p openmobisim-search`. Each expected value comes from outside the crate: test 9,
bike, walk and free-flow car routes equal to networkx's shortest paths by length on a jittered
12 × 12 grid (`tests/networkx_routes.py` writes the fixture; networkx 3.7; arrivals within
1e-9 of length over speed); a free-flow car detour by hand on a 5 × 5 grid with two slow links
(43.2 s against 50.4 s, the route among four of equal time fixed by the tie rule by hand, the
bike direct in 96 s); the switch between a queued short link and a free long route by Vickrey's
bottleneck (72 + t/2 against 144 s: short at departures 0 and 100, long at 300, arrivals 72,
222 and 444 s by hand and equal to the reading's walk to the bit); an empty route at the
origin, a link forbidding cars, transit refused; a brute force on five fully linked nodes with
queues (200 queries, the arrival equal to the best of every simple path walked on the reading);
determinism over 1 and 8 threads and two runs (2 000 queries, 20 × 20 loaded grid). The run time
on the 100 × 100 grid is reported by hand: `cargo test --release -p openmobisim-search --test
run_time -- --ignored --nocapture`.
