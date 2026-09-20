"""Figures for openmobisim runs, in one recognisable look (S162, S163).

``pip install 'openmobisim[viz]'``. Functions are named with their kind first,
so related ones sort and complete together: ``map_*`` draws on a map,
``chart_*`` draws a plot with axes.

* :func:`map_link` — a run's traffic on its network: one ribbon per direction
  of every link, width = volume, colour = delay over free flow.
* :func:`map_demand` — demand as origin-destination desire lines.
* :func:`chart_demand_matrix` — the origin-destination matrix as a heat-map.

matplotlib is imported when a figure is drawn, not when this package is, so
``import openmobisim.viz`` works without it.
"""

from openmobisim.viz._demand import chart_demand_matrix, map_demand
from openmobisim.viz._link import map_link

__all__ = ["chart_demand_matrix", "map_demand", "map_link"]
