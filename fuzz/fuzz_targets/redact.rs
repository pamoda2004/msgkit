#![no_main]
//! Feeds arbitrary text to the redactor: it must not panic, redacting twice must change nothing, and the findings
//! must be ordered, disjoint and on character boundaries. A crash input belongs in `fuzz/corpus/redact`.

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use msgkit_redact::{Mask, RedactConfig, Redactor};

fn redactors() -> &'static [Redactor; 2] {
    static R: OnceLock<[Redactor; 2]> = OnceLock::new();
    R.get_or_init(|| {
        [
            Redactor::new(RedactConfig::default().with_names(["Kamal"])).unwrap(),
            Redactor::new(RedactConfig::default().with_mask(Mask::Pseudonym(b"fuzz".to_vec())).with_strict_card_prefix(false))
                .unwrap(),
        ]
    })
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for r in redactors() {
        let once = r.redact_text(&text);
        assert_eq!(r.redact_text(&once.text).text, once.text);
        let mut end = 0;
        for f in &once.findings {
            assert!(f.start >= end && f.start < f.end && f.end <= text.len());
            assert!(text.is_char_boundary(f.start) && text.is_char_boundary(f.end));
            end = f.end;
        }
    }
});
