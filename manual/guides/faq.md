# Questions and answers

Short answers, each with the page that says more. For a run that went wrong, start with
[When something goes wrong](troubleshooting.md).

**What does openmobisim simulate?** A day's travel, or part of it: travellers choose their modes, routes and
itineraries, and a dynamic loading moves every vehicle and person through the road, bike and walk networks and the
timetable; choice and loading repeat until the pattern settles. See [How a run works](../concepts/architecture.md).

**Is a starter city a model of that city?** No. Its morning is synthetic: plausible, built from open data and the
city's travel survey, and checked against them, but not calibrated. Use the starter cases to learn the tool and to
compare methods; for a study, bring the city's own demand. See [Starter cases](../reference/cases.md).

**Which computers does it run on?** Python 3.11 or later, on Linux, macOS and Windows, each tested on every change.

**How large a network can it run?** The starter cities run in about a minute or two on a laptop with one simulated
traveller for every few people; [Running faster](speed.md) measures each setting on Delft and Amsterdam.

**Can I use my own network and demand?** Yes: OpenStreetMap, GMNS, link tables, trip tables and origin–destination
matrices. See [Bring your own network and demand](own_data.md).

**Can I write my own choice model?** Yes, in Python, without building anything; register it by name and a scenario
uses it like a built-in one. See [Choice](../concepts/choice.md).

**Can I run thousands of scenarios?** Yes: run them in a loop with `outputs="minimal"`, and label each with its
`design_id`. See [Results](results.md#many-runs-the-minimal-outputs).

**Will the same run give the same numbers?** On the same platform, byte for byte, whatever the number of threads;
across platforms, within stated tolerances. See [Reproducibility](../concepts/reproducibility.md).

**How are bikes modelled?** On a bike layer of their own, with separated tracks, painted lanes and mixed traffic told
apart in speed, route choice and mode choice; bikes do not slow the cars or one another. See [Cycling](cycling.md)
and [Limitations](../concepts/limitations.md).

**Where are the defaults from?** Each parameter's page row gives its source, or says it is an assumption; none is
calibrated. See [Parameters](../reference/parameters.md) and [Calibration](calibration.md).

**How do I cite it?** See [How to cite](../reference/citing.md).

**Where do I report a problem?** On the repository's
[issue tracker](https://github.com/bahmanmdd/openmobisim/issues), with the run's `manifest.json`.
