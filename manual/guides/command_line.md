# The command line

The `openmobisim` command (or `python -m openmobisim`) runs the starter cases without writing any Python. It has three
subcommands. The full help, as the code prints it, is in [The command line](../reference/command_line.md).

## List the starter cases

% src: python/openmobisim/__main__.py _cases; tested at the manual's build
```text
$ openmobisim cases
amsterdam      base             Amsterdam
delft          base             Delft
lyon           base             Lyon
nguyendupuis   base             Nguyen–Dupuis
paris          base             Paris
siouxfalls     base, congested  Sioux Falls
```

Each line is a case, its scenarios and its title. The cases are described in
[Starter cases and the data bundle](../reference/cases.md).

## Run one

% src: python/openmobisim/__main__.py _run
```text
$ openmobisim run delft            # the scenario "base"
$ openmobisim run siouxfalls congested --output runs/sf
```

The run writes its files into `--output`, by default `./openmobisim-runs/<case>-<scenario>`. Unless you pass
`--no-pages`, it also writes:
- `summary.html`, the summary page (it needs `matplotlib`: `pip install "openmobisim[viz]"`);
- `map.html`, the interactive map, with per-link results in 15-minute bins.

It then prints:
- the trips completed out of all trips;
- the mean trip time and the number of loadings;
- each mode's share of the people;
- the run's fingerprint, the package version and the platform;
- where the files are.

`--root` names a folder holding the data bundle, if it is not where the package looks.

## Check the installation

% src: python/openmobisim/__main__.py _check; python/openmobisim/_cases.py case_check, CHECK_TOLERANCE
```text
$ openmobisim check                # every case
$ openmobisim check delft siouxfalls
```

This runs each case's default scenarios and compares them with the reference results shipped with the package. The
verdicts:
- `same`: the same platform and the same numbers;
- `same numbers (another platform)`;
- `within tolerance`;
- `different`;
- `no reference`.

The command exits with 1 if any scenario differs. The tolerances across platforms are in
[Reproducibility](../concepts/reproducibility.md).
