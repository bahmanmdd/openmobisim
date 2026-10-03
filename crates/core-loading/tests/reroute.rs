//! En-route rerouting in the loading (S213), with a scripted rerouter.
//!
//! A diamond: from `a`, link `ab` to `b`, then either `bc`–`cd` (planned; `cd` a one-lane
//! service road, so its queue fills `bc` and spills back into `ab`) or `be`–`ed` (free). A
//! vehicle blocked at the front of `ab` long enough is offered a new route; the script sends it
//! by `e`.

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LiveTimes, Recording, Reroute, RerouteReason, RerouteRule, Rules, Trajectory,
    Vehicle, run_ltm_chained,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

const M_PER_DEG_LON: f64 = 111_320.0 * 0.6981; // cos(45.7°)

struct Diamond {
    net: RoadNetwork,
    ab: LinkId,
    bc: LinkId,
    cd: LinkId,
    be: LinkId,
    ed: LinkId,
}

fn diamond() -> Diamond {
    let at = |x: f64, y: f64| LonLat::new(4.8 + x / M_PER_DEG_LON, 45.7 + y / 111_320.0);
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(300.0, 0.0));
    b.add_node("c", at(600.0, 150.0));
    b.add_node("e", at(600.0, -150.0));
    b.add_node("d", at(900.0, 0.0));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Service));
    b.add_link("be", "b", "e", LinkSpec::new(RoadClass::Secondary));
    b.add_link("ed", "e", "d", LinkSpec::new(RoadClass::Secondary));
    let net = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable");
    let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).expect("link");
    let (ab, bc, cd, be, ed) = (id("ab"), id("bc"), id("cd"), id("be"), id("ed"));
    Diamond { net, ab, bc, cd, be, ed }
}

/// A vehicle every second for twenty minutes, all planned by `c`.
fn demand(d: &Diamond) -> Vec<Vehicle> {
    (0..1200u32)
        .map(|n| Vehicle::new(VehicleId::new(n), vec![d.ab, d.bc, d.cd], Pcu(1.0), Second(n)))
        .collect()
}

/// Sends a vehicle at the end of `ab` planned onto `bc` by `e` instead; counts its calls.
struct ByE {
    ab: LinkId,
    bc: LinkId,
    detour: Vec<LinkId>,
    calls: usize,
    decline: bool,
}

impl Reroute for ByE {
    fn reroute(
        &mut self,
        _vehicle: VehicleId,
        current: LinkId,
        planned: &[LinkId],
        _now: f64,
        live: &dyn LiveTimes,
        reason: RerouteReason,
    ) -> Option<Vec<LinkId>> {
        assert_eq!(reason, RerouteReason::Stuck);
        self.calls += 1;
        // The live estimate of a blocked link includes how long its front has waited.
        assert!(live.live_seconds(current) > 0.0);
        if self.decline || current != self.ab || planned.first() != Some(&self.bc) {
            return None;
        }
        Some(self.detour.clone())
    }
}

fn load(
    d: &Diamond,
    rule: Option<RerouteRule>,
    rr: Option<&mut ByE>,
) -> (Vec<Trajectory>, usize, Vec<openmobisim_core_loading::RerouteRecord>) {
    let turns = TurnTable::build(&d.net, SignalDefaults::SHIPPED);
    let vehicles = demand(d);
    let out = run_ltm_chained(
        &d.net,
        &turns,
        &vehicles,
        &[],
        Duration(6.0 * 3600.0),
        Duration(60.0),
        FidelityLevel::Full,
        Recording::Trajectories,
        Rules { priority: false, reroute: rule },
        rr.map(|r| r as &mut dyn Reroute),
    );
    let n = out.trajectories.len();
    (out.trajectories, n, out.reroutes)
}

fn last_arrival(t: &[Trajectory]) -> u32 {
    t.iter().map(|t| t.arrival().get()).max().unwrap_or(0)
}

#[test]
fn a_vehicle_stuck_long_enough_takes_the_way_round_and_the_day_ends_sooner() {
    let d = diamond();
    let (plain, n_plain, none) = load(&d, None, None);
    assert_eq!(n_plain, 1200);
    assert!(none.is_empty());
    let rule = RerouteRule { after_s: 120.0, max: 3 };
    let mut rr = ByE { ab: d.ab, bc: d.bc, detour: vec![d.be, d.ed], calls: 0, decline: false };
    let (rerouted, n, records) = load(&d, Some(rule), Some(&mut rr));
    assert_eq!(n, 1200, "every trip completes");
    assert!(!records.is_empty() && rr.calls >= records.len());
    for r in &records {
        assert_eq!((r.link, r.planned_next, r.new_next), (d.ab, d.bc, d.be));
        assert_eq!(r.reason.as_str(), "stuck");
    }
    // The realised routes: those that re-routed went by `e`, the others by `c`, all whole.
    let by_e: Vec<&Trajectory> =
        rerouted.iter().filter(|t| t.links.iter().any(|x| x.link == d.be)).collect();
    assert_eq!(by_e.len(), records.len(), "one reroute each, and each shows in the trajectory");
    for t in &by_e {
        let links: Vec<LinkId> = t.links.iter().map(|x| x.link).collect();
        assert_eq!(links, vec![d.ab, d.be, d.ed]);
    }
    assert!(rerouted.iter().filter(|t| t.links.iter().any(|x| x.link == d.cd)).count() > 0);
    assert!(
        last_arrival(&rerouted) < last_arrival(&plain),
        "{} vs {}",
        last_arrival(&rerouted),
        last_arrival(&plain)
    );
}

#[test]
fn a_rerouter_that_always_declines_changes_nothing() {
    let d = diamond();
    let (plain, _, _) = load(&d, None, None);
    let rule = RerouteRule { after_s: 120.0, max: 3 };
    let mut rr = ByE { ab: d.ab, bc: d.bc, detour: vec![d.be, d.ed], calls: 0, decline: true };
    let (declined, _, records) = load(&d, Some(rule), Some(&mut rr));
    assert!(rr.calls > 0, "vehicles were offered routes");
    assert!(records.is_empty());
    let key = |t: &[Trajectory]| {
        let mut v: Vec<(u32, u32)> =
            t.iter().map(|t| (t.vehicle.raw(), t.arrival().get())).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(key(&plain), key(&declined), "timers and offers leave the loading as it was");
}

#[test]
fn a_vehicle_re_routes_at_most_max_times_and_only_after_waiting() {
    let d = diamond();
    let rule = RerouteRule { after_s: 120.0, max: 0 };
    let mut rr = ByE { ab: d.ab, bc: d.bc, detour: vec![d.be, d.ed], calls: 0, decline: false };
    let (_, _, records) = load(&d, Some(rule), Some(&mut rr));
    assert_eq!((rr.calls, records.len()), (0, 0), "max 0: never offered");
    let rule = RerouteRule { after_s: 1.0e6, max: 3 };
    let mut rr = ByE { ab: d.ab, bc: d.bc, detour: vec![d.be, d.ed], calls: 0, decline: false };
    let (_, _, records) = load(&d, Some(rule), Some(&mut rr));
    assert_eq!((rr.calls, records.len()), (0, 0), "nobody waits a million seconds");
}
