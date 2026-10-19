# Route sets

% src: crates/core-routes/src/{lib.rs,generate.rs,search.rs,store.rs,attributes.rs,snap.rs}; crates/core-sim/src/{route_update.rs,route_choice.rs (Chooser::fill),route_cache.rs,run.rs (snapping, DEFAULT_CHOICE_DETOUR_LIMIT, DEFAULT_CHOICE_MAX_ROUTES)}; python/openmobisim/scenario.py (Scenario.__init__ route_method and route_update defaults; from_parts route_method, route_options, choice_detour_limit, choice_max_routes, route_update_options, route_cache)

A driver choosing a route does not weigh every path through a city: they weigh a few. A **route set** is those few
for one **origin-destination pair**: the routes a car trip between two places may take. The [choice model](choice.md)
picks one route from it for each traveller; the [loading](loading.md) drives it. This page explains how the sets are
made, how they grow during a run, and which of their routes a traveller is offered.

Route sets are for **car trips**, and for the car legs of park-and-ride. Bike and walk trips take one route on their
own layer (see *For bikes* below); transit journeys are found in the timetable ([Transit](transit.md)).

## From a trip to a pair

% src: crates/core-sim/src/run.rs (NodeSnapper::nearest for each trip's origin and destination); crates/core-routes/src/snap.rs

Each trip's origin and destination are **snapped** to the nearest **street** node: a node with a road a car can enter
from, which a motorway, a trunk road or one of their ramps is not (ties broken by node id). A node that has only such
roads is taken only when the point lies closer to it than `car_snap_motorway_m` (`mode_options`, 50 m): a through trip
that a count places on the motorway keeps its node, an address beside the motorway does not start on the carriageway.
Trips whose ends snap to the same two nodes share one pair and one route set; a trip whose two ends snap to the same
node has nothing to drive and completes at once.

## How routes are searched

% src: crates/core-routes/src/search.rs module docs (link-based search, costs, A*, bounded search, determinism)

Every method is built on one search:

- It runs over **links**, following the turns the network allows, so a U-turn away from a dead end is never taken
  ([Networks](networks.md)).
- Its **cost** is the free-flow time: each link's length over its free-flow speed, plus the control delay of a signal
  at its end. Only links a car may use count.
- It is **goal-directed** (A\*): it expands routes in order of their cost so far plus a lower bound on the rest (the
  straight-line distance to the destination times the least cost per metre any link has). The bound never
  overestimates, so the route found is a least-cost one, as with Dijkstra's algorithm; only less of the network is
  explored.
- Ties between equal routes are broken by link id, so a search never depends on the order of its inputs.

A route's **cost** in a set is its free-flow time, never a penalised one, whatever the method did to find it.

## The methods

% src: crates/core-routes/src/generate.rs Penalty (Default, generate), MonteCarlo (Default, doc), congestion_propensity, PROPENSITY_BIN_SECONDS, Shortest, sort_best_first; search.rs max_overlap, set_noise

`route_method=` picks the method; `openmobisim.route_methods()` lists them.

### `penalty`

The shortest route, then repeatedly the shortest route after **multiplying the cost of the links of the route just
found** by `penalty` (1.5), so that each search is pushed off the routes already found. A candidate is kept if:

- it is new;
- its free-flow time is at most `max_detour` (1.3) times the shortest route's;
- it shares at most `max_overlap` (0.75) of its cost with any one route kept before.

It stops at `max_paths` (5) routes or after `max_attempts` (15) searches. Each search is **bounded**: a partial route
whose true (unpenalised) cost already exceeds the detour limit is dropped. The overlap of a route $r$ with an earlier
route $k$ is the share of $r$'s cost on links they share,

$$
\text{overlap}_r = \max_k \frac{\sum_{\ell \in r \cap k} c_\ell}{\sum_{\ell \in r} c_\ell}
$$

with $c_\ell$ the link's free-flow time. The method finds distinct, plausible alternatives; it does not promise the
best $k$ routes.

### `shortest`

One route per pair: the shortest. No options.

### `montecarlo`

The shortest route, then, for each of `draws` (16) draws, the shortest route when every link's cost is multiplied by

$$
1 + \sigma\, u_\ell\, b_\ell
$$

where $u_\ell$ is a uniform random number for the link and draw, $\sigma$ is `sigma` (4) and $b_\ell$ is the link's
**propensity to congest**, between 0 and 1. A route is kept if it is new, costs at most `max_detour` (2) times the
shortest, and shares at most `max_overlap` (0.9) with each route kept before, up to `max_paths` (10).

The propensity is computed from the run's own trips: every trip is put on its pair's shortest route, and for each
link the people leaving in the busiest half-hour whose route uses it are divided by what the link carries in half an
hour; it is clipped at 1. So the random costs rise where the demand is likely to jam, and the routes found go round
it. `biased` 0 removes the bias (every link perturbed alike); `seed` changes every draw. The draws are a function of
the seed, the pair, the draw and the link alone.

### Options

| Method | Option | Default | Meaning |
|---|---|---|---|
| `penalty` | `max_paths` | 5 | the most routes kept |
| `penalty` | `max_detour` | 1.3 | the longest route kept, as a multiple of the shortest's free-flow time |
| `penalty` | `max_overlap` | 0.75 | the largest share of its cost a route may share with an earlier one |
| `penalty` | `penalty` | 1.5 | the factor on the links of each route found |
| `penalty` | `max_attempts` | 15 | the most searches per pair |
| `montecarlo` | `draws` | 16 | random cost draws per pair |
| `montecarlo` | `sigma` | 4 | how far a link's cost may rise, times its propensity |
| `montecarlo` | `max_detour` | 2 | as for `penalty` |
| `montecarlo` | `max_overlap` | 0.9 | as for `penalty` |
| `montecarlo` | `max_paths` | 10 | as for `penalty` |
| `montecarlo` | `seed` | 0 | which draws |
| `montecarlo` | `biased` | 1 | 1 biases the draws towards links likely to congest; 0 does not |

Given with `route_options=`; unknown names and values out of range are refused. Every value is uncalibrated.

### Which method a run uses

% src: python/openmobisim/scenario.py Scenario.__init__ (reloads, route_method, route_update defaults)

| The run | `route_method` by default | `route_update` by default |
|---|---|---|
| loads more than once: `"msa"` with more than one iteration, or a free-flow loading built in groups (the default at `flow_level` 2 to 4) | `"shortest"` | `"best_response"` |
| loads once | `"penalty"` | `"none"` |

So a default run starts from one route per pair and lets the route update find the others on congested times. Naming
a method, or giving `route_options` (which then mean `"penalty"`), overrides this.

## How sets grow: the route update

% src: crates/core-sim/src/route_update.rs (BestResponse::update, update_by_trees, DEFAULT_MAX_ROUTES, DEFAULT_SLACK, REPLACE_GAIN_S, DEFAULT_TREE_STEP_S); run.rs (grow_sets; route update during the free-flow loading's groups and after each iteration)

The sets are made once, on free-flow times. `route_update="best_response"` grows them during the run: after each
loading (and after each group's loading while the free-flow loading is built up), for every pair it searches the
**fastest route at the times that loading produced**, leaving at the median departure of the pair's trips (or at
`searches` spread quantiles of them), and adds it to the set if it is new and no slower than the set's best route at
that departure. The set is fixed while travellers choose, and grows between choices.

| Option | Default | Meaning |
|---|---|---|
| `max_routes` | 10 | the most routes a set holds. A full set is still searched: a route at least one second faster than its best replaces a route nobody chose in the last choice, never the set's free-flow shortest; if every route is in use, nothing changes |
| `searches` | 1 | searches per pair per update, at evenly spread quantiles of the pair's departures |
| `slack` | 0.02 | a pair is not searched while its best route is within this share of its free-flow time: nothing is faster than free flow, so at most this much is left to gain |
| `tree_step_s` | 0 | above 0, departures are rounded to this step and the pairs of one origin at one rounded departure share one search tree (faster, approximate) |

Given with `route_update_options=`. `"none"` leaves the sets as the method made them. Searching each pair's fastest
route at congested times is the time-dependent search above, on link times read from the loading
([Equilibration](equilibration.md)).

## What a traveller is offered

% src: crates/core-sim/src/route_choice.rs Chooser::fill; run.rs DEFAULT_CHOICE_DETOUR_LIMIT, DEFAULT_CHOICE_MAX_ROUTES

A set holds every route found; a traveller is offered only some of them, at their departure:

1. Each route's **expected time** is computed: its free-flow time at the first choice, then the time the last loading
   gives it for a traveller leaving at the trip's departure.
2. Routes slower than the best by more than `choice_detour_limit` (0.5: half as slow again) are not offered.
3. Of the rest, at most `choice_max_routes` (5) are offered, the fastest.

The best route is always offered. A route not offered now stays in the set and can be offered later, when times
change. `choice_detour_limit=0` and `choice_max_routes=0` lift the limits. With mode choice the limits apply within
each mode. How the offered routes are weighed, including their **path size** (how much of a route is its own), is on
the [Choice](choice.md) page.

## The route cache

% src: crates/core-sim/src/route_cache.rs; python/openmobisim/scenario.py from_parts route_cache

Generating sets is the largest single cost of a run that iterates. With `Scenario(route_cache=True)`, the sets a run
generates are kept in memory for the next run **of the same Python process** that asks for the same: the same network
(its whole content), the same pairs, the same method and options and, for `montecarlo`, the same trips. What is kept
is the set as the method made it, before any route update; a hit is the set a miss would have made, so no result
depends on the cache. It holds at most four sets, the least recently used dropped first;
`openmobisim.route_cache_clear()` empties it and `openmobisim.route_cache_info()` returns `(hits, misses, held)`.

## Reading the sets

{py:meth}`openmobisim.Run.route_sets` returns the sets a run used, the routes added by the update included, and
{py:meth}`openmobisim.Run.route_choices` which route each trip took. `openmobisim.route_sets_build` makes sets for
pairs given as coordinates, outside a run.

## For bikes

% src: crates/core-graph/src/layers.rs (BikeCost; "Routes are shortest paths under one of the two BikeCosts"); crates/core-routes/src/search.rs (SearchContext::with_costs)

A bike trip has **no route set**: it takes the single least-cost route on the bike layer, under the cost
`Scenario(bike_cost=…)` sets ([Networks](networks.md), *For bikes*), and so does a walk on the walk layer. These costs
do not change during a run, so the route is found once. A cyclist therefore does not choose among routes, and the
route update does not apply to bikes. Bikes do compete with other modes when a trip chooses its mode
([Modes and itineraries](modes.md)).

## Limits

- Sets are made on free-flow times; congestion enters them only through the route update.
- Neither generation method promises the best routes or all the plausible ones.
- The route update searches at one departure per pair by default (the median), so a route that pays only at some
  hours may be missed.
- A trip is snapped to the nearest street node (a motorway-only node only within 50 m), whatever lies between.
- Bike and walk trips have one route each; bike route choice is not stochastic.

## References

The penalty method goes back to de la Barra, Pérez and Añez {footcite}`delabarra1993`; it and the Monte Carlo
method belong to the families of choice-set generation compared in {footcite}`bekhor2006` and reviewed in
{footcite}`bovy2007`; a choice set used in estimation as a sample of all routes is discussed in
{footcite}`frejinger2009`.

```{footbibliography}
```
