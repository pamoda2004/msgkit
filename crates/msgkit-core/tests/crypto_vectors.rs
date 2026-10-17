//! Pins the cryptographic behaviour that the `hmac` and `sha2` dependencies provide.
//!
//! The expected values come from RFC 4231 (HMAC-SHA-256) and from an independent implementation
//! (Python's `hashlib`), not from this crate, so a dependency upgrade that changed a result would fail here.

use msgkit_core::id::derive_id;
use msgkit_core::webhook::{sign, verify_signature};

/// RFC 4231 test cases 1, 2 and 6 (6 uses a key longer than the block size) plus the empty key and message.
const HMAC_VECTORS: [(&[u8], &[u8], &str); 4] = [
    (&[0x0b; 20], b"Hi There", "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"),
    (
        b"Jefe",
        b"what do ya want for nothing?",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    ),
    (
        &[0xaa; 131],
        b"Test Using Larger Than Block-Size Key - Hash Key First",
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
    ),
    (b"", b"", "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"),
];

#[test]
fn signing_matches_the_rfc_4231_vectors() {
    for (key, message, expected) in HMAC_VECTORS {
        assert_eq!(sign(key, message), format!("sha256={expected}"));
    }
}

#[test]
fn verification_accepts_the_rfc_4231_vectors() {
    for (key, message, expected) in HMAC_VECTORS {
        assert!(verify_signature(key, message, &format!("sha256={expected}")));
    }
}

#[test]
fn a_changed_body_key_or_signature_is_rejected() {
    let (key, message, expected) = HMAC_VECTORS[1];
    let header = format!("sha256={expected}");
    assert!(verify_signature(key, message, &header));

    // Tampered body: every single-byte change must fail.
    for i in 0..message.len() {
        let mut tampered = message.to_vec();
        tampered[i] ^= 1;
        assert!(!verify_signature(key, &tampered, &header), "byte {i} was not detected");
    }
    assert!(!verify_signature(key, b"", &header));
    assert!(!verify_signature(b"jefe", message, &header), "key is case sensitive");
    assert!(!verify_signature(b"", message, &header));

    // Tampered signature: flipping any single hex digit must fail.
    for i in 0..expected.len() {
        let mut bytes = expected.as_bytes().to_vec();
        bytes[i] = if bytes[i] == b'0' { b'1' } else { b'0' };
        let forged = format!("sha256={}", String::from_utf8(bytes).unwrap());
        assert!(!verify_signature(key, message, &forged), "digit {i} was not detected");
    }
}

#[test]
fn malformed_headers_are_rejected() {
    let (key, message, expected) = HMAC_VECTORS[0];
    for header in [
        String::new(),
        "sha256=".to_owned(),
        format!("sha1={expected}"),
        format!("SHA256={expected}"),
        expected.to_owned(),
        format!("sha256={}", &expected[..expected.len() - 2]),
        format!("sha256={expected}00"),
        format!("sha256={}g", &expected[..expected.len() - 1]),
        format!("sha256={expected} extra"),
        "sha256=\u{0}".to_owned(),
    ] {
        assert!(!verify_signature(key, message, &header), "accepted {header:?}");
    }
    // Surrounding whitespace in the header is tolerated; that is existing behaviour.
    assert!(verify_signature(key, message, &format!("  sha256={expected}\n")));
}

#[test]
fn derived_ids_are_unchanged() {
    // Computed independently: sha256("msgkit:v1" + 0x1f + part ...)[..16] as hex.
    assert_eq!(derive_id(&["a", "b"]), "94c18c34a413ea8898760f75759424ea");
    assert_eq!(derive_id(&[]), "332a461b11c0cee5002d6ebc7144cec7");
    assert_eq!(
        derive_id(&["sms", "msg", "1700000000000", "1", "+15550100111", "hi", "0"]),
        "c37db18a1aeea85051ddfa0dbc57b843"
    );
}
