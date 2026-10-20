//! What is found, and what each validator rejects. All numbers here are invented or are the card numbers payment
//! providers publish for testing.

use msgkit_redact::{Confidence, Kind, RedactConfig, Redactor};

fn redactor() -> Redactor {
    Redactor::new(RedactConfig::default()).unwrap()
}

fn kinds(text: &str) -> Vec<Kind> {
    redactor().detect(text).into_iter().map(|f| f.kind).collect()
}

/// A number with the given prefix and length whose last digit makes it pass the Luhn check.
fn luhn_complete(prefix: &str, len: usize) -> String {
    let mut digits: Vec<u32> = prefix.chars().map(|c| c.to_digit(10).unwrap()).collect();
    while digits.len() < len - 1 {
        digits.push(0);
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 0 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                *d
            }
        })
        .sum();
    digits.push((10 - sum % 10) % 10);
    digits.iter().map(|d| char::from_digit(*d, 10).unwrap()).collect()
}

#[test]
fn emails() {
    let r = redactor();
    assert_eq!(r.redact_text("write to kamal@example.org today").text, "write to [EMAIL] today");
    assert_eq!(r.redact_text("<a.b+tag@mail.example.com>, ok").text, "<[EMAIL]>, ok");
    assert_eq!(r.redact_text("sent from kamal@example.org.").text, "sent from [EMAIL].");
    assert_eq!(r.redact_text("a@b.c").text, "a@b.c", "a one-letter top-level domain is not an address");
    assert_eq!(r.redact_text("@handle and user@ and @").text, "@handle and user@ and @");
}

#[test]
fn international_and_national_phone_numbers() {
    let r = redactor();
    for text in [
        "+94700000000",
        "+94 70 000 0000",
        "+94-70-000-0000",
        "0094700000000",
        "0700000000",
        "070 000 0000",
        "070-000-0000",
        "(011) 000 0000",
        "011 0000000",
        "+44 20 0000 0000",
        "+1 202 555 0100",
    ] {
        let out = r.redact_text(&format!("call {text} now"));
        assert_eq!(out.text, "call [PHONE] now", "{text}");
    }
}

#[test]
fn a_sri_lankan_number_ends_after_nine_national_digits() {
    let r = redactor();
    assert_eq!(r.redact_text("+94 70 000 0000 5 pm").text, "[PHONE] 5 pm");
    assert_eq!(r.redact_text("+94700000000 and +94710000000").text, "[PHONE] and [PHONE]");
}

#[test]
fn national_numbers_need_a_known_code_and_the_country_setting() {
    // The mobile level 07x and the geographic codes of the 2009 plan are accepted ...
    for code in ["070", "071", "072", "074", "075", "076", "077", "078", "079", "011", "021", "081", "091"] {
        assert_eq!(kinds(&format!("{code} 000 0000")), [Kind::Phone], "{code}");
    }
    // ... a code that is not allocated is not a phone number (012 is reserved, 010 and 099 do not exist).
    for code in ["010", "012", "099", "022", "060"] {
        assert_eq!(kinds(&format!("{code} 000 0000")), vec![], "{code}");
    }
    // Further codes are configurable.
    let extra = Redactor::new(RedactConfig::default().with_extra_area_codes(["12"])).unwrap();
    assert_eq!(extra.detect("012 000 0000").len(), 1);
    // Without a country setting only numbers written with + or 00 are found.
    let none = Redactor::new(RedactConfig::default().with_default_country_code(None)).unwrap();
    assert_eq!(none.redact_text("0700000000 and +94700000000").text, "0700000000 and [PHONE]");
    // A longer digit run is not a phone number.
    assert_eq!(kinds("07000000001"), vec![]);
    assert_eq!(kinds("x0700000000"), vec![]);
}

#[test]
fn old_nic_numbers() {
    let r = redactor();
    for nic in ["850010000V", "850010000v", "850010000X", "855010000V", "853660000V", "858660000V"] {
        let out = r.redact_text(&format!("NIC {nic}."));
        assert_eq!(out.text, "NIC [NIC].", "{nic}");
        assert_eq!(out.findings[0].confidence, Confidence::High);
    }
    // Day numbers 000, 367 to 500 and 867 up are not valid.
    for nic in ["850000000V", "853670000V", "854990000V", "855000000V", "858670000V", "859990000V"] {
        assert_eq!(kinds(nic), vec![], "{nic}");
    }
    // The letter is required, and the number must stand alone.
    assert_eq!(kinds("850010000"), vec![]);
    assert_eq!(kinds("A850010000V"), vec![]);
}

#[test]
fn new_nic_numbers() {
    let r = redactor();
    for nic in ["198500100000", "200010000000", "200050100000"] {
        let out = r.redact_text(&format!("id {nic} ok"));
        assert_eq!(out.text, "id [NIC] ok", "{nic}");
        assert_eq!(out.findings[0].confidence, Confidence::Medium);
    }
    // Birth year after `max_birth_year`, a day that does not exist, or the wrong length.
    for nic in ["202500100000", "198500000000", "198536700000", "19850010000", "1985001000000"] {
        assert_eq!(kinds(nic), vec![], "{nic}");
    }
    let later = Redactor::new(RedactConfig::default().with_max_birth_year(2030)).unwrap();
    assert_eq!(later.detect("202500100000").len(), 1);
}

#[test]
fn cards_need_luhn_and_a_known_network() {
    let r = redactor();
    for card in [
        "4111111111111111",
        "4111 1111 1111 1111",
        "4111-1111-1111-1111",
        "5555555555554444",
        "378282246310005",
        "3782 822463 10005",
        "6011111111111117",
        "3530111333300000",
        "30569309025904",
        "3056 9309 0259 04",
    ] {
        let out = r.redact_text(&format!("card {card} end"));
        assert_eq!(out.text, "card [CARD] end", "{card}");
        assert_eq!(out.findings[0].confidence, Confidence::High);
    }
    assert_eq!(kinds("4111111111111112"), vec![], "fails Luhn");
    assert_eq!(kinds("4111 1111 1111 111"), vec![], "wrong length for the network");
    // Luhn-valid with a prefix of no known network: found only when strict_card_prefix is off.
    let unknown = luhn_complete("9999", 16);
    assert_eq!(kinds(&unknown), vec![], "{unknown}");
    let loose = Redactor::new(RedactConfig::default().with_strict_card_prefix(false)).unwrap();
    let found = loose.detect(&unknown);
    assert_eq!((found.len(), found[0].kind, found[0].confidence), (1, Kind::Card, Confidence::Medium));
}

#[test]
fn ibans_need_a_known_country_and_mod_97() {
    let r = redactor();
    assert_eq!(r.redact_text("pay GB82 WEST 1234 5698 7654 32 today").text, "pay [IBAN] today");
    assert_eq!(r.redact_text("DE89370400440532013000").text, "[IBAN]");
    assert_eq!(
        r.redact_text("GB82 WEST 1234 5698 7654 32 ACCOUNT").text,
        "[IBAN] ACCOUNT",
        "stops at the length"
    );
    assert_eq!(kinds("GB82 WEST 1234 5698 7654 33"), vec![], "bad check digits");
    assert_eq!(kinds("XX82 WEST 1234 5698 7654 32"), vec![], "unknown country");
    assert_eq!(kinds("gb82 west 1234 5698 7654 32"), vec![], "lower case is not an IBAN here");
}

#[test]
fn account_numbers_need_a_keyword_close_before_them() {
    let r = redactor();
    for text in [
        "Account number: 100200300400",
        "A/C 1002 0030 0400 please",
        "acct no 100200300400",
        "Acc. No. 100200300400",
        "my savings account is 10020030040",
    ] {
        let out = r.redact_text(text);
        assert!(out.text.contains("[ACCOUNT]"), "{text} -> {}", out.text);
        assert_eq!(out.findings[0].confidence, Confidence::Medium);
    }
    assert_eq!(kinds("order 100200300400"), vec![], "no keyword");
    assert_eq!(kinds("account 1234567"), vec![], "fewer than 8 digits");
    assert_eq!(kinds("accountant 100200300400"), vec![], "the keyword is part of a longer word");
    assert_eq!(kinds("Account: Rs. 12500000"), vec![], "a price");
    assert_eq!(kinds("bank 100200300400 km"), vec![], "a quantity");
    // Beyond the window the keyword no longer applies.
    let far = format!("account{}100200300400", " and some other words in between ".repeat(5));
    assert_eq!(kinds(&far), vec![]);
    // Custom keywords, and fewer required digits.
    let custom =
        Redactor::new(RedactConfig::default().with_account_keywords(["khata"]).with_min_account_digits(6))
            .unwrap();
    assert_eq!(custom.redact_text("khata 123456").text, "khata [ACCOUNT]");
    assert_eq!(custom.detect("account 123456"), vec![]);
}

#[test]
fn a_keyword_does_not_reach_past_the_placeholder_of_its_own_number() {
    // Found by the fuzzer: masking the account number shortens the text, and a second pass must not then reach
    // the digits that follow.
    let r = redactor();
    let text = "Account number: 100200300400, IBAN GB82 WEST 1234 5698 7654 31";
    let once = r.redact_text(text).text;
    assert_eq!(once, "Account number: [ACCOUNT], IBAN GB82 WEST 1234 5698 7654 31");
    assert_eq!(r.redact_text(&once).text, once);
}

#[test]
fn masking_a_value_can_expose_its_glued_neighbour_and_it_is_masked_on_a_later_pass() {
    // No word boundary separates the number from the IBAN until the number is masked.
    let r = redactor();
    let out = r.redact_text("+94700000000GB82 WEST 1234 5698 7654 32|");
    assert_eq!(out.text, "[PHONE][IBAN]|");
    assert_eq!((out.findings.len(), out.later.clone()), (1, vec![Kind::Iban]));
    assert_eq!(out.counts()[&Kind::Iban], 1);
    assert_eq!(r.redact_text(&out.text).text, out.text);
}

#[test]
fn a_card_is_not_also_reported_as_an_account_or_a_phone() {
    let out = redactor().redact_text("bank card 4111 1111 1111 1111");
    assert_eq!(out.text, "bank card [CARD]");
    assert_eq!(out.findings.len(), 1);
}

#[test]
fn overlaps_go_to_the_more_specific_kind() {
    let r = redactor();
    // An email containing a name from the list is one email.
    let named = Redactor::new(RedactConfig::default().with_names(["Kamal"])).unwrap();
    assert_eq!(named.redact_text("Kamal wrote from kamal@example.org").text, "[NAME] wrote from [EMAIL]");
    // A phone number next to a keyword is a phone number.
    assert_eq!(r.redact_text("bank 0700000000").text, "bank [PHONE]");
    // Every character is masked once.
    let out = r.redact_text("+94700000000 850010000V kamal@example.org 4111111111111111");
    assert_eq!(out.text, "[PHONE] [NIC] [EMAIL] [CARD]");
}

#[test]
fn kinds_can_be_switched_off() {
    let only_cards = Redactor::new(RedactConfig::default().with_kinds([Kind::Card])).unwrap();
    assert_eq!(only_cards.redact_text("+94700000000 4111111111111111").text, "+94700000000 [CARD]");
}

#[test]
fn min_confidence_high_keeps_only_validated_findings() {
    let high = Redactor::new(RedactConfig::default().with_min_confidence(Confidence::High)).unwrap();
    // International phone and a card are validated; a national number, a bare new NIC and an account are shape only.
    let text = "+94700000000 0700000000 198500100000 account 100200300400 4111111111111111";
    assert_eq!(high.redact_text(text).text, "[PHONE] 0700000000 198500100000 account 100200300400 [CARD]");
}

#[test]
fn allow_lists_keep_chosen_values() {
    let r = Redactor::new(
        RedactConfig::default()
            .with_allow_exact(["+94700000000"])
            .with_allow_patterns([r"0110000\d{3}", r"support@.*"]),
    )
    .unwrap();
    let out = r.redact_text(
        "support +94700000000, desk 011 0000000?, 0110000000 and 0710000000, support@example.org",
    );
    assert_eq!(out.text, "support +94700000000, desk [PHONE]?, 0110000000 and [PHONE], support@example.org");
}

#[test]
fn findings_hold_positions_and_never_the_text() {
    let text = "ü call +94700000000 ü";
    let findings = redactor().detect(text);
    assert_eq!(findings.len(), 1);
    assert_eq!(&text[findings[0].start..findings[0].end], "+94700000000");
    assert!(!format!("{findings:?}").contains("94700000000"));
}

#[test]
fn names_are_whole_words_without_regard_to_case() {
    let r = Redactor::new(RedactConfig::default().with_names(["Kamal Perera", "Nimal"])).unwrap();
    assert_eq!(
        r.redact_text("kamal perera and NIMAL, not Nimalka or Animal").text,
        "[NAME] and [NAME], not Nimalka or Animal"
    );
    // Sinhala names: a combining mark counts as part of the word.
    let si = Redactor::new(RedactConfig::default().with_names(["\u{0d9a}\u{0db8}\u{0dbd}\u{0dca}"])).unwrap();
    assert_eq!(
        si.redact_text("\u{0d9a}\u{0db8}\u{0dbd}\u{0dca} \u{0d9a}\u{0db8}\u{0dbd}\u{0dca}\u{0dc3}").text,
        "[NAME] \u{0d9a}\u{0db8}\u{0dbd}\u{0dca}\u{0dc3}"
    );
}
