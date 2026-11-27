# Traveller classes and choice coefficients

% src: crates/binding/README.md ("Inputs": `trips`, `classes`, keywords of `run`); crates/loop/README.md ("Choose")

Every traveller belongs to a **traveller class**, and every class has its own **choice coefficients**: the weights
with which its travellers compare a minute in a car, a minute on a bike, a wait at a stop and so on. This page shows
what a class is, how the modes open to a traveller are given, and how to give the coefficients.

## What a class is

% src: crates/binding/README.md ("Inputs": `trips`, `classes`); crates/demand/README.md ("Inputs", "Repairs")

A class is a name with a row of sixteen coefficients, nothing more:

| Where | What is written there |
|---|---|
| The trips table, column `class_id` | the class of each traveller |
| The list `classes` given to `Demand.from_table` (in a settings file, `[demand] classes`) | the classes of the run |
| The keyword `coefficients` of `run` (in a settings file, one table per class) | the sixteen coefficients of each class |

A trip naming a class that is not in the list is dropped, counted and told in a warning.

## The modes open to a trip

% src: crates/binding/README.md ("Inputs": a mode set); crates/demand/README.md ("Repairs")

The modes a traveller may use are **the trip's own**, written in the `modes` column of the trips table as one string
of mode names joined by `;`, each of `car`, `bike`, `walk`, `transit` (`car;bike;walk;transit`). They are not a
property of the class: two travellers of one class may have different modes. In the published cases the two go
together by construction, for example every traveller of the class `bike_only` has `bike;walk;transit`
([The cases](cases.md)).

A mode that no link of the network allows is taken out of a trip's set, and a trip left with no mode is dropped; both
are counted. A traveller with `transit` in its modes and no stop within `stop_walk_max` of an end is offered no
journey, and is counted in the column `without_stop` of the iterations table.

## The sixteen coefficients

% src: crates/loop/README.md ("Units", "Choose"); PARAMETERS.md (`coefficients`); crates/choice/README.md ("Units")

Each alternative of a traveller (a car route, a bike route, a walk, a transit journey) is described by sixteen numbers
$x_k$, and its cost for a traveller of class $c$ is

$$
\text{cost} = \sum_{k=1}^{16} \beta_{c,k}\, x_k
$$

with $\beta_{c,k}$ the class's coefficients. A cost is what the traveller wants little of: a positive coefficient
makes more of its attribute worse. An alternative fills only the columns of its own mode, and the nine times of any
alternative add up to its door-to-door time.

| Position | Name | What it multiplies | Default |
|---|---|---|---|
| 1 | `car_drive_min` | minutes driving | 0.1 |
| 2 | `bike_ride_min` | minutes riding a bike | 0.1 |
| 3 | `walk_min` | minutes walking: a walk trip, and the walks of a car or bike trip between a door and the network | 0.165 |
| 4 | `transit_in_vehicle_min` | minutes in a transit vehicle | 0.1 |
| 5 | `transit_access_min` | minutes walking to the first stop | 0.165 |
| 6 | `transit_egress_min` | minutes walking from the last stop | 0.165 |
| 7 | `transit_first_wait_min` | minutes from reaching the first stop to the first boarding | 0.145 |
| 8 | `transit_transfer_wait_min` | minutes waiting at a transfer | 0.145 |
| 9 | `transit_transfer_walk_min` | minutes walking between two stops at a transfer | 0.165 |
| 10 | `transit_transfers` | the number of transfers | 0.5 |
| 11 | `money_eur` | euros paid | 0.6 |
| 12 | `car_ln_path_size` | the logarithm of a car route's path size, which is below 0 for a route that shares links with other routes of the set | −1 |
| 13 | `is_car` | 1 for a car alternative: the car's constant | 0 |
| 14 | `is_bike` | 1 for a bike alternative: the bike's constant | 0 |
| 15 | `is_walk` | 1 for a walk alternative: the walk's constant | 0.5 |
| 16 | `is_transit` | 1 for a transit alternative: the transit constant | 0.25 |

The defaults are starting values to be calibrated for a study, not estimates for any city; their sources are in
[Parameters](../reference/parameters.md). With the defaults a cost reads as tenths of a car minute: a minute of
walking costs as much as 1.65 minutes of driving, and a transfer as much as 5. Money is 0 for every alternative in
release 0.2, since no input carries a price, so `money_eur` has no effect yet.

## Giving the coefficients

% src: crates/binding/README.md (keywords of `run`; `run_file`: `coefficients`; "Errors"); run by the writer on dotnet

Left out, every class takes the defaults. Given, the keyword `coefficients` of `run` is a dict from **every** class id
to its sixteen numbers in the order of the table. The example lowers the cost of a minute on the bike for one class of
the synthetic city `dotnet` and compares that class's mode shares:

```python
import openmobisim as oms
import pyarrow.compute as pc
import pyarrow.parquet as pq

classes = ["bike_only", "car_and_bike", "car_only", "no_vehicle"]
network = oms.Network.from_tables("dotnet/nodes.parquet", "dotnet/links.parquet")
transit = oms.Transit.from_gtfs("dotnet/gtfs.zip", network)
demand = oms.Demand.from_table("dotnet/trips.parquet", classes, network, transit=transit)

default = [0.1, 0.1, 0.165, 0.1, 0.165, 0.165, 0.145, 0.145, 0.165,   # the nine times
           0.5, 0.6, -1.0,                                            # per transfer, per euro, on ln path size
           0.0, 0.0, 0.5, 0.25]                                       # the constants: car, bike, walk, transit
keen_cyclists = list(default)
keen_cyclists[1] = 0.08                                               # a minute on the bike costs 0.08, not 0.1

coefficients = {class_id: default for class_id in classes}
coefficients["car_and_bike"] = keen_cyclists

def shares(path, class_id):
    trips = pq.read_table(path)
    trips = trips.filter(pc.equal(trips["class_id"], class_id))
    counts = trips.group_by("mode").aggregate([("trip_id", "count")]).sort_by("mode").to_pylist()
    return {row["mode"]: round(100 * row["trip_id_count"] / trips.num_rows, 1) for row in counts}

before = oms.run(network, demand, transit, window=(27000, 34200), out="classes-default")
after = oms.run(network, demand, transit, window=(27000, 34200), out="classes-keen", coefficients=coefficients)
for class_id in ("car_and_bike", "car_only"):
    print(class_id, shares(before.trips, class_id), shares(after.trips, class_id))
```

```text
car_and_bike {'bike': 27.8, 'car': 62.3, 'transit': 9.9, 'walk': 0.1} {'bike': 34.0, 'car': 57.6, 'transit': 8.3, 'walk': 0.1}
car_only {'car': 84.3, 'transit': 15.4, 'walk': 0.3} {'car': 85.4, 'transit': 14.4, 'walk': 0.2}
```

| Class | Mode | Share with the defaults (% of the class's travellers) | Share with `bike_ride_min` 0.08 for `car_and_bike` (% of the class's travellers) |
|---|---|---|---|
| `car_and_bike` | bike | 27.8 | 34.0 |
| `car_and_bike` | car | 62.3 | 57.6 |
| `car_and_bike` | transit | 9.9 | 8.3 |
| `car_and_bike` | walk | 0.1 | 0.1 |
| `car_only` | car | 84.3 | 85.4 |
| `car_only` | transit | 15.4 | 14.4 |
| `car_only` | walk | 0.3 | 0.2 |

The second column of shares is a changed run, made here to show the effect of one coefficient; it is not the published
case. Each share is rounded to 0.1 %, so a class's shares may add to 100.1 %.

The class whose coefficient changed cycles more. The class `car_only`, whose coefficients did not change, drives a
little more: the classes share the roads, and the travel time of a car on a link is the one thing a traveller's choice
changes for the others ([The loading](../concepts/loading.md)). Checked on the two runs' trips tables: the 6 731
travellers of `car_only` who drive in both runs take 15.2 min on average with the defaults and 14.3 min in the changed
run.

In a settings file the same change is one table per class, each coefficient by its name, all sixteen written:

```toml
[parameters.coefficients."car_and_bike"]
car_drive_min = 0.1
bike_ride_min = 0.08
# ... the other fourteen, each by its name
```

Refused, naming what is wrong: a class without a row, a row for a class the run does not have, a row that is not
sixteen numbers, a coefficient that is not a finite number, and, in a run with a timetable, a negative coefficient on
a transit time or on the transfers. The record writes each class's sixteen under
`[parameters.coefficients."<class id>"]`.

## The two choice models

% src: crates/choice/README.md ("Meanings and assumptions"); crates/loop/README.md ("Choose", "Assess");
% PARAMETERS.md (`choice`, `nest_scale`)

The parameter `choice` names the model that reads the costs:

| `choice` | Model | What a traveller takes |
|---|---|---|
| `"logit"` (the default) | a nested logit with one nest per mode, and a path-size logit over the car routes within the car nest | each alternative with a probability that falls as its cost rises; a traveller's random taste is drawn once and kept for the whole run |
| `"deterministic"` | the least-cost choice | the alternative of least cost in its set |

Under the logit the parameter `nest_scale` (0.5 by default) is the scale of the nests, a number in (0, 1]; at 1 the
model is the plain logit. The coefficient `car_ln_path_size` is read times `nest_scale` under
the logit, so that −1 is the path-size logit at every nest scale, and as 0 under the least-cost choice. The equations
are in [The choice](../concepts/choice.md).

## For bikes

% src: crates/loop/README.md ("Choose", "Measure", "Bikes"); crates/choice/README.md ("Bikes")

The bike is an alternative like any other, in a nest of its own. Its cost is `bike_ride_min` times its riding
minutes, plus `walk_min` times the minutes walked between the doors and the bike network, plus the constant
`is_bike`. A traveller's set holds one bike route, the shortest, found once, so the bike has no path-size term.
[Cycling](cycling.md) gathers everything on bikes.

## See also

- [The choice](../concepts/choice.md): the models and their equations.
- [Parameters](../reference/parameters.md): the defaults and their sources.
