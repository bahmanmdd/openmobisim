"""The cases of the data collection: what each is, where its raw files come from, and its
study areas. Every other script reads this.

A **case** is one network (real or made up) with whatever else it has: layers, a timetable, parkings,
demand, counts. Its folder is ``cases/<key>/``:

    raw/          the files as downloaded
    processed/    networks and layers as GeoParquet tables (and GeoJSON when small), parkings, stops,
                  the clipped timetable, import reports, the suite's checks
    demand/       the demand used with it (synthetic unless said), with how it was made

The folders sit under the data root: the folder named by the environment variable
``OPENMOBISIM_RECIPE_DATA``, or a folder ``data`` beside the scripts.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path

from regions import (  # the study areas of the test networks, drawn in S172
    AMSTERDAM_A10,
    LYON_CENTRE,
    MANHATTAN_BELOW_60TH,
    PARIS_INSIDE_PERIPHERIQUE,
    PARIS_WITH_PERIPHERIQUE,
    STOCKHOLM_INNER_CITY,
)

#: The data root: the folder that holds ``cases/``, ``bundle/`` and ``sources/``. The environment variable
#: ``OPENMOBISIM_RECIPE_DATA`` names it; without it, it is a folder ``data`` beside the scripts.
DATA = Path(
    os.environ.get("OPENMOBISIM_RECIPE_DATA") or Path(__file__).resolve().parent / "data"
).expanduser().resolve()
CASES_DIR = DATA / "cases"

OSM = "ODbL 1.0 (© OpenStreetMap contributors)"
BBBIKE = "https://download.bbbike.org/osm/bbbike/{0}/{0}.osm.pbf"


@dataclass
class Raw:
    """A raw file: its name in ``raw/``, where it came from, when, and under which licence."""

    name: str
    url: str
    provider: str
    retrieved: str
    licence: str
    note: str = ""


@dataclass
class Case:
    """One case of the collection."""

    key: str
    title: str
    where: str
    kind: str  # "toy", "grid", "table", "osm"
    raw: list[Raw] = field(default_factory=list)
    #: Study areas by name: a polygon of (lon, lat), a (west, south, east, north) box, or None
    #: for the whole extract. The first is the case's main one.
    regions: dict[str, object] = field(default_factory=dict)
    #: The raw file of the network and of the timetable, if any.
    pbf: str | None = None
    gtfs: str | None = None
    #: Whether the case is in the test-network suite.
    suite: bool = False
    note: str = ""


def bbbike(city: str, retrieved: str) -> Raw:
    """The raw-file entry of a BBBike city extract."""
    return Raw(f"{city}.osm.pbf", BBBIKE.format(city), "BBBike city extract", retrieved, OSM)


CASES: dict[str, Case] = {
    c.key: c
    for c in [
        Case("toy", "The toy network", "made up (hand-verifiable)", "toy",
             note="Sixteen road nodes, bike and walk layers, hub H with parking, tram T1 and bus B1: every "
                  "number of its cases is derived by hand."),
        Case("grid", "Manhattan-style grids", "made up", "grid",
             note="The square grids of two-way streets the tests and benchmarks use "
                  "(`examples.manhattan_grid`); the 8 × 8, 200 m, signalised one is exported."),
        Case("siouxfalls", "Sioux Falls", "Sioux Falls, South Dakota, USA (TNTP)", "table",
             raw=[Raw("SiouxFalls_net.tntp", "https://raw.githubusercontent.com/bstabler/TransportationNetworks/master/SiouxFalls/SiouxFalls_net.tntp",
                      "TNTP (bstabler/TransportationNetworks)", "2026-09-22", "see the TNTP repository"),
                  Raw("SiouxFalls_flow.tntp", "https://raw.githubusercontent.com/bstabler/TransportationNetworks/master/SiouxFalls/SiouxFalls_flow.tntp",
                      "TNTP (bstabler/TransportationNetworks)", "2026-09-22", "see the TNTP repository"),
                  Raw("SiouxFalls_Dynamic.zip", "https://web.archive.org/web/20170102193824/http://www.bgu.ac.il/~bargera/tntp/SiouxFalls_Dynamic/SiouxFalls_Dynamic.zip",
                      "TNTP (Bar-Gera's site, via the Internet Archive, 2017-01-02 copy)", "2026-10-05",
                      "academic benchmark data, as TNTP",
                      note="a dynamic version of the network (km, veh/h, a time-varying OD); its link lengths give the case's real lengths")],
             note="The classic 24-node benchmark, read with `network_read_table`; for dynamic runs "
                  "with real lengths and a base demand, see the bundle's Sioux Falls case."),
        Case("nguyendupuis", "Nguyen–Dupuis", "made up (Nguyen and Dupuis, 1984)", "table",
             raw=[Raw("sensors-25-02262.xml", "https://www.ebi.ac.uk/europepmc/webservices/rest/PMC11991330/fullTextXML",
                      "Sensors 25(7) 2262 (2025), doi:10.3390/s25072262, via Europe PMC", "2026-10-06", "CC BY 4.0",
                      note="the article tabling the network's numbers (Tables 1 and 2)"),
                  Raw("links_table2.csv", "(from the article's Table 2)", "Sensors 25(7) 2262 (2025)", "2026-10-06",
                      "CC BY 4.0"),
                  Raw("od_table1.csv", "(from the article's Table 1)", "Sensors 25(7) 2262 (2025)", "2026-10-06",
                      "CC BY 4.0")],
             note="The 13-node, 19-link network of Nguyen and Dupuis (Transportation Science 18(2), 1984), a small "
                  "benchmark of route choice and assignment studies; made dynamic: "
                  "times in minutes, capacities and demand ×10 per hour, 1 km per minute, a schematic "
                  "layout. A starter case."),
        Case("andorra", "Andorra", "Andorra", "osm",
             raw=[Raw("andorra-260915.osm.pbf", "https://download.geofabrik.de/europe/andorra-latest.osm.pbf",
                      "Geofabrik", "2026-09-15", OSM)],
             regions={"country": None}, pbf="andorra-260915.osm.pbf", suite=True,
             note="Mountain valleys and a few main roads; the first figures and the CI extract."),
        Case("luxembourg", "Luxembourg", "Luxembourg", "osm",
             raw=[Raw("luxembourg-260915.osm.pbf", "https://download.geofabrik.de/europe/luxembourg-latest.osm.pbf",
                      "Geofabrik", "2026-09-15", OSM)],
             regions={"country": None}, pbf="luxembourg-260915.osm.pbf",
             note="A whole small country; the choice and profiling measurements."),
        Case("delft", "Delft", "Delft, Netherlands", "osm",
             raw=[Raw("zuid-holland-261011.osm.pbf", "https://download.geofabrik.de/europe/netherlands/zuid-holland-latest.osm.pbf",
                      "Geofabrik", "2026-10-11", OSM),
                  Raw("gtfs-nl-260928.zip", "http://gtfs.ovapi.nl/nl/gtfs-nl.zip", "OVapi (the national Dutch GTFS feed)",
                      "2026-09-28", "see gtfs.ovapi.nl (open data)", note="the same file as Amsterdam's (a hard link)"),
                  Raw("inweva-ochtendspits-2025.geojson",
                      "https://geo.rijkswaterstaat.nl/services/ogc/gdr/inweva/ows?service=WFS&version=2.0.0&request=GetFeature&typeNames=inweva:ochtendspits_intensiteiten_2025&outputFormat=application/json&srsName=EPSG:4326&bbox=51.945,4.295,52.055,4.430,urn:ogc:def:crs:EPSG::4326",
                      "Rijkswaterstaat, INWEVA 2025 morning peak (WFS)", "2026-10-12", "CC0 1.0",
                      note="motorway counts at the morning recipe's gateways; the box widened by 0.02 degrees")],
             regions={"municipality": (4.315, 51.965, 4.410, 52.035)},
             pbf="zuid-holland-261011.osm.pbf", gtfs="gtfs-nl-260928.zip",
             note="A small, lightly congested city, the fast starter case: it opens every notebook "
                  "with real cases (many studies use OmniTRANS's Delft network)."),
        Case("amsterdam", "Amsterdam", "Amsterdam, Netherlands", "osm",
             raw=[Raw("noord-holland-260924.osm.pbf", "https://download.geofabrik.de/europe/netherlands/noord-holland-latest.osm.pbf",
                      "Geofabrik", "2026-09-24", OSM),
                  Raw("gtfs-nl-260928.zip", "http://gtfs.ovapi.nl/nl/gtfs-nl.zip", "OVapi (the national Dutch GTFS feed)",
                      "2026-09-28", "see gtfs.ovapi.nl (open data)", note="all 40 Dutch agencies; clipped when read"),
                  Raw("inweva-ochtendspits-2025.geojson",
                      "https://geo.rijkswaterstaat.nl/services/ogc/gdr/inweva/ows?service=WFS&version=2.0.0&request=GetFeature&typeNames=inweva:ochtendspits_intensiteiten_2025&outputFormat=application/json&srsName=EPSG:4326&bbox=52.26,4.72,52.45,5.05,urn:ogc:def:crs:EPSG::4326",
                      "Rijkswaterstaat, INWEVA 2025 morning peak (WFS)", "2026-10-12", "CC0 1.0",
                      note="motorway counts at the morning recipe's gateways; the box widened by 0.02 degrees")],
             regions={"a10": AMSTERDAM_A10, "wide": (4.74, 52.28, 5.03, 52.43)},
             pbf="noord-holland-260924.osm.pbf", gtfs="gtfs-nl-260928.zip", suite=True,
             note="The gate city: inside the A10 (the suite), and the A10 plus 3–5 km."),
        Case("manhattan", "Manhattan below 60th Street", "New York, USA", "osm",
             raw=[bbbike("NewYork", "2026-09-21")], regions={"below_60th": MANHATTAN_BELOW_60TH},
             pbf="NewYork.osm.pbf", suite=True),
        Case("stockholm", "Stockholm inner city", "Stockholm, Sweden", "osm",
             raw=[bbbike("Stockholm", "2026-09-21")], regions={"inner_city": STOCKHOLM_INNER_CITY},
             pbf="Stockholm.osm.pbf", suite=True),
        Case("singapore", "Singapore central area", "Singapore", "osm",
             raw=[bbbike("Singapore", "2026-09-21")], regions={"central": (103.838, 1.276, 103.866, 1.304)},
             pbf="Singapore.osm.pbf", suite=True),
        Case("seoul", "Seoul: Gangnam and Seocho", "Seoul, South Korea", "osm",
             raw=[bbbike("Seoul", "2026-09-21")], regions={"gangnam_seocho": (127.00, 37.455, 127.10, 37.535)},
             pbf="Seoul.osm.pbf", suite=True),
        Case("sanfrancisco", "San Francisco", "San Francisco, USA", "osm",
             raw=[bbbike("SanFrancisco", "2026-09-21")], regions={"city": (-122.517, 37.708, -122.355, 37.832)},
             pbf="SanFrancisco.osm.pbf", suite=True),
        Case("zurich", "Zurich", "Zurich, Switzerland", "osm",
             raw=[bbbike("Zuerich", "2026-09-21")], regions={"city": (8.47, 47.34, 8.60, 47.42)},
             pbf="Zuerich.osm.pbf", suite=True),
        Case("copenhagen", "Copenhagen", "Copenhagen, Denmark", "osm",
             raw=[bbbike("Copenhagen", "2026-09-21")], pbf="Copenhagen.osm.pbf",
             note="Downloaded with the suite and not used yet: a cycling city, a candidate for later."),
        Case("portland", "Portland", "Portland, Oregon, USA", "osm",
             raw=[bbbike("Portland", "2026-09-21")], pbf="Portland.osm.pbf",
             note="Downloaded with the suite and not used yet."),
        Case("paris", "Paris inside the Périphérique", "Paris, France", "osm",
             raw=[Raw("paris-bbbike-260926.osm.pbf", BBBIKE.format("Paris"), "BBBike city extract", "2026-09-26", OSM),
                  Raw("idfm-gtfs-260930.zip", "https://eu.ftp.opendatasoft.com/stif/GTFS/IDFM-gtfs.zip",
                      "Île-de-France Mobilités (the regional GTFS feed)", "2026-09-30",
                      "Licence Mobilités (February 2021)", note="the whole region; clipped when read")],
             regions={"inside_peripherique": PARIS_INSIDE_PERIPHERIQUE,
                      "with_peripherique": PARIS_WITH_PERIPHERIQUE},
             pbf="paris-bbbike-260926.osm.pbf", gtfs="idfm-gtfs-260930.zip", suite=True,
             note="A case study candidate."),
        Case("lyon", "Lyon and Villeurbanne", "Lyon, France", "osm",
             raw=[Raw("lyon-bbbike-260926.osm.pbf", BBBIKE.format("Lyon"), "BBBike city extract", "2026-09-26", OSM),
                  Raw("tcl-gtfs-260930.zip", "https://www.data.gouv.fr/api/1/datasets/r/abebedc6-28cf-4e2e-9c64-db57a40156f8",
                      "TCL via data.gouv.fr (the 'GTFS modifié' resource of the TCL dataset)", "2026-09-30",
                      "Licence Mobilités (February 2021)",
                      note="the official theoretical timetable needs a Grand Lyon data account")],
             regions={"centre": LYON_CENTRE},
             pbf="lyon-bbbike-260926.osm.pbf", gtfs="tcl-gtfs-260930.zip", suite=True,
             note="A case study candidate."),
    ]
}  # fmt: skip


def case_dir(key: str) -> Path:
    """The folder of a case under the data root."""
    return CASES_DIR / key


def raw_path(key: str, name: str) -> Path:
    """The path of one of a case's raw files."""
    return CASES_DIR / key / "raw" / name
