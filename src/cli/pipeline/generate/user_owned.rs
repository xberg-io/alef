//! Reading `[workspace.ownership] user_owned` at the point every writer already stands: a
//! `base_dir`.
//!
//! Anchored on `base_dir` rather than threaded through the ~20 `write_files_report` /
//! `write_scaffold_files_report` call sites, and that is the point rather than a shortcut. The
//! two ownership facts these guards already consult -- `.alef-ownership.toml`
//! (`cli::cache::is_scaffold_owned_path`) and the snippet coverage ledger
//! (`e2e::snippets::ownership::is_ledger_owned_snippet_path`) -- are both `base_dir`-anchored
//! for the same reason, and `super::scaffold::write_scaffold_files_report`'s own doc records
//! what the alternative costs: the lockfile-relock hook is threaded per caller, so "a future
//! third caller of this exact function needs to remember the same call, not get it for free."
//! A declaration a new write stage can forget to pass is a declaration that silently stops
//! protecting a consumer's files the day someone adds a stage. ~keep
//!
//! KNOWN LIMIT, stated rather than hidden: this reads `base_dir/alef.toml`, so a run driven by
//! `alef --config <elsewhere>.toml` from a different directory finds no declaration and
//! behaves exactly as alef does today. `base_dir` is `std::env::current_dir()` at every write
//! call site, and `--config` defaults to the relative path `alef.toml`, so the two coincide
//! for every default invocation. Widening this needs the config path plumbed to the writers,
//! which is the threading this module exists to avoid; it is a real gap, not a solved one. ~keep

use crate::core::config::UserOwnedPaths;
use std::path::Path;

/// The alef config filename this lookup assumes under `base_dir`. Matches
/// `bin_cli::args::Cli::config`'s default.
const CONFIG_FILE_NAME: &str = "alef.toml";

/// The declared-user-owned matcher for a tree rooted at `base_dir`.
///
/// Returns [`UserOwnedPaths::none`] when no config is reachable or the config declares nothing
/// -- byte-for-byte today's behaviour for every repository that does not configure this.
///
/// A config that exists, mentions `[workspace.ownership]`, and fails to deserialize is a HARD
/// ERROR rather than a fail-open. The failure mode this closes is the one this repository names
/// most often: a check that passes because it examined nothing. Falling open there would let a
/// consumer commit a declaration, watch alef report no refusals, and conclude the files are
/// protected -- while alef was overwriting them the whole time because the config never parsed.
/// A config that does not mention the section at all still fails open, since for that repo the
/// parse result cannot change any outcome. ~keep
pub(crate) fn declared_user_owned(base_dir: &Path) -> anyhow::Result<UserOwnedPaths> {
    let config_path = base_dir.join(CONFIG_FILE_NAME);
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return Ok(UserOwnedPaths::none());
    };
    let declares_ownership = content.contains("[workspace.ownership]") || content.contains("user_owned");
    match toml::from_str::<crate::core::config::NewAlefConfig>(&content) {
        Ok(config) => UserOwnedPaths::compile(&config.workspace.ownership.user_owned),
        Err(error) if declares_ownership => Err(anyhow::anyhow!(
            "{} declares [workspace.ownership] but could not be parsed ({error}). Refusing to \
             continue -- the declared paths would otherwise be overwritten.",
            config_path.display()
        )),
        Err(_) => Ok(UserOwnedPaths::none()),
    }
}

/// Record `full_path` as a declared user-owned skip and answer `true`, when the declaration
/// matches it and the file already exists.
///
/// Called ahead of `can_skip`, the merge targets, the binary branch and the ownership guard in
/// [`super::scaffold::write_scaffold_files_report`], and unconditional in that function's
/// `overwrite` argument. `can_skip` already expresses "written once, then the user's" -- but
/// only for the stages that pass `overwrite: false`, and the e2e, test-apps, README and docs
/// stages all pass `true` unconditionally, which is how a deliberate hand-maintained seed ends
/// up in the refusal tally on every single run with a remedy (`alef adopt`) that `alef adopt`
/// itself refuses for the same file. The declaration is the one ownership statement no
/// `overwrite` argument outranks.
///
/// `false` for an ABSENT declared path, so it falls through to the ordinary write and the path
/// is seeded exactly once. Suppressing creation instead would leave `alef verify`'s
/// missing-generated-file check failing forever for a path nothing will ever write -- the same
/// stable bad state this option exists to end, arriving through another door. That seeding
/// write is unstamped and unrecorded (see the `generated_header` and
/// `record_scaffold_owned_path` call sites in both writers): a marker or an ownership-record
/// entry is a claim of alef authorship, and either one would let a later run's ownership guard
/// authorise exactly the overwrite the declaration forbids -- besides enrolling a file alef
/// will never rewrite into `alef verify`'s marker-driven staleness walk, where the consumer's
/// first hand-edit would make it permanently stale with no reachable remedy.
///
/// The path is still added to `expected_paths`: the orphan sweeps read that set to decide a
/// path is still wanted, and omitting a declared path would have the next sweep DELETE the file
/// this branch just declined to touch -- strictly worse than the refusal it replaces. ~keep
pub(super) fn skip_declared_existing(
    declared: &UserOwnedPaths,
    base_dir: &Path,
    full_path: &Path,
    report: &mut super::write::WriteReport,
) -> bool {
    if !declared.matches(base_dir, full_path) || !full_path.exists() {
        return false;
    }
    report.expected_paths.insert(full_path.to_path_buf());
    report.user_owned_paths.insert(full_path.to_path_buf());
    tracing::debug!("  declared user-owned (not written): {}", full_path.display());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_with_no_config_declares_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let declared = declared_user_owned(temp.path()).expect("no config must fail open");
        assert!(declared.is_empty());
    }

    #[test]
    fn a_declared_glob_is_compiled_from_the_workspace_section() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(CONFIG_FILE_NAME),
            "[workspace.ownership]\nuser_owned = [\"e2e/*/package.json\"]\n\n\
             [[crates]]\nname = \"sample_core\"\nsources = [\"src/lib.rs\"]\n",
        )
        .expect("write config");
        let declared = declared_user_owned(temp.path()).expect("valid config");
        assert!(declared.matches(temp.path(), &temp.path().join("e2e/node/package.json")));
        assert!(!declared.matches(temp.path(), &temp.path().join("packages/node/index.js")));
    }

    #[test]
    fn an_unparseable_config_that_declares_ownership_is_a_hard_error_not_a_silent_fail_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(CONFIG_FILE_NAME),
            "[workspace.ownership]\nuser_owned = [\"e2e/*/package.json\"]\nthis is not toml\n",
        )
        .expect("write config");
        let error = declared_user_owned(temp.path()).expect_err("must not fail open");
        assert!(
            format!("{error:#}").contains("[workspace.ownership]"),
            "the error must name the declaration whose protection would otherwise be silently lost"
        );
    }

    #[test]
    fn an_unparseable_config_that_declares_nothing_still_fails_open() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join(CONFIG_FILE_NAME), "this is not toml\n").expect("write config");
        let declared = declared_user_owned(temp.path()).expect("must fail open");
        assert!(declared.is_empty());
    }
}
