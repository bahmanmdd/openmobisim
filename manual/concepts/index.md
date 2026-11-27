# Concepts and methods

How the model works: each method in plain words, then its equations, its defaults, what it means for bikes, its limits
and its references. The pages are in a reading order: the network, the demand, the routes, the choice, transit, the
loading and the iteration. This reading order is not the order in which the modules depend on
one another (the loading is below the search, and the search below the demand), which
[How a run works](architecture.md#inside-the-simulator) gives.

## Figures

The diagrams of the concepts, in one place; each is explained on the page named under it.

### The two parts: Python and Rust

What you use, Python, and what computes, Rust, each module using only modules below it or beside it in its row. See
[How a run works](architecture.md#the-two-parts-python-and-rust).

```{image} ../_static/figures/architecture_paper.svg
:alt: The architecture: the Python module you use, the compiled bridge, and the Rust modules beneath, each using only modules below it or beside it in its row
:class: only-light
:width: 100%
```
```{image} ../_static/figures/architecture_night.svg
:alt: The architecture: the Python module you use, the compiled bridge, and the Rust modules beneath, each using only modules below it or beside it in its row
:class: only-dark
:width: 100%
```

### From files to results

Each input read into an object, a run turning them into results, and the pages written from the run. See
[How a run works](architecture.md#data-flow-from-files-to-results).

```{image} ../_static/figures/dataflow_paper.svg
:alt: The data flow: input files are read into a network, a timetable and the travellers; a run turns them into four tables and a record; the map and the summary are written from the run
:class: only-light
:width: 100%
```
```{image} ../_static/figures/dataflow_night.svg
:alt: The data flow: input files are read into a network, a timetable and the travellers; a run turns them into four tables and a record; the map and the summary are written from the run
:class: only-dark
:width: 100%
```

### One run, iteration by iteration

What is done once, then the loop of search, choose, load, measure and assess until the run settles. See
[How a run works](architecture.md#interactions-in-a-run) and [The iteration](iteration.md).

```{image} ../_static/figures/run_loop_paper.svg
:alt: A run: the inputs built once, then the loop of search, choose, load, measure and assess until the stopping rule or the cap, then the results written
:class: only-light
:width: 100%
```
```{image} ../_static/figures/run_loop_night.svg
:alt: A run: the inputs built once, then the loop of search, choose, load, measure and assess until the stopping rule or the cap, then the results written
:class: only-dark
:width: 100%
```

### Modes and legs

The four modes as chains of legs from door to door. See [How a run works](architecture.md#one-trip-leg-by-leg).

```{image} ../_static/figures/modes_paper.svg
:alt: The four modes as chains of legs from door to door
:class: only-light
:width: 100%
```
```{image} ../_static/figures/modes_night.svg
:alt: The four modes as chains of legs from door to door
:class: only-dark
:width: 100%
```

### The modules

Which part of the Rust core uses which. See [The modules](../reference/crates.md).

```{image} ../_static/figures/crates_paper.svg
:alt: Which module uses which: an arrow points from a module to one it uses directly
:class: only-light
:width: 100%
```
```{image} ../_static/figures/crates_night.svg
:alt: Which module uses which: an arrow points from a module to one it uses directly
:class: only-dark
:width: 100%
```

## The pages

```{toctree}
:maxdepth: 1

architecture
terms
networks
demand
search
choice
transit
loading
iteration
outputs
reproducibility
validation
limitations
```
