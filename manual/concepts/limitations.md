# Known limitations

% Gathered from the *Limits* section of each concept page (each states facts about the code as built).

What openmobisim does not do, stated as facts about the code, page by page. State the ones that matter for your
results with them.

## [How a run works](architecture.md)

- Level 1 of the flow levels (a volume–delay model) is not built.
- `kpis.parquet`'s `design_id` and `replication` are labels a run is given (`Scenario.run(design_id=…,
  replication=…)`), `"default"` and 0 for a single run; a replication differs from another only through its
  `master_seed`. A design space is run as a loop of runs: there is no batch runner yet.

## [Networks](networks.md)

- OpenStreetMap's turn restrictions are not applied; every movement but a U-turn away from a dead end is allowed.
- Signal timings are not read: every signal has the same cycle, green share and a delay that does not respond to the
  traffic of the run.
- A signalised mid-block crossing (no junction within `signal_merge_m`) delays both directions like a junction's
  approach: in the starter cities, 13–51 crossings each.
- Which junction a stop-line signal belongs to is read from distances along the street (`signal_merge_m`), not from
  the tags of the junction's approaches: a stop line between two junctions closer than that serves the nearer one.
- Lanes per movement (`turn:lanes`) are not read.
- A changing speed limit along a way takes its first value; implicit limits are one table for every country.
- Sides of a street (`cycleway:right|left`) assume right-hand traffic.
- Reversible lanes are read as two-way.
- Bikes and pedestrians do not congest, and do not interact with cars.
- Every default is uncalibrated; each one's source is on the [parameters page](../reference/parameters.md).

## [Demand](demand.md)

- Departure times are fixed: there is no choice of when to travel.
- Trips made from an OD matrix are single trips, not chains; a day of several trips needs a trips table.
- Columns of the trips table beyond the eight required and two optional ones are not read. Values per trip or traveller
  for the choice model are given to the `Scenario` instead (`trip_values`, `person_values`; see [Choice](choice.md)).
- A traveller's weight is a whole number.
- One day: departures are seconds after midnight of one day.

## [Route sets](route_sets.md)

- Sets are made on free-flow times; congestion enters them only through the route update.
- Neither generation method promises the best routes or all the plausible ones.
- The route update searches at one departure per pair by default (the median), so a route that pays only at some
  hours may be missed.
- A trip is snapped to the nearest street node (a motorway-only node only within 50 m), whatever lies between.
- Bike and walk trips have one route each; bike route choice is not stochastic.

## [Choice](choice.md)

- The defaults are uncalibrated. openmobisim does not estimate coefficients; estimate them elsewhere and pass them.
- The built-in models are the logit and the nested logit with one level of nests (the mode). Other forms (a mixed logit,
  a model that is not random-utility) can be written as your own model.
- `deterministic` reads `time_min` only.
- A Python model is called once per batch from the run, which is slower than a built-in model.
- `choice_max_routes` limits car routes, not itineraries.
- Bikes have one route per trip (above).

## [Modes and itineraries](modes.md)

- Every vehicle is the traveller's own, starting at their first trip's origin. There is no car sharing, bike sharing or
  taxi, except the car a trip with no alternative falls back to.
- A vehicle left at a parking at the end of the day is not weighed: nothing penalises not fetching it.
- A transit pass is free: its price is not among the money attributes; it waives the fare and does nothing else.
- A car or bike trip has no walk to or from the vehicle in the loading; mode choice counts two and three minutes
  for a car (`car_access_s`, `car_egress_s`), which its travel time in the results does not hold.
- A cyclist or walker has one route per trip, not a choice among routes.

## [Transit](transit.md)

- One service day; trips of the day before that run past midnight are not read.
- Vehicles have no capacity: nobody is left behind at a full vehicle, and crowding is not felt.
- Only buses ride the roads; trams, metros, trains and ferries keep the schedule (or a delay given as a disruption).
- A bus's dwell is fixed, not dependent on boardings.
- Fares are not read from the feed; give prices in `price_options` (see [Modes and itineraries](modes.md)).

## [The loading](loading.md)

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
  models of Tampère et al. (2011), whose other requirements the junction rules meet: an approach's share also depends
  on the traffic at the other exits it feeds, and an approach that uses most of its own capacity is held earlier when
  it enters a nearly full exit ([Junctions](loading.md#junctions)).
- Departures enter their first link held by its room only, not by its inflow capacity.
- At level 2 a queue caused by a narrower link stands at the end of the link before it, and holds back every vehicle
  behind it on that link, whichever way it turns; trip times along a single chain of links are unchanged by this.
- **Not limits of this loading:** the problems of a time-stepped link transmission model (causality errors between
  short links, Courant-condition oscillation, numerical diffusion of peaks, dynamics lost within a step, dependence on
  the step) do not arise, because every vehicle moves at its exact time; measured on Delft at levels 2 and 4, a step of
  60 s or 300 s and output bins of 60 s or 900 s give the same trips to the second. In their place is the size of the
  vehicles: traffic moves in whole vehicles, and a simulated traveller of weight $w$ is one vehicle of $w$ PCU, so
  queues form and clear in steps of that size ([Sampling the demand](demand.md#sampling-the-demand)).
- **What is coarse:** the costs the next iteration chooses on are means per 300-s bin of entry; the per-link outputs
  are sums per output bin by time of exit; a signal is a cycle average; the route update's trees step at `tree_step_s`.
- **The full model (levels 3 and 4) under heavy congestion:** the first loading with spillback after the point-queue
  warm-up gridlocks many routes and the iterations recover slowly (Lyon starter case: 91 % of trips completed with a
  poor verdict in ten iterations, against 97 % and acceptable at the point queue); the result is sensitive to small
  changes of input; run more iterations than the default and read the results as a range. The run time there is the
  rerouting of stuck vehicles, not the loading. Level 3 gave no gain over level 4 on that case.

## [Equilibration](equilibration.md)

- Only two strategies exist; others (re-planning a share of travellers on their whole day, for instance) are not built.
- The gaps leave out trips still under way when the window ended; `incomplete_share` says how many.
- `gap_network` is measured on a sample, at the last iteration only.
- At `flow_level=0` nothing interacts, so `"msa"` has nothing to settle and stops at the choice model's own floor.
- Iterations are not guaranteed to converge: under heavy congestion at level 4, where queues spill back, they can drift.
- `time_change` is a sum over (link, bin) cells of the change in mean time, so with traveller weights above 1 a few
  vehicles crossing a bin boundary move it (Lyon starter case, weight 5: near 0.3 while every aggregate is stable). It
  is reported, not read by the verdict.

## [Parking](parking.md)

- Parkings serve only park-and-ride and bike-and-ride. Parking at the destination of a car trip is not simulated; its
  price can be given per trip (`trip_values` `parking_eur`).
- Capacity is soft: nobody is turned away from a full parking.
- A vehicle left at a parking at the end of the window is counted (`vehicles_left_at_parkings`) but not weighed in any
  choice.
- Shared bikes and their docks are not part of this version.

## [Reproducibility](reproducibility.md)

- A design space with replications is run as a loop of runs, each labelled with its `design_id` and `replication`;
  a replication is made by changing the master seed. There is no batch runner yet.
- The guarantee is per platform (above).

## For bikes

Bike routes do not depend on traffic (they are found once per run on the bike layer); there is no stochastic bike
route choice and no bike sharing in this version.
