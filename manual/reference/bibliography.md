# Bibliography

% src: PARAMETERS.md (the Source column); README.md ("The ten tests", "Cite"); the contracts crates/choice, crates/io-gtfs, crates/io-osm, crates/loading, crates/loop, crates/search and crates/transit (README.md of each)

Every work that the modules' contracts, the table of parameters and the list of the ten tests cite, once, in
alphabetical order. Under each work: the pages of the manual it supports, and the parameters whose default or meaning
it is the source of ([Parameters](parameters.md)). A reference marked *as cited* is given as far as the project's own
documents give it.

Ben-Akiva, M. and Bierlaire, M. (1999). Discrete choice methods and their applications to short term travel decisions. In R. W. Hall (ed.), *Handbook of Transportation Science*, pp. 5–33. Kluwer Academic Publishers.
: The path-size logit. Supports [Choice](../concepts/choice.md) and [The iteration](../concepts/iteration.md); the
  parameter `coefficients` (the coefficient on the logarithm of the path size).

Ben-Akiva, M. and Lerman, S. R. (1985). *Discrete Choice Analysis: Theory and Application to Travel Demand*. MIT Press, Cambridge, MA.
: The nested logit with one nest scale (chapter 10). Supports [Choice](../concepts/choice.md); the parameter
  `nest_scale`.

Braess, D. (1968). Über ein Paradoxon aus der Verkehrsplanung. *Unternehmensforschung* 12, 258–268.
: The network of test 4. Supports [Validation](../concepts/validation.md).

Cerema (2016). *Évaluation de la capacité aux feux, retour d'observations en France*. October 2016. *As cited.*
: The capacity of a signalised approach lane, the saturation flow times the green time over the cycle. Supports
  [Networks](../concepts/networks.md); the parameter `osm_class_defaults`.

Cerema. *Fondamentaux de la conception routière*, rapport de synthèse. *As cited; the sources give no year.*
: The capacity per lane of trunk roads and of ramps. Supports [Networks](../concepts/networks.md); the parameter
  `osm_class_defaults`.

Cominetti, R., Correa, J. and Olver, N. (2022). *As cited: the sources give the authors and the year.*
: The steady state of a network of queues with capacities, the answer of test 4. Supports
  [Validation](../concepts/validation.md).

Daganzo, C. F. (1995). The cell transmission model, part II: network traffic. *Transportation Research Part B* 29(2), 79–93.
: The merge rule of test 2. The sources name "Daganzo's merge rule" without a work; this is the paper that states it.
  Supports [Validation](../concepts/validation.md) and [The loading](../concepts/loading.md).

de Hartog, J. J., Boogaard, H., Nijland, H. and Hoek, G. (2011). *Environmental Health Perspectives*. *As cited: the sources give the authors, the journal and the year.*
: The urban cycling speed in the Netherlands, stops included. Supports the parameter `bike_speed`.

Delling, D., Pajor, T. and Werneck, R. F. (2015). Round-based public transit routing. *Transportation Science* 49(3), 591–604.
: The journey search RAPTOR, and the minimum change time. Supports [Transit](../concepts/transit.md); the parameters
  `boarding_slack` and `max_rides`.

El-Geneidy, A., Grimsrud, M., Wasfi, R., Tétreault, P. and Surprenant-Legault, J. (2014). New evidence on walking distances to transit stops: identifying redundancies and gaps using variable service areas. *Transportation* 41, 193–210.
: The walking distances to stops. Supports [Demand](../concepts/demand.md); the parameter `stop_walk_max`.

General Transit Feed Specification. *GTFS Schedule Reference*. <https://gtfs.org/documentation/schedule/reference/>
: The files and fields of a feed. Supports [Transit](../concepts/transit.md) and
  [Input tables and files](inputs.md#a-gtfs-feed); the parameter `boarding_slack` (`min_transfer_time` of
  `transfers.txt`).

Guerra, E., Cervero, R. and Tischler, D. (2012). Half-mile circle: does it best represent transit station catchments? *Transportation Research Record* 2276, 101–109.
: The quarter-mile walking standard of transit. Supports the parameter `transfer_walk_max`.

Hart, P. E., Nilsson, N. J. and Raphael, B. (1968). A formal basis for the heuristic determination of minimum cost paths. *IEEE Transactions on Systems Science and Cybernetics* 4(2), 100–107.
: The goal-directed search A\*. Supports [Search](../concepts/search.md).

ICTAVRU: *Instruction sur les conditions techniques d'aménagement des voies rapides urbaines*, the French instruction for urban expressways. *As cited; the sources give no publisher or year.*
: The capacity per lane of motorways. Supports [Networks](../concepts/networks.md); the parameter
  `osm_class_defaults`.

Kaufman, D. E. and Smith, R. L. (1993). Fastest paths in time-dependent networks for intelligent vehicle-highway systems application. *IVHS Journal* 1(1), 1–11.
: The exactness of a label-setting search on link times that are first-in-first-out. Supports
  [Search](../concepts/search.md).

KiM Netherlands Institute for Transport Policy Analysis (2023). *Values of time and reliability in passenger transport*. *As cited.*
: The value of time of 10 € per hour. Supports [Choice](../concepts/choice.md); the parameter `coefficients` (the
  cost per euro).

Knoblauch, R. L., Pietrucha, M. T. and Nitzburg, M. (1996). Field studies of pedestrian walking speed and start-up time. *Transportation Research Record* 1538, 27–38.
: Measured mean walking speeds. Supports the parameter `walk_speed`.

Larsson, T. and Patriksson, M. (1995). An augmented Lagrangean dual algorithm for link capacity side constrained traffic assignment problems. *Transportation Research Part B* 29(6), 433–455.
: The equilibrium with link capacities as side constraints, the answer of tests 5 and 6. Supports
  [Validation](../concepts/validation.md).

Madadi, B. (2026). *openmobisim example cases: multimodal networks and synthetic morning demand for four European cities and a synthetic test city* (v2). Zenodo. <https://doi.org/10.5281/zenodo.24585967>
: The published cases. Supports [The cases](cases.md) and [How to cite](citing.md).

Mahmassani, H. S. and Chang, G.-L. (1987). On boundedly rational user equilibrium in transportation systems. *Transportation Science* 21(2), 89–99.
: The band of indifference in the rate at which travellers are offered a new choice under the least-cost choice.
  Supports [The iteration](../concepts/iteration.md).

Nguyen, S. and Dupuis, C. (1984). An efficient method for computing traffic equilibria in networks with asymmetric transportation costs. *Transportation Science* 18(2), 185–202.
: The network of test 5. The sources name the network "Nguyen–Dupuis" without a work; this is the paper it comes
  from. Supports [Validation](../concepts/validation.md).

OpenStreetMap Wiki. *Key:oneway*, *Key:lanes*, *Key:maxspeed* and *Default speed limits*. <https://wiki.openstreetmap.org/>
: The meaning of the tags read from an extract. Supports [Networks](../concepts/networks.md) and
  [Input tables and files](inputs.md#an-openstreetmap-extract).

Osuna, E. E. and Newell, G. F. (1972). Control strategies for an idealized public transportation system. *Transportation Science* 6(1), 52–72.
: The expected wait at a stop under irregular headways, a closed form of test 8. Supports
  [Validation](../concepts/validation.md) and [Transit](../concepts/transit.md).

Robbins, H. and Monro, S. (1951). A stochastic approximation method. *The Annals of Mathematical Statistics* 22(3), 400–407.
: The condition under which the averaging of the iterations settles. Supports
  [The iteration](../concepts/iteration.md); the parameter `step_exponent`.

Sheffi, Y. (1985). *Urban Transportation Networks: Equilibrium Analysis with Mathematical Programming Methods*. Prentice-Hall, Englewood Cliffs, NJ.
: The method of successive averages. Supports [The iteration](../concepts/iteration.md); the parameter
  `step_exponent`.

Smith, M. J. (1984). The stability of a dynamic model of traffic assignment: an application of a method of Lyapunov. *Transportation Science* 18(3), 245–252.
: Proportional switching between routes. Supports [The iteration](../concepts/iteration.md).

Spiess, H. and Florian, M. (1989). Optimal strategies: a new assignment model for transit networks. *Transportation Research Part B* 23(2), 83–102.
: The transit network of test 8. Supports [Validation](../concepts/validation.md).

Tarjan, R. (1972). Depth-first search and linear graph algorithms. *SIAM Journal on Computing* 1(2), 146–160.
: The strongly connected components of each layer of a network read from an extract. Supports
  [Networks](../concepts/networks.md).

Transportation Networks for Research Core Team. *Transportation Networks for Research*. <https://github.com/bstabler/TransportationNetworks>
: The TNTP files of Sioux Falls, the network of test 6. Supports [Validation](../concepts/validation.md).

Transportation Research Board (2022). *Highway Capacity Manual*, 7th edition. *As cited.*
: The base saturation flow per lane. Supports the parameter `capacity_per_lane`.

Vickrey, W. S. (1969). Congestion theory and transport investment. *The American Economic Review* 59(2), 251–260.
: The point queue of the loading, and the bottleneck of tests 1 and 2. Supports [The loading](../concepts/loading.md)
  and [Validation](../concepts/validation.md).

Vincenty, T. (1975). Direct and inverse solutions of geodesics on the ellipsoid with application of nested equations. *Survey Review* 23(176), 88–93.
: The length of a link read from an extract, the sum of the geodesics between its nodes. Supports
  [Networks](../concepts/networks.md).

Wardrop, J. G. (1952). Some theoretical aspects of road traffic research. *Proceedings of the Institution of Civil Engineers* 1(3), 325–378.
: Equal times on the routes in use, the answer of test 3. The sources name Wardrop without a work; this is the paper
  that states the principle. Supports [Validation](../concepts/validation.md).
