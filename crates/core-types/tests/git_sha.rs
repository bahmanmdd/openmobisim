//! The git commit recorded at build time: twelve hex digits, or `unknown` without a repository.

use openmobisim_core_types::GIT_SHA;

#[test]
fn the_git_sha_is_twelve_hex_digits_or_unknown() {
    let hex = GIT_SHA.len() == 12 && GIT_SHA.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    assert!(hex || GIT_SHA == "unknown", "got {GIT_SHA:?}");
}
