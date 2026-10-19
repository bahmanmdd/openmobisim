# Warnings and errors

openmobisim warns when a run does something you may not expect, and refuses inputs it cannot use. Every message says
what happened and what to change.

## Warnings of a run

% src: python/openmobisim/scenario.py WindowWarning, NoAlternativeWarning, FallbackWarning (and warnings.simplefilter
% "always"), Scenario.__init__ (the class-coefficient warning), NO_ALTERNATIVE_CHANGES
| Warning | When | What it says | What to do |
|---|---|---|---|
| `WindowWarning` | Trips of the demand depart outside the simulation window | How many trips and people (share of the demand), before or after the window, their departure times, and a window that would hold them | Widen the window, or take the trips out of the demand. See [The simulation window](../guides/window.md) |
| `NoAlternativeWarning` | Trips had no alternative: no mode they may use reached their destination within the limits. They were not simulated | How many trips and people; by class and stated mode; why, by mode offered; the places most of them are between; the option to change for each reason | Read {py:meth}`openmobisim.Run.report_no_alternative`; change the limit named. See [Modes](../concepts/modes.md) |
| `FallbackWarning` | With `mode_options={"fallback_car": 1}` only: trips with no alternative travelled by car instead | How many trips and people, and the places most of them are between | Read {py:attr}`openmobisim.Run.fallback_car` |
| `UserWarning` (class coefficients) | A run-wide coefficient (`choice_options`) is also set by some classes | Which classes keep their own value | Set the coefficient in those classes, or accept that it applies to the other classes only |

`WindowWarning`, `NoAlternativeWarning` and `FallbackWarning` are exported by the package, so you can filter or catch
them:

```python
import warnings
import openmobisim as ms

with warnings.catch_warnings():
    warnings.simplefilter("error", ms.NoAlternativeWarning)   # make it an error in a study script
    run = scenario.run("study")
```

`WindowWarning` is shown at every run, not only the first time.

## Warnings when a network is read from a table

% src: python/openmobisim/network.py network_read_table (three warnings.warn)
| When | What it means | What to do |
|---|---|---|
| Links have a `class` not recognised as a road class | They are treated as unclassified (one modest lane) | Give recognised names (`primary`, `residential`, …) |
| Links name a node the node table does not have | They are left out of the network | Complete the node table |
| Links ask for a capacity their class's jam density cannot carry at their free-flow speed | Their capacity is kept and their jam density raised so the fundamental diagram closes: their storage at flow levels 3 and 4 is larger than their class's | Give `lanes`, or a class that fits, if you know the real ones |

## Errors

% src: the ValueError, TypeError, RuntimeError, FileNotFoundError and FileExistsError raises of python/openmobisim/*.py and the Rust
% option parsers (from_options)
An input openmobisim cannot use is refused before anything runs, with a message that names what is wrong and, where
there is a list of valid values, the list. Most are a `ValueError`:

| Where | What is refused | Error |
|---|---|---|
| Any option | A misspelt name, with the list of valid names (network options when the network is read, the others when the scenario runs); an option class's misspelt keyword where it is written. See [Calibrating parameters](../guides/calibration.md) | `ValueError`; `TypeError` for an option class |
| Any option | A value out of range: a negative time, a flag other than 0 or 1, a factor not above 0, a window that ends before it starts | `ValueError` |
| A method by name | An unknown flow level, route method, route update, equilibration or choice model, with the list | `ValueError` |
| Trips | A row with a latitude outside ±90, a departure that is not whole seconds after midnight, or an unknown mode, with the row's number and the field; a demand with no trip left, for example all outside the window | `ValueError` |
| Traveller classes | A row without a class, a class listed twice, an unknown column, with the row's number | `ValueError` |
| Zones and matrices | A zone listed twice; a matrix row naming a zone the zones table does not have | `ValueError` |
| A choice model | A coefficient for an attribute no alternative carries, with the attributes the alternatives carry | `ValueError` |
| Edits | An unknown link, line or facility, 0 lanes, a headway not above 0 (`network_edit`, `transit_edit`) | `ValueError` |
| `Scenario.replace` | A name that is not an argument of `from_parts`, with the list | `TypeError` |
| A timetable | A feed that cannot be read, lacks a required file, or runs nothing on the chosen day | `ValueError` |
| A table file | A path that does not exist | `FileNotFoundError` |
| A run's folder | A folder that already holds a run, unless `overwrite=True` | `FileExistsError` |
| `Run.open` | A folder with no `manifest.json`; then a result only the run in memory holds, or a file a minimal run left out, refused by name when asked for | `FileNotFoundError`; `RuntimeError` |
| A starter case | An unknown case, with the list; a file that differs from the bundle's checksums; a case that cannot be downloaded | `ValueError`; `FileNotFoundError` |
