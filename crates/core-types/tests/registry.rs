//! The plugin registry: resolved once, deterministic, and loud about mistakes.

// Exact float comparison is the assertion here, not an oversight: these values
// come out of arithmetic that is required to be reproducible to the bit.
#![allow(clippy::float_cmp, reason = "bit-exact results are the property under test")]

use openmobisim_core_types::registry::{Registry, RegistryError};

/// A stand-in plugin point: something built from a parameter struct.
type Model = Box<dyn Fn(f64) -> f64 + Send + Sync>;

#[derive(Clone, Copy)]
struct Params {
    scale: f64,
}

/// `Model` is a boxed closure and therefore not `Debug`, so `unwrap_err` is
/// unavailable; this is the equivalent.
fn expect_err(result: Result<Model, RegistryError>) -> RegistryError {
    match result {
        Ok(_) => panic!("expected the registry to refuse this"),
        Err(e) => e,
    }
}

fn registry() -> Registry<Params, Model> {
    let mut reg = Registry::new("choice_model");
    reg.register("deterministic", |p: &Params| {
        let k = p.scale;
        Ok(Box::new(move |x: f64| k * x) as Model)
    })
    .unwrap();
    reg.register("mnl", |p: &Params| {
        if p.scale <= 0.0 {
            return Err(RegistryError::construction(
                "choice_model",
                "mnl",
                "scale must be positive",
            ));
        }
        let k = p.scale;
        Ok(Box::new(move |x: f64| (k * x).exp()) as Model)
    })
    .unwrap();
    reg
}

#[test]
fn resolves_to_a_concrete_callable() {
    let built = registry().resolve("deterministic", &Params { scale: 3.0 }).unwrap();
    assert_eq!(built(2.0), 6.0);
}

#[test]
fn names_are_sorted_and_independent_of_registration_order() {
    let mut a: Registry<Params, Model> = Registry::new("p");
    a.register("zulu", |_| Ok(Box::new(|x| x) as Model)).unwrap();
    a.register("alpha", |_| Ok(Box::new(|x| x) as Model)).unwrap();

    let mut b: Registry<Params, Model> = Registry::new("p");
    b.register("alpha", |_| Ok(Box::new(|x| x) as Model)).unwrap();
    b.register("zulu", |_| Ok(Box::new(|x| x) as Model)).unwrap();

    assert_eq!(a.names().collect::<Vec<_>>(), ["alpha", "zulu"]);
    assert_eq!(a.names().collect::<Vec<_>>(), b.names().collect::<Vec<_>>());
}

#[test]
fn an_unknown_name_lists_the_alternatives() {
    let err = expect_err(registry().resolve("nested_logit", &Params { scale: 1.0 }));
    match &err {
        RegistryError::UnknownName { point, requested, available } => {
            assert_eq!(*point, "choice_model");
            assert_eq!(requested, "nested_logit");
            assert_eq!(available, &["deterministic".to_owned(), "mnl".to_owned()]);
        }
        other => panic!("wrong error: {other:?}"),
    }
    let message = err.to_string();
    assert!(message.contains("nested_logit"), "{message}");
    assert!(message.contains("deterministic"), "{message}");
}

#[test]
fn registering_a_name_twice_is_refused() {
    let mut reg = registry();
    let err = reg.register("mnl", |_| Ok(Box::new(|x| x) as Model)).unwrap_err();
    assert!(matches!(err, RegistryError::DuplicateName { .. }), "{err:?}");
}

#[test]
fn a_constructor_may_reject_its_parameters_at_build_time() {
    let err = expect_err(registry().resolve("mnl", &Params { scale: -1.0 }));
    assert!(matches!(err, RegistryError::Construction { .. }), "{err:?}");
    assert!(err.to_string().contains("scale must be positive"));
}

#[test]
fn an_empty_registry_says_so() {
    let reg: Registry<Params, Model> = Registry::new("fleet_policy");
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
    assert!(!reg.contains("anything"));
    let err = expect_err(reg.resolve("anything", &Params { scale: 1.0 }));
    assert!(err.to_string().contains("none registered"), "{err}");
}
