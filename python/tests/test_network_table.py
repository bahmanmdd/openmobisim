"""``network_read_table`` (I-y): a network from a plain link table.

What is defended: a column is read under any of its documented aliases; a
column not given defaults from the class table exactly as an unstated OSM tag
does (I-y's own promise); explicit units convert correctly; an unrecognised
`class` value falls back to unclassified with one aggregate warning, not
silence and not a crash; a node table's `x`/`y` and a coordinate-free table's
placeholder layout are both deterministic; the TNTP text dialect (a metadata
block, a ``~``-prefixed header, ``;``-terminated rows) parses the same way a
plain CSV does; and the round trip promised by I-y
holds: a table's links and their attributes come back out of the network
unchanged.
"""

from __future__ import annotations

import numpy as np
import openmobisim as ms
import pytest
from openmobisim.network import _as_bool, _as_float, _parse_delimited_text


def test_from_to_and_their_aliases_are_required_and_recognised():
    rows = [{"init_node": "a", "term_node": "b", "length": 100}]
    net = ms.network_read_table(rows, [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 1, "y": 0}])
    assert net.link_count == 1 and net.node_count == 2

    with pytest.raises(ValueError, match="from/to"):
        ms.network_read_table([{"length": 100}])


def test_an_absent_optional_column_defaults_from_the_class_table():
    # No lanes, capacity or speed given at all: every number must be the
    # residential row's own, exactly as an OSM way with no tags gets it.
    net = ms.network_read_table(
        [{"from": "a", "to": "b", "class": "residential"}],
        [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 1, "y": 0}],
    )
    assert net.link_capacity_pcu_h()[0] == pytest.approx(1400.0)
    assert net.link_lanes()[0] == 1


def test_a_given_capacity_overrides_the_class_row_and_lanes_still_scale_it_otherwise():
    rows = [
        {"from": "a", "to": "b", "class": "residential", "capacity": 999},
        {"from": "b", "to": "c", "class": "residential", "lanes": 2},
    ]
    nodes = [{"id": n, "x": i, "y": 0} for i, n in enumerate("abc")]
    net = ms.network_read_table(rows, nodes)
    cap = net.link_capacity_pcu_h()
    assert cap[0] == pytest.approx(999.0), "an explicit capacity is used as given"
    assert cap[1] == pytest.approx(2 * 1400.0), "no explicit capacity: lanes still scale the row"


def test_an_unrecognised_class_falls_back_and_warns_once_in_aggregate():
    rows = [
        {"from": "a", "to": "b", "class": "3"},
        {"from": "b", "to": "c", "class": "3"},
        {"from": "c", "to": "a"},  # no class at all: defaults quietly, not counted below
    ]
    nodes = [{"id": n, "x": i, "y": 0} for i, n in enumerate("abc")]
    with pytest.warns(UserWarning, match="2 link"):
        net = ms.network_read_table(rows, nodes)
    # RoadClass::Unclassified's index (core-graph/src/defaults.rs, RoadClass::ALL).
    assert (net.link_class() == 10).all()


def test_a_capacity_too_high_for_the_class_rows_jam_density_is_reduced_and_warned_about():
    # A single unclassified lane's jam density (140 veh/km) cannot carry
    # 20000 veh/h at 100 km/h; the core resolves it (design §12.1) and this
    # must say so, since the number a caller gets back is not the one asked
    # for — found on a real Sioux Falls link, kept as a regression test.
    rows = [
        {"from": "a", "to": "b", "length": 1000, "free_flow_speed": 100, "capacity": 20_000},
        {"from": "b", "to": "c", "length": 1000, "free_flow_speed": 30, "capacity": 400},
    ]
    nodes = [{"id": n, "x": i, "y": 0} for i, n in enumerate("abc")]
    with pytest.warns(UserWarning, match="1 link"):
        net = ms.network_read_table(rows, nodes)
    cap = net.link_capacity_pcu_h()
    assert cap[0] < 20_000, "the impossible request was reduced, not honoured silently"
    assert cap[1] == pytest.approx(400.0), "a request the diagram can carry is untouched"


def test_free_flow_time_is_converted_to_a_speed_using_length_and_the_stated_units():
    # 1 mile in 1 minute is 60 mph = 96.56 km/h.
    net = ms.network_read_table(
        [{"from": "a", "to": "b", "length": 1, "free_flow_time": 1}],
        [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 1, "y": 0}],
        length_unit="mi",
        time_unit="min",
    )
    assert net.link_free_flow_s()[0] == pytest.approx(60.0, abs=0.5)


def test_a_given_free_flow_speed_is_read_in_its_stated_unit_and_speed_never_aliases_to_it():
    # The standard TNTP "speed" column is not free-flow speed (it collided
    # with it once, silently zeroing every diagram — kept as a regression
    # test): only "free_flow_speed"/"free_flow_speed_km_h" may set it.
    net = ms.network_read_table(
        [{"from": "a", "to": "b", "length": 1000, "speed": 0, "free_flow_speed_km_h": 36}],
        [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 1, "y": 0}],
    )
    assert net.link_free_flow_s()[0] == pytest.approx(100.0, abs=0.1)  # 1000 m at 36 km/h = 100 s


def test_speed_unit_mi_h_converts():
    net = ms.network_read_table(
        [{"from": "a", "to": "b", "length": 1609.344, "free_flow_speed": 60}],
        [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 1, "y": 0}],
        speed_unit="mi_h",
    )
    assert net.link_free_flow_s()[0] == pytest.approx(60.0, abs=0.5)  # 1 mile at 60 mph = 1 min


def test_bad_units_are_refused_before_any_row_is_read():
    with pytest.raises(ValueError, match="length_unit"):
        ms.network_read_table([{"from": "a", "to": "b"}], length_unit="furlongs")
    with pytest.raises(ValueError, match="coordinates"):
        ms.network_read_table([{"from": "a", "to": "b"}], coordinates="mercator")


def test_xy_nodes_place_deterministically_and_scale_with_coordinate_scale_m():
    links = [{"from": "a", "to": "b"}]
    nodes = [{"id": "a", "x": 0, "y": 0}, {"id": "b", "x": 10, "y": 0}]
    close = ms.network_read_table(links, nodes, coordinates="xy", coordinate_scale_m=1.0)
    far = ms.network_read_table(links, nodes, coordinates="xy", coordinate_scale_m=100.0)
    same_again = ms.network_read_table(links, nodes, coordinates="xy", coordinate_scale_m=1.0)
    d_close = _lonlat_distance_m(close.node_lonlat("a"), close.node_lonlat("b"))
    d_far = _lonlat_distance_m(far.node_lonlat("a"), far.node_lonlat("b"))
    assert d_far == pytest.approx(100 * d_close, rel=1e-6)
    assert close.node_lonlat("a") == same_again.node_lonlat("a"), "the layout is deterministic"


def test_coordinate_free_tables_get_a_deterministic_placeholder_layout():
    links = [{"from": "a", "to": "b"}, {"from": "b", "to": "c"}]
    net1 = ms.network_read_table(links)
    net2 = ms.network_read_table(links)
    assert net1.node_count == 3
    assert net1.node_lonlat("a") == net2.node_lonlat("a")
    assert net1.node_lonlat("a") != net1.node_lonlat("b"), "distinct placeholders per node"


def test_a_tntp_style_file_parses_the_same_as_a_plain_csv(tmp_path):
    # A metadata block, a "~"-prefixed, tab-separated header (whose leading
    # tab the tilde stands in for) and ";"-terminated rows, exactly as the
    # standard TransportationNetworks benchmark files are shaped.
    tntp = tmp_path / "mini_net.tntp"
    tntp.write_text(
        "<NUMBER OF ZONES> 3\n<NUMBER OF LINKS> 2\n<END OF METADATA>\n\n"
        "~ \tinit_node\tterm_node\tcapacity\tlength\tfree_flow_time\tspeed\tlink_type\t;\n"
        "\t\t1\t2\t1200\t0.2\t0.4\t0\t1\t;\n"
        "\t\t2\t3\t1400\t0.15\t0.3\t0\t2\t;\n"
    )
    net = ms.network_read_table(str(tntp), length_unit="mi", time_unit="min")
    assert net.link_count == 2 and net.node_count == 3
    assert net.link_length_m()[0] == pytest.approx(0.2 * 1609.344)
    assert net.link_free_flow_s()[0] == pytest.approx(24.0)

    csv_path = tmp_path / "mini_net.csv"
    csv_path.write_text(
        "from,to,capacity,length,free_flow_time\n1,2,1200,0.2,0.4\n2,3,1400,0.15,0.3\n"
    )
    same = ms.network_read_table(str(csv_path), length_unit="mi", time_unit="min")
    np.testing.assert_allclose(same.link_length_m(), net.link_length_m())
    np.testing.assert_allclose(same.link_free_flow_s(), net.link_free_flow_s())


def test_the_round_trip_the_roadmap_asks_for_a_table_in_gives_the_same_links_out():
    # Zero-padded, already-sorted ids: a network's internal order is by
    # sorted external id (Foundations §1), so giving ids that already sort
    # in table order lets this compare index i to row i directly, with no
    # id-to-index accessor needed.
    node_ids = ["n0", "n1", "n2", "n3"]
    nodes = [{"id": nid, "x": i, "y": 0} for i, nid in enumerate(node_ids)]
    rows = [
        {"id": "l0", "from": "n0", "to": "n1", "length": 100, "class": "primary", "capacity": 900},
        {"id": "l1", "from": "n1", "to": "n2", "length": 200, "class": "residential", "lanes": 2},
        {"id": "l2", "from": "n2", "to": "n3", "length": 50, "class": "service"},
    ]
    net = ms.network_read_table(rows, nodes)
    assert list(net.link_from()) == [0, 1, 2]
    assert list(net.link_to()) == [1, 2, 3]
    np.testing.assert_allclose(net.link_length_m(), [100.0, 200.0, 50.0])
    assert net.link_capacity_pcu_h()[0] == pytest.approx(900.0)
    assert net.link_capacity_pcu_h()[1] == pytest.approx(2 * 1400.0)
    # Row 0 (primary, no lanes given) defaults to the class row's own 2;
    # row 1 states 2 explicitly; row 2 (service, no lanes) defaults to 1.
    assert list(net.link_lanes()) == [2, 2, 1]


def _lonlat_distance_m(a: tuple[float, float], b: tuple[float, float]) -> float:
    lat = (a[1] + b[1]) / 2
    dx = (b[0] - a[0]) * 111_320.0 * np.cos(np.radians(lat))
    dy = (b[1] - a[1]) * 110_574.0
    return float(np.hypot(dx, dy))


# --- the small parsing helpers, tested directly -----------------------------------


def test_parse_delimited_text_handles_the_tntp_leading_tab_offset():
    text = "~ \ta\tb\t;\n\t\t1\t2\t;\n"
    assert _parse_delimited_text(text) == [{"a": "1", "b": "2"}]


def test_parse_delimited_text_skips_metadata_and_blank_lines():
    text = "<NUMBER OF ZONES> 5\n\na,b\n1,2\n"
    assert _parse_delimited_text(text) == [{"a": "1", "b": "2"}]


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("1", True),
        ("true", True),
        ("yes", True),
        ("0", False),
        ("no", False),
        ("", False),
        (None, False),
    ],
)
def test_as_bool_recognises_common_spellings(value, expected):
    assert _as_bool(value) is expected


@pytest.mark.parametrize(
    ("value", "expected"),
    [("1.5", 1.5), ("", None), (None, None), ("nan", None), ("abc", None)],
)
def test_as_float_rejects_blanks_and_non_finite_values(value, expected):
    assert _as_float(value) == expected
