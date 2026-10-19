# Scenarios and edits

A study compares runs that differ in one thing: a closed road, a new cycle track, a more frequent line, an incident.
openmobisim makes each change a copy of the network or timetable, so the original stays as it was and the runs line
up link by link.

## Edit the network

% src: python/openmobisim/network.py network_edit (Args)
```python
works = ms.network_edit(network, close=["link-1203", "link-1204"])
wider = ms.network_edit(network, lanes={"link-88": 3})
bottleneck = ms.network_edit(network, capacity_factor={"link-88": 0.6})
track = ms.network_edit(network, bike_facility={"bike-512": "separated"})
```

| Change | What it does |
|---|---|
| `close` | Road links closed to cars and buses for the whole run; no route uses them, and buses take another way. Walking and cycling keep their own layers. Travellers know in advance |
| `lanes` | Lanes per direction: capacity, jam density and storage scale with them (at least 1; to remove a road, close it) |
| `capacity_factor` | Capacity multiplied, storage kept: a bottleneck |
| `bike_facility` | A bike link becomes `separated`, `lane` or `mixed`: its speed, route cost and the choice model's attributes follow |

Links keep their indices, so `run.link_bins()` of two runs compare link by link. The changes enter the run's
fingerprint.

% src: python/openmobisim/scenario.py Scenario.replace, Scenario.parts
**A scenario from another.** `base.replace(network=works)` is the base scenario with only its network changed;
any argument of `from_parts` can be changed so (`base.replace(flow_level=4)`,
`base.replace(choice_options={"beta_time_min": -0.1})`), and a misspelt name is refused with the list. `base.parts`
holds the arguments a scenario was built from.

## Edit the timetable

`ms.transit_edit(transit, cancel=[...], headway={...})`; see
[Transit and parking](transit_parking.md#change-the-service).

## Things that happen at a time of day

% src: scenario.py Scenario.from_parts(disruptions=, disruptions_known=)
```python
incident = [{"links": ["link-88"], "capacity_factor": 0.0, "from_s": 8 * 3600, "to_s": 9 * 3600}]
late_tram = [{"line": "T1", "delay_s": 600, "from_s": 7.5 * 3600, "to_s": 8.5 * 3600}]
scenario = ms.Scenario.from_parts(network, trips, disruptions=incident + late_tram)
```

- **On roads**, the links keep a share of their capacity in the window: 0 closes them, 0.5 halves it. Overlapping
  disruptions multiply.
- **On lines**, runs leaving their first stop in the window are late at every call, or do not run (`"cancel": True`).
  A late bus leaves late and drives on among the cars.

**Known or not.** By default (`disruptions_known=False`), travellers do not know: the run reaches its equilibrium
without the disruption, then loads the day once more with it, every choice kept. Only what happens within the day
reacts. With `disruptions_known=True`, every loading includes it, and travellers plan around it.

**At which level.** Road disruptions act through capacity, so not at `flow_level=0`. At the point queue (the default),
a closure's queue takes no road space: it delays only the vehicles in it. With spillback (`flow_level` 3 or 4), it
backs up across the junctions behind it.

## Your own values

% src: scenario.py from_parts(link_values=, person_values=, trip_values=); core-sim link_values.rs, demand_values.rs
A choice model can weigh values you give: per link (`link_values`, summed along each alternative), and per traveller
or trip (`person_values`, `trip_values`). Use these for, say, the attractiveness of a street or a traveller's income.
See [Choice](../concepts/choice.md).

## Compare two runs

Keep the master seed the same, so every traveller draws the same in both runs and the difference is the change's (see
[Experiments and reproducibility](reproducibility.md)).

## A worked comparison on Delft

Delft's default morning first, then the same morning with its busiest street closed, then with a cycle track on the
busiest street where bikes ride among the cars. The three runs share their master seed, so the differences are the
changes'.

```{include} _examples/scenarios-base.md
```

The street to close is found from the base run's own link table, and closed whole: every link of its OpenStreetMap
way, both directions.

```{include} _examples/scenarios-closure.md
```

The cycle track goes on the bike layer: its links are named by the bike layer's ids.

```{include} _examples/scenarios-track.md
```

```{include} _examples/scenarios-compare.md
```

A change to one street moves the city's totals little; it shows on the street itself:

```{include} _examples/scenarios-changed-links.md
```

`viz.map_link(closed)` beside `viz.map_link(before)` shows where the closed street's traffic went.

## For bikes

`bike_facility` edits are the way to study cycling infrastructure. Note that in this version an edited bike link
rides at the full facility speed, without the street's speed limit applied (see [Cycling](cycling.md)).
