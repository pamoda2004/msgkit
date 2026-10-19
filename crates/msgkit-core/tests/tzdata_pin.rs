//! The bundled IANA time-zone database is pinned. A new version of `jiff` or `jiff-tzdb` can change the UTC
//! timestamps computed for some zones (new rules, corrections), so it is reviewed as a behaviour change
//! (CONTRIBUTING.md, "Time-zone data"). This test fails when the pins or the bundled version change, so an
//! automated dependency update cannot be merged by accident: the person reviewing it updates the constants
//! below together with the CHANGELOG entry that names the old and new tzdata versions and the zones affected.

use std::fs;
use std::path::PathBuf;

/// The tzdata release bundled by `jiff-tzdb` 0.1.9. Change it only together with the CHANGELOG entry.
const EXPECTED_TZDATA: &str = "2026e";

fn read(path: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap().replace("\r\n", "\n")
}

#[test]
fn the_bundled_database_is_the_reviewed_release() {
    assert_eq!(
        msgkit_core::tzdata_version(),
        EXPECTED_TZDATA,
        "the bundled tzdata changed: review it as a behaviour change (CONTRIBUTING.md, \"Time-zone data\"), \
         list the old and new versions and the zones whose rules changed in the CHANGELOG, then update this constant"
    );
}

#[test]
fn the_time_zone_crates_are_pinned_exactly_with_the_bundled_database_only() {
    let manifest = read("Cargo.toml");
    let line = |name: &str| {
        manifest
            .lines()
            .find(|l| l.starts_with(&format!("{name} = ")))
            .unwrap_or_else(|| panic!("{name} is not a dependency"))
    };
    let jiff = line("jiff");
    assert!(jiff.contains("version = \"=0.2.38\""), "jiff must be pinned with `=`: {jiff}");
    assert!(
        jiff.contains("default-features = false"),
        "jiff's default features read OS time-zone files: {jiff}"
    );
    assert!(jiff.contains("\"tzdb-bundle-always\""), "{jiff}");
    for forbidden in ["tz-system", "tzdb-zoneinfo", "tzdb-concatenated", "tzdb-bundle-platform", "\"js\""] {
        assert!(!jiff.contains(forbidden), "{forbidden} must stay off: {jiff}");
    }
    assert!(
        line("jiff-tzdb").contains("\"=0.1.9\""),
        "jiff-tzdb must be pinned with `=`: {}",
        line("jiff-tzdb")
    );
}

#[test]
fn the_update_policy_is_documented() {
    let contributing =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../CONTRIBUTING.md")).unwrap();
    assert!(
        contributing.contains("Time-zone data"),
        "CONTRIBUTING.md must describe the tzdata update policy"
    );
}
