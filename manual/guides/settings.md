# Which setting when

The settings that change a study's results most, by what you want. Every one has a default that suits a first run;
the pages linked explain each.

% src: concepts/loading.md (flow levels), equilibration.md, route_sets.md (which method a run uses), choice.md,
% guides/speed.md; python/openmobisim/scenario.py (defaults)

| You want | Set | Why | More |
|---|---|---|---|
| A first look, or a check of the network and demand | `flow_level=0`, `equilibration="free_flow"` | No vehicle affects another and nothing iterates: the quickest run, travel times at free flow | [The loading](../concepts/loading.md) |
| A planning study of a morning | the defaults: `flow_level=2`, `equilibration="msa"` | Queues form at every bottleneck, the iterations settle steadily, and nothing can gridlock | [Equilibration](../concepts/equilibration.md) |
| Queues that block the junctions behind them: incidents, works, traffic control | `flow_level=4` | Queues fill the road and spill back, with room freed at a link's end reaching its start after the backward wave | [The loading](../concepts/loading.md) |
| A quick estimate of a congested morning, without iterating | `equilibration="free_flow"` | One loading, built up in groups so later groups see the congestion of earlier ones | [Equilibration](../concepts/equilibration.md) |
| Several car routes per pair from the start, for a route choice study | `route_method="penalty"` | The default starts from one route per pair and lets the route update add those congestion makes worthwhile | [Route sets](../concepts/route_sets.md) |
| Routes that go round the places likely to jam | `route_method="montecarlo"` | Random link costs, raised most where the demand is likely to exceed capacity, find the detours drivers would take there | [Route sets](../concepts/route_sets.md) |
| Travellers who all take the best alternative | `choice_model="deterministic"` | All or nothing: a deterministic user equilibrium, as in textbooks and the validation cases | [Choice](../concepts/choice.md) |
| Mode choice that a mode's number of routes cannot sway | the default: `nested_logit` when a run offers several modes | One nest per mode, so a mode with many routes does not gain share from their number alone | [Choice](../concepts/choice.md) |
| A faster run for screening | `ms.demand_sample(trips, weight=k)` or a case's `weight=`, and `preset="fast"` | One simulated traveller for `k` people, and shared route searches; totals kept | [Running faster](speed.md) |
| Every person simulated, for final numbers | `weight=1` and `preset="complete"` | Queues form car by car, and every route search is its own | [Running faster](speed.md) |
| Thousands of runs | `outputs="minimal"` | A few kilobytes a run, the same results | [Results](results.md#many-runs-the-minimal-outputs) |
| Travellers who plan around a disruption | `disruptions_known=True` | Every loading includes it; by default only the day itself reacts | [Scenarios](scenarios.md) |
| Two runs that differ only in your change | the same `master_seed` | Every traveller draws the same in both | [Experiments and reproducibility](reproducibility.md) |
