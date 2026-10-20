//! Finding sensitive values: a regular expression proposes a candidate, a validator confirms or rejects it.
//!
//! The regular expressions are linear-time (the `regex` crate has no backtracking), so no input can make a call
//! slow in a way that its length does not explain.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use msgkit_core::phone::normalize_phone;
use regex::{Regex, RegexBuilder};

use crate::config::{Confidence, ConfigError, Kind, RedactConfig};

/// The geographic trunk codes (without the leading 0) of the Telecommunications Regulatory Commission of Sri Lanka's
/// national numbering plan, Annex 1, revised 22 October 2009. The level `07` (mobile) is accepted as a whole. Codes
/// added after 2009 can be given with `RedactConfig::extra_area_codes`.
pub(crate) const AREA_CODES: &[&str] = &[
    "11", "21", "23", "24", "25", "26", "27", "31", "32", "33", "34", "35", "36", "37", "38", "41", "45",
    "47", "51", "52", "54", "55", "57", "63", "65", "66", "67", "81", "91",
];

/// IBAN length per country (ISO 13616 registry), for the countries this crate recognizes.
const IBAN_LENGTHS: &[(&str, usize)] = &[
    ("AD", 24),
    ("AE", 23),
    ("AT", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BR", 29),
    ("CH", 21),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DK", 18),
    ("EE", 20),
    ("ES", 24),
    ("FI", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GR", 27),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("LB", 28),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("MC", 27),
    ("MT", 31),
    ("NL", 18),
    ("NO", 15),
    ("PK", 24),
    ("PL", 28),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("SA", 24),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("TR", 26),
];

/// One finding before overlaps are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub confidence: Confidence,
}

pub(crate) struct Detector {
    kinds: HashSet<Kind>,
    min_confidence: Confidence,
    strict_card: bool,
    national_phones: bool,
    min_account_digits: usize,
    max_birth_year: u16,
    extra_area: Vec<String>,
    email: Regex,
    iban: Regex,
    cards: [Regex; 4],
    nic_old: Regex,
    nic_new: Regex,
    phone_intl: Regex,
    phone_national: Regex,
    keywords: Option<Regex>,
    digit_run: Regex,
    names: Option<Regex>,
    allow_exact: HashSet<String>,
    allow_patterns: Vec<Regex>,
    placeholder: Regex,
}

fn rx(pattern: &str) -> Regex {
    // The patterns are constants of this crate; a failure here is a bug caught by the tests.
    Regex::new(pattern).expect("built-in pattern")
}

/// Whether `c` is a letter, digit or combining mark (Unicode `\w`), so a boundary check also works for Sinhala and
/// Tamil, whose vowel signs are combining marks.
fn is_word(c: char) -> bool {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let mut buf = [0u8; 4];
    WORD.get_or_init(|| rx(r"^\w$")).is_match(c.encode_utf8(&mut buf))
}

fn boundary_before(text: &str, start: usize) -> bool {
    text[..start].chars().next_back().is_none_or(|c| !is_word(c))
}

fn boundary_after(text: &str, end: usize) -> bool {
    text[end..].chars().next().is_none_or(|c| !is_word(c))
}

fn digits_of(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (i, c) in digits.chars().rev().enumerate() {
        let Some(mut d) = c.to_digit(10) else { return false };
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    // `is_multiple_of` is newer than this crate's minimum Rust version.
    #[allow(clippy::manual_is_multiple_of)]
    let valid = sum % 10 == 0;
    valid && !digits.is_empty()
}

/// The lengths a card number with this prefix may have, for the networks this crate knows.
fn card_lengths(digits: &str) -> Option<&'static [usize]> {
    let p = |n: usize| digits.get(..n).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    if digits.starts_with('4') {
        Some(&[13, 16, 19])
    } else if (51..=55).contains(&p(2)) || (2221..=2720).contains(&p(4)) {
        Some(&[16])
    } else if p(2) == 34 || p(2) == 37 {
        Some(&[15])
    } else if p(4) == 6011
        || (644..=649).contains(&p(3))
        || p(2) == 65
        || (622_126..=622_925).contains(&p(6))
        || (3528..=3589).contains(&p(4))
    {
        Some(&[16, 17, 18, 19])
    } else if (300..=305).contains(&p(3)) || [36, 38, 39].contains(&p(2)) {
        Some(&[14, 15, 16, 17, 18, 19])
    } else if p(2) == 62 {
        Some(&[16, 17, 18, 19])
    } else {
        None
    }
}

fn iban_mod97(iban: &str) -> bool {
    let rearranged = format!("{}{}", &iban[4..], &iban[..4]);
    let mut remainder = 0u32;
    for c in rearranged.chars() {
        let value = match c {
            '0'..='9' => c.to_digit(10).unwrap_or(0),
            'A'..='Z' => c as u32 - 'A' as u32 + 10,
            _ => return false,
        };
        // A letter is two digits (10 to 35).
        remainder = if value >= 10 { (remainder * 100 + value) % 97 } else { (remainder * 10 + value) % 97 };
    }
    remainder == 1
}

/// `ddd` is the day number of a Sri Lankan NIC: 1 to 366, or 501 to 866 for women. Every year is allowed 366 days:
/// the numbers are commonly described as counting February as 29 days, and accepting one more number only errs on
/// the private side.
fn nic_day_ok(ddd: u32) -> bool {
    (1..=366).contains(&ddd) || (501..=866).contains(&ddd)
}

/// Whether `s` ends with the word `w` (for an alphabetic word, not as the tail of a longer word).
fn ends_with_word(s: &str, w: &str) -> bool {
    s.strip_suffix(w).is_some_and(|rest| {
        let alphabetic = w.chars().next_back().is_some_and(char::is_alphabetic);
        !alphabetic || rest.chars().next_back().is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Whether the digits at `[start, end)` look like an amount or a quantity (a price, a mileage, an engine size)
/// rather than an identifier: a currency before them (`Rs.`, `LKR`, `$`, the rupee signs and the Sinhala and Tamil
/// rupee words) or a unit after them (`km`, `cc`, `lakhs`, `m`, `k`, `/=`).
fn is_quantity(text: &str, start: usize, end: usize) -> bool {
    const BEFORE: &[&str] = &[
        "rs",
        "rs.",
        "lkr",
        "usd",
        "$",
        "\u{20a8}",
        "\u{0dbb}\u{0dd4}",
        "\u{0dbb}\u{0dd4}.",
        "\u{0bb0}\u{0bc2}",
        "\u{0bb0}\u{0bc2}.",
        "price",
    ];
    const AFTER: &[&str] = &["km", "kms", "cc", "lakh", "lakhs", "mn", "million", "mil", "k", "m", "l"];
    let before = text[..start].trim_end_matches([' ', ':', '-']).to_lowercase();
    if BEFORE.iter().any(|w| ends_with_word(&before, w)) {
        return true;
    }
    let after = text[end..].trim_start_matches(' ');
    if after.starts_with("/=") {
        return true;
    }
    let word: String = after.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
    !word.is_empty() && AFTER.contains(&word.as_str())
}

/// The largest index `<= i` that is a character boundary of `s`.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl Detector {
    pub(crate) fn new(cfg: &RedactConfig) -> Result<Self, ConfigError> {
        let err = |what: &str| ConfigError(what.to_owned());
        let keywords = if cfg.account_keywords.is_empty() {
            None
        } else {
            let mut words: Vec<String> =
                cfg.account_keywords.iter().map(|w| regex::escape(w.trim())).collect();
            words.sort_by_key(|w| std::cmp::Reverse(w.len()));
            Some(
                RegexBuilder::new(&format!("(?i)(?:{})", words.join("|")))
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|_| err("account_keywords could not be compiled"))?,
            )
        };
        let names = if cfg.names.is_empty() {
            None
        } else {
            let mut names: Vec<String> = cfg.names.iter().map(|n| regex::escape(n.trim())).collect();
            names.sort_by_key(|n| std::cmp::Reverse(n.len()));
            Some(
                RegexBuilder::new(&format!("(?i)(?:{})", names.join("|")))
                    .size_limit(16 << 20)
                    .build()
                    .map_err(|_| err("names could not be compiled (too many or too long)"))?,
            )
        };
        let allow_patterns = cfg
            .allow_patterns
            .iter()
            .map(|p| {
                RegexBuilder::new(&format!("^(?:{p})$"))
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|_| err("allow_patterns: a pattern could not be compiled"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Detector {
            kinds: cfg.kinds.iter().copied().collect(),
            min_confidence: cfg.min_confidence,
            strict_card: cfg.strict_card_prefix,
            national_phones: cfg.default_country_code.is_some(),
            min_account_digits: usize::from(cfg.min_account_digits),
            max_birth_year: cfg.max_birth_year,
            extra_area: cfg.extra_area_codes.clone(),
            email: rx(r"(?i)[a-z0-9._%+\-]+@(?:[a-z0-9\-]+\.)+[a-z][a-z0-9\-]+"),
            iban: rx(r"\b[A-Z]{2}[0-9]{2}(?: ?[A-Z0-9]){11,30}\b"),
            cards: [
                rx(r"\b[0-9]{13,19}\b"),
                rx(r"\b[0-9]{4}(?:[ -][0-9]{4}){2,3}(?:[ -][0-9]{1,3})?\b"),
                rx(r"\b3[47][0-9]{2}[ -][0-9]{6}[ -][0-9]{5}\b"),
                rx(r"\b3[0689][0-9]{2}[ -][0-9]{6}[ -][0-9]{4}\b"),
            ],
            nic_old: rx(r"\b[0-9]{9}[VvXx]\b"),
            nic_new: rx(r"\b[0-9]{12}\b"),
            phone_intl: rx(r"(?:\+|\b00)[1-9](?:[ .\-()]?[0-9]){7,16}"),
            phone_national: rx(r"(?:\(0[0-9]{2}\)|\b0[0-9]{2})[ .\-]?[0-9]{3}[ .\-]?[0-9]{4}\b"),
            keywords,
            digit_run: rx(r"\b[0-9](?:[ -]?[0-9]){5,16}\b"),
            names,
            allow_exact: cfg.allow_exact.iter().cloned().collect(),
            allow_patterns,
            placeholder: rx(
                r"\[(?:EMAIL|IBAN|CARD|NIC|PHONE|ACCOUNT|NAME)(?::[0-9a-f]{16}| \*[0-9A-Za-z]{1,4}| @[^\]\s]{1,253})?\]",
            ),
        })
    }

    fn on(&self, kind: Kind) -> bool {
        self.kinds.contains(&kind)
    }

    /// Every finding in `text`, overlaps resolved, in order of position.
    pub(crate) fn find(&self, text: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        if self.on(Kind::Email) {
            self.emails(text, &mut hits);
        }
        if self.on(Kind::Iban) {
            self.ibans(text, &mut hits);
        }
        if self.on(Kind::Card) {
            self.card_numbers(text, &mut hits);
        }
        if self.on(Kind::Nic) {
            self.nics(text, &mut hits);
        }
        if self.on(Kind::Phone) {
            self.phones(text, &mut hits);
        }
        if self.on(Kind::Account) {
            self.accounts(text, &mut hits);
        }
        if let Some(names) = &self.names {
            for m in names.find_iter(text) {
                if boundary_before(text, m.start()) && boundary_after(text, m.end()) {
                    hits.push(Hit {
                        kind: Kind::Name,
                        start: m.start(),
                        end: m.end(),
                        confidence: Confidence::High,
                    });
                }
            }
        }
        self.resolve(text, hits)
    }

    fn resolve(&self, text: &str, mut hits: Vec<Hit>) -> Vec<Hit> {
        // A finding inside a placeholder this crate wrote earlier is not found again: redacting twice changes nothing.
        let placeholders: Vec<(usize, usize)> =
            self.placeholder.find_iter(text).map(|m| (m.start(), m.end())).collect();
        hits.retain(|h| {
            h.confidence >= self.min_confidence
                && !placeholders.iter().any(|&(s, e)| h.start < e && s < h.end)
                && !self.allowed(&text[h.start..h.end])
        });
        hits.sort_by_key(|h| (h.kind.rank(), h.start, std::cmp::Reverse(h.end - h.start)));
        let mut accepted: BTreeMap<usize, Hit> = BTreeMap::new();
        for h in hits {
            let before = accepted.range(..=h.start).next_back().is_some_and(|(_, p)| p.end > h.start);
            let after = accepted.range(h.start..).next().is_some_and(|(s, _)| *s < h.end);
            if !before && !after {
                accepted.insert(h.start, h);
            }
        }
        accepted.into_values().collect()
    }

    fn allowed(&self, found: &str) -> bool {
        self.allow_exact.contains(found) || self.allow_patterns.iter().any(|p| p.is_match(found))
    }

    fn emails(&self, text: &str, hits: &mut Vec<Hit>) {
        for m in self.email.find_iter(text) {
            let s = m.as_str();
            let Some((local, domain)) = s.rsplit_once('@') else { continue };
            let labels_ok =
                domain.split('.').all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'));
            if local.len() <= 64
                && domain.len() <= 253
                && labels_ok
                && !local.starts_with('.')
                && !local.ends_with('.')
            {
                hits.push(Hit {
                    kind: Kind::Email,
                    start: m.start(),
                    end: m.end(),
                    confidence: Confidence::High,
                });
            }
        }
    }

    fn ibans(&self, text: &str, hits: &mut Vec<Hit>) {
        for m in self.iban.find_iter(text) {
            let s = m.as_str();
            let Some(&(_, len)) = IBAN_LENGTHS.iter().find(|(country, _)| s.starts_with(country)) else {
                continue;
            };
            // The candidate may run on into the next word: keep exactly the country's length.
            let mut count = 0;
            let mut end = None;
            for (i, c) in s.char_indices() {
                if c != ' ' {
                    count += 1;
                    if count == len {
                        end = Some(m.start() + i + c.len_utf8());
                        break;
                    }
                }
            }
            let Some(end) = end else { continue };
            let compact: String = text[m.start()..end].chars().filter(|c| *c != ' ').collect();
            if iban_mod97(&compact) && boundary_after(text, end) {
                hits.push(Hit { kind: Kind::Iban, start: m.start(), end, confidence: Confidence::High });
            }
        }
    }

    fn card_numbers(&self, text: &str, hits: &mut Vec<Hit>) {
        for re in &self.cards {
            for m in re.find_iter(text) {
                let digits = digits_of(m.as_str());
                // A number made of one repeated digit (all zeros pass Luhn) is a placeholder, not a card.
                let one_digit = digits.chars().all(|c| digits.starts_with(c));
                if !(13..=19).contains(&digits.len()) || one_digit || !luhn(&digits) {
                    continue;
                }
                let known = card_lengths(&digits).is_some_and(|lengths| lengths.contains(&digits.len()));
                if known || !self.strict_card {
                    let confidence = if known { Confidence::High } else { Confidence::Medium };
                    hits.push(Hit { kind: Kind::Card, start: m.start(), end: m.end(), confidence });
                }
            }
        }
    }

    fn nics(&self, text: &str, hits: &mut Vec<Hit>) {
        for m in self.nic_old.find_iter(text) {
            let ddd = m.as_str()[2..5].parse::<u32>().unwrap_or(0);
            if nic_day_ok(ddd) {
                hits.push(Hit {
                    kind: Kind::Nic,
                    start: m.start(),
                    end: m.end(),
                    confidence: Confidence::High,
                });
            }
        }
        for m in self.nic_new.find_iter(text) {
            let s = m.as_str();
            let year = s[..4].parse::<u16>().unwrap_or(0);
            let ddd = s[4..7].parse::<u32>().unwrap_or(0);
            if (1900..=self.max_birth_year).contains(&year)
                && nic_day_ok(ddd)
                && !is_quantity(text, m.start(), m.end())
            {
                hits.push(Hit {
                    kind: Kind::Nic,
                    start: m.start(),
                    end: m.end(),
                    confidence: Confidence::Medium,
                });
            }
        }
    }

    fn phones(&self, text: &str, hits: &mut Vec<Hit>) {
        for m in self.phone_intl.find_iter(text) {
            let s = m.as_str();
            let prefix = if s.starts_with('+') { 1 } else { 2 };
            // Byte offset (within the match) just after each digit that follows the prefix.
            let ends: Vec<usize> = s
                .char_indices()
                .skip(prefix)
                .filter(|(_, c)| c.is_ascii_digit())
                .map(|(i, _)| i + 1)
                .collect();
            let keep = if s[prefix..].starts_with("94") { 11 } else { ends.len().min(15) };
            if ends.len() < keep || keep < 8 {
                continue;
            }
            let end = m.start() + ends[keep - 1];
            if normalize_phone(&text[m.start()..end], None).is_some() {
                hits.push(Hit { kind: Kind::Phone, start: m.start(), end, confidence: Confidence::High });
            }
        }
        if !self.national_phones {
            return;
        }
        for m in self.phone_national.find_iter(text) {
            let digits = digits_of(m.as_str());
            let code = &digits[1..3];
            let known = code.starts_with('7')
                || AREA_CODES.contains(&code)
                || self.extra_area.iter().any(|c| c == code);
            if known && boundary_before(text, m.start()) {
                hits.push(Hit {
                    kind: Kind::Phone,
                    start: m.start(),
                    end: m.end(),
                    confidence: Confidence::Medium,
                });
            }
        }
    }

    fn accounts(&self, text: &str, hits: &mut Vec<Hit>) {
        let Some(keywords) = &self.keywords else { return };
        for k in keywords.find_iter(text) {
            if !boundary_before(text, k.start()) || !boundary_after(text, k.end()) {
                continue;
            }
            let window_end = floor_boundary(text, k.end() + 60);
            let window = &text[k.end()..window_end];
            let window = window.split("\n\n").next().unwrap_or(window);
            for run in self.digit_run.find_iter(window) {
                let (start, end) = (k.end() + run.start(), k.end() + run.end());
                let digits = digits_of(run.as_str());
                if digits.len() < self.min_account_digits
                    || digits.len() > 17
                    || is_quantity(text, start, end)
                {
                    continue;
                }
                hits.push(Hit { kind: Kind::Account, start, end, confidence: Confidence::Medium });
                break;
            }
        }
    }
}
