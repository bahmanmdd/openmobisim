//! Numbers the user gives per link of a layer, offered to choice models (S236, roadmap I-bb
//! U4): greenery along a bike link, its slope, the car flow on the road beside it (from a
//! previous run), a toll, lighting — anything a utility may weigh.
//!
//! A column named `name` on layer `layer` (`road`, `bike` or `walk`) becomes two attributes of
//! every alternative whose vehicle or walking leg runs on that layer:
//!
//! - `<layer>_<name>_km`: the sum over the leg's links of the value times the link's length in
//!   kilometres (its mean along the leg is this over `length_km`);
//! - `<layer>_<name>_sum`: the plain sum over the leg's links (a toll, a count of signals).
//!
//! An alternative with no leg on the layer has 0 (a transit itinerary's walks to and from
//! stops are not searched link by link, so they carry none). Like every attribute, they are
//! filled only when a model reads them: the built-in models when given `beta_<attribute>`, a
//! model of the user's own when it lists them in `attributes` or lists none.
//!
//! **Cost:** a length-weighted copy of each column, made once per run; per alternative and per
//! attribute read, one pass over its leg's links.

use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId};

use crate::layers::StaticLayers;

/// The layer a column belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum ValueLayer {
    /// The road network (car routes, park-and-ride's car legs).
    Road,
    /// The bike layer (bike routes, bike-and-ride's bike legs).
    Bike,
    /// The walk layer (walk routes).
    Walk,
}

impl ValueLayer {
    /// The stable name: `road`, `bike` or `walk`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ValueLayer::Road => "road",
            ValueLayer::Bike => "bike",
            ValueLayer::Walk => "walk",
        }
    }

    /// The layer named `name`, if it is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "road" => Some(ValueLayer::Road),
            "bike" => Some(ValueLayer::Bike),
            "walk" => Some(ValueLayer::Walk),
            _ => None,
        }
    }
}

/// How a column is summed along a leg.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Aggregate {
    /// Value times kilometres.
    Km,
    /// The plain sum.
    Sum,
}

/// One column: its layer, its name and a value per link of the layer, in link order.
#[derive(Clone, Debug, PartialEq)]
struct Column {
    layer: ValueLayer,
    name: String,
    values: Vec<f64>,
}

/// The user's link values: see the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkValues {
    columns: Vec<Column>,
}

impl LinkValues {
    /// No values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same values with a column `name` on `layer`, one value per link of the layer in
    /// link order.
    ///
    /// # Errors
    ///
    /// A message for a name that is empty or not lower-case letters, digits and `_`, that the
    /// layer already has, or whose attributes would take a built-in attribute's name
    /// (`reserved`); or for a value that is not finite.
    pub fn with_column(
        mut self,
        layer: ValueLayer,
        name: &str,
        values: Vec<f64>,
        reserved: &[&str],
    ) -> Result<Self, String> {
        let what = format!("link value {:?} on the {} layer", name, layer.as_str());
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!("{what}: a name is lower-case letters, digits and _"));
        }
        if self.columns.iter().any(|c| c.layer == layer && c.name == name) {
            return Err(format!("{what} is given twice"));
        }
        for suffix in ["km", "sum"] {
            let attribute = format!("{}_{name}_{suffix}", layer.as_str());
            if reserved.contains(&attribute.as_str()) {
                return Err(format!("{what}: {attribute} is a built-in attribute; choose another"));
            }
        }
        if let Some(i) = values.iter().position(|v| !v.is_finite()) {
            return Err(format!("{what}: link {i} has {}, not a finite number", values[i]));
        }
        self.columns.push(Column { layer, name: name.to_string(), values });
        Ok(self)
    }

    /// Whether there is no column.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Every column as `(layer, name, values)`, in the order given.
    pub fn columns(&self) -> impl Iterator<Item = (ValueLayer, &str, &[f64])> {
        self.columns.iter().map(|c| (c.layer, c.name.as_str(), c.values.as_slice()))
    }

    /// The attributes the columns make, in the order given: each column's `_km`, then `_sum`.
    #[must_use]
    pub fn attribute_names(&self) -> Vec<String> {
        self.columns
            .iter()
            .flat_map(|c| ["km", "sum"].map(|s| format!("{}_{}_{s}", c.layer.as_str(), c.name)))
            .collect()
    }

    /// Ready for a run on `road` and `layers`: each column checked against its layer's link
    /// count and weighted by its links' lengths.
    ///
    /// # Errors
    ///
    /// A message for a column whose layer the run does not have, or whose length differs
    /// from the layer's link count.
    pub(crate) fn prepare(
        &self,
        road: &RoadNetwork,
        layers: &StaticLayers,
    ) -> Result<PreparedLinkValues, String> {
        let mut columns = Vec::with_capacity(self.columns.len());
        for c in &self.columns {
            let graph = match c.layer {
                ValueLayer::Road => Some(road),
                ValueLayer::Bike => layers.get(StaticLayer::Bike).map(|l| l.network().network()),
                ValueLayer::Walk => layers.get(StaticLayer::Walk).map(|l| l.network().network()),
            };
            let Some(graph) = graph else {
                // The run never offers a leg on this layer: its attributes are 0.
                columns.push(Prepared {
                    layer: c.layer,
                    name: c.name.clone(),
                    sum: Vec::new(),
                    km: Vec::new(),
                });
                continue;
            };
            let links = graph.link_count() as usize;
            if c.values.len() != links {
                return Err(format!(
                    "link value {:?} on the {} layer has {} values; the layer has {links} links",
                    c.name,
                    c.layer.as_str(),
                    c.values.len()
                ));
            }
            let km = c
                .values
                .iter()
                .enumerate()
                .map(|(i, v)| v * graph.link_length(LinkId::from_index(i)).get() / 1000.0)
                .collect();
            columns.push(Prepared {
                layer: c.layer,
                name: c.name.clone(),
                sum: c.values.clone(),
                km,
            });
        }
        Ok(PreparedLinkValues { columns })
    }
}

#[derive(Debug)]
struct Prepared {
    layer: ValueLayer,
    name: String,
    sum: Vec<f64>,
    km: Vec<f64>,
}

/// [`LinkValues`] ready for a run: each column also weighted by its links' lengths.
#[derive(Debug, Default)]
pub(crate) struct PreparedLinkValues {
    columns: Vec<Prepared>,
}

impl PreparedLinkValues {
    /// The column and the aggregate attribute `name` stands for, if it is one of these.
    pub(crate) fn lookup(&self, name: &str) -> Option<(usize, Aggregate)> {
        let (rest, aggregate) = if let Some(r) = name.strip_suffix("_km") {
            (r, Aggregate::Km)
        } else {
            (name.strip_suffix("_sum")?, Aggregate::Sum)
        };
        self.columns
            .iter()
            .position(|c| {
                rest.strip_prefix(c.layer.as_str())
                    .and_then(|r| r.strip_prefix('_'))
                    .is_some_and(|n| n == c.name)
            })
            .map(|i| (i, aggregate))
    }

    /// The column named `name` on `layer`, if given (S248: the toll, [`crate::prices`]).
    pub(crate) fn column(&self, layer: ValueLayer, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.layer == layer && c.name == name)
    }

    /// The layer of column `column`.
    pub(crate) fn layer(&self, column: usize) -> ValueLayer {
        self.columns[column].layer
    }

    /// Column `column` summed by `aggregate` over `links` (link indices on its layer), in their
    /// order.
    pub(crate) fn total(
        &self,
        column: usize,
        aggregate: Aggregate,
        links: impl Iterator<Item = usize>,
    ) -> f64 {
        let c = &self.columns[column];
        let values = match aggregate {
            Aggregate::Km => &c.km,
            Aggregate::Sum => &c.sum,
        };
        links.map(|l| values.get(l).copied().unwrap_or(0.0)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_and_make_two_attributes_each() {
        let v = LinkValues::new()
            .with_column(ValueLayer::Bike, "greenery", vec![0.5, 1.0], &["bike_mixed_km"])
            .unwrap();
        assert_eq!(v.attribute_names(), ["bike_greenery_km", "bike_greenery_sum"]);
        let bad = |name: &str| {
            LinkValues::new().with_column(ValueLayer::Bike, name, vec![], &["bike_mixed_km"])
        };
        assert!(bad("Green").is_err() && bad("").is_err() && bad("a-b").is_err());
        assert!(bad("mixed").unwrap_err().contains("built-in"));
        assert!(
            v.clone()
                .with_column(ValueLayer::Bike, "greenery", vec![], &[])
                .unwrap_err()
                .contains("twice")
        );
        assert!(
            LinkValues::new()
                .with_column(ValueLayer::Road, "toll", vec![f64::NAN], &[])
                .unwrap_err()
                .contains("finite")
        );
    }

    #[test]
    fn a_name_is_looked_up_by_its_layer_and_aggregate() {
        let p = PreparedLinkValues {
            columns: vec![
                Prepared {
                    layer: ValueLayer::Bike,
                    name: "green".into(),
                    sum: vec![1.0, 2.0, 4.0],
                    km: vec![0.1, 0.4, 1.2],
                },
                Prepared {
                    layer: ValueLayer::Road,
                    name: "toll_eur".into(),
                    sum: vec![0.0, 2.5],
                    km: vec![0.0, 0.5],
                },
            ],
        };
        assert_eq!(p.lookup("bike_green_km"), Some((0, Aggregate::Km)));
        assert_eq!(p.lookup("road_toll_eur_sum"), Some((1, Aggregate::Sum)));
        assert_eq!(p.lookup("walk_green_km"), None);
        assert_eq!(p.lookup("bike_green"), None);
        assert!((p.total(0, Aggregate::Sum, [0, 2].into_iter()) - 5.0).abs() < 1e-12);
        assert!((p.total(0, Aggregate::Km, [1, 2].into_iter()) - 1.6).abs() < 1e-12);
    }
}
