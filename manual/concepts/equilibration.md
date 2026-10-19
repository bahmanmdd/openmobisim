# Equilibration

% src: crates/core-sim/src/equilibration.rs (IterationReport, gap_verdict, Equilibration, FreeFlow, Msa); crates/core-sim/src/run.rs (the free-flow loading's increments, the iteration loop); crates/core-sim/src/route_choice.rs Chooser::update; python/openmobisim/scenario.py (Scenario.__init__, from_parts equilibration and equilibration_options, Run.convergence, convergence_gap, convergence_verdict, converged)

Travellers choose their routes on the times they expect, and those times depend on everyone's choices. **Equilibration**
repeats choice and loading until the two agree: load the network, read the times it produced, let some travellers
choose again on those times, load again. A run that ends with the choices consistent with the times they produced is
near an **equilibrium**: the state in which, given the choice model, nobody gains by choosing differently. This page
describes the two strategies the code has, how each iteration runs, and how a run measures how far from equilibrium
it ended.

## The two strategies

% src: python/openmobisim/scenario.py Scenario(equilibration="msa"); crates/core-sim/src/equilibration.rs Registry::builtin

| `equilibration` | What it does |
|---|---|
| `"msa"` (the default) | the free-flow loading, then iterations of the **method of successive averages** in traveller form, until converged or `iterations` (10) loadings |
| `"free_flow"` | the free-flow loading alone: a quick estimate of the morning without iterating |

`openmobisim.equilibration_strategies()` lists them.

## Iteration 0: the free-flow loading

% src: crates/core-sim/src/run.rs (increments, group, first_group, partial loadings); equilibration.rs Equilibration::increments, warmup_iterations; python/openmobisim/scenario.py FREE_FLOW_INCREMENTS, Scenario.__init__ (increments, warmup defaults)

Every run starts with the **free-flow loading** (its iteration 0), which is all `"free_flow"` does. It is built up
in groups, an incremental assignment {footcite}`sheffi1985`:

1. Each traveller is put in one of `increments` groups (5 by default) of about equal size, by a random draw keyed to
   the traveller.
2. The first group chooses on free-flow times.
3. The network is loaded with the groups that have chosen. The route update (below) searches the next group's pairs
   on the times that loading produced, and the next group chooses on those times.
4. Step 3 repeats until every group has chosen; then everyone is loaded together. That loading is iteration 0's.

So each group sees the congestion of those before it, and routes spread around congestion before the first loading of
everyone. With `warmup` 1 (the default), all of these loadings use the point queue ([The loading](loading.md)), in
which nothing spills back and nothing locks, whatever `flow_level` says. At `flow_level=0` there are no groups, since
nothing congests.

## Iterations 1 onwards: the method of successive averages

% src: crates/core-sim/src/equilibration.rs Msa::reselects; run.rs iteration loop (load_once, route_update.update, chooser.update, itin.choose, changes applied only if not stopping)

Each iteration $i \geq 1$ of `"msa"`:

1. **Reads the times** of the last loading: for each link and each time bin of `cost_bin_s` (300 s), the mean time of
   the traversals that **entered** the link in that bin. A link nobody entered in a bin reads the last recorded bin's
   time less the time elapsed since it, down to its free-flow time (under first-in-first-out, nobody entering now
   leaves before those already on the link, so a standing queue nothing could enter is not read as an empty link); a
   link nobody entered at all reads its free-flow time; a vehicle still on a link when the window ended counts with
   the time it had taken so far, at least the link's free-flow time. A route's time for a traveller leaving at $t$ is found link by link, each entered when the last
   one is left.
2. **Grows the route sets** with the route update, if one is active (below).
3. **Measures the gaps** of the loading just made (below).
4. **Lets some travellers choose again** on those times: traveller $n$ chooses again before iteration $i$ when a
   random number keyed to $(n, i)$ falls below

   $$
   \frac{1}{i + 1}
   $$

   so half of them before iteration 1, a third before iteration 2, and so on. The others keep their choice. This is
   the method of successive averages {footcite}`sheffi1985` applied to travellers rather than to flows, as in
simulation-based dynamic assignment {footcite}`sbayti2007`: the share
   that moves shrinks, so the pattern settles. Trips with itineraries (transit, park-and-ride, bike-and-ride, and
   every trip choosing its mode) choose again the same way.
5. **Loads the network** at `flow_level`, unless the iteration is one of the first `warmup` loadings.

The run stops after `iterations` loadings, or earlier when converged. The choices made after the last loading are not
applied: the run's result is its last loading, with the choices that produced it.

## Measuring how far from equilibrium

% src: crates/core-sim/src/equilibration.rs IterationReport (gap, gap_expected, gap_excess, incomplete_share, gap_network, gap_network_excess, gap_flow*, itinerary_gap, itinerary_gap_excess), disequilibrium, GAP_GOOD, GAP_ACCEPTABLE; python/openmobisim/scenario.py Run.convergence_gap, convergence_verdict

The **gap** of a loading compares what travellers paid with the best they could have had, over the trips that finished
inside the window, each weighted by the people it stands for ($w$):

$$
\text{gap} = \frac{\sum_n w_n\,(t^{\text{chosen}}_n - t^{\text{least}}_n)}{\sum_n w_n\, t^{\text{least}}_n}
$$

where $t^{\text{chosen}}_n$ is the time of the route traveller $n$ took and $t^{\text{least}}_n$ the least time among
the routes of their choice set, both at the times this loading produced and the traveller's departure: the relative
gap of traffic assignment {footcite}`boyce2004`, taken over travellers and their departure times. A random
choice model such as the logit sends some travellers down slower routes **by design**, so its gap is never 0 even at
equilibrium. Hence the **expected gap**, the same sum with each traveller's *expected* time under the model's
probabilities in place of the time of the route drawn, and the **disequilibrium**, the part the model does not
explain:

$$
g^{\text{excess}} = g - g^{\text{expected}}
$$

where $g$ is the gap above (`gap`), $g^{\text{expected}}$ the expected gap (`gap_expected`) and $g^{\text{excess}}$ the
disequilibrium (`gap_excess`).

For an all-or-nothing model (`"deterministic"`) the expected gap is 0 and the two are equal.

| Measure | Against | When | Meaning |
|---|---|---|---|
| `gap` | the least time in the traveller's choice set | every iteration | the usual relative gap of traffic assignment |
| `gap_expected` | the same | every iteration | the gap a converged run has by design |
| `gap_excess` | the same | every iteration | the disequilibrium of the car routes |
| `gap_network` | the least time over **any** route (a time-dependent search) | the last iteration, on a sample of `gap_sample` (300) finished trips | whether the choice sets missed a route that traffic made worthwhile |
| `gap_network_excess` | the same | the last iteration | `gap_network` less what the model explains inside the set |
| `gap_transit`, `gap_car_transit`, `gap_bike_transit`, `gap_car`, `gap_bike`, `gap_walk` | the best itinerary of the mode each trip kept | every iteration | the same gap for itineraries and for trips choosing their mode |
| `itinerary_gap_excess` | the same | every iteration | the itineraries' disequilibrium |
| `gap_flow`, `gap_flow_floor`, `gap_flow_excess` | the model's own probabilities | every iteration | the share of travellers not where the probabilities put them, what chance alone gives, and the difference: a check of the model with itself, not a gap |
| `incomplete_share` | — | every iteration | the share of travellers left out of the gaps because their trip had not finished |

`Run.convergence()` gives each of these per iteration, with what moved: `reselected_share` (who chose again),
`changed_share` (whose route changed), `time_change` (how much the link times moved, weighted by traffic) and, with mode
choice, `mode_changed_share` against its chance floor `mode_changed_floor`.

**How a run is judged.** {py:attr}`openmobisim.Run.convergence_gap` is the worst of the last iteration's
`gap_excess`, `gap_network_excess` and `itinerary_gap_excess` (the plain gap where the model gives no
probabilities). {py:attr}`openmobisim.Run.convergence_verdict` reads it:

| Verdict | Disequilibrium |
|---|---|
| good | below 0.05 |
| acceptable | below 0.15 |
| poor | 0.15 or more |

## When `"msa"` stops

% src: crates/core-sim/src/equilibration.rs Msa::is_converged; python/openmobisim/scenario.py GAP_TOLERANCE, ROUTE_GROWTH_TOLERANCE

`"msa"` stops early, before `iterations`, when all three hold:

1. at least four loadings have been made (iteration 0, everyone's first choice, never counts);
2. the last route update added routes for at most `route_growth_tolerance` (1) of the pairs it searched;
3. the disequilibrium, averaged over the last three iterations, is below `gap_tolerance` (0.05). Here an iteration's
   disequilibrium is the worse of `gap_excess` and `itinerary_gap_excess`.

The mean of three, not the last value, because sampled choice makes the gap noisy. The stopping rule does not read
`gap_network`, which is measured only at the end: so a run can stop as converged ({py:attr}`openmobisim.Run.converged`)
and still be judged acceptable or poor by `convergence_verdict`, when its choice sets missed routes. A
`gap_tolerance` of 0 never stops early.

## The route update between iterations

% src: crates/core-sim/src/route_update.rs (BestResponse, DEFAULT_MAX_ROUTES, DEFAULT_SLACK, REPLACE_GAIN_S); python/openmobisim/scenario.py Scenario.__init__ (route_update default)

Route sets are made once, on free-flow times. Traffic can make worthwhile a route that no set holds, and an
equilibrium over fixed sets then stops short of the network's own (the `gap_network` shows it). The **route update**
`"best_response"`, the default for any run that loads more than once, closes that: after each loading (and after each
group's loading in iteration 0) it searches each origin-destination pair for its fastest route at the times that
loading produced, and adds it to the pair's set if it is new and at least as fast as the set's best. So **the
equilibrium is over the sets as the route method and the update made them**. The sets grow *before* the iteration's
gap is measured, so the gap is against the set as grown: a route found this iteration counts as the least-cost
alternative although nobody could choose it yet, and the gap rises when routes are found. That is why the stopping
rule waits for the growth to end. Details are in [Route sets](route_sets.md).

## Disruptions travellers did not know of

% src: crates/core-sim/src/run.rs (disruptions not known: one more loading)

With `disruptions=` and `disruptions_known=False` (the default), the run equilibrates without the disruption and then
loads the morning once more with it, every choice as the iterations left it: only what happens within the day
(rerouting, passengers whose run never comes) reacts. That last loading is the run's result: the completion,
the travel times, the mode totals, the link bins and the skims are the disrupted day's, while
{py:meth}`openmobisim.Run.convergence` (the iteration table) describes the equilibrium reached before it, so its last
row and {py:attr}`openmobisim.Run.completion` differ. See [Scenarios and edits](../guides/scenarios.md).

## Defaults and options

% src: crates/core-sim/src/equilibration.rs Msa::default, FreeFlow::default; python/openmobisim/scenario.py from_parts equilibration_options (passed as given)

`equilibration_options` sets, by name:

| Option | Strategy | Default | Meaning |
|---|---|---|---|
| `iterations` | `msa` | 10 | the most loadings |
| `gap_tolerance` | `msa` | 0.05 | stop when the mean disequilibrium of the last three iterations is below it; 0 never stops early |
| `route_growth_tolerance` | `msa` | 1 | … and the last route update added routes for at most this share of the pairs it searched |
| `gap_sample` | `msa` | 300 | trips tested against the whole network at the last iteration; 0 for none |
| `itinerary_gap_sample` | `msa` | every trip | itinerary trips, besides those choosing again, that plan their whole choice set to measure the gap; with fewer, the gap before the last iteration is an estimate |
| `cost_bin_s` | `msa` | 300 s | the time bins the link times are read in |
| `increments` | both | 5 (the run uses 1 at `flow_level` 0, where nothing congests) | groups of the free-flow loading |
| `warmup` | both | 1 | how many of the first loadings (the groups included) use the point queue; the last loading of an iterating run never does |

These are the core's defaults, one table for a run from Python and a run of the Rust core alike: the
[parameters page](../reference/parameters.md) lists them, with a note where a default is a rule of the run's level.
Every value is uncalibrated.

## For bikes

Cyclists and pedestrians on their own layers do not congest, so their routes do not change from one iteration to the
next: a cycling trip's route is found once. Where bikes take part in equilibration is through **mode choice**: a trip
choosing its mode chooses again like any other, weighing a ride, whose time is fixed, against modes whose times the
iterations change; `gap_bike` and `mode_changed_share` say how that settled. Bike-and-ride itineraries are measured by
`gap_bike_transit`.

## Limits

- Only two strategies exist; others (re-planning a share of travellers on their whole day, for instance) are not built.
- The gaps leave out trips still under way when the window ended; `incomplete_share` says how many.
- `gap_network` is measured on a sample, at the last iteration only.
- At `flow_level=0` nothing interacts, so `"msa"` has nothing to settle and stops at the choice model's own floor.
- Iterations are not guaranteed to converge: under heavy congestion at level 4, where queues spill back, they can drift
  ([The loading](loading.md), Limits).
- `time_change` is a sum over (link, bin) cells of the change in mean time, so with traveller weights above 1 a few
  vehicles crossing a bin boundary move it: on the Lyon starter case (weight 5) it stays near 0.3 while completion,
  the mean trip and the gap are stable. It is reported, not read by the verdict; read `gap_excess` and
  `changed_share` for whether a run settled.

## References

```{footbibliography}
```
