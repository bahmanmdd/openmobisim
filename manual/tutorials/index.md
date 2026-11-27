# Tutorials

Two notebooks, shown with the outputs of their last run (the run times they show were measured on a MacBook Pro with
an Apple M2 Pro, 12 cores and 32 GB of memory). Each runs from start to end as shown with
`pip install openmobisim pyarrow`, and downloads its case itself.

Until release 0.2 is on PyPI, `pip install openmobisim` gives release 0.1, which has another interface; 0.2 is then
built from the repository with a Rust toolchain: in a virtual environment, `pip install maturin pyarrow`, then
`maturin develop --release` in the repository.

- **[Delft: a first run](delft.ipynb).** Download a case, run it with one command and read what the run wrote: who
  took which mode and how long it took, how the iterations settled, the traffic on the links for cars, bikes and
  walkers, one traveller's journey leg by leg, the map and the summary.
- **[Amsterdam: a case built in Python](amsterdam.ipynb).** The same steps written out one by one on a larger case,
  the way you would with tables of your own: the network from tables, the timetable from GTFS, the travellers from a
  table, then the run and its results by class.

```{toctree}
:maxdepth: 1

delft
amsterdam
```
