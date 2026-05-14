use alef_core::backend::GeneratedFile;
use alef_core::config::{Language, PrecommitConfig, ResolvedCrateConfig};
use alef_core::template_versions as tv;
use std::path::PathBuf;

const DEFAULT_SHARED_HOOKS_REPO: &str = "https://github.com/kreuzberg-dev/pre-commit-hooks";
const DEFAULT_ALEF_HOOKS_REPO: &str = "https://github.com/kreuzberg-dev/alef";

pub(crate) fn scaffold_pre_commit_config(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    if std::path::Path::new(".pre-commit-config.yaml").exists() {
        return vec![];
    }
    generate_pre_commit_config(config, languages)
}

/// Generate the `.pre-commit-config.yaml` content based on configured languages.
///
/// Separated from `scaffold_pre_commit_config` for testability.
pub(crate) fn generate_pre_commit_config(config: &ResolvedCrateConfig, languages: &[Language]) -> Vec<GeneratedFile> {
    let has = |lang: Language| languages.contains(&lang);
    let crate_dir = config.core_crate_dir();
    let precommit = config.scaffold.as_ref().and_then(|s| s.precommit.as_ref());

    // Build clippy --exclude args for binding crates that need special compilation
    // (native extensions with host-incompatible link flags). Wasm is NOT excluded
    // because rust-toolchain.toml includes the wasm32 target.
    let clippy_excludes = {
        let suffixes: &[(&str, Language)] = &[
            ("-py", Language::Python),
            ("-node", Language::Node),
            ("-php", Language::Php),
            ("-rb", Language::Ruby),
            ("-r", Language::R),
        ];
        let mut excludes = String::new();
        for (suffix, lang) in suffixes {
            if has(*lang) {
                excludes.push_str(&crate::template_env::render(
                    "precommit_clippy_exclude.jinja",
                    minijinja::context! { crate_dir => &crate_dir, suffix => suffix },
                ));
            }
        }
        excludes
    };

    let yaml = crate::template_env::render(
        "precommit_config.yaml.jinja",
        minijinja::context! {
            has_python => has(Language::Python),
            clippy_excludes => clippy_excludes,
            gitfluff => tv::precommit::GITFLUFF_REV,
            pre_commit_hooks => tv::precommit::PRE_COMMIT_HOOKS_REV,
            pyproject_fmt => tv::precommit::PYPROJECT_FMT_REV,
            cargo_sort => tv::precommit::CARGO_SORT_REV,
            pre_commit_cargo => tv::precommit::PRE_COMMIT_CARGO_REV,
            cargo_machete => tv::precommit::CARGO_MACHETE_REV,
            cargo_deny => tv::precommit::CARGO_DENY_REV,
            rumdl => tv::precommit::RUMDL_REV,
            include_shared_hooks => precommit_bool(precommit, |p| p.include_shared_hooks, true),
            shared_hooks_repo => precommit_string(
                precommit,
                |p| p.shared_hooks_repo.as_deref(),
                DEFAULT_SHARED_HOOKS_REPO,
            ),
            shared_hooks_rev => precommit_string(
                precommit,
                |p| p.shared_hooks_rev.as_deref(),
                tv::precommit::KREUZBERG_PRECOMMIT_HOOKS_REV,
            ),
            include_alef_hooks => precommit_bool(precommit, |p| p.include_alef_hooks, true),
            alef_hooks_repo => precommit_string(precommit, |p| p.alef_hooks_repo.as_deref(), DEFAULT_ALEF_HOOKS_REPO),
            alef_hooks_rev => precommit_string(precommit, |p| p.alef_hooks_rev.as_deref(), tv::precommit::ALEF_REV),
            typos => tv::precommit::TYPOS_REV,
        },
    );

    vec![GeneratedFile {
        path: PathBuf::from(".pre-commit-config.yaml"),
        content: yaml,
        generated_header: false,
    }]
}

fn precommit_bool(
    config: Option<&PrecommitConfig>,
    field: impl Fn(&PrecommitConfig) -> Option<bool>,
    default: bool,
) -> bool {
    config.and_then(field).unwrap_or(default)
}

fn precommit_string<'a>(
    config: Option<&'a PrecommitConfig>,
    field: impl Fn(&'a PrecommitConfig) -> Option<&'a str>,
    default: &'a str,
) -> &'a str {
    config.and_then(field).unwrap_or(default)
}
