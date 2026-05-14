//! `NewAlefConfig` and `ResolveError` — the multi-crate config schema.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::extras::Language;
use super::output::{BuildCommandConfig, GeneratedHeaderConfig, PrecommitConfig, ScaffoldConfig};
use super::raw_crate::RawCrateConfig;
use super::resolve_helpers::{merge_map, resolve_output_paths};
use super::resolved::ResolvedCrateConfig;
use super::workspace::WorkspaceConfig;

/// Error variants produced when resolving a [`NewAlefConfig`] into per-crate views.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Two `[[crates]]` entries share the same `name`.
    #[error("duplicate crate name `{0}` — every [[crates]] entry must have a unique name")]
    DuplicateCrateName(String),

    /// A crate has no target languages after merging workspace and per-crate config.
    #[error("crate `{0}` has no target languages — set `languages` on the crate or in `[workspace]`")]
    EmptyLanguages(String),

    /// Two or more crates would write to the same output path for the same language.
    #[error(
        "overlapping output path for language `{lang}`: `{path}` is claimed by crates: {crates}",
        path = path.display(),
        crates = crates.join(", ")
    )]
    OverlappingOutputPath {
        lang: String,
        path: PathBuf,
        crates: Vec<String>,
    },
}

/// Top-level multi-crate configuration (new schema).
///
/// Deserializes from an `alef.toml` that has a `[workspace]` section and one
/// or more `[[crates]]` entries.  Call [`NewAlefConfig::resolve`] to produce
/// the per-crate [`ResolvedCrateConfig`] list that backends consume.
///
/// ```toml
/// [workspace]
/// languages = ["python", "node"]
///
/// [[crates]]
/// name = "spikard"
/// sources = ["src/lib.rs"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewAlefConfig {
    /// Workspace-level shared defaults.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// One entry per independently published binding package.
    pub crates: Vec<RawCrateConfig>,
}

impl NewAlefConfig {
    /// Merge workspace defaults into each crate and validate the result.
    ///
    /// Returns a `Vec<ResolvedCrateConfig>` in the same order as `self.crates`.
    ///
    /// # Errors
    ///
    /// - [`ResolveError::DuplicateCrateName`] when two crates share a name.
    /// - [`ResolveError::EmptyLanguages`] when a crate has no target languages.
    /// - [`ResolveError::OverlappingOutputPath`] when two crates resolve to the
    ///   same output directory for the same language.
    pub fn resolve(&self) -> Result<Vec<ResolvedCrateConfig>, ResolveError> {
        // --- Uniqueness check ---------------------------------------------------
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (idx, krate) in self.crates.iter().enumerate() {
            if seen.insert(krate.name.as_str(), idx).is_some() {
                return Err(ResolveError::DuplicateCrateName(krate.name.clone()));
            }
        }

        let multi_crate = self.crates.len() > 1;
        let mut resolved: Vec<ResolvedCrateConfig> = Vec::with_capacity(self.crates.len());

        for krate in &self.crates {
            resolved.push(self.resolve_one(krate, multi_crate)?);
        }

        // --- Overlapping output path check --------------------------------------
        // For each language, build a map path → crate names; error on any dup.
        let mut path_owners: HashMap<String, HashMap<PathBuf, Vec<String>>> = HashMap::new();
        for cfg in &resolved {
            for (lang, path) in &cfg.output_paths {
                path_owners
                    .entry(lang.clone())
                    .or_default()
                    .entry(path.clone())
                    .or_default()
                    .push(cfg.name.clone());
            }
        }
        for (lang, path_map) in path_owners {
            for (path, crates) in path_map {
                if crates.len() > 1 {
                    return Err(ResolveError::OverlappingOutputPath { lang, path, crates });
                }
            }
        }

        Ok(resolved)
    }

    fn resolve_one(&self, krate: &RawCrateConfig, multi_crate: bool) -> Result<ResolvedCrateConfig, ResolveError> {
        let ws = &self.workspace;

        // --- Languages ----------------------------------------------------------
        let languages: Vec<Language> = match krate.languages.as_deref() {
            Some(langs) if !langs.is_empty() => langs.to_vec(),
            Some(_) => {
                // Explicitly empty per-crate list: treat as "no override" and use workspace.
                if ws.languages.is_empty() {
                    return Err(ResolveError::EmptyLanguages(krate.name.clone()));
                }
                ws.languages.clone()
            }
            None => {
                if ws.languages.is_empty() {
                    return Err(ResolveError::EmptyLanguages(krate.name.clone()));
                }
                ws.languages.clone()
            }
        };

        // --- Output paths -------------------------------------------------------
        let output_paths = resolve_output_paths(krate, &ws.output_template, &languages, multi_crate);

        // --- HashMap pipelines -------------------------------------------------
        // Most per-language pipeline maps use per-key wholesale overlay. Build
        // commands are intentionally field-wise so workspace defaults and crate
        // overrides can compose without restating preconditions/release commands.
        // `path_mappings` and `extra_dependencies` are intentionally NOT merged
        // here: WorkspaceConfig has no fields for them, so they remain strictly
        // per-crate (taken verbatim below).
        let lint = merge_map(&ws.lint, &krate.lint);
        let test = merge_map(&ws.test, &krate.test);
        let setup = merge_map(&ws.setup, &krate.setup);
        let update = merge_map(&ws.update, &krate.update);
        let clean = merge_map(&ws.clean, &krate.clean);
        let build_commands = merge_build_command_maps(&ws.build_commands, &krate.build_commands);
        let format_overrides = merge_map(&ws.format_overrides, &krate.format_overrides);
        let generate_overrides = merge_map(&ws.generate_overrides, &krate.generate_overrides);

        Ok(ResolvedCrateConfig {
            name: krate.name.clone(),
            sources: krate.sources.clone(),
            source_crates: krate.source_crates.clone(),
            version_from: krate.version_from.clone().unwrap_or_else(|| "Cargo.toml".to_string()),
            core_import: krate.core_import.clone(),
            workspace_root: krate.workspace_root.clone(),
            skip_core_import: krate.skip_core_import,
            error_type: krate.error_type.clone(),
            error_constructor: krate.error_constructor.clone(),
            features: krate.features.clone(),
            path_mappings: krate.path_mappings.clone(),
            extra_dependencies: krate.extra_dependencies.clone(),
            auto_path_mappings: krate.auto_path_mappings.unwrap_or(true),
            languages,
            python: krate.python.clone(),
            node: krate.node.clone(),
            ruby: krate.ruby.clone(),
            php: krate.php.clone(),
            elixir: krate.elixir.clone(),
            wasm: krate.wasm.clone(),
            ffi: krate.ffi.clone(),
            go: krate.go.clone(),
            java: krate.java.clone(),
            dart: krate.dart.clone(),
            kotlin: krate.kotlin.clone(),
            swift: krate.swift.clone(),
            gleam: krate.gleam.clone(),
            csharp: krate.csharp.clone(),
            r: krate.r.clone(),
            zig: krate.zig.clone(),
            exclude: krate.exclude.clone(),
            include: krate.include.clone(),
            output_paths,
            explicit_output: krate.output.clone(),
            lint,
            test,
            setup,
            update,
            clean,
            build_commands,
            // Per-crate generate/format/dto override the workspace value when set.
            // None inherits the workspace default. tools and opaque_types are
            // workspace-only by design (see WorkspaceConfig docs).
            generate: krate.generate.clone().unwrap_or_else(|| ws.generate.clone()),
            generate_overrides,
            format: krate.format.clone().unwrap_or_else(|| ws.format.clone()),
            format_overrides,
            dto: krate.dto.clone().unwrap_or_else(|| ws.dto.clone()),
            tools: ws.tools.clone(),
            opaque_types: ws.opaque_types.clone(),
            sync: ws.sync.clone(),
            publish: krate.publish.clone(),
            e2e: krate.e2e.clone(),
            adapters: krate.adapters.clone(),
            trait_bridges: krate.trait_bridges.clone(),
            scaffold: merge_scaffold(
                ws.scaffold.as_ref(),
                krate.scaffold.as_ref(),
                ws.generated_header.as_ref(),
                ws.precommit.as_ref(),
            ),
            readme: krate.readme.clone(),
            custom_files: krate.custom_files.clone(),
            custom_modules: krate.custom_modules.clone(),
            custom_registrations: krate.custom_registrations.clone(),
        })
    }
}

fn merge_scaffold(
    workspace: Option<&ScaffoldConfig>,
    krate: Option<&ScaffoldConfig>,
    workspace_header: Option<&GeneratedHeaderConfig>,
    workspace_precommit: Option<&PrecommitConfig>,
) -> Option<ScaffoldConfig> {
    if workspace.is_none() && krate.is_none() && workspace_header.is_none() && workspace_precommit.is_none() {
        return None;
    }

    let generated_header = merge_generated_header(
        workspace.and_then(|s| s.generated_header.as_ref()).or(workspace_header),
        krate.and_then(|s| s.generated_header.as_ref()),
    );
    let precommit = merge_precommit(
        workspace.and_then(|s| s.precommit.as_ref()).or(workspace_precommit),
        krate.and_then(|s| s.precommit.as_ref()),
    );

    Some(ScaffoldConfig {
        description: krate
            .and_then(|s| s.description.clone())
            .or_else(|| workspace.and_then(|s| s.description.clone())),
        license: krate
            .and_then(|s| s.license.clone())
            .or_else(|| workspace.and_then(|s| s.license.clone())),
        repository: krate
            .and_then(|s| s.repository.clone())
            .or_else(|| workspace.and_then(|s| s.repository.clone())),
        homepage: krate
            .and_then(|s| s.homepage.clone())
            .or_else(|| workspace.and_then(|s| s.homepage.clone())),
        authors: krate
            .filter(|s| !s.authors.is_empty())
            .map(|s| s.authors.clone())
            .or_else(|| workspace.map(|s| s.authors.clone()))
            .unwrap_or_default(),
        keywords: krate
            .filter(|s| !s.keywords.is_empty())
            .map(|s| s.keywords.clone())
            .or_else(|| workspace.map(|s| s.keywords.clone()))
            .unwrap_or_default(),
        generated_header,
        precommit,
        cargo: krate
            .and_then(|s| s.cargo.clone())
            .or_else(|| workspace.and_then(|s| s.cargo.clone())),
    })
}

fn merge_generated_header(
    workspace: Option<&GeneratedHeaderConfig>,
    krate: Option<&GeneratedHeaderConfig>,
) -> Option<GeneratedHeaderConfig> {
    if workspace.is_none() && krate.is_none() {
        return None;
    }
    Some(GeneratedHeaderConfig {
        issues_url: krate
            .and_then(|h| h.issues_url.clone())
            .or_else(|| workspace.and_then(|h| h.issues_url.clone())),
        regenerate_command: krate
            .and_then(|h| h.regenerate_command.clone())
            .or_else(|| workspace.and_then(|h| h.regenerate_command.clone())),
        verify_command: krate
            .and_then(|h| h.verify_command.clone())
            .or_else(|| workspace.and_then(|h| h.verify_command.clone())),
    })
}

fn merge_precommit(workspace: Option<&PrecommitConfig>, krate: Option<&PrecommitConfig>) -> Option<PrecommitConfig> {
    if workspace.is_none() && krate.is_none() {
        return None;
    }
    Some(PrecommitConfig {
        include_shared_hooks: krate
            .and_then(|p| p.include_shared_hooks)
            .or_else(|| workspace.and_then(|p| p.include_shared_hooks)),
        shared_hooks_repo: krate
            .and_then(|p| p.shared_hooks_repo.clone())
            .or_else(|| workspace.and_then(|p| p.shared_hooks_repo.clone())),
        shared_hooks_rev: krate
            .and_then(|p| p.shared_hooks_rev.clone())
            .or_else(|| workspace.and_then(|p| p.shared_hooks_rev.clone())),
        include_alef_hooks: krate
            .and_then(|p| p.include_alef_hooks)
            .or_else(|| workspace.and_then(|p| p.include_alef_hooks)),
        alef_hooks_repo: krate
            .and_then(|p| p.alef_hooks_repo.clone())
            .or_else(|| workspace.and_then(|p| p.alef_hooks_repo.clone())),
        alef_hooks_rev: krate
            .and_then(|p| p.alef_hooks_rev.clone())
            .or_else(|| workspace.and_then(|p| p.alef_hooks_rev.clone())),
    })
}

fn merge_build_command_maps(
    workspace: &HashMap<String, BuildCommandConfig>,
    krate: &HashMap<String, BuildCommandConfig>,
) -> HashMap<String, BuildCommandConfig> {
    let mut merged = workspace.clone();
    for (lang, override_cfg) in krate {
        let next = merged
            .remove(lang)
            .map(|base| base.merge_overlay(override_cfg))
            .unwrap_or_else(|| override_cfg.clone());
        merged.insert(lang.clone(), next);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::dto;
    use crate::config::extras::Language;

    fn two_crate_config() -> NewAlefConfig {
        toml::from_str(
            r#"
[workspace]
languages = ["python", "node"]

[workspace.output_template]
python = "packages/python/{crate}/"
node   = "packages/node/{crate}/"

[[crates]]
name = "alpha"
sources = ["crates/alpha/src/lib.rs"]

[[crates]]
name = "beta"
sources = ["crates/beta/src/lib.rs"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn resolve_single_crate_inherits_workspace_languages() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert_eq!(resolved.len(), 1);
        let spikard = &resolved[0];
        assert_eq!(spikard.name, "spikard");
        assert_eq!(spikard.languages.len(), 2);
        assert!(spikard.languages.contains(&Language::Python));
        assert!(spikard.languages.contains(&Language::Go));
    }

    #[test]
    fn resolve_per_crate_languages_override_workspace() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
languages = ["node"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        let spikard = &resolved[0];
        assert_eq!(spikard.languages, vec![Language::Node]);
    }

    #[test]
    fn resolve_merges_workspace_scaffold_field_by_field() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[workspace.scaffold]
description = "Workspace description"
license = "MIT"
repository = "https://github.com/acme/workspace"
authors = ["Workspace Team"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Crate description"
keywords = ["bindings"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().unwrap().remove(0);
        let scaffold = resolved.scaffold.unwrap();
        assert_eq!(scaffold.description.as_deref(), Some("Crate description"));
        assert_eq!(scaffold.license.as_deref(), Some("MIT"));
        assert_eq!(
            scaffold.repository.as_deref(),
            Some("https://github.com/acme/workspace")
        );
        assert_eq!(scaffold.authors, vec!["Workspace Team"]);
        assert_eq!(scaffold.keywords, vec!["bindings"]);
    }

    #[test]
    fn resolve_merges_workspace_header_and_precommit_defaults() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[workspace.generated_header]
issues_url = "https://docs.example.invalid/alef"

[workspace.precommit]
shared_hooks_repo = "https://github.com/acme/hooks"
include_alef_hooks = false

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[crates.scaffold.generated_header]
verify_command = "spikard verify"

[crates.scaffold.precommit]
shared_hooks_rev = "v1.2.3"
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().unwrap().remove(0);
        let scaffold = resolved.scaffold.unwrap();
        let header = scaffold.generated_header.unwrap();
        let precommit = scaffold.precommit.unwrap();

        assert_eq!(header.issues_url.as_deref(), Some("https://docs.example.invalid/alef"));
        assert_eq!(header.verify_command.as_deref(), Some("spikard verify"));
        assert_eq!(
            precommit.shared_hooks_repo.as_deref(),
            Some("https://github.com/acme/hooks")
        );
        assert_eq!(precommit.shared_hooks_rev.as_deref(), Some("v1.2.3"));
        assert_eq!(precommit.include_alef_hooks, Some(false));
    }

    #[test]
    fn resolve_build_commands_merges_workspace_and_crate_fields() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["go"]

[workspace.build_commands.go]
precondition = "command -v go"
before = "cargo build --release -p my-lib-ffi"
build = "cd packages/go && go build ./..."
build_release = "cd packages/go && go build -tags release ./..."

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.build_commands.go]
build = "cd packages/go && go build -tags dev ./..."
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed").remove(0);
        let build = resolved.build_commands.get("go").expect("go build config");
        assert_eq!(build.precondition.as_deref(), Some("command -v go"));
        assert_eq!(
            build.before.as_ref().unwrap().commands(),
            vec!["cargo build --release -p my-lib-ffi"]
        );
        assert_eq!(
            build.build.as_ref().unwrap().commands(),
            vec!["cd packages/go && go build -tags dev ./..."]
        );
        assert_eq!(
            build.build_release.as_ref().unwrap().commands(),
            vec!["cd packages/go && go build -tags release ./..."]
        );
    }

    #[test]
    fn new_alef_config_resolve_propagates_field_renames() {
        // Per-language `rename_fields` declared on a `[crates.<lang>]` table must
        // survive resolution intact — the resolver replaces the per-language
        // config wholesale rather than merging field-by-field.
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_spikard"

[crates.python.rename_fields]
"User.type" = "user_type"
"User.id" = "identifier"

[crates.node]
package_name = "@spikard/node"

[crates.node.rename_fields]
"User.type" = "userType"
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        let spikard = &resolved[0];

        let py = spikard.python.as_ref().expect("python config should be present");
        assert_eq!(py.rename_fields.get("User.type").map(String::as_str), Some("user_type"));
        assert_eq!(py.rename_fields.get("User.id").map(String::as_str), Some("identifier"));

        let node_cfg = spikard.node.as_ref().expect("node config should be present");
        assert_eq!(
            node_cfg.rename_fields.get("User.type").map(String::as_str),
            Some("userType")
        );
    }

    #[test]
    fn resolve_workspace_lint_default_merged_with_crate_override() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "node"]

[workspace.lint.python]
check = "ruff check ."

[workspace.lint.node]
check = "oxlint ."

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[crates.lint.python]
check = "ruff check crates/spikard-py/"
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        let spikard = &resolved[0];

        // Per-crate python lint overrides workspace
        let py_lint = spikard.lint.get("python").expect("python lint should be present");
        assert_eq!(
            py_lint.check.as_ref().unwrap().commands(),
            vec!["ruff check crates/spikard-py/"],
            "per-crate python lint should win over workspace default"
        );

        // Workspace node lint is inherited (no per-crate override)
        let node_lint = spikard.lint.get("node").expect("node lint should be present");
        assert_eq!(
            node_lint.check.as_ref().unwrap().commands(),
            vec!["oxlint ."],
            "workspace node lint should be inherited when no per-crate override"
        );
    }

    #[test]
    fn resolve_multi_crate_output_paths_use_template() {
        let cfg = two_crate_config();
        let resolved = cfg.resolve().expect("resolve should succeed");

        let alpha = resolved.iter().find(|c| c.name == "alpha").unwrap();
        let beta = resolved.iter().find(|c| c.name == "beta").unwrap();

        assert_eq!(
            alpha.output_paths.get("python"),
            Some(&std::path::PathBuf::from("packages/python/alpha/")),
            "alpha python output path"
        );
        assert_eq!(
            beta.output_paths.get("python"),
            Some(&std::path::PathBuf::from("packages/python/beta/")),
            "beta python output path"
        );
        assert_eq!(
            alpha.output_paths.get("node"),
            Some(&std::path::PathBuf::from("packages/node/alpha/")),
            "alpha node output path"
        );
    }

    #[test]
    fn resolve_duplicate_crate_name_errors() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[[crates]]
name = "spikard"
sources = ["src/other.rs"]
"#,
        )
        .unwrap();

        let err = cfg.resolve().unwrap_err();
        assert!(
            matches!(err, ResolveError::DuplicateCrateName(ref n) if n == "spikard"),
            "expected DuplicateCrateName(spikard), got: {err}"
        );
    }

    #[test]
    fn resolve_empty_languages_errors_when_workspace_also_empty() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let err = cfg.resolve().unwrap_err();
        assert!(
            matches!(err, ResolveError::EmptyLanguages(ref n) if n == "spikard"),
            "expected EmptyLanguages(spikard), got: {err}"
        );
    }

    #[test]
    fn resolve_overlapping_output_path_errors() {
        // Both crates have no template and identical names would collide; force
        // a collision by using an explicit output path on both.
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "alpha"
sources = ["src/lib.rs"]

[crates.output]
python = "packages/python/shared/"

[[crates]]
name = "beta"
sources = ["src/other.rs"]

[crates.output]
python = "packages/python/shared/"
"#,
        )
        .unwrap();

        let err = cfg.resolve().unwrap_err();
        assert!(
            matches!(err, ResolveError::OverlappingOutputPath { ref lang, .. } if lang == "python"),
            "expected OverlappingOutputPath for python, got: {err}"
        );
    }

    #[test]
    fn resolve_version_from_defaults_to_cargo_toml() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert_eq!(resolved[0].version_from, "Cargo.toml");
    }

    #[test]
    fn resolve_auto_path_mappings_defaults_to_true() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert!(resolved[0].auto_path_mappings);
    }

    #[test]
    fn resolve_workspace_tools_and_dto_flow_through() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[workspace.tools]
python_package_manager = "uv"

[workspace.opaque_types]
Tree = "tree_sitter::Tree"

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert_eq!(resolved[0].tools.python_package_manager.as_deref(), Some("uv"));
        assert_eq!(
            resolved[0].opaque_types.get("Tree").map(String::as_str),
            Some("tree_sitter::Tree")
        );
    }

    #[test]
    fn resolve_workspace_generate_format_dto_flow_through_when_crate_unset() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[workspace.generate]
public_api = false
bindings = false

[workspace.format]
enabled = false

[workspace.dto]
python = "typed-dict"
node   = "zod"

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert!(
            !resolved[0].generate.public_api,
            "workspace generate.public_api must flow through"
        );
        assert!(
            !resolved[0].generate.bindings,
            "workspace generate.bindings must flow through"
        );
        assert!(
            !resolved[0].format.enabled,
            "workspace format.enabled must flow through"
        );
        assert!(matches!(resolved[0].dto.python, dto::PythonDtoStyle::TypedDict));
        assert!(matches!(resolved[0].dto.node, dto::NodeDtoStyle::Zod));
    }

    #[test]
    fn resolve_per_crate_generate_format_dto_override_workspace() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[workspace.generate]
public_api = false

[workspace.format]
enabled = false

[workspace.dto]
python = "typed-dict"

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]

[crates.generate]
public_api = true

[crates.format]
enabled = true

[crates.dto]
python = "dataclass"
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert!(
            resolved[0].generate.public_api,
            "per-crate generate.public_api must override workspace"
        );
        assert!(
            resolved[0].format.enabled,
            "per-crate format.enabled must override workspace"
        );
        assert!(
            matches!(resolved[0].dto.python, dto::PythonDtoStyle::Dataclass),
            "per-crate dto.python must override workspace"
        );
    }

    #[test]
    fn resolve_per_crate_explicit_empty_languages_inherits_workspace() {
        // Explicit `languages = []` per-crate falls back to workspace defaults
        // (matches the behavior the resolver already implements).
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
languages = []
"#,
        )
        .unwrap();

        let resolved = cfg.resolve().expect("resolve should succeed");
        assert_eq!(resolved[0].languages, vec![Language::Python, Language::Node]);
    }

    #[test]
    fn resolve_per_crate_empty_languages_with_empty_workspace_errors() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
languages = []
"#,
        )
        .unwrap();

        let err = cfg
            .resolve()
            .expect_err("resolve must fail when both per-crate and workspace languages are empty");
        match err {
            ResolveError::EmptyLanguages(name) => assert_eq!(name, "spikard"),
            other => panic!("expected EmptyLanguages, got {other:?}"),
        }
    }

    // --- deny_unknown_fields tests ---

    #[test]
    fn unknown_top_level_key_is_rejected() {
        // A misspelled key must produce a parse error, not silently succeed with the
        // field ignored.
        // typos: ignore start
        let result: Result<NewAlefConfig, _> = toml::from_str(
            r#"
wrkspace = "typo"

[[crates]]
name = "spikard"
sources = ["src/lib.rs"]
"#,
        );
        // typos: ignore end
        assert!(
            result.is_err(),
            "unknown top-level key should be rejected by deny_unknown_fields"
        );
    }

    // --- new backfill tests ---

    #[test]
    fn new_alef_config_resolve_rejects_duplicate_crate_name() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "dup"
sources = ["src/lib.rs"]

[[crates]]
name = "dup"
sources = ["src/other.rs"]
"#,
        )
        .unwrap();
        let err = cfg.resolve().unwrap_err();
        assert!(matches!(err, ResolveError::DuplicateCrateName(ref n) if n == "dup"));
    }

    #[test]
    fn new_alef_config_resolve_rejects_overlapping_output_paths() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "a"
sources = ["src/lib.rs"]

[crates.output]
python = "packages/python/shared/"

[[crates]]
name = "b"
sources = ["src/other.rs"]

[crates.output]
python = "packages/python/shared/"
"#,
        )
        .unwrap();
        let err = cfg.resolve().unwrap_err();
        assert!(matches!(err, ResolveError::OverlappingOutputPath { ref lang, .. } if lang == "python"));
    }

    #[test]
    fn new_alef_config_resolve_per_crate_languages_overrides_workspace() {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "x"
sources = ["src/lib.rs"]
languages = ["node"]
"#,
        )
        .unwrap();
        let resolved = cfg.resolve().unwrap();
        assert_eq!(resolved[0].languages, vec![Language::Node]);
    }
}
