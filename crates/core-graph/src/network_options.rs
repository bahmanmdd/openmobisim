//! Every parameter that shapes a network when it is built, by name (S225, roadmap I-ae).
//!
//! A network is built from data plus defaults: the road-class table (a speed, lanes, saturation
//! flow and jam density per class, used where the data says nothing), the global multipliers, the
//! signal settings, and the bike and walk layers' speeds. [`NetworkDefaults`] holds them all; the
//! readers take it from a map of names to numbers ([`NetworkDefaults::from_options`]), refuse a
//! name they do not know with the list of those they do, and the built network keeps it
//! ([`crate::RoadNetwork::defaults`]) so that everything derived later — the turn table's signal
//! capacities, the bike layer's costs — uses the same values, and its fingerprint covers them.
//!
//! **The names** (every value an uncalibrated default unless its source says otherwise, counted
//! in [`crate::DEFAULTS_VERSION`]):
//!
//! | Name | Unit | What |
//! |---|---|---|
//! | `<class>.free_flow_km_h` | km/h | the class's free-flow speed where the data gives none |
//! | `<class>.lanes` | lanes per direction | where the data gives none |
//! | `<class>.saturation_flow_veh_h_lane` | veh/h per lane | the class's capacity per lane |
//! | `<class>.jam_density_veh_km_lane` | veh/km per lane | storage per lane |
//! | `free_flow_speed_factor`, `capacity_factor`, `jam_density_factor`, `control_delay_factor`, `green_fraction_factor` | — | the global multipliers ([`GlobalMultipliers`]) |
//! | `signal_cycle_s`, `signal_green_fraction`, `signal_degree_of_saturation` | s, —, — | the signal settings ([`SignalDefaults`]) |
//! | `bike_mixed_km_h`, `bike_dedicated_km_h`, `bike_mixed_cost_factor`, `bike_lane_cost_factor`, `walk_km_h`, `ferry_km_h`, `ferry_wait_s` | km/h, km/h, —, —, km/h, km/h, s | the bike and walk layers ([`StaticLayerDefaults`]) |
//!
//! `<class>` is a road class's name as OSM writes it (`motorway`, `primary_link`, `residential`,
//! …; [`RoadClass::as_str`]).

use std::collections::BTreeMap;

use crate::defaults::{DefaultRow, GlobalMultipliers, RoadClass, SignalDefaults, default_row};
use crate::layers::StaticLayerDefaults;

/// One row per road class: the defaults used where the data gives nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClassTable {
    rows: [DefaultRow; RoadClass::ALL.len()],
}

impl ClassTable {
    /// The shipped table ([`default_row`] for every class).
    #[must_use]
    pub fn shipped() -> Self {
        Self { rows: RoadClass::ALL.map(default_row) }
    }

    /// The row of `class`.
    #[must_use]
    pub fn row(&self, class: RoadClass) -> DefaultRow {
        self.rows[class as usize]
    }
}

/// Everything a network is built with besides its data. See the [module docs](self).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NetworkDefaults {
    /// The road-class table.
    pub classes: ClassTable,
    /// The global multipliers.
    pub multipliers: GlobalMultipliers,
    /// The signal settings, before [`GlobalMultipliers::green_fraction`] (see
    /// [`Self::effective_signals`]).
    pub signals: SignalDefaults,
    /// The bike and walk layers' speeds and costs.
    pub layers: StaticLayerDefaults,
}

impl Default for NetworkDefaults {
    fn default() -> Self {
        Self::shipped()
    }
}

/// The global and layer names, with how to read and write each.
type Field = (&'static str, fn(&NetworkDefaults) -> f64, fn(&mut NetworkDefaults, f64));

const FIELDS: [Field; 15] = [
    (
        "free_flow_speed_factor",
        |d| d.multipliers.free_flow_speed,
        |d, v| d.multipliers.free_flow_speed = v,
    ),
    ("capacity_factor", |d| d.multipliers.capacity, |d, v| d.multipliers.capacity = v),
    ("jam_density_factor", |d| d.multipliers.jam_density, |d, v| d.multipliers.jam_density = v),
    (
        "control_delay_factor",
        |d| d.multipliers.control_delay,
        |d, v| d.multipliers.control_delay = v,
    ),
    (
        "green_fraction_factor",
        |d| d.multipliers.green_fraction,
        |d, v| d.multipliers.green_fraction = v,
    ),
    ("signal_cycle_s", |d| d.signals.cycle_seconds, |d, v| d.signals.cycle_seconds = v),
    ("signal_green_fraction", |d| d.signals.green_fraction, |d, v| d.signals.green_fraction = v),
    (
        "signal_degree_of_saturation",
        |d| d.signals.nominal_degree_of_saturation,
        |d, v| d.signals.nominal_degree_of_saturation = v,
    ),
    ("bike_mixed_km_h", |d| d.layers.bike_mixed_km_h, |d, v| d.layers.bike_mixed_km_h = v),
    (
        "bike_dedicated_km_h",
        |d| d.layers.bike_dedicated_km_h,
        |d, v| d.layers.bike_dedicated_km_h = v,
    ),
    (
        "bike_mixed_cost_factor",
        |d| d.layers.bike_mixed_cost_factor,
        |d, v| d.layers.bike_mixed_cost_factor = v,
    ),
    (
        "bike_lane_cost_factor",
        |d| d.layers.bike_lane_cost_factor,
        |d, v| d.layers.bike_lane_cost_factor = v,
    ),
    ("walk_km_h", |d| d.layers.walk_km_h, |d, v| d.layers.walk_km_h = v),
    ("ferry_km_h", |d| d.layers.ferry_km_h, |d, v| d.layers.ferry_km_h = v),
    ("ferry_wait_s", |d| d.layers.ferry_wait_s, |d, v| d.layers.ferry_wait_s = v),
];

/// The per-class fields, by the name after the class's dot.
const CLASS_FIELDS: [&str; 4] =
    ["free_flow_km_h", "lanes", "saturation_flow_veh_h_lane", "jam_density_veh_km_lane"];

impl NetworkDefaults {
    /// The shipped values.
    #[must_use]
    pub fn shipped() -> Self {
        Self {
            classes: ClassTable::shipped(),
            multipliers: GlobalMultipliers::default(),
            signals: SignalDefaults::SHIPPED,
            layers: StaticLayerDefaults::SHIPPED,
        }
    }

    /// The signal settings the network uses: [`Self::signals`] with the green fraction scaled by
    /// [`GlobalMultipliers::green_fraction`] (S225: the factor had no effect before).
    #[must_use]
    pub fn effective_signals(&self) -> SignalDefaults {
        SignalDefaults {
            green_fraction: self.signals.green_fraction * self.multipliers.green_fraction,
            ..self.signals
        }
    }

    /// Every name [`Self::from_options`] accepts, sorted.
    #[must_use]
    pub fn names() -> Vec<String> {
        let mut out: Vec<String> = FIELDS.iter().map(|(n, _, _)| (*n).to_string()).collect();
        for class in RoadClass::ALL {
            for field in CLASS_FIELDS {
                out.push(format!("{}.{field}", class.as_str()));
            }
        }
        out.sort();
        out
    }

    /// Every parameter's value, by name, sorted by name.
    #[must_use]
    pub fn values(&self) -> Vec<(String, f64)> {
        let mut out: Vec<(String, f64)> =
            FIELDS.iter().map(|(n, get, _)| ((*n).to_string(), get(self))).collect();
        for class in RoadClass::ALL {
            let row = self.classes.row(class);
            let name = class.as_str();
            out.push((format!("{name}.free_flow_km_h"), row.free_flow_km_h));
            out.push((format!("{name}.lanes"), f64::from(row.lanes_per_direction)));
            out.push((
                format!("{name}.saturation_flow_veh_h_lane"),
                row.saturation_flow_veh_h_lane,
            ));
            out.push((format!("{name}.jam_density_veh_km_lane"), row.jam_density_veh_km_lane));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The values that differ from the shipped ones, `name=value;…` in name order; empty at the
    /// shipped values. Part of the network's fingerprint.
    #[must_use]
    pub fn descriptor(&self) -> String {
        let shipped = Self::shipped().values();
        self.values()
            .into_iter()
            .zip(shipped)
            .filter(|((_, v), (_, s))| v.to_bits() != s.to_bits())
            .map(|((n, v), _)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The shipped values with `options` applied, by name.
    ///
    /// # Errors
    ///
    /// A message naming an unknown name (with every known one), or a value that is not finite
    /// and positive (a lane count must also be a whole number from 1 to 20; a fraction, from 0 to
    /// 1).
    pub fn from_options(options: &BTreeMap<String, f64>) -> Result<Self, String> {
        let mut d = Self::shipped();
        for (name, &v) in options {
            if !(v.is_finite() && v > 0.0) {
                return Err(format!("network option {name} must be a positive number, got {v}"));
            }
            if let Some((_, _, set)) = FIELDS.iter().find(|(n, _, _)| *n == name) {
                if matches!(name.as_str(), "signal_green_fraction" | "signal_degree_of_saturation")
                    && v > 1.0
                {
                    return Err(format!("network option {name} must be from 0 to 1, got {v}"));
                }
                set(&mut d, v);
                continue;
            }
            let class = name.split_once('.').and_then(|(c, field)| {
                let class = RoadClass::ALL.into_iter().find(|k| k.as_str() == c)?;
                CLASS_FIELDS.contains(&field).then_some((class, field))
            });
            let Some((class, field)) = class else {
                return Err(format!(
                    "no network option called {name:?}; the options are: {}",
                    Self::names().join(", ")
                ));
            };
            let row = &mut d.classes.rows[class as usize];
            match field {
                "free_flow_km_h" => row.free_flow_km_h = v,
                "lanes" => {
                    if v.fract() != 0.0 || v > 20.0 {
                        return Err(format!(
                            "network option {name} must be a whole number from 1 to 20, got {v}"
                        ));
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a whole number from 1 to 20"
                    )]
                    let lanes = v as u8;
                    row.lanes_per_direction = lanes;
                }
                "saturation_flow_veh_h_lane" => row.saturation_flow_veh_h_lane = v,
                _ => row.jam_density_veh_km_lane = v,
            }
        }
        Ok(d)
    }
}
