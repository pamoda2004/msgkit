//! Replays the committed fuzz seeds (`fuzz/corpus/redact/`) through the same checks the cargo-fuzz target runs, on the
//! stable toolchain and on every platform. When a fuzz run finds a crash, add its input to that directory.

use std::fs;
use std::path::Path;

use msgkit_redact::{Mask, RedactConfig, Redactor};

#[test]
fn the_redact_corpus_holds_the_fuzz_target_invariants() {
    let redactors = [
        Redactor::new(RedactConfig::default().with_names(["Kamal"])).unwrap(),
        Redactor::new(
            RedactConfig::default()
                .with_mask(Mask::Pseudonym(b"fuzz".to_vec()))
                .with_strict_card_prefix(false),
        )
        .unwrap(),
    ];
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/redact");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let data = fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&data);
        for r in &redactors {
            let once = r.redact_text(&text);
            assert_eq!(r.redact_text(&once.text).text, once.text);
            let mut end = 0;
            for f in &once.findings {
                assert!(f.start >= end && f.start < f.end && f.end <= text.len());
                assert!(text.is_char_boundary(f.start) && text.is_char_boundary(f.end));
                end = f.end;
            }
        }
        checked += 1;
    }
    assert!(checked >= 6, "only {checked} seeds were checked");
}
