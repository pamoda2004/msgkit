//! Properties that hold for every input: no panic, redacting twice changes nothing, findings are ordered and do not
//! overlap, everything that is recognized is gone, and text without digits or `@` is untouched.

use std::sync::OnceLock;

use msgkit_redact::{Mask, RedactConfig, Redactor};
use proptest::prelude::*;

fn redactors() -> &'static [Redactor; 3] {
    static R: OnceLock<[Redactor; 3]> = OnceLock::new();
    R.get_or_init(|| {
        [
            Redactor::new(RedactConfig::default()).unwrap(),
            Redactor::new(
                RedactConfig::default().with_mask(Mask::KeepLast(3)).with_strict_card_prefix(false),
            )
            .unwrap(),
            Redactor::new(
                RedactConfig::default()
                    .with_mask(Mask::Pseudonym(b"property-test-key".to_vec()))
                    .with_names(["Kamal", "\u{0d9a}\u{0db8}\u{0dbd}\u{0dca}"]),
            )
            .unwrap(),
        ]
    })
}

/// Every one of these is found by the default configuration.
const IDENTIFIERS: &[&str] = &[
    "+94700000000",
    "+94 70 000 0000",
    "0094710000000",
    "0700000000",
    "070-000-0000",
    "(011) 000 0000",
    "kamal@example.org",
    "a.b+c@mail.example.com",
    "850010000V",
    "855010000x",
    "198500100000",
    "4111111111111111",
    "5555 5555 5555 4444",
    "3782 822463 10005",
    "GB82 WEST 1234 5698 7654 32",
];

const FILLER: &[&str] = &[
    "hello",
    "price",
    "Rs. 6,500,000",
    "85,000 km",
    "WP CAB-1234",
    "2015",
    "65 lakhs",
    "ok",
    "see you at 10:30",
    "\u{0dbd}\u{0d9a}\u{0dca}\u{0dc2} 65",
    "thanks!",
    "2024-03-01",
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_and_is_idempotent_on_any_text(text in "\\PC{0,300}") {
        for r in redactors() {
            let once = r.redact_text(&text);
            let twice = r.redact_text(&once.text);
            prop_assert_eq!(&twice.text, &once.text);
            prop_assert!(twice.findings.is_empty());
        }
    }

    #[test]
    fn findings_are_ordered_non_overlapping_and_on_character_boundaries(text in "[ -~\u{80}-\u{2ff}\u{0d80}-\u{0dff}]{0,300}") {
        for r in redactors() {
            let findings = r.detect(&text);
            let mut previous_end = 0;
            for f in &findings {
                prop_assert!(f.start >= previous_end && f.start < f.end && f.end <= text.len());
                prop_assert!(text.is_char_boundary(f.start) && text.is_char_boundary(f.end));
                previous_end = f.end;
            }
        }
    }

    #[test]
    fn digit_runs_next_to_words_do_not_break_anything(text in "[a-z0-9 +()./@-]{0,200}") {
        for r in redactors() {
            let once = r.redact_text(&text);
            prop_assert_eq!(r.redact_text(&once.text).text, once.text);
        }
    }

    #[test]
    fn recognized_identifiers_never_survive(
        parts in proptest::collection::vec(
            prop_oneof![
                proptest::sample::select(IDENTIFIERS).prop_map(|s| (s, true)),
                proptest::sample::select(FILLER).prop_map(|s| (s, false)),
            ],
            1..8,
        )
    ) {
        let text = parts.iter().map(|(s, _)| *s).collect::<Vec<_>>().join(" | ");
        let out = redactors()[0].redact_text(&text).text;
        for (part, is_identifier) in &parts {
            if *is_identifier {
                prop_assert!(!out.contains(part), "{} survived in {}", part, out);
            } else {
                prop_assert!(out.contains(part), "{} was changed in {}", part, out);
            }
        }
    }

    #[test]
    fn text_without_digits_or_at_signs_is_untouched(text in "[a-zA-Z .,!?\u{0d80}-\u{0dff}-]{0,200}") {
        // Names are the one exception: the third redactor is configured with some.
        for r in &redactors()[..2] {
            prop_assert_eq!(r.redact_text(&text).text, text.clone());
        }
    }
}
