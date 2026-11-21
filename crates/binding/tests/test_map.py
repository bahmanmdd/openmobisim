"""Run.map on io-parquet's hand run (test_hand_run.py): the file is written and returned, names no
address, is the same bytes when written twice; a path that is a file of the run is refused; and,
where a Chrome or Chromium binary is found, the page opens headless and shows the hand values.

The hand values (derived in crates/io-parquet/tests/hand_run.rs and crates/map/tests/hand_run.rs):
3 links (ab, st1, st2); 2 rows of car traffic (the three cars enter st1 in the bin from 0 s and
st2 in the bin from 120 s); 2 pairs by door; 3 journeys (walk, bike, car); no transit hop.
- st1 in its first bin of 60 s: 3 cars, 180 veh/h; their mean time (144 + 145 + 146) / 3 = 145 s
  against 144 s free flow, a delay of 0.7 %, shown "1%"; 180 / 1 800 = 10 % of capacity; one pair
  (pair 2, a->c) of 3 travellers whose journeys use it.
- Pair 1 (a->b): 4 travellers, 1.5 km apart, a 1.2 km W and b 0.3 km E of the centre (the mean of the
  links' ends, at 1 166.7 m); the walk journey of 2 (a share of 2 / 4 = 50 %; its
  finished traveller 1 125 s, shown "19 min"), the bike of 1 (25 %, 360 s, "6.0 min"), 1 without an
  alternative.
For a bike: the bike's journey is one of the three and its share and minutes are checked.
"""

import re
import shutil
import subprocess
import time
from html.parser import HTMLParser
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest

from test_hand_run import hand_run, hand_tables

# An address: a scheme's or a protocol-relative "//host.domain". Base64 has no ".", and a comment's
# "//" is followed by a space, so neither is one.
ADDRESS = re.compile(r"//[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+")
CHROMES = ("chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "chrome")
MAC_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


def chrome():
    """A Chrome or Chromium binary, or None."""
    for name in CHROMES:
        found = shutil.which(name)
        if found:
            return found
    return MAC_CHROME if Path(MAC_CHROME).is_file() else None


class Texts(HTMLParser):
    """The text of each element with an id, its children's included."""

    def __init__(self):
        super().__init__()
        self.open, self.texts = [], {}

    def handle_starttag(self, tag, attrs):
        if tag in ("br", "input", "img", "meta", "link", "path", "text"):
            return
        self.open.append(dict(attrs).get("id"))

    def handle_endtag(self, tag):
        if self.open and tag not in ("br", "input", "img", "meta", "link", "path", "text"):
            self.open.pop()

    def handle_data(self, data):
        for element in self.open:
            if element:
                self.texts[element] = self.texts.get(element, "") + data + " "


def text(texts, element):
    return " ".join(texts.get(element, "").split())


def test_map_writes_one_file_naming_no_address_the_same_bytes_twice(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    path = tmp_path / "hand run.html"
    start = time.perf_counter()
    returned = result.map(path)
    print(f"map: {time.perf_counter() - start:.3f} s, {path.stat().st_size} bytes")
    assert Path(returned) == path and path.is_file()
    first = path.read_bytes()
    result.map(str(path))
    assert path.read_bytes() == first, "a second call wrote other bytes"
    (tmp_path / "other").mkdir()
    assert Path(result.map(tmp_path / "other" / "hand run.html")).read_bytes() == first
    text = first.decode("utf-8")
    assert "http://" not in text and "https://" not in text
    found = ADDRESS.search(text)
    assert found is None, f"an address: {found.group(0)}"
    # The title is the file's stem.
    assert "<title>hand run</title>" in text
    assert sorted(p.name for p in tmp_path.iterdir()) == ["hand run.html", "other", "run"]


def test_a_folder_as_the_path_is_an_os_error_and_leaves_no_partial_file(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    (tmp_path / "folder").mkdir()
    with pytest.raises(OSError):
        result.map(tmp_path / "folder")
    assert sorted(p.name for p in tmp_path.iterdir()) == ["folder", "run"]
    assert list((tmp_path / "folder").iterdir()) == []


def test_a_file_of_the_run_is_refused_and_left_unchanged(tmp_path):
    result = hand_run(*hand_tables(), tmp_path / "run")
    for output in (result.trips, result.links, result.iterations, result.routes, result.record):
        before = Path(output).read_bytes()
        with pytest.raises(ValueError, match=re.escape(Path(output).name)):
            result.map(output)
        assert Path(output).read_bytes() == before
    # The run's input files, one given through another spelling of its path.
    folder = tmp_path / "in"
    folder.mkdir()
    files = []
    for name, columns in zip(("nodes", "links", "trips"), hand_tables()):
        pq.write_table(pa.table(columns), folder / f"{name}.parquet")
        files.append(folder / f"{name}.parquet")
    from_files = hand_run(*files, tmp_path / "files")
    for path in (files[0], folder / ".." / "in" / "trips.parquet"):
        before = Path(path).read_bytes()
        with pytest.raises(ValueError, match=re.escape(Path(path).name)):
            from_files.map(path)
        assert Path(path).read_bytes() == before
    assert not any(p.name.endswith(".partial") for p in folder.iterdir())


def test_the_page_opens_and_shows_the_hand_values(tmp_path):
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    result = hand_run(*hand_tables(), tmp_path / "run")
    path = Path(result.map(tmp_path / 'hand "<map>".html'))
    start = time.perf_counter()
    done = subprocess.run(
        [binary, "--headless=new", "--disable-gpu", "--no-sandbox",
         f"--user-data-dir={tmp_path / 'profile'}", "--virtual-time-budget=5000", "--dump-dom",
         path.as_uri() + "#p=0&l=1&b=0"],
        capture_output=True, text=True, timeout=120,
    )
    print(f"headless Chrome: {time.perf_counter() - start:.1f} s")
    body = re.search(r"<body([^>]*)>", done.stdout)
    assert body, done.stdout[:2000] + done.stderr[-2000:]
    attributes = dict(re.findall(r'data-([a-z]+)="([^"]*)"', body.group(1)))
    assert attributes.get("state") == "ready", attributes
    read = {k: int(attributes[k]) for k in ("links", "rows", "pairs", "journeys", "hops")}
    assert read == {"links": 3, "rows": 2, "pairs": 2, "journeys": 3, "hops": 0}
    texts = Texts()
    texts.feed(done.stdout)
    # The title, the file's stem, shown as written.
    assert text(texts.texts, "title") == 'hand "<map>"'
    # st1 in its first bin.
    link = text(texts.texts, "linkinfo")
    assert link.startswith("Link st1 · car link · 1 lane · 50 km/h · 2,000 m"), link
    assert "180 veh/h · delay 1% · 10% of capacity" in link, link
    assert "3 travellers · 1 pair Pair 2 3" in link, link
    # Pair 1's journeys: name, share, minutes.
    pair = text(texts.texts, "tripcard")
    assert pair.startswith("A 1.2 km W of the centre → B 0.3 km E of the centre 4 travellers · 1.5 km walk 50% bike 25%"), pair
    assert "Walk 50% 19 min Bike 25% 6.0 min" in pair, pair
    assert pair.endswith("1 without an alternative"), pair


# The volume ramp's colours are blues: a pixel bluer than green by 20 and with little red. The
# cyclists' teal and the network's grey are not.
BLUES = """(() => { const d = document.getElementById("map").getContext("2d").getImageData(750, 440, 1, 120).data, ys = [];
  for (let i = 0; i < 120; i++) { const r = d[4 * i], g = d[4 * i + 1], b = d[4 * i + 2]; if (b > g + 20 && r < 60 && b > 100) ys.push(440 + i); }
  return ys; })()"""


def text_of(page, selector):
    return page.run(f"(document.querySelector({selector!r}) || {{}}).textContent || ''")


def test_real_clicks_select_a_link_open_a_pair_and_keep_the_view(tmp_path):
    """Real presses of the mouse on the hand map, with road traffic and cyclists ticked, coloured by
    volume, bin 0 (07:00–07:01), the view centred on st1 at 5 m a pixel: st1 (a->b, along y = 0)
    lies from x = 600 to 900 px at y = 475 px; the cyclists' path ab lies on the same line (one
    cyclist: 8 px wide, on the line, being one-way); st1's ribbon on the right of its travel, below.
    By hand: st1 in bin 0 carries 3 cars, 180 veh/h, a mean time of 145 s against 144 s free flow
    (delay 0.7 %, shown 1 %), 10 % of its 1 800 veh/h; one pair uses it, pair 2 (a->c, 3 travellers,
    2.5 km apart; a lies 1.2 km W and c 1.3 km E of the centre, the mean of the links' ends at
    1 166.7 m). Then each click as the approved page answers it (see the steps)."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    from cdp import Page

    result = hand_run(*hand_tables(), tmp_path / "run")
    path = Path(result.map(tmp_path / "hand.html"))
    page = Page(binary, tmp_path / "profile", path.as_uri() + "#c=volume&ly=rb&b=0&z=750.0,0.0,5.000")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        view = lambda: page.run("new URLSearchParams(location.hash.slice(1)).get('z')")  # noqa: E731
        start = view()
        ys = page.run(BLUES)
        assert ys and all(y > 475 for y in ys), ys
        ribbon = ys[len(ys) // 2]
        # 1. The ribbon, the cyclists' line beside it: st1.
        page.click(750, ribbon)
        link = text_of(page, "#linkinfo")
        assert link.startswith("Link st1 · car link · 1 lane · 50 km/h · 2,000 m"), link
        assert "180 veh/h · delay 1% · 10% of capacity" in link and "3 travellers · 1 pair" in link, link
        # 2. The cyclists' line itself: the bike link ab.
        page.click(750, 475)
        assert text_of(page, "#linkinfo").startswith("Bike link · 1,500 m"), text_of(page, "#linkinfo")
        # 3. Cyclists unticked: the same point is st1's base line, never the walk and bike path under it.
        box = page.run("(() => { const r = document.querySelector('#layerrows input[data-layer=bike]').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()")
        page.click(*box)
        page.click(750, 475)
        assert text_of(page, "#linkinfo").startswith("Link st1 ·"), text_of(page, "#linkinfo")
        # 4. The pair row: pair 2's journeys, the view where it was, st1 still selected, the row open.
        row = page.run("(() => { const r = document.querySelector('#linkinfo .prow').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()")
        page.click(*row)
        pair = text_of(page, "#tripcard")
        assert pair.startswith("A1.2 km W of the centre → B1.3 km E of the centre3 travellers · 2.5 km"), pair
        assert text_of(page, "#linkinfo").startswith("Link st1 ·") and view() == start
        assert page.run("document.querySelector('#linkinfo .prow').classList.contains('on')")
        # 5. The empty map: nothing closes while the pair opened from the panel is shown.
        page.click(750, 800)
        assert text_of(page, "#linkinfo").startswith("Link st1 ·") and text_of(page, "#tripcard") == pair
        # 6. The open row again (where it is now, below the pair's panel): the pair closes, the link
        # stays, the view has not moved.
        row = page.run("(() => { const r = document.querySelector('#linkinfo .prow.on').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()")
        page.click(*row)
        assert text_of(page, "#tripcard") == "" and text_of(page, "#linkinfo").startswith("Link st1 ·")
        assert view() == start
        # 7. Esc: the link panel closes.
        page.key("Escape")
        assert page.run("document.getElementById('linkinfo').hidden")
        assert page.run("document.body.dataset.state") == "ready"
    finally:
        page.close()


def test_a_hash_naming_what_the_page_does_not_hold_is_ignored(tmp_path):
    """A pair, a link, a hop or a bin the hand map does not have, named in the address: the page
    opens as without them (2 pairs, 3 links, 60 bins, no hop)."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    result = hand_run(*hand_tables(), tmp_path / "run")
    path = Path(result.map(tmp_path / "hand.html"))
    # A car-only link named as a bike or walk link (st1 is link 1, st2 link 2) is ignored too.
    for bad in ("p=9999", "p=-1", "p=x", "l=3", "l=99", "li=t0", "li=b7", "b=60", "b=x", "hm=9",
                "li=b1", "li=w2"):
        done = subprocess.run(
            [binary, "--headless=new", "--disable-gpu", "--no-sandbox",
             f"--user-data-dir={tmp_path / 'profile'}", "--virtual-time-budget=4000", "--dump-dom",
             path.as_uri() + "#" + bad],
            capture_output=True, text=True, timeout=120,
        )
        body = re.search(r"<body([^>]*)>", done.stdout)
        attributes = dict(re.findall(r'data-([a-z]+)="([^"]*)"', body.group(1)))
        assert attributes.get("state") == "ready", (bad, attributes)
        texts = Texts()
        texts.feed(done.stdout)
        assert text(texts.texts, "tripcard") == "" and texts.texts.get("linkinfo", "").strip() == "", bad


def open_bad(binary, tmp_path, path, address):
    """The page at `path` with `address`: its body's state and the link and pair panels' texts."""
    done = subprocess.run(
        [binary, "--headless=new", "--disable-gpu", "--no-sandbox",
         f"--user-data-dir={tmp_path / 'profile'}", "--virtual-time-budget=4000", "--dump-dom",
         path.as_uri() + "#" + address],
        capture_output=True, text=True, timeout=120,
    )
    body = re.search(r"<body([^>]*)>", done.stdout)
    texts = Texts()
    texts.feed(done.stdout)
    state = dict(re.findall(r'data-([a-z]+)="([^"]*)"', body.group(1))).get("state")
    return state, text(texts.texts, "linkinfo"), text(texts.texts, "tripcard")


def test_a_bike_or_walk_link_on_a_page_without_that_layer_is_ignored(tmp_path):
    """The hand run's three cars alone: no cyclists, no walkers. An address naming a bike or a walk
    link (the path ab is link 0) is ignored: the page opens ready, no link selected."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    nodes, links, trips = hand_tables()
    cars = {k: [v for v, m in zip(trips[k], trips["modes"]) if m == "car"] for k in trips}
    path = Path(hand_run(nodes, links, cars, tmp_path / "run").map(tmp_path / "cars.html"))
    for bad in ("li=b0", "li=w0", "li=b1", "li=w1"):
        assert open_bad(binary, tmp_path, path, bad) == ("ready", "", ""), bad


def ribbon_ys(page, x):
    """The rows of screen pixels at column `x` drawn in the volume ramp's blues: a ribbon."""
    return page.run(BLUES.replace("getImageData(750, 440", f"getImageData({x}, 440"))


def click_on(page, selector):
    box = page.run(f"(() => {{ const r = document.querySelector({selector!r}).getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; }})()")
    page.click(*box)


def state(page):
    """What is open: the link panel's first words, whether a pair's journeys are shown, the marked
    row's pair."""
    shown = not page.run("document.getElementById('linkinfo').hidden")
    link = text_of(page, "#linkinfo") if shown else ""
    on = page.run("(document.querySelector('#linkinfo .prow.on') || {}).textContent || ''") if shown else ""
    return " ".join(link.split()[:2]), text_of(page, "#tripcard") != "", on.split("Pair ")[-1][:1] if on else ""


def test_four_click_sequences_end_as_the_rule_says(tmp_path):
    """The rule: a click on a link selects it; while a pair is open, a click on the empty map closes
    nothing; the open pair's row is marked in whatever link panel lists it. The hand map, road only,
    coloured by volume, centred on st1 at 5 m a pixel: st1 from x = 600 to 900 px, st2 from
    900 to 1 100 px at y = 475, each ribbon below it. Pair 2 (a->c, by car on st1 then st2) is the
    pair both list; the address names it p=1. The whole window shown (st2's cars enter it in bin 2).
    Each sequence's end: the link last clicked open, pair 2
    open, its row marked."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    from cdp import Page

    result = hand_run(*hand_tables(), tmp_path / "run")
    path = Path(result.map(tmp_path / "hand.html"))
    view = "c=volume&ly=r&z=750.0,0.0,5.000"
    page = Page(binary, tmp_path / "profile", path.as_uri() + "#" + view)
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        st1, st2 = ribbon_ys(page, 750), ribbon_ys(page, 1000)
        assert st1 and st2
        y1, y2 = st1[len(st1) // 2], st2[len(st2) // 2]
        # 1. st1, its pair row, st2, the empty map.
        page.click(750, y1)
        click_on(page, "#linkinfo .prow")
        page.click(1000, y2)
        page.click(750, 800)
        assert state(page) == ("Link st2", True, "2"), state(page)
        # 2. ×, st1, the empty map.
        click_on(page, "#linkclose")
        assert state(page) == ("", True, "")
        page.click(750, y1)
        page.click(750, 800)
        assert state(page) == ("Link st1", True, "2"), state(page)
        address = page.run("location.href")
    finally:
        page.close()
    # 3. The pair from the address, st1, the empty map.
    page = Page(binary, tmp_path / "profile3", path.as_uri() + "#" + view + "&p=1")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        assert state(page) == ("", True, "")
        page.click(750, y1)
        page.click(750, 800)
        assert state(page) == ("Link st1", True, "2"), state(page)
    finally:
        page.close()
    # 4. The address of sequence 2 opened again, the empty map.
    page = Page(binary, tmp_path / "profile4", address)
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        assert state(page) == ("Link st1", True, "2"), state(page)
        page.click(750, 800)
        assert state(page) == ("Link st1", True, "2"), state(page)
    finally:
        page.close()


def with_cyclists(nodes, links, trips, n):
    """The hand tables with `n` more cyclists from a to b, leaving with the hand run's bike."""
    trips = {k: list(v) for k, v in trips.items()}
    at = trips["trip_id"].index("bike")
    for i in range(n):
        for k in trips:
            trips[k].append(f"bike-{i}" if k == "trip_id" else trips[k][at])
    return nodes, links, trips


def test_a_line_drawn_on_top_of_a_ribbon_wins_where_the_pointer_is_on_both(tmp_path):
    """With 3 000 cyclists (the full width, rules::FLOW_FULL), the cyclists' path ab (8 px wide on the
    line y = 475, from 471 to 479 px) is drawn over st1's ribbon, which begins below the line; at
    y = 478 the pointer is on both: the bike link wins."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    from cdp import Page

    result = hand_run(*with_cyclists(*hand_tables(), 2_999), tmp_path / "run")
    path = Path(result.map(tmp_path / "hand.html"))
    page = Page(binary, tmp_path / "profile", path.as_uri() + "#c=volume&ly=r&b=0&z=750.0,0.0,5.000")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        assert 478 in ribbon_ys(page, 750), "the ribbon is under y = 478"
    finally:
        page.close()
    page = Page(binary, tmp_path / "profile2", path.as_uri() + "#c=volume&ly=rb&b=0&z=750.0,0.0,5.000")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        page.click(750, 478)
        assert text_of(page, "#linkinfo").startswith("Bike link · 1,500 m"), text_of(page, "#linkinfo")
    finally:
        page.close()


# The controls as the page shows them: the "Whole period" box ticked, the time slider off, the
# slider's bin, the bin the address names ("all": the whole window), the Play button's word.
CONTROLS = """[document.getElementById('allbins').checked, document.getElementById('bin').disabled,
  document.getElementById('bin').value, new URLSearchParams(location.hash.slice(1)).get('b'),
  document.getElementById('play').textContent]"""
# Play has stepped: the address names a bin after the first.
STEPPED = "/^[1-9][0-9]*$/.test(new URLSearchParams(location.hash.slice(1)).get('b') || '')"


def readings(page, expression, seconds=1.2):
    """The distinct values of `expression`, read every 0.1 s for `seconds` s: over 1.2 s, more than
    two of Play's steps (0.45 s each), one value when nothing steps."""
    seen, end = [], time.monotonic() + seconds
    while time.monotonic() < end:
        value = page.run(expression)
        seen += [] if value in seen else [value]
        time.sleep(0.1)
    return seen


def playing(binary, tmp_path, layers):
    """The hand map with `layers` ticked, coloured by volume, centred on st1 at 5 m a pixel, the
    whole window shown; Play pressed and seen stepping."""
    from cdp import Page

    path = Path(hand_run(*hand_tables(), tmp_path / "run").map(tmp_path / "hand.html"))
    page = Page(binary, tmp_path / "profile", path.as_uri() + f"#c=volume&ly={layers}&z=750.0,0.0,5.000")
    page.wait("document.body && document.body.dataset.state === 'ready'")
    click_on(page, "#play")
    page.wait(STEPPED)
    return page


def test_whole_period_ticked_while_playing_stops_the_play_and_shows_the_whole_window(tmp_path):
    """Play, then the "Whole period" box ticked: the play stops and the whole window is shown, the
    box, the slider and what is drawn agreeing. The hand map, road traffic alone; its window 07:00 to
    08:00 in 60 bins of 60 s; st1's cars enter it in bin 0, st2's in bin 2, so both ribbons are drawn
    only for the whole window (or bins 0 to 2 together). By the rule, over 1.2 s one reading: the box
    ticked, the slider off, the address's bin "all", the button "Play", the subtitle "all bins,
    07:00–08:00", st1's ribbon (x = 750 px) and st2's (x = 1 000 px) drawn. For bikes: Play and the
    whole window are road traffic's bins; the cyclists' layer is counted over the whole window and
    not stepped (the test below)."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    page = playing(binary, tmp_path, "r")
    try:
        click_on(page, "#allbins")
        seen = readings(page, f"""[{CONTROLS}, document.getElementById('subtitle').textContent.slice(0, 21),
            {BLUES}.length > 0, {BLUES.replace("getImageData(750, 440", "getImageData(1000, 440")}.length > 0]""")
        assert len(seen) == 1, seen
        (whole, off, _, bin_, button), subtitle, st1, st2 = seen[0]
        assert (whole, off, bin_, button, subtitle, st1, st2) == (True, True, "all", "Play", "all bins, 07:00–08:00", True, True), seen
    finally:
        page.close()


# The cyclists' teal (paper theme, #04938e) drawn 1 px wide on the line y = 475 px, half on each
# of two rows of pixels over the surface: green above red by 40 and at least the blue. The
# network's grey and the volume's blues are not.
TEALS = BLUES.replace("b > g + 20 && r < 60 && b > 100", "g > r + 40 && g >= b")


def test_road_traffic_hidden_while_playing_stops_the_play(tmp_path):
    """Play, then the "Road traffic" layer unticked (its group, the Pause button with it, hidden): the
    play stops at the bin it showed. The hand map, road traffic and cyclists ticked. By the rule,
    over 1.2 s one reading: the box unticked, the slider on, at the bin the address names, the button
    "Play", the address's layers the cyclists alone ("b"). For bikes: the cyclists' layer stays
    ticked and drawn (its teal at x = 750 px on its line, y = 475 px), and the map's pixels are the
    same at every reading: Play steps road traffic only, the cyclists are counted over the window."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    page = playing(binary, tmp_path, "rb")
    try:
        click_on(page, "#layerrows input[data-layer=road]")
        pixels = """(() => { const c = document.getElementById('map'), d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
          let h = 0; for (let i = 0; i < d.length; i++) h = (Math.imul(h, 31) + d[i]) | 0; return h; })()"""
        seen = readings(page, f"""[{CONTROLS}, new URLSearchParams(location.hash.slice(1)).get('ly'),
            document.querySelector('#layerrows input[data-layer=bike]').checked, {pixels}]""")
        assert len(seen) == 1, seen
        (whole, off, slider, bin_, button), layers, cyclists, _ = seen[0]
        assert (whole, off, slider, button, layers, cyclists) == (False, False, bin_, "Play", "b", True), seen
        assert 474 in page.run(TEALS), page.run(TEALS)
    finally:
        page.close()


# Installed in the page before its exports: every text filled on a canvas with its place, and each
# file saved (the link the page clicks to download it) kept instead of downloaded.
RECORD = """(() => { const fill = CanvasRenderingContext2D.prototype.fillText;
  CanvasRenderingContext2D.prototype.fillText = function (t, x, y) { window.texts.push([String(t), x, y]); return fill.apply(this, arguments); };
  HTMLAnchorElement.prototype.click = function () { window.saved = this.href; }; return true; })()"""


def export_subtitles(page):
    """The subtitles of the PNG and the SVG exports, each saved by a real click on its button: the
    PNG's the text filled at the subtitle's place, (16, 48) px; the SVG's its text element there."""
    page.run("window.texts = []")
    click_on(page, "#exportpng")
    png = [t for t, x, y in page.run("window.texts") if (x, y) == (16, 48)]
    page.run("window.svg = null")
    click_on(page, "#exportsvg")
    page.run("fetch(window.saved).then((r) => r.text()).then((t) => { window.svg = t; }), true")
    page.wait("window.svg !== null")
    return png, re.findall(r'<text x="16" y="48"[^>]*>(.*?)</text>', page.run("window.svg"))


def test_the_exports_subtitle_says_only_what_is_drawn(tmp_path):
    """The subtitle of the PNG and the SVG exports: road traffic's caption (the bin shown, what its
    ribbons' width and colour mean) while road traffic is drawn, and nothing once it is unticked, as
    each other layer's words are written only while it is drawn. The hand map, road traffic and
    cyclists ticked, coloured by volume, bin 0 (07:00–07:01). For bikes: with road traffic unticked
    the cyclists are still drawn, and the exports say what their lines mean in the cyclists' own
    legend ("Cyclists · width = cyclists"), not in the subtitle."""
    binary = chrome()
    if binary is None:
        pytest.skip("no Chrome or Chromium binary")
    from cdp import Page

    path = Path(hand_run(*hand_tables(), tmp_path / "run").map(tmp_path / "hand.html"))
    page = Page(binary, tmp_path / "profile", path.as_uri() + "#c=volume&ly=rb&b=0&z=750.0,0.0,5.000")
    try:
        page.wait("document.body && document.body.dataset.state === 'ready'")
        page.run(RECORD)
        caption = "07:00–07:01 · one ribbon per direction · width = road capacity, colour = volume"
        assert export_subtitles(page) == ([caption], [caption])
        click_on(page, "#layerrows input[data-layer=road]")
        assert export_subtitles(page) == ([""], [""])
    finally:
        page.close()
