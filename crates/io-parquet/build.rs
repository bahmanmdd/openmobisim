//! Records the target triple so the manifest can report which platform a
//! run's artifacts were produced on (Foundations §6) — the same pattern
//! `py-bindings/build.rs` uses for the same reason.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=OPENMOBISIM_TARGET={target}");
}
