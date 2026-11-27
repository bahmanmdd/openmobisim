```{include} ../../PARAMETERS.md
```

## Where each parameter is given

% src: crates/binding/README.md ("Keywords")
A parameter is a keyword of the entry point whose module reads it, or a key of `[parameters]` in the settings file
`run.toml`, which gives each to the right entry point ([The settings file and the record](record.md)).

| Entry point | Parameters |
|---|---|
| `Network.from_osm`, `Network.from_tables` | `capacity_per_lane`, `bike_speed`, `walk_speed` |
| `Network.from_osm` alone | `osm_class_defaults` |
| `Transit.from_gtfs` | `transfer_walk_max` |
| `Demand.from_table` | `stop_walk_max` |
| `run` | `boarding_slack`, `max_rides`, `nearby_stops`, `max_car_routes`, `max_transit_journeys`, `choice`, `coefficients`, `nest_scale`, `step_exponent`, `iteration_cap`, `iteration_floor`, `link_bin`, `seed` |
