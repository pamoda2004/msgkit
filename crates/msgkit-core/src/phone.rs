//! Minimal phone-number normalization to E.164.
//!
//! This deliberately avoids a full numbering-plan database. It only handles
//! numbers that are unambiguous: those with an international prefix, or
//! national numbers when the caller supplies the country calling code.

/// Returns the E.164 form (`+<digits>`) of `raw`, or `None` if `raw` does not
/// look like a phone number (names, alphanumeric sender ids, short codes).
///
/// `default_country_code` (digits only, e.g. `"94"`) is used for national
/// numbers that start with a trunk prefix `0`.
#[must_use]
pub fn normalize_phone(raw: &str, default_country_code: Option<&str>) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(c, '-' | '(' | ')' | '.' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}')
        })
        .collect();
    if cleaned.is_empty() {
        return None;
    }

    let digits_only = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let in_range = |s: &str| (7..=15).contains(&s.len());

    if let Some(rest) = cleaned.strip_prefix('+') {
        return (digits_only(rest) && in_range(rest)).then(|| format!("+{rest}"));
    }
    if let Some(rest) = cleaned.strip_prefix("00") {
        return (digits_only(rest) && in_range(rest)).then(|| format!("+{rest}"));
    }
    if let (Some(cc), Some(rest)) = (default_country_code, cleaned.strip_prefix('0')) {
        let cc = cc.trim_start_matches('+');
        let full = format!("{cc}{rest}");
        return (digits_only(&full) && in_range(&full)).then(|| format!("+{full}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn international_formats() {
        assert_eq!(normalize_phone("+94 70 000 0000", None).as_deref(), Some("+94700000000"));
        assert_eq!(normalize_phone("0094-70-0000000", None).as_deref(), Some("+94700000000"));
    }

    #[test]
    fn national_needs_country_code() {
        assert_eq!(normalize_phone("0700000000", None), None);
        assert_eq!(normalize_phone("0700000000", Some("94")).as_deref(), Some("+94700000000"));
    }

    #[test]
    fn non_numbers_are_rejected() {
        assert_eq!(normalize_phone("DIALOG", None), None);
        assert_eq!(normalize_phone("Alice Smith", Some("94")), None);
        assert_eq!(normalize_phone("", None), None);
        assert_eq!(normalize_phone("+12", None), None);
    }
}
