# Security policy

## Reporting a vulnerability

Please report security issues **privately**, not in public issues. Contact the maintainers through the repository's private vulnerability reporting (GitHub: *Security → Report a vulnerability*) or the maintainer email listed in the repository profile. Include a description, affected version and reproduction steps. You can expect an acknowledgement within a few days.

## Scope of particular interest

* Signature verification bypasses in webhook handling (`msgkit-core::webhook`, `msgkit-service`).
* Memory-safety issues at the C ABI boundary (`msgkit-ffi`).
* Panics or unbounded resource use triggered by untrusted input (exports, webhook bodies).
* Authentication bypass on the HTTP sidecar.

## Supported versions

Only the latest released minor version receives security fixes while the project is pre-1.0.
