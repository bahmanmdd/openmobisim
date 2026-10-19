# Calibrating parameters

Every number openmobisim assumes is a named parameter with a default. You can list them all, change any by name, and
the run records what you changed. The defaults are **not calibrated**: each comes from a published source or is
stated as an assumption ([Parameters](../reference/parameters.md) gives the source of each). For a study, calibrate
the ones your results depend on against your own data.

## Every parameter in one table

% src: python/openmobisim/parameters.py parameters(); tested at the manual's build (251 rows, the groups below)
```python
import pandas as pd
import openmobisim as ms

table = pd.DataFrame(ms.parameters())   # group, argument, name, value, unit, note
table[table.name.str.contains("walk_max")]
```

```text
       group         argument                 name   value unit
107    modes     mode_options           walk_max_s  1800.0    s
112  transit  transit_options    access_walk_max_s  1800.0    s
114  transit  transit_options  transfer_walk_max_s   300.0    s
121  parking  parking_options           walk_max_s   300.0    s
```

The `argument` column tells you where to change a parameter:

| Group | Parameters | Change it through |
|---|---|---|
| `network` | 95: speeds, lanes, saturation flows and jam densities by road class; signals; the bike and walk layers | `network_options` of the network reader |
| `loading` | 6: priority at merges, rerouting, turn pockets | `loading_options` |
| `modes` | 6: the longest walk and ride offered, the car fallback, the car's access and egress times and where car trips join the roads | `mode_options` |
| `transit` | 11: walks to and between stops, boarding, journey limits, buses on the roads | `transit_options` |
| `parking` | 13: walks, reach, candidates, bins | `parking_options` |
| `prices` | 7: running costs, fares, parking fees | `price_options` |
| `scenario` | 7: the flow level and step, the window, the seed, the choice-set limits | the `Scenario` argument of the same name |
| `equilibration.<strategy>` | 10 | `equilibration_options` |
| `route_method.<method>` | 12 | `route_options` |
| `route_update.<update>` | 4 | `route_update_options` |
| `choice_model.<model>` | 79: the coefficients of `logit` and `nested_logit` | `choice_options`, or a class's own |

## Changing them by name

% src: scenario.py Scenario.from_parts (the *_options arguments); _core.pyi network_read_osm(network_options=);
% tested: an override recorded in manifest.json (bike_max_s 2700; choice_descriptor beta_time_min=-0.12)
```python
network = ms.network_read_osm(
    "city.osm.pbf",
    network_options={"residential.free_flow_km_h": 25, "signal_cycle_s": 80},
)
scenario = ms.Scenario.from_parts(
    network, trips,
    mode_options={"bike_max_s": 2700},          # offer rides of up to 45 minutes
    choice_options={"beta_time_min": -0.12},    # the weight of a minute in the utility
)
```

% src: python/openmobisim/options.py (generated from the core's parameter table)
**Or with the typed classes**, one per group, whose keywords an editor completes with their defaults:

```python
network = ms.network_read_osm(
    "city.osm.pbf",
    network_options=ms.NetworkOptions(signal_cycle_s=80, residential=ms.LinkClassOptions(free_flow_km_h=25)),
)
scenario = ms.Scenario.from_parts(
    network, trips,
    mode_options=ms.ModeOptions(bike_max_s=2700),
    choice_options=ms.NestedLogitOptions(beta_time_min=-0.12),
)
```

Each class (`NetworkOptions`, `LoadingOptions`, `ModeOptions`, `TransitOptions`, `ParkingOptions`, `PriceOptions`,
`MsaOptions`, `FreeFlowOptions`, `PenaltyOptions`, `MonteCarloOptions`, `BestResponseOptions`, `LogitOptions`,
`NestedLogitOptions`) is a read-only dict of the options given, so it goes wherever the dict goes and the two make the
same run. A misspelt keyword fails where it is written. The two options whose default is a rule of the flow level
(`reroute`, `increments`) read `None` until you set them.

**A misspelt name is refused, with the list of valid ones.** Network options are checked when the network is read;
the others when the scenario runs:

```text
ValueError: mode_options has no "walk_max"; the options are: walk_max_s, bike_max_s, fallback_car, car_access_s, car_egress_s, car_snap_motorway_m
ValueError: no network option called "motorway.free_flow_kmh"; the options are: bike_dedicated_km_h, …
```

A coefficient no alternative carries is refused in the same way, with the attributes the alternatives carry.

**To read a network's values back**, call `ms.network_options(network)`. Without a network, it lists the shipped values.

## Coefficients by class

% src: scenario.py Scenario.__init__ (class coefficients win by name, with a warning); demand.py demand_read_classes
A traveller class may set its own coefficients: any `beta_*` column of the class table, or key of a `classes` dict. A
class's value **replaces** the run-wide one for its travellers. When you set a run-wide coefficient that some classes
set too, the scenario warns you, so a study that varies it is not misled:

```text
UserWarning: choice_options' beta_mode_bike is replaced by the class's own for bike_only, car_and_bike: a class's
coefficients win by name, so the run-wide value applies to the other classes only.
```

## What is recorded

% src: crates/io-parquet/src/manifest.rs; crates/core-sim/src/identity.rs
Every run writes the values it used to `manifest.json`: each method with all its options in canonical form (for
example `choice_descriptor`: `logit;beta_bike_mixed_km=-0.16;…;beta_time_min=-0.12;…;class:bike_only.beta_mode_bike=3.1`),
the limits, and `defaults_version`, the version of the shipped defaults. The values also enter the run's fingerprint,
so two runs with different parameters never share one. See [Reproducibility](../concepts/reproducibility.md).

## A calibration loop

The parameters are plain arguments, so a calibration is a loop in Python. Each run is repeatable, and runs with the
same seed share their random draws (common random numbers), so the difference between two runs is the parameter's
effect, not noise:

```python
for beta in (-0.08, -0.10, -0.12):
    run = ms.Scenario.from_parts(network, trips, choice_options={"beta_time_min": beta}).run(f"beta{beta}")
    # compare run.trip_modes(), run.link_bins_table(), … with your counts or survey
```

Notebook [06 · Parameters and experiments](../tutorials/06_parameters_and_experiments.ipynb) works through this on a
starter case.

## For bikes

The cycling parameters are in four groups:

| Group | Parameters |
|---|---|
| `network` | The bike layer's speeds by facility (`bike_dedicated_km_h`, `bike_mixed_km_h`) and the cost factors that make routes prefer cycleways and cycle lanes (`bike_lane_cost_factor`, `bike_mixed_cost_factor`) |
| `modes` | The longest ride offered (`bike_max_s`) |
| `parking` | The bike's reach to a parking (`reach_bike_s`) and its time terms (`floor_bike_s`, `slope_bike_s`) |
| `choice_model` | The bike's weights: `beta_mode_bike`, `beta_bike_min`, and the kilometres by facility (`beta_bike_separated_km`, `beta_bike_lane_km`, `beta_bike_mixed_km`) |

Calibrating them against counts on cycle paths, or a survey's cycling share, follows the same loop.
