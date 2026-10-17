//! Runs the language-independent conformance suite in `conformance/` against the Rust core.
//!
//! Every case directory holds `case.json` (the operation and its input), optional input files and
//! `expected.json` (the canonical response envelope plus one trailing LF). The Python, Node.js, Kotlin and
//! HTTP runners check the same files, so a difference in any binding fails CI.
//!
//! Rust is the reference implementation: after an intentional behaviour change regenerate the expected
//! files with `UPDATE_CONFORMANCE=1 cargo test -p msgkit-core --test conformance`, then review the diff.

use std::fs;
use std::path::{Path, PathBuf};

use msgkit_core::api;
use serde_json::Value;

#[path = "support/canonical.rs"]
mod canonical;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases")
}

/// Builds the operation input: `input` plus the files named in `files` (as strings) and `json_files`.
fn load_input(dir: &Path, spec: &Value) -> Value {
    let mut input = spec["input"].clone();
    let object = input.as_object_mut().expect("`input` is an object");
    if let Some(files) = spec.get("files").and_then(Value::as_object) {
        for (key, name) in files {
            let text = fs::read_to_string(dir.join(name.as_str().unwrap())).unwrap();
            object.insert(key.clone(), Value::String(text));
        }
    }
    if let Some(files) = spec.get("json_files").and_then(Value::as_object) {
        for (key, name) in files {
            let text = fs::read_to_string(dir.join(name.as_str().unwrap())).unwrap();
            object.insert(key.clone(), serde_json::from_str(&text).unwrap());
        }
    }
    input
}

#[test]
fn the_rust_core_matches_every_conformance_case() {
    let update = std::env::var_os("UPDATE_CONFORMANCE").is_some();
    let mut names: Vec<PathBuf> = fs::read_dir(cases_dir())
        .expect("conformance/cases exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    names.sort();
    assert!(names.len() >= 40, "expected the full suite, found {} cases", names.len());

    let mut failures = Vec::new();
    for dir in &names {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let spec: Value = serde_json::from_str(&fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let input = load_input(dir, &spec);
        let response = api::dispatch(spec["op"].as_str().unwrap(), &input.to_string());
        let value: Value = serde_json::from_str(&response).unwrap();
        let mut actual = String::new();
        canonical::canonical(&value, &mut actual);
        actual.push('\n');

        let expected_path = dir.join("expected.json");
        if update {
            fs::write(&expected_path, &actual).unwrap();
            continue;
        }
        match fs::read(&expected_path) {
            Ok(expected) if expected == actual.as_bytes() => {}
            Ok(_) => failures.push(format!("{name}: output differs from expected.json")),
            Err(_) => failures.push(format!("{name}: expected.json is missing")),
        }
    }
    assert!(failures.is_empty(), "conformance failures:\n{}", failures.join("\n"));
}
