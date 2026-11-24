"""The values a run records are the values it used: for each keyword, a case built so that the
keyword binds, run at two values, with the answer from outside the simulator (a hand number,
derived in each test) or, where a hand number is long to derive, the property that the tables
differ from the run at the default (the test says why they must). Each test also reads the
value back from the record, run_record.toml, so a value recorded but not used, or used but not recorded, fails.

The seed's range: a seed above 2^63 - 1, the largest integer of TOML, is refused where the
keyword is read, before the run; 2^63 - 1 runs and reads back.

Units: a walker at 4.8 km/h covers a metre in 0.75 s; at 5 km/h in 0.72 s. Every trip end and
stop lies on a node (gaps 0 m), and every link's length is stated. Transit cases are in
longitude/latitude (the GTFS reader's requirement), metres placed east of 4.36 E, 52.0 N.
For bikes: the iteration floor and the cap are judged on a bike trip at 15 km/h, whose 1 500 m
take 360 s in every iteration.
"""

import math
import time
import tomllib

import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from grid import logit_run
from test_defaults import LINK_COLUMNS, TRIP_COLUMNS, feed, lon_lat, table

START = 28_800
# Time alone: 1 per minute on the nine time columns, 0 on the rest; with the deterministic
# choice the trip takes its earliest alternative.
TIME_ONLY = (1.0,) * 9 + (0.0,) * 7


def read(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def by_id(result):
    return {r["trip_id"]: r for r in pq.read_table(result.trips).to_pylist()}


def metres_line(length_m, modes, lanes=None, speed=None, **network_kw):
    nodes = {"node_id": ["a", "b"], "x_m": [0.0, length_m], "y_m": [0.0, 0.0]}
    links = table(LINK_COLUMNS, [("ab", "a", "b", modes, length_m, lanes, speed, None)])
    return oms.Network.from_tables(nodes, links, **network_kw)


def metres_trips(rows, classes=("all",)):
    """`rows`: (id, destination x, departure, modes), from (0, 0) to (x, 0)."""
    return table(TRIP_COLUMNS, [(i, 0.0, 0.0, x, 0.0, d, "all", m) for i, x, d, m in rows])


def transit_case(tmp_path, nodes, links, stops, runs, trip, network_kw=None, transit_kw=None,
                 demand_kw=None, run_kw=None, travellers=1):
    """A walk network in longitude/latitude (`nodes`: id -> x metres on one line; `links`:
    (a, b, length_m), each way), a feed with `stops` (id -> node) and `runs` (id -> [(stop,
    second of the day)]), `travellers` identical transit-only trips `t0`, `t1`, ... (`trip`:
    origin node, destination node, their departure), run from START for an hour; the Run."""
    tmp_path.mkdir(parents=True, exist_ok=True)
    positions = {n: lon_lat(x, 0.0) for n, x in nodes.items()}
    node_table = {"node_id": list(positions), "lon_deg": [p[0] for p in positions.values()],
                  "lat_deg": [p[1] for p in positions.values()]}
    rows = [(f"{u}-{v}", u, v, "walk", m, None, None, None)
            for a, b, m in links for u, v in ((a, b), (b, a))]
    network = oms.Network.from_tables(node_table, table(LINK_COLUMNS, rows), **(network_kw or {}))
    clock = {r: [(s, f"{t // 3600:02d}:{t % 3600 // 60:02d}:{t % 60:02d}") for s, t in calls]
             for r, calls in runs.items()}
    path = feed(tmp_path / "gtfs.zip", {s: positions[n] for s, n in stops.items()}, clock)
    transit = oms.Transit.from_gtfs(path, network, **(transit_kw or {}))
    o, d, departure = trip
    trips = table(["trip_id", "origin_lon_deg", "origin_lat_deg", "destination_lon_deg",
                   "destination_lat_deg", "departure_s", "class_id", "modes"],
                  [(f"t{i}", *positions[o], *positions[d], departure, "all", "transit")
                   for i in range(travellers)])
    demand = oms.Demand.from_table(trips, ["all"], network, transit, **(demand_kw or {}))
    keywords = {"choice": "deterministic", "coefficients": {"all": TIME_ONLY}, "link_bin": 60,
                "iteration_cap": 1, **(run_kw or {})}
    return oms.run(network, demand, transit, window=(START, START + 3_600),
                   out=tmp_path / "run", **keywords)


def test_walk_speed_5_km_h_walks_1000_m_in_720_s(tmp_path):
    # 1 000 m / (5 000 / 3 600 m/s) = 720 s (750 s at the default 4.8 km/h).
    network = metres_line(1_000.0, "walk", walk_speed=5.0)
    demand = oms.Demand.from_table(metres_trips([("w", 1_000.0, START, "walk")]), ["all"],
                                   network)
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path)
    assert by_id(result)["w"]["travel_time_s"] == 720.0
    assert read(result.record)["parameters"]["walk_speed"] == 5.0


def test_capacity_per_lane_900_on_two_lanes_lets_a_car_out_every_2_s(tmp_path):
    # 2 lanes x 900 veh/h = 1 800 veh/h, one car every 2 s (one a second at the default 1 800).
    # 1 000 m at 36 km/h is 100 s; three cars entering at 0 s leave at max(t + 100, previous
    # exit + 2) = 100, 102, 104 s: travel times 100, 102, 104 s.
    network = metres_line(1_000.0, "car", lanes=2, speed=36.0, capacity_per_lane=900.0)
    trips = metres_trips([(f"car{i}", 1_000.0, START, "car") for i in range(3)])
    demand = oms.Demand.from_table(trips, ["all"], network)
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path)
    assert sorted(t["travel_time_s"] for t in by_id(result).values()) == [100.0, 102.0, 104.0]
    assert read(result.record)["parameters"]["capacity_per_lane"] == 900.0


def test_a_walk_of_4500_s_is_unfinished_in_a_one_hour_window_and_finished_in_two(tmp_path):
    # 6 000 m at 4.8 km/h: 6 000 x 0.75 = 4 500 s, from 07:00:00 (25 200 s): at the door at
    # 29 700 s, after a window ending at 28 800 s and before one ending at 32 400 s.
    network = metres_line(6_000.0, "walk")
    demand = oms.Demand.from_table(metres_trips([("w", 6_000.0, 25_200, "walk")]), ["all"],
                                   network)
    short = oms.run(network, demand, window=(25_200, 28_800), out=tmp_path / "one")
    long = oms.run(network, demand, window=(25_200, 32_400), out=tmp_path / "two")
    assert (by_id(short)["w"]["finished"], by_id(short)["w"]["arrival_s"]) == (False, None)
    assert (by_id(long)["w"]["finished"], by_id(long)["w"]["arrival_s"]) == (True, 29_700.0)
    record = read(long.record)
    assert (record["window_start_s"], record["window_end_s"]) == (25_200, 32_400)


def bike_runs(tmp_path, name, **run_kw):
    network = metres_line(1_500.0, "bike")
    demand = oms.Demand.from_table(metres_trips([("b", 1_500.0, START, "bike")]), ["all"],
                                   network)
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path / name,
                     **run_kw)
    rows = pq.read_table(result.iterations).num_rows
    return result, rows, read(result.record)


def test_the_iteration_floor_and_the_cap_set_the_rows_and_the_record_says_which_stopped(tmp_path):
    # The rule (crates/loop/README.md): stop after an iteration when its found share is at most
    # 0.5 % and the last five mean trip times lie within 0.3 % of the latest; not before
    # iteration_floor iterations; always at the cap. One bike trip has one route at 360 s in
    # every iteration, found at iteration 0 and never a cheaper one after: the rule holds first
    # after the fifth iteration. So: floor 0, 5 rows, by the rule; floor 8, 8 rows, by the rule;
    # cap 3, 3 rows, by the cap.
    for name, run_kw, rows_want, by_rule in [("rule", {}, 5, True),
                                             ("floor", {"iteration_floor": 8}, 8, True),
                                             ("cap", {"iteration_cap": 3}, 3, False)]:
        result, rows, record = bike_runs(tmp_path, name, **run_kw)
        assert (rows, result.stopped_by_rule) == (rows_want, by_rule), name
        ended = {k: record["result"][k] for k in ("iterations", "stopped_by_rule")}
        assert ended == {"iterations": rows, "stopped_by_rule": by_rule}, name
        assert record["parameters"]["iteration_floor"] == run_kw.get("iteration_floor", 0)


def test_transfer_walk_max_300_against_400_drops_the_two_footpaths_of_375_m(tmp_path):
    # Two stops on nodes 375 m apart by the only walk link: a footpath each way at 400 m
    # (375 <= 400), none at 300 m (375 > 300). The GTFS reader's footpath count says it.
    nodes, links = {"s1": 0.0, "s2": 375.0}, [("s1", "s2", 375.0)]
    stops, runs = {"S1": "s1", "S2": "s2"}, {"r": [("S1", 29_400), ("S2", 29_700)]}
    for limit, footpaths in ((300.0, 0), (400.0, 2)):
        result = transit_case(tmp_path / f"{limit:.0f}", nodes, links, stops, runs,
                              ("s1", "s2", START), transit_kw={"transfer_walk_max": limit})
        record = read(result.record)
        assert result.gtfs_report["footpaths"] == footpaths, limit
        assert record["result"]["transit"]["gtfs_report"]["footpaths"] == footpaths, limit
        assert record["parameters"]["transfer_walk_max"] == limit


def test_stop_walk_max_1000_against_1200_leaves_a_stop_1100_m_away_out_of_reach(tmp_path):
    # The only stop near the origin is 1 100 m away on foot: out of reach at 1 000 m, so the
    # trip is counted without_stop (1) and has no alternative; in reach at 1 200 m (0).
    nodes = {"o": 0.0, "s": 1_100.0, "d": 3_000.0}
    links = [("o", "s", 1_100.0), ("s", "d", 1_900.0)]
    stops, runs = {"S": "s", "D": "d"}, {"r": [("S", 30_000), ("D", 30_300)]}
    for limit, without, alternatives in ((1_000.0, 1, 0), (1_200.0, 0, 1)):
        result = transit_case(tmp_path / f"{limit:.0f}", nodes, links, stops, runs,
                              ("o", "d", START), demand_kw={"stop_walk_max": limit})
        last = pq.read_table(result.iterations).to_pylist()[-1]
        assert last["without_stop"] == without, limit
        assert by_id(result)["t0"]["alternatives"] == alternatives, limit
        assert read(result.record)["parameters"]["stop_walk_max"] == limit


def test_a_boarding_slack_of_120_s_misses_a_run_leaving_100_s_after_the_traveller_arrives(tmp_path):
    # At stop A (the origin, 0 m) at 28 800 s. Run r0 leaves A at 28 900 s, reaches C at
    # 29 200 s; r1 leaves at 29 800 s, reaches C at 30 100 s; the door is at C. Slack 60 s:
    # 28 800 + 60 <= 28 900, r0, 400 s door to door. Slack 120 s: 28 920 > 28 900, r1, 1 300 s.
    nodes, links = {"a": 0.0, "c": 5_000.0}, [("a", "c", 5_000.0)]
    stops = {"A": "a", "C": "c"}
    runs = {"r0": [("A", 28_900), ("C", 29_200)], "r1": [("A", 29_800), ("C", 30_100)]}
    for slack, seconds in ((60, 400.0), (120, 1_300.0)):
        result = transit_case(tmp_path / str(slack), nodes, links, stops, runs,
                              ("a", "c", START), run_kw={"boarding_slack": slack})
        assert by_id(result)["t0"]["travel_time_s"] == seconds, slack
        assert read(result.record)["parameters"]["boarding_slack"] == slack


def test_max_rides_1_refuses_the_two_ride_journey_that_max_rides_2_takes(tmp_path):
    # A to B on r0 (28 900 to 29 200 s), B to C on r1 (29 300 to 29 600 s): two rides, at the
    # door at C at 29 600 s, 800 s; no run goes from A to C, so one ride is no journey.
    nodes = {"a": 0.0, "b": 2_000.0, "c": 4_000.0}
    links = [("a", "b", 2_000.0), ("b", "c", 2_000.0)]
    stops = {"A": "a", "B": "b", "C": "c"}
    runs = {"r0": [("A", 28_900), ("B", 29_200)], "r1": [("B", 29_300), ("C", 29_600)]}
    for rides, mode, seconds in ((1, None, None), (2, "transit", 800.0)):
        result = transit_case(tmp_path / str(rides), nodes, links, stops, runs,
                              ("a", "c", START), run_kw={"max_rides": rides})
        trip = by_id(result)["t0"]
        assert (trip["mode"], trip["travel_time_s"]) == (mode, seconds), rides
        assert read(result.record)["parameters"]["max_rides"] == rides


def test_nearby_stops_2_offers_the_later_journey_from_the_nearest_stop_that_1_does_not(tmp_path):
    # crates/loop/tests/nearby_stops.rs's hand case, START later. The door at 0 m, stop A at
    # 100 m (75 s), B at 1 000 m (750 s), C at the destination, 5 000 m. r0: B 29 800 s -> C
    # 30 100 s; r1: A 29 600 s -> C 30 200 s. At 1 the one search between any stops finds the
    # earliest, r0 from B, and with time alone the cheapest is the same: one alternative. At 2
    # the nearest stop A tried alone adds r1: two alternatives, the earlier (1 300 s) taken.
    nodes = {"o": 0.0, "a": 100.0, "b": 1_000.0, "c": 5_000.0}
    links = [("o", "a", 100.0), ("a", "b", 900.0), ("b", "c", 4_000.0)]
    stops = {"A": "a", "B": "b", "C": "c"}
    runs = {"r0": [("B", 29_800), ("C", 30_100)], "r1": [("A", 29_600), ("C", 30_200)]}
    for nearby, alternatives in ((1, 1), (2, 2)):
        result = transit_case(tmp_path / str(nearby), nodes, links, stops, runs,
                              ("o", "c", START), run_kw={"nearby_stops": nearby})
        trip = by_id(result)["t0"]
        assert (trip["alternatives"], trip["travel_time_s"]) == (alternatives, 1_300.0), nearby
        assert read(result.record)["parameters"]["nearby_stops"] == nearby


def test_max_transit_journeys_1_keeps_one_of_the_two_journeys_that_2_keeps(tmp_path):
    # The case above at nearby_stops 2: two journeys, r0 from B (1 300 s) and r1 from A
    # (1 400 s). With time alone the cheaper is r0, the earlier: a limit of 1 keeps it alone, a
    # limit of 2 keeps both; r0 is taken in both.
    nodes = {"o": 0.0, "a": 100.0, "b": 1_000.0, "c": 5_000.0}
    links = [("o", "a", 100.0), ("a", "b", 900.0), ("b", "c", 4_000.0)]
    stops = {"A": "a", "B": "b", "C": "c"}
    runs = {"r0": [("B", 29_800), ("C", 30_100)], "r1": [("A", 29_600), ("C", 30_200)]}
    for limit, alternatives in ((1, 1), (2, 2)):
        result = transit_case(tmp_path / str(limit), nodes, links, stops, runs, ("o", "c", START),
                              run_kw={"nearby_stops": 2, "max_transit_journeys": limit})
        trip = by_id(result)["t0"]
        assert (trip["alternatives"], trip["travel_time_s"]) == (alternatives, 1_300.0), limit
        assert read(result.record)["parameters"]["max_transit_journeys"] == limit

def logit_tables(folder, **run_kw):
    result = logit_run(folder, **run_kw)
    return result, (folder / "trips.parquet").read_bytes() + (folder / "links.parquet").read_bytes()


def test_another_seed_changes_the_logit_choices(tmp_path):
    # Why it must: every draw of the logit is a function of the seed, and 400 travellers choose
    # among car, bike and walk with probabilities strictly between 0 and 1 (several modes are
    # taken, checked): the chance that all 400 choices agree under two seeds is negligible.
    default, a = logit_tables(tmp_path / "seed0")
    other, b = logit_tables(tmp_path / "seed1", seed=1)
    modes = {m for m in pq.read_table(default.trips).column("mode").to_pylist() if m}
    assert len(modes) >= 2, modes
    assert a != b
    assert (read(default.record)["parameters"]["seed"], read(other.record)["parameters"]["seed"]) \
        == (0, 1)


def test_the_nest_scale_sets_the_share_of_the_cheaper_of_two_journeys_in_one_nest(tmp_path):
    # The nested logit of crates/choice/README.md: v_j = -cost_j / mu, and in a nest
    # P(j | n) = exp(v_j) / sum_k exp(v_k). Transit only, so the transit nest is chosen with
    # probability 1, and in it two journeys (the nearby_stops case above, at nearby_stops 2):
    # r0 from B, 1 300 s door to door, and r1 from A, 1 400 s. At 0.3 per minute on every time
    # column and 0 elsewhere their costs are 0.3 x 1 300 / 60 = 6.5 and 0.3 x 1 400 / 60 = 7.0,
    # so P(r0) = 1 / (1 + exp(-(7.0 - 6.5) / mu)): 1 / (1 + e^-1) = 0.7311 at mu 0.5 and
    # 1 / (1 + e^-0.5) = 0.6225 at mu 1. 2 000 identical travellers, one iteration (everyone
    # chooses at iteration 0), each with its own draws; nothing is loaded on a link, so nothing
    # changes the costs. The journey taken is read from travel_time_s (1 300 or 1 400 s). Judged
    # within 3 binomial standard deviations, sqrt(p (1 - p) / 2 000): 0.0099 and 0.0108; the
    # two shares are 0.109, about 10 standard deviations, apart.
    nodes = {"o": 0.0, "a": 100.0, "b": 1_000.0, "c": 5_000.0}
    links = [("o", "a", 100.0), ("a", "b", 900.0), ("b", "c", 4_000.0)]
    stops = {"A": "a", "B": "b", "C": "c"}
    runs = {"r0": [("B", 29_800), ("C", 30_100)], "r1": [("A", 29_600), ("C", 30_200)]}
    n, weights = 2_000, (0.3,) * 9 + (0.0,) * 7
    for mu in (0.5, 1.0):
        started = time.perf_counter()
        result = transit_case(tmp_path / str(mu), nodes, links, stops, runs, ("o", "c", START),
                              run_kw={"choice": "logit", "nest_scale": mu, "nearby_stops": 2,
                                      "coefficients": {"all": weights}},
                              travellers=n)
        seconds = time.perf_counter() - started
        trips = list(by_id(result).values())
        assert all(t["alternatives"] == 2 for t in trips)
        times = [t["travel_time_s"] for t in trips]
        assert set(times) <= {1_300.0, 1_400.0}, set(times)
        share = times.count(1_300.0) / n
        p = 1 / (1 + math.exp(-0.5 / mu))
        sd = math.sqrt(p * (1 - p) / n)
        print(f"nest scale {mu}: share of the 1 300 s journey {share:.4f} against {p:.4f} "
              f"(sd {sd:.4f}), {seconds:.2f} s")
        assert abs(share - p) <= 3 * sd, (mu, share, p, sd)
        assert read(result.record)["parameters"]["nest_scale"] == mu


def test_the_deterministic_choice_sends_every_traveller_by_the_cheaper_bike(tmp_path):
    # 1 000 travellers, 1 500 m open to bike and walk, the default coefficients: the bike rides
    # 360 s, 6 min x 0.1 = 0.6; the walk takes 1 125 s, 18.75 min x 0.165 + 0.5 = 3.594. The
    # deterministic choice takes the bike for every one. The logit (one row per nest: the plain
    # logit) would send each by walk with probability 1 / (1 + exp(2.994)) = 0.048, so all
    # 1 000 by bike with probability 0.952^1 000, about 1e-21: the record says which model ran.
    n = 1_000
    network = metres_line(1_500.0, "bike;walk")
    trips = metres_trips([(f"t{i}", 1_500.0, START, "bike;walk") for i in range(n)])
    demand = oms.Demand.from_table(trips, ["all"], network)
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path,
                     choice="deterministic", iteration_cap=1)
    modes = [t["mode"] for t in by_id(result).values()]
    assert modes.count("bike") == n, modes.count("walk")
    assert read(result.record)["parameters"]["choice"] == "deterministic"


def test_a_seed_above_2_63_minus_1_is_refused_before_the_run_and_2_63_minus_1_runs(tmp_path):
    network = metres_line(1_500.0, "bike")
    demand = oms.Demand.from_table(metres_trips([("b", 1_500.0, START, "bike")]), ["all"],
                                   network)
    out = tmp_path / "refused"
    with pytest.raises(ValueError, match=r"seed: 9223372036854775808 is outside 0 to "
                                         r"9223372036854775807"):
        oms.run(network, demand, window=(START, START + 3_600), out=out, seed=2**63)
    assert not out.exists()
    result = oms.run(network, demand, window=(START, START + 3_600), out=tmp_path / "largest",
                     seed=2**63 - 1)
    assert read(result.record)["parameters"]["seed"] == 2**63 - 1
