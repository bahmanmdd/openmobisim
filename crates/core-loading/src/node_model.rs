//! The node model (S48, qualified by S77) in aggregate form: per-turn demand,
//! per-link supply, proportional distribution, full-blocking FIFO on the
//! incoming approach.
//!
//! One call solves one node for one interval of flow — **no fixed point**
//! (S48). This is the reference statement of the rules. [`crate::ltm`] applies
//! the same rules to individual vehicles in time order (supply shared in
//! proportion to discharge capacity, a blocked vehicle blocking its whole
//! approach) and does not call this function; it stays for aggregate loadings
//! (a volume-delay level, a static screening mode) and as the specification
//! the vehicle form is checked against.

use openmobisim_core_types::ids::LinkId;
use openmobisim_core_types::units::Flow;

/// One turn's demand for a step: the share of the incoming link's sending
/// flow aimed at this outgoing link.
///
/// Read from the vehicles about to leave the incoming link, grouped by their
/// next link (S85) — [`crate::ltm`] builds these, this module does not care
/// where they came from.
#[derive(Clone, Copy, Debug)]
pub struct TurnDemand {
    /// The approach.
    pub in_link: LinkId,
    /// Where the demand wants to go.
    pub out_link: LinkId,
    /// How much of it there is.
    pub demand: Flow,
}

/// Solve one node: accepted flow per turn, in the same order as `turns`.
///
/// 1. **Per-link supply:** demand aimed at each outgoing link is summed; the
///    link accepts all of it if there is room, otherwise every turn aimed at
///    it is scaled down by the same factor (*proportional distribution to
///    competing turns*, S48).
/// 2. **Full-blocking FIFO (S77):** an incoming link's *every* turn is then
///    scaled by the **most** restrictive of its own outgoing links' factors
///    — one saturated downstream link blocks the whole approach, not just
///    the turn that feeds it. This is a recorded, directional bias (S77),
///    not a bug: node-level FIFO over-propagates spillback, and the
///    alternative (the Tampère generic node model class) is a reserved
///    add-on, not this function's job.
///
/// `receiving` is queried once per distinct outgoing link among `turns`.
///
/// # Panics
///
/// Never: every link looked up below was recorded from `turns` in the same
/// pass, immediately before the lookup.
#[must_use]
pub fn solve_node(turns: &[TurnDemand], receiving: impl Fn(LinkId) -> Flow) -> Vec<Flow> {
    if turns.is_empty() {
        return Vec::new();
    }

    // Distinct outgoing links, first-seen order — deterministic and, for a
    // node's handful of turns, cheaper than a hash map.
    let mut out_links: Vec<LinkId> = Vec::new();
    let mut out_demand: Vec<Flow> = Vec::new();
    for t in turns {
        match out_links.iter().position(|&l| l == t.out_link) {
            Some(idx) => out_demand[idx] += t.demand,
            None => {
                out_links.push(t.out_link);
                out_demand.push(t.demand);
            }
        }
    }
    let out_factor: Vec<f64> = out_links
        .iter()
        .zip(&out_demand)
        .map(|(&l, &d)| if d.get() <= 0.0 { 1.0 } else { (receiving(l) / d).min(1.0) })
        .collect();

    // Distinct incoming links, first-seen order; each gets the tightest of
    // its own turns' factors (the FIFO full-blocking rule, S77).
    let mut in_links: Vec<LinkId> = Vec::new();
    let mut in_factor: Vec<f64> = Vec::new();
    for t in turns {
        let out_idx =
            out_links.iter().position(|&l| l == t.out_link).expect("out_link was just recorded");
        let factor = out_factor[out_idx];
        match in_links.iter().position(|&l| l == t.in_link) {
            Some(idx) => in_factor[idx] = in_factor[idx].min(factor),
            None => {
                in_links.push(t.in_link);
                in_factor.push(factor);
            }
        }
    }

    turns
        .iter()
        .map(|t| {
            let in_idx =
                in_links.iter().position(|&l| l == t.in_link).expect("in_link was just recorded");
            t.demand * in_factor[in_idx]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use openmobisim_core_types::ids::EntityId;

    use super::*;

    fn link(raw: u32) -> LinkId {
        LinkId::new(raw)
    }

    #[test]
    fn uncontested_turns_pass_at_full_demand() {
        let turns = [TurnDemand {
            in_link: link(0),
            out_link: link(1),
            demand: Flow::from_veh_per_hour(600.0),
        }];
        let accepted = solve_node(&turns, |_| Flow::from_veh_per_hour(1800.0));
        assert_eq!(accepted[0], Flow::from_veh_per_hour(600.0));
    }

    #[test]
    fn competing_turns_share_supply_proportionally() {
        // Two approaches both want out-link 2, demanding 600 and 200 veh/h;
        // it can only receive 400 veh/h — a factor of 1/2.
        let turns = [
            TurnDemand {
                in_link: link(0),
                out_link: link(2),
                demand: Flow::from_veh_per_hour(600.0),
            },
            TurnDemand {
                in_link: link(1),
                out_link: link(2),
                demand: Flow::from_veh_per_hour(200.0),
            },
        ];
        let accepted = solve_node(&turns, |_| Flow::from_veh_per_hour(400.0));
        assert!((accepted[0].as_veh_per_hour() - 300.0).abs() < 1e-9);
        assert!((accepted[1].as_veh_per_hour() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_blocked_turn_blocks_its_whole_approach() {
        // Link 0 has two turns, to link 1 (unconstrained) and link 2 (fully
        // saturated). S77: the saturated turn's factor applies to *both*.
        let turns = [
            TurnDemand {
                in_link: link(0),
                out_link: link(1),
                demand: Flow::from_veh_per_hour(300.0),
            },
            TurnDemand {
                in_link: link(0),
                out_link: link(2),
                demand: Flow::from_veh_per_hour(300.0),
            },
        ];
        let accepted = solve_node(&turns, |l| {
            if l == link(2) { Flow::ZERO } else { Flow::from_veh_per_hour(1800.0) }
        });
        assert_eq!(accepted[0], Flow::ZERO, "link 1's turn is blocked by link 2's saturation");
        assert_eq!(accepted[1], Flow::ZERO);
    }

    #[test]
    fn conservation_never_exceeds_receiving_flow() {
        let turns = [
            TurnDemand {
                in_link: link(0),
                out_link: link(3),
                demand: Flow::from_veh_per_hour(900.0),
            },
            TurnDemand {
                in_link: link(1),
                out_link: link(3),
                demand: Flow::from_veh_per_hour(900.0),
            },
            TurnDemand {
                in_link: link(2),
                out_link: link(3),
                demand: Flow::from_veh_per_hour(900.0),
            },
        ];
        let receiving = Flow::from_veh_per_hour(1000.0);
        let accepted = solve_node(&turns, |_| receiving);
        let total: Flow = accepted.into_iter().sum();
        assert!(total.as_veh_per_hour() <= 1000.0 + 1e-9);
    }
}
