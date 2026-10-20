//! # msgkit-redact
//!
//! Optional, **best-effort** masking of identifiers in message text and in msgkit events: email addresses, phone
//! numbers (international, and Sri Lankan national format), Sri Lankan NIC numbers (old and new), payment card
//! numbers, IBANs and bank account numbers.
//!
//! ```
//! use msgkit_redact::{RedactConfig, Redactor};
//!
//! let redactor = Redactor::new(RedactConfig::default()).unwrap();
//! let out = redactor.redact_text("Call +94700000000 or mail kamal@example.org, NIC 850010000V");
//! assert_eq!(out.text, "Call [PHONE] or mail [EMAIL], NIC [NIC]");
//! assert_eq!(out.findings.len(), 3); // kinds and positions, never the matched text
//! ```
//!
//! ## What this is not
//!
//! * It is **not anonymisation.** It finds what it can recognize; anything it does not recognize stays. **Names are
//!   not detected** (there is no name recognition): give the names you know in [`RedactConfig::names`].
//! * A validator decides each match (Luhn for cards, mod 97 for IBANs, the structure of a NIC, the numbering plan for
//!   phones), and shape-only matches are marked [`Confidence::Medium`]. Real data still produces false positives
//!   (a 13 to 19 digit number that passes Luhn and has a card prefix, such as some barcodes) and false negatives
//!   (a number written with unusual separators, spelled out, or split over lines).
//! * It is pure: no I/O, no clock, no environment, no network. The regular expressions run in linear time.
//!
//! Everything it knows about Sri Lankan numbers is in `docs/REDACTION.md`, with its sources.

#![forbid(unsafe_code)]

mod config;
mod detect;
mod event;
mod mask;

use std::collections::BTreeMap;
use std::fmt;

pub use config::{Confidence, ConfigError, IdPolicy, Kind, Mask, RedactConfig};
pub use event::Summary;

use detect::Detector;

/// One thing found in a text: what, where (byte offsets into the input) and how sure. It never holds the text itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finding {
    /// What was found.
    pub kind: Kind,
    /// Byte offset of the first character in the input.
    pub start: usize,
    /// Byte offset after the last character in the input.
    pub end: usize,
    /// How sure the detector is.
    pub confidence: Confidence,
}

/// A redacted text and what was found in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    /// The text with every finding replaced by its mask.
    pub text: String,
    /// The findings, in order of position, with offsets into the **original** text.
    pub findings: Vec<Finding>,
}

impl Redacted {
    /// How many findings of each kind.
    #[must_use]
    pub fn counts(&self) -> BTreeMap<Kind, usize> {
        let mut counts = BTreeMap::new();
        for f in &self.findings {
            *counts.entry(f.kind).or_insert(0) += 1;
        }
        counts
    }
}

/// Finds and masks identifiers. Build it once (it compiles its patterns) and reuse it.
pub struct Redactor {
    detector: Detector,
    config: RedactConfig,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The configuration's own Debug already hides a pseudonym key.
        f.debug_struct("Redactor").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Redactor {
    /// Builds a redactor.
    ///
    /// # Errors
    /// [`ConfigError`] when a setting cannot be used. The error names the setting, never its value.
    pub fn new(config: RedactConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        let detector = Detector::new(&config)?;
        Ok(Redactor { detector, config })
    }

    /// The configuration in use.
    #[must_use]
    pub fn config(&self) -> &RedactConfig {
        &self.config
    }

    /// Finds identifiers without changing anything (a dry run). Offsets are bytes of `text`.
    #[must_use]
    pub fn detect(&self, text: &str) -> Vec<Finding> {
        self.detector
            .find(text)
            .into_iter()
            .map(|h| Finding { kind: h.kind, start: h.start, end: h.end, confidence: h.confidence })
            .collect()
    }

    /// Masks every identifier found in `text`.
    ///
    /// Redacting an already redacted text changes nothing: a placeholder is never found again.
    #[must_use]
    pub fn redact_text(&self, text: &str) -> Redacted {
        let hits = self.detector.find(text);
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for h in &hits {
            out.push_str(&text[at..h.start]);
            out.push_str(&self.mask_value(h.kind, &text[h.start..h.end]));
            at = h.end;
        }
        out.push_str(&text[at..]);
        let findings = hits
            .into_iter()
            .map(|h| Finding { kind: h.kind, start: h.start, end: h.end, confidence: h.confidence })
            .collect();
        Redacted { text: out, findings }
    }
}
