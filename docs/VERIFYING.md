# Verifying a release

> **Nothing has been published yet.** There is no release, no tag and no package on PyPI, npm or Maven Central, and the package names are not reserved (see the decisions in `docs/AUDIT.md`). This page describes what the release workflow (`.github/workflows/release.yml`) is built to produce and how you would check it. None of it has been exercised on a real release.

Every file attached to a GitHub release of msgkit (the Python wheels, the npm package, the Kotlin JAR and AAR, and the SBOMs) comes with:

| What | File | Made by | Proves |
|---|---|---|---|
| Checksums | `SHA256SUMS` | the workflow | the file you have is the file that was released |
| A Sigstore signature | `<file>.sigstore.json` | `cosign sign-blob`, keyless | **this repository's release workflow, running for a version tag, produced the file**, and the signature was recorded in the public Sigstore transparency log |
| Build provenance | GitHub artifact attestation (SLSA) | `actions/attest` | which commit, workflow and runner built the file |
| An SBOM attestation | GitHub artifact attestation | `actions/attest` | the CycloneDX SBOM that belongs to the npm package, the wheels, the JAR and the AAR |
| A software bill of materials | `msgkit-<part>-<version>.cdx.json` | `scripts/sbom.py` | what is inside (CycloneDX 1.5) |

There are **no signing keys**: signing is keyless. Sigstore issues a certificate that lasts minutes to the workflow's GitHub identity (OIDC), and that identity is what you check.

**What this does not prove.** A valid signature says the file came from this repository's workflow; it does not say the code is free of bugs or that the repository itself was not compromised. It does not cover a Maven Central release (not automated: it needs a verified namespace and GPG), and a copy you build yourself from source will not match byte for byte unless you reproduce the whole toolchain.

## 1. Checksums

```bash
sha256sum --check SHA256SUMS --ignore-missing     # Linux
shasum -a 256 --check SHA256SUMS --ignore-missing # macOS
```

On Windows: `(Get-FileHash .\file.whl -Algorithm SHA256).Hash` and compare it with the line in `SHA256SUMS`. `SHA256SUMS` is itself signed and attested, so check its signature first (step 2).

## 2. The Sigstore signature (cosign)

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) (the release workflow uses v3.1.3), then, for each file:

```bash
cosign verify-blob \
  --bundle msgkit_core-0.2.0-py3-none-manylinux_2_28_x86_64.whl.sigstore.json \
  --certificate-identity-regexp '^https://github\.com/pamoda2004/msgkit/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  msgkit_core-0.2.0-py3-none-manylinux_2_28_x86_64.whl
```

`Verified OK` means all of these hold: the signature matches the bytes of the file; the certificate was issued by Sigstore to **that workflow file of that repository at a `vX.Y.Z` tag**; the issuer is GitHub Actions; and the signature is in the transparency log. Insist on the identity: a signature from some other repository or branch is a valid signature that proves nothing about msgkit. The pattern above accepts only tags; a *dry run* of the workflow signs with a `refs/heads/...` identity and is deliberately not accepted.

## 3. Build provenance and SBOM attestations (GitHub CLI)

With the [GitHub CLI](https://cli.github.com/) 2.49 or later:

```bash
gh attestation verify msgkit_core-0.2.0-py3-none-manylinux_2_28_x86_64.whl --repo pamoda2004/msgkit
# the SBOM attached to that file:
gh attestation verify msgkit_core-0.2.0-py3-none-manylinux_2_28_x86_64.whl --repo pamoda2004/msgkit \
  --predicate-type https://cyclonedx.org/bom
```

The first command checks the SLSA build provenance: the workflow, the commit and the tag it was built from. Add `--signer-workflow pamoda2004/msgkit/.github/workflows/release.yml` to pin the workflow, and `--format json` to read the details. Attestations are verified against Sigstore's public-good trust root; `gh attestation verify --bundle <file>` verifies offline against a downloaded bundle. This corresponds to SLSA Build level 2; level 3 would need the builds moved into a reusable workflow, which is a later step.

## 4. The SBOMs

There is one CycloneDX 1.5 JSON file per released part:

| File | Describes |
|---|---|
| `msgkit-ffi-<version>.cdx.json` | the native library inside every wheel, JAR and AAR (its Rust dependencies) |
| `msgkit-wasm-<version>.cdx.json` | the WebAssembly module inside the npm package |
| `msgkit-service-<version>.cdx.json`, `msgkit-cli-<version>.cdx.json` | the HTTP service and the command line (not attached to releases as binaries yet) |
| `msgkit-node-<version>.cdx.json` | the npm package (no runtime dependencies) |
| `msgkit-python-<version>.cdx.json` | the wheel (no runtime dependencies; it bundles the native library) |
| `msgkit-kotlin-<version>.cdx.json`, `msgkit-android-<version>.cdx.json` | the JAR and the AAR (JNA, kotlinx-serialization) |

Feed them to any CycloneDX-aware tool, for example [Grype](https://github.com/anchore/grype) (`grype sbom:msgkit-ffi-0.2.0.cdx.json`) or [OSV-Scanner](https://google.github.io/osv-scanner/) for known vulnerabilities, or Dependency-Track. To check one yourself against the schema: `python scripts/sbom.py --validate-only <directory>` from a checkout (needs `requirements-sbom.txt`).

The SBOMs list what the build resolved from the lock files at release time, with default features (no SQLCipher); the time inside is the commit time, and no local path is in them (CI checks both).

## 5. PyPI and npm

* **PyPI.** The publish step uses trusted publishing, and the publish action attaches PEP 740 attestations when it does. Check them with `pypi-attestations` or pip's `--require-hashes` against the digests on the release page. (Not exercised yet.)
* **npm.** The package is published with `--provenance`: `npm audit signatures` in a project that depends on it verifies the registry signature and the provenance. (Not exercised yet.)

## Running the workflow without releasing (dry run)

The maintainer can start `release.yml` by hand (*Actions → Release → Run workflow*). A manual run builds everything, generates the SBOMs and signs every file, and keeps the results **only as workflow artifacts**: it never creates a GitHub release, never publishes to PyPI or npm, and never stores attestations (those steps run only for a *push* of a `v*` tag; a tag chosen in the form still counts as a manual run). `scripts/tests/test_release_workflow.py` fails CI if one of those guards is removed. The signing step is real, so a dry run writes an entry to the public Sigstore transparency log (Rekor) for the branch identity.
