# Transit

% src: crates/transit/README.md; crates/io-gtfs/README.md; crates/demand/README.md; crates/loop/README.md ("Search", "Choose", "Load", "Measure"); PARAMETERS.md

**Transit** is scheduled public transport: trains, metros, trams, buses and ferries that run to a timetable. A transit
trip walks from its door to a stop, waits, rides one or more vehicles, walking between them where it changes, and
walks from the last stop to its door. openmobisim simulates the timetable of **one service day**, read from a GTFS
feed (the General Transit Feed Specification, the format in which transit agencies publish their timetables). Every
vehicle runs exactly as the timetable says: road traffic does not delay it, and passengers do not fill it.

Three parts of the simulator make a transit trip, and one reads the result:

| Part | What it does for transit | It takes | It gives |
|---|---|---|---|
| The GTFS reader (module `io-gtfs`) | Reads one service day of a feed | A GTFS `.zip`, the [network](networks.md) (its projection, its walk links and walk speed), `transfer_walk_max` | The timetable: stops, runs and footpaths; a report of what was read |
| The travellers (module `demand`, [Demand](demand.md)) | Finds the stops each trip can walk to and from | The trips, the network, the timetable's stops, `stop_walk_max` | Per trip an access list and an egress list: stops with their walking seconds |
| The journey search (module `transit`) | Finds journeys on the timetable | The timetable, the two lists, the instant the traveller is at each access stop, `boarding_slack`, `max_rides` | Journeys: the rides, the walks between them, the arrival at the door |
| The iteration (module `loop`, [The iteration](iteration.md)) | Asks for each trip's journeys once, prices them and offers them to the [choice](choice.md) beside the car, the bike and the walk | The journeys, the class's coefficients | The transit alternatives in each traveller's set; the journey taken, in the [results](outputs.md) |

Where transit sits in a whole run is drawn in [How a run works](architecture.md).

## One service day from a GTFS feed

% src: crates/io-gtfs/README.md ("Inputs", "Outputs", "Meanings and assumptions", "Repairs"); crates/binding/README.md (`Transit.from_gtfs`)

`Transit.from_gtfs(path, network)` reads one feed, a `.zip`, on a network built from longitude and latitude (the
network's projection places the stops, and its walk links give the walks between them). It reads `stops.txt`,
`trips.txt` and `stop_times.txt`, the calendar (`calendar.txt`, `calendar_dates.txt`, either or both), and
`frequencies.txt` and `transfers.txt` when they are there. Shapes and fares are not read; `routes.txt` is read only for
the names shown on the run's pages.

- **The service day** is the busiest weekday: the Monday to Friday with the most trips in service, counted over the
  feed's whole range of dates with every calendar exception applied; the earliest such day on a tie. A feed with no
  service from Monday to Friday is read on its busiest day of all seven, and this is noted. No other day can be asked
  for.
- **A stop** is every row of `stops.txt` with `location_type` 0 or blank and a position, whether a vehicle calls at it
  or not. No stop is dropped by area.
- **A run** is one vehicle trip of the timetable: every GTFS trip in service that day, with its **calls** in order,
  each call a stop with an arrival second and a departure second. A blank time is filled from the other time of the
  same call, or interpolated between the timed calls around it. A trip whose first or last call has no time, or that
  has a single call, is dropped and counted.
- **Frequencies.** A trip listed in `frequencies.txt` is a template: one run leaves at each row's start time and every
  headway after it until the row's end time.
- **Runs past midnight.** Times after 24:00:00 are kept (25:10:00 is 90 600 s). The runs of the day before that are
  still on the road after midnight enter the day with the calls that depart after midnight, shifted back by 86 400 s.
- **Boarding and alighting flags** (`pickup_type` and `drop_off_type` 1) are counted and ignored: every call is open to
  boarding and to alighting.

Times are whole seconds after the service day's midnight, the same clock as the trips' departures and the run's window.

A row the reader cannot use is set right or left out, counted, and reported as a warning; the feed is still read. The
reader refuses only a feed from which no timetable can be made: a file that is no zip, a missing required file or
column, a feed without any day in service, or a network built in metres (on which no stop can be placed). The counts
are in `Run.gtfs_report` and in the run's record. For the Delft case of the [published cases](../reference/cases.md),
the reader reports:

| Count | Value |
|---|---|
| Service day read | 9 October 2026 |
| Runs | 1 707 |
| Calls | 39 156 |
| Stops | 1 340 |
| Footpaths between stops | 902 |

### The kind of service

% src: crates/io-gtfs/README.md ("Outputs": `Lines`, `Kind`); crates/map/README.md ("Transit")

Beside the timetable, the reader keeps each stop's name and each run's line and kind of service, for the map and the
summary only. The search, the choice and the run's results do not read them: every kind runs the same way, on its
timetable.

| Kind | GTFS `route_type` |
|---|---|
| rail | 2, 100–199, 300–399 |
| metro | 1, 12, 400–699 |
| tram | 0, 5, 900–999 |
| bus | 3, 11, 200–299, 700–899 |
| ferry | 4, 1000–1099, 1200–1299 |
| other | anything else |

## Stops, runs, patterns and footpaths

% src: crates/transit/README.md ("Inputs", "Meanings and assumptions"); crates/io-gtfs/README.md ("Footpaths", "`transfers.txt`")

The timetable holds three things.

- **Stops**, each a position.
- **Runs**, each a sequence of calls. Runs that serve the same stops in the same order and never overtake one another
  form a **pattern**; an express and a local on the same stops are two patterns. The search scans patterns, not single
  runs.
- **Footpaths**, the walks a passenger may make between two stops when changing. They are derived once, when the feed
  is read, on the network's walk links: each stop is linked to its nearest walk node by a straight line, and a footpath
  from stop *a* to stop *b* is *a*'s straight line, the walk between the two nodes, and *b*'s straight line. It is kept
  when it is at most `transfer_walk_max` metres (400 m by default), and takes its metres over the walk speed. Where
  `transfers.txt` gives a minimum transfer time between the two stops of a footpath, the footpath takes the longer of
  the two times.

## Walking to and from stops

% src: crates/demand/README.md ("Outputs", "Meanings and assumptions": linking, reach, lists); PARAMETERS.md (`stop_walk_max`)

The walk from the door to the first stop is the **access walk**, the walk from the last stop to the door the **egress
walk**. Which stops a trip can use is settled before the run, when the travellers are built:

- The **access list** of a trip holds every stop that can be reached on foot from its origin within `stop_walk_max`
  (1 500 m by default), each with its walking seconds: the straight line from the door to its walk node, the walk on
  the network to the stop's node, and the straight line to the stop, all at the walk speed.
- The **egress list** holds the same from each stop to the destination.

A trip that may use transit and has an empty list at either end has no transit alternative. This is counted, not an
error: the iterations table reports the trips with transit open to them and no journey offered (`without_transit`) and,
of those, the trips with no stop in reach at one end (`without_stop`), which a larger `stop_walk_max` can reach. In
the Delft case, 609 of 37 649 trips are in both counts.

## The journey search

% src: crates/transit/README.md ("Outputs", "Units", "Meanings and assumptions"); crates/transit/src/cheapest.rs (module documentation: "The cost", "How")

A **journey** starts at a stop of the access list at the instant the traveller reaches it, boards a run, rides it to a
later call, may walk one footpath to another stop and board again, and ends at a stop of the egress list. It takes at
least one ride and at most `max_rides` rides (4 by default), and at most one footpath between two rides. Walking all
the way is not a journey; it is the walk alternative.

**Boarding slack.** A traveller boards a run at a stop only when at the stop at least `boarding_slack` seconds (60 s by
default) before the run leaves. The slack applies at every boarding, the first included.

The search is **RAPTOR**, the round-based public transit routing of Delling, Pajor and Werneck (2015). It works in
rounds: round *k* finds, for every stop, the earliest arrival using at most *k* rides, by scanning each pattern that
passes a stop improved in the round before and boarding the earliest run that can be caught, then walking the footpaths
from the stops a ride improved. Two questions are asked of it.

**The earliest journey per number of rides.** For each number of rides that reaches the door earlier than every smaller
number does, the search returns the earliest journey with that many rides, fewest rides first. The last of them is the
earliest journey of all. A slower journey with fewer rides is kept; a later journey with more rides is not. Among
journeys arriving at the same second the search takes the one with the fewest rides, then the one whose last ride is on
the run of lowest identifier, then the egress stop of lowest identifier.

**The cheapest journey per number of rides.** The earliest journey is not always the one a traveller prefers: a slow
vehicle boarded at once can arrive before a fast one that has to be waited for, and a nearer stop can have a later
run. So the same rounds are run a second time on the traveller's own cost, a sum that is linear in the journey's
parts:

$$
c = \beta_\text{access}\, t_\text{access} + \beta_\text{first wait}\, t_\text{first wait} + \beta_\text{ride}\, t_\text{ride}
  + \beta_\text{transfer wait}\, t_\text{transfer wait} + \beta_\text{transfer walk}\, t_\text{transfer walk}
  + \beta_\text{egress}\, t_\text{egress} + \beta_\text{transfer}\, n_\text{transfers},
$$

with the times $t$ in minutes, $n_\text{transfers}$ the rides less one, and the coefficients $\beta$ those of the
traveller's class. Each stop then holds a set of labels, each an arrival with the cost so far, in place of one
arrival, and a label is dropped when another, with no more rides, is there no later and costs no more once it has
waited for the first one's arrival. From a label the search boards every run that can be caught, not the earliest
alone, because a later, slower run can reach a connection for less. For each number of rides that makes a journey
cheaper than every smaller number, the search returns the cheapest; of two journeys of equal cost, the earlier at the
door. Every coefficient of a transit time and of the transfers must be zero or positive, or the run is
refused.

Both searches are compared with an exhaustive enumeration of every journey on random timetables
([Validation](validation.md), test 8).

**Whole seconds.** The timetable is in whole seconds. A walk is not rounded in the journey's parts, but it is put on
the clock rounded up to the whole second: a traveller at a stop at 1 400.5 s misses the run that leaves at 1 400 s and
catches the one at 1 401 s.

## What a transit alternative is, and how many are offered

% src: crates/loop/README.md ("An alternative", "Search", the paragraph on `nearby_stops`, `Cheapest::by_rides` and `max_transit_journeys`); PARAMETERS.md (`nearby_stops`, `max_transit_journeys`)

A **transit alternative** is one journey with its two walks: the access walk, the rides with the walks between them,
and the egress walk. It stands in the traveller's choice set beside the car route, the bike route and the walk, and the
[choice](choice.md) treats all the journeys of a traveller as one nest, the transit mode.

The iteration searches transit once, at iteration 0, because a journey's time never changes during a run
([The iteration](iteration.md)). For each trip with transit open to it, it gathers journeys in three steps and then
trims them:

1. **The earliest journeys between any stops in reach**: the earliest journey per number of rides, from the whole
   access list to the whole egress list.
2. **The nearest stops tried alone.** The earliest journey often walks farther than a traveller would like, since a
   minute of walking weighs more than a minute of riding. With `nearby_stops` at its default of 3, the same search is
   asked again from each of the 2 stops nearest the origin alone (to any stop of the egress list), and to each of the 2
   stops nearest the destination alone (from any stop of the access list). At `nearby_stops = 1` this step is left out.
3. **The cheapest journeys for the traveller's class**, per number of rides, between any stops of the two lists.

A journey found twice is offered once. Of the journeys that ride the same runs in the same order, and so differ only
in the stop boarded or left at, only the cheapest for the traveller is offered. Of what remains, at most
`max_transit_journeys` are offered (10 by default): when more were found, the cheapest for the traveller are kept.

## The cost of a journey, and the wait at the first stop

% src: crates/loop/README.md ("Choose": the sixteen columns, the first wait, the default coefficients); PARAMETERS.md (`coefficients`); crates/io-parquet/README.md (`routes.parquet`); a run of the Delft case

```{image} ../_static/figures/transit_journey_paper.svg
:alt: A transit journey from door to door: access walk, first wait, ride, transfer walk, transfer wait, ride, egress walk, each with its default coefficient
:class: only-light
:width: 100%
```
```{image} ../_static/figures/transit_journey_night.svg
:alt: A transit journey from door to door: access walk, first wait, ride, transfer walk, transfer wait, ride, egress walk, each with its default coefficient
:class: only-dark
:width: 100%
```

A traveller leaves its door at the trip's departure second, whatever the timetable says: the departure is an input,
not a choice. The **first wait** therefore runs from the end of the access walk to the departure of the first run
boarded, the boarding slack included, and is part of the door-to-door time. A journey's parts add up exactly to its
door-to-door time:

$$
T = t_\text{access} + t_\text{first wait} + t_\text{ride} + t_\text{transfer walk} + t_\text{transfer wait} + t_\text{egress}.
$$

Each part has its own coefficient in the choice, so that a minute of waiting or walking can weigh more than a minute
in the vehicle. The defaults, which are starting values to be calibrated per study:

| Part of a journey | Column of the choice row | Default coefficient (cost per minute, or per transfer) |
|---|---|---|
| Access walk | `transit_access_min` | 0.165 |
| Wait at the first stop | `transit_first_wait_min` | 0.145 |
| Time in vehicles | `transit_in_vehicle_min` | 0.1 |
| Walks between rides | `transit_transfer_walk_min` | 0.165 |
| Waits at transfers | `transit_transfer_wait_min` | 0.145 |
| Egress walk | `transit_egress_min` | 0.165 |
| Each transfer | `transit_transfers` | 0.5 |
| Taking transit at all | `is_transit` | 0.25 |

A cost is a sum of coefficient times quantity, positive is worse. Money is 0 for every alternative in release 0.2:
fares are not read.

**An example.** The routes table of a run lists the legs of every journey taken. For one trip of the Delft case:

```python
import pyarrow.parquet as pq

trip = "od0000016"
row = pq.read_table("delft-run/trips.parquet", filters=[("trip_id", "=", trip)]).to_pylist()[0]
print(row["departure_s"], row["mode"], round(row["travel_time_s"], 2), round(row["cost"], 4))
legs = pq.read_table("delft-run/routes.parquet", filters=[("trip_id", "=", trip)]).to_pylist()
for leg in legs:
    if leg["kind"] == "ride":
        print(leg["kind"], leg["board_stop"], leg["board_s"], leg["alight_stop"], leg["alight_s"])
    else:
        print(leg["kind"], round(leg["seconds"], 2))
```

```text
27049 transit 3533.77 8.3625
walk_gap 66.64
ride 3989648 27768 3990650 28620
transfer_walk 15.14
ride 3428643 28800 3621208 29640
walk_gap 942.77
```

The traveller leaves at 27 049 s of the clock (07:30:49) and reaches the first stop 66.64 s later. The first run it
takes leaves at 27 768 s, so it waits 652.36 s there. The table below derives its time and its cost from these legs by
hand; both agree with the trips table.

| Part | Time (s) | Coefficient (cost per minute) | Cost |
|---|---|---|---|
| Access walk | 66.64 | 0.165 | 0.1833 |
| Wait at the first stop | 652.36 | 0.145 | 1.5765 |
| Time in vehicles (852 s and 840 s) | 1 692.00 | 0.1 | 2.8200 |
| Walk between the rides | 15.14 | 0.165 | 0.0416 |
| Wait at the transfer | 164.86 | 0.145 | 0.3984 |
| Egress walk | 942.77 | 0.165 | 2.5926 |
| One transfer | | 0.5 per transfer | 0.5000 |
| Taking transit | | 0.25 | 0.2500 |
| **Door to door** | **3 533.77** | | **8.3625** |

(The folder `delft-run` is the run of the Delft case made on [What a run gives](outputs.md).)

## Transit in the iteration

% src: crates/loop/README.md ("Search", "Load", "Measure", "Assess", "Money is 0 …"); crates/transit/README.md ("A day's timetable, nothing else")

Transit **runs as scheduled**. In each stage of [the iteration](iteration.md):

| Stage | What happens for transit |
|---|---|
| Search | At iteration 0 only: the journeys above are found and added to the set. Later iterations search cars alone |
| Choose | The journeys are alternatives like any other, in the nest of the transit mode |
| Load | A journey loads nothing: no vehicle is put on a road, and a vehicle never fills |
| Measure | A journey's time is the one the search priced it at; it reads no car's crossing of a link |
| Assess | A transit trip is finished when its departure plus its journey's door-to-door time is before the window's end |

Because nothing in a journey depends on the road traffic or on the other passengers, a traveller's transit alternatives
cost the same at every iteration. What changes from one iteration to the next is how they compare with the car, whose
time does change.

## Defaults

% src: PARAMETERS.md; crates/binding/README.md ("Keywords")

| Parameter | Given to | Unit | Default | What it bounds |
|---|---|---|---|---|
| `transfer_walk_max` | `Transit.from_gtfs` | m | 400 | The longest footpath between two stops |
| `stop_walk_max` | `Demand.from_table` | m | 1 500 | The longest access or egress walk |
| `boarding_slack` | `run` | s | 60 | How long before a run leaves a traveller must be at the stop |
| `max_rides` | `run` | rides | 4 | The most rides of a journey |
| `nearby_stops` | `run` | searches per trip end | 3 | The searches for the earliest journeys at each end of a trip: one between all stops in reach, the others from the nearest stops alone |
| `max_transit_journeys` | `run` | journeys per trip | 10 | The most journeys offered to a traveller |

Their sources are on the [parameters page](../reference/parameters.md).

## For bikes

% src: crates/transit/README.md ("Bikes"); crates/io-gtfs/README.md ("Bikes"); crates/demand/README.md ("Bikes")

Nothing in transit is a bike. A run carries passengers on its timetable, a footpath is a walk on the walk network, and
the access and egress walks are on foot: a bike is never in an access or egress list, so there is no bike-and-ride
journey in release 0.2. The feed's `bikes_allowed` is not read. For a traveller who may cycle, the bike is a separate
alternative, door to door, that the choice weighs against the journeys.

## Limits

% src: crates/transit/README.md ("A day's timetable, nothing else"); crates/io-gtfs/README.md ("Meanings and assumptions"); crates/loop/README.md ("Search", "Load", "Money is 0 …")

- One service day, the busiest weekday of the feed; no other day can be asked for, and no service change within the
  day is modelled.
- Transit is not delayed by road traffic: a bus keeps its timetable in any congestion, and no bus is put on a road.
- A vehicle has no capacity: nobody is left behind, and crowding is not felt.
- Fares are not read: money is 0 for every alternative.
- The access and egress walks are on foot; there is no bike-and-ride or park-and-ride.
- A call closed to boarding or to alighting in the feed is open to both here.
- The part of a run of the day before that lies before midnight is not in the day.
- A traveller does not time its departure to the timetable: the departure second is an input.
- The limit `max_transit_journeys` can drop a traveller's earliest journey, which may be the only one that arrives
  within the window.
- At most one footpath between two rides, and at most `max_rides` rides.

All limits of the model are gathered in [Limitations](limitations.md).

## References

- Delling, D., Pajor, T. and Werneck, R. F. (2015). Round-based public transit routing. *Transportation Science*
  49(3), 591–604.
- El-Geneidy, A., Grimsrud, M., Wasfi, R., Tétreault, P. and Surprenant-Legault, J. (2014). New evidence on walking
  distances to transit stops: identifying redundancies and gaps using variable service areas. *Transportation* 41,
  193–210. (The source of `stop_walk_max`.)
- Guerra, E., Cervero, R. and Tischler, D. (2012). Half-mile circle: does it best represent transit station
  catchments? *Transportation Research Record* 2276, 101–109. (The source of `transfer_walk_max`.)

The full list is in the [bibliography](../reference/bibliography.md).
