//! WebAssembly build of msgkit.
//!
//! The module needs **no imports** and no generated glue: it exports a tiny
//! C-like ABI that any host with a `WebAssembly` API can drive (the Node
//! package in `packages/node` is the reference host).
//!
//! ```text
//! ptr = msgkit_alloc(len)                  host copies UTF-8 bytes into memory at ptr
//! out = msgkit_call(op_ptr, op_len, in_ptr, in_len)
//! n   = msgkit_last_len()                  byte length of the result at `out`
//! ...host reads n bytes at `out`...
//! msgkit_dealloc(out, n)                   free the result
//! msgkit_dealloc(op_ptr, op_len); msgkit_dealloc(in_ptr, in_len)
//! ```
//!
//! The module tracks every buffer it hands out: `msgkit_dealloc` ignores its length argument and any pointer that
//! is not live, and `msgkit_call` only reads ranges inside live buffers. `msgkit_alloc` returns 0 for a request
//! over 512 MiB.
//!
//! Operations and the JSON envelope are exactly those of `msgkit_core::api`.
//! The module is single-threaded, so the "last result length" is a plain
//! global.

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

static LAST_LEN: AtomicUsize = AtomicUsize::new(0);

/// Every live buffer this module handed out: start address to length. The host's pointers and lengths are
/// checked against it, so a wrong length or a repeated free can never make the module build a slice or a box
/// that the module did not allocate.
static LIVE: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

/// The largest single buffer `msgkit_alloc` hands out (512 MiB). A WebAssembly module has at most 4 GiB of
/// memory, and parsing needs several times the size of its input, so a larger request cannot succeed anyway.
const MAX_ALLOC_BYTES: usize = 512 * 1024 * 1024;

const BAD_UTF8: &str =
    r#"{"ok":false,"error":{"code":"invalid_input","message":"argument is not valid UTF-8"}}"#;
const BAD_RANGE: &str = r#"{"ok":false,"error":{"code":"invalid_input","message":"argument is not a live buffer of this module"}}"#;

fn live() -> MutexGuard<'static, BTreeMap<usize, usize>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records `boxed` as live and returns its address.
fn register(boxed: Box<[u8]>) -> *mut u8 {
    let len = boxed.len();
    let ptr = Box::into_raw(boxed).cast::<u8>();
    live().insert(ptr as usize, len);
    ptr
}

/// Whether `[ptr, ptr + len)` lies inside one live buffer.
fn is_live_range(ptr: *const u8, len: usize) -> bool {
    let start = ptr as usize;
    let live = live();
    live.range(..=start)
        .next_back()
        .is_some_and(|(&base, &size)| start.checked_add(len).is_some_and(|end| end <= base + size))
}

/// Allocates `len` bytes inside the module and returns the address, or NULL (0) when `len` is more than the
/// module will hand out in one buffer (512 MiB).
#[no_mangle]
pub extern "C" fn msgkit_alloc(len: usize) -> *mut u8 {
    if len > MAX_ALLOC_BYTES {
        return std::ptr::null_mut();
    }
    // Always at least one byte so the pointer is real and every buffer is tracked.
    register(vec![0u8; len.max(1)].into_boxed_slice())
}

/// Frees memory obtained from [`msgkit_alloc`] or returned by [`msgkit_call`].
///
/// The module knows the length of every buffer it handed out, so `_len` is **ignored** (it stays in the
/// signature so existing hosts keep working). A pointer that is not the start of a live buffer of this module,
/// including one that was already freed, is ignored.
///
/// # Safety
/// Nothing beyond what the signature says: the function never trusts `ptr` or `_len`. It is `unsafe` only to
/// keep the exported signature unchanged.
#[no_mangle]
pub unsafe extern "C" fn msgkit_dealloc(ptr: *mut u8, _len: usize) {
    let size = live().remove(&(ptr as usize));
    if let Some(size) = size {
        // SAFETY: `ptr` was the start of a `Box<[u8]>` of exactly `size` bytes (see `register`), and removing it
        // from `LIVE` means no other free can reach it.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, size)) });
    }
}

/// Runs an operation. Returns the address of the UTF-8 JSON envelope; its
/// length is available from [`msgkit_last_len`] until the next call.
///
/// Both ranges must lie inside buffers from [`msgkit_alloc`]; otherwise the envelope is an `invalid_input` error.
///
/// # Safety
/// Nothing beyond what the signature says: the ranges are checked against the module's live buffers.
#[no_mangle]
pub unsafe extern "C" fn msgkit_call(
    op_ptr: *const u8,
    op_len: usize,
    in_ptr: *const u8,
    in_len: usize,
) -> *mut u8 {
    let range = |ptr: *const u8, len: usize| -> Option<&'static [u8]> {
        if len == 0 {
            Some(&[])
        } else if !ptr.is_null() && is_live_range(ptr, len) {
            // SAFETY: the range lies inside one live buffer, which stays allocated during this call (the module
            // is single-threaded and the host cannot free it while the call runs).
            Some(unsafe { slice::from_raw_parts(ptr, len) })
        } else {
            None
        }
    };
    let out = match (range(op_ptr, op_len), range(in_ptr, in_len)) {
        (Some(op), Some(input)) => match (std::str::from_utf8(op), std::str::from_utf8(input)) {
            (Ok(op), Ok(input)) => msgkit_core::api::dispatch(op, input),
            _ => BAD_UTF8.to_owned(),
        },
        _ => BAD_RANGE.to_owned(),
    };
    // Envelopes are never empty, so the boxed slice is at least one byte, as every tracked buffer is.
    let boxed = out.into_bytes().into_boxed_slice();
    debug_assert!(!boxed.is_empty());
    LAST_LEN.store(boxed.len(), Ordering::Relaxed);
    register(boxed)
}

/// Byte length of the result of the most recent [`msgkit_call`].
#[no_mangle]
pub extern "C" fn msgkit_last_len() -> usize {
    LAST_LEN.load(Ordering::Relaxed)
}

/// Pointer to a NUL-terminated version string (static; do not free).
#[no_mangle]
pub extern "C" fn msgkit_version() -> *const u8 {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr()
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    /// The wasm ABI keeps the last result length in a global (the real module
    /// is single-threaded), so tests that call `msgkit_call` must not overlap.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Copies `bytes` into a buffer allocated by the module, like a host does.
    fn put(bytes: &[u8]) -> *mut u8 {
        let ptr = msgkit_alloc(bytes.len());
        // SAFETY: `ptr` is a live buffer of at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        ptr
    }

    /// Calls the module with raw pointers and frees the result.
    fn call_raw(op: *const u8, op_len: usize, input: *const u8, in_len: usize) -> String {
        // SAFETY: the module checks the ranges itself; the result is freed once with its reported length.
        unsafe {
            let out = msgkit_call(op, op_len, input, in_len);
            let n = msgkit_last_len();
            let text = std::str::from_utf8(slice::from_raw_parts(out, n)).unwrap().to_owned();
            msgkit_dealloc(out, n);
            text
        }
    }

    fn call(op: &[u8], input: &[u8]) -> String {
        let (op_ptr, in_ptr) = (put(op), put(input));
        let text = call_raw(op_ptr, op.len(), in_ptr, input.len());
        unsafe {
            msgkit_dealloc(op_ptr, op.len());
            msgkit_dealloc(in_ptr, input.len());
        }
        text
    }

    fn live_count() -> usize {
        live().len()
    }

    #[test]
    fn call_round_trip_and_free() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        let text = call(b"version", b"{}");
        assert!(text.contains(r#""ok":true"#), "{text}");
        assert_eq!(live_count(), before, "every buffer was freed");
    }

    #[test]
    fn alloc_dealloc_zero_length() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        let p = msgkit_alloc(0);
        assert_eq!(live_count(), before + 1);
        unsafe { msgkit_dealloc(p, 0) };
        assert_eq!(live_count(), before);
    }

    #[test]
    fn invalid_utf8_is_an_error_envelope() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(call(&[0xff, 0xfe], b"{}").contains("invalid_input"));
    }

    #[test]
    fn dealloc_ignores_a_wrong_length() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        // Too long and too short: either would be undefined behaviour if the module trusted it.
        let p = msgkit_alloc(16);
        unsafe { msgkit_dealloc(p, usize::MAX) };
        assert_eq!(live_count(), before, "freed once, with its real length");
        let q = msgkit_alloc(16);
        unsafe { msgkit_dealloc(q, 1) };
        assert_eq!(live_count(), before);
    }

    #[test]
    fn dealloc_ignores_a_repeated_or_foreign_pointer() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        let p = msgkit_alloc(8);
        unsafe {
            msgkit_dealloc(p, 8);
            msgkit_dealloc(p, 8); // double free
            msgkit_dealloc(std::ptr::null_mut(), 8);
            let mut on_the_stack = [0u8; 8];
            msgkit_dealloc(on_the_stack.as_mut_ptr(), 8); // not ours
        }
        assert_eq!(live_count(), before);
    }

    #[test]
    fn dealloc_ignores_an_interior_pointer_of_a_live_buffer() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        let p = msgkit_alloc(8);
        unsafe { msgkit_dealloc(p.wrapping_add(4), 4) };
        assert_eq!(live_count(), before + 1, "the buffer is still live");
        unsafe { msgkit_dealloc(p, 8) };
        assert_eq!(live_count(), before);
    }

    #[test]
    fn call_refuses_ranges_outside_live_buffers() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let before = live_count();
        let op = put(b"version");
        let stack = *b"{}";
        // An input that did not come from msgkit_alloc.
        let text = call_raw(op, 7, stack.as_ptr(), stack.len());
        assert!(text.contains("not a live buffer"), "{text}");
        // A length that runs past the end of a live buffer.
        let input = put(b"{}");
        let text = call_raw(op, 7, input, 1_000_000);
        assert!(text.contains("not a live buffer"), "{text}");
        unsafe {
            msgkit_dealloc(op, 7);
            msgkit_dealloc(input, 2);
        }
        assert_eq!(live_count(), before);
    }

    #[test]
    fn alloc_over_the_cap_returns_null() {
        assert!(msgkit_alloc(MAX_ALLOC_BYTES + 1).is_null());
        assert!(msgkit_alloc(usize::MAX).is_null());
    }

    #[test]
    fn an_over_limit_input_is_limit_exceeded_through_the_module() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let text = call(
            b"parse_whatsapp_export",
            br#"{"text":"01/02/2024, 10:00 - A: hi\n","options":{"limits":{"max_input_bytes":3}}}"#,
        );
        assert!(text.contains(r#""code":"limit_exceeded""#), "{text}");
    }
}
