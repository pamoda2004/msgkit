/*
 * msgkit C ABI. See crates/msgkit-ffi/src/lib.rs and docs/API.md.
 *
 * All strings are NUL-terminated UTF-8. Strings returned by msgkit_call are
 * owned by the caller and must be released with msgkit_free exactly once.
 */
#ifndef MSGKIT_H
#define MSGKIT_H

#ifdef __cplusplus
extern "C" {
#endif

/* Runs `op` on the JSON document `input_json`; returns a JSON envelope
 * {"ok":true,"data":...} or {"ok":false,"error":{"code","message"}}.
 * Returns NULL only if the result could not be allocated. */
char *msgkit_call(const char *op, const char *input_json);

/* Releases a string returned by msgkit_call. NULL is a no-op. */
void msgkit_free(char *s);

/* Library version. Static string, do not free. */
const char *msgkit_version(void);

#ifdef __cplusplus
}
#endif

#endif /* MSGKIT_H */
