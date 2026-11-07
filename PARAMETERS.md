# Parameters of a run

Every parameter of `run(network, demand, transit=None, **parameters)` is a row here: at most 40, each with a unit, a
default, a source and a test that reads it (`tools/check_limits.sh` counts the rows and looks for each name in the
tests). A parameter that is not in this table does not exist.

| Parameter | Unit | Default | Source | Test that reads it |
|---|---|---|---|---|
| capacity_per_lane | veh/h per lane | 1 800 | Transportation Research Board, *Highway Capacity Manual*, 7th edition (2022), base saturation flow per lane | `crates/graph/tests/network.rs`: `two_lanes_at_1800_veh_h_per_lane_pass_one_vehicle_per_second` |
| bike_speed | km/h | 15 | de Hartog, Boogaard, Nijland and Hoek, *Environmental Health Perspectives* (2011), urban cycling speed in the Netherlands with stops included | `crates/graph/tests/network.rs`: `a_bike_rides_1500_m_at_15_km_h_in_360_s` |
| walk_speed | km/h | 4.8 | Knoblauch, Pietrucha and Nitzburg, *Transportation Research Record* 1538 (1996), measured mean walking speeds | `crates/graph/tests/network.rs`: `a_walker_covers_1000_m_at_4_8_km_h_in_750_s` |
| stop_walk_max | m | 1 500 | El-Geneidy, Grimsrud, Wasfi, Tétreault and Surprenant-Legault, *Transportation* 41 (2014) 193–210: the 85th percentile of the walk to a stop is 524 m for buses and 1 259 m for commuter rail | `crates/demand/tests/by_hand.rs`: `at_stop_walk_max_1500_m_a_walk_of_1500_m_is_in_and_one_of_1501_m_out` |
| boarding_slack | s | 60 | GTFS `transfers.txt`, `min_transfer_time` in common feeds; Delling, Pajor and Werneck, *Transportation Science* 49 (2015), the minimum change time | `crates/loop/tests/by_hand.rs`: `with_a_boarding_slack_of_600_s_the_run_at_600_s_cannot_be_boarded_from_a_stop_reached_at_23_s` |
| max_rides | rides | 4 | Dutch journey planners cap at three transfers; RAPTOR rounds beyond four rarely improve the arrival (Delling, Pajor and Werneck 2015) | `crates/loop/tests/by_hand.rs`: `max_rides_1_refuses_a_two_ride_journey_that_max_rides_2_takes` |
| choice | name | logit (nests by mode) | the specification: the benchmarks need the deterministic model and the logit family; a run with mode choice needs the logit family | `crates/loop/tests/determinism.rs` (logit), `crates/loop/tests/common/mod.rs` (deterministic) |
| coefficients | cost per s, per €, per unit of ln PS | 1/600, 0.6, −1 (one class) | a time scale of 0.1 per minute, the order of the route-choice estimates of Ben-Akiva and Bierlaire (1999); a value of time of 10 €/h (KiM, *Values of time and reliability in passenger transport*, 2023); −1 on ln PS by the path-size logit's definition | `crates/loop/tests/determinism.rs`: `the_same_tables_at_1_and_8_threads_and_over_two_runs` |
| nest_scale | none | 0.5 | Ben-Akiva and Lerman, *Discrete Choice Analysis* (1985), chapter 10: a mid-range nest scale | `crates/loop/tests/determinism.rs`: `the_same_tables_at_1_and_8_threads_and_over_two_runs` |
| iteration_cap | iterations | 50 | below the 60-iteration run test 10 compares the stopped state against | `crates/loop/tests/by_hand.rs`: `a_bike_rides_1500_m_in_360_s_and_a_walker_walks_it_in_1125_s` |
| iteration_floor | iterations | 0 | the convergence tier of tests 4 and 6 runs to a fixed count: the rule stops Sioux Falls at twice the population at 22 iterations where 400 are needed, and the deterministic model's residual at 72 000 travellers vanishes at 1 000 iterations, not at 300 (measured 2026-11-07) | `crates/loop/tests/by_hand.rs`: `the_rule_is_not_read_before_the_floor` |
| link_bin | s | 300 | the usual reporting bin of dynamic assignment; Delft's 2 h window gives 24 bins per link | `crates/loop/tests/two_routes.rs` (60 s bins) |
| seed | none | 0 | any fixed value; two runs must agree (test 10) | `crates/loop/tests/determinism.rs`: `the_same_tables_at_1_and_8_threads_and_over_two_runs` |
