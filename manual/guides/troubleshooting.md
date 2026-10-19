# When something goes wrong

Most problems announce themselves: a warning at the end of a run, an error before it starts, or a number in the
results that looks wrong. This page goes from symptom to cause. Every warning and error is listed in
[Warnings and errors](../reference/warnings.md).

## Trips are missing from the results

% src: scenario.py WindowWarning, NoAlternativeWarning, Run.window, Run.report_no_alternative, Run.completion
| Symptom | Cause | Where to look | What to change |
|---|---|---|---|
| `WindowWarning` | Trips depart outside the simulation window and were left out | {py:attr}`openmobisim.Run.window` | The window, or the departures. See [The simulation window](window.md) |
| `NoAlternativeWarning` | No mode a trip may use reaches its destination within the limits | {py:meth}`openmobisim.Run.report_no_alternative`: every trip, and why for each mode | The limit named for each reason. See [Modes](../concepts/modes.md#a-trip-with-no-alternative) |
| Many `truncated` trips in `run.completion` | Trips still travelling when the window ends | The departures' times; `report_gridlock` (below) | A longer window, or departures earlier in it |
| `no_vehicle_available` trips | A trip states a mode whose vehicle its traveller does not have at the origin (a car trip of a class without a car) | The trips' `mode` and `user_class` | The class's ownership, or the trips' modes. Check that classes were drawn only for the trips that did not have one |
| `no_feasible_path` trips | The roads do not connect origin and destination | `ms.network_check(network)` | The network |

## Traffic stands still

% src: scenario.py Run.report_gridlock, Run.report_spillback; the loading levels
At the full model (`flow_level=4`), queues take road space and can spill back up to junctions, and in the worst case
lock in a loop.
- {py:meth}`openmobisim.Run.report_gridlock` says what stood still when the window ended: the vehicles still on the
  network or waiting to enter, and the closed loops of links waiting on one another.

The point queue (`flow_level=2`, the default) cannot lock, because its queues take no road space. Its delays are
optimistic where a real queue would have backed up. {py:meth}`openmobisim.Run.report_spillback` lists those links:
the ones where the most traffic on the link at once exceeded what it can store, by more than a few cars. See
[The loading](../concepts/loading.md).

## The run did not converge, or converged poorly

% src: scenario.py Run.converged, convergence_gap, convergence_verdict, convergence; core-sim equilibration
- `run.converged` says whether `msa` met its stopping rule before its iteration limit.
- `run.convergence_verdict` judges the gap: `good` below 5 %, `acceptable` below 15 %, `poor` above.

The two measure different things. A run can stop as converged while its verdict is only `acceptable`, because the
stopping rule does not read the same gap. `run.convergence()` gives every iteration's measures. More iterations
(`equilibration_options={"iterations": …}`), or a smaller `gap_tolerance`, let a run settle further. See
[Equilibration](../concepts/equilibration.md).

## A run is slow

`run.timings()` and `timings.csv` say where the time went, stage by stage and iteration by iteration. See
[Running faster](speed.md).

## An option is refused

A misspelt name is refused with the list of valid names; a value out of range, with the reason. See
[Calibrating parameters](calibration.md).

## Results differ from a colleague's

Compare the fingerprints. If they differ, so did the inputs: `manifest.json` records every setting. If they match but
the numbers differ slightly, compare the platforms in the two manifests. The bit-for-bit guarantee holds on one
platform. See [Reproducibility](../concepts/reproducibility.md).
