#![no_main]
//! The input is split into a secret, a body and a header; verification must never panic, and a signature
//! made with `sign` must always verify while a changed body must not.

use libfuzzer_sys::fuzz_target;
use msgkit_core::webhook::{sign, verify_signature};

fuzz_target!(|data: &[u8]| {
    let Some((&split, rest)) = data.split_first() else { return };
    let (secret, rest) = rest.split_at((usize::from(split) % (rest.len() + 1)).min(rest.len()));
    let (body, header) = rest.split_at(rest.len() / 2);
    let _ = verify_signature(secret, body, &String::from_utf8_lossy(header));

    let signature = sign(secret, body);
    assert!(verify_signature(secret, body, &signature));
    let mut other = body.to_vec();
    other.push(0);
    assert!(!verify_signature(secret, &other, &signature));
});
