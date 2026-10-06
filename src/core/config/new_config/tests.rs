use super::*;
use crate::core::config::dto;
use crate::core::config::extras::Language;

pub(super) fn two_crate_config() -> NewAlefConfig {
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
name = "sample_router"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().expect("resolve should succeed");
    assert_eq!(resolved.len(), 1);
    let sample_router = &resolved[0];
    assert_eq!(sample_router.name, "sample_router");
    assert_eq!(sample_router.languages.len(), 2);
    assert!(sample_router.languages.contains(&Language::Python));
    assert!(sample_router.languages.contains(&Language::Go));
}

#[test]
fn resolve_per_crate_languages_override_workspace() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
languages = ["node"]
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().expect("resolve should succeed");
    let sample_router = &resolved[0];
    assert_eq!(sample_router.languages, vec![Language::Node]);
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
name = "sample_router"
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
fn resolve_merges_workspace_generated_header_defaults() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.generated_header]
issues_url = "https://docs.example.invalid/alef"

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.scaffold.generated_header]
verify_command = "sample_router verify"
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().unwrap().remove(0);
    let scaffold = resolved.scaffold.unwrap();
    let header = scaffold.generated_header.unwrap();

    assert_eq!(header.issues_url.as_deref(), Some("https://docs.example.invalid/alef"));
    assert_eq!(header.verify_command.as_deref(), Some("sample_router verify"));
}

/// 0.82.0 removed `[build_commands.<lang>]` from the schema entirely: a leftover
/// `[workspace.build_commands.go]` (this exact fixture parsed and merged cleanly before the
/// removal -- see the deleted `resolve_build_commands_merges_workspace_and_crate_fields` this
/// test replaces) must now be a parse error. ~keep
#[test]
fn resolve_rejects_removed_build_commands_table() {
    let err = toml::from_str::<NewAlefConfig>(
        r#"
[workspace]
languages = ["go"]

[workspace.build_commands.go]
build = "cd packages/go && go build ./..."

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
    )
    .expect_err("[workspace.build_commands] must no longer parse");
    assert!(
        err.to_string().contains("build_commands"),
        "error should name the removed `build_commands` field: {err}"
    );
}

#[test]
fn new_alef_config_resolve_propagates_field_renames() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_sample_router"

[crates.python.rename_fields]
"User.type" = "user_type"
"User.id" = "identifier"

[crates.node]
package_name = "@sample_router/node"

[crates.node.rename_fields]
"User.type" = "userType"
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().expect("resolve should succeed");
    let sample_router = &resolved[0];

    let py = sample_router.python.as_ref().expect("python config should be present");
    assert_eq!(py.rename_fields.get("User.type").map(String::as_str), Some("user_type"));
    assert_eq!(py.rename_fields.get("User.id").map(String::as_str), Some("identifier"));

    let node_cfg = sample_router.node.as_ref().expect("node config should be present");
    assert_eq!(
        node_cfg.rename_fields.get("User.type").map(String::as_str),
        Some("userType")
    );
}

/// 0.82.0 removed `[lint.<lang>]` from the schema entirely: this exact fixture (a workspace
/// default merged with a per-crate override) parsed and merged cleanly before the removal -- see
/// the deleted `resolve_workspace_lint_default_merged_with_crate_override` this test replaces --
/// and must now fail to parse.
#[test]
fn resolve_rejects_removed_lint_table() {
    let err = toml::from_str::<NewAlefConfig>(
        r#"
[workspace]
languages = ["python", "node"]

[workspace.lint.python]
check = "ruff check ."

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
"#,
    )
    .expect_err("[workspace.lint] must no longer parse");
    assert!(
        err.to_string().contains("lint"),
        "error should name the removed `lint` field: {err}"
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
fn resolve_no_crates_errors_instead_of_silently_processing_nothing() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
crates = []

[workspace]
languages = ["python"]
"#,
    )
    .unwrap();

    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(err, ResolveError::NoCratesConfigured),
        "expected NoCratesConfigured, got: {err}"
    );
}

#[test]
fn resolve_duplicate_crate_name_errors() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates]]
name = "sample_router"
sources = ["src/other.rs"]
"#,
    )
    .unwrap();

    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(err, ResolveError::DuplicateCrateName(ref n) if n == "sample_router"),
        "expected DuplicateCrateName(sample_router), got: {err}"
    );
}

#[test]
fn resolve_empty_languages_errors_when_workspace_also_empty() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();

    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(err, ResolveError::EmptyLanguages(ref n) if n == "sample_router"),
        "expected EmptyLanguages(sample_router), got: {err}"
    );
}

#[test]
fn resolve_overlapping_output_path_errors() {
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
name = "sample_router"
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
name = "sample_router"
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
Tree = "sample_language::Tree"

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().expect("resolve should succeed");
    assert_eq!(resolved[0].tools.python_package_manager.as_deref(), Some("uv"));
    assert_eq!(
        resolved[0].opaque_types.get("Tree").map(String::as_str),
        Some("sample_language::Tree")
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

[workspace.dto]
python = "typed-dict"
node   = "zod"

[[crates]]
name = "sample_router"
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

[workspace.dto]
python = "typed-dict"

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.generate]
public_api = true

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
        matches!(resolved[0].dto.python, dto::PythonDtoStyle::Dataclass),
        "per-crate dto.python must override workspace"
    );
}

#[test]
fn resolve_per_crate_explicit_empty_languages_inherits_workspace() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "sample_router"
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
name = "sample_router"
sources = ["src/lib.rs"]
languages = []
"#,
    )
    .unwrap();

    let err = cfg
        .resolve()
        .expect_err("resolve must fail when both per-crate and workspace languages are empty");
    match err {
        ResolveError::EmptyLanguages(name) => assert_eq!(name, "sample_router"),
        other => panic!("expected EmptyLanguages, got {other:?}"),
    }
}

#[test]
fn unknown_top_level_key_is_rejected() {
    let result: Result<NewAlefConfig, _> = toml::from_str(
        r#"
wrkspace = "typo"

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
"#,
    );
    assert!(
        result.is_err(),
        "unknown top-level key should be rejected by deny_unknown_fields"
    );
}

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

/// Regression for the gap `validate_nuget_package_id`'s own doc comment names but that no
/// per-crate check can close: `resolve_one` validates each crate's NuGet package ID in
/// isolation, so two crates whose IDs differ only by case both pass individually and only
/// collide once actually published to nuget.org (which folds case via
/// `StringComparer.OrdinalIgnoreCase`).
#[test]
fn new_alef_config_resolve_rejects_case_insensitive_nuget_collisions() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "crate-a"
sources = ["src/lib.rs"]

[crates.csharp]
package_id = "MyLib"

[[crates]]
name = "crate-b"
sources = ["src/other.rs"]

[crates.csharp]
package_id = "mylib"
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    let ResolveError::InvalidConfig(message) = err else {
        panic!("expected InvalidConfig, got {err:?}");
    };
    assert!(message.contains("crate-a"), "got: {message}");
    assert!(message.contains("crate-b"), "got: {message}");
}
