//! Deterministic identifiers.
//!
//! Ids are derived from the content of a record, so importing the same export
//! twice produces identical ids and a store can de-duplicate on them.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

/// Hashes `parts` into a 32-character hex id.
pub fn derive_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"msgkit:v1");
    for part in parts {
        // Unit separator keeps ("ab","c") distinct from ("a","bc").
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    hex::encode(&hasher.finalize()[..16])
}

/// Numbers repeated records that are otherwise identical.
///
/// WhatsApp exports only have minute precision, so two "ok" messages from the
/// same person in the same minute would collide. The n-th occurrence of a key
/// gets ordinal `n`, which is stable across re-imports of the same file.
#[derive(Debug, Default)]
pub(crate) struct Ordinals(HashMap<String, u32>);

impl Ordinals {
    pub(crate) fn next(&mut self, key: &str) -> u32 {
        let n = self.0.entry(key.to_owned()).or_insert(0);
        let current = *n;
        *n += 1;
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_same_id() {
        assert_eq!(derive_id(&["a", "b"]), derive_id(&["a", "b"]));
    }

    #[test]
    fn part_boundaries_matter() {
        assert_ne!(derive_id(&["ab", "c"]), derive_id(&["a", "bc"]));
    }

    #[test]
    fn ordinals_count_per_key() {
        let mut o = Ordinals::default();
        assert_eq!(o.next("x"), 0);
        assert_eq!(o.next("x"), 1);
        assert_eq!(o.next("y"), 0);
    }
}
