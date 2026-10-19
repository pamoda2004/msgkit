//! Lists every bundled time zone with a fingerprint of its UTC offsets from 1970 to 2040.
//!
//! Used to review a `jiff` / `jiff-tzdb` update (CONTRIBUTING.md, "Time-zone data"): run it before and after the
//! bump and diff the two outputs; every changed line is a zone whose computed UTC timestamps changed.
//!
//!     cargo run --release -p msgkit-core --example tz_fingerprint > tz-before.txt
//!     (bump the dependencies)
//!     cargo run --release -p msgkit-core --example tz_fingerprint > tz-after.txt
//!     diff tz-before.txt tz-after.txt

use jiff::tz::TimeZone;
use jiff::Timestamp;

fn main() {
    println!("# tzdata {}", msgkit_core::tzdata_version());
    let mut names: Vec<&str> = jiff_tzdb::available().collect();
    names.sort_unstable();
    let (start, end) = (0_i64, 2_208_988_800_i64); // 1970-01-01 and 2040-01-01
    for name in names {
        let Ok(zone) = TimeZone::get(name) else { continue };
        // FNV-1a over the offset in effect every 3 hours: any change of an offset (or of when it applies) that
        // lasts at least 3 hours changes the fingerprint.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut t = start;
        while t < end {
            let offset = zone.to_offset(Timestamp::from_second(t).expect("in range")).seconds();
            for byte in offset.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            t += 3 * 3600;
        }
        println!("{name}\t{hash:016x}");
    }
}
