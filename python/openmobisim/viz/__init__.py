"""Figures for openmobisim runs, in one recognisable look.

``pip install 'openmobisim[viz]'``. Functions are named with their kind first,
so related ones sort and complete together: ``map_*`` draws on a map,
``chart_*`` draws a plot with axes.

* :func:`map_link` — a run's traffic on its network: one ribbon per direction
  of every link, width = volume, colour = delay over free flow.
* :func:`map_demand` — demand as origin-destination desire lines.
* :func:`map_route` — the alternative routes of one origin-destination pair.
* :func:`map_transit` — a run's transit: lines by kind, width = passengers on
  board, stops as hubs.
* :func:`map_parking` — a run's parkings: car parks and bike parkings, sized by
  capacity, coloured by how full they got.
* :func:`map_interactive` — a run and its route sets as one interactive HTML file:
  zoom levels, time, colour, hover, and route inspection. Needs no matplotlib.
* :func:`chart_demand_matrix` — the origin-destination matrix as a heat-map.
* :func:`chart_mode_share` — the share of the trips each mode took, by departure
  time.
* :func:`summary` — a run's summary as one self-contained HTML page: headline
  numbers, the flow map, modes, convergence, where the time went, settings.

matplotlib is imported when a figure is drawn, not when this package is, so
``import openmobisim.viz`` works without it.
"""

from openmobisim.viz._demand import chart_demand_matrix, map_demand
from openmobisim.viz._interactive import map_interactive
from openmobisim.viz._link import map_link
from openmobisim.viz._modes import chart_mode_share
from openmobisim.viz._parking import map_parking
from openmobisim.viz._route import map_route
from openmobisim.viz._summary import summary
from openmobisim.viz._transit import map_transit

__all__ = [
    "chart_demand_matrix",
    "chart_mode_share",
    "map_demand",
    "map_interactive",
    "map_link",
    "map_parking",
    "map_route",
    "map_transit",
    "summary",
]
