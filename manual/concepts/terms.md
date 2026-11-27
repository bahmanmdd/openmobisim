# Terms

% src: the pages of this part and of the reference, each term as the page linked in its row defines it; behind them crates/*/README.md and PARAMETERS.md

The terms this manual uses, each in one sentence, with the page that explains it. A term in two rows has two meanings,
told apart by the words in brackets: a run of the simulator is not a run of a transit vehicle, and a link's headway is
not a timetable's.

| Term | Meaning | Explained in |
|---|---|---|
| access list | Of a trip that may use transit, every stop the traveller can walk to from the origin within `stop_walk_max`, each with its walking time. | [Demand](demand.md#linking-a-trip-end-to-stops) |
| access walk | The walk of a transit trip from its origin door to the first stop: the straight line to the walk network, the walk over the walk links, and the straight line to the stop. | [Transit](transit.md#walking-to-and-from-stops) |
| alternative | One way of making a trip by one mode: a walk from the door, then a route on the network (by car, by bike or on foot) or a journey on the timetable, then a walk to the door. | [Routes and route sets](search.md#an-alternative) |
| attribute | One of the sixteen numbers that describe every alternative to the choice model: nine times in minutes, the number of transfers, the money in euros, the logarithm of the path size and four mode constants. | [Choice](choice.md#the-cost-of-an-alternative) |
| boarding slack | How long before a run of the timetable leaves a traveller must be at the stop to board it (`boarding_slack`, 60 s by default), at every boarding. | [Transit](transit.md#the-journey-search) |
| call | One stop of a run of the timetable, with the second the vehicle arrives there and the second it departs. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| capacity | The largest flow of cars the downstream end of a car link passes, in vehicles per hour. | [Networks](networks.md#capacity) |
| case | A folder of input files with a settings file, ready to run; five are published beside the package. | [The cases](../reference/cases.md) |
| choice | The one decision each traveller makes in a run: which alternative of its set to take, which settles its mode and its route at once. | [Choice](choice.md) |
| choice model | The rule that turns the costs of the alternatives of a set into the alternative taken: the least cost, or the logit. | [Choice](choice.md#the-two-models) |
| class (of travellers) | A group of travellers who weigh their alternatives in the same way, that is, who share one coefficient per attribute; it is nothing more than that. | [Demand](demand.md#traveller-classes) |
| coefficient | The weight a traveller class gives to one attribute in the cost of an alternative, in cost per unit of the attribute; positive is worse. | [Choice](choice.md#the-cost-of-an-alternative) |
| contract | The README of a module of the simulator, which states its inputs, outputs, units, assumptions and what it means for bikes. | [How a run works](architecture.md#inside-the-simulator) |
| contraction | In the OpenStreetMap reader, merging away a node that is no junction and joins two links of the same road class and the same car fields. | [Networks](networks.md#from-an-openstreetmap-extract) |
| cost | The choice model's sum, over the attributes of an alternative, of coefficient times attribute; it has no unit of its own, and the least cost is the best. | [Choice](choice.md#the-cost-of-an-alternative) |
| crossing time | The time a car takes on one link, from entering it to leaving it, the wait in its queue included. | [The loading](loading.md#what-the-loading-records) |
| delay over free flow | On the map and the summary, the mean crossing time of a link's cars over its car free-flow time, less 1 and never below 0. | [What a run gives](outputs.md#the-map-and-the-summary) |
| demand | The travellers of a run: a table of trips, read and never generated, each linked to the network and to the stops in walking reach. | [Demand](demand.md) |
| departure | The second of the clock at which a traveller leaves the origin door; it is an input and is never changed or chosen. | [Demand](demand.md#departure-times-and-the-window) |
| door | The position at which a trip starts (its origin) or ends (its destination), usually not on a node; travel times and costs are from door to door. | [Demand](demand.md#a-trip) |
| duality gap | Under the least-cost choice, the sum over the travellers of the cost of the alternative held less the least cost in the set, over the sum of the least costs; it is 0 when everyone holds the cheapest alternative of its set. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| egress list | Of a trip that may use transit, every stop from which the traveller can walk to the destination within `stop_walk_max`, each with its walking time. | [Demand](demand.md#linking-a-trip-end-to-stops) |
| egress walk | The walk of a transit trip from the last stop to its destination door. | [Transit](transit.md#walking-to-and-from-stops) |
| equilibrium | The state a run approaches, in which no traveller would choose otherwise at the travel times that the choices themselves produce. | [The iteration and the stopping rule](iteration.md) |
| fingerprint | A 64-bit number, written as 16 hexadecimal digits, computed from everything the network, the demand or the timetable was built from, so that a change to an input is seen. | [Reproducibility](reproducibility.md#the-fingerprints-of-the-inputs) |
| finished trip | A trip that is at its destination door before the window's end, and so has a travel time. | [The loading](loading.md#the-window-and-unfinished-trips) |
| first in, first out | The property that vehicles leave a link in the order in which they entered it. | [The loading](loading.md#cars-on-a-point-queue) |
| first wait | The time of a transit trip from the end of its access walk to the departure of the first run it boards, the boarding slack included. | [Transit](transit.md#the-cost-of-a-journey-and-the-wait-at-the-first-stop) |
| footpath | A walk of the timetable between two stops, at most `transfer_walk_max` long over the walk links, that a passenger may make to change from one ride to the next. | [Transit](transit.md#stops-runs-patterns-and-footpaths) |
| found | Said of an alternative the search adds to a set whose cost, without the path-size term, is strictly below that of every alternative of the same mode already in the set; the first alternative of a mode always counts. | [Routes and route sets](search.md#how-a-set-grows) |
| found share | The share of the trips with an alternative for which the search of an iteration found an alternative. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| free-flow speed | The speed of a mode on a link when nothing is in its way: the link's own car speed for a car, one speed for the whole network for a bike and for a walker. | [Networks](networks.md#free-flow-time) |
| free-flow time | The time a mode needs to cross a link when nothing is in its way: the link's length over the mode's free-flow speed. | [Networks](networks.md#free-flow-time) |
| gap (of a trip end) | The straight line between a door and the node its trip end is linked to, walked at the walk speed and on no link. | [Demand](demand.md#linking-a-trip-end-to-the-network) |
| GTFS feed | A timetable in the General Transit Feed Specification, the format in which transit agencies publish their timetables. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| headway (of a frequency) | In a GTFS feed, the time between two successive runs of a trip listed in `frequencies.txt`. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| headway (of a link) | The least time between two cars leaving the downstream end of a car link: 3 600 s over the link's capacity in vehicles per hour. | [The loading](loading.md#cars-on-a-point-queue) |
| inclusive value | In the nested logit, the logarithm of the sum over the alternatives of a nest of the exponential of minus cost over nest scale; also called the logsum. | [Choice](choice.md#the-logit-family) |
| iteration | One pass of the five stages search, choose, load, measure and assess, numbered from 0; also the module of the simulator that repeats them. | [The iteration and the stopping rule](iteration.md#the-five-stages) |
| iteration cap | The number of iterations after which a run always stops, whatever the stopping rule says (`iteration_cap`, 50 by default). | [The iteration and the stopping rule](iteration.md#the-stopping-rule) |
| iteration floor | The number of iterations before which the stopping rule is not read (`iteration_floor`, 0 by default). | [The iteration and the stopping rule](iteration.md#the-stopping-rule) |
| iterations table | The file `iterations.parquet`, one row per iteration: the mean trip time, the trip counts, the shares, the measure of the distance to equilibrium and the seconds each stage took. | [What a run gives](outputs.md#the-iterations-table) |
| journey | A trip's way through the timetable: from a stop of the access list, one to `max_rides` rides with at most one footpath between two of them, to a stop of the egress list. | [Transit](transit.md#the-journey-search) |
| kind of service | What a run of the timetable is (rail, metro, tram, bus, ferry or other), read from the feed's `route_type` and used by the map and the summary only. | [Transit](transit.md#the-kind-of-service) |
| largest strongly connected part | Of the links of one mode read from OpenStreetMap, those whose two ends lie in the largest set of nodes that all reach one another by that mode; the other links are left out. | [Networks](networks.md#from-an-openstreetmap-extract) |
| layer (of a network) | The links of one mode that the OpenStreetMap reader builds: a car layer, a bike layer and a walk layer, put together in the one network. | [Networks](networks.md#from-an-openstreetmap-extract) |
| least-cost choice | The choice model `"deterministic"`, under which a traveller takes the alternative of least cost in its set. | [Choice](choice.md#the-least-cost) |
| leg | One part of the way a trip went, a row of the routes table: a walk off the network, a route on the links by car, by bike or on foot, a ride, or a transfer walk. | [Output files](../reference/outputs.md#the-kinds-of-leg) |
| link | A directed connection from one node to another, with a length, the modes allowed on it and, for cars, lanes, a free-flow speed and a capacity; a street open in both directions is two links. | [Networks](networks.md#nodes-and-links) |
| link bin | One of the periods of `link_bin` seconds (300 s by default) into which the window is cut for the links table, also called a time bin; a vehicle is counted in the bin in which it entered the link. | [The loading](loading.md#what-the-loading-records) |
| link time reading | The time a traveller entering a link at a given instant takes to cross it, obtained by applying the point queue's rule to the car crossings the last loading recorded; for a bike and a walker it is the free-flow time. | [The loading](loading.md#the-link-time-reading) |
| links table | As an input, the table of the network's links; as a result, the file `links.parquet`, one row per link and link bin that someone entered. | [What a run gives](outputs.md#the-links-table) |
| loading | The part of the simulator that takes the route each traveller has chosen and works out when each car, bike and walker passes each link of it; it never chooses. | [The loading](loading.md) |
| logit | The choice model `"logit"`: a nested logit with one nest per mode and one nest scale, under which a traveller takes an alternative with a probability that falls as its cost rises. | [Choice](choice.md#the-logit-family) |
| map | A page of a finished run, one HTML file that shows the traffic per link and link bin, the cyclists, the walkers, transit and the journeys between the busiest pairs of places. | [What a run gives](outputs.md#the-map-and-the-summary) |
| mean trip time | The mean door-to-door time of the trips that finished within the window, in seconds. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| method of successive averages | The classical remedy against all travellers moving at once: at iteration *k* only a share 1/(*k*+1) is moved, a share that shrinks so that the moves die out. | [The iteration and the stopping rule](iteration.md#who-is-offered-a-new-choice) |
| mode | One of the four ways of travelling, car, bike, walk and transit, none ranked above another. | [Networks](networks.md#one-network-for-car-bike-and-walk) |
| mode constant | An attribute that is 1 for the alternatives of one mode and 0 for the others, so that its coefficient is a cost of taking that mode at all. | [Choice](choice.md#the-cost-of-an-alternative) |
| mode set | The modes among car, bike, walk and transit that are open to a trip, or allowed on a link (car, bike and walk only), written as names joined by `;`. | [Demand](demand.md#the-modes-open-to-a-trip) |
| module | One part of the simulator with its own contract (a Rust crate), which uses only the modules below it. | [How a run works](architecture.md#inside-the-simulator) |
| nest | Under the logit, the alternatives of one mode in a traveller's set, treated as more alike than alternatives of different modes. | [Choice](choice.md#the-logit-family) |
| nest scale | The number μ, above 0 and at most 1, that says how much alike the alternatives of one nest are (`nest_scale`, 0.5 by default); at 1 the model is the plain logit. | [Choice](choice.md#the-logit-family) |
| network | What cars, bikes and walkers move on: a set of nodes and links, one network for the three modes. | [Networks](networks.md) |
| node | A point of the network, with an identifier and a position, at which links start and end. | [Networks](networks.md#nodes-and-links) |
| offered share | The share of the trips with an alternative that were offered a new choice in an iteration; 1 at iteration 0, where everyone chooses. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| OpenStreetMap extract | A `.osm.pbf` file of OpenStreetMap data, from which a network can be read. | [Networks](networks.md#from-an-openstreetmap-extract) |
| origin-destination pair | The trips with the same car origin node and the same car destination node, whose distinct car routes `max_car_routes` limits. | [Routes and route sets](search.md#the-limit-on-car-routes-per-origin-destination-pair) |
| outside answer | The expected value of a test, taken from theory, a derivation by hand, a published result or another tool, and never from the simulator's own output. | [Validation](validation.md) |
| pair (of places) | On the map and the summary, the trips with the same origin door and the same destination door. | [What a run gives](outputs.md#the-map-and-the-summary) |
| parameter | A named value of a run with a unit, a default and a source, given as a keyword in Python or as a key of the settings file. | [Parameters](../reference/parameters.md) |
| path size | A number between 1/*n* and 1 that measures how much of a car route is its own among the *n* car routes of a traveller's set: 1 for a route that shares no link, 1/*n* for each of *n* identical routes. | [Routes and route sets](search.md#the-path-size-of-overlapping-car-routes) |
| path-size logit | The logit with a term, the coefficient times the logarithm of the path size, added to the cost of a car route, so that overlapping routes are not counted as separate options. | [Choice](choice.md#the-logit-family) |
| pattern | The runs of the timetable that serve the same stops in the same order and never overtake one another. | [Transit](transit.md#stops-runs-patterns-and-footpaths) |
| point queue | The rule by which cars are loaded: a car crosses a link at free flow and leaves its downstream end no sooner than one headway after the car before it, in a queue that takes no room on the road. | [The loading](loading.md#cars-on-a-point-queue) |
| record | The file `run_record.toml` that every run writes: part one holds the settings the run ran with, every default filled in, and part two what the run found and did. | [What a run gives](outputs.md#the-run-record) |
| refusal | A Python exception, raised only when what was asked cannot be done at all. | [Repairs, warnings and refusals](../reference/warnings.md#refusals) |
| region | A box or a polygon in WGS84 degrees that limits the part of an OpenStreetMap extract that is read. | [Networks](networks.md#from-an-openstreetmap-extract) |
| repair | A faulty row or value of an input that is set right or left out instead of refused, counted under a key by kind and shown as a warning. | [Repairs, warnings and refusals](../reference/warnings.md#repairs) |
| ride | One stretch of a journey on one run of the timetable, from the stop boarded to the stop left. | [Transit](transit.md#the-journey-search) |
| road class | A name of a link such as `motorway`, `residential` or `cycleway`: the OpenStreetMap reader takes missing speeds, lanes and capacities from it, and otherwise it only serves to draw the link. | [Networks](networks.md#shapes-and-road-classes-for-drawing-only) |
| route | A chain of links, each allowed to the mode, the downstream node of one being the upstream node of the next, from an origin node to a destination node. | [Routes and route sets](search.md#the-fastest-route) |
| routes table | The file `routes.parquet`, one row per leg of what each trip took at the last iteration. | [What a run gives](outputs.md#the-routes-table) |
| run (of a transit vehicle) | One vehicle trip of the timetable on the service day, with its calls in order. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| run (of the simulator) | One execution of the simulator on a network, a demand and, if any, a timetable over a window: the iterations up to the stopping rule or the cap, and the five files written from the last of them. | [How a run works](architecture.md#interactions-in-a-run) |
| second of the clock | A time of day as a number of seconds after midnight of the run's day (28 800 is 08:00). | [What a run gives](outputs.md#conventions-of-the-tables) |
| seed | The whole number (`seed`, 0 by default) from which every random number of a run follows: the travellers' tastes under the logit, and who is offered a new choice at each iteration. | [Reproducibility](reproducibility.md#the-seed-and-the-random-streams) |
| service day | The one day of a GTFS feed whose timetable is simulated: the busiest weekday of the feed. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| set | The alternatives the search has found for one traveller so far; it only grows, and each alternative keeps its position in it for the whole run. | [Routes and route sets](search.md#how-a-set-grows) |
| settings file | The file `run.toml`, which names the window, the parameters, the input files and the classes of a run for `run_file`. | [The settings file and the run record](../reference/record.md) |
| shape | The points between the two ends of a link in its direction of travel, used to draw the link and for nothing else. | [Networks](networks.md#shapes-and-road-classes-for-drawing-only) |
| share who would change | Under the logit, the share of the trips with an alternative for which the model, choosing again over their own set at the costs the iteration measured, would take another alternative than the one they hold. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| signal | A mark on a node, and on every link whose downstream node carries it, that the OpenStreetMap reader stores and that nothing reads. | [Networks](networks.md#signals-read-and-stored-applied-nowhere) |
| spillback | A queue reaching back from its link onto the links upstream, which the point queue does not represent. | [The loading](loading.md#cars-on-a-point-queue) |
| stage | One of the five steps of an iteration: search, choose, load, measure, assess. | [The iteration and the stopping rule](iteration.md#the-five-stages) |
| step exponent | The exponent *a* of the share of travellers offered a new choice at iteration *k* under the logit, 1/(*k*+1)^*a* (`step_exponent`, 0.75 by default). | [The iteration and the stopping rule](iteration.md#under-the-logit) |
| stop | A point of the timetable, with a position, where a traveller boards or leaves a run. | [Transit](transit.md#one-service-day-from-a-gtfs-feed) |
| stopping rule | The rule that ends a run before the iteration cap: the found share is at most 0.5 %, and over the last five iterations the mean trip time varies by at most 0.3 %. | [The iteration and the stopping rule](iteration.md#the-stopping-rule) |
| summary | A page of a finished run, one HTML file that shows the run's numbers: trips completed and not, the modes' shares and mean trip times, the busiest links, the iterations and the record. | [What a run gives](outputs.md#the-map-and-the-summary) |
| switch share | The share of the trips with an alternative that changed alternative in the choice of an iteration. | [The iteration and the stopping rule](iteration.md#what-each-iteration-reports) |
| taste | Under the logit, the random part of a traveller's utility, one draw per alternative and per mode, drawn once from the seed and kept for the whole run. | [Choice](choice.md#how-each-traveller-takes-one-alternative) |
| time-only run | A run whose coefficients are 1 on each of the nine times and 0 on the rest, so that the cost of an alternative is its door-to-door time in minutes. | [Choice](choice.md#the-cost-of-an-alternative) |
| timetable | The stops, runs and footpaths of one service day, on which transit journeys are found. | [Transit](transit.md#stops-runs-patterns-and-footpaths) |
| transfer | A change from one ride of a journey to the next; a journey has one transfer fewer than it has rides. | [Transit](transit.md#the-journey-search) |
| transfer walk | The leg of a transit trip that walks a footpath between two rides, where the next ride leaves from another stop. | [Output files](../reference/outputs.md#the-kinds-of-leg) |
| transit | Scheduled public transport (trains, metros, trams, buses and ferries), which runs on its timetable and not on the links of the network. | [Transit](transit.md) |
| traveller | The person making a trip; every trip is its own traveller, chooses on its own and is counted once. | [Demand](demand.md#a-trip) |
| trip | One movement of one traveller from an origin to a destination, door to door, with a departure, a class and a mode set. | [Demand](demand.md#a-trip) |
| trips table | As an input, the table of the demand, one row per trip; as a result, the file `trips.parquet`, one row per trip that was run. | [What a run gives](outputs.md#the-trips-table) |
| unfinished trip | A trip that took an alternative and is not at its destination door when the window ends; it has no travel time. | [The loading](loading.md#the-window-and-unfinished-trips) |
| upstream end, downstream end | The two ends of a link: it runs from its upstream node to its downstream node, and cars queue at its downstream end. | [Networks](networks.md#nodes-and-links) |
| warning | A Python `UserWarning` that reports a repair, or something else set aside or ignored, after which the work goes on. | [Repairs, warnings and refusals](../reference/warnings.md) |
| window | The period a run simulates, two seconds of the clock `(start_s, end_s)` given by the user and never defaulted, from its start up to but not including its end. | [Demand](demand.md#departure-times-and-the-window) |
| without an alternative | Said of a trip that no mode open to it can make: it is counted, not simulated. | [What a run gives](outputs.md#the-trips-table) |
