# Consumer projects: using msgkit from another project

Four small, **separate** projects that use msgkit the way an outside project would: as an *installed dependency*, not from source. Each one is the same program (chat statistics plus a webhook-signature check) in a different language, and **all four print byte-identical output**:

```
events: 8
messages: 7, calls: 1
per sender:
  Alice: 3
  Bob: 4
attachments: 2
missed calls: 1
first: 2023-12-31T16:11:00Z  last: 2023-12-31T18:40:00Z
signature valid: true
tampered body valid: false
```

If your project can do what these do, msgkit works as a library for it. They read a sample chat from the repository's test fixtures; pass a path as the first argument to use another export.

| Project | Consumes | Needs |
|---|---|---|
| [`rust/`](rust) | the `msgkit-core` crate (by path; by git URL or crates.io once published) | Rust toolchain |
| [`node/`](node) | `@pamoda2004/msgkit` from an npm tarball | Node 20+ and the wasm build |
| [`python/`](python) | the `msgkit-core` wheel (imported as `msgkit`), installed into a fresh virtual environment | Python 3.9+, `pip install build` |
| [`kotlin/`](kotlin) | `io.github.pamoda2004:msgkit` from the local Maven repository (natives bundled in the JAR) | JDK 17+, Android SDK variable set |

Commands below are for Windows CMD, run from the repository root. Replace `\` with `/` on macOS/Linux.

## Rust

```bat
cd examples\consumers\rust
cargo run
```

## Node.js

```bat
cd packages\node
node scripts\build.mjs
npm pack --pack-destination ..\..\examples\consumers\node
cd ..\..\examples\consumers\node
npm install --no-save pamoda2004-msgkit-0.2.0.tgz
node app.mjs
```

## Python

```bat
python -m pip install build
python packages\python\scripts\build_wheel.py --out examples\consumers\python\wheelhouse
cd examples\consumers\python
python -m venv .venv
for %f in (wheelhouse\*.whl) do .venv\Scripts\python -m pip install %f
.venv\Scripts\python app.py
```

The program runs from a virtual environment that contains only the installed wheel (no `PYTHONPATH`, no repository on the import path), so the native library it uses is the one bundled in the wheel.

## Kotlin / JVM

Publish the library, with the native library bundled in the JAR, to your local Maven repository, then build the consumer:

```bat
set "JAVA_HOME=C:\Program Files\Android\Android Studio\jbr"
set "ANDROID_HOME=%LOCALAPPDATA%\Android\Sdk"
cargo build -p msgkit-ffi --release
mkdir packages\kotlin\msgkit\natives\win32-x86-64
copy target\release\msgkit_ffi.dll packages\kotlin\msgkit\natives\win32-x86-64\
cd packages\kotlin
gradlew.bat :msgkit:publishToMavenLocal
cd ..\..\examples\consumers\kotlin
gradlew.bat run
```

`io.github.pamoda2004:msgkit:0.2.0` is now in `~/.m2`. The consumer's `build.gradle.kts` is the whole integration:

```kotlin
repositories { mavenLocal(); mavenCentral() }
dependencies { implementation("io.github.pamoda2004:msgkit:0.2.0") }
```

## Any other language

Run `msgkit-service` and call it over HTTP (see [`../http`](../http/README.md) and `openapi.yaml`), or load the C library (`crates/msgkit-ffi/include/msgkit.h`).
