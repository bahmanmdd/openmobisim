//! The four modes: their documented order, names and discriminants.

use openmobisim_types::mode::{Mode, ModeSet};

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

#[test]
fn a_mode_set_holds_exactly_the_modes_put_in_it() {
    // By hand: {car, walk} holds car and walk, not bike or transit; listing walk twice changes
    // nothing; the empty set holds nothing; iteration follows the discriminants 0 to 3.
    let set = ModeSet::of(&[Mode::Walk, Mode::Car, Mode::Walk]);
    assert!(set.contains(Mode::Car) && set.contains(Mode::Walk));
    assert!(!set.contains(Mode::Bike) && !set.contains(Mode::Transit));
    assert_eq!(set.iter().collect::<Vec<_>>(), [Mode::Car, Mode::Walk]);
    assert_eq!(set, ModeSet::EMPTY.with(Mode::Car).with(Mode::Walk));
    assert!(!set.is_empty());

    assert!(ModeSet::EMPTY.is_empty());
    assert_eq!(ModeSet::EMPTY.iter().count(), 0);
    for mode in Mode::ALL {
        assert!(!ModeSet::EMPTY.contains(mode));
        assert_eq!(ModeSet::of(&[mode]).iter().collect::<Vec<_>>(), [mode]);
    }
    assert_eq!(ModeSet::of(&Mode::ALL).iter().collect::<Vec<_>>(), Mode::ALL);
    assert_eq!(format!("{set:?}"), "{Car, Walk}");
}
