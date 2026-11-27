# Limitations

% src: README.md ("Known limits"); the "Meanings and assumptions", "Errors", "Repairs" and "Bikes" sections of every contract under crates/*/README.md; PARAMETERS.md

Everything the model does not do, or does approximately, gathered in one place and stated as facts about the program
as it is. Each section belongs to one part of the model and links to the page that explains it. State the ones that
matter for your results with them.

openmobisim is a small simulator: travellers choose mode and route, cars queue on a point queue, bikes and
walkers travel at free flow, transit runs on its timetable, and choice and loading are iterated to a stable state. The
table lists the limits that most often decide whether the model fits a question; the sections below give all of them.

| The model does not have | What it does instead | Read with care when |
|---|---|---|
| Spillback: a queue has no length on the road | Every car waits at the downstream end of its link, and a link stores any number of cars | A queue in reality blocks junctions upstream (a motorway exit backing up onto the main carriageway) |
| Junctions: signals, turn delays, priority, merges | Each link's capacity is the only thing that holds a car; signals are read and applied nowhere | Delay at junctions, not on links, decides the question |
| Bike and pedestrian congestion | One bike speed and one walk speed, on every link, at any flow | Cycle paths are crowded, or bike speeds differ by path or by person |
| Transit delayed by traffic, or full | Every vehicle keeps its timetable and takes every passenger | Buses share congested roads, or vehicles are crowded |
| A choice of departure time | The departure second of every trip is an input | Travellers would leave earlier or later to avoid a queue |
| Money | The cost of every alternative has 0 euros | Fares, tolls or parking charges matter |
| More than one day, or less than a whole trip | One service day; a trip not at its door when the window ends has no travel time | The window is short for the trips in it |

## The network

% src: crates/graph/README.md ("Meanings and assumptions", "Repairs", "Bikes"); crates/io-osm/README.md ("Meanings and assumptions": signals, the region; "The class table"; "Trimmed from release 0.1"; the experimental `CarRoads::Main`); PARAMETERS.md (`osm_class_defaults`, `bike_speed`, `walk_speed`); crates/binding/README.md ("No signals from tables"; "Experimental")

Explained on [Networks](networks.md).

- **A link has a length, the modes allowed and, for cars, lanes, a free-flow speed and a capacity.** Nothing else: there
  is no travel-time column, and a link's free-flow time is its length over the mode's free-flow speed.
- **One speed per mode for bikes and walkers**, the same on every link: `bike_speed` and `walk_speed`. A cycle path and
  a street are ridden at the same speed.
- **Turn restrictions are not read**, and no movement at a node is forbidden or delayed.
- **Capacities read from OpenStreetMap are defaults by road class.** Each car link
  takes its road class's capacity per lane times its lanes, and for roads with junctions at grade that capacity is a
  saturation flow of 1 800 vehicles per hour per lane times a share of green time. **The green share of each road class
  is an assumption**, not a value of a guideline, and none is calibrated.
- **Speeds and lanes where OpenStreetMap gives none are one table for every country.**
- **Signal timings, bus lanes, parkings, the duration of ferries and the levels of cycling infrastructure are not
  read** from OpenStreetMap.
- **A network read from OpenStreetMap is trimmed, per mode, to its largest strongly connected part; a network from
  tables is used as given.** Links outside that part are dropped and counted.
- **A region cuts the network.** Ordinary streets are cut one node beyond the region's edge; motorways, trunk roads
  and their slip roads are kept up to 2 000 m outside it.
- **Distances are on a plane.** Positions in longitude and latitude are projected to the UTM zone of the network's
  centre, within 0.1 % of the ground distance inside the zone. A link's length, when not given, is the straight line
  between its two nodes, not the length along its shape.

## The demand

% src: crates/demand/README.md ("Inputs", "Meanings and assumptions", "Repairs", "Bikes"); crates/loop/README.md ("Money is 0 …"; "Repairs"); crates/io-osm/README.md (the experimental `trips::draw`); crates/binding/README.md (`make_case`)

Explained on [Demand](demand.md).

- **A trip is one traveller making one movement from door to door**: an origin, a destination, a departure second, a
  class and the modes open to it. The model does not link the trips of one person through the day.
- **The departure time is an input.** Nobody leaves earlier or later because of a queue or a timetable.
- **The modes open to a trip are an input.** Whether a traveller owns a car or a bike is expressed by the modes given.
- **A door is linked to the network by a straight line** to the nearest node the mode can use, walked at the walk speed
  by every mode, the driver included. There is no parking and no search for a parking place.
- **Money is 0 for every alternative.** The cost has a column for euros and no source that fills it: no fare, toll,
  fuel or parking charge.
- **Trips outside the window are dropped**, as are trips whose walk to the network would end after the window for every
  mode. They are counted in the record and are in no result.
- **`make_case` is an experimental helper** that draws uncalibrated trips from an OpenStreetMap network.

## Routes and route sets

% src: crates/search/README.md ("Meanings and assumptions": fastest, ties); crates/loop/README.md ("Search", "Car routes per origin-destination pair", "What the choice is over"); PARAMETERS.md (`max_car_routes`); README.md ("Known limits")

Explained on [Routes and route sets](search.md).

- **A traveller chooses only among what the search found.** A set holds, per mode, the fastest route on the empty
  network and, for cars, every route that was the fastest at some iteration. It is not every route: a car route that
  is never the fastest at any iteration is never offered.
- **Of two routes of exactly equal time, one is offered.** The search's tie rule finds one of them.
- **The search can miss the earliest arrival at a tie.** When two routes reach a node at exactly the same instant in
  different places of a queue, the route kept can arrive later than the one dropped, by a headway or more.
- **A bike and a walker have one route each**: the shortest by length, found once.
- **The car routes of an origin-destination pair are limited** by `max_car_routes` (6 by default). A faster route
  found by an origin-destination pair at its limit is lost and replaces none. Such a pair can also hold more than the limit, when its travellers' first routes differ.

## Choice

% src: crates/choice/README.md ("Meanings and assumptions"); crates/loop/README.md ("Choose"); PARAMETERS.md (`coefficients`, `nest_scale`)

Explained on [Choice](choice.md).

- **Two models**: the least-cost choice, and one nested logit whose nests are the modes, with one nest scale for all nests.
- **The cost is linear** in the attributes of an alternative: minutes of each kind of time, transfers, euros, a constant per
  mode. There is no reliability, comfort or crowding term.
- **The default coefficients are starting values**, to be calibrated for each study; they are not estimates for any
  city.
- **A traveller's taste is drawn once** and kept for the whole run, whatever the iteration.
- **The path size corrects overlap among car routes only**, and is read as 0 under the least-cost choice. Transit
  journeys that share rides are not corrected for their overlap.
- **Classes differ by their coefficients and nothing else.**

## Transit

% src: crates/transit/README.md ("Meanings and assumptions", "Bikes"); crates/io-gtfs/README.md ("Meanings and assumptions", "Bikes"); crates/demand/README.md ("Lists"); crates/loop/README.md ("Search": `max_transit_journeys`; "Load")

Explained on [Transit](transit.md).

- **One service day**: the busiest weekday of the feed. No other day can be asked for.
- **Transit runs as scheduled.** No vehicle is delayed by road traffic, no bus is on a road, and no realised times are
  read.
- **A vehicle has no capacity.** Nobody is left behind and crowding is not felt.
- **Fares are not read.**
- **The access and egress are walks.** There is no bike-and-ride and no park-and-ride; `bikes_allowed` is not read.
- **A call closed to boarding or alighting in the feed is open to both.**
- **Travellers know the timetable** and take a journey found on it; but they do not time their departure to it.
- **At most one footpath between two rides**, footpaths of at most `transfer_walk_max`, access and egress walks of at
  most `stop_walk_max`, at most `max_rides` rides.
- **At most `max_transit_journeys` journeys are offered** to a traveller; the limit can drop its earliest journey, which
  may be the only one arriving within the window.
- **The part of a run of the day before that lies before midnight is not in the day.**
- **Transit is searched once**, at iteration 0: a journey's time and cost are the same at every iteration.

## The loading

% src: crates/loading/README.md ("Meanings and assumptions", "Bikes"); crates/graph/README.md ("Meanings and assumptions"); README.md ("Known limits")

Explained on [The loading](loading.md).

- **No spillback.** The point queue stores any number of cars on a link. A queue never reaches the links upstream, so it
  never blocks them.
- **No junction model.** No rule at a node, no merge rule and no signal holds a car; nothing costs time at a turn. Where
  links meet, cars queue on the next link's end in the order they reach it.
- **Signals are stored and applied nowhere.** A signal never changes a link's free-flow time or its capacity. On a
  network read from OpenStreetMap, a link's capacity stands in for the junctions of its road class
  ([above](#the-network)).
- **No storage, jam density or wave speed**, and **no passenger-car unit**: every vehicle counts as one.
- **Bikes and walkers travel at free flow**: one bike speed and one walk speed, on every link, for everyone. There is
  no bike congestion, and bikes, walkers and cars do not delay one another.
- **Transit is not loaded.** No bus is on a road; a vehicle has no capacity.
- **The window cuts trips off.** A trip not at its door when the window ends is unfinished and has no travel time. The
  mean trip time is over the finished trips only: read it together with the unfinished count beside it.
- **Near the window's end, costs read too low.** A link that a traveller would enter after the window's end is read
  without the cars that would have entered it after the end. Where every trip finishes, this does not arise.
- **The time of a route not taken is an estimate.** The link time reading holds every other car where the loading
  recorded it. Where moving the traveller would move other cars, the reading is off by one headway of the link for each
  car held on the wrong side, and queues further on can add to that; along a route the error is not bounded by one
  headway.
- **A car trip while a bottleneck's queue builds up is slightly above theory**: by about 1 s on a trip of about 692 s
  (0.1 to 0.2 %).

## The iteration and the stopping rule

% src: README.md ("Known limits"); crates/loop/README.md ("Stop", "Assess", "The window and the mean", "Who chooses again"); PARAMETERS.md (`step_exponent`, `iteration_cap`)

Explained on [The iteration](iteration.md).

- **A run the rule stops is near, not at, the equilibrium.** The rule reads the slope of the mean trip time, which the
  shrinking step makes small before the distance is. With two car routes, one behind a bottleneck, the trip time of
  the departure period farthest from theory is about 2 s from it.
- **The distance to equilibrium is reported, never judged.** The share who would change (logit) and the duality gap
  (least-cost choice) are in every row of the iterations table; no value of either stops or fails a run.
- **The distance is over the sets found.** A share who would change of 0 means that nobody would change among the
  alternatives the search found, not among all routes of the network.
- **The settled state depends on the step exponent**, because the sets grow by the search at the costs each iteration
  meets, so another step exponent can settle at another state.
- **The mean trip time the rule reads is over finished trips**; with unfinished trips, two iterations' means are over
  different trips.
- **Under the least-cost choice the rule that offers a new choice more often to travellers far from their best has
  limits of its own**:
  it depends on constants added to every cost, favours short trips, and above its band can move travellers behind one
  queue together.
- **Results are a sample.** Under the logit another seed gives other tastes and other travellers offered a new choice; a single
  run's
  numbers scatter around the model's.

## What a run gives

% src: crates/io-parquet/README.md ("Outputs", "Undetectable, said once", "a known limit: paths are compared, not files"); crates/map/README.md ("A pair", "Tests"); crates/summary/README.md ("Tests"); crates/binding/README.md ("No command line, …")

Explained on [What a run gives](outputs.md).

- **Only the last iteration is written**, and of each traveller's set only the alternative taken.
- **No time per leg of a route**, no single vehicle's passage of a link, and no transit vehicle's load is written.
- **Walks off the network are in no link count**: the walks between a door and the network, and every walk of a transit
  trip.
- **A links table holds no bike or walk time**, since both are the free-flow time by definition.
- **The map holds the 2 000 busiest pairs of places.**
- **A value changed by hand in a results table is not detected.**
- **An output file is told from an input file by its path.** An input that is a hard link of one of the five output
  names in the output folder is not recognised and would be replaced.
- **The interface is the Python module alone**: there is no command line, no scenario object, no figure function, no
  log and no progress output.

## Reproducibility

% src: crates/types/README.md ("Determinism"); crates/loop/README.md ("Across platforms, as measured"); crates/io-parquet/README.md; crates/binding/README.md

Explained on [Reproducibility](reproducibility.md).

- **Identical results are promised for the same program on the same platform**; across platforms they were measured on
  three, not proven.
- **The computation times in the iterations table differ from run to run.**
- **A run made from tables in memory cannot be run again from its record alone.**
- **The record names the input files; it does not hold them.**
- **A fingerprint is not a cryptographic hash.**

## Validation

% src: README.md ("The ten tests", "Known limits"); the tests' headers (crates/loop/tests/braess.rs, sioux_falls.rs)

Explained on [Validation](validation.md).

- **The tests check the model's logic against theory, hand derivations and other tools**, not its realism against
  observed traffic. No case in the tests is calibrated to counts or measured travel times.
- **A single link's queue delay on Sioux Falls is not compared**: the sum over the links agrees with the outside
  solver, single links differ.
- **The equilibrium tests agree within a few percent.**

## For bikes

% src: README.md ("Known limits", last item); crates/graph/README.md ("Bikes"); crates/loading/README.md ("Bikes"); crates/transit/README.md ("Bikes"); crates/io-osm/README.md ("Trimmed from release 0.1": the bike layer's infrastructure levels)

Bikes are a mode of every part of the model, and these are the limits that concern them, gathered from above:

| Limit | Consequence |
|---|---|
| One bike speed, on every link, for every cyclist | A cycle path and a street are ridden alike; a fast and a slow cyclist do not exist |
| No bike congestion | A crowded cycle path is as fast as an empty one |
| Bikes and cars do not interact | A queue of cars does not slow a bike on the same street, and bikes take no capacity from cars |
| No signal or junction delay | A bike's time is its route's length over its speed, and its route the shortest by length |
| The kind of cycling infrastructure is not read | The choice between a bike route on a cycle path and one on a busy street is by length alone |
| One bike route per traveller | There is no route choice among bike routes |
| No bike-and-ride | A bike is never the access to a stop |
| No parking | A bike, like a car, starts and ends at the node of its network nearest the door; nothing is spent on parking it |

## References

This page gathers the limits of the other concept pages, which cite the works behind each method. The full list is in
the [bibliography](../reference/bibliography.md).
