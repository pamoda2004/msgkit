//! `msgkit-core` stays pure: it reads no files, opens no sockets, reads no environment and never asks for the
//! time. In particular, time zones come from the database bundled in the binary, never from the operating
//! system's time-zone files, and nothing depends on the system clock. If one of these tests fails, someone
//! added an impure dependency or call: remove it rather than relaxing the test.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use msgkit_core::whatsapp_export::{try_parse, DateOrder, WhatsAppExportOptions};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Things that make code depend on the machine it runs on.
const FORBIDDEN: &[(&str, &str)] = &[
    ("::now(", "reads the system clock"),
    ("SystemTime", "reads the system clock"),
    ("Instant", "reads a clock"),
    ("chrono::Local", "uses the system time zone"),
    ("Local::", "uses the system time zone"),
    ("TimeZone::system", "uses the system time zone"),
    ("tz::system", "uses the system time zone"),
    ("std::fs", "reads or writes files"),
    ("std::env", "reads the environment"),
    ("std::net", "opens network connections"),
    ("std::process", "starts processes"),
    ("TZDIR", "points at operating-system time-zone files"),
    ("/usr/share/zoneinfo", "reads operating-system time-zone files"),
];

#[test]
fn the_library_source_has_no_clock_file_environment_or_network_access() {
    let mut files = Vec::new();
    rust_files(&root().join("crates/msgkit-core/src"), &mut files);
    assert!(files.len() >= 10, "found only {} source files", files.len());
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for (needle, why) in FORBIDDEN {
                if code.contains(needle) {
                    violations.push(format!("{}:{}: `{needle}` {why}", file.display(), number + 1));
                }
            }
        }
    }
    assert!(violations.is_empty(), "msgkit-core must stay pure:\n{}", violations.join("\n"));
}

#[test]
fn only_the_bundled_time_zone_database_is_compiled_in() {
    // `cargo tree` resolves only the packages this platform builds, so it needs nothing that has not been
    // downloaded already (`cargo metadata` would fetch every optional dependency, which fails offline).
    let output = Command::new(env!("CARGO"))
        .args(["tree", "--workspace", "--invert", "jiff", "--prefix", "none", "--format", "{p}|{f}"])
        .args(["--locked", "--offline"])
        .current_dir(root())
        .output()
        .expect("cargo tree");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let tree = String::from_utf8(output.stdout).unwrap();
    let roots: Vec<&str> = tree.lines().filter(|l| l.starts_with("jiff v")).collect();
    assert_eq!(
        roots.len(),
        1,
        "expected exactly one jiff in the dependency graph:
{tree}"
    );
    let features = roots[0].split('|').nth(1).unwrap();
    // `tz-system` (OS API), `tzdb-zoneinfo` and `tzdb-concatenated` (OS files), `tzdb-bundle-platform` (OS
    // dependent) and `js` must stay off, in every crate of the workspace.
    assert_eq!(features, "alloc,std,tzdb-bundle-always", "jiff must use only the bundled time-zone database");
}

#[test]
fn time_zone_results_do_not_depend_on_operating_system_files_or_the_tz_variable() {
    // A decoy time-zone directory with a broken "Asia/Colombo": code that consulted it would fail or differ.
    let decoy = std::env::temp_dir().join(format!("msgkit-decoy-zoneinfo-{}", std::process::id()));
    fs::create_dir_all(decoy.join("Asia")).unwrap();
    fs::write(decoy.join("Asia/Colombo"), b"this is not a TZif file").unwrap();

    let chat = "01/06/2024, 12:00 - Alex: hello\n";
    let opts =
        WhatsAppExportOptions::default().with_date_order(DateOrder::Dmy).with_time_zone("Asia/Colombo");
    let before = try_parse(chat, &opts).unwrap();

    std::env::set_var("TZDIR", &decoy);
    std::env::set_var("TZ", "Pacific/Apia");
    let after = try_parse(chat, &opts).unwrap();
    std::env::remove_var("TZDIR");
    std::env::remove_var("TZ");
    let _ = fs::remove_dir_all(&decoy);

    assert_eq!(before, after);
    assert_eq!(after.events[0].timestamp().format("%Y-%m-%dT%H:%MZ").to_string(), "2024-06-01T06:30Z");
}
