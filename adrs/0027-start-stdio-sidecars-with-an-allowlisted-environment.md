---
status: proposed
date: 2026-11-15
decision-makers:
  - Alef maintainers
amends: ["0017"]
---

# Start stdio sidecars with an allowlisted environment

## Context and Problem Statement

ADR-0017 starts a stdio sidecar with `Command::new(executable)`, so the child inherits the host's whole environment and working directory. A host such as poly runs in CI with registry tokens, cloud credentials and `GITHUB_TOKEN` in its environment, and inside the repository it is linting. A component is verified code from a signed lock, but it is third-party code with a narrow job; it has no reason to read the host's secrets or resolve relative paths against the host's working tree.

## Decision Drivers

* A sidecar should receive only what it needs to run, not whatever the host process happens to hold.
* Sidecars written in Go, Python or shell must still start on Linux, macOS and Windows.
* A host whose sidecar genuinely needs more (a tool's own configuration variables) must be able to pass it without patching the runtime.

## Considered Options

* Clear the environment, pass a fixed allowlist, let the host extend it, and run in the component's cache directory.
* Inherit everything, and document that sidecars see the host environment.
* Clear the environment entirely and let the host name every variable.

## Decision Outcome

Chosen option: "Clear the environment, pass a fixed allowlist, let the host extend it", because it removes ambient secrets by default while keeping the variables every runtime needs.

The runtime calls `env_clear()` and passes, when the host has them, `PATH`, `HOME`, `USER`, `LOGNAME`, `TMPDIR`, `TEMP`, `TMP`, `TZ`, `LANG`, `LANGUAGE` and `LC_*`, plus `SYSTEMROOT`, `WINDIR`, `SYSTEMDRIVE`, `USERPROFILE`, `PATHEXT` and `COMSPEC` for Windows; it then sets `ALEF_STDIO_PROTOCOLS` itself and never inherits the host's value. `ComponentManager::with_sidecar_env_passthrough` adds names, with a trailing `*` matching a prefix; names compare case-insensitively on Windows. The child's working directory is the directory holding its verified executable, inside the component's cache object.

### Consequences

* Good, because credentials in the host environment no longer reach sidecars by default.
* Good, because a sidecar can no longer depend on, or write relative to, the host's working directory by accident.
* Bad, because a sidecar that read a host variable (a tool configuration variable, a proxy setting such as `HTTPS_PROXY`) stops seeing it until the host passes it through; this is a behavior change for existing hosts.

### Confirmation

Unit tests in `crates/alef-component-runtime/src/stdio/process.rs` cover the allowlist, prefix patterns and Windows case folding; `tests/stdio_sidecar.rs` checks a real sidecar's environment and working directory, and the passthrough option.

## Pros and Cons of the Options

### Clear the environment, pass a fixed allowlist, let the host extend it

* Good, because it is safe by default and still works out of the box.
* Bad, because the default list is a judgment call that will need occasional additions.

### Inherit everything

* Good, because nothing changes for existing sidecars.
* Bad, because every sidecar sees every secret the host holds.

### Clear the environment entirely

* Good, because nothing leaks.
* Bad, because sidecars fail to start until every host learns which variables its platform needs.

## More Information

* [ADR 0017: Framed stdio transport for component sidecars](0017-framed-stdio-transport-for-component-sidecars.md)
