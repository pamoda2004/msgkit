//! # msgkit-core
//!
//! A portable toolkit for importing messages and calls from SMS, WhatsApp and
//! Messenger into one normalized model.
//!
//! The crate has no I/O and no platform dependencies. It turns *data you
//! already have* (export files, webhook bodies, rows read from a device) into
//! [`Event`]s. Where those events are stored is up to the host application.
//!
//! * [`model`]: the normalized types: the public contract.
//! * [`whatsapp_export`], [`facebook`]: parsers for user-downloaded exports.
//! * [`webhook`]: signature checks and normalizers for official business APIs.
//! * [`device`]: normalizer for Android SMS/call-log rows.
//! * [`api`]: JSON-in/JSON-out dispatcher used by every language binding.
//! * [`store`]: the storage trait and an in-memory implementation.

pub mod api;
pub mod device;
pub mod error;
pub mod facebook;
pub mod id;
pub mod model;
pub mod phone;
pub mod store;
mod tz;
pub mod webhook;
pub mod whatsapp_export;

pub use error::{Error, Result};
pub use model::*;
pub use tz::tzdata_version;

/// Version of the JSON data contract (`schema/`). Bumped independently of the
/// crate version: minor for additive changes, major for breaking ones.
pub const SCHEMA_VERSION: &str = "1.1.0";
