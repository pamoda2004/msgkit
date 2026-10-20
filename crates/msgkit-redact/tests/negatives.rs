//! Text that must never be masked: Sri Lankan marketplace talk (prices, mileage, vehicle registration numbers, years,
//! dates and times) and other everyday numbers. A change that masks any of these is a regression.

use msgkit_redact::{RedactConfig, Redactor};

/// Prices, in the many ways Sri Lankan sellers write them.
const PRICES: &[&str] = &[
    "Rs. 6,500,000",
    "Rs.6,500,000",
    "Rs 6500000",
    "6500000",
    "65L",
    "65 L",
    "65 lakhs",
    "65 lakh",
    "Rs. 65 lakhs negotiable",
    "\u{0dbd}\u{0d9a}\u{0dca}\u{0dc2} 65",
    "\u{0dbb}\u{0dd4}. 6,500,000",
    "\u{0dbb}\u{0dd4}. 65 \u{0dbd}\u{0d9a}\u{0dca}\u{0dc2}",
    "\u{0bb0}\u{0bc2}. 6,500,000",
    "6.5M",
    "6.5 million",
    "LKR 6,500,000",
    "LKR 6500000",
    "Price 6500000/=",
    "Price Rs.12500000/=",
    "12500000/=",
    "Rs. 12500000",
    "Rs. 120000000",
    "$ 25000",
    "USD 25000",
    "Rs 1,250,000.00",
];

/// Mileage and other vehicle details.
const VEHICLES: &[&str] = &[
    "85,000 km",
    "85000 km",
    "85000km",
    "85k km",
    "85 000 km",
    "1500cc",
    "1,500 cc",
    "CAB-1234",
    "WP CAB-1234",
    "WP  CAB 1234",
    "CP KA-5678",
    "58-1234",
    "Registered 2018",
    "Toyota Aqua 2015",
    "2015",
    "Year 2015 | 85,000 km | 1500cc | Rs. 6,500,000",
    "WP CAB-1234 | 2015 | 85,000 km | Rs. 6,500,000",
    "Toyota Aqua 2015, 85,000 km, Rs. 6,500,000 negotiable, WP CAB-1234",
    "Leasing from any bank available. Price Rs. 12500000",
    "bank leasing available Rs. 12500000",
    "Bank loan: Rs. 12500000 monthly 125000",
    "Current price Rs. 65 lakhs",
];

/// Dates, times and other numbers that look like identifiers but are not.
const DATES_AND_OTHERS: &[&str] = &[
    "2024-03-01",
    "2024-03-01T10:00:00Z",
    "01/02/2024, 10:00",
    "01/02/2024, 10:00 - ",
    "31.12.2023",
    "10:30 PM",
    "10.30 pm",
    "12 : 30",
    "202410201200",
    "20241020",
    "IMG_20241020_120000.jpg",
    "ORD-20241020-0042",
    "v2.3.4",
    "123e4567-e89b-12d3-a456-426614174000",
    "978-3-16-148410-0",
    "ISBN 9783161484100",
    "No. 25, Galle Road, Colombo 03",
    "2 bedroom, 1500 sq ft, 10 perches",
    "75 km/h",
    "0.5 km",
    "A 1 B 2 C 3",
    "id 0000000000000000",
    "00123456",
    "bank leasing 12500000",
    "Current price 65000000",
    "savings 12500000",
    "a1b2c3d4e5f60718293a4b5c6d7e8f90",
];

fn corpus() -> impl Iterator<Item = &'static str> {
    PRICES.iter().chain(VEHICLES).chain(DATES_AND_OTHERS).copied()
}

#[test]
fn marketplace_text_is_never_masked() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    for text in corpus() {
        let found = r.detect(text);
        assert_eq!(found, vec![], "{text:?} was found");
        assert_eq!(r.redact_text(text).text, text);
    }
}

#[test]
fn marketplace_text_is_not_masked_by_the_strictest_settings_either() {
    // Every kind on, card prefix check off, an extra area code: still no false positive.
    let r =
        Redactor::new(RedactConfig::default().with_strict_card_prefix(false).with_extra_area_codes(["12"]))
            .unwrap();
    for text in corpus() {
        assert_eq!(r.redact_text(text).text, text, "{text:?}");
    }
}

#[test]
fn only_the_phone_number_in_an_advertisement_is_masked() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    for (ad, expected) in [
        (
            "Toyota Aqua 2015, 85,000 km, Rs. 6,500,000 negotiable. WP CAB-1234. Call 0700000000",
            "Toyota Aqua 2015, 85,000 km, Rs. 6,500,000 negotiable. WP CAB-1234. Call [PHONE]",
        ),
        (
            "Honda Vezel 2016 | 65 lakhs | 60k km | +94 70 000 0000 | bank leasing 12500000",
            "Honda Vezel 2016 | 65 lakhs | 60k km | [PHONE] | bank leasing 12500000",
        ),
        ("WhatsApp 070 000 0000 / 011 0000000, Rs. 6.5M", "WhatsApp [PHONE] / [PHONE], Rs. 6.5M"),
    ] {
        assert_eq!(r.redact_text(ad).text, expected);
    }
}

#[test]
fn identifiers_are_still_masked_next_to_marketplace_numbers() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    // A price and an account number in one message: only the account number goes.
    let text = "Rs. 6,500,000 please deposit to bank account 100200300400";
    assert_eq!(r.redact_text(text).text, "Rs. 6,500,000 please deposit to bank account [ACCOUNT]");
    // A card number after a price.
    assert_eq!(r.redact_text("Rs. 85000 paid with 4111 1111 1111 1111").text, "Rs. 85000 paid with [CARD]");
}
