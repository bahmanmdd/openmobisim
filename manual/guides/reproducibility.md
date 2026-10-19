# Experiments and reproducibility

A run can be repeated exactly, and every run records what went into it. This guide shows how to use that in a study.
What the fingerprint covers, and why results do not depend on the number of cores, is in
[Reproducibility](../concepts/reproducibility.md).

## Comparing two designs

% src: crates/core-types/src/rng.rs (common random numbers); python/openmobisim/scenario.py Scenario(master_seed=)
Keep the master seed the same (it is 0 unless you set it). Every traveller then gets the same random draws in both
runs wherever their alternatives are the same, so the difference between the results is the design's:

```python
base = ms.Scenario.from_parts(network, trips, master_seed=0).run("base")
design = ms.Scenario.from_parts(edited_network, trips, master_seed=0).run("new-bridge")
```

## Replications

% src: scenario.py Scenario(master_seed=); demand.py demand_sample(seed=), demand_assign_classes(seed=)
To see how much a result varies with chance, run the same scenario with several master seeds. When the demand is
itself drawn (`demand_from_od`, `demand_assign_classes`, `demand_sample`), give those a seed too:

```python
runs = [ms.Scenario.from_parts(network, trips, master_seed=s).run(f"rep{s}") for s in range(5)]
```

The seed only matters where something is drawn at random. Under the `deterministic` choice model nothing is, and the
seed changes only the fingerprint.

## What a run recorded

% src: scenario.py Run.fingerprint, Run.manifest; crates/io-parquet/src/manifest.rs
```python
run.fingerprint            # 16 hex digits: the inputs
run.manifest()["threads"]  # what ran it: also the platform, versions, every method and option
```

Two runs with the same fingerprint were given the same inputs; on the same platform their results are identical. The
fingerprint is printed on the summary page and in the figures' footers, so a figure can be traced to its run.

## The number of cores

% src: scenario.py Scenario.run(threads=)
```python
run = scenario.run("study", threads=4)   # default: every logical core
```

The results are the same with any number. Fewer threads leave cores free for other work, or let several runs share a
machine.
