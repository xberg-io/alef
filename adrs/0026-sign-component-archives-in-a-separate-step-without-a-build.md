---
status: proposed
date: 2026-11-15
decision-makers:
  - Alef maintainers
---

# Sign component archives in a separate step without a build

## Context and Problem Statement

`alef component package` assembles an archive and signs its manifest in one step, so the Ed25519 release key must be present on the runner that also compiles the components and executes every dependency's build script. `package --plan` re-fingerprints the checkout with `cargo metadata`, `rustc -vV` and `go version`, so packaging cannot simply move to another host either. A consumer (poly) wants the key on a runner that never builds.

## Decision Drivers

* The signing key never shares a runner with untrusted build scripts.
* The signing host needs no checkout, no toolchain and no `alef.toml`.
* `lock` and the runtime accept the result exactly as they accept a signed `package`.
* The signer does not trust the archives it is handed.

## Considered Options

* A separate `alef component sign` over `package --unsigned` output.
* Detached signatures next to unchanged unsigned archives.
* Run `package` (with the key) on a fresh runner from the build job's libraries.

## Decision Outcome

Chosen option: "A separate `alef component sign`", because it keeps the archive format, `lock` and the runtime unchanged while moving the key off the build runner.

`alef component sign --input DIR --output DIR (--key-file PATH | --key-env NAME) --key-id ID` reads each `*.record.json` in `--input` and the archive it names, and returns before configuration loads, so it runs no cargo, rustc or go. Every archive is validated before anything is written: the record must name its own archive, its manifest hash, archive size and archive hash must match, and the archive must pass the runtime extractor's rules (`component.json` first, at most three entries, regular files with plain names, metadata entries at most 64 KiB, the library at exactly its declared size), with the embedded manifest byte-identical to the record's and the library hashing to the manifest's digest. An archive that already carries a signature is refused unless `--resign` is given. `--plan` additionally requires a `build` row whose `input_hash` and version match each record; `--public-key` requires the private key to match the public key configured for `--key-id`. The signed archive is written through the same writer `package` uses, so it is byte-identical to what a signed `package` of the same library produces, and signing is deterministic.

Detached signatures were rejected because the runtime reads `component.sig` from inside the archive and the lock pins the archive's hash; changing either would need a new archive format and runtime support. Running `package` on the signing runner would need the toolchain and checkout there and would re-run fingerprinting, which is exactly what the key host must avoid.

### Consequences

* Good, because the key is read only on a runner that executes no third-party build code.
* Good, because `lock`, `verify` and the runtime need no change and see the same bytes either way.
* Bad, because the signer certifies the archive the build job produced; it cannot detect a library that a compromised build compiled maliciously, only one altered after packaging.
* Bad, because `input_hash` in the record is not covered by the signature, so `--plan` matching guards against mixing up artifacts, not against a build job that lies about its inputs.

### Confirmation

`src/component/sign/tests.rs` signs `package --unsigned` output and checks it is byte-identical to a signed `package`, passes `verify_record_with` and `build_lock` (the `lock` path), and installs through `alef_component_runtime::ArtifactCache` under the lock's keys; it covers determinism, keys from a file and an environment value, path traversal, symlink, extra, duplicate and reordered entries, library hash and size mismatches, a tampered archive or record, symlinked input files, already-signed input with and without `--resign`, plan mismatches, a wrong public key or key ID, and that a single bad input writes nothing. `src/bin_cli/component_commands_tests.rs` runs the command with no configuration file.

```console
mise exec -- cargo test --lib component::sign
```

## Pros and Cons of the Options

### A separate `alef component sign` over `package --unsigned` output

* Good, because the archive format and trust chain stay as they are.
* Bad, because the archive is decompressed and rewritten once more.

### Detached signatures next to unchanged unsigned archives

* Good, because the unsigned archive would not be rewritten.
* Bad, because the runtime and lock format would change, and old runtimes could not verify new releases.

### Run `package` (with the key) on a fresh runner from the build job's libraries

* Good, because no new command is needed.
* Bad, because the signing runner needs the checkout and toolchain and re-runs fingerprinting.

## More Information

* [Component distribution](../docs/design/components.md#two-stage-release-build-without-the-key-sign-without-a-build)
