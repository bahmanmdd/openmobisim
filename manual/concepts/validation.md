# Validation

How do we know openmobisim computes what it says? It is checked in three ways:
- **automated tests**, which must pass before any change is accepted;
- **validation cases**, small networks whose answer is known independently of the simulator, run before each release;
- **reference results** of the starter cases, which guard against unintended change.

## Automated tests

% src: crates/*/tests; python/tests (file names); .github/workflows/ci.yml
Every change runs the full test suite on Linux, macOS and Windows. Among the tests that compare with an answer known
in advance:

| What | Compared with | Where |
|---|---|---|
| The loading against a reference: a bottleneck whose queue spills back over two links, a saturated merge, a diverge with a blocked branch | The macroscopic link transmission model, written in the test from the papers {footcite}`yperman2007`; every link must count the same traffic leaving it every second, to within whole vehicles | `python/tests/test_ltm_reference.py` |
| Free flow | A trip's time equals the sum of its links' free-flow times | `crates/core-loading/tests/level0.rs` |
| Capacity changes | A closed link holds its cars until it opens, then discharges at capacity; a narrowed link at its share | `crates/core-loading/tests/capacity_changes.rs` |
| Buses on the roads | A bus leaves after its dwell, waits for its scheduled time when early, and does not hold up the car behind | `crates/core-loading/tests/chains.rs` |
| Transit journeys | The journeys found against the timetable, counted independently | `python/tests/test_transit_timetables.py` |
| Repeatability | The same run twice, with 1 and 7 threads, and without parallelism, byte for byte; a multimodal run and the Delft case at 1 thread and at every core | CI's determinism job; `python/tests/test_threads.py` |
| The fingerprint | Changing any parameter a run uses changes its fingerprint | `python/tests/test_fingerprint_complete.py` |

## Validation cases

% src: the project's validation cases (run before each release; results kept per platform)
Validation cases are small networks built from scratch whose answer comes from theory, the literature, or an
independent count of the model's own rules. They are run by hand before each release rather than in CI, because a
known solution of an idealised system need not match the model exactly. Each comparison is judged by the kind of
reference it uses:

| Kind | Meaning | A difference means |
|---|---|---|
| exact | What the model must give by its own rules, counted independently | a fault |
| theory | A closed form for the idealised system | small: discretisation; large: worth a look |
| literature | A published number for a model that differs on purpose | the direction and size the case explains |
| contrast | Another model, shown beside the reference | nothing: it is not judged |

A row **matches** within its tolerance, is **close** within three times it, and is **off** beyond.

Every case below shows its set-up, its reference and its rows from the last run, which is repeated before each
release.

```{include} _validation_results.md
```

### Sioux Falls against its published equilibrium

Sioux Falls is also published with a static user equilibrium at its full demand {footcite}`leblanc1975,tntp`. That
model differs from openmobisim's on purpose: its link times come from a volume-delay function rather than from
queues and waves, and its demand is a constant flow rather than departures over time. The two are shown side by side
in notebook [07 · Validation](../tutorials/07_validation.ipynb), as a contrast, not judged. At low demand the two
models must agree, which V-S1 checks.

Notebook 07 also works through a free-flow trip by hand and a bottleneck against queueing theory.

## Reference results

The starter cases' reference results are checked with `openmobisim check`: the same platform must give the same
numbers, another platform the same within tolerances. See [Starter cases](../reference/cases.md).

## For bikes

Bike trips take part in the repeatability tests and the mode-choice tests. There is no validation case yet with a known
answer for bike route choice: bike routes are least-cost routes on the bike layer, checked by the layer tests.

## References

```{footbibliography}
```
