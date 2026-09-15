//! Records the target triple so that `build_info()` can report which platform
//! a wheel was built for. This is a manifest field (Foundations §6), and the
//! first thing worth knowing about a wheel that misbehaves.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    println!("cargo:rustc-env=OPENMOBISIM_TARGET={target}");
}
