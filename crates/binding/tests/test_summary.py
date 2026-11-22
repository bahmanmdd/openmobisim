"""Run.summary on io-parquet's hand run (test_hand_run.py): the file is written and returned, names
no address, is the same bytes when written twice, shows the hand numbers; a path that is a file of
the run is refused (the check Run.map makes). The page is checked as a file; its behaviour in a
browser is not tested automatically.

The hand numbers (derived in crates/summary/tests/hand_run.rs): 7 trips, 5 finished, 1 still
travelling at the window's end, 1 without an alternative; 9.0 vehicle-km (3 cars on 2 km and 3 on
1 km); 1.5 km cycled (one bike on the 1 500 m path) and 3.0 km walked (two walkers on it); three
iterations, ended by the iteration cap.
"""

import re
import time
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from test_hand_run import hand_run, hand_tables
from test_map import ADDRESS


def value(text, key):
    """The text of the element named `key` (`data-k="key"`)."""
    found = re.search(r'data-k="' + re.escape(key) + r'">([^<]*)<', text)
    assert found, f"no {key}"
    return found.group(1)


def test_summary_writes_one_file_the_same_bytes_twice_with_the_hand_numbers(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    path = tmp_path / "hand run.html"
    start = time.perf_counter()
    assert Path(result.summary(path)) == path
    print(f"summary: {time.perf_counter() - start:.3f} s, {path.stat().st_size} bytes")
    first = path.read_bytes()
    result.summary(str(path))
    assert path.read_bytes() == first, "a second call wrote other bytes"
    (tmp_path / "other").mkdir()
    assert Path(result.summary(tmp_path / "other" / "hand run.html")).read_bytes() == first
    text = first.decode("utf-8")
    assert "http://" not in text and "https://" not in text
    found = ADDRESS.search(text)
    assert found is None, f"an address: {found.group(0)}"
    assert "<title>hand run</title>" in text
    hand = {"trips": "7", "finished": "5", "still-travelling": "1", "without-alternative": "1",
            "vehicle-km": "9.0", "km-cycled": "1.5", "km-walked": "3.0",
            "stopped-by": "The iteration cap"}
    assert {k: value(text, k) for k in hand} == hand
    assert len(re.findall(r'data-k="it\.\d+\.mean"', text)) == 3
    # The record, run_record.toml, shown whole.
    record = Path(result.record).read_text()
    assert record.splitlines()[0].replace("&", "&amp;") in text


def test_a_file_of_the_run_is_refused_and_left_unchanged(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    for output in (result.trips, result.links, result.iterations, result.routes, result.record):
        before = Path(output).read_bytes()
        with pytest.raises(ValueError, match=r"Run\.summary: .*" + re.escape(Path(output).name)):
            result.summary(output)
        assert Path(output).read_bytes() == before
    folder = tmp_path / "in"
    folder.mkdir()
    files = []
    for name, columns in zip(("nodes", "links", "trips"), hand_tables()):
        pq.write_table(pa.table(columns), folder / f"{name}.parquet")
        files.append(folder / f"{name}.parquet")
    from_files = hand_run(*files, tmp_path / "files")
    before = files[1].read_bytes()
    with pytest.raises(ValueError, match=re.escape("links.parquet")):
        from_files.summary(folder / ".." / "in" / "links.parquet")
    assert files[1].read_bytes() == before
    (tmp_path / "folder").mkdir()
    with pytest.raises(OSError):
        result.summary(tmp_path / "folder")
    assert not any(p.name.endswith(".partial") for p in tmp_path.iterdir())


def test_a_title_given_names_both_pages_else_the_files_stem(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    summary = Path(result.summary(tmp_path / "s.html", title='Hand run: "<a>"')).read_text(encoding="utf-8")
    assert "<title>Hand run: &quot;&lt;a&gt;&quot;</title>" in summary
    page = Path(result.map(tmp_path / "m.html", title="Hand run")).read_text(encoding="utf-8")
    assert "<title>Hand run</title>" in page and '"title":"Hand run"' in page
    assert "<title>m</title>" in Path(result.map(tmp_path / "m.html")).read_text(encoding="utf-8")
    with pytest.raises(ValueError, match="run_record.toml"):
        result.map(result.record, title="anything")
