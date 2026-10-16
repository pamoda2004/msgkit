# msgkit for Kotlin (JVM and Android)

Import **SMS, WhatsApp and Messenger** messages and calls into one normalized model.

```kotlin
val report = Msgkit.parseWhatsAppExport(
    File("_chat.txt").readText(),
    WhatsAppExportOptions(selfName = "Alice", utcOffsetMinutes = 330),
)
report.warnings.forEach { println("warning: $it") }
report.messages.forEach { println("${it.timestamp} ${it.sender.id}: ${it.body}") }
report.calls.forEach { println("${it.timestamp} ${it.outcome} ${it.durationSecs}s") }
```

Two artifacts, one API:

| Artifact | Use it for | Native libraries |
|---|---|---|
| `io.github.pamoda2004:msgkit` | JVM servers and desktop apps (Linux, macOS, Windows) | bundled in the JAR, extracted by JNA at run time |
| `io.github.pamoda2004:msgkit-android` | Android apps (API 26+) | `libmsgkit_ffi.so` for `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86` |

```kotlin
// build.gradle.kts of a JVM app
dependencies { implementation("io.github.pamoda2004:msgkit:0.1.0") }

// build.gradle.kts of an Android app
dependencies { implementation("io.github.pamoda2004:msgkit-android:0.1.0") }
```

> The `dev.msgkit` group id is a placeholder. Change `group` in both `build.gradle.kts` files before publishing under your own namespace.

## API

All methods are on the thread-safe `Msgkit` object and throw `MsgkitException` (with a stable `code`) on failure. Bad records inside valid input never throw; they are listed in `ImportReport.warnings`.

| Method | Purpose |
|---|---|
| `parseWhatsAppExport(text, options)` | WhatsApp "Export chat" `.txt` |
| `parseFacebookExport(threadJson, options)` | one Messenger `message_N.json` |
| `normalizeDevice(batch, options)` | Android SMS / call-log rows |
| `normalizeWhatsAppWebhook(rawBody)` | WhatsApp Business Cloud API webhook |
| `normalizeMessengerWebhook(rawBody)` | Messenger Platform webhook |
| `verifySignature(secret, body, signature)` | `X-Hub-Signature-256` check on the **raw** body |
| `version()` | native library and JSON schema versions |
| `dispatchRaw(op, inputJson)` | low-level: raw JSON envelope for any operation |

Results are Kotlin data classes (`Event` is a sealed interface: `Message` or `Call`) that mirror the [JSON Schemas](../../schema/README.md). `Event.instant` gives a `java.time.Instant`. Decoding ignores unknown fields, so a newer native library does not break older apps.

> `normalize*Webhook` does **not** verify signatures. Call `verifySignature` with the same raw body first.

### Reading Android's SMS and call log

msgkit normalizes rows; reading them needs the `READ_SMS` / `READ_CALL_LOG` runtime permissions in your app. See [docs/ANDROID_COLLECTOR.md](../../docs/ANDROID_COLLECTOR.md) for the cursor code and the Google Play policy caveats.

## Notes

* **Charset.** Strings cross the native boundary as explicit UTF-8 bytes, not JNA `String`s (which use the platform default charset and would corrupt non-ASCII text on many Windows setups). A test covers Sinhala, emoji and accents.
* **R8/ProGuard.** `msgkit-android` ships consumer rules that keep JNA and the native interface.
* **JNA.** The JVM artifact depends on `net.java.dev.jna:jna`; the Android artifact depends on the `@aar` flavour (which carries `libjnidispatch.so`) and excludes the JVM jar.
* **16 KiB pages.** The 64-bit Android libraries are linked with 16 KiB alignment, as required for Android 15+ devices and Google Play.
* **Overriding the library.** Set `MSGKIT_LIB=/path/to/libmsgkit_ffi.so` (or use `-Djna.library.path`) to use a specific build.

## Development

Requirements: JDK 17+, the Android SDK and NDK (for the Android module), and a Rust toolchain.

```bash
cargo build -p msgkit-ffi --release            # native library for this machine
./gradlew :msgkit:test                         # JVM tests (find target/release or target/debug automatically)

# Android
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
python scripts/build_android_libs.py           # fills msgkit-android/src/main/jniLibs
./gradlew :msgkit-android:assembleRelease      # build/outputs/aar/msgkit-android-release.aar
./gradlew :msgkit-android:connectedDebugAndroidTest   # needs a running emulator or device
```

`ANDROID_HOME` must point at the SDK (or create `local.properties` with `sdk.dir=...`). For desktop JARs that bundle natives, place each library under `msgkit/natives/<jna-platform>/` (for example `win32-x86-64/msgkit_ffi.dll`, `linux-x86-64/libmsgkit_ffi.so`, `darwin-aarch64/libmsgkit_ffi.dylib`) before running `./gradlew :msgkit:jar`; the release workflow does this.
