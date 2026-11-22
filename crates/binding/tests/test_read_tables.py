"""A run's tables read back for its pages, as the owner's rule says: a file is refused only when it
cannot be read (test_open.py); what it holds that does not agree is set aside with a warning
naming the file and what was found, and the pages are drawn from what agrees.

The hand run (test_hand_run.py) from three files: pair a->b holds 4 travellers (the walk of 2,
the bike, `lost` without an alternative), pair a->c the three cars; links.parquet's 4 rows: ab
in the bins from 25 200 s and 28 200 s, st1 from 25 200 s, st2 from 25 320 s (bins 0, 50, 0, 2).
1. The bike said to have driven (trips.parquet's mode `car`), its routes rows removed, or its
   three rows given twice: the bike is left out of the journeys: the warning names it, the map's
   footer count is 1, pair a->b holds 3 travellers, the summary's footer says 1 trip.
2. A `walk_gap` without its seconds (the bike's first leg): the bike's legs' seconds are unknown
   (NaN, so the map's strip is not drawn) and its journey's door-to-door time stays 360 s.
3. links.parquet's rows given twice: the first used, the pages the fresh run's, byte for byte.
4. The record's `link_bin` 60 changed to 120, with a bike that enters ab at 25 260 s (bin 1 of
   60 s, off a grid of 120 s): that row set aside; the window moved 30 s on (`window_start_s`
   25 230 s, `[result] window_end_used_s` 28 830 s): all 5 rows set aside.
5. routes.parquet rewritten by pyarrow with its defaults (snappy, the list's inner field
   `element`) and its columns in reverse order: read as written, no warning, the same pages.
6. An empty route (a trip whose two doors are one node): an empty list of links, not null, and
   both pages written.
7. iterations.parquet without its column would_change_share, as a run written before that share
   was reported has it: the run opens with one warning naming the file and the column, and the
   summary is written, giving the warning again and showing the share as unknown (a dash).

For bikes: the bike's journey is the one whose routes disagree or lack seconds above.
"""

import math
import re
import warnings
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq

from test_drawing import decoded
from test_files import write
from test_hand_run import START, hand_run, hand_tables
from test_open import edit_record, hand_files, opened, pages


def rewrite(path, change):
    """`path` read by pyarrow, its rows (a list of dicts) changed by `change`, written back by
    pyarrow with its defaults, the columns as they were."""
    table = pq.read_table(path)
    rows = change(table.to_pylist())
    pq.write_table(pa.Table.from_pylist(rows, schema=table.schema), path)


def pair_ab(meta, a):
    """The index of pair a->b (doors 0 and 1 500 m: 15 000 dm)."""
    return next(p for p in range(meta["n_pairs"]) if a["qdx"][p] == 15_000)


def left_out_pages(tmp_path, run):
    meta, a = decoded(Path(run.map(tmp_path / "map.html")))
    summary = Path(run.summary(tmp_path / "summary.html")).read_text(encoding="utf-8")
    return meta["left_out"], a["qn"][pair_ab(meta, a)], re.search(r'data-k="left-out">(\d+)<', summary)


def test_a_trip_whose_routes_disagree_with_its_mode_is_left_out_with_a_warning(tmp_path):
    for case, change, table in [
        ("mode", lambda rows: [{**r, "mode": "car"} if r["trip_id"] == "bike" else r for r in rows], "trips"),
        ("missing", lambda rows: [r for r in rows if r["trip_id"] != "bike"], "routes"),
        ("twice", lambda rows: rows + [r for r in rows if r["trip_id"] == "bike"], "routes"),
    ]:
        fresh = hand_files(tmp_path / case)
        rewrite(tmp_path / case / "run" / f"{table}.parquet", change)
        run, said = opened(fresh.record)
        routes = str(tmp_path / case / "run" / "routes.parquet")
        reason = ("their legs are misnumbered, of an unknown kind, without a value their kind needs, or "
                  "name a link, run or stop the inputs do not hold") if case == "twice" else (
                  "their legs do not agree with their mode in trips.parquet")
        assert said == [f"{routes}: 1 trips left out of the journeys: {reason}: bike"], said
        left_out, travellers, summary = left_out_pages(tmp_path / case, run)
        assert (left_out, travellers, summary.group(1)) == (1, 3, "1"), case


def test_a_walk_without_its_seconds_leaves_its_journeys_legs_unknown(tmp_path):
    fresh = hand_files(tmp_path)
    routes = tmp_path / "run" / "routes.parquet"
    rewrite(routes, lambda rows: [{**r, "seconds": None} if (r["trip_id"], r["leg"]) == ("bike", 0) else r
                                  for r in rows])
    run, said = opened(fresh.record)
    assert said == [f"{routes}: 1 trips with a walk whose seconds are missing: their legs' seconds are unknown: bike"]
    meta, a = decoded(Path(run.map(tmp_path / "map.html")))
    p = pair_ab(meta, a)
    bike = next(o for o in range(a["qs"][p], a["qs"][p + 1]) if a["om"][o] == 1)
    seconds = a["lsec"][a["ol"][bike]:a["ol"][bike + 1]]
    assert [math.isnan(x) for x in seconds] == [True, True, False] and seconds[2] == 0.0, seconds
    assert a["ot"][bike] == 360.0


def test_link_rows_given_twice_the_first_is_used(tmp_path):
    fresh = hand_files(tmp_path)
    links = tmp_path / "run" / "links.parquet"
    rewrite(links, lambda rows: rows + rows)
    run, said = opened(fresh.record)
    assert said == [f"{links}: 4 (link, bin) rows given twice, the first used: ab at bin 0, ab at bin 50, "
                    "st1 at bin 0, st2 at bin 2"], said
    assert pages(run, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")


def test_rows_off_the_records_grid_or_window_are_set_aside(tmp_path):
    nodes, links, trips = hand_tables()
    trips = {k: list(v) for k, v in trips.items()}
    at = trips["trip_id"].index("bike")
    for k in trips:
        trips[k].append("bike-60" if k == "trip_id" else START + 60 if k == "departure_s" else trips[k][at])
    files = [write(tmp_path / "in", f"{n}.parquet", t) for n, t in (("nodes", nodes), ("links", links), ("trips", trips))]
    fresh = hand_run(*files, tmp_path / "run")
    links_file = tmp_path / "run" / "links.parquet"
    edit_record(fresh.record, "link_bin = 60", "link_bin = 120")
    _, said = opened(fresh.record)
    grid = "rows set aside: their bin does not start on the record's grid inside its window"
    assert said == [f"{links_file}: 1 {grid}: ab at 25260"], said
    edit_record(fresh.record, "link_bin = 120", "link_bin = 60")
    edit_record(fresh.record, "window_start_s = 25200", "window_start_s = 25230")
    edit_record(fresh.record, "window_end_used_s = 28800", "window_end_used_s = 28830")
    _, said = opened(fresh.record)
    assert said == [f"{links_file}: 5 {grid}: ab at 25200, ab at 25260, ab at 28200, st1 at 25200, st2 at 25320"], said


def test_routes_rewritten_by_pyarrow_in_another_order_read_the_same(tmp_path):
    fresh = hand_files(tmp_path)
    routes = tmp_path / "run" / "routes.parquet"
    table = pq.read_table(routes)
    pq.write_table(table.select(list(reversed(table.column_names))), routes)
    assert str(pq.read_schema(routes).field("links").type) == "list<element: string>"
    run, said = opened(fresh.record)
    assert said == []
    assert pages(run, tmp_path / "opened") == pages(fresh, tmp_path / "fresh")


def test_an_empty_route_is_an_empty_list_and_both_pages_take_it(tmp_path):
    nodes, links, trips = hand_tables()
    trips = {k: list(v) for k, v in trips.items()}
    at = trips["trip_id"].index("bike")
    for k in trips:
        trips[k].append("stay" if k == "trip_id" else 0.0 if k == "destination_x_m" else trips[k][at])
    result = hand_run(nodes, links, trips, tmp_path / "run")
    rows = [r for r in pq.read_table(result.routes).to_pylist() if r["trip_id"] == "stay"]
    assert [(r["kind"], r["links"]) for r in rows] == [("walk_gap", None), ("bike", []), ("walk_gap", None)], rows
    assert Path(result.map(tmp_path / "map.html")).is_file()
    assert Path(result.summary(tmp_path / "summary.html")).is_file()


def test_an_iterations_table_without_the_share_who_would_change_opens_with_a_warning(tmp_path):
    fresh = hand_files(tmp_path)
    iterations = tmp_path / "run" / "iterations.parquet"
    table = pq.read_table(iterations)
    pq.write_table(table.drop_columns(["would_change_share"]), iterations)
    warning = (f"{iterations}: it has no column would_change_share (a run written before that "
               "share was reported): the share of trips that would change is read as unknown")
    run, said = opened(fresh.record)
    assert said == [warning], said
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        summary = Path(run.summary(tmp_path / "summary.html")).read_text(encoding="utf-8")
    assert [str(w.message) for w in caught] == [warning]
    assert re.search(r'data-k="it\.0\.would">([^<]*)<', summary).group(1) == "\u2014"
