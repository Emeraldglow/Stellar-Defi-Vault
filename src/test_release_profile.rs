#![cfg(test)]

//! Guards the release profile required by issue #657.
//!
//! CI builds the deployable Wasm with `--release`; silently changing one of
//! these settings would remove overflow checks or reintroduce unwinding into
//! the Soroban artifact. This test reads the manifest at compile time so the
//! profile cannot drift without a test failure.

extern crate std;

use std::string::String;

fn release_profile() -> String {
    let manifest = include_str!("../Cargo.toml");
    let release = manifest
        .split("[profile.release]")
        .nth(1)
        .expect("Cargo.toml must define [profile.release]");
    release
        .split("\n[profile.")
        .next()
        .expect("release profile section")
        .into()
}

#[test]
fn release_profile_enables_overflow_checks() {
    assert!(
        release_profile().contains("overflow-checks = true"),
        "release builds must keep overflow checks enabled"
    );
}

#[test]
fn release_profile_is_optimised_for_soroban() {
    let profile = release_profile();
    for expected in [
        "opt-level = \"z\"",
        "debug = 0",
        "strip = \"symbols\"",
        "debug-assertions = false",
        "panic = \"abort\"",
        "codegen-units = 1",
        "lto = true",
    ] {
        assert!(
            profile.contains(expected),
            "release profile is missing `{expected}`"
        );
    }
}

#[test]
fn release_with_logs_inherits_release() {
    let manifest = include_str!("../Cargo.toml");
    let logs = manifest
        .split("[profile.release-with-logs]")
        .nth(1)
        .expect("Cargo.toml must define [profile.release-with-logs]");
    assert!(logs.contains("inherits = \"release\""));
    assert!(logs.contains("debug-assertions = true"));
}
