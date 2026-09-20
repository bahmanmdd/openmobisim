"""Type stubs for the compiled core.

Kept by hand, and deliberately: the signatures here are the contract between
Rust and Python, and writing them out is how that contract stays visible on
the Python side.
"""

import numpy as np
import numpy.typing as npt

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
    """A road network.

    Built by :func:`manhattan_grid`, :func:`toy_network` or
    :func:`network_read_osm`; passed to a ``Scenario``; read as numpy arrays.
    Every per-link array is indexed by the link's internal id, the same index
    the per-link results (:class:`LinkBins`) use.
    """

    #: Where the network came from: ``"osm"`` or ``"synthetic"``.
    source: str
    #: How many nodes.
    node_count: int
    #: How many directed links.
    link_count: int

    def node_lonlat(self, name: str) -> tuple[float, float]:
        """The WGS84 ``(lon, lat)`` of the node with external id ``name``.

        Raises:
            ValueError: If there is no such node.
        """

    def link_geometry(self) -> tuple[npt.NDArray[np.float64], npt.NDArray[np.uint32]]:
        """Every link's polyline as ``(coordinates, offsets)``.

        ``coordinates`` is ``(n_points, 2)`` WGS84 ``(lon, lat)``; link ``i``'s
        points are ``coordinates[offsets[i]:offsets[i + 1]]``, in the direction
        of travel. A link without stored street geometry is the straight line
        between its nodes.
        """

    def link_length_m(self) -> npt.NDArray[np.float64]:
        """Every link's length in metres."""

    def link_free_flow_s(self) -> npt.NDArray[np.float64]:
        """Every link's free-flow traversal time in seconds, control delay included."""

    def link_class(self) -> npt.NDArray[np.uint8]:
        """Every link's road class as a number (0 = motorway)."""

    def link_lanes(self) -> npt.NDArray[np.uint8]:
        """Every link's lane count in its own direction."""

    def link_capacity_pcu_h(self) -> npt.NDArray[np.float64]:
        """Every link's capacity across all its lanes, in PCU per hour.

        The most the link can discharge, before any signal takes its share.
        """

    def link_storage_pcu(self) -> npt.NDArray[np.float64]:
        """Every link's storage at jam density, in PCU."""

class LinkBins:
    """Per-link, per-time-bin results: one row per (bin, link) that saw traffic.

    Rows are sorted by bin then link. A row counts the traversals of the link
    that *finished* in the bin, including those of trips still under way when
    the window ended.
    """

    #: The length of one time bin, in seconds.
    bin_seconds: int

    def __len__(self) -> int:
        """How many rows."""

    def bins(self) -> npt.NDArray[np.uint32]:
        """Each row's bin index; bin ``b`` covers ``[b, b + 1) * bin_seconds``."""

    def links(self) -> npt.NDArray[np.uint32]:
        """Each row's link, as an index into the network's link arrays."""

    def crossings(self) -> npt.NDArray[np.uint32]:
        """Each row's number of finished traversals, unweighted."""

    def pcu(self) -> npt.NDArray[np.float64]:
        """Each row's traffic that left the link, in PCU (weight and vehicle size included)."""

    def pcu_seconds(self) -> npt.NDArray[np.float64]:
        """Each row's PCU-weighted traversal time, in PCU-seconds."""

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

def toy_network() -> Network:
    """The toy network's road part: sixteen nodes and links, every number checkable by hand.

    Returns:
        A :class:`Network`. Node names are ``"W"``, ``"N1"``, ``"S"``,
        ``"M"``, … (see :meth:`Network.node_lonlat`).
    """

def network_read_osm(path: str, contract: bool = True) -> Network:
    """Read a road network from an OpenStreetMap ``.osm.pbf`` extract.

    Args:
        path: The extract.
        contract: Merge chains of degree-two nodes whose links agree on every
            parameter (the default; turn it off only to debug an import).

    Returns:
        A :class:`Network` with the defaults table's parameters and the street
        geometry kept for maps.

    Raises:
        ValueError: If the file cannot be read or holds no usable road network.
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
    #: Per-link, per-time-bin results, if the run asked for them.
    link_bins: LinkBins | None

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
    flow_level: int = 0,
    flow_step_s: int = 300,
    link_bin_s: int | None = None,
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
        flow_level: 0 for free flow, or 2, 3, 4 for the link transmission
            model as a point queue, a spatial queue and the full diagram.
        flow_step_s: The loading step in seconds, for levels 2-4.
        link_bin_s: If given, also record per-link results in bins of this
            many seconds.

    Returns:
        A :class:`RunSummary`.

    Raises:
        ValueError: If the trips/persons arguments are not exactly one form
            each, a given file cannot be read, the demand is empty, or
            writing an output artifact fails.
    """
