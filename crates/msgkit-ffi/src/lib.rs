//! C ABI for msgkit.
//!
//! One function does the work: [`msgkit_call`] takes an operation name and a
//! JSON document and returns a JSON envelope (see `msgkit_core::api`). Any
//! language with a C FFI (Python `ctypes`, C# P/Invoke, Go `cgo`, Java
//! Panama/JNA, Swift, Dart, PHP FFI...) can use msgkit through it.
//!
//! Memory rules:
//! * Strings are NUL-terminated UTF-8.
//! * Every string returned by `msgkit_call` is owned by the caller and must be
//!   released with [`msgkit_free`], exactly once.
//! * Inputs are only borrowed for the duration of the call.
//! * Panics never cross the boundary; they become an `internal_error` envelope.

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

const INTERNAL_ERROR: &str = r#"{"ok":false,"error":{"code":"internal_error","message":"internal error"}}"#;
const NULL_ARGUMENT: &str = r#"{"ok":false,"error":{"code":"invalid_input","message":"null argument"}}"#;
const BAD_UTF8: &str =
    r#"{"ok":false,"error":{"code":"invalid_input","message":"argument is not valid UTF-8"}}"#;

fn into_c(s: &str) -> *mut c_char {
    // JSON text never contains a raw NUL byte, so this cannot fail in practice.
    CString::new(s).map_or(ptr::null_mut(), CString::into_raw)
}

/// Runs `op` on the JSON document `input_json` and returns a JSON envelope.
///
/// Returns NULL only if memory for the result could not be prepared.
///
/// # Safety
/// `op` and `input_json` must each be NULL or point to a valid NUL-terminated
/// string that stays alive for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn msgkit_call(op: *const c_char, input_json: *const c_char) -> *mut c_char {
    if op.is_null() || input_json.is_null() {
        return into_c(NULL_ARGUMENT);
    }
    // SAFETY: both pointers are non-null and the caller guarantees validity.
    let (op, input) = unsafe { (CStr::from_ptr(op), CStr::from_ptr(input_json)) };
    let (Ok(op), Ok(input)) = (op.to_str(), input.to_str()) else {
        return into_c(BAD_UTF8);
    };
    let out = catch_unwind(AssertUnwindSafe(|| msgkit_core::api::dispatch(op, input)))
        .unwrap_or_else(|_| INTERNAL_ERROR.to_owned());
    into_c(&out)
}

/// Frees a string returned by [`msgkit_call`]. Passing NULL is a no-op.
///
/// # Safety
/// `s` must be NULL or a pointer previously returned by `msgkit_call` that has
/// not been freed yet.
#[no_mangle]
pub unsafe extern "C" fn msgkit_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: the caller guarantees `s` came from `CString::into_raw` here.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// The library's version as a static NUL-terminated string. Do not free it.
#[no_mangle]
pub extern "C" fn msgkit_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(op: &str, input: &str) -> serde_json::Value {
        let op = CString::new(op).unwrap();
        let input = CString::new(input).unwrap();
        // SAFETY: valid C strings; result freed exactly once below.
        let raw = unsafe { msgkit_call(op.as_ptr(), input.as_ptr()) };
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_owned();
        unsafe { msgkit_free(raw) };
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn round_trip() {
        let v = call("parse_whatsapp_export", r#"{"text":"01/02/2024, 10:00 - A: hi\n"}"#);
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["events"][0]["body"], "hi");
    }

    #[test]
    fn errors_are_envelopes() {
        assert_eq!(call("nope", "{}")["error"]["code"], "unknown_op");
        assert_eq!(call("version", "garbage")["ok"], true, "version ignores input");
        assert_eq!(call("parse_whatsapp_export", "garbage")["error"]["code"], "invalid_input");
    }

    #[test]
    fn null_arguments_are_rejected_not_dereferenced() {
        // SAFETY: NULL is explicitly allowed.
        let raw = unsafe { msgkit_call(ptr::null(), ptr::null()) };
        let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_owned();
        unsafe { msgkit_free(raw) };
        assert!(text.contains("null argument"));
        unsafe { msgkit_free(ptr::null_mut()) };
    }

    #[test]
    fn version_is_a_c_string() {
        let v = unsafe { CStr::from_ptr(msgkit_version()) }.to_str().unwrap();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }
}
