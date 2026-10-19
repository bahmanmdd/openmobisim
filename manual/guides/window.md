# The simulation window and departures

A run simulates one stretch of the day, the **simulation window**: it starts at the window's start with an empty
network and stops at the window's end. Choosing the window, and which departures to put in it, decides what a run's
results describe.

## Setting the window

% src: python/openmobisim/scenario.py _window, Scenario.from_parts(window=); crates/py-bindings/src/pipeline.rs
% Window::resolve, WINDOW_DEFAULT_S
```python
scenario = ms.Scenario.from_parts(network, trips, window=("07:30", "09:30"))
```

The start and end are clock times (`"07:30"`) or seconds after midnight (`27000`). Either may be `None`.

| Given | Start | End |
|---|---|---|
| Both | as given | as given |
| Only the start | as given | 3 hours after the start |
| Only the end | the earliest departure, rounded down to the quarter hour | as given |
| Neither | the earliest departure, rounded down to the quarter hour | 3 hours after the start |

A window that ends before it starts is refused.

## What happens at its edges

% src: crates/py-bindings/src/pipeline.rs Window::resolve; scenario.py Run.window, Run._window_text, WindowWarning;
% crates/core-sim (the loading starts at the window's start); crates/core-transit/src/timetable.rs Timetable::within
- **Trips departing outside the window are left out.** This covers departures before the start, and at or after the
  end. They are not simulated and appear in no result, not even as unfinished. Every run that leaves trips out issues
  a `WindowWarning`. It says how many trips and people were left out, when they depart, and a window that would hold
  them. Python's default filter shows a warning once per place it is raised from; `warnings.simplefilter("always",
  openmobisim.WindowWarning)` shows it at every run (the library sets no filter of its own).
- **Trips still travelling at the end are unfinished.** They are counted as `truncated` in
  {py:attr}`openmobisim.Run.completion`, and their time so far is not a completed trip's time.
- **The network starts empty.** No vehicle is on the road at the window's start; the loading begins there.
- **A traveller's day starts at their first trip inside the window.** A trip clipped before the start is not
  simulated, and the traveller's car or bike starts where their first trip in the window starts (as if the earlier
  trip had brought it there).
- **Transit is cut to the window.** Each run of the timetable keeps only its calls inside the window. A run under way
  when the window opens starts at its first stop inside it. A run left with fewer than two calls is dropped.

{py:attr}`openmobisim.Run.window` gives all of this as numbers:
- the start and end, and whether each was given;
- the trips and people left out before and after the window, with their departure times;
- what the timetable kept and dropped.

`manifest.json` records the window and the trips and people left out (`window_start_seconds`, `window_end_seconds`, both in
seconds after midnight, `trips_outside_window`, `people_outside_window`).

## Choosing the departures

A window has to leave the last trips time to arrive, or they end unfinished. One way to do that is to let trips depart
only in the first part of the window. For a study period of 07:30–09:30 with departures from 07:30 to 08:30:

**When you draw trips from a matrix**, draw their departures in that hour:

```python
trips = ms.demand_from_od(od, zones, start_s=7.5 * 3600, end_s=8.5 * 3600, profile="am_peak", seed=0)
```

**When your trips already have departure times** over a longer period, keep the ones in the hour. Keep their own
times, and leave the table you read them from unchanged:

```python
start, end = 7.5 * 3600, 8.5 * 3600
in_hour = [t for t in trips if start <= t[6] < end]   # column 6: departure_time_s
scenario = ms.Scenario.from_parts(network, in_hour, window=("07:30", "09:30"))
```

The trips outside the hour are then not part of the run. Every total in the results counts the trips in the hour
only.

**To simulate a whole morning's demand**, widen the window instead. For departures from 06:30 to 09:30, a window of
06:30–10:30 holds them all and leaves the last an hour to arrive. Results for the study period can then be read from
the per-link results' time bins.

## For bikes

The window applies to every mode alike: a bike trip departing outside it is left out, and one still riding at its
end is unfinished. Bike-and-ride trips also depend on the timetable cut to the window: a ride to a stop whose last
run of the window has left finds no journey.

## See also

- [Reproducibility](../concepts/reproducibility.md): the window is part of the run's fingerprint.
- [The loading](../concepts/loading.md): how vehicles move from the window's start.
