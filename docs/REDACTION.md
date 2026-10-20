# Redaction (`msgkit-redact`)

`msgkit-redact` is an **optional** Rust crate that masks identifiers in text and in msgkit events. It is not part of `msgkit-core` (the core stays pure and small), it has no binding yet, and it is **best effort**: it finds what it recognizes and leaves the rest. Read "What it does not do" before relying on it.

```rust
use msgkit_redact::{RedactConfig, Redactor};

let redactor = Redactor::new(RedactConfig::default())?;
let out = redactor.redact_text("Call +94700000000 or mail kamal@example.org, NIC 850010000V");
assert_eq!(out.text, "Call [PHONE] or mail [EMAIL], NIC [NIC]");
// out.findings: kind, byte range and confidence of each match, never the matched text
let (report, summary) = redactor.redact_report(&import_report); // events; summary.counts per kind
```

## What it finds

Every detector proposes a candidate with a regular expression (linear time) and a validator confirms it. A match is **High** confidence when a validator confirmed it and **Medium** when it rests on shape or context. `min_confidence` keeps only the High ones.

| Kind | What matches | Validation | Confidence |
|---|---|---|---|
| Email | a local part, an `@` and a dotted domain: letters, digits and `._%+-` before the `@`, dot-separated labels after it, a last label of at least two characters that starts with a letter | local part at most 64 and domain at most 253 bytes, no empty or hyphen-edged label | High |
| Phone, international | `+` or `00`, a country code that does not start with 0, then digits with single spaces, dots, hyphens or parentheses between them | at least 8 and at most 15 digits, accepted by `msgkit_core::phone::normalize_phone` (one source of truth); a `+94` number ends after exactly 9 national digits, so `+94 70 000 0000 5 pm` masks the number and keeps the `5` | High |
| Phone, Sri Lankan national | `0`, two digits, then 7 digits (`070 000 0000`, `070-0000000`, `(011) 000 0000`, `0700000000`) | the two digits are `7x` (the whole mobile level) or a geographic trunk code from the 2009 numbering plan (below) or `extra_area_codes`; only when `default_country_code` is `94` (the default) | Medium |
| NIC, old format | 9 digits and `V`, `v`, `X` or `x`: year (2), day number (3), serial (3), check digit (1) | day number 1 to 366, or 501 to 866 for women (a woman's day number is the day plus 500) | High |
| NIC, new format | 12 digits: year (4), day number (3), serial (4), check digit (1) | year 1900 to `max_birth_year` (default 2010), the same day numbers, and not an amount or a quantity (below) | Medium |
| Card number | 13 to 19 digits, contiguous or in groups of four separated by one space or hyphen, plus the 4-6-5 (Amex) and 4-6-4 (Diners) layouts | Luhn, not a single repeated digit, and (unless `strict_card_prefix` is off) a known network prefix with a matching length: Visa, Mastercard (51 to 55 and 2221 to 2720), Amex, Discover, JCB, Diners, UnionPay | High (known network) or Medium (Luhn only) |
| IBAN | two letters, two digits and up to 30 letters or digits, spaces allowed; upper case | a country in the built-in table (48 countries) with its registered length, then mod 97 equals 1; a candidate that runs on into the next word is cut at the registered length | High |
| Bank account | a run of 8 to 17 digits (`min_account_digits`, default 8; single spaces or hyphens allowed) within 60 bytes after a keyword | the keyword (`account`, `acc no`, `acc. no`, `acc number`, `a/c`, `acct`, configurable) stands alone as a word, and the digits are not an amount or a quantity | Medium |
| Name | a name from `names`, as a whole word, any case | none | High |

**Amounts and quantities are never account or new-NIC numbers.** A digit run directly after `Rs`, `Rs.`, `LKR`, `USD`, `$`, the rupee sign `₨`, the Sinhala `රු` or the Tamil `ரூ` (with or without a dot), or `price`, or directly before `km`, `cc`, `lakh`, `lakhs`, `mn`, `million`, `mil`, `k`, `m`, `l` or `/=`, is left alone.

### Sri Lankan sources, and what is not verified

* **Numbering plan.** The mobile level `07` and the geographic trunk codes come from the Telecommunications Regulatory Commission of Sri Lanka's *Sri Lanka Numbering Plan: the 10-digit open numbering system*, revised 22 October 2009 (`http://www.trc.gov.lk/content/files/policy/numberingmanual.pdf`; section 2.1.4 allocates level `07` to mobile services and Annex 1 lists the geographic codes). I read the document itself on 2026-10-20. Annex 1 lists 29 codes: `11 21 23 24 25 26 27 31 32 33 34 35 36 37 38 41 45 47 51 52 54 55 57 63 65 66 67 81 91`. The same annex reserves `12` (Colombo expansion), `17` (satellite), `19` (private networks) and `60` (temporary overlays), which are not accepted. **The document is from 2009**: codes allocated since are not in the list, so add them with `extra_area_codes` if you meet them. National numbers are 10 digits: `0`, a 2-digit code, 7 digits. I did not check the current plan (the TRCSL site was reachable only as this 2009 PDF); an operator's mobile prefix inside `07x` is not checked, because the plan allocates the whole level.
* **NIC.** The year, day-number and serial structure above is the commonly published layout. **No checksum is validated**: I could not verify an official check-digit algorithm, so the last digit is accepted as it is. The day-number range accepts 366 in every year: the numbers are commonly described as counting February as 29 days in every year, and accepting one extra day number only errs on the private side. This was not checked against an official source.
* **Cards and IBANs.** Network prefixes and lengths follow the public ISO/IEC 7812 allocations and the networks' published ranges as I know them, and the IBAN lengths are from the ISO 13616 registry for the countries in the table. They were not re-checked against the registries for this release.
* **Bank accounts have no common format in Sri Lanka** (lengths differ by bank), which is why an account number is only recognized next to a keyword. Sinhala and Tamil keywords are **not built in**: add them with `account_keywords` once you have real examples.

## How it masks

| `Mask` | Result | Notes |
|---|---|---|
| `Placeholder` (default) | `[PHONE]`, `[NIC]`, `[CARD]`, `[ACCOUNT]`, `[IBAN]`, `[EMAIL]`, `[NAME]` | keeps none of the original characters |
| `KeepLast(n)` | `[CARD *1111]` | the last `n` (1 to 4) characters of a phone, card, account or IBAN; never for NIC, email or names |
| `Pseudonym(key)` | `[PHONE:7f3a9c217f3a9c21]` | 8 bytes (16 hex characters) of HMAC-SHA256 over the kind and the normalized value under your key: the same number written in different ways gets one token, different values get different tokens, another key gives other tokens. Keep the key out of source control; without it a token cannot be linked back. |
| `keep_email_domain` | `[EMAIL @example.org]` | with a placeholder or `KeepLast`; not with a pseudonym |

Placeholders are never matched again, and the text is masked again until a pass finds nothing, so redacting twice changes nothing. (Masking a value can expose its neighbour: a number glued to an IBAN has no word boundary until the number is masked. `Redacted::findings` lists the first pass with positions; `Redacted::later` lists the kinds found on later passes, almost always none, and `counts()` adds both. The fuzzer found two such cases in the first CI run and both are regression tests.)

## Events

`redact_event` and `redact_report` mask the message body, attachment file names and addresses, participant ids, the conversation id and (with `redact_display_names` or a `names` list) names. **`raw` is dropped by default** (it is the unredacted source); with `drop_raw` off every string in it is masked instead. Kept: timestamps, direction, channel, call duration and outcome, attachment kinds and mime types, the platform's own ids (`external_id`, `source_id`) and, by default, the event id. An id is a hash of the content, not the content, but it can confirm a guess ([IDS.md](IDS.md)); `IdPolicy::Pseudonymize` (with a pseudonym key) replaces it with a keyed pseudonym that is still the same for the same event. Warnings are kept: msgkit never puts message content into a warning.

## False positives and false negatives

* **Validators** (Luhn, mod 97, NIC structure, the numbering plan, `normalize_phone`) decide every match; shape alone is never enough for a High finding.
* **Boundaries:** a match must not sit inside a longer run of digits or letters.
* **Context:** an account number needs a keyword; amounts and quantities are excluded.
* **Overlaps:** when matches overlap, IBAN, then card, NIC, email, phone, account and name win in that order, and every character is masked once.
* **Control:** `kinds` switches kinds off, `min_confidence` drops Medium findings, `allow_exact` and `allow_patterns` keep chosen values (your own support number), `detect` shows what would be masked without changing anything, and every report carries counts, never values.
* **A negative corpus** (`crates/msgkit-redact/tests/negatives.rs`) must never be masked: Sri Lankan marketplace text (prices in rupees, lakhs and millions, in English, Sinhala and Tamil; mileage; vehicle registration numbers; years; dates and times) and other everyday numbers. A change that masks any of them fails the build.
* **Known false positives:** a 13 to 19 digit number that passes Luhn and starts like a card (some barcodes, some long references); a 12 digit number from 1900 to 2010 that is not a NIC (for example a compact date-time from those years); a digit run after "account" that is not an account number.
* **Known false negatives:** numbers written with unusual separators, spelled out in words, split over lines or inside a longer token; Unicode digits; an IBAN in lower case; a national phone number in another country; a bank account without a keyword; every name that is not in `names`.

## What it does not do

* It does not make data anonymous. Names, addresses, places, free-text descriptions and context can identify a person without any of the above.
* It does not decide whether you may process the data ([PRIVACY.md](PRIVACY.md)).
* It does not look at images, audio or attachments' content.
* It is Rust only. A `redact` operation for the Python, Node.js and Kotlin packages and the HTTP service is on the backlog; it needs the schema, the error taxonomy and the conformance suite to cover it first.

## Testing

All test data is synthetic: the card numbers payment providers publish for testing, invented phone numbers and NICs, and `example.org` addresses. The crate has per-rule tests (`tests/detection.rs`), the negative corpus, masking and configuration tests, event tests, property tests (no panic, idempotence, ordered and disjoint findings, recognized identifiers never survive, text without digits is untouched) and a fuzz target (`fuzz/fuzz_targets/redact.rs`, seeds in `fuzz/corpus/redact/`, replayed on every platform by `tests/fuzz_corpus.rs`).
