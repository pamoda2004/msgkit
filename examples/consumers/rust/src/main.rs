//! A tiny application that uses msgkit-core as an ordinary dependency.
//! Prints chat statistics; the same program exists for Node.js, Python and Kotlin and prints the same output.

use std::collections::BTreeMap;

use msgkit_core::webhook::{sign, verify_signature};
use msgkit_core::whatsapp_export::{parse, WhatsAppExportOptions};
use msgkit_core::{CallOutcome, Event};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../../crates/msgkit-core/tests/fixtures/whatsapp_android_dmy.txt").into());
    let text = std::fs::read_to_string(&path).expect("cannot read the chat file");
    let opts = WhatsAppExportOptions::default().with_self_name("Alice").with_utc_offset_minutes(330);
    let report = parse(&text, &opts);

    let mut per_sender: BTreeMap<String, usize> = BTreeMap::new();
    let (mut messages, mut calls, mut attachments, mut missed) = (0, 0, 0, 0);
    for event in &report.events {
        match event {
            Event::Message(m) => {
                messages += 1;
                attachments += m.attachments.len();
                *per_sender.entry(m.sender.id.clone()).or_default() += 1;
            }
            Event::Call(c) => {
                calls += 1;
                missed += usize::from(c.outcome == CallOutcome::Missed);
            }
        }
    }

    println!("events: {}", report.events.len());
    println!("messages: {messages}, calls: {calls}");
    println!("per sender:");
    for (name, n) in &per_sender {
        println!("  {name}: {n}");
    }
    println!("attachments: {attachments}");
    println!("missed calls: {missed}");
    let (first, last) = (report.events.first().unwrap(), report.events.last().unwrap());
    println!(
        "first: {}  last: {}",
        first.timestamp().format("%Y-%m-%dT%H:%M:%SZ"),
        last.timestamp().format("%Y-%m-%dT%H:%M:%SZ")
    );

    let body = r#"{"object":"page","entry":[]}"#;
    let good = sign(b"app-secret", body.as_bytes());
    println!("signature valid: {}", verify_signature(b"app-secret", body.as_bytes(), &good));
    println!("tampered body valid: {}", verify_signature(b"app-secret", format!("{body} ").as_bytes(), &good));
}
