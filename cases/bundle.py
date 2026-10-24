"""The starter bundle: the files a reader needs for the tutorial
notebooks and the default scenarios, small, versioned, with checksums and licences.

    <python with openmobisim>  bundle.py date amsterdam     # the service day: the full feed's busiest weekday
    <python with osmium>       bundle.py clip amsterdam     # OSM extract and GTFS feed clipped to the study area
    <python with openmobisim>  bundle.py check amsterdam    # the clipped files read as the full ones do
    <python with openmobisim>  bundle.py pack               # bundle/<version>/: every case, MANIFEST.json
    <python with openmobisim>  bundle.py tables delft       # the network as GMNS and GeoParquet: <case>-tables.zip

``clip`` needs pyosmium (``pip install osmium``), kept out of the project's environment. Output goes to
``bundle/<version>/<case>/`` of the data root (rebuilt from ``raw/`` by this script).

**A clip keeps what a read of the study area needs**, so the network and the timetable read from the clipped files are
the ones read from the full downloads (``check`` asserts it: the same network fingerprint, the same timetable):

- **OpenStreetMap**: every way with a node inside the area's box widened by ``MARGIN_M`` (3 km: the reader keeps main
  roads up to 2 km past a study area and each road's first node past it), with all its nodes (complete ways,
  tags kept), and every node inside the widened box. Relations are left out: the reader does not read them.
- **GTFS**: the service day fixed in the case's ``case.json`` (the full feed's busiest weekday, as ``transit_read_gtfs``
  finds it; a clipped feed's may differ), and on it every trip calling at a stop inside the widened box, with all its
  calls and stops (so times interpolated between calls stay the same), their routes, agencies, services, transfers and
  frequencies.
"""

from __future__ import annotations

import csv
import io
import json
import math
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from cases import CASES, DATA, raw_path  # noqa: E402

VERSION = "v1"
# The benchmarks' prices (S254): none, so money (at its default weight) changes nothing in them.
NO_PRICES = {"car_eur_km": 0.0}
# The benchmarks record their link times by 15 minutes (S269), so that their summaries and interactive maps show
# the traffic.
BENCH_BINS = {"link_bin_s": 900}
#: A city's own scenario settings over `BASE_CITY_2` (S282): Paris stops at a gap of 4 %, so that its run, which stops
#: on the in-set gap, also ends with the verdict "good".
CITY_BASE: dict[str, dict] = {"paris": {"equilibration_options": {"gap_tolerance": 0.04}}}
OUT = DATA / "bundle" / VERSION
MARGIN_M = 3000.0
#: A case's own margin on the south (S268): Delft's south part had some tails that go long in isolation;
#: its extract stops 300 m past the study area there, its ways cut at the box, so the motorways' carriageways end
#: there (joined by turnarounds) instead of running 2 km on through empty polder.
MARGIN_SOUTH_M = {"delft": 300.0}

#: The cities' study areas, by the case's region key; the box a clip keeps is their bounding box, widened.
# Paris with its ring road (S268), so that the Portes' junctions are not cut off.
AREAS = {"delft": "municipality", "amsterdam": "wide", "paris": "with_peripherique", "lyon": "centre"}


def area_box(case: str) -> tuple[float, float, float, float]:
    """The study area's bounding box widened by ``MARGIN_M``: (west, south, east, north)."""
    region = CASES[case].regions[AREAS[case]]
    if isinstance(region, tuple) and len(region) == 4:
        w, s, e, n = region
    else:
        lons, lats = [p[0] for p in region], [p[1] for p in region]
        w, s, e, n = min(lons), min(lats), max(lons), max(lats)
    dlat = MARGIN_M / 110_574.0
    dlon = MARGIN_M / (111_320.0 * math.cos(math.radians((s + n) / 2)))
    south = MARGIN_SOUTH_M.get(case, MARGIN_M) / 110_574.0
    return (w - dlon, s - south, e + dlon, n + dlat)


def inside(box: tuple[float, float, float, float], lon: float, lat: float) -> bool:
    """Whether the point (lon, lat) lies in the box (west, south, east, north)."""
    return box[0] <= lon <= box[2] and box[1] <= lat <= box[3]


# --- OpenStreetMap -------------------------------------------------------------------------------


def clip_pbf(source: Path, target: Path, box: tuple[float, float, float, float], cut: bool = False) -> dict:
    """The extract's part in `box`: every way with a node inside, whole; with `cut`, only the nodes inside, so
    the reader cuts each way at the box (a node an extract lacks is a cut, as at any extract's edge).
    """
    import osmium

    target.parent.mkdir(parents=True, exist_ok=True)
    target.unlink(missing_ok=True)
    counts = {"nodes_inside": 0, "ways": 0}
    writer_of = (lambda: osmium.SimpleWriter(str(target))) if cut else (
        lambda: osmium.BackReferenceWriter(str(target), ref_src=str(source), remove_tags=False))
    with writer_of() as writer:
        for obj in osmium.FileProcessor(str(source)).with_locations():
            if obj.is_node():
                if obj.location.valid() and inside(box, obj.location.lon, obj.location.lat):
                    writer.add_node(obj)
                    counts["nodes_inside"] += 1
            elif obj.is_way() and any(n.location.valid() and inside(box, n.lon, n.lat) for n in obj.nodes):
                writer.add_way(obj)
                counts["ways"] += 1
    return counts


# --- GTFS ------------------------------------------------------------------------------------------


def _rows(z: zipfile.ZipFile, name: str):
    """The rows of a feed's file as dicts, and its header; none if the file is not there."""
    names = {Path(n).name: n for n in z.namelist()}
    if name not in names:
        return [], []
    with z.open(names[name]) as raw:
        reader = csv.DictReader(io.TextIOWrapper(raw, encoding="utf-8-sig", newline=""))
        rows = list(reader)
        return rows, reader.fieldnames or []


def _stream(z: zipfile.ZipFile, name: str):
    names = {Path(n).name: n for n in z.namelist()}
    with z.open(names[name]) as raw:
        reader = csv.DictReader(io.TextIOWrapper(raw, encoding="utf-8-sig", newline=""))
        yield reader.fieldnames or []
        yield from reader


def services_on(z: zipfile.ZipFile, day: str) -> set[str]:
    """The service ids running on ``day`` (``YYYYMMDD``): ``calendar.txt``, then ``calendar_dates.txt``."""
    import datetime as dt

    date = dt.date(int(day[:4]), int(day[4:6]), int(day[6:]))
    weekday = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"][date.weekday()]
    on: set[str] = set()
    rows, _ = _rows(z, "calendar.txt")
    for r in rows:
        if r["start_date"] <= day <= r["end_date"] and r.get(weekday) == "1":
            on.add(r["service_id"])
    rows, _ = _rows(z, "calendar_dates.txt")
    for r in rows:
        if r["date"] == day:
            if r["exception_type"] == "1":
                on.add(r["service_id"])
            elif r["exception_type"] == "2":
                on.discard(r["service_id"])
    return on


#: The service a city's timetable keeps (S269): the runs with
#: a stop time from 05:00 to 12:00. The mornings depart 06:30-09:30 and a journey with transit is offered up to two hours
#: (`journey_max_s`), so every rider is served; the day's later buses stay off the roads of every run.
SERVICE_WINDOW_S = (5 * 3600, 12 * 3600)


def _gtfs_s(t: str) -> int | None:
    """A GTFS time ("25:10:00" is past midnight) in seconds, or None."""
    try:
        h, m, sec = (int(x) for x in t.strip().split(":"))
    except ValueError:
        return None
    return h * 3600 + m * 60 + sec


def clip_gtfs(source: Path, target: Path, box: tuple[float, float, float, float], day: str,
              window: tuple[int, int] | None = SERVICE_WINDOW_S) -> dict:
    """The day's runs that call in `box`, and with `window`, have a stop time within it."""
    target.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(source) as z:
        running = services_on(z, day)
        trips_all, _ = _rows(z, "trips.txt")
        on_day = {t["trip_id"] for t in trips_all if t["service_id"] in running}
        del trips_all
        stops, stop_fields = _rows(z, "stops.txt")
        def lonlat(s):
            try:
                return float(s["stop_lon"]), float(s["stop_lat"])
            except (KeyError, ValueError):
                return None
        near = {s["stop_id"] for s in stops if (p := lonlat(s)) and inside(box, *p)}
        # The trips that call inside, then all their calls and stops.
        trips_kept: set[str] = set()
        it = _stream(z, "stop_times.txt")
        st_fields = next(it)
        for r in it:
            if r["stop_id"] in near and r["trip_id"] in on_day:
                trips_kept.add(r["trip_id"])
        calls: list[dict] = []
        stops_used: set[str] = set()
        it = _stream(z, "stop_times.txt")
        next(it)
        for r in it:
            if r["trip_id"] in trips_kept:
                calls.append(r)
        if window is not None:
            # A run is kept if one of its stop times falls in the window.
            served: set[str] = set()
            for r in calls:
                for t in (r.get("arrival_time", ""), r.get("departure_time", "")):
                    x = _gtfs_s(t) if t else None
                    if x is not None and window[0] <= x <= window[1]:
                        served.add(r["trip_id"])
                        break
            trips_kept &= served
            calls = [r for r in calls if r["trip_id"] in trips_kept]
        for r in calls:
            stops_used.add(r["stop_id"])
        by_id = {s["stop_id"]: s for s in stops}
        parents = {by_id[s].get("parent_station", "") for s in stops_used if s in by_id} - {""}
        stops_out = [s for s in stops if s["stop_id"] in stops_used or s["stop_id"] in parents]
        trips, trip_fields = _rows(z, "trips.txt")
        trips_out = [t for t in trips if t["trip_id"] in trips_kept]
        routes_used = {t["route_id"] for t in trips_out}
        services = {t["service_id"] for t in trips_out}
        routes, route_fields = _rows(z, "routes.txt")
        routes_out = [r for r in routes if r["route_id"] in routes_used]
        agencies_used = {r.get("agency_id", "") for r in routes_out}
        files: dict[str, tuple[list[str], list[dict]]] = {
            "stops.txt": (stop_fields, stops_out),
            "stop_times.txt": (st_fields, calls),
            "trips.txt": (trip_fields, trips_out),
            "routes.txt": (route_fields, routes_out),
        }
        agency, agency_fields = _rows(z, "agency.txt")
        files["agency.txt"] = (agency_fields, [a for a in agency if a.get("agency_id", "") in agencies_used
                                               or len(agency) == 1])
        # The day's services: in calendar.txt only on the day itself, its exceptions on that day.
        rows, fields = _rows(z, "calendar.txt")
        if fields:
            files["calendar.txt"] = (fields, [{**r, "start_date": day, "end_date": day} for r in rows
                                              if r["service_id"] in services])
        rows, fields = _rows(z, "calendar_dates.txt")
        if fields:
            files["calendar_dates.txt"] = (fields, [r for r in rows if r["service_id"] in services
                                                    and r["date"] == day])
        kept_stops = {s["stop_id"] for s in stops_out}
        rows, fields = _rows(z, "transfers.txt")
        if fields:
            files["transfers.txt"] = (fields, [r for r in rows if r.get("from_stop_id") in kept_stops
                                               and r.get("to_stop_id") in kept_stops])
        rows, fields = _rows(z, "frequencies.txt")
        if fields:
            files["frequencies.txt"] = (fields, [r for r in rows if r["trip_id"] in trips_kept])
        for name in ("feed_info.txt", "attributions.txt"):
            rows, fields = _rows(z, name)
            if fields:
                files[name] = (fields, rows)
    target.unlink(missing_ok=True)
    with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED) as out:
        for name, (fields, rows) in files.items():
            buf = io.StringIO()
            w = csv.DictWriter(buf, fieldnames=fields, lineterminator="\n", extrasaction="ignore")
            w.writeheader()
            w.writerows(rows)
            out.writestr(name, buf.getvalue())
    return {"stops": len(stops_out), "trips": len(trips_out), "calls": len(calls), "routes": len(routes_out)}


# --- commands ------------------------------------------------------------------------------------


def case_json(case: str) -> dict:
    """The case's ``case.json`` in the bundle folder; empty if there is none."""
    path = OUT / case / "case.json"
    return json.loads(path.read_text()) if path.exists() else {}


def write_case_json(key: str, **fields: object) -> None:
    """Merge ``fields`` into the case's ``case.json`` (made if it is missing)."""
    (OUT / key).mkdir(parents=True, exist_ok=True)
    data = {**case_json(key), **fields}
    (OUT / key / "case.json").write_text(json.dumps(data, indent=1))


def full_network(case: str):
    """The case's network, read from its full (unclipped) extract for the study area."""
    import openmobisim as ms

    c = CASES[case]
    return ms.network_read_osm(str(raw_path(case, c.pbf)), region=c.regions[AREAS[case]])


def date(case: str) -> None:
    """The full feed's busiest weekday, as the reader finds it, into ``case.json``."""
    import openmobisim as ms

    c = CASES[case]
    transit = ms.transit_read_gtfs(str(raw_path(case, c.gtfs)), network=full_network(case))
    region = c.regions[AREAS[case]]
    write_case_json(case, case=case, date=transit.date, region_key=AREAS[case],
                    region=list(region) if isinstance(region, tuple) else [list(p) for p in region])
    print(case, transit.date)


def clip(case: str) -> None:
    """Clip the case's OSM extract and GTFS feed to its study area; ``clip.json`` records what was kept."""
    c = CASES[case]
    box = area_box(case)
    folder = OUT / case
    report = {"case": case, "box": box, "margin_m": MARGIN_M,
              "pbf": clip_pbf(raw_path(case, c.pbf), folder / "network.osm.pbf", box, cut=case in MARGIN_SOUTH_M)}
    if c.gtfs:
        day = case_json(case)["date"].replace("-", "")
        report["gtfs"] = clip_gtfs(raw_path(case, c.gtfs), folder / "gtfs.zip", box, day)
    (folder / "clip.json").write_text(json.dumps(report, indent=1))
    print(json.dumps(report))


def fingerprints(net, transit) -> dict:
    """What identifies a network and a timetable: a one-trip run's network fingerprint and run fingerprint."""
    import openmobisim as ms

    lines = transit.lines() if transit is not None else None
    stops = transit.stops() if transit is not None else None
    return {
        "links": net.link_count, "nodes": net.node_count,
        "lines": None if lines is None else len(lines["route_id"]),
        "runs": None if transit is None else transit.run_count,
        "calls": None if transit is None else transit.call_count,
        "stops": None if stops is None else len(stops["stop_id"]),
        "run_fingerprint": _one_trip_fingerprint(ms, net, transit),
    }


def _one_trip_fingerprint(ms, net, transit) -> tuple[str, str]:
    lon, lat = net.link_geometry()[0][0]
    row = [("t", 0, float(lon), float(lat), float(lon), float(lat), 8 * 3600, "x", None, "car")]
    run = ms.Scenario.from_parts(net, row, classes={"x": {"modes": ["car"]}}, transit=transit,
                                 equilibration="free_flow", flow_level=0).run("bundle-check", quiet=True)
    return run.manifest()["network_fingerprint"], run.fingerprint


def check(case: str) -> None:
    """The clipped files read as the full ones: the same network and the same timetable on the case's day."""
    import openmobisim as ms

    c = CASES[case]
    region = c.regions[AREAS[case]]
    day = case_json(case).get("date")
    full = full_network(case)
    clipped = ms.network_read_osm(str(OUT / case / "network.osm.pbf"), region=region)
    t_full = ms.transit_read_gtfs(str(raw_path(case, c.gtfs)), network=full, date=day) if c.gtfs else None
    t_clip = ms.transit_read_gtfs(str(OUT / case / "gtfs.zip"), network=clipped, date=day) if c.gtfs else None
    a, b = fingerprints(full, t_full), fingerprints(clipped, t_clip)
    p_full = ms.parking_read_osm(str(raw_path(case, c.pbf)), region=region)
    p_clip = ms.parking_read_osm(str(OUT / case / "network.osm.pbf"), region=region)
    a["parkings"], b["parkings"] = p_full.count, p_clip.count
    same = a == b
    write_case_json(case, checked={"same": same, "full": a, "clipped": b})
    print(json.dumps({"case": case, "same": same, "full": a, "clipped": b}))
    if not same:
        raise SystemExit(f"{case}: the clipped files do not read as the full ones")


# --- pack ------------------------------------------------------------------------------------------

# The cities' titles are their names (S268): simple, nothing else.
TITLES = {
    "delft": "Delft",
    "amsterdam": "Amsterdam",
    "paris": "Paris",
    "lyon": "Lyon",
    "siouxfalls": "Sioux Falls",
    "nguyendupuis": "Nguyen–Dupuis",
}
OSM_LICENCE = "ODbL 1.0: © OpenStreetMap contributors (https://www.openstreetmap.org/copyright)"
FEEDS = {
    # S245: checked against each publisher. The Dutch feed: CC0 (OVapi's announcement of the feed, 2014: "licensed under
    # CC0"; OVapi asks that nobody claims to represent the agencies). Both French feeds: the "Licence Mobilités"
    # (February 2021; the publishers' own catalogues): a derived database stays under it (5.5), its text or link goes
    # with the data (5.3), anything published from it carries the notice of 5.4.
    "amsterdam": ("the Dutch national GTFS feed (gtfs.ovapi.nl, published by OVapi from the NDOV data of the Dutch "
                  "transit agencies; the agencies do not endorse this use)", "CC0 1.0 (OVapi)"),
    "delft": ("the Dutch national GTFS feed (gtfs.ovapi.nl, published by OVapi from the NDOV data of the Dutch "
              "transit agencies; the agencies do not endorse this use)", "CC0 1.0 (OVapi)"),
    "paris": ("Île-de-France Mobilités' GTFS feed, 'Horaires prévus sur les lignes de transport en commun "
              "d'Ile-de-France (GTFS Datahub)' (data.iledefrance-mobilites.fr), one service day, clipped",
              "Licence Mobilités (February 2021, https://cloud.fabmob.io/s/eYWWJBdM3fQiFNm): contains information from "
              "Île-de-France Mobilités' GTFS Datahub, made available under the « Licence Mobilités »"),
    "lyon": ("SYTRAL Mobilités' TCL timetable, 'Horaires théoriques du réseau Transports en Commun Lyonnais' "
             "(data.gouv.fr dataset 5de42c4d8b4c417a10e62ec9, its 'GTFS modifié' resource), one service day, clipped",
             "Licence Mobilités (February 2021, https://cloud.fabmob.io/s/eYWWJBdM3fQiFNm), as SYTRAL Mobilités states "
             "it: contains information from the TCL timetable, made available under the « Licence Mobilités »"),
}
FROM_OD_CITY = {"start_s": 7 * 3600, "end_s": 9 * 3600, "profile": "am_peak", "spread_m": 500.0}
# S265: the second morning recipe (`morning2.py`): 06:30-09:30 with the same bell (16 % of the day's trips in 07:00-09:00,
# 10 % in 07:30-08:30), each simulated traveller standing for WEIGHT people, the base scenario's per-link results in
# 15-minute bins (the summary page and the map show the traffic; `openmobisim run` then matches the case's stored
# result summary). Trip ends at the zone's own points (`zone_points.csv`, by residents and workplaces; S282), the same
# for all runs and scenarios, so that travellers between the same zones share their route searches; `spread_m` places
# a zone without points (none in the cities).
FROM_OD_CITY_2 = {"start_s": 6 * 3600 + 1800, "end_s": 9 * 3600 + 1800, "profile": "am_peak", "spread_m": 500.0}
WEIGHT = {"delft": 1, "amsterdam": 5, "lyon": 5, "paris": 10}  # Delft, the smallest: every traveller simulated
# S267: the searches shared by origin and half hour (the route update's search trees, the park-and-ride drives), and
# the run stopping once its gap is good (5 %, the verdict's threshold).
BASE_CITY_2 = {"link_bin_s": 900, "route_update_options": {"tree_step_s": 1800},
               "parking_options": {"reach_car_step_s": 1800}, "equilibration_options": {"gap_tolerance": 0.05}}
GHSL_LICENCE = ("CC BY 4.0, (c) European Union, 1995-2026 (https://creativecommons.org/licenses/by/4.0/); changed: "
                "summed by 1 km zone")


def _copy(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(source.read_bytes())


def _car_classes(folder: Path) -> None:
    (folder / "demand").mkdir(parents=True, exist_ok=True)
    (folder / "demand" / "classes.csv").write_text("class,share,modes\ndefault,1,car\n")


def pack_city(case: str) -> None:
    """Copy a city's morning demand into its bundle folder and complete its ``case.json``."""
    folder = OUT / case
    morning = DATA / "cases" / case / "demand" / "morning2"
    for name in ("od.csv", "zones.csv", "zone_points.csv", "classes.csv", "morning.json"):  # morning.json: how it was made
        _copy(morning / name, folder / "demand" / name)
    # The motorway gateways' counts (S268): for validation, and for other tools.
    if (morning / "gateway_counts.csv").exists():
        _copy(morning / "gateway_counts.csv", folder / "demand" / "gateway_counts.csv")
    if case in ("paris", "lyon"):
        # The Licence Mobilités goes with the data it covers (its article 5.3; S245).
        _copy(DATA / "licences" / "licence_mobilites_2021-02.pdf", folder / "LICENCE_MOBILITES.pdf")
    info = case_json(case)
    feed, feed_licence = FEEDS[case]
    write_case_json(
        case, case=case, title=TITLES[case], kind="osm", bundle=VERSION,
        network={"pbf": "network.osm.pbf", "region": info["region"]},
        transit={"gtfs": "gtfs.zip", "date": info["date"]},
        parkings=True,
        # The study period, strict; the matrix stays whole and the run keeps the trips departing in
        # its first hour, at their own times (S274, S278: the selection is the package's, at load).
        window=["07:30", "09:30"], departures=["07:30", "08:30"],
        demand={"od": "demand/od.csv", "zones": "demand/zones.csv", "zone_points": "demand/zone_points.csv",
                "classes": "demand/classes.csv", "from_od": FROM_OD_CITY_2, "weight": WEIGHT[case]},
        scenarios={"base": {**BASE_CITY_2, **CITY_BASE.get(case, {})}},
        sources=[{"what": "the road, bike and walk networks and the parkings", "from": "OpenStreetMap",
                  "licence": OSM_LICENCE},
                 {"what": f"the timetable of {info['date']}, its runs from 05:00 to 12:00", "from": feed,
                  "licence": feed_licence},
                 {"what": "each zone's residents and non-residential built volume (demand/zones.csv), and the "
                          "points within each zone where its trips start, weighted by residents, and end, weighted by "
                          "non-residential volume and residents (demand/zone_points.csv)",
                  "from": "the Global Human Settlement Layer R2023A, epoch 2020: GHS-POP and GHS-BUILT-V NRES at "
                          "3 arc-seconds (European Commission, Joint Research Centre; "
                          "https://human-settlement.emergency.copernicus.eu/)",
                  "licence": GHSL_LICENCE},
                 *([{"what": "the motorway gateways' morning counts (demand/gateway_counts.csv), from which the "
                             "through traffic is sized",
                     "from": "Rijkswaterstaat, INWEVA 2025, morning peak (https://geo.rijkswaterstaat.nl/services/ogc/"
                             "gdr/inweva/ows, layer ochtendspits_intensiteiten_2025)",
                     "licence": "CC0 1.0 (https://creativecommons.org/publicdomain/zero/1.0/)"}]
                   if case in ("delft", "amsterdam") else []),
                 # S282: the French gateways by a share of capacity, 0.35, tuned to TomTom's congestion.
                 *([{"what": "the motorway gateways' morning flows (demand/gateway_counts.csv), from which the "
                             "through traffic is sized: 0.35 of each carriageway's capacity, an assumption tuned "
                             "so that the morning's congestion stays slightly below TomTom's (the median of "
                             "Amsterdam's counted gateways, 0.45, put it above)",
                     "from": "synthetic: openmobisim's morning recipe (cases/morning2.py), from Rijkswaterstaat's "
                             "INWEVA 2025 morning counts at Amsterdam's gateways",
                     "licence": "CC BY 4.0"}]
                   if case in ("paris", "lyon") else []),
                 {"what": "the morning (zones, OD matrix, trips from outside, goods vehicles) and the traveller "
                          "classes",
                  "from": "synthetic: openmobisim's morning recipe (cases/morning2.py; totals from the city's "
                          "travel survey; demand/morning.json says how; not a forecast)", "licence": "CC BY 4.0"}],
    )


def pack_siouxfalls() -> None:
    """Make the Sioux Falls case's bundle folder from its processed tables."""
    folder = OUT / "siouxfalls"
    src = DATA / "cases" / "siouxfalls"
    with (src / "processed" / "links_metric.csv").open() as f:
        rows = list(csv.DictReader(f))
    (folder / "network").mkdir(parents=True, exist_ok=True)
    with (folder / "network" / "links.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["from", "to", "length", "free_flow_time", "capacity", "lanes"])
        for r in rows:
            w.writerow([r["from"], r["to"], r["length_m"], r["free_flow_min"], r["capacity_veh_h"],
                        int(float(r["lanes"]))])
    nodes = []
    for line in (src / "raw" / "SiouxFalls_node.tntp").read_text().splitlines()[1:]:
        f = line.replace(";", "").split()
        if len(f) >= 3:
            nodes.append((f[0], f[1], f[2]))
    with (folder / "network" / "nodes.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["id", "lon", "lat"])
        w.writerows(nodes)
    (folder / "demand").mkdir(parents=True, exist_ok=True)
    with (folder / "demand" / "zones.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["zone", "lon", "lat"])
        w.writerows(nodes)
    _copy(src / "demand" / "base" / "od.csv", folder / "demand" / "od.csv")
    _car_classes(folder)
    write_case_json(
        "siouxfalls", case="siouxfalls", title=TITLES["siouxfalls"], kind="table", bundle=VERSION,
        network={"links": "network/links.csv", "nodes": "network/nodes.csv",
                 "read": {"length_unit": "m", "time_unit": "min"}},
        transit=None, parkings=False,
        demand={"od": "demand/od.csv", "zones": "demand/zones.csv", "classes": "demand/classes.csv",
                "from_od": {"start_s": 7 * 3600, "end_s": 8 * 3600, "profile": "uniform", "spread_m": 0.0}},
        # A benchmark of the literature carries no prices (S254): its travellers choose on time, as its
        # equilibria are computed; money weighs by default.
        scenarios={"base": {"multiple": 1.0, "price_options": NO_PRICES, **BENCH_BINS},
                   "congested": {"multiple": 1.5, "price_options": NO_PRICES, **BENCH_BINS}},
        sources=[{"what": "the network (free-flow times, capacities) and the OD matrix",
                  "from": "TNTP, bstabler/TransportationNetworks (SiouxFalls)", "licence": "academic benchmark data"},
                 {"what": "the link lengths", "from": "TNTP's dynamic Sioux Falls (Bar-Gera's TNTP site)",
                  "licence": "academic benchmark data"},
                 {"what": "lanes and the base demand (0.5 x the TNTP matrix over one hour)",
                  "from": "openmobisim, bundle v1", "licence": "CC BY 4.0"}],
    )


def pack_nguyendupuis() -> None:
    """Make the Nguyen-Dupuis case's bundle folder from its processed tables."""
    import openmobisim as ms

    folder = OUT / "nguyendupuis"
    src = DATA / "cases" / "nguyendupuis"
    _copy(src / "processed" / "links.csv", folder / "network" / "links.csv")
    _copy(src / "processed" / "nodes.csv", folder / "network" / "nodes.csv")
    _copy(src / "demand" / "od.csv", folder / "demand" / "od.csv")
    read = {"length_unit": "km", "coordinates": "xy"}
    net = ms.network_read_table(str(folder / "network" / "links.csv"), str(folder / "network" / "nodes.csv"),
                                **read)
    with (folder / "demand" / "zones.csv").open("w", newline="") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(["zone", "lon", "lat"])
        for z in ("1", "2", "3", "4"):
            lon, lat = net.node_lonlat(z)
            w.writerow([z, repr(lon), repr(lat)])
    _car_classes(folder)
    write_case_json(
        "nguyendupuis", case="nguyendupuis", title=TITLES["nguyendupuis"], kind="table", bundle=VERSION,
        network={"links": "network/links.csv", "nodes": "network/nodes.csv", "read": read},
        transit=None, parkings=False,
        demand={"od": "demand/od.csv", "zones": "demand/zones.csv", "classes": "demand/classes.csv",
                "from_od": {"start_s": 7 * 3600, "end_s": 8 * 3600, "profile": "uniform", "spread_m": 0.0}},
        scenarios={"base": {"price_options": NO_PRICES, **BENCH_BINS}},  # a benchmark: no prices (S254)
        sources=[{"what": "the network and the OD pairs",
                  "from": "Nguyen and Dupuis, Transportation Science 18(2), 1984; numbers as tabled in Sensors "
                          "25(7) 2262, 2025, doi:10.3390/s25072262 (Tables 1 and 2)", "licence": "CC BY 4.0"},
                 {"what": "made dynamic: minutes, capacities and demand x 10 per hour, 1 km per minute, one lane, "
                          "a schematic layout", "from": "openmobisim, bundle v1", "licence": "CC BY 4.0"}],
    )


def pack(*cases: str) -> None:
    """Every case's folder, then its archive and the manifest of checksums the package carries."""
    import hashlib

    cases = cases or ("delft", "amsterdam", "paris", "lyon", "siouxfalls", "nguyendupuis")
    for case in cases:
        if case in AREAS:
            pack_city(case)
        elif case == "siouxfalls":
            pack_siouxfalls()
        else:
            pack_nguyendupuis()

    def sha(path: Path) -> str:
        h = hashlib.sha256()
        with path.open("rb") as f:
            for block in iter(lambda: f.read(1 << 20), b""):
                h.update(block)
        return h.hexdigest()

    manifest_path = OUT / "MANIFEST.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {"bundle": VERSION, "cases": {}}
    for case in cases:
        folder = OUT / case
        files = sorted(p for p in folder.rglob("*") if p.is_file() and p.name != "clip.json")
        archive = OUT / f"{case}.zip"
        archive.unlink(missing_ok=True)
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as z:
            for p in files:
                info = zipfile.ZipInfo(p.relative_to(folder).as_posix(), date_time=(2026, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, p.read_bytes())
        manifest["cases"][case] = {
            "archive": {"name": archive.name, "sha256": sha(archive), "bytes": archive.stat().st_size},
            "files": {p.relative_to(folder).as_posix(): sha(p) for p in files},
        }
    manifest["cases"] = dict(sorted(manifest["cases"].items()))
    manifest_path.write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps({c: manifest["cases"][c]["archive"] for c in cases}, indent=1))


# S282: the network as tables in standard formats, for people who do
# not run an OpenStreetMap importer: GMNS (node, link and config tables per layer) and GeoParquet (each layer's links,
# their geometry as WKB in WGS 84, longitude first). A convenience, not the truth: the importer's version is stated, and the clipped
# extract stays the input. A separate archive per city, in the published release only: the package's case download stays as it is
# (the tables add about a quarter to a city's size: Delft 3.9 MB, Paris 24 MB zipped).
TABLES_README = """# {title}: the network as tables

Derived from this case's OpenStreetMap extract (`network.osm.pbf` in the case's archive) by openmobisim {version}
(build {sha}), with its default reading rules: a convenience for tools that do not read OpenStreetMap, not a second
source. The case's own files remain the input; rebuild these from them with `network_write_gmns`.

- `gmns/road`, `gmns/bike`, `gmns/walk`: each layer as a GMNS network (`node.csv`, `link.csv`, `config.csv`: lengths in
  metres, speeds in km/h, coordinates in WGS 84 (EPSG:4326, longitude first), the links' geometry as WKT).
- `road_links.parquet`, `bike_links.parquet`, `walk_links.parquet`: the same links as GeoParquet (version 1.1), their
  geometry as WKB in WGS 84 (OGC:CRS84, longitude first), the other columns as in `link.csv`.

Licence: the Open Database License (ODbL 1.0), as the OpenStreetMap data it is derived from: © OpenStreetMap
contributors, https://www.openstreetmap.org/copyright.
"""


def _wkb_linestring(wkt: str) -> bytes:
    """A ``LINESTRING (x y, x y, ...)`` as little-endian WKB."""
    import struct

    body = wkt[wkt.index("(") + 1 : wkt.rindex(")")]
    points = [tuple(float(v) for v in p.split()) for p in body.split(",")]
    return struct.pack("<BII", 1, 2, len(points)) + b"".join(struct.pack("<dd", x, y) for x, y in points)


def _geoparquet(link_csv: Path, out: Path) -> None:
    import pyarrow as pa
    import pyarrow.csv as pc
    import pyarrow.parquet as pq

    table = pc.read_csv(link_csv, convert_options=pc.ConvertOptions(strings_can_be_null=True))
    wkt = table.column("geometry").to_pylist()
    geometry = pa.array([_wkb_linestring(w) if w else None for w in wkt], type=pa.binary())
    table = table.set_column(table.schema.get_field_index("geometry"), "geometry", geometry)
    geo = {"version": "1.1.0", "primary_column": "geometry",
           # no "crs": GeoParquet's default, OGC:CRS84, is WGS 84 longitude first, as the coordinates are
           "columns": {"geometry": {"encoding": "WKB", "geometry_types": ["LineString"]}}}
    table = table.replace_schema_metadata({**(table.schema.metadata or {}), b"geo": json.dumps(geo).encode()})
    pq.write_table(table, out, compression="zstd")


def tables(case: str) -> None:
    """``<case>-tables.zip`` beside the case's archive: the network as GMNS and GeoParquet."""
    import shutil
    import tempfile

    import openmobisim as ms

    info = json.loads((OUT / case / "case.json").read_text())
    network = ms.examples.case(case, root=OUT).network()
    with tempfile.TemporaryDirectory() as folder:
        root = Path(folder) / f"{case}-tables"
        ms.network_write_gmns(network, str(root / "gmns"))
        for layer in ("road", "bike", "walk"):
            _geoparquet(root / "gmns" / layer / "link.csv", root / f"{layer}_links.parquet")
        build = ms.build_info()
        (root / "README.md").write_text(TABLES_README.format(title=info.get("title", case), version=build["version"],
                                                            sha=build["git_sha"]))
        target = OUT / f"{case}-tables.zip"
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as z:
            for path in sorted(root.rglob("*")):
                if path.is_file():
                    z.write(path, path.relative_to(root.parent))
        shutil.rmtree(root)
    print(f"{case}: {target.name}, {target.stat().st_size / 1e6:.1f} MB")


def main() -> None:
    """Run the command named first on the command line for each case named after it."""
    command, *args = sys.argv[1:]
    if command == "pack":
        pack(*args)
        return
    commands = {"date": date, "clip": clip, "check": check, "tables": tables}
    if command not in commands:
        raise SystemExit(f"unknown command {command!r}; the commands: {sorted(commands)}")
    for case in args:
        commands[command](case)


if __name__ == "__main__":
    main()
