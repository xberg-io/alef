use crate::cli::pipeline::helpers::{check_precondition, run_before, run_command_streamed};
use crate::core::config::{Language, ResolvedCrateConfig};
use rayon::prelude::*;
use tracing::warn;

enum LintPhase {
    Format,
    Check,
    Typecheck,
}

/// Filter languages through precondition and before hooks for lint/fmt.
///
/// Returns the subset of languages whose precondition passed (or was absent).
/// Runs before hooks for each passing language. Fails if any before hook fails.
fn prepare_lint_languages(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<Vec<Language>> {
    let mut ready = Vec::with_capacity(languages.len());
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            continue;
        }
        run_before(lang, lang_lint.before.as_ref())?;
        ready.push(lang);
    }
    Ok(ready)
}

/// Run a single lint phase across all languages in parallel.
fn run_phase(config: &ResolvedCrateConfig, languages: &[Language], phase: LintPhase) -> anyhow::Result<()> {
    let tasks: Vec<(&Language, String)> = languages
        .iter()
        .filter_map(|lang| {
            let lang_lint = config.lint_config_for_language(*lang);
            let cmds = match phase {
                LintPhase::Format => lang_lint.format,
                LintPhase::Check => lang_lint.check,
                LintPhase::Typecheck => lang_lint.typecheck,
            };
            cmds.map(|cmd_list| {
                cmd_list
                    .commands()
                    .into_iter()
                    .map(|c| (lang, c.to_string()))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();

    let results: Vec<anyhow::Result<()>> = tasks
        .par_iter()
        .map(|(lang, cmd)| run_command_streamed(cmd, Some(&lang.to_string())))
        .collect();

    let mut first_error: Option<anyhow::Error> = None;
    for result in results {
        if let Err(e) = result {
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

/// Run all configured lint/format commands on generated output.
///
/// Executes in two waves for correctness:
/// 1. Format (all languages in parallel) — normalizes code first
/// 2. Check + Typecheck (all languages in parallel) — validates normalized code
pub fn lint(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    // Wave 1: format all languages in parallel
    run_phase(config, &ready, LintPhase::Format)?;
    // Wave 2: check + typecheck all languages in parallel
    run_phase(config, &ready, LintPhase::Check)?;
    run_phase(config, &ready, LintPhase::Typecheck)?;
    Ok(())
}

/// Run only format commands on generated output.
pub fn fmt(config: &ResolvedCrateConfig, languages: &[Language]) -> anyhow::Result<()> {
    let ready = prepare_lint_languages(config, languages)?;
    run_phase(config, &ready, LintPhase::Format)
}

/// Run format commands as part of post-generation, never propagating failure.
///
/// Formatting after `alef generate` is best-effort: formatters are *expected*
/// to modify generated files (that's their purpose), and a missing formatter,
/// a config issue, or a non-zero exit must not abort the generate run. Each
/// per-language failure is logged as a warning and processing continues with
/// the next language.
pub fn fmt_post_generate(config: &ResolvedCrateConfig, languages: &[Language]) {
    for &lang in languages {
        let lang_lint = config.lint_config_for_language(lang);
        if !check_precondition(lang, lang_lint.precondition.as_deref()) {
            // `check_precondition` already warns and returns false on miss.
            continue;
        }
        if let Err(e) = run_before(lang, lang_lint.before.as_ref()) {
            warn!("[{lang}] post-generation `before` hook failed: {e:#}");
            continue;
        }
        let Some(cmd_list) = lang_lint.format else {
            continue;
        };
        for cmd in cmd_list.commands() {
            if let Err(e) = run_command_streamed(cmd, Some(&lang.to_string())) {
                warn!("[{lang}] post-generation format command failed (continuing): {e:#}");
            }
        }
    }
}

#[cfg(all(test, unix))]
mod fmt_post_generate_tests {
    // Tests in this module rely on `sh -c` and the POSIX `true`/`false`
    // builtins. They are skipped on Windows where `sh` is not on PATH and
    // a missing-program error is indistinguishable from a precondition
    // miss (both cause `check_precondition` to return false), which would
    // make every test trivially pass for the wrong reason.
    use super::*;
    use crate::core::config::output::{LintConfig, StringOrVec};

    fn config_with_lint(lang: Language, cfg: LintConfig) -> ResolvedCrateConfig {
        let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        let mut c = alef_cfg.resolve().unwrap().remove(0);
        c.lint.insert(lang.to_string(), cfg);
        c
    }

    #[test]
    fn fmt_post_generate_swallows_failing_format_command() {
        // A format command that exits non-zero must not panic or propagate;
        // fmt_post_generate has no return value, so reaching the end is the
        // contract.
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_swallows_failing_before_hook() {
        let lint = LintConfig {
            precondition: Some("true".to_string()),
            before: Some(StringOrVec::Single("false".to_string())),
            format: Some(StringOrVec::Single("true".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        fmt_post_generate(&cfg, &[Language::Python]);
    }

    #[test]
    fn fmt_post_generate_skips_when_precondition_fails() {
        let lint = LintConfig {
            precondition: Some("false".to_string()),
            before: None,
            format: Some(StringOrVec::Single("false".to_string())),
            check: None,
            typecheck: None,
        };
        let cfg = config_with_lint(Language::Python, lint);
        // Precondition is `false` → format step never runs, so the (otherwise
        // failing) `false` command isn't invoked. No panic, no propagation.
        fmt_post_generate(&cfg, &[Language::Python]);
    }
}
