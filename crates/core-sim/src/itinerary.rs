//! Itineraries: a trip routed leg by leg through hubs (S198).
//!
//! A leg runs on one layer, from a node of that layer's graph to another; a
//! hub joins two legs, its access point on the first layer to its access point
//! on the second, at the hub's transfer time (design §3.2). Transit's
//! walk · transit · walk and the park-and-ride and bike-and-ride trips are
//! itineraries of this kind.
//!
//! **In this first version** a car leg is costed at free flow (its loading, in
//! a run, comes with park-and-ride); a bike or walk leg at its layer's static
//! speeds, routed by the layer's cost. **Cost:** one shortest-path search per
//! leg.

use openmobisim_core_graph::hubs::HubSet;
use openmobisim_core_graph::layers::Layer;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, HubId, LinkId, NodeId};

use crate::layers::StaticLayers;

/// The networks an itinerary's legs run on.
#[derive(Clone, Copy)]
pub struct Networks<'a> {
    /// The road network, for car legs.
    pub road: &'a RoadNetwork,
    /// Its turns.
    pub road_turns: &'a TurnTable,
    /// The bike and walk layers.
    pub layers: &'a StaticLayers,
}

/// One leg, routed.
#[derive(Clone, Debug, PartialEq)]
pub struct LegRoute {
    /// The layer it runs on.
    pub layer: Layer,
    /// Its links, on that layer's graph.
    pub links: Vec<LinkId>,
    /// Its travel time, in seconds.
    pub seconds: f64,
}

/// A trip routed through hubs: its legs and the transfers between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Itinerary {
    /// The legs, in order.
    pub legs: Vec<LegRoute>,
    /// The hub between leg `i` and leg `i + 1`, and its transfer time in seconds.
    pub transfers: Vec<(HubId, f64)>,
}

impl Itinerary {
    /// The whole itinerary's time, legs and transfers, in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.legs.iter().map(|l| l.seconds).sum::<f64>()
            + self.transfers.iter().map(|t| t.1).sum::<f64>()
    }
}

impl Networks<'_> {
    /// The shortest leg on `layer` from `from` to `to`, or `None` if the layer
    /// is missing or has no route. A car leg by free-flow time; a bike leg by
    /// the layer's bike cost, a walk leg by time; its seconds are travel time.
    #[must_use]
    pub fn leg(&self, layer: Layer, from: NodeId, to: NodeId) -> Option<LegRoute> {
        match layer {
            Layer::Car => {
                let ctx = SearchContext::new(self.road, self.road_turns);
                let route = Search::new(&ctx).shortest(from, to)?;
                Some(LegRoute { layer, links: route.links, seconds: route.cost })
            }
            Layer::Bike | Layer::Walk => {
                let setup = self.layers.get(layer.as_static()?)?;
                let graph = setup.network().network();
                let ctx = SearchContext::with_costs(graph, setup.turns(), setup.costs().to_vec());
                let route = Search::new(&ctx).shortest(from, to)?;
                let seconds = route.links.iter().map(|l| setup.seconds()[l.index()]).sum();
                Some(LegRoute { layer, links: route.links, seconds })
            }
            Layer::Transit => None,
        }
    }

    /// From `from` on `first` to `to` on `second` through `hub`: a leg to the
    /// hub's access point on `first`, the transfer, a leg from its access point
    /// on `second`. `None` if the hub has no access point on either layer or a
    /// leg has no route.
    #[must_use]
    pub fn via_hub(
        &self,
        hubs: &HubSet,
        hub: HubId,
        first: Layer,
        from: NodeId,
        second: Layer,
        to: NodeId,
    ) -> Option<Itinerary> {
        let arrive = hubs.access_point(hubs.access_point_on(hub, first)?).node;
        let leave = hubs.access_point(hubs.access_point_on(hub, second)?).node;
        let legs = vec![self.leg(first, from, arrive)?, self.leg(second, leave, to)?];
        Some(Itinerary { legs, transfers: vec![(hub, hubs.transfer_seconds(hub))] })
    }
}
