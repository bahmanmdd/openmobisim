# Concepts and methods

How the model works: each method in plain words, then its equations, its defaults, what it means for bikes, its limits and its references.

## Figures

The diagrams of the concepts, in one place; each is explained on the page named under it.

### The two layers

What you use, Python, and what computes, Rust. See [How a run works](architecture.md#the-two-layers).

```{image} ../_static/figures/architecture_paper.svg
:alt: The architecture: the Python package you use, the compiled bridge, and the Rust crates beneath
:class: only-light
:width: 100%
```
```{image} ../_static/figures/architecture_night.svg
:alt: The architecture: the Python package you use, the compiled bridge, and the Rust crates beneath
:class: only-dark
:width: 100%
```

### From files to results

Each input file read into an object, a scenario combining them with the settings, and a run turning them into results. See [How a run works](architecture.md#data-flow-from-files-to-results).

```{image} ../_static/figures/dataflow_paper.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, a run turns them into a Run object and output files, and the figures are drawn from the Run
:class: only-light
:width: 100%
```
```{image} ../_static/figures/dataflow_night.svg
:alt: The data flow: input files are read into four objects, a Scenario combines them with the settings, a run turns them into a Run object and output files, and the figures are drawn from the Run
:class: only-dark
:width: 100%
```

### One run, loading by loading

The stages a run does once, then the loop of choice, loading, route update and gap until it settles. See [How a run works](architecture.md#interactions-in-a-run).

```{image} ../_static/figures/run_loop_paper.svg
:alt: A run: the stages done once at the start, the loop of choice, loading, route update and gap until convergence, and the end
:class: only-light
:width: 100%
```
```{image} ../_static/figures/run_loop_night.svg
:alt: A run: the stages done once at the start, the loop of choice, loading, route update and gap until convergence, and the end
:class: only-dark
:width: 100%
```

### Modes, legs and hubs

The six modes as chains of legs, each on its layer, joined at stops and parkings. See [Modes and itineraries](modes.md).

```{image} ../_static/figures/modes_paper.svg
:alt: The six modes as chains of legs from origin to destination, each leg on its layer, with the stops and parkings where a trip changes layer
:class: only-light
:width: 100%
```
```{image} ../_static/figures/modes_night.svg
:alt: The six modes as chains of legs from origin to destination, each leg on its layer, with the stops and parkings where a trip changes layer
:class: only-dark
:width: 100%
```

### The crates

Which part of the Rust core uses which. See [The Rust crates](../reference/crates.md).

```{image} ../_static/figures/crates_paper.svg
:alt: Which crate uses which: an arrow points from a crate to one it depends on
:class: only-light
:width: 100%
```
```{image} ../_static/figures/crates_night.svg
:alt: Which crate uses which: an arrow points from a crate to one it depends on
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
modes
route_sets
choice
transit
parking
loading
equilibration
outputs
reproducibility
validation
limitations
```
