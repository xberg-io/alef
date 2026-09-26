// Test module: a loud skip is expected diagnostic output here, not debugging noise. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

//! Portability gate for the Tier-2 `{{mock_sub_origin}}` e2e fixture token.
//!
//! The e2e mock server answers a second, subdomain hostname (`sub.<mock_alt_origin's host>`,
//! concretely `sub.localhost`) so cross-host crawler behaviour -- `allow_subdomains`,
//! `stay_on_domain`, external-link handling -- can be expressed in a fixture at all. That only
//! works because `*.localhost` resolves to loopback: RFC 6761 reserves `.localhost` for exactly
//! this, and macOS (mDNSResponder), glibc >= 2.36 with `systemd-resolved`, and Windows 10 1803+
//! all honor it. musl/Alpine, minimal containers, and older glibc without `nss-myhostname` do
//! not, so a fixture that silently depended on it there would not fail with one clear message --
//! it would 404 every request through the subdomain origin and leave whoever is debugging it to
//! rediscover the DNS gap from scratch.
//!
//! This file is that one clear message. It is an ordinary integration test, so the existing 3-OS
//! `cargo test --workspace` job picks it up with no workflow edit.
//!
//! # Fail or skip?
//!
//! Absence of `*.localhost` -> loopback is treated as a **loud, counted skip**, not a hard
//! failure, mirroring the `ALEF_REQUIRE_GO` / `ALEF_REQUIRE_SWIFT` / `ALEF_REQUIRE_RUBY`
//! convention in `src/test_support/toolchain.rs`: a platform property this crate does not control
//! (and, per `commit 137d78252`, must gate on actually working rather than assuming) should not
//! red a build that is otherwise fine, the same way a missing Swift toolchain does not red Linux.
//! But per the same commit's whole point, a silent skip is exactly the failure mode this repo
//! treats as its dominant defect, so the skip is never silent: it prints a `SKIPPED:` line naming
//! the host and the escape hatch, and it appends a row to this run's toolchain-census TSV so
//! `task test:census` (which already sums every `*.tsv` under the census directory after
//! `cargo test --workspace`, see `Taskfile.yml`) reports it by name instead of it vanishing into
//! an undifferentiated green run. [`REQUIRE_ENV`] is the opt-in that promotes the skip to a hard
//! failure, for a CI leg a maintainer knows must support the mapping -- unset by default, exactly
//! like the three `ALEF_REQUIRE_*` variables it mirrors.
//!
//! `src/test_support/toolchain.rs`'s own census writer is `pub(crate)` and unreachable from this
//! integration-test crate, so [`record_census`] below duplicates its file layout and path
//! derivation rather than importing it. `scripts/toolchain-census.sh` sums every `*.tsv` file it
//! finds regardless of which binary wrote it, so duplicating the layout is sufficient to be
//! counted -- no script or workflow change is needed for this file to participate.

use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;

/// The subdomain the Tier-2 `{{mock_sub_origin}}` fixture token depends on resolving to loopback.
/// Kept as one constant so the probe and the failure/skip message can never end up naming a
/// different host than the one actually checked.
const ALT_SUBDOMAIN: &str = "sub.localhost";

/// An RFC 2606 reserved name that must never resolve anywhere. The negative control below drives
/// this through the exact same predicate the positive case uses
/// ([`is_loopback_resolvable`]), so it fails for the right reason -- the predicate correctly
/// reporting "not loopback-resolvable" -- rather than for an unrelated one. This repo has already
/// shipped a control that passed for the wrong reason once (a planted typo rejected by a
/// `duplicate key` error instead of the unknown-field check it was meant to exercise), so the two
/// cases here share one code path on purpose. ~keep
const NEVER_RESOLVES: &str = "alef-unresolvable-alt-host.invalid";

/// Escape hatch that promotes an absent `*.localhost` -> loopback mapping from a counted, loud
/// skip into a hard test failure. See the module docs' "Fail or skip?" section. Unset by default.
const REQUIRE_ENV: &str = "ALEF_REQUIRE_ALT_HOST_LOOPBACK";

/// Toolchain-census row name for this check. Not a language toolchain, but
/// `scripts/toolchain-census.sh` only ever treats its rows as named counters, and the same
/// "counted, never silent" contract applies to a platform DNS capability a fixture tier depends
/// on. ~keep
const CENSUS_NAME: &str = "wildcard-localhost";

/// True only when `host` resolves to at least one address, and *every* address it resolves to is
/// loopback. Checking only the first address would pass a host that round-robins between a
/// loopback and a routable address, which is not the guarantee Tier-2 fixtures need.
fn is_loopback_resolvable(host: &str) -> bool {
    match (host, 0).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<IpAddr> = addrs.map(|socket_addr| socket_addr.ip()).collect();
            !addrs.is_empty() && addrs.iter().all(IpAddr::is_loopback)
        }
        Err(_) => false,
    }
}

/// `<target-dir>/toolchain-census/<this-test-binary>.tsv`, matching
/// `src/test_support/toolchain.rs::census_file`'s derivation exactly (test executables live at
/// `<target-dir>/<profile>/deps/<name>-<hash>`, so the target directory is three levels up) so
/// this file's row lands where `scripts/toolchain-census.sh` already looks.
fn census_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = exe.file_stem()?.to_str()?.to_owned();
    let target_dir = exe.parent()?.parent()?.parent()?;
    Some(target_dir.join("toolchain-census").join(format!("{name}.tsv")))
}

/// Records one attempt of the `wildcard-localhost` check. Best-effort, like its
/// `src/test_support/toolchain.rs` counterpart: a portability probe must not fail the run because
/// the census file itself could not be written.
fn record_census(executed: bool) {
    let Some(path) = census_file() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let (attempted, executed_count, skipped) = if executed { (1, 1, 0) } else { (1, 0, 1) };
    let row = format!("{CENSUS_NAME}\t{attempted}\t{executed_count}\t{skipped}\n");
    let _ = std::fs::write(&path, row);
}

/// Negative control: a name that must never resolve must never be reported loopback-resolvable.
/// Unconditional -- unlike the positive case, nothing about `.invalid` resolution is platform
/// dependent, so this always runs and never skips.
#[test]
fn unresolvable_invalid_host_is_never_reported_loopback_resolvable() {
    assert!(
        !is_loopback_resolvable(NEVER_RESOLVES),
        "{NEVER_RESOLVES} is an RFC 2606 reserved name that must never resolve to anything; \
         is_loopback_resolvable reported it as loopback-resolvable, which means the predicate is \
         not actually checking resolution at all"
    );
}

/// Positive case: on a platform that honors RFC 6761 `*.localhost`, `sub.localhost` must resolve,
/// and every address it resolves to must be loopback. See the module docs' "Fail or skip?"
/// section for why an absent mapping is a loud, counted skip rather than a hard failure by
/// default.
#[test]
fn sub_localhost_resolves_to_loopback_where_the_platform_supports_it() {
    if is_loopback_resolvable(ALT_SUBDOMAIN) {
        record_census(true);
        return;
    }
    record_census(false);
    let message = format!(
        "{ALT_SUBDOMAIN} did not resolve, or resolved to a non-loopback address. Tier-2 \
         subdomain fixtures ({{{{mock_sub_origin}}}}) are unavailable on this platform: they \
         depend on `*.localhost` -> loopback (RFC 6761), which musl/Alpine, minimal containers, \
         and older glibc without nss-myhostname do not provide. If this platform must run \
         Tier-2 fixtures, set `[crates.e2e] alt_host` in alef.toml to a resolvable override. Set \
         {REQUIRE_ENV}=1 to turn this into a hard failure on a platform known to support the \
         mapping."
    );
    if std::env::var_os(REQUIRE_ENV).is_some() {
        panic!("{message}");
    }
    eprintln!("SKIPPED: {message}");
}
