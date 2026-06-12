use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed};
use crate::core::config::{Language, ResolvedCrateConfig};
use rayon::prelude::*;

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
    // Phase 1 (sequential): check preconditions, run before hooks, collect command lists.
    let mut plans: Vec<(Language, Vec<String>)> = Vec::new();
    for &lang in languages {
        let update_cfg = config.update_config_for_language(lang);
        if !check_precondition(lang, update_cfg.precondition.as_deref()) {
            continue;
        }
        run_before(lang, update_cfg.before.as_ref())?;
        let cmds = if latest {
            update_cfg.upgrade.as_ref()
        } else {
            update_cfg.update.as_ref()
        };
        let cmd_strings: Vec<String> = cmds
            .map(|cmd_list| cmd_list.commands().into_iter().map(|c| c.to_string()).collect())
            .unwrap_or_default();
        plans.push((lang, cmd_strings));
    }

    // Deduplicate shared commands (e.g. pnpm workspace commands shared by Node and Wasm).
    let plans = dedupe_plans(plans);

    // Phase 2 (parallel): run each language's deduped command list with live output.
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
            eprintln!("✗ update failed: {lang} — {e}");
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
            (
                Language::Node,
                vec!["cargo update".to_string()], // hypothetical duplicate
            ),
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
