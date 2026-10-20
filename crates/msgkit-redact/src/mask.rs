//! Writing a finding back as a placeholder, a partial mask or a keyed pseudonym.

use hmac::{Hmac, KeyInit, Mac};
use msgkit_core::phone::normalize_phone;
use sha2::Sha256;

use crate::config::{Kind, Mask};
use crate::Redactor;

type HmacSha256 = Hmac<Sha256>;

/// The value that identifies "the same thing" for a pseudonym: two spellings of one phone number get one token.
fn normalized(kind: Kind, original: &str) -> String {
    match kind {
        Kind::Phone => normalize_phone(original, Some("94"))
            .unwrap_or_else(|| original.chars().filter(char::is_ascii_digit).collect()),
        Kind::Card | Kind::Account => original.chars().filter(char::is_ascii_digit).collect(),
        Kind::Iban | Kind::Nic => {
            original.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_uppercase()
        }
        Kind::Email | Kind::Name => original.to_lowercase(),
    }
}

impl Redactor {
    /// 16 hex characters: 8 bytes of HMAC-SHA256 over the label and the normalized value, under the pseudonym key.
    fn token(&self, label: &str, value: &str) -> Option<String> {
        let Mask::Pseudonym(key) = &self.config.mask else { return None };
        let mut mac = HmacSha256::new_from_slice(key).ok()?;
        mac.update(label.as_bytes());
        mac.update(&[0x1f]);
        mac.update(value.as_bytes());
        Some(hex::encode(&mac.finalize().into_bytes()[..8]))
    }

    /// The mask for one finding of `kind` whose text is `original`.
    pub(crate) fn mask_value(&self, kind: Kind, original: &str) -> String {
        let label = kind.label();
        match &self.config.mask {
            Mask::Placeholder => self.placeholder(kind, original),
            Mask::KeepLast(n) => {
                if matches!(kind, Kind::Phone | Kind::Card | Kind::Account | Kind::Iban) {
                    let value = normalized(kind, original);
                    let keep = usize::from(*n).min(value.len());
                    // Normalized values are ASCII, so slicing by bytes is safe.
                    format!("[{label} *{}]", &value[value.len() - keep..])
                } else {
                    self.placeholder(kind, original)
                }
            }
            Mask::Pseudonym(_) => match self.token(label, &normalized(kind, original)) {
                Some(token) => format!("[{label}:{token}]"),
                None => format!("[{label}]"),
            },
        }
    }

    fn placeholder(&self, kind: Kind, original: &str) -> String {
        if kind == Kind::Email && self.config.keep_email_domain {
            if let Some((_, domain)) = original.rsplit_once('@') {
                return format!("[EMAIL @{}]", domain.to_lowercase());
            }
        }
        format!("[{}]", kind.label())
    }

    /// A keyed pseudonym id for an event id (32 hex characters), or `None` without a pseudonym key.
    pub(crate) fn pseudonym_id(&self, id: &str) -> Option<String> {
        let Mask::Pseudonym(key) = &self.config.mask else { return None };
        let mut mac = HmacSha256::new_from_slice(key).ok()?;
        mac.update(b"msgkit-redact:id");
        mac.update(&[0x1f]);
        mac.update(id.as_bytes());
        Some(hex::encode(&mac.finalize().into_bytes()[..16]))
    }
}
