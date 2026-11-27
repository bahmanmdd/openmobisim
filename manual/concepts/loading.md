# The loading

% src: crates/loading/README.md; crates/loop/README.md ("Load", "Measure", "The window and the mean"); README.md ("The ten tests", "Known limits")

The **loading** (dynamic network loading, in the literature) takes the route every traveller has chosen and works out
when each one passes each link of it: where queues form, how long each car waits, who is at the door before the study
period ends. It never chooses. Every car, bike and walker arrives with its route and its departure, chosen beforehand,
and the loading moves each of them as an individual vehicle, in continuous time, with no time step.

Each mode is loaded by its own rule:

| Mode | Rule | Does it delay anyone? |
|---|---|---|
| Car | A **point queue**: the downstream end of every link is a bottleneck that lets one car through per headway | Yes: a car waits behind the cars ahead of it on the same link |
| Bike | Free flow: each link takes its length over the one bike speed | No |
| Walk | Free flow: each link takes its length over the one walk speed | No |
| Transit | Not loaded: a journey runs on the timetable ([Transit](transit.md)) | No |

The loading sits between the choice and the next search:

| It takes | From |
|---|---|
| The links: their two ends, length, free-flow time per mode, capacity, modes allowed | [The network](networks.md) |
| One vehicle per trip that took a car, a bike or a walk: its route (the links in order) and the instant it enters the first link | [The iteration](iteration.md), after the [choice](choice.md) |
| The window and the length of a time bin | The run's settings |

| It gives | To |
|---|---|
| Per trip, the arrival, or that it is still travelling at the window's end | The iteration's measure and assess stages; the trips table ([What a run gives](outputs.md)) |
| Per link and time bin, the cars, bikes and walkers that entered, and the cars' crossing times | The links table, the map and the summary |
| The **link time reading**: the time a traveller entering a link at an instant takes to cross it | The [route search](search.md) of the next iteration, and the pricing of every car route in every traveller's set |

Where the loading stands in a run is drawn in [How a run works](architecture.md).

## Cars on a point queue

% src: crates/loading/README.md ("Units", "Meanings and assumptions": point queue, events); crates/graph/README.md ("Meanings and assumptions": free-flow time, capacity)

```{image} ../_static/figures/point_queue_paper.svg
:alt: A link as a point queue: cars cross it in its free-flow time and leave its downstream end no closer than one headway apart; on the right, cumulative arrivals and departures at the gate, with the delay of a car and the cars in the queue between them
:class: only-light
:width: 100%
```
```{image} ../_static/figures/point_queue_night.svg
:alt: A link as a point queue: cars cross it in its free-flow time and leave its downstream end no closer than one headway apart; on the right, cumulative arrivals and departures at the gate, with the delay of a car and the cars in the queue between them
:class: only-dark
:width: 100%
```

A car crosses a link at its free-flow speed and then meets the link's downstream end, which lets cars through one at a
time. This is the bottleneck of Vickrey (1969): a point of fixed capacity where arriving cars wait and leave in the
order they came, in a queue that has no length on the road.

A car that enters link $a$ at the instant $t$ leaves it at the later of two instants: the instant it would reach the
downstream end at free flow, and one headway after the car that entered before it left.

$$
x = \max\left(t + \tau_a,\; x_\text{prev} + h_a\right), \qquad \tau_a = \frac{\ell_a}{v_a}, \qquad h_a = \frac{3\,600\ \text{s}}{C_a}
$$

| Symbol | Meaning | Unit |
|---|---|---|
| $t$ | The instant the car enters the link | s |
| $x$ | The instant the car leaves the link, and enters the next link of its route | s |
| $x_\text{prev}$ | The instant the car that entered before it left | s |
| $\tau_a$ | The car free-flow time of the link: its length $\ell_a$ over its car free-flow speed $v_a$ | s |
| $h_a$ | The headway of the link | s |
| $C_a$ | The capacity of the link: the flow its downstream end passes | vehicles per hour |

A link of 1 800 vehicles per hour has a headway of 2 s; one of 1 200 vehicles per hour, 3 s. A car link may have an
unlimited capacity: its headway is then 0 and every car crosses it at free flow.

**An example by hand.** A link of 1 000 m at 50 km/h takes 72 s at free flow. Its capacity is 1 200 vehicles per hour,
so its headway is 3 s. A car enters every 2 s: car $k$ (counted from 0) enters at $2k$ s. The first car leaves at 72 s.
Every later car reaches the end 2 s after the car ahead but may leave only 3 s after it, so car $k$ leaves at
$72 + 3k$ s and is delayed by $k$ seconds. This case is the first of the ten tests ([Validation](validation.md)).

What follows from the rule:

- **First in, first out.** Cars leave a link in the order they entered it.
- **A queue takes no room.** A link holds any number of cars, so a queue never backs up into the links upstream (no
  spillback).
- **Only the link's own end holds a car.** There is no rule at a node, no merge rule and no signal. Where two links
  feed a narrower one, the cars queue on the narrower link's end in the order they reach that link, which is what a
  merge with priorities in proportion to the demands gives (test 2).
- **No time step.** Instants are real numbers of seconds, never rounded. The loading handles one event at a time, a car
  reaching the downstream end of a link, in the order of the instant and then of the trip's identifier, so two cars at
  the very same instant are always taken in the same order.

Every vehicle, a car, a bike or a walker, enters the first link of its route on the whole second at which its walk
from the door to the network ends.

## Bikes, walkers and transit

% src: crates/loading/README.md ("Bikes"); crates/loop/README.md ("Load", "Measure")

A bike and a walker cross each link of their route in its length over their mode's speed (`bike_speed`, 15 km/h by
default; `walk_speed`, 4.8 km/h), and enter the next link when they leave one. They interact with nothing: their times
and their counts are the same, to the last bit, with or without cars on their links. A bike rides 1 500 m at 15 km/h in
360 s; a walker covers 1 000 m at 4.8 km/h in 750 s.

A transit journey is not loaded. Its vehicles are not on the roads and its time is the timetable's
([Transit](transit.md)). The walks of a transit journey, and the walks between a door and the network of every mode,
are not on links either, so the loading does not count them.

## The window and unfinished trips

% src: crates/loading/README.md ("The window is half-open …"); crates/loop/README.md ("The window and the mean"); README.md ("Known limits", first item)

The loading covers the run's **window**, the study period, from its start up to but not including its end.

- A trip that is at its door before the window's end is **finished** and has a travel time.
- A trip that is not is **unfinished**: the loading records the link it is on and when it entered that link, and the
  trip has no travel time.
- A crossing that starts before the end is recorded with its exact exit, even when the exit lies after the end. No
  link is entered at or after the end.

Two consequences matter when a result is read.

**The mean trip time is over the finished trips only.** A queue that pushes trips past the window's end removes the
longest trips from the mean. Read the mean together with the unfinished count, which stands beside it in every row of
the iterations table; a window long enough for every trip to finish makes the means of two iterations, or of two runs,
comparable ([The simulation window](../guides/window.md)).

**Near the window's end, costs read too low.** A traveller pricing a route reads a link it would enter after the end
without the cars that would have entered it after the end, since those crossings were never made. Where every car
finishes within the window, no crossing is missing and this does not arise.

## What the loading records

% src: crates/loading/README.md ("Outputs"); crates/io-parquet/README.md (`links.parquet`; "A crossing is filed under the bin …"; "Bikes"); a run of the Delft case

The window is cut into **time bins** of `link_bin` seconds (300 s by default). For every link and bin in which someone
entered the link, the loading keeps one row. A vehicle is filed under the bin in which it **entered** the link.

| Recorded per link and bin | Unit | Column of `links.parquet` |
|---|---|---|
| The cars that entered the link in the bin | cars | `cars` |
| The sum of those cars' crossing times (entry to exit, queue included) | s | `car_time_s` |
| The exits of the first and of the last of those cars | s of the clock | `first_exit_s`, `last_exit_s` |
| The bikes that entered | bikes | `bikes` |
| The walkers that entered | walkers | `walkers` |

The mean crossing time of a bin is `car_time_s` over `cars`. Bikes and walkers have counts and no time, because each
crosses the link at its free-flow time by definition.

The loading also keeps every single car's crossing (its entry, its exit and its trip), from which the link time
reading below is made. These are not written to a file.

**An example.** One link of the Delft case, of 233 m at 30 km/h with a capacity of 810 vehicles per hour, through the
first half hour of the window (the folder `delft-run` is the run made on [What a run gives](outputs.md); the case's own
`links.parquet` holds the link's length, speed and capacity):

```python
import pyarrow.parquet as pq

link = "car:48204096:00004115"
row = pq.read_table("delft/links.parquet", filters=[("link_id", "=", link)]).to_pylist()[0]
free_flow = row["length_m"] / (row["car_speed_km_h"] / 3.6)
print(f"free-flow time {free_flow:.2f} s, headway {3600 / row['capacity_veh_h']:.2f} s")
bins = pq.read_table("delft-run/links.parquet", filters=[("link_id", "=", link)]).to_pylist()
for b in bins[:6]:
    mean = b["car_time_s"] / b["cars"]
    spacing = (b["last_exit_s"] - b["first_exit_s"]) / (b["cars"] - 1)
    print(b["bin_start_s"], b["cars"], f"{mean:.1f}", f"{spacing:.2f}")
```

```text
free-flow time 27.94 s, headway 4.44 s
27000 10 28.6 18.63
27300 82 67.9 4.55
27600 88 153.9 4.44
27900 90 256.4 4.44
28200 67 300.7 4.44
28500 54 260.6 4.44
```

| Bin starts (s of the clock) | Cars that entered | Mean crossing time (s) | Mean spacing of their exits (s) |
|---|---|---|---|
| 27 000 | 10 | 28.6 | 18.63 |
| 27 300 | 82 | 67.9 | 4.55 |
| 27 600 | 88 | 153.9 | 4.44 |
| 27 900 | 90 | 256.4 | 4.44 |
| 28 200 | 67 | 300.7 | 4.44 |
| 28 500 | 54 | 260.6 | 4.44 |

At 4.44 s per car the link's end passes 67.5 cars in a bin of 300 s. In the first bin 10 cars enter and cross at nearly
the free-flow time. From the second bin on more cars enter than can leave, the queue grows, and the cars leave exactly
one headway apart: the crossing time rises to 300.7 s, of which 27.9 s is driving and the rest is waiting. When fewer
than 67.5 cars enter in a bin, the queue shrinks.

## The link time reading

% src: crates/loading/README.md ("Outputs": `LinkTimes`; "Link time reading"; "The reading's limits, on a route the traveller did not take")

The next iteration needs to know what a route would cost. The **link time reading** answers one question: a traveller
entering this link at this instant, how long does it take to cross? It applies the point queue's own rule to the cars
the loading recorded:

> a car entering a link at the instant $t$ leaves at the later of $t$ plus the free-flow time, and one headway after
> the car ahead of it.

Nothing is interpolated and no time bin is read: the reading works from the single crossings. Which recorded car is
"ahead" of the traveller is settled by four rules:

1. the cars that entered the link before $t$ are ahead;
2. of the cars that entered at exactly $t$, those of a lower trip identifier are ahead (the loading's own order);
3. the traveller's own recorded car is never ahead of it: the cars behind it are replayed without it;
4. a recorded car whose place the traveller took on the link before is behind the traveller.

Before the first car, and on a link no car entered, the reading is the free-flow time. For a bike and a walker it is
the free-flow time everywhere.

A route's time is read link by link: each link is entered at the instant the one before it is left. Two properties
follow.

- **A traveller's own route reads exactly what it took.** Walking its own route from its own departure, a traveller
  reads the time its trip took, to the last bit. So the cost of the alternative a traveller holds is the cost it
  experienced.
- **A route not taken is read as if the traveller alone had moved.** The reading holds every other car where the
  loading recorded it. In truth, moving one car lets the cars behind it on its old route move up and pushes back the
  cars behind it on its new route. Where no other car's entry on the route changes, the reading is exact; where one
  does, it is off by one headway of that link for each car held on the wrong side of the traveller, and queues further
  along can add to that. Only loading again with the traveller moved settles it. The tests compare the reading with
  such a second loading on hand-built cases, among them cases where the two differ by 4 s (452 s read, 456 s taken).

Who uses the reading: the [route search](search.md), which finds each car's fastest route on it, and the iteration's
measure stage, which prices again every car route in every traveller's set ([The iteration](iteration.md)).

## Defaults

% src: PARAMETERS.md; crates/binding/README.md ("Keywords")

| Parameter | Given to | Unit | Default | What the loading does with it |
|---|---|---|---|---|
| `capacity_per_lane` | `Network.from_tables`, `Network.from_osm` | vehicles per hour per lane | 1 800 | The capacity of a car link whose row gives none: its lanes times this value |
| `bike_speed` | `Network.from_tables`, `Network.from_osm` | km/h | 15 | The speed of every bike on every link |
| `walk_speed` | `Network.from_tables`, `Network.from_osm` | km/h | 4.8 | The speed of every walker on every link, and of every walk between a door and the network |
| `link_bin` | `run` | s | 300 | The length of a time bin of the links table |

The window is given to `run` as `window=(start_s, end_s)` and has no default. The sources of the defaults are on the
[parameters page](../reference/parameters.md).

## For bikes

% src: crates/loading/README.md ("Bikes"); crates/io-parquet/README.md ("Bikes"); README.md ("Known limits", last item)

A bike is loaded as a vehicle of its own on the links that allow bikes, a street or a cycle path alike, and is counted
on every link of its route in the bin in which it enters. It rides at one speed, `bike_speed`, on every link, and never
queues: 1 000 cars queued on a street leave the time of a bike on that street unchanged, a case the tests check. Since a
bike's time does not depend on the traffic, a bike route costs the same at every iteration. The links table gives the
bikes of each link and bin in the column `bikes`.

## Limits

% src: crates/loading/README.md ("Meanings and assumptions"); crates/graph/README.md ("Signals … are stored and applied nowhere"; "No storage, jam density, passenger-car unit or wave speed"); README.md ("Known limits")

- **No spillback.** A link stores any number of cars, so a queue never blocks the links upstream.
- **No junctions.** No node rule, merge rule, turn delay or signal holds a car. Traffic signals read from OpenStreetMap
  are stored and applied nowhere. On a network read from OpenStreetMap the capacity of a link stands in for its
  junctions ([Networks](networks.md)).
- **Every vehicle counts as one.** There is no passenger-car unit, and no bus is on the roads.
- **Bikes and walkers travel at free flow**, at one speed each; there is no bike congestion, and cars and bikes do not
  delay one another.
- **Transit is not loaded**: no bus is on a road, and no vehicle fills.
- **The mean trip time is over finished trips only**, and near the window's end costs read too low (above).
- **The reading is exact only for the route a traveller took**, and for a route where moving the traveller moves no
  other car (above).
- **A car trip while a bottleneck's queue builds up is slightly above theory.** In the test of cars against bikes
  behind a bottleneck, the mean car trip in the build-up period was measured about 1.5 s above the theoretical value on
  a trip of about 692 s (0.2 %) with the step exponent at 1; about 0.4 s of it comes from departures on whole seconds,
  and the rest is not explained. At the default step exponent, 0.75, the difference is +0.9 ± 0.8 s over 20 seeds,
  which tells neither explanation apart.

All limits of the model are gathered in [Limitations](limitations.md).

## References

- Vickrey, W. S. (1969). Congestion theory and transport investment. *The American Economic Review* 59(2), 251–260.
- Daganzo, C. F. (1995). The cell transmission model, part II: network traffic. *Transportation Research Part B*
  29(2), 79–93. (The merge rule test 2 is compared with.)
- Newell, G. F. (1982). *Applications of Queueing Theory*, 2nd edition. Chapman and Hall, London. (The tandem of two
  bottlenecks in the loading's tests.)

The full list is in the [bibliography](../reference/bibliography.md).
