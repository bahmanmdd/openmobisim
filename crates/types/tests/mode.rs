//! The four modes: their documented order, names and discriminants.

use openmobisim_types::mode::Mode;

#[test]
fn four_modes_in_the_documented_order_round_trip_their_names_and_discriminants() {
    let expected = [
        (Mode::Car, "car"),
        (Mode::Bike, "bike"),
        (Mode::Walk, "walk"),
        (Mode::Transit, "transit"),
    ];
    assert_eq!(Mode::ALL.len(), expected.len());
    for (i, (mode, name)) in expected.into_iter().enumerate() {
        let raw = u8::try_from(i).unwrap();
        assert_eq!(Mode::ALL[i], mode);
        assert_eq!(mode.as_u8(), raw);
        assert_eq!(Mode::from_u8(raw), Some(mode));
        assert_eq!(mode.to_string(), name);
    }
    assert_eq!(Mode::from_u8(4), None);
    assert_eq!(Mode::from_u8(u8::MAX), None);
}
