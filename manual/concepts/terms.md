# Terms

% Merged from the writers' term lists (concepts and reference pages); each term links to the page that defines it.

The terms this manual uses, each in one sentence, with the page that explains it.

| Term | Meaning | Explained in |
|---|---|---|
| access point | A layer and a node of it at which a hub is entered or left. | [modes](../concepts/modes.md) |
| alternative | One option a trip may take: a route, or a door-to-door itinerary. | [choice](../concepts/choice.md) |
| at-grade road | Any road with junctions and crossings: every class but motorways, trunk roads and their slip roads. | [parameters](../reference/parameters.md) |
| attribute | A named number every alternative carries (its time, its cost, …) that a model weighs. | [choice](../concepts/choice.md) |
| backward wave speed | The speed at which the back of a queue moves upstream as it clears; derived here. | [networks](../concepts/networks.md) |
| boarding slack | How long before a run leaves a passenger must be at the stop to board it. | [transit](../concepts/transit.md) |
| busway | A road open to buses only. | [networks](../concepts/networks.md) |
| capacity | The most vehicles a link passes per hour. | [networks](../concepts/networks.md) |
| choice model | What decides which alternative each trip takes. | [choice](../concepts/choice.md) |
| choice set | The alternatives a traveller is offered at one choice. | [choice](../concepts/choice.md) |
| choice set (offered routes) | The routes of a set a traveller is offered at their departure, within the detour and count limits. | [route sets](../concepts/route_sets.md) |
| coefficient | The weight of one attribute in the utility, `beta_<attribute>`. | [choice](../concepts/choice.md) |
| common random numbers | The same random errors for the same alternatives across scenarios, so scenarios differ only where they differ. | [choice](../concepts/choice.md) |
| contraction | Merging chains of links joined at nodes that are not junctions. | [networks](../concepts/networks.md) |
| control delay | The average wait at a signal, added to the free-flow time of the link that ends there. | [networks](../concepts/networks.md) |
| critical density | The density at which a link carries its capacity: capacity over free-flow speed. | [networks](../concepts/networks.md) |
| demand, trip, traveller, weight | The trips; one journey from an origin to a destination; who makes them; how many people a traveller stands for. | [demand](../concepts/demand.md) |
| disequilibrium | The gap less the expected gap: what the choice model does not explain. | [equilibration](../concepts/equilibration.md) |
| en-route rerouting | A vehicle stuck at the front of its link taking a new route from where it is. | [loading](../concepts/loading.md) |
| equilibration | Repeating choice and loading until the choices agree with the times they produce. | [equilibration](../concepts/equilibration.md) |
| equilibrium | The state in which, given the choice model, nobody gains by choosing differently. | [equilibration](../concepts/equilibration.md) |
| expected gap | The gap a converged run has by design under a random choice model. | [equilibration](../concepts/equilibration.md) |
| fallback car | The car taken by a trip that had no alternative at all. | [modes](../concepts/modes.md) |
| fast run, complete run | A starter case run with its own traveller weight, or with one traveller per person. | [demand](../concepts/demand.md) |
| flow level | How much of traffic's physics the loading represents: 0 free flow, 2 point queue, 3 spatial queue, 4 full model. | [loading](../concepts/loading.md) |
| free-flow loading | A run's first loading (iteration 0), built up in groups each choosing on the congestion of those before it. | [equilibration](../concepts/equilibration.md) |
| free-flow speed | The speed on an empty link. | [networks](../concepts/networks.md) |
| free-flow time | A link's length over its free-flow speed, plus a signal's control delay. | [networks](../concepts/networks.md) |
| fundamental diagram | The relation between how densely vehicles occupy a road and how many pass per hour; triangular here. | [networks](../concepts/networks.md) |
| gap | How much more the chosen routes cost than the least-cost route of the choice set, relative to it. | [equilibration](../concepts/equilibration.md) |
| green fraction | The share of a signal's cycle an approach has green. | [networks](../concepts/networks.md) |
| gridlock | A closed loop of full links whose front vehicles wait on one another, so none can move. | [loading](../concepts/loading.md) |
| Gumbel-max construction | Taking the alternative with the largest utility plus a standard Gumbel error: an exact draw from the logit. | [choice](../concepts/choice.md) |
| hub | A place where a traveller changes layer (a stop, a parking), with access points and transfers. | [modes](../concepts/modes.md) |
| hub expectation mismatch | The difference between the time to park expected at the choice and the time paid. | [parking](../concepts/parking.md) |
| identity (of an alternative) | A hash that names an alternative whatever else is offered, so its random draws stay the same. | [choice](../concepts/choice.md) |
| incremental assignment | Assigning demand in parts, each on the times the parts before it produced. | [equilibration](../concepts/equilibration.md) |
| itinerary | A door-to-door alternative of a transit, park-and-ride or bike-and-ride trip. | [modes](../concepts/modes.md) |
| jam density | The most vehicles a link holds per kilometre, standing still. | [networks](../concepts/networks.md) |
| layer | A network a kind of traveller moves on: road, bike, walk, transit. | [networks](../concepts/networks.md) |
| link transmission model | The loading model of levels 2–4, which tracks what has entered and left each link. | [loading](../concepts/loading.md) |
| loading | Moving every car and bus along its route through the window, finding when each reaches each link. | [loading](../concepts/loading.md) |
| loading step | The bookkeeping interval of the loading (`flow_step_s`); results do not depend on it. | [loading](../concepts/loading.md) |
| logsum | The expected utility of a situation's best alternative, up to a constant; utility-based accessibility. | [choice](../concepts/choice.md) |
| method of successive averages (MSA) | Letting a shrinking share, 1/(i+1), of travellers choose again at iteration i. | [equilibration](../concepts/equilibration.md) |
| mixed traffic / lane / separated | The three levels of cycling infrastructure. | [networks](../concepts/networks.md) |
| mode | How a trip is made: car, bike, walk, transit, park-and-ride (`car_transit`), bike-and-ride (`bike_transit`). | [modes](../concepts/modes.md) |
| mode constant | The coefficient of `mode_<mode>`: a fixed bonus or penalty for one mode. | [choice](../concepts/choice.md) |
| nest scale (mu) | How close substitutes the alternatives of one nest are in a nested logit; 1 is the flat logit. | [parameters](../reference/parameters.md) |
| nest, inclusive value | In the nested logit, a group of alternatives (one mode's), and the log-sum of its scaled utilities. | [choice](../concepts/choice.md) |
| network | What travellers move on: one road network and the layers built over it. | [networks](../concepts/networks.md) |
| network gap | The gap against the least-cost route over the whole network, not only the choice set. | [equilibration](../concepts/equilibration.md) |
| network option | A number a network is built with, named and overridable (`network_options`). | [networks](../concepts/networks.md) |
| OD matrix | The number of trips from each origin zone to each destination zone. | [demand](../concepts/demand.md) |
| origin queue | Where a departing vehicle waits, outside the network, for room on its first link. | [loading](../concepts/loading.md) |
| origin-destination pair | An origin and a destination, here the nodes a trip's ends snap to. | [route sets](../concepts/route_sets.md) |
| overlap | The largest share of a route's cost on links it shares with one other route of its set. | [route sets](../concepts/route_sets.md) |
| Pareto set (of journeys) | For each number of vehicles, the earliest arrival that beats every journey with fewer. | [transit](../concepts/transit.md) |
| parking, capacity, availability | A car park or bike parking; how many vehicles it holds; the share of a time bin it had a free space. | [parking](../concepts/parking.md) |
| passenger car unit (PCU) | A vehicle's size in cars: a car is 1, a bus `bus_pcu`. | [outputs](../reference/outputs.md) |
| path size | How much of a route (or itinerary) is its own, not shared with the other alternatives; 1 for one that shares nothing. | [choice](../concepts/choice.md) |
| pattern | The runs of one line calling at the same stops with the same boarding rules, none overtaking another. | [transit](../concepts/transit.md) |
| PCU (passenger car unit) | How much road a vehicle takes: 1 for a car, 2 for a bus by default, times the people a simulated traveller stands for. | [loading](../concepts/loading.md) |
| point queue | A loading where queues form behind capacity but take no road space. | [loading](../concepts/loading.md) |
| product term | A coefficient on the product of two attributes, `beta_<a>*<b>`. | [choice](../concepts/choice.md) |
| propensity to congest | For a link, the busiest half-hour's load on shortest routes over its capacity, clipped at 1. | [route sets](../concepts/route_sets.md) |
| RAPTOR, round | Round-based public transit routing; round k finds the earliest arrivals with at most k vehicles. | [transit](../concepts/transit.md) |
| road class | A link's kind, from OpenStreetMap's `highway` tag (`motorway`, `residential`, …), which sets its defaults. | [networks](../concepts/networks.md) |
| route cache | Sets kept in memory for later runs of the same process with the same inputs. | [route sets](../concepts/route_sets.md) |
| route set | The routes a car trip between one origin and one destination may take. | [route sets](../concepts/route_sets.md) |
| route update | Adding routes to the sets between choices, found on congested times. | [route sets](../concepts/route_sets.md) |
| run fingerprint | A hash of every input that can change a run's results. | [outputs](../reference/outputs.md) |
| sample (of the demand), aggregation | One traveller kept in every `weight`, standing for that many people and travelling as one group. | [demand](../concepts/demand.md) |
| saturation flow | The rate at which a lane discharges a standing queue, per hour (of green, at a signal). | [networks](../concepts/networks.md) |
| service day, run, call | One day of a timetable; one vehicle's trip on it; its stop at one stop (arrival and departure). | [transit](../concepts/transit.md) |
| simulation window | The period a run simulates, from its start to its end. | [loading](../concepts/loading.md) |
| situation | One traveller's one trip at one iteration, with the alternatives offered to it. | [choice](../concepts/choice.md) |
| snapping | Placing a trip's end on the nearest street node (one with a road a car can enter from; a motorway-only node only when the point lies within 50 m of it). | [route sets](../concepts/route_sets.md) |
| soft capacity | A full parking refuses nobody: parking takes longer and the overflow is counted. | [parking](../concepts/parking.md) |
| spatial queue | A loading where queues fill the road and spill back, with room freed at once. | [loading](../concepts/loading.md) |
| spillback | A queue backing up from a link into the links upstream. | [loading](../concepts/loading.md) |
| stated mode / mode choice | A mode given in the demand, kept; or chosen in the run among the modes offered. | [modes](../concepts/modes.md) |
| static layer | A layer whose link times are fixed (bike, walk): nothing on it congests. | [networks](../concepts/networks.md) |
| storage | The vehicles a link holds standing: jam density times length. | [networks](../concepts/networks.md) |
| straddling | A vehicle counted on two links at once while its front has moved on. | [loading](../concepts/loading.md) |
| strongly connected | Every node can reach every other. | [networks](../concepts/networks.md) |
| the chain | Each trip of a traveller starts where the one before ended. | [demand](../concepts/demand.md) |
| traveller class | Travellers who own the same things, may use the same modes and weigh alike. | [modes](../concepts/modes.md) |
| traveller weight | How many people a simulated traveller stands for. | [outputs](../reference/outputs.md) |
| turn | A pair of links, in and out, at a node. | [networks](../concepts/networks.md) |
| turn pocket | Room at the end of a multi-lane approach where vehicles waiting for one movement do not block others. | [loading](../concepts/loading.md) |
| turnaround | A short link a clip adds to join a cut carriageway to the opposite one. | [networks](../concepts/networks.md) |
| unfinished trip | A trip still under way when the window ends; counted and reported. | [loading](../concepts/loading.md) |
| utility | The weighted sum of an alternative's attributes; the larger, the more likely the alternative. | [choice](../concepts/choice.md) |
| value of time | The euros a traveller would pay to save an hour: `beta_time_min · 60 / beta_cost_eur`. | [choice](../concepts/choice.md) |
| warm-up | The first loadings made with the point queue, whatever the flow level. | [equilibration](../concepts/equilibration.md) |
