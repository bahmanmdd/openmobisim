"""The expected attributes of 20 Delft ways, from libosmium (pyosmium) and pyproj.

Kept for reproducibility; it does not run in CI. pyosmium 4.3.2 and pyproj 3.8.0:

    python delft_ways.py fixtures/delft/network.osm.pbf > fixtures/delft/ways.csv

What comes from outside the reader: each way's tags and node positions, decoded by libosmium
(not the `osmpbf` crate the reader uses), and its length, the sum of the WGS84 geodesics between
its nodes by pyproj's `Geod.line_length` (Karney's algorithm, not the reader's Vincenty). What is
written here, from the OpenStreetMap wiki and not from the reader's code: the modes, lanes and
free-flow speed each direction carries, by the rules the crate's README states (version 1's car,
bike and walk rules; `oneway`, `oneway:bicycle`; `lanes`, `lanes:forward`, `lanes:backward`;
`maxspeed:forward`, `:backward`, then `maxspeed`). The raw tags are in the last column so that
each row can be checked by hand. When the reader took version 1's rules this script's rules were
rewritten to them and `ways.csv`'s one changed row (the path: walkers alone, a bike needs a
`bicycle` tag) edited by hand; the script was not run again (pyosmium was not installed).

Each way is the one with the lowest id that meets its case, among the ways with every node in
the extract and at least 2 km inside its bounding box (the extract's edge cuts one-way
carriageways, whose stubs the car network cannot leave or reach), not closed, and of at least
two nodes. Output: one CSV row per way.
"""

import sys

import osmium
from pyproj import Geod

# The 20 cases: a name and a test on the tags.
CASES = [
    ("motorway", lambda t: t.get("highway") == "motorway"),
    ("motorway_link", lambda t: t.get("highway") == "motorway_link"),
    ("trunk", lambda t: t.get("highway") == "trunk"),
    ("both lane sides", lambda t: "lanes:forward" in t and "lanes:backward" in t),
    ("total and forward only", lambda t: two_way(t) and "lanes" in t and "lanes:forward" in t
     and "lanes:backward" not in t),
    ("total and backward only", lambda t: two_way(t) and "lanes" in t and "lanes:backward" in t
     and "lanes:forward" not in t),
    ("maxspeed by direction", lambda t: "maxspeed:forward" in t and "maxspeed:backward" in t),
    ("secondary two-way lanes 2", lambda t: t.get("highway") == "secondary" and two_way(t)
     and t.get("lanes") == "2" and "lanes:forward" not in t),
    ("one-way with bikes both ways", lambda t: t.get("oneway") == "yes"
     and t.get("oneway:bicycle") == "no" and t.get("highway") == "tertiary"),
    ("oneway=alternating", lambda t: t.get("oneway") == "alternating"),
    ("residential", lambda t: t.get("highway") == "residential" and len(t) <= 3),
    ("living_street", lambda t: t.get("highway") == "living_street"),
    ("bicycle=no on a car street", lambda t: t.get("highway") in CAR and t.get("bicycle") == "no"
     and "motorroad" not in t),
    ("parking aisle", lambda t: t.get("highway") == "service" and t.get("service") == "parking_aisle"
     and "access" not in t),
    ("cycleway", lambda t: t.get("highway") == "cycleway" and "oneway" not in t),
    ("one-way cycleway", lambda t: t.get("highway") == "cycleway" and t.get("oneway") == "yes"),
    ("footway", lambda t: t.get("highway") == "footway" and "bicycle" not in t),
    ("footway with bicycle=yes", lambda t: t.get("highway") == "footway" and t.get("bicycle") == "yes"),
    ("path", lambda t: t.get("highway") == "path" and len(t) <= 2),
    ("roundabout", lambda t: t.get("junction") == "roundabout" and t.get("highway") in CAR),
]

# The class table of the crate's README: car (speed km/h, lanes per direction) or None, bike, walk.
CAR = {
    "motorway": (110, 2), "motorway_link": (60, 1), "trunk": (90, 2), "trunk_link": (50, 1),
    "primary": (60, 2), "primary_link": (40, 1), "secondary": (50, 1), "secondary_link": (40, 1),
    "tertiary": (50, 1), "tertiary_link": (30, 1), "unclassified": (40, 1), "road": (40, 1),
    "residential": (30, 1), "living_street": (20, 1), "service": (20, 1),
}
# The classes cars do not use: a footway's (paths, tracks, steps, corridors and bridleways among
# them) and a pedestrian street's bike needs a `bicycle` tag; a busway's bike and walker need one.
FOOTWAY = {"pedestrian", "footway", "path", "track", "steps", "corridor", "bridleway"}
NO_CAR = FOOTWAY | {"cycleway", "busway"}
PERMITS = {"yes", "designated", "permissive"}
CLOSES = {"no", "private"}
READ = ["highway", "oneway", "oneway:bicycle", "junction", "access", "vehicle", "motor_vehicle",
        "motorcar", "bicycle", "foot", "motorroad", "service", "lanes", "lanes:forward",
        "lanes:backward", "lanes:both_ways", "maxspeed", "maxspeed:forward", "maxspeed:backward"]


def two_way(t):
    return t.get("oneway") in (None, "no") and t.get("highway") not in ("motorway", "motorway_link") \
        and t.get("junction") not in ("roundabout", "circular")


def expected(t):
    h = t["highway"]
    closed = t.get("access") in CLOSES
    car_key = next((t[k] for k in ("motorcar", "motor_vehicle", "vehicle") if k in t), None)
    service_only = h == "service" and t.get("service") in (
        "parking_aisle", "driveway", "drive-through", "emergency_access")
    car = h in CAR and not closed and car_key not in CLOSES and not service_only
    bicycle, foot = t.get("bicycle"), t.get("foot")
    motorroad = t.get("motorroad") == "yes"
    bike = not (bicycle in ("no", "use_sidepath") or motorroad or (closed and bicycle not in PERMITS))
    if h in ("motorway", "motorway_link"):
        bike = False
    elif h in ("trunk", "trunk_link", "busway"):
        bike = bike and bicycle in PERMITS
    elif h in FOOTWAY:
        bike = bike and bicycle in PERMITS | {"dismount"}
    walk = not (foot in ("no", "use_sidepath") or motorroad or (closed and foot not in PERMITS))
    if h in ("motorway", "motorway_link") or (h == "busway" and foot not in PERMITS):
        walk = False
    oneway = t.get("oneway")
    if oneway in ("yes", "true", "1"):
        vehicles = (1, 0)
    elif oneway in ("-1", "reverse"):
        vehicles = (0, 1)
    elif oneway in ("no", "false", "0"):
        vehicles = (1, 1)
    elif h in ("motorway", "motorway_link") or t.get("junction") in ("roundabout", "circular"):
        vehicles = (1, 0)
    else:
        vehicles = (1, 1)
    bikes = (1, 1) if t.get("oneway:bicycle") in ("no", "false", "0") else vehicles
    sides = []
    for k, side in enumerate(("forward", "backward")):
        modes = ("c" if car and vehicles[k] else "") + ("b" if bike and bikes[k] else "") \
            + ("w" if walk else "")
        lanes = speed = ""
        if "c" in modes:
            lanes = lanes_of(t, side, vehicles, CAR[h][1])
            speed = float(t.get("maxspeed:" + side, t.get("maxspeed", CAR[h][0])))
        sides.append((modes or "-", lanes, speed))
    return sides


def lanes_of(t, side, vehicles, default):
    other = "backward" if side == "forward" else "forward"
    n = lambda k: int(t[k]) if k in t else None
    total, mine, theirs = n("lanes"), n("lanes:" + side), n("lanes:" + other)
    if vehicles != (1, 1):
        return mine if mine is not None else (total if total is not None else default)
    if mine is not None:
        return mine
    middle = n("lanes:both_ways") or 0
    if total is not None and theirs is not None:
        return total - theirs - middle
    if total is not None:
        return max(1, (total - middle) // 2)
    return default


def main(path):
    ways, wanted = {}, set()
    for w in osmium.FileProcessor(path, osmium.osm.WAY):
        t = dict(w.tags)
        refs = [n.ref for n in w.nodes]
        if "highway" in t and len(refs) >= 2 and refs[0] != refs[-1]:
            ways[w.id] = (t, refs)
            wanted.update(refs)
    pos, box = {}, [180.0, -180.0, 90.0, -90.0]
    for n in osmium.FileProcessor(path, osmium.osm.NODE):
        lon, lat = n.location.lon, n.location.lat
        box = [min(box[0], lon), max(box[1], lon), min(box[2], lat), max(box[3], lat)]
        if n.id in wanted:
            pos[n.id] = (lon, lat)
    # 2 km inside the box: in degrees of latitude (110.6 km) and of longitude at 52°N (68.5 km).
    inside = lambda p: box[0] + 2 / 68.5 < p[0] < box[1] - 2 / 68.5 \
        and box[2] + 2 / 110.6 < p[1] < box[3] - 2 / 110.6
    geod = Geod(ellps="WGS84")
    print("way,case,length_m,forward_modes,forward_lanes,forward_speed_km_h,"
          "backward_modes,backward_lanes,backward_speed_km_h,tags")
    for name, test in CASES:
        for wid in sorted(ways):
            t, refs = ways[wid]
            if all(r in pos and inside(pos[r]) for r in refs) and (t["highway"] in CAR or t["highway"] in NO_CAR) \
                    and t.get("area") != "yes" and test(t):
                break
        else:
            raise SystemExit(f"no way for case {name}")
        length = geod.line_length([pos[r][0] for r in refs], [pos[r][1] for r in refs])
        (fm, fl, fs), (bm, bl, bs) = expected(t)
        tags = "|".join(f"{k}={t[k]}" for k in READ if k in t)
        print(f"{wid},{name},{length!r},{fm},{fl},{fs},{bm},{bl},{bs},{tags}")


if __name__ == "__main__":
    main(sys.argv[1])
