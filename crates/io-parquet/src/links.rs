//! `links.parquet`: the last loading's table, one row per (link, bin of entry) that someone
//! entered, in link id order (the external ids sorted bytewise) then bin. A crossing is filed
//! under the bin its vehicle entered the link in, with its exact exit, even past the window's
//! end (the loading crate's `LinkBins`).
//!
//! | Column | Arrow type | Null | Meaning |
//! |---|---|---|---|
//! | `link_id` | Utf8 | no | the link's external id |
//! | `bin_start_s` | UInt32 | no | the bin's first second, a second of the clock |
//! | `cars` | UInt32 | no | cars that entered the link in the bin |
//! | `car_time_s` | Float64 | yes | their crossing times (exit less entry) summed, seconds; null when no car |
//! | `first_exit_s` | Float64 | yes | when the first of them to enter left, seconds of the clock; null when no car |
//! | `last_exit_s` | Float64 | yes | when the last of them to enter left, seconds of the clock; null when no car |
//! | `bikes` | UInt32 | no | bikes that entered the link in the bin |
//! | `walkers` | UInt32 | no | walkers that entered the link in the bin |

use std::sync::Arc;

use arrow_array::{Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_graph::network::Network;
use openmobisim_loading::link_bins::LinkBins;
use openmobisim_types::ids::EntityId;
use openmobisim_types::units::Duration;

use crate::{WriteError, column};

/// The links table as one batch; `network` gives the external ids. The loading's exits are
/// seconds after the window's start: the window's start is added to put them on the clock.
pub(crate) fn batch(links: &LinkBins, network: &Network) -> Result<RecordBatch, WriteError> {
    let cars = links.cars();
    let window = links.window();
    let start = f64::from(window.origin().get());
    // `offset` 0 for a duration, the window's start for an instant; null where no car.
    let car_column = |v: &[Duration], offset: f64| {
        let values = v.iter().zip(cars).map(|(x, &n)| (n > 0).then_some(offset + x.get()));
        column(values.collect::<Float64Array>())
    };
    let schema = Schema::new(vec![
        Field::new("link_id", DataType::Utf8, false),
        Field::new("bin_start_s", DataType::UInt32, false),
        Field::new("cars", DataType::UInt32, false),
        Field::new("car_time_s", DataType::Float64, true),
        Field::new("first_exit_s", DataType::Float64, true),
        Field::new("last_exit_s", DataType::Float64, true),
        Field::new("bikes", DataType::UInt32, false),
        Field::new("walkers", DataType::UInt32, false),
    ]);
    let columns = vec![
        column(StringArray::from_iter_values(
            links.links().iter().map(|l| network.link_ids().external(l.raw())),
        )),
        column(UInt32Array::from_iter_values(
            links.bins().iter().map(|&b| window.step_start(b).get()),
        )),
        column(UInt32Array::from(cars.to_vec())),
        car_column(links.car_time(), 0.0),
        car_column(links.first_exit(), start),
        car_column(links.last_exit(), start),
        column(UInt32Array::from(links.bikes().to_vec())),
        column(UInt32Array::from(links.walkers().to_vec())),
    ];
    Ok(RecordBatch::try_new(Arc::new(schema), columns)?)
}
