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

    def node_nearest(self, lon: float, lat: float) -> int:
        """The index of the drivable node nearest to ``(lon, lat)``.

        This is the node a trip starting or ending there is routed from or to,
        and node indices key :class:`RouteSets`.
        """

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

    def link_drivable(self) -> npt.NDArray[np.bool_]:
        """Whether each link may be used by a car.

        ``False`` for footways, steps, paths, tracks, cycleways and pedestrian
        streets: they are in the network for the walk and cycle layers, and a
        car is never routed over them.
        """

    def link_roundabout(self) -> npt.NDArray[np.bool_]:
        """Whether each link is part of a roundabout's circulating carriageway."""

    def node_signalised(self) -> npt.NDArray[np.bool_]:
        """Whether each node is signalised, indexed by node index."""

    def link_from(self) -> npt.NDArray[np.uint32]:
        """Every link's start node, as a node index."""

    def link_to(self) -> npt.NDArray[np.uint32]:
        """Every link's end node, as a node index."""

    def report_connectivity(self, mode: str = "car") -> dict[str, object]:
        """Whether the network is strongly connected, and how far from it if not.

        Args:
            mode: ``"car"`` (the default) asks about the links a car may use
                only, so a footway joining two roads does not join them for a
                car; ``"all"`` asks about every link.

        Returns:
            A dict of counts. ``strongly_connected`` is the verdict: every node
            reaches every other. ``node_components`` and ``node_largest`` say
            how many pieces there are and how many nodes the largest holds,
            ``sources`` and ``sinks`` count the nodes a vehicle can only leave
            or only reach, and ``node_component_sizes_top`` lists the ten
            largest pieces. ``link_components`` and ``link_largest`` are the
            same for the link graph through the legal turns, which is
            informational: a vehicle cannot U-turn except at a dead end, so a
            two-way triangle of streets is two components there, one for each
            way round. ``turns``, ``u_turns`` and ``max_turns_per_node`` describe
            the turn table.

        Raises:
            ValueError: If ``mode`` is not one of the two.
        """

    def report_import(self) -> dict[str, object] | None:
        """What the OSM import did; ``None`` for a network not read from OSM.

        The counts (``ways_seen``, ``ways_kept``, ``nodes_seen``, ``nodes_kept``,
        ``junction_nodes``, ``links_before_contraction``,
        ``links_after_contraction``, ``nodes_contracted``), what was left out
        (``closed_loops_dropped``; and, with ``connectivity="strong"``,
        ``components_before``, ``links_disconnected``, ``nodes_disconnected``,
        ``length_disconnected_m``), how much of the drivable network states its own
        speed limit and lane count rather than taking the class default
        (``drivable_links``, ``drivable_length_m``, ``drivable_maxspeed_links``,
        ``drivable_maxspeed_length_m``, ``drivable_lanes_links``,
        ``drivable_lanes_length_m``), the options used (``connectivity``,
        ``region``) and ``diagnostics``: every data-quality diagnostic as
        ``{code: {"severity": ..., "count": ...}}``.
        """

    def report_dropped(
        self,
    ) -> (
        tuple[
            npt.NDArray[np.float64],
            npt.NDArray[np.uint32],
            npt.NDArray[np.int64],
            npt.NDArray[np.uint8],
            npt.NDArray[np.uint8],
        ]
        | None
    ):
        """The roads the import left out, so that they can be looked at.

        Returns ``None`` for a network not read from OSM, otherwise
        ``(coordinates, offsets, way_ids, reasons, classes)``: like
        :meth:`link_geometry`, ``(n_points, 2)`` WGS84 ``(lon, lat)`` with
        ``offsets`` into it, and per dropped link its OSM way id, the reason
        (0 = a closed loop that could not be a link, 1 = outside the largest
        strongly connected part of the drivable network) and its road class
        (numbered as :meth:`link_class`).
        """

class RouteSets:
    """The alternative routes of many origin-destination pairs.

    Keys are pairs of node indices (see :meth:`Network.node_nearest`), sorted.
    Route ``r`` is ``links()[route_offsets()[r]:route_offsets()[r + 1]]``; key
    ``k`` owns the routes ``set_offsets()[k]:set_offsets()[k + 1]``: the method's routes
    best first, then, if a route update grew the sets during a run, the routes it added, in
    the order added (``stamps()`` says when).
    """

    #: The method that made these sets, for example ``"penalty"``.
    method: str
    #: The method and every option with defaults filled in, as one string.
    descriptor: str
    #: The route update that grew the sets after the method made them, with its options, as
    #: one string; empty for sets as the method made them.
    update: str
    #: A short hex identity of the network and the method with its options (and the update,
    #: if the sets were grown).
    identity: str
    #: How many origin-destination pairs have a set.
    key_count: int
    #: How many routes in all.
    route_count: int
    #: Bytes held.
    bytes: int

    def keys(self) -> tuple[npt.NDArray[np.uint32], npt.NDArray[np.uint32]]:
        """The keys as ``(origin_nodes, destination_nodes)``, sorted."""

    def set_offsets(self) -> npt.NDArray[np.uint32]:
        """``key_count + 1`` offsets into the routes."""

    def route_offsets(self) -> npt.NDArray[np.uint32]:
        """``route_count + 1`` offsets into :meth:`links`."""

    def links(self) -> npt.NDArray[np.uint32]:
        """Every route's links, one flat array of link indices."""

    def costs(self) -> npt.NDArray[np.float32]:
        """Every route's free-flow cost in seconds."""

    def stamps(self) -> npt.NDArray[np.uint32]:
        """Every route's stamp: 0 if the method made it, else the iteration it was added for.

        A route update adds routes between iterations; the stamp is the iteration whose
        choice they were added for.
        """

    def overlaps(self) -> npt.NDArray[np.float32]:
        """Every route's largest share of its cost shared with a route found before it."""

    def find(self, origin: int, destination: int) -> int | None:
        """The position of the pair among the keys, or ``None`` if it has no set."""

    def routes(self, key_index: int) -> list[npt.NDArray[np.uint32]]:
        """The routes of the key at ``key_index``, each as link indices.

        The method's routes come best first, then any a route update added.

        Raises:
            ValueError: If ``key_index`` is out of range.
        """

    def link_routes(self, link: int) -> npt.NDArray[np.uint32]:
        """The route numbers that use ``link``, ascending (the inverted index).

        Raises:
            ValueError: If ``link`` is out of range.
        """

    def route_key(self, route: int) -> int:
        """The key (position among the keys) that route number ``route`` belongs to.

        Raises:
            ValueError: If ``route`` is out of range.
        """

def route_methods() -> list[str]:
    """The route-set methods that can be selected, the default (``"penalty"``) first."""

def route_sets_build(
    network: Network,
    od_lonlat: list[tuple[float, float, float, float]],
    method: str = "penalty",
    options: dict[str, float] | None = None,
) -> RouteSets:
    """Make route sets for origin-destination pairs given as coordinates.

    Args:
        network: The network to route on.
        od_lonlat: Pairs ``(origin_lon, origin_lat, destination_lon,
            destination_lat)``. Each end is snapped to the nearest drivable
            node; pairs that snap to one node are dropped.
        method: A name from :func:`route_methods`.
        options: The method's options, numbers by name. For ``"penalty"``:
            ``max_paths`` (5), ``max_detour`` (1.3), ``max_overlap`` (0.75),
            ``penalty`` (1.5), ``max_attempts`` (15).
            For ``"montecarlo"``: ``draws`` (16), ``sigma`` (4), ``max_detour`` (2),
            ``max_overlap`` (0.9), ``max_paths`` (10), ``seed`` (0) and ``biased`` (1; 0 for
            none). The method's noise is biased towards the links the demand is likely to
            congest; here the demand is the given pairs, one traveller each, leaving at once.

    Returns:
        The sets, one per distinct pair, best route first.

    Raises:
        ValueError: For an unknown method, an unknown option or an
            out-of-range value.
    """

class ChoiceBatch:
    """What a Python choice model is handed: situations × alternatives, as arrays.

    A situation is one traveller's one trip; it offers a few alternatives (routes),
    each with the same numeric attributes. See :mod:`openmobisim.choice`.
    """

    #: The iteration these choices are for.
    iteration: int
    #: The attributes on offer.
    attribute_names: list[str]
    #: ``situations + 1`` offsets: situation ``s`` owns alternatives ``offsets[s]:offsets[s + 1]``.
    offsets: npt.NDArray[np.uint32]
    #: Each situation's traveller and trip.
    traveller: npt.NDArray[np.uint32]
    trip: npt.NDArray[np.uint32]
    #: Each alternative's identity (stable when other alternatives come and go).
    identity: npt.NDArray[np.uint32]
    #: Which situation each alternative belongs to.
    situation_of: npt.NDArray[np.uint32]
    #: The attributes by name, one float64 value per alternative.
    attributes: dict[str, npt.NDArray[np.float64]]
    #: Each alternative's standard Gumbel error, keyed on (traveller, trip, iteration, identity).
    gumbel: npt.NDArray[np.float64]

    def __len__(self) -> int: ...

class RouteChoices:
    """What every trip chose, one entry per trip (``-1`` where a trip has no route)."""

    #: The route taken, as an index into the run's route sets.
    route: npt.NDArray[np.int64]
    #: Its position in its pair's set (0 is the best route).
    rank: npt.NDArray[np.int64]
    #: Its pair's index in the route sets.
    pair: npt.NDArray[np.int64]
    #: How many routes the trip could choose from.
    alternatives: npt.NDArray[np.uint32]
    #: What the model gave the route taken (``nan`` if it gave none).
    probability: npt.NDArray[np.float64]
    #: How many people the trip stands for.
    weight: npt.NDArray[np.uint32]

    def __len__(self) -> int: ...

def equilibration_strategies() -> list[str]:
    """The equilibration strategies that can be selected by name, the default (``"none"``) first."""

def route_cache_clear() -> None:
    """Forget every route set the cache holds (the counters of :func:`route_cache_info` stay)."""

def route_cache_info() -> tuple[int, int, int]:
    """What the route-set cache has done: ``(hits, misses, sets held)``."""

def route_update_methods() -> list[str]:
    """The route updates that can be selected by name, the default (``"none"``) first."""

def choice_models() -> list[str]:
    """The choice models that can be selected by name, the default (``"deterministic"``) first."""

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
    """Build an n x n grid network, block_metres apart.

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

def network_read_osm(
    path: str,
    contract: bool = True,
    region: tuple[float, float, float, float] | list[tuple[float, float]] | None = None,
    connectivity: str = "strong",
    contract_drivable: bool = True,
) -> Network:
    """Read a road network from an OpenStreetMap ``.osm.pbf`` extract.

    Args:
        path: The extract.
        contract: Merge chains of degree-two nodes whose links agree on every
            parameter (the default; turn it off only to debug an import).
        region: Cut a study area out of the extract: a rectangle
            ``(west, south, east, north)`` in degrees, or a polygon as a list of
            ``(lon, lat)`` vertices. Nodes outside are dropped and a road that
            crosses the edge ends at its last node inside; a road that leaves
            and re-enters is two roads, never one bridged across the gap.
        connectivity: ``"strong"`` (the default) keeps only the largest
            strongly connected part of the drivable network, so that every node
            a car can use reaches every other: an extract is cut out of a larger
            network, and without this some trips have no route at all. Whole
            links are removed and none is changed; footways and other links a
            car may not use are untouched. What was removed is counted in
            :meth:`Network.report_import` and drawn from
            :meth:`Network.report_dropped`. ``"keep"`` imports every road.
        contract_drivable: Merge car links across nodes that only footways,
            cycleways or pedestrian streets touch. A footway crossing a street
            shares a node with it, and so does every sidewalk that meets a
            driveway; without this (``False``) such a node is a junction and
            splits the street, which in a city that maps its sidewalks cuts a
            third of the car network into pieces a few metres long. With it (the
            default) they merge (by the usual rule: same class, lanes and speed,
            no signal, stop or barrier, nothing a car could turn into), the node
            stays for the footway and the footway links are unchanged. Only
            acts with ``contract``.

    Returns:
        A :class:`Network` with the defaults table's parameters and the street
        geometry kept for maps.

    Raises:
        ValueError: If the file cannot be read, ``region`` or ``connectivity`` is
            not valid, or the result holds no usable road network.
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
    #: The written ``link_bins.parquet``, if the run recorded per-link results.
    link_bins_path: str | None
    #: The run's master seed.
    master_seed: int
    #: The choice model's name.
    choice_model: str
    #: The equilibration strategy's name.
    equilibration: str
    #: The route update's name: ``"none"`` if the route sets stayed as the method made them.
    route_update: str
    #: What each iteration showed, as arrays by name (see ``Run.convergence``).
    convergence: dict[str, npt.NDArray[np.float64] | npt.NDArray[np.uint32]]
    #: Whether the strategy stopped before its most iterations, having converged.
    converged: bool
    #: The run's fingerprint: 16 hex digits of a hash of every input that decides its results.
    fingerprint: str
    total_trips: int
    completed: int
    truncated: int
    no_vehicle_available: int
    no_feasible_path: int
    total_travel_time_s: float
    #: Per-link, per-time-bin results, if the run asked for them.
    link_bins: LinkBins | None
    #: The route sets the trips were routed from.
    route_sets: RouteSets
    #: Which route each trip took, out of how many.
    route_choices: RouteChoices

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
    route_method: str = "penalty",
    route_options: dict[str, float] | None = None,
    master_seed: int = 0,
    choice_model: str | object | None = None,
    choice_options: dict[str, float] | None = None,
    equilibration: str = "none",
    equilibration_options: dict[str, float] | None = None,
    route_update: str = "none",
    route_update_options: dict[str, float] | None = None,
    choice_detour_limit: float | None = None,
    route_cache: bool = False,
) -> RunSummary:
    """Run the whole pipeline and write all four output artifacts.

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
        trips_path: A ``trips.parquet`` path, instead of `trips`.
        persons: An in-memory persons table — rows of
            ``(traveller_id, owns_car, owns_bike, has_transit_pass,
            user_class)``, each field but `traveller_id` optional. At most
            one of `persons`/`persons_path` may be given.
        persons_path: A ``persons.parquet`` path, instead of `persons`.
        class_defaults: ``{class_name: (owns_car, owns_bike,
            has_transit_pass)}`` — what each class owns by default.
        default_weight: How many people a simulated traveller stands for, for
            a trip whose row gives none.
        window_s: Trips still in progress after this second are truncated.
        flow_level: 0 for free flow, or 2, 3, 4 for the link transmission
            model as a point queue, a spatial queue and the full diagram.
        flow_step_s: The loading step in seconds, for levels 2-4.
        link_bin_s: If given, also record per-link results in bins of this
            many seconds.
        route_method: How route sets are generated (see :func:`route_methods`).
        route_options: That method's options.
        master_seed: The scenario's master seed, the one number that starts
            every random stream. Under a sampled choice model it decides who
            takes which route; under the default all-or-nothing model it
            changes only the run's fingerprint.
        choice_model: How each trip picks a route from its pair's set: a name
            from :func:`choice_models` (``None`` is the default,
            ``"deterministic"``), or an object with a ``choose(batch)`` method.
        choice_options: A built-in model's options, numbers by name (for
            ``"logit"``, ``beta_<attribute>`` coefficients).
        equilibration: How choice and loading are repeated: a name from
            :func:`equilibration_strategies` (``"none"``, the default, is one choice
            and one loading; ``"msa"`` iterates).
        equilibration_options: The strategy's options, numbers by name (for
            ``"msa"``: ``iterations``, ``gap_tolerance``, ``gap_sample``, ``cost_bin_s``,
            ``warmup``).
        route_update: How the route sets grow between iterations: a name from
            :func:`route_update_methods` (``"none"``, the default, leaves them as the method made
            them; ``"best_response"`` adds each pair's fastest route at the congested times).
        route_update_options: The update's options, numbers by name (for
            ``"best_response"``: ``searches``, ``max_routes``, ``slack``).
        route_cache: Keep the generated route sets for the next run of this process that asks for
            the same (same network, pairs, method and, for a method that reads it, demand).
        choice_detour_limit: Offer a traveller only the routes whose expected time is within this
            share of the best's (``0`` offers every route, ``None`` the library's default).

    Returns:
        A :class:`RunSummary`.

    Raises:
        ValueError: If the trips/persons arguments are not exactly one form
            each, a given file cannot be read, the demand is empty, or
            writing an output artifact fails.
    """
