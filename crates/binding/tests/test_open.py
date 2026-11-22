"""Run.open: a finished run opened from its folder, by its record, run_record.toml, without running
it, and its fourth table.

1. A property: the hand run (test_hand_run.py) opened from its record gives a map and a summary
   equal, byte for byte, to those of the fresh run, from files with nothing passed (each input
   rebuilt from the file the record names) and from tables in memory passed; the same with a
   timetable (a tram, test_names.py, its feed rebuilt from its file). Nothing is run: the four
   tables are unchanged, byte for byte, and `stopped_by_rule` is the record's.
2. Warned, the opening going on: a passed network whose fingerprint is not the record's
   (another `bike_speed`, 16 km/h; both fingerprints named); a transit passed to a run that had
   none (set aside); a record without the network's fingerprint line (the pages then the fresh
   run's). Refused by name: an input given in memory when the run was made and not passed; a
   table file removed (`OSError`); a table's column renamed (`ValueError`); a `stopped_by_rule`
   that is not a boolean. A key a record never holds (under [result.network]): ignored, one
   warning naming it, the pages the fresh run's. Opened, the pages saying so: a record without `stopped_by_rule`
   (`Run.stopped_by_rule` is None; the summary says the record does not say what ended the run,
   the map's provenance line "its end not recorded"). An `out` holding an input named
   `routes.parquet` is refused for `run`, the input unchanged.
3. `routes.parquet` of the hand run rebuilt by `run_file` from the record's part one is the same
   bytes.

For bikes: the hand run's bike journey is one of those drawn on both pages compared.
"""

import re
import warnings
from pathlib import Path

import pyarrow.parquet as pq
import pytest

import openmobisim as oms
from test_hand_run import hand_run, hand_tables
from test_files import write
from test_defaults import END, LINK_COLUMNS, START, lon_lat, table
from test_names import tram_run
from test_drawing import decoded
from test_run_file import settings

TABLES = ("trips.parquet", "links.parquet", "iterations.parquet", "routes.parquet")


def pages(result, folder):
    """The map and the summary of `result`, written into `folder` under fixed names, as bytes."""
    folder.mkdir(parents=True, exist_ok=True)
    return (Path(result.map(folder / "map.html")).read_bytes(),
            Path(result.summary(folder / "summary.html")).read_bytes())


def hand_files(tmp_path):
    """The hand run from three files; the run."""
    nodes, links, trips = hand_tables()
    files = [write(tmp_path / "in", f"{n}.parquet", t)
             for n, t in (("nodes", nodes), ("links", links), ("trips", trips))]
    return hand_run(*files, tmp_path / "run")


def test_an_opened_run_gives_the_fresh_runs_pages_from_files_with_nothing_passed(tmp_path):
    fresh = hand_files(tmp_path)
    before = [(tmp_path / "run" / t).read_bytes() for t in TABLES]
    opened = oms.Run.open(fresh.record)
    assert pages(opened, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")
    assert [(tmp_path / "run" / t).read_bytes() for t in TABLES] == before
    assert (opened.stopped_by_rule, opened.routes, opened.osm_report) == (fresh.stopped_by_rule, fresh.routes, None)


def test_an_opened_run_from_tables_in_memory_takes_them_passed(tmp_path):
    nodes, links, trips = hand_tables()
    fresh = hand_run(nodes, links, trips, tmp_path / "run")
    network = oms.Network.from_tables(nodes, links, capacity_per_lane=1_800.0, bike_speed=15.0, walk_speed=4.8)
    demand = oms.Demand.from_table(trips, ["commute", "leisure"], network)
    with pytest.raises(ValueError, match="the network was given in memory .*pass it: network="):
        oms.Run.open(fresh.record)
    with pytest.raises(ValueError, match="the demand was given in memory .*pass it: demand="):
        oms.Run.open(fresh.record, network=network)
    opened = oms.Run.open(fresh.record, network=network, demand=demand)
    assert pages(opened, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")


def tram_files(tmp_path):
    """The tram run of test_names.py from files: its feed (written by `tram_run`), and its nodes,
    links and trips tables as Parquet files; the run."""
    (tmp_path / "feed").mkdir(parents=True)
    tram_run(tmp_path / "feed")
    positions = {n: lon_lat(x, 0.0) for n, x in (("a", 0.0), ("b", 1_000.0))}
    nodes = {"node_id": list(positions), "lon_deg": [p[0] for p in positions.values()],
             "lat_deg": [p[1] for p in positions.values()]}
    links = table(LINK_COLUMNS, [("ab", "a", "b", "walk", 1_000.0, None, None, None),
                                 ("ba", "b", "a", "walk", 1_000.0, None, None, None)])
    trips = table(["trip_id", "origin_lon_deg", "origin_lat_deg", "destination_lon_deg",
                   "destination_lat_deg", "departure_s", "class_id", "modes"],
                  [(i, *positions["a"], *positions["b"], START, "all", "transit") for i in ("p", "q")])
    folder = tmp_path / "in"
    network = oms.Network.from_tables(write(folder, "nodes.parquet", nodes), write(folder, "links.parquet", links))
    transit = oms.Transit.from_gtfs(tmp_path / "feed" / "gtfs.zip", network)
    demand = oms.Demand.from_table(write(folder, "trips.parquet", trips), ["all"], network, transit=transit)
    return oms.run(network, demand, transit, window=(START, END), out=tmp_path / "run"), network


def test_an_opened_run_with_a_timetable_gives_the_fresh_runs_pages(tmp_path):
    fresh, network = tram_files(tmp_path)
    opened = oms.Run.open(fresh.record)
    assert pages(opened, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")
    assert opened.gtfs_report == fresh.gtfs_report
    # A network passed, and the transit rebuilt on it: the demand is rebuilt on both.
    assert pages(oms.Run.open(fresh.record, network=network), tmp_path / "passed") == pages(fresh, tmp_path / "fresh")


def opened(record, **passed):
    """Run.open(record, **passed) and the texts of the warnings it gave."""
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        run = oms.Run.open(record, **passed)
    assert all(w.category is UserWarning for w in caught), caught
    return run, [str(w.message) for w in caught]


def edit_record(record, old, new):
    text = Path(record).read_text(encoding="utf-8")
    assert old in text, old
    Path(record).write_text(text.replace(old, new, 1), encoding="utf-8")


def test_an_input_not_the_runs_or_without_a_fingerprint_warns_and_the_opening_goes_on(tmp_path):
    fresh = hand_files(tmp_path)
    nodes, links, _ = hand_tables()
    # Another bike speed (another fingerprint) and road classes (drawing only), to tell its
    # drawing from the record's network.
    classed = {**links, "road_class": ["cycleway", "primary", "residential"]}
    other = oms.Network.from_tables(nodes, classed, capacity_per_lane=1_800.0, bike_speed=16.0, walk_speed=4.8)
    run, said = opened(fresh.record, network=other)
    changed = [w for w in said if "the network changed since the record was written" in w]
    assert len(changed) == 1 and re.search("its fingerprint is [0-9a-f]{16}, the record's [0-9a-f]{16}", changed[0]), said
    # What the page then shows: the journeys and counts of the tables (the hand run's 7 trips in
    # 2 pairs, the pairs' arrays the fresh page's), on the network passed (its three classes).
    meta, arrays = decoded(run.map(tmp_path / "other.html"))
    fresh_meta, fresh_arrays = decoded(fresh.map(tmp_path / "fresh-other.html"))
    assert (meta["trips"], meta["n_pairs"]) == (fresh_meta["trips"], fresh_meta["n_pairs"]) == (7, 2)
    assert all(arrays[k] == fresh_arrays[k] for k in arrays if k.startswith("q"))
    assert sorted(meta["road_classes"]) == ["cycleway", "primary", "residential"]
    assert fresh_meta["road_classes"] is None
    tram_files(tmp_path / "tram")
    run, said = opened(fresh.record, transit=_transit(tmp_path / "tram"))
    assert said == ["the transit passed is set aside: the run had none"], said
    text = Path(fresh.record).read_text(encoding="utf-8")
    network_line = re.search(r'\[result\.network\][^\[]*?(fingerprint = "[0-9a-f]+"\n)', text).group(1)
    edit_record(fresh.record, network_line, "")
    run, said = opened(fresh.record)
    assert said == [f"run_record.toml: the record has no fingerprint of the network: the network given or rebuilt "
                    f"({network_line.split(chr(34))[1]}) cannot be compared with the run's; the work goes on with it"], said
    assert pages(run, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")
    edit_record(fresh.record, "[result.network]\n", "[result.network]\nspeed = 1\n")
    run, said = opened(fresh.record)
    assert said == ["run_record.toml: 1 keys a record never holds, ignored: speed [result.network]",
                    f"run_record.toml: the record has no fingerprint of the network: the network given or rebuilt "
                    f"({network_line.split(chr(34))[1]}) cannot be compared with the run's; the work goes on with it"], said
    assert pages(run, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")


def test_a_table_that_cannot_be_read_is_refused_and_a_record_without_its_end_opened(tmp_path):
    fresh = hand_files(tmp_path)
    record = Path(fresh.record)
    kept = record.read_text(encoding="utf-8")
    stopped = re.search(r"stopped_by_rule = (true|false)", kept).group(0)
    edit_record(record, stopped, 'stopped_by_rule = "no"')
    with pytest.raises(ValueError, match="result.stopped_by_rule is no, not a boolean"):
        oms.Run.open(record)
    edit_record(record, 'stopped_by_rule = "no"\n', "")
    opened = oms.Run.open(record)
    assert opened.stopped_by_rule is None
    summary = Path(opened.summary(tmp_path / "unrecorded.html")).read_text(encoding="utf-8")
    assert '<span data-k="stopped-by">The record does not say what ended the run</span>' in summary
    page = Path(opened.map(tmp_path / "unrecorded-map.html")).read_text(encoding="utf-8")
    assert "its end not recorded" in page
    record.write_text(kept, encoding="utf-8")
    links_file = tmp_path / "run" / "links.parquet"
    table_ = pq.read_table(links_file)
    pq.write_table(table_.rename_columns(["link"] + table_.column_names[1:]), links_file)
    with pytest.raises(ValueError, match=r"links\.parquet: it has no column link_id"):
        oms.Run.open(record)
    (tmp_path / "run" / "routes.parquet").rename(tmp_path / "routes.moved")
    with pytest.raises(OSError, match=r"routes\.parquet"):
        oms.Run.open(record)


def _transit(tmp_path):
    network = oms.Network.from_tables(tmp_path / "in" / "nodes.parquet", tmp_path / "in" / "links.parquet")
    return oms.Transit.from_gtfs(tmp_path / "feed" / "gtfs.zip", network)


def test_a_pages_table_changed_after_opening_is_refused_by_name(tmp_path):
    fresh = hand_files(tmp_path)
    (tmp_path / "run" / "trips.parquet").unlink()
    with pytest.raises(OSError, match=r"trips\.parquet"):
        fresh.map(tmp_path / "map.html")
    assert not (tmp_path / "map.html").exists()


def test_routes_are_rebuilt_the_same_bytes_and_an_out_holding_an_input_named_routes_is_refused(tmp_path):
    fresh = hand_files(tmp_path)
    again = oms.run_file(settings(fresh.record), out=tmp_path / "again")
    assert Path(again.routes).read_bytes() == Path(fresh.routes).read_bytes()
    nodes, links, trips = hand_tables()
    out = tmp_path / "out"
    trips_file = write(out, "routes.parquet", trips)
    before = trips_file.read_bytes()
    with pytest.raises(ValueError, match="routes.parquet is one of the run's input files"):
        hand_run(nodes, links, trips_file, out)
    assert trips_file.read_bytes() == before
