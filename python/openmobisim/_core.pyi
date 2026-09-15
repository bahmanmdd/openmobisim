"""Type stubs for the compiled core.

Kept by hand, and deliberately: the signatures here are the contract between
Rust and Python, and writing them out is how that contract stays visible on
the Python side.
"""

__version__: str

def build_info() -> dict[str, object]:
    """Return facts about the compiled core.

    Returns:
        A dict with keys ``version`` (str), ``code_version`` (int),
        ``rng_scheme_version`` (int), ``parallel`` (bool), ``target`` (str) and
        ``profile`` (str). These are the fields a bug report should quote.
    """

def fixed_order_sum_f64(values: list[float]) -> float:
    """Sum ``values`` with the core's fixed-order reduction.

    Args:
        values: The numbers to add.

    Returns:
        The sum, identical on every call with the same input on the same
        platform, whatever the thread count.
    """

def step_of(
    origin_second: int,
    step_seconds: int,
    window_seconds: int,
    second: int,
) -> int:
    """Return the loading step containing ``second``.

    Args:
        origin_second: The scenario origin, in seconds.
        step_seconds: The loading step length, in seconds.
        window_seconds: The window length; must be a whole number of steps.
        second: The instant to locate, in seconds from the same origin.

    Returns:
        The zero-based step index, clamped to the window.

    Raises:
        ValueError: If the grid is not valid — a zero step, a window that is
            not a whole number of steps, or one running past the end of the
            32-bit clock.
    """

def choice_draw(
    master_seed: int,
    traveller: int,
    trip: int,
    iteration: int,
    alternative: int,
) -> float:
    """Return one uniform draw in ``[0, 1)`` from the choice stream.

    A pure function of its arguments: the same key always gives the same
    number, and changing any component changes it.

    Args:
        master_seed: The scenario's master seed.
        traveller: The traveller's id.
        trip: The trip's id.
        iteration: The assignment iteration.
        alternative: The alternative's identity.

    Returns:
        A number in ``[0, 1)``.
    """

class Network:
    """A road network, opaque from Python.

    Built by :func:`manhattan_grid`, passed straight back into
    :func:`run_pipeline` (or held by a ``Scenario``). Nothing on the Python
    side reads its fields directly.
    """

def manhattan_grid(n: int, block_metres: float, signals: bool) -> Network:
    """Build an n x n grid network, block_metres apart (S105, N3).

    Args:
        n: Nodes per side. At least 2.
        block_metres: The distance between adjacent nodes.
        signals: Whether interior nodes are signal-controlled.

    Returns:
        A :class:`Network`.

    Raises:
        ValueError: If ``n < 2``.
    """

def grid_node_lonlat(network: Network, row: int, col: int) -> tuple[float, float]:
    """The WGS84 (lon, lat) of grid position ``(row, col)``.

    Grid-specific introspection, not a general :class:`Network` method — a
    network built any other way has no row/col positions.

    Args:
        network: Built by :func:`manhattan_grid`.
        row: Zero-based row.
        col: Zero-based column.

    Returns:
        ``(lon, lat)`` in degrees.

    Raises:
        ValueError: If ``(row, col)`` is not a node of this network.
    """

class RunSummary:
    """What one run produced, as plain fields.

    Returned by :func:`run_pipeline`; ``openmobisim.scenario.Run`` wraps this
    into the ergonomic result object ``Scenario.run()`` returns.
    """

    kpis_path: str
    diagnostics_path: str
    events_path: str
    manifest_path: str
    total_trips: int
    completed: int
    truncated: int
    no_vehicle_available: int
    no_feasible_path: int
    total_travel_time_s: float

def run_pipeline(
    network: Network,
    run_id: str,
    output_dir: str,
    trips: list[tuple] | None = None,
    trips_path: str | None = None,
    persons: list[tuple] | None = None,
    persons_path: str | None = None,
    class_defaults: dict[str, tuple[bool, bool, bool]] | None = None,
    default_weight: int = 1,
    window_s: int = 86_400,
) -> RunSummary:
    """Run the whole Phase 1 pipeline and write all four output artifacts.

    Prefer ``openmobisim.Scenario`` to calling this directly — it manages
    ``output_dir`` and the trips/trips_path (and persons/persons_path)
    exactly-one-of pairing for you.

    Args:
        network: Built by :func:`manhattan_grid`.
        run_id: Recorded in every output row.
        output_dir: Where the four artifacts are written.
        trips: An in-memory trips table — rows of
            ``(traveller_id, trip_seq, origin_lon, origin_lat,
            destination_lon, destination_lat, departure_time_s, user_class,
            weight)``. Exactly one of `trips`/`trips_path` must be given.
        trips_path: A ``trips.parquet`` path (S97), instead of `trips`.
        persons: An in-memory persons table — rows of
            ``(traveller_id, owns_car, owns_bike, has_transit_pass,
            user_class)``, each field but `traveller_id` optional. At most
            one of `persons`/`persons_path` may be given.
        persons_path: A ``persons.parquet`` path, instead of `persons`.
        class_defaults: ``{class_name: (owns_car, owns_bike,
            has_transit_pass)}`` — S127's per-class default ownership.
        default_weight: The scenario's `traveller_weight` (S89), for a trip
            whose row gives none.
        window_s: Trips still in progress after this second are truncated
            (S57).

    Returns:
        A :class:`RunSummary`.

    Raises:
        ValueError: If the trips/persons arguments are not exactly one form
            each, a given file cannot be read, the demand is empty, or
            writing an output artifact fails.
    """
