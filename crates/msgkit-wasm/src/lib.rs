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
//! Operations and the JSON envelope are exactly those of `msgkit_core::api`.
//! The module is single-threaded, so the "last result length" is a plain
//! global.

#![deny(unsafe_op_in_unsafe_fn)]

use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};

static LAST_LEN: AtomicUsize = AtomicUsize::new(0);

const BAD_UTF8: &str =
    r#"{"ok":false,"error":{"code":"invalid_input","message":"argument is not valid UTF-8"}}"#;

/// Allocates `len` bytes inside the module and returns the address.
#[no_mangle]
pub extern "C" fn msgkit_alloc(len: usize) -> *mut u8 {
    // Always at least one byte so the pointer is real and `msgkit_dealloc`
    // can rebuild the same boxed slice from `len` alone.
    Box::into_raw(vec![0u8; len.max(1)].into_boxed_slice()).cast::<u8>()
}

/// Frees memory obtained from [`msgkit_alloc`] or returned by [`msgkit_call`].
///
/// # Safety
/// `ptr` must come from this module and `len` must be the length it was
/// allocated or returned with. Each pointer may be freed once.
#[no_mangle]
pub unsafe extern "C" fn msgkit_dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        // SAFETY: every pointer handed out is the start of a `Box<[u8]>` of
        // `max(len, 1)` bytes (see `msgkit_alloc` and `msgkit_call`).
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len.max(1))) });
    }
}

/// Runs an operation. Returns the address of the UTF-8 JSON envelope; its
/// length is available from [`msgkit_last_len`] until the next call.
///
/// # Safety
/// Both `(ptr, len)` ranges must be valid for reads of `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn msgkit_call(
    op_ptr: *const u8,
    op_len: usize,
    in_ptr: *const u8,
    in_len: usize,
) -> *mut u8 {
    // SAFETY: the host guarantees both ranges are valid and initialised.
    let (op, input) =
        unsafe { (slice::from_raw_parts(op_ptr, op_len), slice::from_raw_parts(in_ptr, in_len)) };
    let out = match (std::str::from_utf8(op), std::str::from_utf8(input)) {
        (Ok(op), Ok(input)) => msgkit_core::api::dispatch(op, input),
        _ => BAD_UTF8.to_owned(),
    };
    // Envelopes are never empty, so the boxed slice has exactly `len` >= 1
    // bytes, which is what `msgkit_dealloc` reconstructs.
    let boxed = out.into_bytes().into_boxed_slice();
    debug_assert!(!boxed.is_empty());
    LAST_LEN.store(boxed.len(), Ordering::Relaxed);
    Box::into_raw(boxed).cast::<u8>()
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

    #[test]
    fn call_round_trip_and_free() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let (op, input) = (b"version", b"{}");
        // SAFETY: valid slices; result freed once with its reported length.
        unsafe {
            let out = msgkit_call(op.as_ptr(), op.len(), input.as_ptr(), input.len());
            let n = msgkit_last_len();
            let text = std::str::from_utf8(slice::from_raw_parts(out, n)).unwrap().to_owned();
            msgkit_dealloc(out, n);
            assert!(text.contains(r#""ok":true"#), "{text}");
        }
    }

    #[test]
    fn alloc_dealloc_zero_length() {
        let p = msgkit_alloc(0);
        unsafe { msgkit_dealloc(p, 0) };
    }

    #[test]
    fn invalid_utf8_is_an_error_envelope() {
        let _guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let bad = [0xffu8, 0xfe];
        unsafe {
            let out = msgkit_call(bad.as_ptr(), bad.len(), b"{}".as_ptr(), 2);
            let n = msgkit_last_len();
            let text = std::str::from_utf8(slice::from_raw_parts(out, n)).unwrap().to_owned();
            msgkit_dealloc(out, n);
            assert!(text.contains("invalid_input"));
        }
    }
}
