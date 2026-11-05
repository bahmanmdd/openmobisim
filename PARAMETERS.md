# Parameters of a run

Every parameter of `run(network, demand, transit=None, **parameters)` is a row here: at most 40, each with a unit, a
default, a source and a test that reads it (`tools/check_limits.sh` counts the rows and looks for each name in the
tests). A parameter that is not in this table does not exist.

| Parameter | Unit | Default | Source | Test that reads it |
|---|---|---|---|---|
| capacity_per_lane | veh/h per lane | 1 800 | Transportation Research Board, *Highway Capacity Manual*, 7th edition (2022), base saturation flow per lane | `crates/graph/tests/network.rs`: `two_lanes_at_1800_veh_h_per_lane_pass_one_vehicle_per_second` |
| bike_speed | km/h | 15 | de Hartog, Boogaard, Nijland and Hoek, *Environmental Health Perspectives* (2011), urban cycling speed in the Netherlands with stops included | `crates/graph/tests/network.rs`: `a_bike_rides_1500_m_at_15_km_h_in_360_s` |
| walk_speed | km/h | 4.8 | Knoblauch, Pietrucha and Nitzburg, *Transportation Research Record* 1538 (1996), measured mean walking speeds | `crates/graph/tests/network.rs`: `a_walker_covers_1000_m_at_4_8_km_h_in_750_s` |
