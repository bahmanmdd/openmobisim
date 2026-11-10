"""The five defaults the binding holds (crates/binding/src/lib.rs), each pinned to PARAMETERS.md by a
hand case run with NO keyword anywhere, so the answer follows from the table's default alone. The
loop's defaults are the loop's own (Parameters::defaults) and are not repeated here.

Every trip leaves at 08:00:00 (28 800 s of the clock) in a window to 10:00:00 (default bins of
300 s); trip ends and stops lie exactly on nodes, so every gap (door to node, stop to node) is 0 m.

1. bike_speed 15 km/h = 15 000 / 3 600 m/s: a 1 500 m ride takes 1 500 / (15 000 / 3 600) = 360 s.
2. walk_speed 4.8 km/h = 4 800 / 3 600 m/s: a 1 000 m walk takes 1 000 / (4 800 / 3 600) = 750 s.
3. capacity_per_lane 1 800 veh/h: a car link of 2 lanes with capacity_veh_h None passes
   2 x 1 800 = 3 600 veh/h, one car per second. 1 000 m at 36 km/h is 100 s at free flow. Three
   cars entering together at 0 s leave by the point queue's rule max(t + 100, previous exit + 1)
   at 100, 101 and 102 s (on the clock 28 900, 28 901, 28 902): travel times 100, 101, 102 s,
   the link's summed crossing time 303 s.
4. stop_walk_max 1 500 m: two transit-only trips to the same door; "near" walks 1 499 m to stop
   S1, "far" 1 501 m to S1 and 500 m to stop S3. The bus leaves S1 at 08:30:00 and reaches S2,
   100 m from the door, at 08:40:00; a later bus leaves S3 at 08:50:00 and reaches S2 at
   09:00:00. A stop beyond the limit is offered only to an end with no stop within it, so "far"
   is offered S3 alone, and the later arrival shows that S1 was left out. Margin: 1 m, a part in
   1 500, where the demand crate reads its limit to a part in a billion and the gaps are 0 m
   exactly (a stop and its node, a door and its node, are the same longitude/latitude, projected
   by the same projection), so rounding cannot decide the case; the stated link lengths are the
   walking metres, never a projection. "near": at S1 when its walk of 1 499 / (4 800 / 3 600) =
   1 124.25 s ends, on the whole second 28 800 + 1 125 = 29 925; boards the 08:30:00 bus
   (30 600 s, after 29 925 + 60 s of the default boarding slack), off at 08:40:00 (31 200 s),
   walks 100 m to the door in 75 s: arrives at 31 275 s, a travel time of 2 475 s, mode transit,
   one alternative. "far": at S3 after 500 / (4 800 / 3 600) = 375 s, at 29 175 s; boards the
   08:50:00 bus (31 800 s), off at 09:00:00 (32 400 s), walks 100 m in 75 s: arrives at
   32 475 s, a travel time of 3 675 s. Had S1 been offered, its walk of 1 501 m (1 125.75 s, at
   S1 on the whole second 29 926, before 30 600 s less the slack) would have caught the
   08:30:00 bus and arrived at 31 275 s.
5. transfer_walk_max 400 m: two pairs of lines, each pair joined only by a walk between the first
   line's last stop and the second line's first stop: 399 m for the trip "near", 401 m for "far"
   (the same 1 m margin, the GTFS reader reading its limit to a part in a billion). Each door is
   10 m from its line's stop. "near": at its first stop on the whole second after 7.5 s, 28 808;
   rides 08:10:00 to 08:20:00 (29 400 to 30 000 s); walks 399 m in 299.25 s, at the second stop at
   30 299.25 s, before 08:30:00 less the 60 s slack; rides 08:30:00 to 08:40:00 (30 600 to
   31 200 s); walks 10 m in 7.5 s: arrives at 31 207.5 s, a travel time of 2 407.5 s, two rides.
   "far": no footpath of 401 m exists, the walks are not linked otherwise, so no journey: mode
   null, alternatives 0.

For bikes: case 1 is a bike's ride at the default bike speed; the walks to and from stops (cases
4 and 5) are on foot, never by bike.
"""

import zipfile

import pyarrow.parquet as pq

import openmobisim as oms

START, END = 28_800, 36_000
LINK_COLUMNS = ["link_id", "upstream", "downstream", "modes", "length_m", "lanes",
                "car_speed_km_h", "capacity_veh_h"]
TRIP_COLUMNS = ["trip_id", "origin_x_m", "origin_y_m", "destination_x_m", "destination_y_m",
                "departure_s", "class_id", "modes"]


def table(columns, rows):
    return {c: [r[k] for r in rows] for k, c in enumerate(columns)}


def trips_of(tmp_path, nodes, links, rows):
    """Run the trips `rows` (id, origin x, destination x, modes; y = 0) with no keyword at all and
    return the trips table by id and the links table's rows."""
    network = oms.Network.from_tables(nodes, links)
    trips = table(TRIP_COLUMNS, [(i, o, 0.0, d, 0.0, START, "all", m) for i, o, d, m in rows])
    demand = oms.Demand.from_table(trips, ["all"], network)
    result = oms.run(network, demand, window=(START, END), out=tmp_path)
    by_id = {r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()}
    return by_id, pq.read_table(result.links).to_pylist()


def line(length_m, modes, lanes=None, speed=None):
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, length_m], "y_m": [0.0, 0.0]}
    return nodes, table(LINK_COLUMNS, [("ab", "a", "b", modes, length_m, lanes, speed, None)])


def test_bike_speed_default_rides_1500_m_in_360_s(tmp_path):
    trips, _ = trips_of(tmp_path, *line(1_500.0, "bike;walk"), [("ride", 0.0, 1_500.0, "bike")])
    assert trips["ride"]["mode"] == "bike" and trips["ride"]["travel_time_s"] == 360.0


def test_walk_speed_default_walks_1000_m_in_750_s(tmp_path):
    trips, _ = trips_of(tmp_path, *line(1_000.0, "bike;walk"), [("walk", 0.0, 1_000.0, "walk")])
    assert trips["walk"]["mode"] == "walk" and trips["walk"]["travel_time_s"] == 750.0


def test_capacity_per_lane_default_passes_one_car_per_second_on_two_lanes(tmp_path):
    rows = [(f"car{i}", 0.0, 1_000.0, "car") for i in range(3)]
    trips, links = trips_of(tmp_path, *line(1_000.0, "car", lanes=2, speed=36.0), rows)
    assert sorted(t["travel_time_s"] for t in trips.values()) == [100.0, 101.0, 102.0]
    assert [(r["link_id"], r["cars"], r["car_time_s"], r["first_exit_s"], r["last_exit_s"])
            for r in links] == [("ab", 3, 303.0, 28_900.0, 28_902.0)]


# Cases 4 and 5: a network in longitude/latitude, metres east and north of 4.36 E, 52.0 N at
# about 68.7 km per degree of longitude and 111.2 km per degree of latitude. Every link's length
# is stated, and each is longer than the straight line between its nodes.
def lon_lat(x, y):
    return 4.36 + x / 68_700, 52.0 + y / 111_200


def walk_network(nodes, links):
    """`nodes`: id -> (x, y) metres; `links`: (a, b, length_m), walked both ways."""
    positions = {n: lon_lat(*xy) for n, xy in nodes.items()}
    node_table = {"node_id": list(positions), "lon_deg": [p[0] for p in positions.values()],
                  "lat_deg": [p[1] for p in positions.values()]}
    rows = [(f"{u}-{v}", u, v, "walk", m, None, None, None)
            for a, b, m in links for u, v in ((a, b), (b, a))]
    return oms.Network.from_tables(node_table, table(LINK_COLUMNS, rows)), positions


def feed(path, stops, runs):
    """A GTFS feed (the reference's required files) with `stops` (id -> longitude, latitude) and
    `runs` (trip id -> [(stop, hh:mm:ss)]), each running Monday to Friday of one week."""
    stop_rows = "".join(f"{s},{s},{lat!r},{lon!r}\n" for s, (lon, lat) in stops.items())
    trip_rows = "".join(f"r,weekdays,{t}\n" for t in runs)
    time_rows = "".join(f"{t},{at},{at},{s},{k}\n"
                        for t, calls in runs.items() for k, (s, at) in enumerate(calls))
    files = {
        "agency.txt": "agency_id,agency_name,agency_url,agency_timezone\n"
                      "a,Agency,https://example.org,Europe/Amsterdam\n",
        "routes.txt": "route_id,agency_id,route_short_name,route_type\nr,a,1,3\n",
        "stops.txt": "stop_id,stop_name,stop_lat,stop_lon\n" + stop_rows,
        "trips.txt": "route_id,service_id,trip_id\n" + trip_rows,
        "stop_times.txt": "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n" + time_rows,
        "calendar.txt": "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,"
                        "start_date,end_date\nweekdays,1,1,1,1,1,0,0,20261102,20261106\n",
    }
    with zipfile.ZipFile(path, "w") as z:
        for name, text in files.items():
            z.writestr(name, text)
    return path


def transit_trips(tmp_path, network, positions, stops, runs, rows):
    """Read the feed on `network` and run the transit-only trips `rows` (id, origin node,
    destination node), all with no keyword; the trips table by id."""
    transit = oms.Transit.from_gtfs(feed(tmp_path / "gtfs.zip", stops, runs), network)
    trips = table(["trip_id", "origin_lon_deg", "origin_lat_deg", "destination_lon_deg",
                   "destination_lat_deg", "departure_s", "class_id", "modes"],
                  [(i, *positions[o], *positions[d], START, "all", "transit")
                   for i, o, d in rows])
    demand = oms.Demand.from_table(trips, ["all"], network, transit=transit)
    result = oms.run(network, demand, transit, window=(START, END), out=tmp_path / "run")
    return {r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()}


def test_stop_walk_max_default_reaches_a_stop_at_1499_m_and_not_at_1501_m(tmp_path):
    nodes = {"near": (0.0, 0.0), "s1": (1_000.0, 0.0), "far": (2_000.0, 0.0),
             "s3": (2_000.0, 300.0), "s2": (3_000.0, 0.0), "door": (3_050.0, 0.0)}
    network, positions = walk_network(
        nodes, [("near", "s1", 1_499.0), ("far", "s1", 1_501.0), ("far", "s3", 500.0),
                ("s2", "door", 100.0)])
    stops = {"S1": positions["s1"], "S2": positions["s2"], "S3": positions["s3"]}
    runs = {"bus": [("S1", "08:30:00"), ("S2", "08:40:00")],
            "later": [("S3", "08:50:00"), ("S2", "09:00:00")]}
    trips = transit_trips(tmp_path, network, positions, stops, runs,
                          [("near", "near", "door"), ("far", "far", "door")])
    near, far = trips["near"], trips["far"]
    assert (near["mode"], near["alternatives"], near["finished"]) == ("transit", 1, True)
    assert near["travel_time_s"] == 2_475.0 and near["arrival_s"] == 31_275.0
    assert (far["mode"], far["alternatives"], far["finished"]) == ("transit", 1, True)
    assert far["travel_time_s"] == 3_675.0 and far["arrival_s"] == 32_475.0


def test_transfer_walk_max_default_joins_two_lines_at_399_m_and_not_at_401_m(tmp_path):
    nodes, links, stops, runs = {}, [], {}, {}
    for k, (trip, walk) in enumerate([("near", 399.0), ("far", 401.0)]):
        y = 2_000.0 * k  # the two cases 2 km apart, linked by nothing
        xs = {"o": 0.0, "s1": 5.0, "s2": 1_000.0, "s3": 1_300.0, "s4": 2_300.0, "d": 2_305.0}
        for name, x in xs.items():
            nodes[f"{trip}-{name}"] = (x, y)
        links += [(f"{trip}-o", f"{trip}-s1", 10.0), (f"{trip}-s2", f"{trip}-s3", walk),
                  (f"{trip}-s4", f"{trip}-d", 10.0)]
    network, positions = walk_network(nodes, links)
    for trip in ("near", "far"):
        for s in ("s1", "s2", "s3", "s4"):
            stops[f"{trip}-{s}"] = positions[f"{trip}-{s}"]
        runs[f"{trip}-first"] = [(f"{trip}-s1", "08:10:00"), (f"{trip}-s2", "08:20:00")]
        runs[f"{trip}-second"] = [(f"{trip}-s3", "08:30:00"), (f"{trip}-s4", "08:40:00")]
    trips = transit_trips(tmp_path, network, positions, stops, runs,
                          [("near", "near-o", "near-d"), ("far", "far-o", "far-d")])
    near, far = trips["near"], trips["far"]
    assert (near["mode"], near["alternatives"], near["finished"]) == ("transit", 1, True)
    assert near["travel_time_s"] == 2_407.5 and near["arrival_s"] == 31_207.5
    assert (far["mode"], far["alternatives"], far["finished"]) == (None, 0, False)
