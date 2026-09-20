"""Figures (S162, S163): the geometry that draws them, and that they draw.

The geometry is defended as properties: two directions of one street never
overlap, an offset is the distance asked for and on the right of travel,
direction marks point along travel. Figures are defended by what can be
checked without looking: they are made, they are not blank, and the same run
gives the same picture.
"""

import io

import numpy as np
import openmobisim as ms
import pytest
from openmobisim.viz import _geometry as geo

matplotlib = pytest.importorskip("matplotlib")

from openmobisim import viz  # noqa: E402  (after the importorskip on purpose)

CAR = {"commuter": (True, False, False)}


# --- geometry ------------------------------------------------------------------------


def two_way_street():
    """One street, west to east and back: link 0 eastbound, link 1 westbound."""
    points = np.array(
        [[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [100.0, 0.0], [50.0, 0.0], [0.0, 0.0]]
    )
    offsets = np.array([0, 3, 6], dtype=np.uint32)
    return points, offsets


def test_offset_is_to_the_right_of_travel_and_the_distance_asked_for():
    points, offsets = two_way_street()
    owner = geo.link_of_vertex(offsets)
    moved = geo.offset_points(points, owner, np.array([3.0, 3.0]))
    # Eastbound traffic drives on the south side, westbound on the north side.
    assert (moved[:3, 1] == pytest.approx(-3.0)) and (moved[3:, 1] == pytest.approx(3.0))
    assert np.hypot(*(moved - points).T) == pytest.approx(3.0)


def test_the_two_directions_of_a_street_never_overlap():
    points, offsets = two_way_street()
    owner = geo.link_of_vertex(offsets)
    half_widths = np.array([2.0, 5.0])
    moved = geo.offset_points(points, owner, half_widths)
    gap = moved[3:, 1].mean() - moved[:3, 1].mean()
    assert gap == pytest.approx(half_widths.sum())  # ribbons touch, never cross


def test_a_bend_keeps_its_offset_distance():
    points = np.array([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]])
    owner = np.zeros(3, dtype=np.int64)
    moved = geo.offset_points(points, owner, np.array([1.0]))
    normals = geo.right_normals(points, owner)
    assert np.hypot(*normals.T) == pytest.approx(1.0)
    assert np.hypot(*(moved - points).T) == pytest.approx(1.0)


def test_chevrons_point_along_travel():
    points, offsets = two_way_street()
    tri = geo.chevron_triangles(points, offsets, np.array([0, 1]), np.array([4.0, 4.0]))
    assert tri.shape == (2, 3, 2)
    assert tri[0, 0, 0] > tri[0, 1, 0]  # eastbound: the tip is east of the base
    assert tri[1, 0, 0] < tri[1, 1, 0]  # westbound: the tip is west of the base


def test_desire_lines_bend_right_and_the_reverse_bends_the_other_way():
    a, b = np.array([[0.0, 0.0]]), np.array([[100.0, 0.0]])
    forward = geo.bezier_curves(a, b)
    back = geo.bezier_curves(b, a)
    assert forward[0, 8, 1] < 0 < back[0, 8, 1]
    assert forward[0, 0] == pytest.approx(a[0]) and forward[0, -1] == pytest.approx(b[0])


# --- figures -------------------------------------------------------------------------


def toy_run(level=4, hours=1):
    net = ms.examples.toy_network()
    rows = []
    for origin, dest, count, headway in [
        ("W", "D1", 200, 4),
        ("N2", "D2", 200, 3),
        ("N1", "D1", 100, 5),
    ]:
        (olon, olat), (dlon, dlat) = net.node_lonlat(origin), net.node_lonlat(dest)
        rows += [
            (f"t{len(rows) + k}", 0, olon, olat, dlon, dlat, headway * k, "commuter", None)
            for k in range(count)
        ]
    scenario = ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=hours, flow_level=level, link_bin_s=300
    )
    return scenario.run("viz-test"), rows


def png_bytes(fig):
    buffer = io.BytesIO()
    fig.savefig(buffer, format="png", facecolor=fig.get_facecolor())
    return buffer.getvalue()


def pixels(fig):
    from matplotlib.backends.backend_agg import FigureCanvasAgg

    canvas = FigureCanvasAgg(fig)
    canvas.draw()
    return np.asarray(canvas.buffer_rgba())


@pytest.mark.parametrize("theme", ["paper", "night"])
def test_map_link_draws_a_figure_that_is_not_blank(theme):
    run, _ = toy_run()
    fig = viz.map_link(run, theme=theme, size=(8, 4.5), dpi=80)
    image = pixels(fig)
    assert image.shape == (360, 640, 4)
    assert len(np.unique(image.reshape(-1, 4), axis=0)) > 20, "a real picture has many colours"


def test_the_same_run_draws_the_same_picture():
    run, _ = toy_run()
    a = png_bytes(viz.map_link(run, size=(8, 4.5), dpi=80))
    b = png_bytes(viz.map_link(run, size=(8, 4.5), dpi=80))
    assert a == b


def test_map_link_takes_bins_views_and_a_path(tmp_path):
    run, _ = toy_run()
    out = tmp_path / "map.png"
    viz.map_link(
        run,
        bins=(0, 4),
        colour="volume",
        view=((4.79, 45.69), (4.82, 45.71)),
        size=(6, 3.5),
        dpi=60,
        path=str(out),
    )
    assert out.stat().st_size > 1000
    viz.map_link(run, bins=2, size=(6, 3.5), dpi=60, chevrons=False, scale=1.0, note="a note")


def test_map_link_says_what_is_missing_and_what_is_unknown():
    net = ms.examples.toy_network()
    (olon, olat), (dlon, dlat) = net.node_lonlat("W"), net.node_lonlat("D1")
    rows = [("a", 0, olon, olat, dlon, dlat, 0, "commuter", None)]
    bare = ms.Scenario.from_parts(net, rows, class_defaults=CAR).run("bare")
    with pytest.raises(ValueError, match="link_bin_s"):
        viz.map_link(bare)
    run, _ = toy_run()
    with pytest.raises(ValueError, match="colour"):
        viz.map_link(run, colour="loud")
    with pytest.raises(ValueError, match="theme"):
        viz.map_link(run, theme="neon")


def test_congestion_shows_only_where_vehicles_interact():
    from openmobisim.viz._link import link_table

    free, _ = toy_run(level=0)
    jammed, _ = toy_run(level=4)
    net = free.network
    ff = net.link_free_flow_s()
    f = link_table(free.link_bins(), net.link_count, ff, first=None, stop=None)
    j = link_table(jammed.link_bins(), net.link_count, ff, first=None, stop=None)
    assert f["delay"].max() < 1e-6, "with no interaction there is no delay"
    assert j["delay"].max() > 0.3, "an overloaded signal and merge delay traffic"
    assert (j["volume"] > 0).sum() >= (f["volume"] > 0).sum() - 3


def test_demand_figures_are_drawn(tmp_path):
    _, rows = toy_run()
    fig = viz.map_demand(rows, cell_m=100.0, size=(8, 4.5), dpi=60)
    assert len(np.unique(pixels(fig).reshape(-1, 4), axis=0)) > 20
    fig = viz.chart_demand_matrix(
        rows, cell_m=100.0, top=6, size=(8, 4.5), dpi=60, path=str(tmp_path / "matrix.png")
    )
    assert (tmp_path / "matrix.png").stat().st_size > 1000
    with pytest.raises(ValueError, match="no trips"):
        viz.map_demand([])


def test_a_figure_without_matplotlib_says_how_to_get_it(monkeypatch):
    import builtins

    real = builtins.__import__

    def refuse(name, *args, **kwargs):
        if name == "matplotlib" or name.startswith("matplotlib."):
            raise ImportError("no matplotlib here")
        return real(name, *args, **kwargs)

    from openmobisim.viz import _figure

    monkeypatch.setattr(builtins, "__import__", refuse)
    with pytest.raises(ImportError, match=r"openmobisim\[viz\]"):
        _figure.load_matplotlib()


# --- colour, visibility and furniture (S164) -----------------------------------------


def luminance(colour):
    """Relative luminance of an ``(r, g, b)`` triple in ``[0, 1]``."""
    r, g, b = (c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in colour)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


@pytest.mark.parametrize("theme", ["paper", "night"])
def test_more_delay_is_always_a_deeper_red(theme):
    from openmobisim.viz import _style
    from openmobisim.viz._figure import ramp_rgb

    th = _style.get_theme(theme)
    t = np.linspace(0.0, 1.0, 101)
    colours = ramp_rgb(th.ramp_delay, t)
    high = colours[-1]
    assert high[0] > 0.5 and high[0] > 2 * high[1] and high[0] > 2 * high[2], "the top is a red"
    assert colours[0][2] > colours[0][0], "the bottom is blue-green, not red"
    # From the gold in the middle up to the top the colour only gets darker.
    from_gold = np.array([luminance(c) for c in colours[34:]])
    assert (np.diff(from_gold) <= 1e-9).all(), "past the middle, more delay is darker"
    # The spectrum has the first version's ember ramp available for reversal.
    assert len(th.ramp_delay_ember) >= 4


@pytest.mark.parametrize("theme", ["paper", "night"])
def test_every_colour_of_the_spectrum_stands_out_from_its_surface(theme):
    from openmobisim.viz import _style
    from openmobisim.viz._figure import ramp_rgb, rgb

    th = _style.get_theme(theme)
    surface = luminance(rgb(th.surface))
    for colour in ramp_rgb(th.ramp_delay, np.linspace(0, 1, 21)):
        lum = luminance(colour)
        ratio = (max(lum, surface) + 0.05) / (min(lum, surface) + 0.05)
        assert ratio >= 2.0, f"{theme}: a ribbon colour is too close to the page ({ratio:.2f}:1)"


def test_a_link_with_almost_no_traffic_is_still_drawn():
    net = ms.examples.toy_network()
    (olon, olat), (dlon, dlat) = net.node_lonlat("W"), net.node_lonlat("D1")
    rows = [("only", 0, olon, olat, dlon, dlat, 0, "commuter", None)]
    run = ms.Scenario.from_parts(
        net, rows, class_defaults=CAR, window_hours=2, flow_level=4, link_bin_s=3600
    ).run("one-car")
    kwargs = {"size": (6, 3.4), "dpi": 60, "chevrons": False}
    shown = pixels(viz.map_link(run, **kwargs))
    hidden = pixels(viz.map_link(run, min_volume=1e9, **kwargs))
    assert (shown != hidden).any(), "one car an hour must still show as a line"


def test_volume_over_capacity_and_the_first_ramp_are_available():
    run, _ = toy_run()
    kwargs = {"size": (6, 3.4), "dpi": 60}
    vc = pixels(viz.map_link(run, colour="volume_capacity", **kwargs))
    delay = pixels(viz.map_link(run, colour="delay", **kwargs))
    ember = pixels(viz.map_link(run, ramp="ember", **kwargs))
    assert (vc != delay).any() and (ember != delay).any()
    with pytest.raises(ValueError, match="ramp"):
        viz.map_link(run, ramp="rainbow")


def test_the_logo_and_the_credit_are_optional_and_never_a_copyright_claim():
    run, _ = toy_run()
    kwargs = {"size": (8, 4.5), "dpi": 60}
    plain = viz.map_link(run, **kwargs)
    no_logo = viz.map_link(run, logo=False, **kwargs)
    credited = viz.map_link(run, credit="Ada Lovelace, Analytical Engines", **kwargs)
    words = {t.get_text() for t in plain.texts}
    assert "openmobisim" in words and not any("©" in w and "openmobisim" in w for w in words)
    assert "openmobisim" not in {t.get_text() for t in no_logo.texts}
    assert "Ada Lovelace, Analytical Engines" in {t.get_text() for t in credited.texts}
    assert "Ada Lovelace" not in " ".join(words)


def test_a_small_page_keeps_its_proportions():
    run, _ = toy_run()
    small = viz.map_link(run, size=(4.0, 2.4), dpi=100)
    image = pixels(small)
    assert image.shape == (240, 400, 4)
    assert len(np.unique(image.reshape(-1, 4), axis=0)) > 20
    sizes = {round(t.get_fontsize(), 1) for t in small.texts}
    assert max(sizes) < 10.0, "furniture text shrinks with the page (21 pt at 16 inches)"
