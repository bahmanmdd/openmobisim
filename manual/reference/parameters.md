# Parameters of a run

% src: PARAMETERS.md (every row, in its order); crates/binding/README.md ("Keywords"); the defaults read from the record of a run of the case dotnet with no parameter given

Every parameter of openmobisim is a row of the table below: there are nineteen, and a parameter that is not in the
table does not exist. Each has a unit, a default and the source of that default. A parameter is given as a keyword in
Python or as a key of the settings file `run.toml` ([Where each parameter is given](#where-each-parameter-is-given)),
except `osm_class_defaults`, whose table is fixed and takes no other value; and every run writes the value it used into
its record, except the fixed road-class table (and, in a run without transit, the two walk limits to stops)
([The settings file and the run record](record.md)). The works named in the last column are listed in full in the
[Bibliography](bibliography.md).

| Parameter | Unit | Default | What it does | Source of the default |
|---|---|---|---|---|
| `capacity_per_lane` | vehicles per hour per lane | 1 800 | The capacity of one lane of a car link whose row in the links table gives no capacity: the link's capacity is its lanes times this value. A network read from OpenStreetMap does not use it, because every car link there takes the capacity of its road class (`osm_class_defaults`). | Transportation Research Board (2022), *Highway Capacity Manual*, 7th edition: the base saturation flow per lane |
| `bike_speed` | km/h | 15 | The one speed at which every bike rides every bike link. | de Hartog, Boogaard, Nijland and Hoek (2010): the urban cycling speed in the Netherlands, stops included |
| `walk_speed` | km/h | 4.8 | The one speed of every walker: on the walk links, between a door and the network, to and from a stop and between two stops. | Knoblauch, Pietrucha and Nitzburg (1996): measured mean walking speeds |
| `stop_walk_max` | m | 1 500 | The longest walk between a door and a transit stop. A trip with no stop within it at either end is offered no transit journey and is counted (`without_stop` in the iterations table). | El-Geneidy, Grimsrud, Wasfi, Tétreault and Surprenant-Legault (2014): the 85th percentile of the walk to a stop is 524 m for buses and 1 259 m for commuter rail |
| `transfer_walk_max` | m | 400 | The longest footpath between two stops, which a traveller walks to change from one ride to the next. | Guerra, Cervero and Tischler (2012): the quarter-mile walking standard of transit |
| `boarding_slack` | s | 60 | The least time between reaching a stop and boarding a vehicle there, at the first boarding and at every transfer. | `min_transfer_time` of `transfers.txt` in common GTFS feeds; Delling, Pajor and Werneck (2015): the minimum change time |
| `max_rides` | rides | 4 | The most vehicles a transit journey may ride, so the most transfers is one less. | Delling, Pajor and Werneck (2015): rounds of the search beyond four rarely improve the arrival |
| `nearby_stops` | searches per trip end | 3 | The searches for the earliest transit journeys at each end of a trip. The first search is between all the stops within walking reach; each further one starts from one of the next nearest stops of the origin alone, or ends at one of the next nearest stops of the destination alone, and so finds journeys with less walking. At 1 the first search alone is made. | a choice of this software |
| `max_car_routes` | routes per origin-destination pair | 6 | The most car routes that the trips with the same origin node and destination node gather over the iterations. A route new to a pair at its limit is not offered, and none is replaced; a traveller's first car route is always offered. | a choice of this software |
| `max_transit_journeys` | journeys per trip | 10 | The most transit journeys offered to a traveller: of the journeys the search finds, the cheapest for the traveller are kept. | a choice of this software |
| `choice` | name | `"logit"` | The choice model: `"logit"`, a nested logit with one nest per mode and a path-size logit over the car routes, or `"deterministic"`, the least-cost choice. | a choice of this software |
| `coefficients` | per traveller class, sixteen numbers ([below](#the-sixteen-coefficients)) | the values [below](#the-sixteen-coefficients), for every class | The weights that turn the parts of an alternative (its minutes by kind, its transfers, its money, its path size, its mode) into one cost. They are starting values, to be calibrated for each study. | the value of time: KiM (2023); the path size: Ben-Akiva and Bierlaire (1999); the rest a choice of this software |
| `nest_scale` | none | 0.5 | The scale of every nest of the nested logit, in (0, 1]: at 1 the model is the plain logit over all alternatives, and below 1 the alternatives of one mode are more alike to the traveller than those of different modes. Read under the logit only. | Ben-Akiva and Lerman (1985), chapter 10: a mid-range nest scale |
| `step_exponent` | none | 0.75 | The exponent a of the share of travellers who are offered a new choice at iteration k under the logit, 1 / (k + 1)^a. At 1 it is the plain method of successive averages. Read under the logit only. | Robbins and Monro (1951): the averaging settles for a in (0.5, 1]; Sheffi (1985): the method of successive averages |
| `iteration_cap` | iterations | 50 | The most iterations a run makes: a run that the stopping rule has not ended by then ends there. | a choice of this software |
| `iteration_floor` | iterations | 0 | The iterations before which the stopping rule is not read. A floor equal to the cap runs a fixed number of iterations. | a choice of this software |
| `link_bin` | s | 300 | The length of the time bin in which the traffic of each link is reported. The window is a whole number of bins. | a choice of this software |
| `seed` | none | 0 | The seed of every random draw of the run, a whole number from 0 to 2⁶³ − 1. The same inputs, settings and seed give the same results. | a choice of this software |
| `osm_class_defaults` | km/h, lanes per direction, vehicles per hour per lane | the table [below](#the-road-classes-of-an-openstreetmap-network) | For a network read from OpenStreetMap: by road class, the free-flow speed of cars and the lanes per direction where no tag gives them, and the capacity of one lane. The table is fixed. | the sources [below](#the-road-classes-of-an-openstreetmap-network) |

**For bikes.** `bike_speed` is the one parameter of the bike network: bikes ride at it on every bike link and meet no
congestion. The coefficients `bike_ride_min` and `is_bike` weigh the bike in the choice.

## The sixteen coefficients

% src: PARAMETERS.md (coefficients); crates/loop/README.md ("Choose"); the run.toml of the case delft; the record of a run of dotnet with no coefficient given

Each traveller class has sixteen coefficients, and every class takes the defaults below unless others are given. A
coefficient is a cost, so a positive value is worse. What each one multiplies is in
[the coefficients of a class](record.md#the-coefficients-of-a-class); how the costs enter the choice is in
[Choice](../concepts/choice.md).

| Coefficient | Unit | Default | Source of the default |
|---|---|---|---|
| `car_drive_min` | cost per minute | 0.1 | a choice of this software |
| `bike_ride_min` | cost per minute | 0.1 | as `car_drive_min` |
| `walk_min` | cost per minute | 0.165 | a choice of this software: a minute walked weighs 1.65 times a minute driven |
| `transit_in_vehicle_min` | cost per minute | 0.1 | as `car_drive_min` |
| `transit_access_min` | cost per minute | 0.165 | as `walk_min` |
| `transit_egress_min` | cost per minute | 0.165 | as `walk_min` |
| `transit_first_wait_min` | cost per minute | 0.145 | a choice of this software: a minute waited weighs 1.45 times a minute ridden |
| `transit_transfer_wait_min` | cost per minute | 0.145 | as `transit_first_wait_min` |
| `transit_transfer_walk_min` | cost per minute | 0.165 | as `walk_min` |
| `transit_transfers` | cost per transfer | 0.5 | a choice of this software: a transfer weighs as five minutes ridden |
| `money_eur` | cost per euro | 0.6 | KiM (2023): a value of time of 10 € per hour, at 0.1 per minute |
| `car_ln_path_size` | cost per unit of the logarithm of the path size | −1 | Ben-Akiva and Bierlaire (1999): the path-size logit, in which n identical copies of a route count as the one route |
| `is_car` | cost per car alternative | 0 | a choice of this software |
| `is_bike` | cost per bike alternative | 0 | a choice of this software |
| `is_walk` | cost per walk alternative | 0.5 | a choice of this software |
| `is_transit` | cost per transit alternative | 0.25 | a choice of this software |

The coefficient `car_ln_path_size` is recorded as given. The logit divides a whole cost by the nest scale inside a
nest, so it reads this coefficient times `nest_scale` (−0.5 at the defaults), which keeps the exponent of the path
size at 1 at every nest scale. The least-cost choice reads it as 0, since the term corrects the logit.

## The road classes of an OpenStreetMap network

% src: crates/io-osm/README.md ("The class table"); PARAMETERS.md (osm_class_defaults)

A network read from an OpenStreetMap extract takes, by the road class of each way (its `highway` tag), the free-flow
speed of cars and the lanes per direction where no tag gives them, and the capacity of one lane. A car link's capacity
is its lanes times the capacity per lane of its class. The reader models no junction, so the capacity per lane counts
the junctions of the class in.

| `highway` | Car speed (km/h) | Car lanes per direction | Green share (none) | Capacity per lane (vehicles per hour) |
|---|---|---|---|---|
| `motorway` | 110 | 2 | no junction | 2 000 |
| `motorway_link` | 60 | 1 | ramp | 1 550 |
| `trunk` | 90 | 2 | no junction | 1 800 |
| `trunk_link` | 50 | 1 | ramp | 1 550 |
| `primary` | 60 | 2 | 0.60 | 1 080 |
| `primary_link` | 40 | 1 | 0.50 | 900 |
| `secondary` | 50 | 1 | 0.50 | 900 |
| `secondary_link` | 40 | 1 | 0.50 | 900 |
| `tertiary` | 50 | 1 | 0.45 | 810 |
| `tertiary_link` | 30 | 1 | 0.40 | 720 |
| `unclassified`, `road` | 40 | 1 | 0.40 | 720 |
| `residential` | 30 | 1 | 1/3 | 600 |
| `living_street` | 20 | 1 | 0.25 | 450 |
| `service` | 20 | 1 | 0.25 | 450 |
| `pedestrian`, `footway`, `cycleway`, `busway`, `ferry` | no car link | no car link | no car link | no car link |

The sources of the table:

| Values | Source |
|---|---|
| Speeds and lanes | Common conventions for OpenStreetMap road classes; a choice of this software. A speed limit or a lane count tagged on a way replaces them. |
| Capacity per lane of a motorway, 2 000 | ICTAVRU, the French instruction for urban expressways. |
| Capacity per lane of a trunk road, 1 800, and of a ramp, 1 550 | Cerema, *Fondamentaux de la conception routière*. |
| Capacity per lane of a class with junctions at grade | The capacity of a signalised approach lane, the saturation flow times the green time over the cycle (Cerema 2016, *Évaluation de la capacité aux feux*), with a saturation flow of 1 800 vehicles per hour per lane. |
| Green share of each class | A choice of this software: the guideline gives the formula, not a table by class. |

**For bikes.** The table gives nothing to bikes: a bike link has no lanes and no capacity, and every bike rides at
`bike_speed`.

## Where each parameter is given

% src: crates/binding/README.md ("Keywords")
A parameter is a keyword of the entry point whose module reads it, or a key of `[parameters]` in the settings file
`run.toml`, which gives each to the right entry point ([The settings file and the record](record.md)).

| Entry point | Parameters |
|---|---|
| `Network.from_osm`, `Network.from_tables` | `capacity_per_lane`, `bike_speed`, `walk_speed` |
| `Network.from_osm` alone | `osm_class_defaults` |
| `Transit.from_gtfs` | `transfer_walk_max` |
| `Demand.from_table` | `stop_walk_max` |
| `run` | `boarding_slack`, `max_rides`, `nearby_stops`, `max_car_routes`, `max_transit_journeys`, `choice`, `coefficients`, `nest_scale`, `step_exponent`, `iteration_cap`, `iteration_floor`, `link_bin`, `seed` |
