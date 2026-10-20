# Security policy

msgkit parses personal messages, so security and privacy reports are taken seriously.

## Reporting a vulnerability

Please report vulnerabilities **privately**:

1. Use GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability** (https://github.com/pamoda2004/msgkit/security/advisories/new).
2. If that option is not available, open a public issue titled "Security contact request" **without any details**, and a maintainer will arrange a private channel.

Please include the affected version, a description, and a minimal reproduction using **synthetic data only**. Never attach real exports, message content or credentials.

You can expect an acknowledgement within 7 days. Fixes for confirmed issues are prepared privately, released, and then disclosed in the changelog with credit unless you prefer otherwise.

## Scope of particular interest

* Signature verification bypasses in webhook handling (`msgkit-core::webhook`, `msgkit-service`).
* Memory-safety issues at the C ABI and WebAssembly boundaries (`msgkit-ffi`, `msgkit-wasm`).
* Panics, crashes or unbounded resource use triggered by untrusted input (exports, webhook bodies).
* Authentication bypass on the HTTP service.
* Any path by which message content or personal data can reach logs, error messages or telemetry.

## Verifying a release

Release files are signed with Sigstore (keyless), come with build provenance and a CycloneDX SBOM, and can be checked with `cosign` and the GitHub CLI: see [docs/VERIFYING.md](docs/VERIFYING.md). Nothing has been released yet, so there is nothing to verify today; if you see a "msgkit" package that this repository did not publish, treat it as untrusted and tell us.

## Supported versions

While the project is pre-1.0, only the latest released minor version receives security fixes.

## Safe harbour

Good-faith research that follows this policy, avoids accessing other people's data, and gives us reasonable time to respond will not be pursued legally.