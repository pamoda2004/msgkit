//! Configuration: what to look for, how to mask it, and what to leave alone.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An email address.
    Email,
    /// An IBAN of a country in the built-in table (checked with mod 97).
    Iban,
    /// A payment card number (Luhn, and a known network prefix unless `strict_card_prefix` is off).
    Card,
    /// A Sri Lankan national identity card number, old (9 digits and `V` or `X`) or new (12 digits).
    Nic,
    /// A phone number: international format, or Sri Lankan national format.
    Phone,
    /// A bank account number: a run of digits close to a word such as "account".
    Account,
    /// A name from `RedactConfig::names`.
    Name,
}

impl Kind {
    /// The upper-case label used in placeholders, for example `PHONE` in `[PHONE]`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Kind::Email => "EMAIL",
            Kind::Iban => "IBAN",
            Kind::Card => "CARD",
            Kind::Nic => "NIC",
            Kind::Phone => "PHONE",
            Kind::Account => "ACCOUNT",
            Kind::Name => "NAME",
        }
    }

    /// Detection priority when two findings overlap: the lower rank wins.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Kind::Iban => 0,
            Kind::Card => 1,
            Kind::Nic => 2,
            Kind::Email => 3,
            Kind::Phone => 4,
            Kind::Account => 5,
            Kind::Name => 6,
        }
    }
}

/// How sure a detector is. A match that a validator confirmed (Luhn, mod 97, a structurally valid NIC with its
/// letter, an international phone number) is `High`; a match that rests on shape or context alone (a bare
/// 12-digit NIC, a national phone number, a digit run next to the word "account") is `Medium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Shape or context only.
    Medium,
    /// Confirmed by a validator.
    High,
}

/// How a finding is written back into the text.
#[derive(Clone, PartialEq, Eq)]
pub enum Mask {
    /// `[PHONE]`: a typed placeholder that keeps none of the original characters. The default.
    Placeholder,
    /// `[CARD *1111]`: the last `n` (1 to 4) characters of a phone, card, account or IBAN number. Never used for
    /// NIC numbers, emails or names, which fall back to the placeholder.
    KeepLast(u8),
    /// `[PHONE:7f3a9c217f3a9c21]`: a keyed pseudonym (8 bytes of HMAC-SHA256, 16 hex characters), so the same value
    /// always gets the same token and different values can be told apart. The key is yours; without it the token
    /// cannot be linked back. Changing the key changes every token.
    Pseudonym(Vec<u8>),
}

impl fmt::Debug for Mask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mask::Placeholder => f.write_str("Placeholder"),
            Mask::KeepLast(n) => write!(f, "KeepLast({n})"),
            // The key must never reach a log.
            Mask::Pseudonym(_) => f.write_str("Pseudonym(<redacted>)"),
        }
    }
}

/// What happens to the id of a redacted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdPolicy {
    /// Keep the id (needed to de-duplicate). Ids are hashes of the content, not the content, but they can confirm a
    /// guessed message (see `docs/IDS.md`).
    Keep,
    /// Replace the id with a keyed pseudonym (needs [`Mask::Pseudonym`]): the same event keeps the same id, and the
    /// original id can no longer be used to confirm a guess.
    Pseudonymize,
}

/// A configuration that cannot be used. The text names the setting and never repeats its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub(crate) String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid redaction configuration: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Settings for a [`Redactor`](crate::Redactor).
///
/// `#[non_exhaustive]`: start from [`Default`] and chain the `with_*` setters.
///
/// ```
/// use msgkit_redact::{Kind, Mask, RedactConfig, Redactor};
///
/// let redactor = Redactor::new(
///     RedactConfig::default().with_mask(Mask::KeepLast(4)).with_kinds([Kind::Card, Kind::Phone]),
/// )
/// .unwrap();
/// assert_eq!(redactor.redact_text("call +94700000000").text, "call [PHONE *0000]");
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
// The four flags are independent switches of a configuration, not a state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct RedactConfig {
    /// The kinds to look for. Default: every kind except `Name`, which only applies to `names`.
    pub kinds: Vec<Kind>,
    /// How findings are written back. Default [`Mask::Placeholder`].
    pub mask: Mask,
    /// With a placeholder or `KeepLast`, write an email as `[EMAIL @example.org]`, keeping its domain. Default
    /// `false`. Not allowed with a pseudonym.
    pub keep_email_domain: bool,
    /// The calling code of national phone numbers (`"94"`, the only one supported). `None` finds only numbers
    /// written with `+` or `00`. Default `Some("94")`.
    pub default_country_code: Option<String>,
    /// Require a known network prefix and length on top of the Luhn check for card numbers. Default `true`. Turning
    /// it off finds any 13 to 19 digit number that passes Luhn, and more false positives.
    pub strict_card_prefix: bool,
    /// Report and mask only findings at or above this confidence. Default `Medium` (everything).
    pub min_confidence: Confidence,
    /// Words that mark a following digit run as a bank account number, matched without regard to case. Default: a
    /// short English list (`account`, `acc no`, `acc. no`, `acc number`, `a/c`, `acct`). Generic words such as `bank`
    /// or `current` are left out on purpose: next to a price they would mask it ("bank leasing 12500000"). Sinhala and
    /// Tamil words are not built in; add them here.
    pub account_keywords: Vec<String>,
    /// The fewest digits of an account number (6 to 17). Default 8, which keeps 7-digit prices out.
    pub min_account_digits: u8,
    /// The latest birth year of a new-format (12 digit) NIC number. Default 2010: a bare 12 digit number with a
    /// later "year" is more likely a timestamp.
    pub max_birth_year: u16,
    /// Two-digit trunk codes (without the leading 0) to accept in national phone numbers besides the mobile level
    /// `07x` and the geographic codes of the 2009 numbering plan.
    pub extra_area_codes: Vec<String>,
    /// Texts that are never masked, compared exactly with the matched text (for example your own support number).
    pub allow_exact: Vec<String>,
    /// Regular expressions; a finding whose whole text matches one is never masked.
    pub allow_patterns: Vec<String>,
    /// Names (people, shops) to mask wherever they appear as whole words, matched without regard to case.
    pub names: Vec<String>,
    /// Also mask the display name of every participant. Default `false`.
    pub redact_display_names: bool,
    /// Drop the `raw` payload of an event (it is the unredacted source). Default `true`; with `false` every string
    /// inside it is masked instead.
    pub drop_raw: bool,
    /// What happens to the id of an event. Default [`IdPolicy::Keep`].
    pub id_policy: IdPolicy,
}

impl Default for RedactConfig {
    fn default() -> Self {
        RedactConfig {
            kinds: vec![Kind::Email, Kind::Iban, Kind::Card, Kind::Nic, Kind::Phone, Kind::Account],
            mask: Mask::Placeholder,
            keep_email_domain: false,
            default_country_code: Some("94".to_owned()),
            strict_card_prefix: true,
            min_confidence: Confidence::Medium,
            account_keywords: ["account", "acc no", "acc. no", "acc number", "a/c", "acct"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            min_account_digits: 8,
            max_birth_year: 2010,
            extra_area_codes: Vec::new(),
            allow_exact: Vec::new(),
            allow_patterns: Vec::new(),
            names: Vec::new(),
            redact_display_names: false,
            drop_raw: true,
            id_policy: IdPolicy::Keep,
        }
    }
}

impl RedactConfig {
    /// Sets [`kinds`](Self::kinds).
    #[must_use]
    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = Kind>) -> Self {
        self.kinds = kinds.into_iter().collect();
        self
    }

    /// Sets [`mask`](Self::mask).
    #[must_use]
    pub fn with_mask(mut self, mask: Mask) -> Self {
        self.mask = mask;
        self
    }

    /// Sets [`keep_email_domain`](Self::keep_email_domain).
    #[must_use]
    pub fn with_keep_email_domain(mut self, keep: bool) -> Self {
        self.keep_email_domain = keep;
        self
    }

    /// Sets [`default_country_code`](Self::default_country_code).
    #[must_use]
    pub fn with_default_country_code(mut self, code: Option<&str>) -> Self {
        self.default_country_code = code.map(str::to_owned);
        self
    }

    /// Sets [`strict_card_prefix`](Self::strict_card_prefix).
    #[must_use]
    pub fn with_strict_card_prefix(mut self, strict: bool) -> Self {
        self.strict_card_prefix = strict;
        self
    }

    /// Sets [`min_confidence`](Self::min_confidence).
    #[must_use]
    pub fn with_min_confidence(mut self, confidence: Confidence) -> Self {
        self.min_confidence = confidence;
        self
    }

    /// Sets [`account_keywords`](Self::account_keywords).
    #[must_use]
    pub fn with_account_keywords<S: AsRef<str>>(mut self, words: impl IntoIterator<Item = S>) -> Self {
        self.account_keywords = words.into_iter().map(|w| w.as_ref().to_owned()).collect();
        self
    }

    /// Sets [`min_account_digits`](Self::min_account_digits).
    #[must_use]
    pub fn with_min_account_digits(mut self, digits: u8) -> Self {
        self.min_account_digits = digits;
        self
    }

    /// Sets [`max_birth_year`](Self::max_birth_year).
    #[must_use]
    pub fn with_max_birth_year(mut self, year: u16) -> Self {
        self.max_birth_year = year;
        self
    }

    /// Sets [`extra_area_codes`](Self::extra_area_codes).
    #[must_use]
    pub fn with_extra_area_codes<S: AsRef<str>>(mut self, codes: impl IntoIterator<Item = S>) -> Self {
        self.extra_area_codes = codes.into_iter().map(|c| c.as_ref().to_owned()).collect();
        self
    }

    /// Sets [`allow_exact`](Self::allow_exact).
    #[must_use]
    pub fn with_allow_exact<S: AsRef<str>>(mut self, texts: impl IntoIterator<Item = S>) -> Self {
        self.allow_exact = texts.into_iter().map(|t| t.as_ref().to_owned()).collect();
        self
    }

    /// Sets [`allow_patterns`](Self::allow_patterns).
    #[must_use]
    pub fn with_allow_patterns<S: AsRef<str>>(mut self, patterns: impl IntoIterator<Item = S>) -> Self {
        self.allow_patterns = patterns.into_iter().map(|p| p.as_ref().to_owned()).collect();
        self
    }

    /// Sets [`names`](Self::names).
    #[must_use]
    pub fn with_names<S: AsRef<str>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.names = names.into_iter().map(|n| n.as_ref().to_owned()).collect();
        self
    }

    /// Sets [`redact_display_names`](Self::redact_display_names).
    #[must_use]
    pub fn with_redact_display_names(mut self, redact: bool) -> Self {
        self.redact_display_names = redact;
        self
    }

    /// Sets [`drop_raw`](Self::drop_raw).
    #[must_use]
    pub fn with_drop_raw(mut self, drop: bool) -> Self {
        self.drop_raw = drop;
        self
    }

    /// Sets [`id_policy`](Self::id_policy).
    #[must_use]
    pub fn with_id_policy(mut self, policy: IdPolicy) -> Self {
        self.id_policy = policy;
        self
    }

    /// Checks the settings that can be wrong. The error names the setting, never its value.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        let bad = |what: &str| Err(ConfigError(what.to_owned()));
        match &self.mask {
            Mask::KeepLast(n) if !(1..=4).contains(n) => {
                return bad("mask: KeepLast needs 1 to 4 characters")
            }
            Mask::Pseudonym(key) if key.is_empty() => return bad("mask: the pseudonym key is empty"),
            Mask::Pseudonym(_) if self.keep_email_domain => {
                return bad("keep_email_domain cannot be combined with a pseudonym")
            }
            _ => {}
        }
        if self.id_policy == IdPolicy::Pseudonymize && !matches!(self.mask, Mask::Pseudonym(_)) {
            return bad("id_policy: Pseudonymize needs Mask::Pseudonym");
        }
        if let Some(code) = &self.default_country_code {
            if code != "94" {
                return bad(
                    "default_country_code: only Sri Lanka (\"94\") is supported for national numbers",
                );
            }
        }
        if !(6..=17).contains(&self.min_account_digits) {
            return bad("min_account_digits must be 6 to 17");
        }
        if self.account_keywords.iter().any(|w| w.trim().is_empty() || w.len() > 40) {
            return bad("account_keywords: every word must be 1 to 40 bytes");
        }
        if self.extra_area_codes.iter().any(|c| c.len() != 2 || !c.bytes().all(|b| b.is_ascii_digit())) {
            return bad("extra_area_codes: every code must be exactly two digits");
        }
        if self.names.len() > 10_000 || self.names.iter().any(|n| n.trim().is_empty() || n.len() > 100) {
            return bad("names: at most 10000 names of 1 to 100 bytes");
        }
        Ok(())
    }
}
