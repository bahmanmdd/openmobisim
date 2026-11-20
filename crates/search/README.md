# openmobisim-search

The fastest route per mode at the current link times: the route of earliest arrival from one
node to another, setting out at a given second, each link's time read from the last loading
at the instant the route enters it; and its tree form, every node within a bound of one node. Depends on `openmobisim-types`, `openmobisim-graph`,
`openmobisim-loading` and `rayon`, nothing else.

## Inputs
- The `Network` (`openmobisim-graph`): per link its upstream and downstream node, length,
  free-flow time per mode and modes allowed; per node the links leaving it and entering it.
- The link times, `LinkTimes` (`openmobisim-loading`), the reading of a loading on that
  network; an empty loading reads free flow everywhere.
- A `Query` per route wanted: origin node, destination node, mode (car, bike or walk),
  departure (seconds since the window's start: any instant, not only a whole second) and the
  traveller's trip, or none for a traveller the loading did not have.

## Outputs
- `Search::new(&network, &times)` is the search with its scratch, reusable across queries.
  `Search::fastest(&query)` returns `Option<Route>`: `None` when the destination cannot be
  reached on links the mode may use; the empty route arriving at the departure when the origin
  is the destination; otherwise the route of earliest arrival.
- `Route { links, arrival }`: the links in the order travelled, and the arrival at the
  destination in seconds since the window's start.
- `Search::reach(origin, mode, departure, bound)` returns `Vec<(NodeId, Duration)>`: every
  node the mode reaches from the origin within `bound` seconds of the departure, with its
  earliest arrival in seconds since the window's start (as `Route::arrival`), sorted by node id;
  the origin is among them, at the departure.
- `Search::reach_backward(destination, mode, bound)`, bike and walk only: every node from
  which the mode reaches the destination within `bound` seconds, with that time in seconds,
  sorted by node id; the destination is among them, at 0.
- `fastest_many(&network, &times, &queries)` answers every query in parallel (rayon, one
  search per thread); answer `i` is query `i`'s, the same bit for bit at any thread count. The
  queries are taken in the order of mode and destination, so that a thread keeps one bound (below)
  for the queries to one destination. The routes are `fastest`'s, query by query.

## Units
Seconds and metres. The departure and the arrival are `f64` seconds since the window's start
(a `Duration`), never rounded, on the clock of `LinkTimes::exit`.

## Meanings and assumptions
- A route's time is walked link by link: each link entered when the one before it ends, its
  exit `LinkTimes::exit(link, mode, t, reader)` at that entry `t` for the query's traveller,
  the reader carried from link to link (the recorded car whose place the traveller took).
  Nothing else costs time: no turn, no node delay, no waiting at a node (on first-in-first-out
  times waiting never helps). The arrival is `LinkTimes::route_time` of the route added to the
  departure, to the bit. A traveller's own car in the last loading is not in its way.
- Fastest means earliest arrival. The search is label-setting Dijkstra over nodes on the
  arrival, exact on first-in-first-out link times (Kaufman and Smith 1993), which the reading
  is by construction. It is goal-directed (A\*, Hart, Nilsson and Raphael 1968) by a lower bound
  on the rest of the way: the mode's free-flow time from the node to the destination, shrunk by
  one part in a million against rounding. It holds because no link time is below free flow
  (checked in debug builds); it changes how much is explored, never the route. The bound is a
  search backward from the destination on free-flow times, run on only as far as the queries
  to that destination ask (a time is final once no time still queued is below it): the nodes
  their routes come near, not the whole network. A `Search` keeps the bound of the destination
  and mode it last served.
- The tree form is the same label-setting over the same scratch and tie rule, without the
  goal (plain Dijkstra), stopped when the least label is more than the bound after the
  departure: a node is in when its arrival less the departure is at most the bound. `reach`'s
  arrival at a node is the one `fastest` finds to it. `reach_backward` is the bound's search
  run to a limit, over the links entering each node on free-flow times, which is exact for a
  bike and a walker only (their
  times do not depend on the instant); a car's backward search would need its arrival.
- Ties: two routes reaching a node at exactly the same instant are decided by the shorter in
  metres, then fewer links, then the lower id of the link arriving at the node.
- Links that forbid the mode are never taken.

## Errors
Nothing is refused by a return value: an unreachable destination is `None`. `fastest` and
`reach` panic, as the reading does, for transit (it runs on its timetable, not on links), and
for a node the network does not have; `reach_backward` panics for car and transit. In debug builds it panics if a link time read is below the link's
free-flow time (the reading would be broken).

## Bikes
A bike and a walker cross each link in its length over their mode's speed (the graph's
`bike_speed` and `walk_speed`), at every instant, whatever the cars do: their fastest route is
their shortest by length, from any departure, and its arrival is the departure plus the
route's length over the speed. A bike rides every link its mode set allows, a cycle path
included; a street with a car queue costs it nothing more. Test 9 checks these routes against
networkx's shortest paths by length. A bike's `reach` reads no queue either: its tree is its
tree at free flow, judged against networkx's bounded Dijkstra (`tests/reach.rs`); the trees on
loaded times (`tests/brute_force.rs`) are by car. A bike's or a walker's `reach_backward` is the time to a
node from every node around it, on the links its mode may use in their own direction: a
one-way cycle path is ridden one way only, backward as forward.

## Tests
`cargo test -p openmobisim-search`. Each expected value comes from outside the crate: test 9,
bike, walk and free-flow car routes equal to networkx's shortest paths by length on a jittered
12 × 12 grid (`tests/networkx_routes.py` writes the fixture; networkx 3.7; arrivals within
1e-9 of length over speed); the tree form against networkx's `single_source_dijkstra` with a
cutoff on a jittered 10 × 10 grid with 30 links closed to walkers (`networkx_routes.py reach`
writes the fixture: 8 origins, forward on foot within 450 m and by bike within 600 m, and on
foot on the reversed graph; the same nodes, their times within 1e-9 of length over speed;
forward and backward differ for at least 3 origins); `reach` by car on the five loaded nodes
below against every simple path (2 000 trees within 120.02 s, which cuts nodes off, and within
300.02 s, which reaches over best paths of two links or more: a node is reached exactly when
the best path arrives within the bound, at that arrival); the bound by hand (a car at
50 km/h crosses 100 m in 7.2 s: within 7.2 s a node and its four neighbours, within 7.1 s the
node alone); car backward and transit refused; a free-flow car detour by hand on a 5 × 5 grid with two slow links
(43.2 s against 50.4 s, the route among four of equal time fixed by the tie rule by hand, the
bike direct in 96 s); two routes of two links arriving at the same instant by car, bike and on
foot (21.6, 72 and 225 s by hand), the one whose last link has the lower id taken whichever the
search finds first; the switch between a queued short link and a free long route by Vickrey's
bottleneck (72 + t/2 against 144 s: short at departures 0 and 100, long at 300, arrivals 72,
222 and 444 s by hand and equal to the reading's walk to the bit); an empty route at the
origin, a link forbidding cars, transit refused; a brute force on five fully linked nodes with
queues (200 queries, the arrival equal to the best of every simple path walked on the reading).
`fastest_many` answers six queries of mixed modes and destinations each in its place, by the
same hand derivations, as a batch and each as a batch of one, and no query with no answer; its
determinism over 1 and 8 threads and two runs is tested with the whole chain, by car, bike and
walk on loaded readings (`crates/loop/tests/determinism.rs`). The run time
on the 100 × 100 grid is reported by hand: `cargo test --release -p openmobisim-search --test
run_time -- --ignored --nocapture`.
