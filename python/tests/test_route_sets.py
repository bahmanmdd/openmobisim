"""Route sets across the boundary (S165).

What is defended: the store's arrays are consistent (offsets, ranges, sorted
costs, the inverted index), methods are selected by name and bad choices say
what is wrong, the choice of method does not change a run while only the best
route is used, and one pair's alternatives can be drawn.
"""

import numpy as np
import openmobisim as ms
import pytest

pytest.importorskip("matplotlib")

from openmobisim import viz  # noqa: E402

CAR = {"commuter": (True, False, False)}


def grid(n=6):
    return ms.examples.manhattan_grid(n=n, block_metres=200.0, signals=False)


def corner_pairs(net, n=6):
    at = lambda r, c: ms._core.grid_node_lonlat(net, r, c)  # noqa: E731
    pairs = [
        (*at(0, 0), *at(n - 1, n - 1)),
        (*at(0, n - 1), *at(n - 1, 0)),
        (*at(1, 0), *at(n - 2, n - 1)),
        (*at(2, 2), *at(3, 3)),
    ]
    return pairs


def test_the_default_method_is_listed_first_and_others_can_be_selected():
    names = ms.route_methods()
    assert names[0] == "penalty" and {"shortest", "montecarlo"} <= set(names)


def test_the_store_is_a_consistent_set_of_flat_arrays():
    net = grid()
    rs = ms.route_sets_build(net, corner_pairs(net))
    origin, destination = rs.keys()
    assert rs.key_count == len(origin) == len(destination) == 4
    set_off, route_off, links = rs.set_offsets(), rs.route_offsets(), rs.links()
    assert set_off[0] == 0 and set_off[-1] == rs.route_count == len(rs.costs()) == len(
        rs.overlaps()
    )
    assert route_off[0] == 0 and route_off[-1] == len(links)
    assert (np.diff(set_off) >= 0).all() and (np.diff(route_off) > 0).all()
    assert links.max() < net.link_count
    assert list(zip(origin, destination, strict=True)) == sorted(
        zip(origin, destination, strict=True)
    )
    for k in range(rs.key_count):
        routes = rs.routes(k)
        first = int(set_off[k])
        assert len(routes) == set_off[k + 1] - set_off[k]
        costs = rs.costs()[first : first + len(routes)]
        assert (np.diff(costs) >= 0).all(), "best first"
        for r, route in enumerate(routes):
            a, b = route_off[first + r], route_off[first + r + 1]
            assert (route == links[a:b]).all()
            assert rs.route_key(first + r) == k
        assert rs.find(int(origin[k]), int(destination[k])) == k
    assert rs.find(10**6, 0) is None


def test_the_inverted_index_lists_exactly_the_routes_that_use_a_link():
    net = grid()
    rs = ms.route_sets_build(net, corner_pairs(net))
    offsets, links = rs.route_offsets(), rs.links()
    for link in range(net.link_count):
        expected = [r for r in range(rs.route_count) if link in links[offsets[r] : offsets[r + 1]]]
        assert rs.link_routes(link).tolist() == expected
    with pytest.raises(ValueError, match="no link"):
        rs.link_routes(net.link_count)


def test_a_grid_offers_alternatives_within_the_limits():
    net = grid()
    rs = ms.route_sets_build(net, corner_pairs(net), options={"max_paths": 4, "max_detour": 1.2})
    counts = np.diff(rs.set_offsets())
    assert counts.max() > 1 and counts.max() <= 4
    for k in range(rs.key_count):
        costs = rs.costs()[int(rs.set_offsets()[k]) : int(rs.set_offsets()[k + 1])]
        assert costs.max() <= 1.2 * costs.min() * (1 + 1e-6)
    assert (rs.overlaps() <= 0.75 + 1e-6).all()


def test_sets_are_repeatable_and_know_their_settings():
    net = grid()
    pairs = corner_pairs(net)
    a, b = ms.route_sets_build(net, pairs), ms.route_sets_build(net, pairs)
    assert a.identity == b.identity and (a.links() == b.links()).all()
    other = ms.route_sets_build(net, pairs, options={"max_paths": 3})
    shortest = ms.route_sets_build(net, pairs, method="shortest")
    assert len({a.identity, other.identity, shortest.identity}) == 3
    assert a.method == "penalty" and shortest.method == "shortest"
    assert "max_paths=5" in a.descriptor and "max_paths=3" in other.descriptor


def test_the_shortest_method_gives_the_penalty_methods_first_route():
    net = grid()
    pairs = corner_pairs(net)
    full = ms.route_sets_build(net, pairs)
    single = ms.route_sets_build(net, pairs, method="shortest")
    assert (np.diff(single.set_offsets()) == 1).all()
    for k in range(full.key_count):
        assert (single.routes(k)[0] == full.routes(k)[0]).all()


def test_bad_method_and_bad_options_say_what_is_wrong():
    net = grid()
    pairs = corner_pairs(net)
    with pytest.raises(ValueError, match="teleport") as e:
        ms.route_sets_build(net, pairs, method="teleport")
    assert "penalty" in str(e.value)
    with pytest.raises(ValueError, match="max_paths"):
        ms.route_sets_build(net, pairs, options={"max_paths": 0})
    with pytest.raises(ValueError, match="colour"):
        ms.route_sets_build(net, pairs, options={"colour": 1})
    with pytest.raises(ValueError, match="route_method|teleport"):
        ms.Scenario.from_parts(net, [], route_method="teleport").run("bad")


def test_a_run_routes_from_its_sets_and_the_method_does_not_change_the_run():
    net = grid()
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=300.0, max_m=900.0)
    kwargs = {"class_defaults": CAR, "window_hours": 1, "flow_level": 4, "link_bin_s": 300}
    penalty = ms.Scenario.from_parts(net, rows, **kwargs).run("penalty-run")
    shortest = ms.Scenario.from_parts(net, rows, route_method="shortest", **kwargs).run("short-run")
    assert penalty.route_sets().method == "penalty" and shortest.route_sets().method == "shortest"
    assert penalty.route_sets().route_count >= shortest.route_sets().route_count
    assert penalty.completion == shortest.completion
    assert penalty.total_travel_time_s == shortest.total_travel_time_s
    keys = penalty.route_sets().key_count
    assert 0 < keys <= 60


def test_a_point_snaps_to_the_nearest_node():
    net = grid(3)
    at = lambda r, c: ms._core.grid_node_lonlat(net, r, c)  # noqa: E731
    nodes = {net.node_nearest(*at(r, c)) for r in range(3) for c in range(3)}
    assert len(nodes) == 9, "every grid node is its own nearest node"
    lon, lat = at(1, 1)
    assert net.node_nearest(lon + 1e-6, lat - 1e-6) == net.node_nearest(lon, lat)


# --- the figure ----------------------------------------------------------------------


@pytest.mark.parametrize("theme", ["paper", "night"])
def test_map_route_draws_the_alternatives_of_a_pair(theme, tmp_path):
    net = grid()
    pairs = corner_pairs(net)
    rs = ms.route_sets_build(net, pairs)
    out = tmp_path / "route.png"
    fig = viz.map_route(rs, net, 0, theme=theme, size=(8, 4.5), dpi=60, path=str(out))
    assert out.stat().st_size > 1000
    words = " ".join(t.get_text() for t in fig.texts)
    assert "Route alternatives" in words and "penalty" in words
    # By points as well as by position.
    a, b = pairs[0][:2], pairs[0][2:]
    viz.map_route(rs, net, (a, b), size=(6, 3.4), dpi=60)


def test_map_route_says_what_is_wrong():
    net = ms.examples.toy_network()
    (wlon, wlat), (dlon, dlat) = net.node_lonlat("W"), net.node_lonlat("D1")
    rs = ms.route_sets_build(net, [(dlon, dlat, wlon, wlat), (wlon, wlat, dlon, dlat)])
    # The streets are one-way: one direction has a route, the other does not.
    have = [k for k in range(rs.key_count) if len(rs.routes(k))]
    lack = [k for k in range(rs.key_count) if not len(rs.routes(k))]
    assert len(have) == 1 and len(lack) == 1
    viz.map_route(rs, net, have[0], size=(6, 3.4), dpi=60)
    with pytest.raises(ValueError, match="no route"):
        viz.map_route(rs, net, lack[0], size=(6, 3.4), dpi=60)
    with pytest.raises(ValueError, match="out of range"):
        viz.map_route(rs, net, 99)


# --- the congestion-biased Monte Carlo method (S176) ---------------------------------------------


def test_montecarlo_is_biased_by_the_demand_it_is_generated_for():
    net = ms.examples.manhattan_grid(n=8, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 300, seed=5, min_m=300.0, max_m=1400.0, spread_s=300)

    def sets(weight, method="montecarlo"):
        run = ms.Scenario.from_parts(
            net,
            rows,
            class_defaults={"commuter": (True, False, False)},
            window_hours=2,
            flow_level=4,
            default_weight=weight,
            route_method=method,
        ).run(f"mc-{method}-{weight}")
        return run.route_sets(), run

    (light, light_run), (heavy, _), (shortest, shortest_run) = (
        sets(1),
        sets(1000),
        sets(1, "shortest"),
    )
    assert light.method == "montecarlo" and "bias=demand" in light.descriptor
    assert light_run.fingerprint != shortest_run.fingerprint
    # Every pair's first route is a shortest one.
    first = light.set_offsets()[:-1]
    assert np.allclose(light.costs()[first], shortest.costs(), atol=1e-3)
    # The people the trips stand for decide how much noise the links get: on the same trips, more
    # people put more of the network near capacity, so more alternatives are worth finding.
    assert heavy.route_count > light.route_count > shortest.route_count


def test_montecarlo_sets_can_be_built_for_pairs_and_follow_their_options():
    net = grid(8)
    pairs = corner_pairs(net, 8)
    base = ms.route_sets_build(net, pairs, method="montecarlo")
    assert base.method == "montecarlo" and base.key_count == 4
    again = ms.route_sets_build(net, pairs, method="montecarlo")
    assert (base.links() == again.links()).all() and base.identity == again.identity
    # The seed is another set of draws, and part of the set's identity.
    other = ms.route_sets_build(net, pairs, method="montecarlo", options={"seed": 2})
    assert other.identity != base.identity
    # Without the bias, noise on every link finds many more routes.
    unbiased = ms.route_sets_build(net, pairs, method="montecarlo", options={"biased": 0})
    assert "bias=none" in unbiased.descriptor and unbiased.route_count > base.route_count


def test_montecarlo_options_are_refused_before_any_work():
    net = grid()
    pairs = corner_pairs(net)
    with pytest.raises(ValueError, match='"biased", "draws", "max_detour", "max_overlap"'):
        ms.route_sets_build(net, pairs, method="montecarlo", options={"nope": 1})
    for name, value in [("draws", 1001), ("max_paths", 0), ("sigma", 0), ("max_detour", 0.5)]:
        with pytest.raises(ValueError, match=name):
            ms.route_sets_build(net, pairs, method="montecarlo", options={name: value})
