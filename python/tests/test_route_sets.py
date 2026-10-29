"""Route sets across the boundary.

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
    assert names[0] == "link_penalty" and {"shortest", "simulation"} <= set(names)


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
    single = ms.route_sets_build(net, pairs, method="shortest", options={"max_paths": 1})
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
    # Pinned to "none"/"deterministic" as this test's own fixture default (not the library's,
    # "msa"/"logit" since S187): an iterating run's route_method defaults to "shortest" (S179),
    # which would defeat the whole point of comparing "penalty" (the non-iterating default)
    # against an explicit "shortest".
    kwargs = {
        "classes": CAR,
        "window": (0, 3_600),
        "flow_level": 4,
        "link_bin_s": 300,
        "choice_model": "deterministic",
        "equilibration": "free_flow",
        # The plain single loading: everyone at once on free flow, the full model (S223).
        "equilibration_options": {"increments": 1, "warmup": 0},
    }
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
    fig = viz.map_route(rs, net, 0, theme=theme, size_cm=(20.32, 11.43), dpi=60, path=str(out))
    assert out.stat().st_size > 1000
    words = " ".join(t.get_text() for t in fig.texts)
    assert "Route alternatives" in words and "penalty" in words
    # By points as well as by position.
    a, b = pairs[0][:2], pairs[0][2:]
    viz.map_route(rs, net, (a, b), size_cm=(15.24, 8.64), dpi=60)


def test_map_route_draws_each_route_as_one_joined_line():
    # Shifted to the side link by link, a route's links parted at every turn and the route
    # showed gaps; drawn as one line, each route is one path that turns with its streets.
    from matplotlib.collections import LineCollection

    net = grid()
    rs = ms.route_sets_build(net, corner_pairs(net))
    fig = viz.map_route(rs, net, 0, size_cm=(15.24, 8.64), dpi=60)
    alternatives = rs.routes(0)
    route_lines = [
        c for c in fig.axes[0].collections if isinstance(c, LineCollection) and c.get_zorder() >= 2
    ]
    assert len(route_lines) == 2 * len(alternatives)  # each route's halo and line
    assert all(len(c.get_paths()) == 1 for c in route_lines)


def test_map_route_says_what_is_wrong():
    net = ms.examples.toy_network()
    (wlon, wlat), (dlon, dlat) = net.node_lonlat("W"), net.node_lonlat("D1")
    rs = ms.route_sets_build(net, [(dlon, dlat, wlon, wlat), (wlon, wlat, dlon, dlat)])
    # The streets are one-way: one direction has a route, the other does not.
    have = [k for k in range(rs.key_count) if len(rs.routes(k))]
    lack = [k for k in range(rs.key_count) if not len(rs.routes(k))]
    assert len(have) == 1 and len(lack) == 1
    viz.map_route(rs, net, have[0], size_cm=(15.24, 8.64), dpi=60)
    with pytest.raises(ValueError, match="no route"):
        viz.map_route(rs, net, lack[0], size_cm=(15.24, 8.64), dpi=60)
    with pytest.raises(ValueError, match="out of range"):
        viz.map_route(rs, net, 99)


# --- the congestion-biased Monte Carlo method (S176) ---------------------------------------------


def test_montecarlo_is_biased_by_the_demand_it_is_generated_for():
    net = ms.examples.manhattan_grid(n=8, block_metres=200.0, signals=False)
    rows = ms.examples.trips_random(net, 300, seed=5, min_m=300.0, max_m=1400.0, spread_s=300)

    def sets(weight, method="montecarlo", options=None):
        run = ms.Scenario.from_parts(
            net,
            rows,
            classes={"commuter": (True, False, False)},
            window=(0, 7_200),
            flow_level=4,
            default_weight=weight,
            route_method=method,
            route_options=options,
            # The sets as generated: nothing added by an update (the free-flow loading's groups
            # would search, S223).
            route_update="none",
        ).run(f"mc-{method}-{weight}")
        return run.route_sets(), run

    (light, light_run), (heavy, _), (shortest, shortest_run) = (
        sets(1),
        sets(1000),
        sets(1, "shortest", {"max_paths": 1}),
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


# --- the sampled method: generalised cost, preference draws and attribute noise (S298) ----------

SAMPLED_OPTIONS = ("draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma", "tau")


def one_loading(net, rows, name, **kwargs):
    # One loading of everyone at once, the sets as the method made them (no route update).
    settings = {
        "classes": CAR,
        "window": (0, 3_600),
        "flow_level": 4,
        "choice_model": "logit",
        "equilibration": "free_flow",
        "equilibration_options": {"increments": 1, "warmup": 0},
        "route_update": "none",
        "master_seed": 3,
    }
    settings.update(kwargs)
    return ms.Scenario.from_parts(net, rows, **settings).run(name, overwrite=True)


def test_sampled_is_a_method_and_an_update_that_can_be_selected():
    assert "doubly_stochastic" in ms.route_methods()
    assert "doubly_stochastic" in ms.route_update_methods()


def test_a_sampled_run_routes_and_names_the_method_with_every_option():
    net = grid()
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=300.0, max_m=900.0)
    run = one_loading(net, rows, "sampled-run", route_method="sampled")
    shortest = one_loading(net, rows, "sampled-short", route_method="shortest")
    assert run.completion == shortest.completion
    sets = run.route_sets()
    assert sets.method == "sampled" and sets.descriptor.startswith("sampled;")
    descriptor = run.manifest()["route_descriptor"]
    assert descriptor.startswith("sampled;")
    for name in SAMPLED_OPTIONS:
        assert f"{name}=" in descriptor, name
    assert "draws=4;" in descriptor and "seed=0;" in descriptor, "the defaults are filled in"
    # The manifest names the method as the sets were made: the extras included (the logit's
    # coefficients on money and mixed-traffic kilometres give every class extras).
    assert descriptor == sets.descriptor, (descriptor, sets.descriptor)
    assert "extras=given:" in descriptor
    # Its draws find alternatives the shortest method does not hold.
    assert sets.route_count > shortest.route_sets().route_count
    assert run.fingerprint != shortest.fingerprint


def test_sampled_without_draws_is_the_shortest_route_of_each_pair():
    net = grid()
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=300.0, max_m=900.0)
    none = one_loading(net, rows, "sampled-0", route_method="sampled", route_options={"draws": 0})
    shortest = one_loading(
        net, rows, "sampled-0-short", route_method="shortest", route_options={"max_paths": 1}
    )
    a, b = none.route_sets(), shortest.route_sets()
    assert "draws=0;" in a.descriptor
    assert a.key_count == b.key_count and a.route_count == b.route_count == a.key_count
    assert (a.set_offsets() == b.set_offsets()).all()
    assert (a.route_offsets() == b.route_offsets()).all() and (a.links() == b.links()).all()


def test_sampled_options_are_refused_with_the_known_ones():
    net = grid()
    rows = ms.examples.trips_random(net, 20, seed=5, min_m=300.0, max_m=900.0)
    known = '"draws", "max_detour", "max_overlap", "max_paths", "seed", "sigma", "tau"'
    with pytest.raises(ValueError, match=known):
        one_loading(net, rows, "sampled-bad", route_method="sampled", route_options={"draw": 1})
    for name, value in [("draws", 65), ("max_paths", 0), ("sigma", -1), ("max_detour", 0.5)]:
        with pytest.raises(ValueError, match=name):
            one_loading(
                net, rows, "sampled-bad", route_method="sampled", route_options={name: value}
            )


# --- Run.route_set_summary (S298) -------------------------------------------------------------


def test_the_route_set_summary_counts_what_the_sets_and_choices_hold():
    net = grid()
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=300.0, max_m=900.0)
    run = one_loading(net, rows, "summary-sampled", route_method="sampled")
    s = run.route_set_summary()
    assert s is not None and all(type(v) is float for v in s.values())
    assert set(s) == {
        "pairs",
        "routes",
        "routes_per_pair_mean",
        "routes_per_pair_max",
        "routes_added",
        "route_searches",
        "share_on_best",
        "share_routes_unchosen",
        "alternatives_mean",
        "alternatives_p95",
    }
    # The counts, by hand from the sets themselves.
    sets = run.route_sets()
    sizes = [len(sets.routes(k)) for k in range(sets.key_count)]
    assert s["pairs"] == len(sizes) == len(set(zip(*sets.keys(), strict=True)))
    assert s["routes"] == sum(sizes)
    assert s["routes_per_pair_max"] == max(sizes) > 1
    assert s["routes_per_pair_mean"] == pytest.approx(sum(sizes) / len(sizes))
    assert s["routes_added"] == 0 and s["route_searches"] == 0, "no route update"
    rc = run.route_choices()
    routed = rc.route >= 0
    assert 0 <= s["share_on_best"] <= 1
    assert s["share_on_best"] == pytest.approx(
        rc.weight[routed & (rc.rank == 0)].sum() / rc.weight[routed].sum()
    )
    taken = {int(r) for r in rc.route[routed]}
    assert s["share_routes_unchosen"] == pytest.approx(1 - len(taken) / sum(sizes))
    # Each trip is offered its pair's whole set.
    offered = np.array([sizes[int(k)] for k in rc.pair[routed]], dtype=float)
    assert s["alternatives_mean"] == pytest.approx(offered.mean()) and s["alternatives_mean"] >= 1
    assert s["alternatives_p95"] == pytest.approx(np.percentile(offered, 95))
    # Rank 0 is the pair's free-flow shortest route: the route the shortest method makes for it.
    shortest = one_loading(net, rows, "summary-short-ref", route_method="shortest").route_sets()
    origins, destinations = sets.keys()
    for k in range(sets.key_count):
        same = shortest.find(int(origins[k]), int(destinations[k]))
        assert (sets.routes(k)[0] == shortest.routes(same)[0]).all()


def test_the_route_set_summary_of_one_route_per_pair():
    net = grid()
    rows = ms.examples.trips_random(net, 60, seed=5, min_m=300.0, max_m=900.0)
    run = one_loading(
        net, rows, "summary-short", route_method="shortest", route_options={"max_paths": 1},
        route_update="none",
    )  # fmt: skip
    s = run.route_set_summary()
    assert s["routes_per_pair_max"] == 1 and s["routes_per_pair_mean"] == 1
    assert s["share_on_best"] == 1 and s["share_routes_unchosen"] == 0
    assert s["alternatives_mean"] == 1 and s["alternatives_p95"] == 1
    assert s["pairs"] == s["routes"] == run.route_sets().key_count


def test_equal_routes_are_split_by_the_logit_at_the_first_loading():
    # S302: a pair with two routes of equal free-flow time (the 2 × 2 grid's opposite corners)
    # holds both from the start, so the logit, not the order of the links, splits the tie: at
    # the first loading the share on each is a half, within three standard deviations of a
    # binomial share over the trips. With the route alone (max_paths 1) all take one route.
    net = grid(2)
    n = 400
    trips = [(f"t{i}", (0, 0), (1, 1), 0.25 * i / n, 1) for i in range(n)]
    rows = ms.examples.fixed_car_trips(net, trips)
    run = one_loading(net, rows, "twins-split", route_method="shortest", flow_level=2)
    sets = run.route_sets()
    assert sets.key_count == 1 and sets.route_count == 2
    assert "max_paths=2" in sets.descriptor and "twin_tolerance=0.001" in sets.descriptor
    costs = sets.costs()
    assert abs(costs[0] - costs[1]) < 1e-3, "the two routes take the same free-flow time"
    rc = run.route_choices()
    assert (rc.route >= 0).all() and (rc.alternatives == 2).all()
    share = float((rc.rank == 0).mean())
    sigma = (0.25 / n) ** 0.5
    assert abs(share - 0.5) <= 3 * sigma, f"share on the first route {share:.3f}"
    one = one_loading(
        net, rows, "twins-one", route_method="shortest", route_options={"max_paths": 1},
        flow_level=2,
    )  # fmt: skip
    assert one.route_sets().route_count == 1 and one.route_sets().descriptor == "shortest"
    assert (one.route_choices().rank == 0).all()
    # An iterating run starts from the same method by default.
    default = one_loading(
        net, rows, "twins-default", route_method=None, route_update=None, flow_level=2,
        equilibration="msa", equilibration_options={"iterations": 2, "warmup": 0},
    )  # fmt: skip
    assert default.route_sets().method == "shortest"
    assert len(default.route_sets().routes(0)) >= 2, "the route and its twin from the start"
