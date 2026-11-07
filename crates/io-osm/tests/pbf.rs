//! The `.osm.pbf` decoder's refusals: a file that is not there, and one that is not a PBF.
//! (Its reading of a real file is `tests/delft.rs`.)

use std::path::Path;

use openmobisim_io_osm::{OsmError, read_pbf};

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let err = read_pbf(Path::new("tests/fixtures/no-such-file.osm.pbf")).unwrap_err();
    assert!(matches!(err, OsmError::Io { .. }), "{err:?}");
    assert!(err.to_string().contains("no-such-file.osm.pbf"), "{err}");
}

#[test]
fn a_file_that_is_not_a_pbf_is_an_error() {
    let err = read_pbf(Path::new("tests/delft_ways.py")).unwrap_err();
    assert!(matches!(err, OsmError::Decode { .. }), "{err:?}");
}
