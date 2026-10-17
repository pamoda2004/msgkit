package io.github.pamoda2004.msgkit

/**
 * Error reported by msgkit.
 *
 * [code] is one of the stable codes documented in docs/API.md:
 * `invalid_input`, `parse_error`, `unknown_op`, `storage_error`, `internal_error`.
 */
public class MsgkitException(
    public val code: String,
    override val message: String,
) : RuntimeException("$code: $message")
