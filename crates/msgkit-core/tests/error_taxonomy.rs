//! Keeps the error taxonomy in one place: `msgkit_core::error::codes`, the table in `docs/ERRORS.md` and the
//! `code` description in `openapi.yaml` must list the same codes, every code must be covered by a test or a
//! conformance case that exists, and no conformance case may expect a code that is not in the taxonomy.

use std::fs;
use std::path::PathBuf;

use msgkit_core::error::codes;
use msgkit_core::Error;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path))
        .unwrap_or_else(|_| panic!("{path} is missing"))
        .replace("\r\n", "\n")
}

/// The rows of the table in docs/ERRORS.md: (code, covered-by entries).
fn documented_codes() -> Vec<(String, Vec<String>)> {
    let doc = read("docs/ERRORS.md");
    let table =
        doc.split("<!-- codes:begin -->").nth(1).and_then(|t| t.split("<!-- codes:end -->").next()).unwrap();
    let ticked =
        |cell: &str| -> Vec<String> { cell.split('`').skip(1).step_by(2).map(str::to_owned).collect() };
    table
        .lines()
        .filter(|l| l.starts_with("| `"))
        .map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 5, "a row of the table has the wrong number of cells: {l}");
            (ticked(cells[0])[0].clone(), ticked(cells[4]))
        })
        .collect()
}

#[test]
fn the_documented_codes_are_exactly_the_codes_in_the_code() {
    let documented: Vec<String> = documented_codes().into_iter().map(|(c, _)| c).collect();
    assert_eq!(documented, codes::ALL, "docs/ERRORS.md and msgkit_core::error::codes::ALL disagree");
}

#[test]
fn codes_are_unique_lowercase_snake_case() {
    let mut seen = std::collections::BTreeSet::new();
    for code in codes::ALL {
        assert!(seen.insert(*code), "{code} is listed twice");
        assert!(code.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{code} is not snake_case");
    }
}

#[test]
fn openapi_lists_the_same_codes() {
    let spec = read("openapi.yaml");
    let line = spec.lines().find(|l| l.contains("Stable error code")).expect("openapi.yaml describes `code`");
    let listed: Vec<&str> = line.split('`').skip(1).step_by(2).collect();
    assert_eq!(listed, codes::ALL, "the `code` description in openapi.yaml disagrees with the taxonomy");
}

#[test]
fn every_code_is_covered_by_a_case_or_a_test_that_exists() {
    for (code, covered_by) in documented_codes() {
        assert!(!covered_by.is_empty(), "{code} has no test or case");
        for entry in covered_by {
            if let Some(case) = entry.strip_prefix("conformance/cases/") {
                let expected: Value =
                    serde_json::from_str(&read(&format!("conformance/cases/{case}/expected.json"))).unwrap();
                assert_eq!(expected["error"]["code"], code.as_str(), "{entry} does not expect {code}");
            } else {
                let (file, test) =
                    entry.split_once("::").unwrap_or_else(|| panic!("{entry}: expected path::test"));
                assert!(read(file).contains(&format!("fn {test}(")), "{entry}: no such test");
            }
        }
    }
}

#[test]
fn conformance_cases_only_expect_codes_from_the_taxonomy() {
    let mut used = std::collections::BTreeSet::new();
    for entry in fs::read_dir(root().join("conformance/cases")).unwrap() {
        let path = entry.unwrap().path().join("expected.json");
        let expected: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        if let Some(code) = expected["error"]["code"].as_str() {
            assert!(
                codes::ALL.contains(&code),
                "a conformance case expects {code}, which is not in the taxonomy"
            );
            used.insert(code.to_owned());
        }
    }
    // The library codes the dispatcher can return are all pinned by conformance cases.
    for code in [codes::INVALID_INPUT, codes::PARSE_ERROR, codes::UNKNOWN_OP] {
        assert!(used.contains(code), "no conformance case returns {code}");
    }
}

#[test]
fn core_errors_map_to_their_documented_codes() {
    let cases = [
        (Error::InvalidInput(String::new()), codes::INVALID_INPUT),
        (Error::Parse(String::new()), codes::PARSE_ERROR),
        (Error::UnknownOp(String::new()), codes::UNKNOWN_OP),
        (Error::Storage(String::new()), codes::STORAGE_ERROR),
    ];
    for (error, code) in cases {
        assert_eq!(error.code(), code);
        assert!(codes::ALL.contains(&error.code()));
    }
}
