//! Records the git commit this build was made from, so a run's files, `build_info()` and the
//! references can say which code produced them. Twelve hex digits, or `unknown` when the build
//! has no `.git` (a source archive, an sdist) or no `git` program.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn main() {
    println!("cargo:rerun-if-env-changed=OPENMOBISIM_GIT_SHA");
    println!("cargo:rerun-if-changed=build.rs");
    let sha = std::env::var("OPENMOBISIM_GIT_SHA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=OPENMOBISIM_GIT_SHA={sha}");

    // Rebuild when the checked-out commit changes: HEAD itself, the branch it names, and the
    // packed refs that may hold that branch.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(dir);
        println!("cargo:rerun-if-changed={}", dir.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", dir.join("packed-refs").display());
        if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={}", dir.join(reference).display());
        }
    }
}
