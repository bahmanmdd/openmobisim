# Parameters of a run

Every parameter of `run(network, demand, transit=None, **parameters)` is a row here: at most 40, each with a unit, a
default, a source and a test that reads it (`tools/check_limits.sh` counts the rows and looks for each name in the
tests). A parameter that is not in this table does not exist.

| Parameter | Unit | Default | Source | Test that reads it |
|---|---|---|---|---|
