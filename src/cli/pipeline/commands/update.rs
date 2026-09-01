use crate::cli::commands::version_manifests::discover_cargo_locks;
use crate::cli::git::tracked_paths_under;
use crate::cli::pipeline::collect_alef_headered_paths;
use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed};
use crate::core::config::shell::quote_word;
use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use rayon::prelude::*;
use std::path::Path;
use tracing::{debug, error};

/// Build the default Rust update plan: one `cargo update`/`cargo upgrade` invocation per Cargo
/// manifest alef's release tooling discovers, each explicitly `cd`ed into that manifest's own
/// directory.
///
/// ~keep alef 0.82: a bare `cargo update` run once from the CWD never reaches a manifest that is
/// deliberately excluded from any workspace — a Ruby/R/Elixir native-extension crate, or
/// `e2e/rust` — because those are exactly the manifests a root `cargo update -w` cannot see
/// either. That gap is how three lockfiles in one consumer repo drifted far enough that `cargo
/// metadata --locked` failed in CI. [`discover_cargo_locks`] is the same, workspace-topology-
/// agnostic enumeration `alef validate versions` and the version-sync relock step already use,
/// so this reaches exactly the set of manifests those checks hold `alef update` accountable
/// against — never a second, independently-derived walk. `cargo upgrade` is withheld for a
/// manifest alef itself generated (an `alef:hash:` header, detected via
/// [`collect_alef_headered_paths`], never a path allowlist): `cargo upgrade` rewrites the
/// manifest on disk, which would invalidate that stamp and make the very next `alef verify`
/// report drift on a file alef itself asked cargo to rewrite.
fn rust_update_commands(config: &ResolvedCrateConfig, latest: bool) -> anyhow::Result<Vec<String>> {
    let workspace_root =
        std::env::current_dir().context("alef update: failed to determine the current working directory")?;
    rust_update_commands_in(config, &workspace_root, latest)
}

fn rust_update_commands_in(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    latest: bool,
) -> anyhow::Result<Vec<String>> {
    let canonical = config.resolved_version().unwrap_or_default();
    let tracked = tracked_paths_under(workspace_root);
    let generated = collect_alef_headered_paths(workspace_root);
    let locks = discover_cargo_locks(workspace_root, &canonical, tracked.as_ref());
    if locks.is_empty() {
        anyhow::bail!(
            "alef update: no Cargo.lock was discovered under {} — nothing to update. Check that the \
             repository is a git work tree with a committed Cargo.lock, or run with --lang to exclude rust",
            workspace_root.display()
        );
    }

    let mut commands = Vec::new();
    for lock in locks {
        if lock.blocked_on_publish.is_some() {
            debug!(
                lock = %lock.path.display(),
                "alef update: skipping — this lock is waiting on its own pending release to publish"
            );
            continue;
        }
        let Some(dir) = lock.path.parent() else {
            continue;
        };
        let is_generated = generated.contains(&dir.join("Cargo.toml"));
        let mut steps = Vec::new();
        if latest && !is_generated {
            steps.push("cargo upgrade --incompatible".to_string());
        }
        steps.push("cargo update".to_string());
        commands.push(format!(
            "cd {} && {}",
            quote_word(&dir.to_string_lossy()),
            steps.join(" && ")
        ));
    }
    Ok(commands)
}

fn dedupe_plans(plans: Vec<(Language, Vec<String>)>) -> Vec<(Language, Vec<String>)> {
    let mut seen = std::collections::HashSet::<String>::new();
    plans
        .into_iter()
        .map(|(lang, cmds)| {
            let unique: Vec<String> = cmds.into_iter().filter(|c| seen.insert(c.clone())).collect();
            (lang, unique)
        })
        .collect()
}

/// Update dependencies for each language.
///
/// When `latest` is true, runs the aggressive `upgrade` commands (including
/// incompatible/major version bumps). Otherwise runs the safe `update` commands.
///
/// Executes in two phases:
/// 1. Sequential: check preconditions and collect command lists; deduplicate across languages.
/// 2. Parallel: run each language's deduped command list (within-language order preserved).
pub fn update(config: &ResolvedCrateConfig, languages: &[Language], latest: bool) -> anyhow::Result<()> {
    let mut plans: Vec<(Language, Vec<String>)> = Vec::new();
    for &lang in languages {
        let update_cfg = config.update_config_for_language(lang);
        if !check_precondition(lang, update_cfg.precondition.as_deref()) {
            continue;
        }
        run_before(lang, update_cfg.before.as_ref())?;
        // ~keep The default Rust plan iterates every discovered manifest instead of the single
        // static command template every other language uses (see `rust_update_commands`); an
        // explicit `[update.rust]` in alef.toml opts back into the plain templated behaviour,
        // same as every other language's user override.
        let cmd_strings: Vec<String> = if lang == Language::Rust && !config.update.contains_key(&lang.to_string()) {
            rust_update_commands(config, latest)?
        } else {
            let cmds = if latest {
                update_cfg.upgrade.as_ref()
            } else {
                update_cfg.update.as_ref()
            };
            cmds.map(|cmd_list| cmd_list.commands().into_iter().map(|c| c.to_string()).collect())
                .unwrap_or_default()
        };
        plans.push((lang, cmd_strings));
    }

    let plans = dedupe_plans(plans);

    let results: Vec<(Language, anyhow::Result<()>)> = plans
        .par_iter()
        .map(|(lang, cmds)| {
            let label = lang.to_string();
            for cmd in cmds {
                if let Err(e) = run_command_streamed(cmd, Some(&label)) {
                    return (*lang, Err(e));
                }
            }
            (*lang, Ok(()))
        })
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for (lang, result) in results {
        if let Err(e) = result {
            error!("update failed: {lang} — {e}");
            if first_error.is_none() {
                first_error = Some(e);
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    #[test]
    fn dedupe_plans_removes_duplicate_commands_across_languages() {
        let plans = vec![
            (
                Language::Node,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
            (
                Language::Wasm,
                vec![
                    "corepack use pnpm@latest".to_string(),
                    "pnpm up --latest -r -w".to_string(),
                ],
            ),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["corepack use pnpm@latest", "pnpm up --latest -r -w"]);
        assert!(result[1].1.is_empty(), "Wasm should have no commands after dedupe");
    }

    #[test]
    fn dedupe_plans_preserves_within_language_order() {
        let plans = vec![
            (
                Language::Rust,
                vec!["cargo upgrade --incompatible".to_string(), "cargo update".to_string()],
            ),
            (Language::Node, vec!["cargo update".to_string()]),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["cargo upgrade --incompatible", "cargo update"]);
        assert!(result[1].1.is_empty());
    }

    #[test]
    fn dedupe_plans_unique_commands_unchanged() {
        let plans = vec![
            (Language::Python, vec!["uv sync --upgrade".to_string()]),
            (Language::Ruby, vec!["bundle update --all".to_string()]),
        ];
        let result = dedupe_plans(plans);
        assert_eq!(result[0].1, vec!["uv sync --upgrade"]);
        assert_eq!(result[1].1, vec!["bundle update --all"]);
    }
}

#[cfg(test)]
mod rust_update_plan_tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// A minimal resolved config whose `version_from` points at `root`'s own `Cargo.toml`.
    fn config(root: &Path) -> ResolvedCrateConfig {
        let manifest = root.join("Cargo.toml").to_string_lossy().replace('\\', "/");
        let source = format!(
            "[workspace]\nlanguages = [\"ruby\"]\n\
             [[crates]]\nname = \"demo_lib\"\nsources = [\"src/lib.rs\"]\nversion_from = \"{manifest}\"\n"
        );
        let parsed: crate::core::config::NewAlefConfig = toml::from_str(&source).expect("config parses");
        parsed.resolve().expect("config resolves").remove(0)
    }

    fn write_manifest_and_lock(dir: &Path, package_name: &str, version: &str, generated: bool) {
        std::fs::create_dir_all(dir).expect("fixture directory");
        let header = if generated {
            "# This file is auto-generated by alef — DO NOT EDIT.\n"
        } else {
            ""
        };
        std::fs::write(
            dir.join("Cargo.toml"),
            format!("{header}[package]\nname = \"{package_name}\"\nversion = \"{version}\"\n"),
        )
        .expect("fixture manifest");
        std::fs::write(
            dir.join("Cargo.lock"),
            format!("version = 4\n\n[[package]]\nname = \"{package_name}\"\nversion = \"{version}\"\n"),
        )
        .expect("fixture lock");
    }

    /// The regression test for the defect this brief exists to fix: a manifest that is not a
    /// member of any workspace (a Ruby/R/Elixir native-extension crate, or `e2e/rust`) must still
    /// be reached, not just the CWD's own manifest.
    #[test]
    fn reaches_a_manifest_outside_any_workspace_not_just_the_root() {
        let temp = TempDir::new().expect("tempdir");
        write_manifest_and_lock(temp.path(), "demo_lib", "1.2.3", false);
        let extension_dir = temp.path().join("ext").join("fixture_ruby_ext");
        write_manifest_and_lock(&extension_dir, "fixture_ruby_ext", "0.1.0", false);

        let commands = rust_update_commands_in(&config(temp.path()), temp.path(), false).expect("commands are built");

        assert_eq!(
            commands.len(),
            2,
            "both the root and the standalone extension crate must be reached"
        );
        let extension_dir_str = extension_dir.to_string_lossy();
        assert!(
            commands.iter().any(|cmd| cmd.contains(extension_dir_str.as_ref())),
            "the workspace-excluded extension crate's own directory must appear in the plan: {commands:?}"
        );
    }

    #[test]
    fn latest_runs_cargo_upgrade_then_cargo_update_for_a_hand_authored_manifest() {
        let temp = TempDir::new().expect("tempdir");
        write_manifest_and_lock(temp.path(), "demo_lib", "1.2.3", false);

        let commands = rust_update_commands_in(&config(temp.path()), temp.path(), true).expect("commands are built");

        assert_eq!(commands.len(), 1);
        assert!(
            commands[0].ends_with("cargo upgrade --incompatible && cargo update"),
            "a hand-authored manifest must get both commands under --latest: {commands:?}"
        );
    }

    #[test]
    fn without_latest_never_calls_cargo_upgrade() {
        let temp = TempDir::new().expect("tempdir");
        write_manifest_and_lock(temp.path(), "demo_lib", "1.2.3", false);

        let commands = rust_update_commands_in(&config(temp.path()), temp.path(), false).expect("commands are built");

        assert_eq!(commands.len(), 1);
        assert!(
            !commands[0].contains("cargo upgrade"),
            "without --latest, only cargo update may run: {commands:?}"
        );
        assert!(commands[0].ends_with("cargo update"));
    }

    /// The critical rule: `cargo upgrade` must never touch a manifest alef itself generated,
    /// because it would invalidate the `alef:hash:` stamp the generator wrote.
    #[test]
    fn never_runs_cargo_upgrade_against_an_alef_generated_manifest() {
        let temp = TempDir::new().expect("tempdir");
        write_manifest_and_lock(temp.path(), "demo_lib", "1.2.3", false);
        let extension_dir = temp.path().join("ext").join("fixture_ruby_ext");
        write_manifest_and_lock(&extension_dir, "fixture_ruby_ext", "0.1.0", true);

        let commands = rust_update_commands_in(&config(temp.path()), temp.path(), true).expect("commands are built");

        assert_eq!(commands.len(), 2);
        let extension_dir_str = extension_dir.to_string_lossy();
        let extension_cmd = commands
            .iter()
            .find(|cmd| cmd.contains(extension_dir_str.as_ref()))
            .expect("the generated extension crate must still be reached");
        assert!(
            !extension_cmd.contains("cargo upgrade"),
            "an alef-generated manifest must never be handed to cargo upgrade: {extension_cmd}"
        );
        assert!(extension_cmd.ends_with("cargo update"));

        let root_cmd = commands
            .iter()
            .find(|cmd| !cmd.contains(extension_dir_str.as_ref()))
            .expect("the hand-authored root manifest is also present");
        assert!(
            root_cmd.contains("cargo upgrade --incompatible"),
            "a hand-authored manifest is still eligible for cargo upgrade: {root_cmd}"
        );
    }

    /// A lockfile pinning this workspace's own crate at the version currently being released
    /// cannot resolve until that release is published — see `discover_cargo_locks`'s
    /// `blocked_on_publish`. `alef update` must skip it rather than fail on a resolution that
    /// cannot succeed yet.
    #[test]
    fn skips_a_lock_blocked_on_its_own_pending_publish() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"demo_lib\"\nversion = \"3.4.5\"\n",
        )
        .expect("root manifest");
        std::fs::write(
            temp.path().join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"demo_lib\"\nversion = \"3.4.5\"\n",
        )
        .expect("root lock");

        let app_dir = temp.path().join("test_apps").join("rust");
        std::fs::create_dir_all(&app_dir).expect("app dir");
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"demo_lib_e2e\"\nversion = \"3.4.5\"\n\n\
             [dependencies]\ndemo_lib = { version = \"3.4.5\" }\n",
        )
        .expect("app manifest");
        std::fs::write(
            app_dir.join("Cargo.lock"),
            "version = 4\n\n\
             [[package]]\nname = \"demo_lib\"\nversion = \"3.4.4\"\n\
             source = \"registry+https://example.invalid/index\"\n\n\
             [[package]]\nname = \"demo_lib_e2e\"\nversion = \"3.4.0\"\n",
        )
        .expect("app lock");

        let commands = rust_update_commands_in(&config(temp.path()), temp.path(), true).expect("commands are built");

        assert_eq!(
            commands.len(),
            1,
            "the blocked app lock must be skipped, not failed: {commands:?}"
        );
        let app_dir_str = app_dir.to_string_lossy();
        assert!(
            !commands.iter().any(|cmd| cmd.contains(app_dir_str.as_ref())),
            "a lock blocked on its own pending publish must not appear in the plan: {commands:?}"
        );
    }

    #[test]
    fn errors_when_no_cargo_lock_is_discovered_anywhere() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"demo_lib\"\nversion = \"1.2.3\"\n",
        )
        .expect("root manifest");

        let result = rust_update_commands_in(&config(temp.path()), temp.path(), false);

        assert!(
            result.is_err(),
            "no discovered Cargo.lock at all must be a hard error, not a silent no-op"
        );
    }
}
