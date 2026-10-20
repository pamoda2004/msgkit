//! Input limits: how much data one import call accepts.
//!
//! Parsing costs several times the size of its input in memory, and a hostile file can be made to cost much more
//! than a real one. Every entry point therefore checks the input against a [`Limits`] before it does the
//! expensive work and refuses the whole call with [`Error::LimitExceeded`] (code `limit_exceeded`) when a limit
//! is passed. Nothing is truncated: a partial import would look like a complete one.
//!
//! * A limit that is **absent** (or `null` in JSON) has its default. The defaults are generous for real exports
//!   (a WhatsApp export is capped by the app at 40,000 messages, about 4 MB).
//! * To change a limit, give a number of **at least 1**. `0` is refused with `invalid_input`: it would read as
//!   "allow nothing" and must never switch protection off by accident.
//! * To switch the limits off there is one explicit way: [`Limits::unlimited`] (JSON `"unlimited": true`). Use
//!   it only for input you trust.
//!
//! ```
//! use msgkit_core::limits::Limits;
//! use msgkit_core::whatsapp_export::WhatsAppExportOptions;
//!
//! let stricter = WhatsAppExportOptions::default().with_limits(Limits::default().with_max_messages(50_000));
//! let trusted = WhatsAppExportOptions::default().with_limits(Limits::unlimited());
//! # let _ = (stricter, trusted);
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// Default for [`Limits::max_input_bytes`]: 32 MiB.
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
/// Default for [`Limits::max_line_bytes`]: 256 KiB.
pub const DEFAULT_MAX_LINE_BYTES: u64 = 256 * 1024;
/// Default for [`Limits::max_messages`]: 1,000,000.
pub const DEFAULT_MAX_MESSAGES: u64 = 1_000_000;
/// Default for [`Limits::max_json_depth`]: 32.
pub const DEFAULT_MAX_JSON_DEPTH: u64 = 32;

/// Upper bounds for one import call. An absent field (`None`, or `null` in JSON) has its default.
///
/// `#[non_exhaustive]`: start from [`Default`] or [`Limits::unlimited`] and chain the `with_*` setters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
#[schemars(
    description = "Upper bounds for one import call; the call fails with `limit_exceeded` when one is passed. An \
                   absent or null limit has its default. A limit must be at least 1 (0 is refused as \
                   `invalid_input`); to switch the limits off set `unlimited` to true."
)]
pub struct Limits {
    /// Largest accepted input, in bytes: the exported text for a WhatsApp export, the whole JSON document for the
    /// other imports. Absent: 33554432 (32 MiB).
    pub max_input_bytes: Option<u64>,
    /// Longest accepted line of a WhatsApp export, in bytes. Absent: 262144 (256 KiB).
    pub max_line_bytes: Option<u64>,
    /// Most messages, calls and notices accepted in one import. Absent: 1000000.
    pub max_messages: Option<u64>,
    /// Deepest accepted nesting of JSON arrays and objects in the document handed to msgkit. Through the JSON
    /// entry point that document is the whole request, so its wrapper object counts. Absent: 32.
    pub max_json_depth: Option<u64>,
    /// Switches every limit off. The only way to do so; use it only for input you trust. Default `false`.
    pub unlimited: bool,
}

impl Limits {
    /// Every limit switched off. Only for input you trust.
    #[must_use]
    pub fn unlimited() -> Self {
        Limits { unlimited: true, ..Limits::default() }
    }

    /// Sets [`max_input_bytes`](Self::max_input_bytes) (at least 1).
    #[must_use]
    pub fn with_max_input_bytes(mut self, bytes: u64) -> Self {
        self.max_input_bytes = Some(bytes);
        self
    }

    /// Sets [`max_line_bytes`](Self::max_line_bytes) (at least 1).
    #[must_use]
    pub fn with_max_line_bytes(mut self, bytes: u64) -> Self {
        self.max_line_bytes = Some(bytes);
        self
    }

    /// Sets [`max_messages`](Self::max_messages) (at least 1).
    #[must_use]
    pub fn with_max_messages(mut self, count: u64) -> Self {
        self.max_messages = Some(count);
        self
    }

    /// Sets [`max_json_depth`](Self::max_json_depth) (at least 1).
    #[must_use]
    pub fn with_max_json_depth(mut self, depth: u64) -> Self {
        self.max_json_depth = Some(depth);
        self
    }

    /// Refuses a limit of `0`. Every entry point calls this first.
    ///
    /// # Errors
    /// [`Error::InvalidInput`], naming the limit and never anything else.
    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("max_input_bytes", self.max_input_bytes),
            ("max_line_bytes", self.max_line_bytes),
            ("max_messages", self.max_messages),
            ("max_json_depth", self.max_json_depth),
        ] {
            if value == Some(0) && !self.unlimited {
                return Err(Error::InvalidInput(format!(
                    "limits.{name} must be at least 1 (set `unlimited` to switch the limits off)"
                )));
            }
        }
        Ok(())
    }

    /// The input size limit in effect: `None` when unlimited, else the value or its default.
    #[must_use]
    pub fn input_bytes_limit(&self) -> Option<u64> {
        (!self.unlimited).then(|| self.max_input_bytes.unwrap_or(DEFAULT_MAX_INPUT_BYTES))
    }

    fn line_bytes_limit(&self) -> Option<u64> {
        (!self.unlimited).then(|| self.max_line_bytes.unwrap_or(DEFAULT_MAX_LINE_BYTES))
    }

    fn messages_limit(&self) -> Option<u64> {
        (!self.unlimited).then(|| self.max_messages.unwrap_or(DEFAULT_MAX_MESSAGES))
    }

    fn json_depth_limit(&self) -> Option<u64> {
        (!self.unlimited).then(|| self.max_json_depth.unwrap_or(DEFAULT_MAX_JSON_DEPTH))
    }

    /// Fails when `len` bytes of input are more than [`max_input_bytes`](Self::max_input_bytes).
    ///
    /// # Errors
    /// [`Error::LimitExceeded`].
    pub fn check_input_bytes(&self, len: usize) -> Result<()> {
        match self.input_bytes_limit() {
            Some(max) if len as u64 > max => {
                Err(Error::LimitExceeded(format!("the input is larger than max_input_bytes ({max} bytes)")))
            }
            _ => Ok(()),
        }
    }

    /// Fails when a line of `len` bytes is more than [`max_line_bytes`](Self::max_line_bytes). The error names
    /// the line number, never its content.
    ///
    /// # Errors
    /// [`Error::LimitExceeded`].
    pub fn check_line_bytes(&self, line_number: usize, len: usize) -> Result<()> {
        match self.line_bytes_limit() {
            Some(max) if len as u64 > max => Err(Error::LimitExceeded(format!(
                "line {line_number} is longer than max_line_bytes ({max} bytes)"
            ))),
            _ => Ok(()),
        }
    }

    /// Fails when `count` is more than [`max_messages`](Self::max_messages).
    ///
    /// # Errors
    /// [`Error::LimitExceeded`].
    pub fn check_messages(&self, count: usize) -> Result<()> {
        match self.messages_limit() {
            Some(max) if count as u64 > max => {
                Err(Error::LimitExceeded(format!("the input has more than max_messages ({max}) items")))
            }
            _ => Ok(()),
        }
    }

    fn depth_error(max: u64) -> Error {
        Error::LimitExceeded(format!("the JSON is nested deeper than max_json_depth ({max})"))
    }

    /// Fails when the JSON text nests arrays and objects deeper than
    /// [`max_json_depth`](Self::max_json_depth). Reads the text once, without building a value, and does not
    /// need the text to be valid JSON (a syntax error is reported later by the parser).
    ///
    /// # Errors
    /// [`Error::LimitExceeded`].
    pub fn check_json_depth_str(&self, json: &str) -> Result<()> {
        let Some(max) = self.json_depth_limit() else { return Ok(()) };
        let (mut depth, mut in_string, mut escaped) = (0u64, false, false);
        for &b in json.as_bytes() {
            if in_string {
                match b {
                    _ if escaped => escaped = false,
                    b'\\' => escaped = true,
                    b'"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > max {
                        return Err(Self::depth_error(max));
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        Ok(())
    }

    /// Fails when an already parsed JSON value nests deeper than [`max_json_depth`](Self::max_json_depth).
    /// Iterative, so a value built by hand cannot overflow the stack.
    ///
    /// # Errors
    /// [`Error::LimitExceeded`].
    pub fn check_json_depth_value(&self, value: &Value) -> Result<()> {
        let Some(max) = self.json_depth_limit() else { return Ok(()) };
        let mut stack = vec![(value, 0u64)];
        while let Some((v, depth)) = stack.pop() {
            let children: Vec<&Value> = match v {
                Value::Array(items) => items.iter().collect(),
                Value::Object(map) => map.values().collect(),
                _ => continue,
            };
            let depth = depth + 1;
            if depth > max {
                return Err(Self::depth_error(max));
            }
            stack.extend(children.into_iter().map(|c| (c, depth)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(levels: usize) -> String {
        format!("{}1{}", "[".repeat(levels), "]".repeat(levels))
    }

    #[test]
    fn depth_counts_arrays_and_objects() {
        let l = Limits::default().with_max_json_depth(3);
        assert!(l.check_json_depth_str(&nested(3)).is_ok());
        assert!(l.check_json_depth_str(&nested(4)).is_err());
        assert!(l.check_json_depth_str(r#"{"a":{"b":[1]}}"#).is_ok());
        assert!(l.check_json_depth_str(r#"{"a":{"b":[[1]]}}"#).is_err());
    }

    #[test]
    fn brackets_inside_strings_do_not_count() {
        let l = Limits::default().with_max_json_depth(1);
        assert!(l.check_json_depth_str(r#"["[[[[", "\"[[[[", "\\"]"#).is_ok());
        assert!(l.check_json_depth_str(r#"[["x"]]"#).is_err());
    }

    #[test]
    fn value_depth_matches_text_depth() {
        let l = Limits::default().with_max_json_depth(3);
        for levels in [1, 3, 4, 10] {
            let text = nested(levels);
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(l.check_json_depth_str(&text).is_ok(), l.check_json_depth_value(&value).is_ok());
        }
    }

    #[test]
    fn unlimited_switches_everything_off() {
        let l = Limits::unlimited();
        assert!(l.check_input_bytes(usize::MAX).is_ok());
        assert!(l.check_line_bytes(1, usize::MAX).is_ok());
        assert!(l.check_messages(usize::MAX).is_ok());
        assert!(l.check_json_depth_str(&nested(5000)).is_ok());
        assert_eq!(l.input_bytes_limit(), None);
    }

    #[test]
    fn the_limit_itself_is_accepted_and_one_more_is_not() {
        let l = Limits::default().with_max_input_bytes(10).with_max_line_bytes(5).with_max_messages(2);
        assert!(l.check_input_bytes(10).is_ok() && l.check_input_bytes(11).is_err());
        assert!(l.check_line_bytes(1, 5).is_ok() && l.check_line_bytes(1, 6).is_err());
        assert!(l.check_messages(2).is_ok() && l.check_messages(3).is_err());
    }

    #[test]
    fn errors_name_the_limit_and_nothing_else() {
        let e = Limits::default().with_max_line_bytes(5).check_line_bytes(7, 6).unwrap_err();
        assert_eq!(e.code(), "limit_exceeded");
        assert_eq!(e.to_string(), "limit exceeded: line 7 is longer than max_line_bytes (5 bytes)");
    }

    #[test]
    fn absent_and_null_mean_the_default() {
        let l: Limits = serde_json::from_str(r#"{"max_messages": 5, "max_json_depth": null}"#).unwrap();
        assert_eq!(l.max_messages, Some(5));
        assert_eq!(l.max_json_depth, None);
        assert_eq!(l.input_bytes_limit(), Some(DEFAULT_MAX_INPUT_BYTES));
        assert_eq!(l.json_depth_limit(), Some(DEFAULT_MAX_JSON_DEPTH));
        assert_eq!(Limits::default().messages_limit(), Some(DEFAULT_MAX_MESSAGES));
        assert_eq!(Limits::default().line_bytes_limit(), Some(DEFAULT_MAX_LINE_BYTES));
    }

    #[test]
    fn zero_is_invalid_input_and_never_switches_a_limit_off() {
        for json in [
            r#"{"max_input_bytes": 0}"#,
            r#"{"max_line_bytes": 0}"#,
            r#"{"max_messages": 0}"#,
            r#"{"max_json_depth": 0}"#,
        ] {
            let l: Limits = serde_json::from_str(json).unwrap();
            let e = l.validate().unwrap_err();
            assert_eq!(e.code(), "invalid_input", "{json}");
            assert!(e.to_string().contains("at least 1"), "{e}");
        }
        assert!(Limits::default().with_max_messages(1).validate().is_ok());
        assert!(Limits::default().validate().is_ok());
        // `unlimited` is the explicit opt-out; a stray 0 next to it is not an error, it is simply moot.
        let l: Limits = serde_json::from_str(r#"{"unlimited": true, "max_messages": 0}"#).unwrap();
        assert!(l.validate().is_ok() && l.check_messages(usize::MAX).is_ok());
    }
}
