---
status: proposed
date: 2026-11-15
decision-makers:
  - Alef maintainers
amends: ["0015", "0024"]
---

# Bind loaded component code to the verified file

## Context and Problem Statement

`alef-component-runtime` verifies a cached library (size, SHA-256, signed manifest) by reading it through its path, and later passes the same path to `dlopen` or, for a stdio sidecar, to `exec`. The manager trusts that verification for the rest of the process, and ADR-0024's respawn re-executes the cached executable without verifying it again. Anyone who can replace the file between the check and the load, or before a respawn, runs unverified code inside or beside the host. The cache directory was created with the process umask, so on a shared machine that could be another user.

## Decision Drivers

* What runs must be the bytes that were verified, not whatever the path names later.
* A respawn must not execute code that was never checked against the lock.
* The common path (a verified stamp, a hot cache) must stay cheap.
* The runtime supports Linux, macOS and Windows, with and without `/proc`.

## Considered Options

* Verify through one descriptor and load that file identity; re-verify before every spawn and respawn.
* Copy each verified library into a fresh private temporary file and load the copy.
* Keep path-based verification and document the window.

## Decision Outcome

Chosen option: "Verify through one descriptor and load that file identity", because it closes the window on Linux and narrows it elsewhere to an attacker who can already write the user's private cache, without copying libraries on every start.

* The cache opens the library once, reads its metadata from that descriptor, compares the verified stamp against that metadata, and hashes the descriptor. The manager keeps the descriptor open with the verified artifact.
* On Linux a library is loaded with `dlopen("/proc/self/fd/N")` on that descriptor, and an ELF sidecar is executed from the same path (its `argv[0]` stays the cached path). A `#!` script sidecar cannot be executed from a close-on-exec descriptor and is started by path.
* Everywhere else, and on Linux without `/proc`, the path is re-checked immediately before `dlopen` or spawn against the descriptor's device, inode, size and modification time (size and modification time on Windows), and a mismatch fails with `ComponentError::ArtifactChanged`. The change time is not compared because making a sidecar executable changes it. A replacement between that check and the load is not detected.
* Every sidecar spawn and respawn verifies the executable through `ArtifactCache` first (cheap with the verified stamp), so ADR-0024's respawn never executes a file that was not checked against the lock.
* The cache creates its directories private to the current user (`0700` on Unix). When the object directory is writable by group or others, or owned by another user, the runtime ignores the verified stamp and hashes the library on every verification instead of refusing to run, so shared or oddly provisioned caches keep working with full verification. This is what limits the residual window above to the cache owner.

Copying every library to a private temporary file was rejected: it costs a full copy per process start for large libraries, leaves temporary files behind on crashes, and on macOS a copy is still loaded by path. Documenting the window was rejected because the respawn path re-executed files that were never re-checked at all.

### Consequences

* Good, because on Linux a native library or ELF sidecar cannot be swapped between verification and load.
* Good, because a respawned sidecar is verified like a first start.
* Bad, because on macOS, Windows and for script sidecars a same-user attacker who wins the race between the final check and the load is still not detected.
* Bad, because the manager keeps one descriptor open per verified artifact for the life of the process.
* Bad, because without the verified stamp every respawn re-hashes the executable.

### Confirmation

`crates/alef-component-runtime/src/identity.rs` tests that a replaced path is refused and that, on Linux, loading goes through the verified descriptor even after a swap. The cache tests cover the stamp being compared with descriptor metadata, and the stdio sidecar tests cover a respawn refusing an executable edited after the first spawn.

## Pros and Cons of the Options

### Verify through one descriptor and load that file identity

* Good, because no extra copies are made.
* Good, because the stamp keeps the hot path to a `stat` and a small file read.
* Bad, because the strongest guarantee is Linux-only.

### Copy each verified library into a fresh private temporary file

* Good, because the loaded file is unreachable to other users on every platform.
* Bad, because every start copies the library, and crashes leave copies behind.

### Keep path-based verification and document the window

* Good, because nothing changes.
* Bad, because a respawn executes whatever the path names.

## More Information

* [ADR 0015: ABI and runtime stability policy for component crates](0015-abi-and-runtime-stability-policy-for-component-crates.md)
* [ADR 0024: Respawn stdio sidecars after a fatal connection failure](0024-respawn-stdio-sidecars-after-a-fatal-connection-failure.md)
