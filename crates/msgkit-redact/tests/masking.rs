//! How findings are written back, and which configurations are refused.

use msgkit_redact::{IdPolicy, Mask, RedactConfig, Redactor};

const KEY: &[u8] = b"a-test-key-that-is-not-a-secret";

fn with(mask: Mask) -> Redactor {
    Redactor::new(RedactConfig::default().with_mask(mask)).unwrap()
}

const TEXT: &str = "phone +94700000000, mail kamal@example.org, nic 850010000V, card 4111 1111 1111 1111, account 100200300400, iban GB82 WEST 1234 5698 7654 32";

#[test]
fn the_default_is_a_typed_placeholder_that_keeps_nothing() {
    let out = with(Mask::Placeholder).redact_text(TEXT);
    assert_eq!(
        out.text,
        "phone [PHONE], mail [EMAIL], nic [NIC], card [CARD], account [ACCOUNT], iban [IBAN]"
    );
    for original in ["94700000000", "kamal", "850010000", "4111", "1002003", "GB82", "5698"] {
        assert!(!out.text.contains(original), "{original}");
    }
}

#[test]
fn keep_last_shows_the_end_of_phone_card_account_and_iban_only() {
    let out = with(Mask::KeepLast(4)).redact_text(TEXT);
    assert_eq!(
        out.text,
        "phone [PHONE *0000], mail [EMAIL], nic [NIC], card [CARD *1111], account [ACCOUNT *0400], iban [IBAN *5432]"
    );
    assert_eq!(with(Mask::KeepLast(2)).redact_text("card 4111 1111 1111 1111").text, "card [CARD *11]");
    // Short values are not padded or over-shown.
    assert_eq!(with(Mask::KeepLast(4)).redact_text("call 0700000000").text, "call [PHONE *0000]");
}

#[test]
fn keep_email_domain_keeps_only_the_domain() {
    let r = Redactor::new(RedactConfig::default().with_keep_email_domain(true)).unwrap();
    assert_eq!(r.redact_text("kamal@Example.ORG").text, "[EMAIL @example.org]");
    // The placeholder is not found again.
    let once = r.redact_text("kamal@example.org").text;
    assert_eq!(r.redact_text(&once).text, once);
}

#[test]
fn pseudonyms_are_16_hex_characters_and_stable() {
    let r = with(Mask::Pseudonym(KEY.to_vec()));
    let a = r.redact_text("+94700000000").text;
    let token = a.strip_prefix("[PHONE:").and_then(|t| t.strip_suffix(']')).unwrap();
    assert_eq!(token.len(), 16, "8 bytes");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // The same number written differently, and found again later, gets the same token ...
    for same in ["+94 70 000 0000", "0094700000000", "0700000000", "070-000-0000"] {
        assert_eq!(r.redact_text(same).text, a, "{same}");
    }
    // ... a different number, a different kind and a different key get different tokens.
    assert_ne!(r.redact_text("+94710000000").text, a);
    assert_ne!(with(Mask::Pseudonym(b"another-key".to_vec())).redact_text("+94700000000").text, a);
    let card = r.redact_text("4111111111111111").text;
    assert_eq!(r.redact_text("4111 1111 1111 1111").text, card);
    assert!(card.starts_with("[CARD:"));
    // Emails differ only by case.
    assert_eq!(r.redact_text("Kamal@Example.org").text, r.redact_text("kamal@example.org").text);
}

#[test]
fn a_pseudonym_is_not_found_again_even_when_it_is_all_digits() {
    let r = with(Mask::Pseudonym(KEY.to_vec()));
    // Many values, so some tokens are mostly digits.
    for i in 0..300 {
        let text = format!("call +94{:09}", 700_000_000 + i);
        let once = r.redact_text(&text).text;
        assert_eq!(r.redact_text(&once).text, once, "{once}");
    }
}

#[test]
fn the_key_is_never_printed() {
    let r = with(Mask::Pseudonym(KEY.to_vec()));
    let debug = format!("{r:?} {:?}", r.config());
    assert!(
        !debug.contains("a-test-key") && !debug.contains("97, 45") && debug.contains("<redacted>"),
        "{debug}"
    );
}

#[test]
fn unusable_configurations_are_refused_without_echoing_values() {
    let bad = [
        RedactConfig::default().with_mask(Mask::KeepLast(0)),
        RedactConfig::default().with_mask(Mask::KeepLast(5)),
        RedactConfig::default().with_mask(Mask::Pseudonym(Vec::new())),
        RedactConfig::default().with_mask(Mask::Pseudonym(KEY.to_vec())).with_keep_email_domain(true),
        RedactConfig::default().with_id_policy(IdPolicy::Pseudonymize),
        RedactConfig::default().with_default_country_code(Some("44")),
        RedactConfig::default().with_min_account_digits(5),
        RedactConfig::default().with_min_account_digits(18),
        RedactConfig::default().with_account_keywords([" "]),
        RedactConfig::default().with_extra_area_codes(["123"]),
        RedactConfig::default().with_extra_area_codes(["ab"]),
        RedactConfig::default().with_names(vec!["secret-name-"; 10_001]),
        RedactConfig::default().with_allow_patterns(["("]),
    ];
    for config in bad {
        let e = Redactor::new(config).unwrap_err().to_string();
        assert!(e.starts_with("invalid redaction configuration: "), "{e}");
        assert!(!e.contains("secret-name") && !e.contains("123") && !e.contains("a-test-key"), "{e}");
    }
}
