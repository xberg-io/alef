//! `[workspace.ownership]` -- the consuming repository's declaration that it, not alef, is the
//! author of a generated path.
//!
//! THE CONDITION THIS NAMES: alef has a two-state ownership model
//! (`GeneratedHeaderConfig`/`GeneratedFile::generated_header`) -- *overwritten on every run*
//! (`true`) versus *written once when absent, then user-owned* (`false`) -- and both states are
//! decided by the **generator**, from the emitting backend's own judgement about a path. There
//! is no state the **consumer** can declare, and for a seed the consumer has grown past alef's
//! placeholder the generator's judgement is not the one that should win.
//!
//! MEASURED (the reason this exists): the create-once state is honoured by
//! `write_scaffold_files_report`'s `can_skip` only when its caller passes `overwrite: false`.
//! Several stages pass `overwrite: true` unconditionally -- `bin_cli::all_commands::e2e_stage`
//! (both the e2e-suite and test-apps writes), the README write, `alef docs`, `alef e2e` --
//! so for every path those stages emit, `can_skip` never runs, the ownership guard is reached,
//! and a hand-maintained seed that carries no provenance marker is refused. The refusal is
//! recorded in `WriteReport::refused_paths` and reported by
//! `cli::pipeline::generate::write::report_refused_writes` as "N file(s) were NOT written, M of
//! them holding content that DIFFERS from what alef would now generate",
//! naming `alef adopt <path>` as the remedy -- which `alef adopt` then refuses for the same
//! files, because `cli::commands::adopt::is_create_once_seed` classifies them as seeds and
//! adoption would consent to a later overwriting regen replacing them. The consumer is left in
//! a stable bad state: the files are deliberate, the refusal is correct, and every run reports
//! a failure with no reachable remedy.
//!
//! [`OwnershipConfig::user_owned`] is the missing third state, and it is declared rather than
//! derived: a committed, reviewable list of repo-relative globs saying "these paths are this
//! repository's, not alef's". A path it matches is never written over, is never stamped, is
//! never verified for content, and is counted as a **declared skip** rather than a failure.
//!
//! Deliberately NOT a way to silence a warning. The count is still reported on every run
//! (`cli::pipeline::generate::write::report_user_owned_skips`) and `alef verify` states it as
//! coverage, for the same reason `VerifyConfig`'s module doc gives: a run that narrowed its own
//! scope has to say so. What changes is the *category* -- a declared, auditable disposition
//! instead of an unexplained failure tally. ~keep
//!
//! How this differs from the exclusion knobs alef already has:
//!
//! - `[crates.exclude]` / `exclude_types` drop an **item** from the extracted IR, so no binding
//!   for it is generated in any language. They change what alef knows about, not who owns a
//!   file that alef does generate.
//! - `[crates.verify] ignore_ephemeral` ([`super::VerifyConfig`]) says the opposite of this
//!   one: that output is regenerated per run, deliberately gitignored, and **never committed**,
//!   so its *absence* must stop being a permanent failure. It affects one verify report and
//!   explicitly does nothing for a path that exists on disk; alef still owns and overwrites
//!   those paths. `user_owned` covers paths that *are* committed, *are* present, and must not
//!   be written -- it affects the write path first and verify second.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Consumer-declared ownership dispositions for generated paths.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OwnershipConfig {
    /// Repo-relative glob patterns naming paths this repository owns and maintains by hand,
    /// even though alef's generators emit them.
    ///
    /// For every path one of these patterns matches, alef:
    ///
    /// 1. **Never overwrites it.** The check runs before the create-once skip, before the
    ///    content comparison and before the provenance-marker ownership guard, so it holds
    ///    regardless of `generated_header`, regardless of the `overwrite` flag a given write
    ///    stage passes, and regardless of `--clobber-create-once-seeds`. This is strictly
    ///    stronger than `generated_header = false`, which only holds for stages that pass
    ///    `overwrite: false`.
    /// 2. **Seeds it once when it is absent**, exactly as it would today but *without* a
    ///    provenance marker, and reports having done so. Suppressing the creation instead
    ///    would leave `alef verify`'s missing-generated-file check failing forever with no
    ///    remedy -- the same stable bad state this option exists to end.
    /// 3. **Never adds a provenance marker and never records it** in `.alef-ownership.toml`.
    ///    Both are claims of alef authorship, and the declaration is a disclaimer of exactly
    ///    that. One residue, stated rather than hidden: a path that was already alef-owned
    ///    and marked BEFORE it was declared keeps that marker, and the post-format stamping
    ///    pass (`cli::pipeline::generate::write::finalize_hashes`) still refreshes its
    ///    `alef:hash:` line from the file's own bytes. That pass rewrites the hash line and
    ///    nothing else -- the body is untouched -- and `alef verify` no longer reads the
    ///    stamp for a declared path. To retire the marker entirely, delete the line by hand
    ///    once; nothing will put it back.
    /// 4. **Never verifies its contents.** `alef verify` will not report it as stale, frozen,
    ///    or adoptable, and `--exit-code` never gates on it. It is still counted and named in
    ///    verify's coverage report, so a run that stopped checking 17 files says so.
    ///
    /// Matched with [`glob::Pattern`] against the path relative to `base_dir`, with the same
    /// semantics as `[crates.verify] ignore_ephemeral`: `*` is not separator-aware, so
    /// `test_apps/*` matches `test_apps/swift/Package.swift`.
    ///
    /// A pattern that does not compile as a glob is a hard error, unlike
    /// [`super::VerifyConfig::ignore_ephemeral`], which drops one silently. The asymmetry is
    /// deliberate and load-bearing: a dropped `ignore_ephemeral` pattern only widens a
    /// read-only report, while a dropped `user_owned` pattern lets alef overwrite a file the
    /// consumer declared their own. Silence there costs work.
    ///
    /// ```toml
    /// [workspace.ownership]
    /// user_owned = [
    ///   "e2e/*/package.json",
    ///   "test_apps/*/Package.swift",
    ///   "packages/*/src/test/**",
    /// ]
    /// ```
    #[serde(default)]
    pub user_owned: Vec<String>,
}

/// Compiled [`OwnershipConfig::user_owned`] patterns.
///
/// Compiled once per write phase and passed down, rather than recompiled per path: a single
/// `alef all` write phase evaluates this against every prepared output path, and the consumer
/// trees this exists for carry tens of thousands. ~keep
#[derive(Debug, Clone, Default)]
pub struct UserOwnedPaths {
    patterns: Vec<glob::Pattern>,
}

impl UserOwnedPaths {
    /// A declaration that matches nothing -- the state of every repo that does not configure
    /// `[workspace.ownership]`, and the fail-open state when no config is reachable.
    #[must_use]
    pub fn none() -> Self {
        Self { patterns: Vec::new() }
    }

    /// Compile `patterns`, failing on the first one that is not a valid glob.
    ///
    /// See [`OwnershipConfig::user_owned`] for why a malformed pattern is fatal here and merely
    /// dropped in `[crates.verify] ignore_ephemeral`: silently dropping an unusable pattern lets
    /// alef overwrite a file this repository declared its own. ~keep
    pub fn compile(patterns: &[String]) -> anyhow::Result<Self> {
        let mut compiled = Vec::with_capacity(patterns.len());
        for pattern in patterns {
            let parsed = glob::Pattern::new(pattern).map_err(|error| {
                anyhow::anyhow!("[workspace.ownership] user_owned pattern {pattern:?} is not a valid glob: {error}")
            })?;
            compiled.push(parsed);
        }
        Ok(Self { patterns: compiled })
    }

    /// Whether any pattern is declared at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether `full_path` (absolute, as every writer carries it) is declared user-owned
    /// relative to `base_dir`.
    ///
    /// A path that cannot be made relative to `base_dir` never matches. Patterns are declared
    /// repo-relative, so a path outside the repo is not one they can be describing, and the
    /// conservative answer keeps alef's existing behaviour (including the provenance-marker
    /// ownership guard, which still protects the file) rather than silently widening the
    /// declaration. ~keep
    #[must_use]
    pub fn matches(&self, base_dir: &Path, full_path: &Path) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let Ok(relative) = full_path.strip_prefix(base_dir) else {
            return false;
        };
        self.patterns.iter().any(|pattern| pattern.matches_path(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_declaration_matches_nothing() {
        let declared = UserOwnedPaths::compile(&OwnershipConfig::default().user_owned).expect("compile");
        assert!(declared.is_empty());
        assert!(!declared.matches(Path::new("/repo"), Path::new("/repo/e2e/node/package.json")));
    }

    #[test]
    fn a_declared_glob_matches_every_path_beneath_it() {
        let declared = UserOwnedPaths::compile(&["test_apps/*/Package.swift".to_owned()]).expect("compile");
        assert!(declared.matches(Path::new("/repo"), Path::new("/repo/test_apps/swift/Package.swift")));
        assert!(!declared.matches(Path::new("/repo"), Path::new("/repo/packages/swift/Package.swift")));
    }

    #[test]
    fn a_path_outside_base_dir_is_never_declared() {
        let declared = UserOwnedPaths::compile(&["**".to_owned()]).expect("compile");
        assert!(!declared.matches(Path::new("/repo"), Path::new("/elsewhere/e2e/node/package.json")));
    }

    #[test]
    fn a_malformed_pattern_is_a_hard_error_naming_the_pattern() {
        let error = UserOwnedPaths::compile(&["e2e/[unclosed".to_owned()]).expect_err("must reject");
        let message = format!("{error:#}");
        assert!(
            message.contains("e2e/[unclosed") && message.contains("user_owned"),
            "the error must name the offending pattern and the option it came from: {message}"
        );
    }

    #[test]
    fn ownership_config_round_trips_through_toml() {
        let parsed: OwnershipConfig =
            toml::from_str("user_owned = [\"e2e/*/package.json\"]").expect("parse [workspace.ownership]");
        assert_eq!(parsed.user_owned, vec!["e2e/*/package.json".to_owned()]);
    }
}
