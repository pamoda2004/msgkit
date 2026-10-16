package dev.msgkit.internal

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer

/**
 * The C ABI of `msgkit-ffi` (see `crates/msgkit-ffi/include/msgkit.h`).
 *
 * Strings cross the boundary as NUL-terminated UTF-8 byte arrays rather than
 * JNA `String`s: JNA encodes `String` with the platform default charset
 * (Windows-1252 on many Windows installs), which would corrupt non-ASCII text.
 */
@Suppress("FunctionName", "ktlint:standard:function-naming")
internal interface NativeLib : Library {
    /** Returns an owned, NUL-terminated UTF-8 JSON envelope; free it with [msgkit_free]. */
    fun msgkit_call(
        op: ByteArray,
        inputJson: ByteArray,
    ): Pointer?

    fun msgkit_free(s: Pointer)

    /** Static string; do not free. */
    fun msgkit_version(): Pointer
}

internal object NativeBridge {
    private const val LIBRARY = "msgkit_ffi"

    val lib: NativeLib by lazy {
        // MSGKIT_LIB points at an exact library file (handy in tests and unusual layouts).
        val explicit = System.getenv("MSGKIT_LIB")
        Native.load(explicit?.takeIf { it.isNotBlank() } ?: LIBRARY, NativeLib::class.java)
    }

    private fun nulTerminated(s: String): ByteArray = s.toByteArray(Charsets.UTF_8) + 0

    /** Runs `op` and returns the JSON response envelope. Thread-safe. */
    fun call(
        op: String,
        inputJson: String,
    ): String {
        val result =
            lib.msgkit_call(nulTerminated(op), nulTerminated(inputJson))
                ?: error("msgkit returned no result (allocation failure)")
        try {
            return result.getString(0, "UTF-8")
        } finally {
            lib.msgkit_free(result)
        }
    }

    fun version(): String = lib.msgkit_version().getString(0, "UTF-8")
}
