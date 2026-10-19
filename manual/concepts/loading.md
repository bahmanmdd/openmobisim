# The loading

% src: crates/core-loading/src/{lib.rs,ltm.rs,level0.rs,curves.rs,link_bins.rs,vehicle.rs}; crates/core-sim/src/{run.rs,loading_rules.rs,reroute.rs}; python/openmobisim/scenario.py (Scenario.__init__, from_parts flow_level, flow_step_s, loading_options; Run.report_spillback, Run.report_gridlock)

The **loading** (dynamic network loading, in the literature) moves every car and bus along its route through the
morning and works out when each one reaches each link: how long queues grow, where traffic slows, who arrives
when. It moves **individual vehicles**, each carrying its own route, and holds every link to the two conditions of the
**link transmission model** {footcite}`yperman2007`, which applies to networks Newell's simplified theory of
kinematic waves: counts of the vehicles that have passed each point {footcite}`newell1993` and a triangular fundamental
diagram {footcite}`newell1993b`. It is computed **event by event in continuous time**: every vehicle enters and leaves every link at its exact moment, and there is no time step. The 300-second
numbers among the settings are the bins of the outputs and of the costs the next iteration chooses on, never of the
traffic. Where each part comes from, and what is openmobisim's own, is set out in
[Where the model comes from](#where-the-model-comes-from). It never chooses a route: every vehicle arrives with one,
chosen beforehand by the [choice model](choice.md) from the trip's [route set](route_sets.md). Its result, the time each link took at each
moment, is what travellers choose on next time ([Equilibration](equilibration.md)).

Only the road network is loaded. Bikes and pedestrians move on their own layers at fixed speeds and are not part of
the loading ([Networks](networks.md)).

## The levels

% src: python/openmobisim/scenario.py FLOW_LEVELS, Scenario(flow_level=2); crates/core-loading/src/ltm.rs FidelityLevel, LtmNetwork::apply_level; level0.rs

`Scenario(flow_level=…)` chooses how much of traffic's physics the loading represents:

| `flow_level` | Name | Queues | Spillback (a queue backing up into the links upstream) | What it is for |
|---|---|---|---|---|
| 0 | free flow | none: no vehicle affects another | no | debugging; a lower bound on travel times |
| 2 (the default) | point queue | form at the end of a link, behind its capacity or that of the link ahead, and delay everyone behind them on that link | no: a queue takes no road space | planning studies; equilibrium iterations settle steadily, and nothing can gridlock |
| 3 | spatial queue | as at 2, and they fill the road | yes, and room freed at a link's end is available at its start at once | a comparison model (the classic spatial queue); on the Lyon starter case it behaves and costs as level 4 does |
| 4 | full model | as at 3, with room freed at a link's end reaching its start after the backward wave has travelled the link | yes | studies where spillback itself matters: incidents, traffic control; run with more iterations than the default and read as a range ([Limits](#limits)) |

Levels 2, 3 and 4 are one model, the **link transmission model** {footcite}`yperman2007`, run with one of its limits
lifted: at level 2 a link's storage is infinite; at level 3 the backward wave is instant. Level 1 does not exist in
the code. Level 0 is a separate, trivial loading: each vehicle crosses each link in its free-flow time, signal delays
included.

**Level 2, the point queue.** The end of every link is a **bottleneck** in Vickrey's sense {footcite}`vickrey1969`: a
point of fixed capacity where the traffic beyond it waits and leaves in order, and the queue has no length on the road.
The link transmission model adds one rule at the link's start: without spillback, a link takes in at most its capacity
(its receiving flow {footcite}`raadsen2016`). Three things follow.

- A queue caused by a narrower link ahead stands at the end of the link before it, not at the end of the narrower link.
  Along a single chain of links this changes nobody's arrival time (a vehicle's delay at a bottleneck depends on when
  it would reach it and on the capacity, not on where its queue stands {footcite}`newell1993b`); it changes which link
  the waiting is counted on, and so the link times the next choice reads.
- On the link where it stands, a queue holds back every vehicle behind it, whichever way it turns, because each link is
  first in, first out ([Junctions](#junctions)). Vickrey calls a queue that delays traffic not bound for the bottleneck
  a *triggerneck*; at level 2 this happens on that one link and never further upstream.
- Nothing gridlocks: no queue takes room that another vehicle needs.

## Where the model comes from

% src: crates/core-loading/src/ltm/mod.rs module docs; curves.rs room_at; ltm/node.rs; level0.rs

**What to call it.** Raadsen, Bliemer and Bell {footcite}`raadsen2016` sort loading models into three kinds:
microscopic; macroscopic, which move traffic as a fluid; and mesoscopic, which model vehicles individually but move
them by a fundamental diagram. openmobisim's loading is of the third kind: an **event-based, vehicle-based form of the
link transmission model**. *Event-based* because nothing is computed per time step: a vehicle's entry to and exit from
each link are events, and a link is only touched when one of its vehicles moves. Raadsen, Bliemer and Bell's own model,
eLTM, is macroscopic: it moves flows, and it showed that the link transmission model can be solved exactly without a
time step, event by event; openmobisim does the same with vehicles. Its nearest published relative is Mahut's discrete
flow model {footcite}`mahut2000`: individual vehicles in continuous time, solved event by event from the times each
vehicle crosses each node, under a simplified car-following rule whose continuous counterpart is Newell's theory; its
junctions resolve conflicts by gap acceptance, where openmobisim's use the rules in [Junctions](#junctions). The
loading is also *agent-based* in the sense the
field uses the term: every traveller is simulated as an individual (their own class,
weight, choice and vehicle), every vehicle moves as its own event, and every result is a sum over individuals; there
are no flows. It does not mean that travellers reason during the day: within the day a vehicle only re-routes when
stuck ([Remedies for gridlock](#remedies-for-gridlock-levels-3-and-4)), and the iterations ([Equilibration](equilibration.md)) are a day-to-day
learning of routes and modes. The macroscopic link transmission model is what the reference tests compare the loading
with ([Validation](validation.md)). The loading is sequential by construction: one event queue, in time order.

| Part of the loading | From the literature | openmobisim's own |
|---|---|---|
| The link conditions | the sending and receiving conditions of the link transmission model {footcite}`yperman2007`, on Newell's triangular fundamental diagram {footcite}`newell1993b` | applied to each vehicle at the moment it moves, on counts in PCU |
| No time step | eLTM {footcite}`raadsen2016`: the link transmission model solved exactly, event by event, for flows | the events are vehicles' movements; no threshold |
| Individual vehicles | the mesoscopic kind of loading model {footcite}`raadsen2016`; Mahut's event-based discrete flow model {footcite}`mahut2000` | each vehicle carries its route, so link flows always agree with route flows (eLTM turns flows at junctions by splitting rates fixed per period, which its authors note may not) |
| The point queue (level 2) | Vickrey's bottleneck {footcite}`vickrey1969` | at every link's end, with the link's capacity at its start |
| Junctions | the requirements for first-order node models of Tampère et al. {footcite}`tampere2011`, and the merge and diverge their class reduces to | no node model is solved: links are first in, first out, and a full link's room is shared in an order of service by capacity ([Junctions](#junctions)) |
| Signals | averaged over the cycle, as Tampère et al. {footcite}`tampere2011` also do | capacity times the green share, and a control delay, per approach |
| Vehicles straddling links, origin queues, give-way at roundabouts, the rule that keeps an outsider from closing a waiting loop, rerouting | none | all, each with its recorded bias ([Limits](#limits)) |
| Turn pockets (an option, on by default) | the partial first-in-first-out of Wright et al. {footcite}`wright2017` | how it is applied ([Remedies for gridlock](#remedies-for-gridlock-levels-3-and-4)) |
| Priority at merges (an option, off by default) | none | by road class ([Remedies for gridlock](#remedies-for-gridlock-levels-3-and-4)) |

## How a vehicle moves (levels 2 to 4)

% src: crates/core-loading/src/ltm.rs module docs "What it computes", admissible, heard_room; curves.rs room_at

Every vehicle carries its route, its departure time and its weight in **PCU** (passenger car units: how much road a
vehicle takes, 1 for a car, `bus_pcu` for a bus), times the number of people the simulated traveller stands for. A
link $\ell$ of length $L$ has the free-flow speed $v$, capacity $q$, jam density $k_j$ and backward wave speed $w$ of
its fundamental diagram ([Networks](networks.md)). Four rules limit a vehicle of $p$ PCU:

1. **Free-flow travel.** Its front reaches the end of the link no earlier than it entered plus $L / v$.
2. **Discharge.** Vehicle fronts leave the link's end no closer together than the saturation headway
   $p / (q\, g)$, where $g$ is the signal's green fraction on a signalised approach and 1 elsewhere.
3. **The signal's delay.** On a signalised approach a vehicle passes the stop line when the next link has room, then
   waits the signal's control delay before entering it. While it waits it holds no road space.
4. **Room on the next link.** It enters the next link only if that link has room, and no sooner than $p / q$ after
   the vehicle before it. The room on link $\ell$ at time $t$ is the link transmission model's receiving condition,
   from the counts of what has entered it, $N_{\text{up}}$, and left it, $N_{\text{dn}}$ {footcite}`newell1993`:

   $$
   \text{room}_\ell(t) = S_\ell + N_{\text{dn},\ell}\!\left(t - \frac{L}{w}\right) - N_{\text{up},\ell}(t),
   \qquad S_\ell = k_j\, L
   $$

   At level 2, $S_\ell$ is infinite; at level 3, $L / w$ is 0.

Rules 1 and 2 are the link transmission model's **sending condition**: what has left link $\ell$ by time $t$ is at most
what entered it a free-flow time earlier, and it leaves no faster than the link's capacity,

$$
N_{\text{dn},\ell}(t) \le N_{\text{up},\ell}\!\left(t - \frac{L}{v}\right);
$$

rule 4 is its **receiving condition** {footcite}`yperman2007`. Here they hold for each vehicle's front at the moment it
moves, and for the counts to within the part of one vehicle that straddles a link's end (below), rather than for flows
in time steps.

A vehicle **straddles** links: its front takes the room there is ahead and the rest of it stays counted on the link
behind, as it physically does, so a car crossing a 3-metre junction piece is counted on three pieces at once and no
link ever holds more than its storage. A vehicle that cannot move blocks those behind it in its queue: each link is
one first-in-first-out queue (unless turn pockets apply, below).

**Departures** wait outside the network, in an origin queue, until their first link has room; the wait counts in
their travel time. They are held by the link's room only, not by its inflow capacity (openmobisim's choice). How
vehicles from several links share the room of one is described in [Junctions](#junctions).

**Computed in time order.** Movements are processed from one queue of events ordered by time, so every rule is
checked at the moment it applies: the result is exact in continuous time, and a total order of events makes it
independent of the order of the input. The **loading step** (`flow_step_s`, 300 s) is only a bookkeeping boundary:
no vehicle's timing depends on it.

**Traffic stops only at jam density.** A vehicle waits only for room on the link ahead, or for the vehicle in front
to clear a link's end. So vehicles that wait on one another for ever must be waiting on links that are all full: no
vehicle is removed or forced on.

## Junctions

% src: crates/core-loading/src/ltm/mod.rs module docs ("How: in time order", "Recorded biases"); ltm/node.rs admissible; python/tests/test_ltm_reference.py

No separate node model is solved: what passes a junction follows from the link rules, applied one vehicle at a time.

- **First in, first out.** Each link is one queue. A vehicle that cannot enter the link ahead holds back every vehicle
  behind it, whichever way they turn, as in Daganzo's diverge {footcite}`daganzo1995`, and the turning shares are those
  of the vehicles' own routes. Turn pockets, an
  option, relax this ([Remedies for gridlock](#remedies-for-gridlock-levels-3-and-4)).
- **Sharing a full link.** Where several links feed one that cannot take them all, the vehicles waiting for it are
  served in proportion to the discharge capacities of the links they come from; an approach that is not held back is
  served as its vehicles arrive. The order is set by a service tag per approach, `max(V, F_prev + h)`, adapted from
  fair queueing in computer networks {footcite}`demers1989` with each approach weighted by its capacity, and assigned
  when a vehicle reaches the front: `h` is the approach's
  saturation headway, `F_prev` its last tag and `V` the largest tag already served onto the link. The tag is the
  approach's, not the movement's: it advances with every vehicle the approach releases, wherever that vehicle goes.

**Against the requirements for first-order node models** set out by Tampère et al. {footcite}`tampere2011`, on whose
node model eLTM's junctions rest: these rules apply at any junction; a vehicle moves as soon as nothing holds it, so
every flow is held by a constraint; no vehicle is created or removed; each link is first in, first out (their
conservation of turning fractions); and a full link's room is shared by the approaches' discharge capacities, not by
their demands, so an approach held by a full link passes the same flow however many vehicles queue behind it (their
invariance principle). On a single merge and a single diverge the rules give the flows of the two special cases of
their class, the merge by capacity and the first-in-first-out diverge ([Validation](validation.md)). At a general
junction, where several approaches each turn several ways, the shares differ from their node model, which shares a
full exit by *oriented capacities* (an approach's capacity times the share of its traffic bound there): in the
loading, the share an approach gets at a full exit also depends on the traffic at the other exits it feeds, and a full
exit can be left slightly under-used; a validation case for general junctions comes in a later release. Not
represented: the capacity of the junction itself (conflicts between crossing movements, their node supply
constraints), apart from a signal's green share and the give-way at roundabouts, which their node models do not
cover.

## Signals and roundabouts

% src: crates/core-loading/src/ltm.rs must_give_way (every level), closes_a_waiting_loop; module docs "Roundabouts", "Any local lock", "Recorded biases"

A signal is represented by its average effect: the discharge share $g$ and the control delay. Queues within one cycle
are not represented. Two things to know when reading a signalised approach's capacity: (i) the approach discharges at
its capacity times its green share, and the green share taken is the **largest over the approach's turns** (the
through movement's, usually), so a turn with a shorter green is served faster than it would be at a real signal: an
upper bound, documented as such for 0.1; green shares per movement (from `turn:lanes`) come later. (ii) The control
delay is a cycle average served **after** the stop line: platoons within a cycle and the queue's length on the approach
while it waits out the red are not represented.

At a **roundabout** (links tagged `junction=roundabout`), a vehicle entering the ring gives way while the
circulating link before its entry holds a vehicle bound for the same link, at every level. More generally, a vehicle
from outside a forming loop of waiting vehicles may not take the last room on a link when that would close the loop
on itself. These two rules make gridlock rarer; they do not rule it out.

## Remedies for gridlock (levels 3 and 4)

% src: crates/core-sim/src/loading_rules.rs LoadingOptions; crates/core-loading/src/ltm.rs with_priority, outranks, PRIORITY_CAPACITY_RATIO, with_pockets, with_rerouting; crates/core-sim/src/reroute.rs Rerouter; python/openmobisim/scenario.py Scenario.__init__ (reroute off at flow_level ≤ 2)

**Gridlock** is a closed loop of links whose front vehicles wait on one another, all full, so none can move. It can
only happen where queues take space (levels 3 and 4). Three rules work against it, set by `loading_options`:

| Option | Default | What it does |
|---|---|---|
| `reroute` | on at levels 3 and 4, and at 2 in a loading with road disruptions; off at 2 otherwise | **en-route rerouting**: a vehicle blocked at the front of its link for `reroute_after_s` (300 s) is offered a new route from where it is, and takes it if it is at least `reroute_min_gain` (0.1, a tenth) faster than the rest of its own; at most `reroute_max` (3) times per trip. Buses never re-route |
| `pocket_length_m` | 50 (metres per lane; 0 turns them off) | **turn pockets**: on an approach of two lanes or more, a vehicle at the end of the link may pass vehicles ahead waiting for another movement, as long as those fit in their movements' pockets and none ahead is bound for its own movement; 0 turns them off |
| `priority` | off | **priority at merges**: at an unsignalised merge a vehicle gives way while the front vehicle of a higher-priority approach (a higher road class, or the same class with at least 1.5 times the capacity) is bound for the same link, and a departure gives way to every approach |

A rerouting vehicle times the rest of its planned route on, for each link, the larger of the time the last loading
gave it and the loading's live estimate now (free-flow time, plus the time its queue takes to discharge, plus how long
its front has been blocked); then it searches the fastest route from the end of its link to its destination on the
same costs. Its realised route is recorded ({py:meth}`openmobisim.Run.route_realised`). A turn pocket is a capacity,
not a place: the pocket length is shared among a link's movements by lane share, and the link's discharge capacity
stays shared. Priority is openmobisim's own remedy for loops of links that lock; it is off because, with one queue per link, a vehicle giving way holds up everything behind it; Tampère et al. also advise
against sharing room by priority alone, as priority is obeyed less strictly in congestion {footcite}`tampere2011`.
Pockets are a simple form of the partial first-in-first-out of Wright et al. {footcite}`wright2017`, in which a queue
for one exit holds back only the part of an approach's lanes that serves it: here that part is set by the pockets'
length, not by their intervals drawn from the lane geometry.

These rules are applied at whatever level is asked for; `reroute` unsaid is on at levels 3 and 4, and at level 2
only in a loading with road disruptions, where a closed or narrowed link ahead is the one thing that can hold a
vehicle (nothing else waits for room there); `reroute: 1` turns it on regardless, `0` off. Pockets are on by
default at every level: off, one queue per link locked Sioux Falls' morning in the full model at its base demand,
and a one-lane street closed for an hour in Delft locked a fifth of its trips; at the point queue they make the
entry queues of a multi-lane approach per movement, so a vehicle waiting for a full link's inflow headway no longer
holds up those behind it bound elsewhere.

## Buses on the roads

% src: crates/core-loading/src/ltm.rs "Chained vehicles"; crates/core-sim/src/run.rs (bus_load, chains); python/openmobisim/scenario.py from_parts transit

A bus is a vehicle of `bus_pcu` PCU (2 by default) loaded among the cars, one vehicle per leg from stop to stop,
**chained**: each leg departs when the one before arrives, after the dwell time and not before its scheduled
departure. While it dwells it is off the road, so it does not block the traffic behind it. Congestion makes buses
late, and their passengers with them. See [Transit](transit.md).

## The window

% src: python/openmobisim/scenario.py from_parts window, _window; crates/core-loading/src/ltm.rs run_ltm_inner_chained (start_s, n_steps, completed.retain), LtmNetwork::starting_at; crates/core-sim/src/run.rs

The loading runs over the **simulation window**, `Scenario(window=("07:30", "09:30"))`: its clock starts at the
window's start and stops at its end. Without a window it starts at the earliest departure, rounded down to the quarter
hour, and lasts three hours. A trip departing outside the window is left out, with a `WindowWarning`. A trip still on
the road at the end is **unfinished**: counted and reported, not dropped ([Outputs](outputs.md)). Leave time for the
last departures to arrive inside the window.

## What the loading records

% src: crates/core-loading/src/link_bins.rs; ltm.rs LtmOutput, LockReport, record_unfinished; crates/core-sim/src/run.rs; python/openmobisim/scenario.py Run.link_bins, report_spillback, report_gridlock

| Record | What it holds | Read with |
|---|---|---|
| trips | each trip's departure and arrival, or that it was unfinished | `kpis.parquet`, `events.parquet` |
| per-link bins | for each link and time bin, the traversals that **finished** in it, their PCU and their PCU-weighted time, so flow and mean time follow | {py:meth}`openmobisim.Run.link_bins` (with `link_bin_s`), `link_bins.parquet` |
| per-link bins by entry time | the same, filed by when traversals **entered** the link, unfinished ones included with the time they had taken so far (at least free flow): what the next choice reads | used inside the run ([Equilibration](equilibration.md)) |
| peak occupancy | the most PCU on each link at once | {py:meth}`openmobisim.Run.report_spillback` |
| what stood still at the end | vehicles still on a link or waiting to enter, links whose front vehicle waits, closed waiting loops | {py:meth}`openmobisim.Run.report_gridlock`; a `gridlock` diagnostic per loop |
| reroutes | each reroute, and the realised routes of the trips that re-routed | {py:meth}`openmobisim.Run.route_changes`, {py:meth}`openmobisim.Run.route_realised` |

**The spillback report.** Under the point queue a queue takes no space, so the report lists the links whose peak
occupancy exceeded their storage by more than `min_excess_pcu` (5 by default): where a real queue would have backed up
into the links upstream, and where the run's delays are therefore optimistic.

**The gridlock report.** `room_waits_with_room` counts front vehicles waiting for room on a link that had room; the
model lets traffic stop only at jam density, so it is always 0, and anything else is a defect.

## Timed changes

% src: crates/core-loading/src/ltm.rs Rules::capacity_changes, with_capacity_changes; python/openmobisim/scenario.py _disruptions

A scenario's `disruptions=` change a link's capacity, or close it, at a time of day, during the loading. See
[Scenarios and edits](../guides/scenarios.md).

## Defaults and options

% src: python/openmobisim/scenario.py Scenario.__init__ signature; crates/core-sim/src/loading_rules.rs LoadingOptions::SHIPPED

| Argument | Default | Meaning |
|---|---|---|
| `flow_level` | 2 | the level (table above) |
| `flow_step_s` | 300 s | the loading step; results do not depend on it |
| `loading_options` | see the [parameters page](../reference/parameters.md), group `loading` | `priority`, `reroute`, `reroute_after_s`, `reroute_max`, `reroute_min_gain`, `pocket_length_m` |
| `window` | earliest departure, rounded down to the quarter hour, for three hours | the simulation window |

Every value is an uncalibrated default. Under `equilibration="free_flow"` or in the first loading of `"msa"` (the
free-flow loading), levels 2, 3 and 4 all load as the point queue at the default `warmup` of 1
([Equilibration](equilibration.md)).

## For bikes

Bikes are not loaded: they ride their own layer at fixed speeds, take no road capacity and do not slow cars, and cars
do not slow them. A cyclist's travel time is the time of their route on the bike layer
([Networks](networks.md)), whatever the traffic. Where bikes and cars share a street the loading does not see them.

## Limits

- No level 1 (a volume-delay loading) exists.
- A signal is a cycle average: no phases, no queue within a cycle, and its delay does not respond to the traffic of
  the run.
- Give-way at roundabouts follows the `junction=roundabout` tag only: a ring mapped without it does not give way.
  Priority is by presence on the circulating link, not by a gap in time.
- Nothing resolves a standing gridlock: rerouting lets stuck vehicles leave a lock, but a loop that fills through its
  own circulation stays locked until the window ends ({py:meth}`openmobisim.Run.report_gridlock` reports it).
- Turn pockets give every movement the same pocket, whatever its volume; lanes per movement are not read.
- A vehicle straddling a link can stretch: its part on a link can be shorter than the link while room has not yet
  reached the link's start. Its PCU is always counted in full.
- Bicycles, pedestrians and cars do not interact.
- A junction has no capacity of its own: crossing movements do not conflict, apart from a signal's green share and the
  give-way at roundabouts.
- Where several approaches each turn several ways, a full exit is not shared by oriented capacities as in the node
  models of Tampère et al. {footcite}`tampere2011`: an approach's share also depends on the traffic at the other exits
  it feeds, and an approach that uses most of its own capacity is held earlier than in their models when it enters a
  nearly full exit, as its own headway and the exit's add up ([Junctions](#junctions)).
- Departures enter their first link held by its room only, not by its inflow capacity.

**Not limits of this loading.** The problems of a link transmission model computed in time steps do not arise here,
because every vehicle's movement is computed at its exact time: no causality errors between links shorter than a step,
no oscillation from the Courant (CFL) condition, no numerical diffusion of peaks, no dynamics lost within a step, and
no dependence on the step. Measured on the Delft starter case at levels 2 and 4: a loading step of 60 s and of 300 s,
and output bins of 60 s and of 900 s, give the same 65 956 trips to the second, and the 60-s bins sum to the 900-s
table exactly. What a vehicle-based model has instead is the size of its vehicles: traffic moves in whole vehicles, and
a simulated traveller who stands for $w$ people is one vehicle of $w$ PCU, so queues form and clear in steps of that
size ([Sampling the demand](demand.md#sampling-the-demand)). Against the macroscopic link transmission model on the
reference cases, the counts agree to within one or two vehicles per link and second at one traveller per person
([Validation](validation.md)).

**What is coarse, and where.** The costs the next iteration chooses on are means per 300-s bin of entry
([Equilibration](equilibration.md)); the per-link outputs are sums per output bin by the time of exit, a traversal
counted whole in the bin it ended in; a signal is a cycle average; the route update's trees step at `tree_step_s`.
None of these touches a vehicle's movement.

**The full model (levels 3 and 4) under heavy congestion.** When routes settled at the point queue (the warm-up) are
first loaded with spillback, many of them gridlock, and the iterations recover slowly: on the Lyon starter case the
point queue completes 97 % of trips with an acceptable verdict in ten iterations, and level 4 completes 91 % with a
poor verdict, still improving at the tenth. In that regime the result is sensitive to small changes of the input (a
few trips more or fewer can change which junctions lock), so run with more iterations than the default and read the
results as a range over perturbations. The run time at levels 3 and 4 is dominated by the rerouting of stuck vehicles
(`reroute`, on by default there), not by the loading: on the Lyon case ten full loadings take 11 s without rerouting
and 280 s with it. Level 3 gave no gain over level 4 on that case.

## References

```{footbibliography}
```
