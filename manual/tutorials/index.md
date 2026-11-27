# Tutorials

Two notebooks, shown with their outputs. Each needs `pip install openmobisim pandas pyarrow` and downloads its case
itself.

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
