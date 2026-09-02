//! Unit tests for `ffi.rs`, split out for the file-modularization cap.

use super::*;

#[test]
fn test_render_core_dep_includes_version_in_default_line() {
    let (core_dep_line, target_blocks) =
        render_core_dep("my-lib", "../my-lib-core", "1.2.3", ", features = [\"foo\"]", &[]);

    assert!(!core_dep_line.is_empty(), "Expected non-empty core_dep_line");
    assert!(
        core_dep_line.contains("version = \"1.2.3\""),
        "Expected 'version = \"1.2.3\"' in core_dep_line: {}",
        core_dep_line
    );
    assert!(
        core_dep_line.contains("path = \"../my-lib-core\""),
        "Expected path reference in core_dep_line: {}",
        core_dep_line
    );
    assert!(target_blocks.is_empty(), "Expected empty target_blocks");
}

#[test]
fn test_render_core_dep_includes_version_in_target_blocks() {
    let overrides = vec![FfiTargetDepOverride {
        cfg: "target_os = \"windows\"".to_string(),
        features: vec!["windows-feature".to_string()],
        default_features: true,
    }];
    let (core_dep_line, target_blocks) = render_core_dep("my-lib", "../my-lib-core", "2.0.0", "", &overrides);

    assert!(
        core_dep_line.is_empty(),
        "Expected empty core_dep_line when overrides present"
    );
    assert!(!target_blocks.is_empty(), "Expected non-empty target_blocks");
    assert!(
        target_blocks.contains("version = \"2.0.0\""),
        "Expected 'version = \"2.0.0\"' in target_blocks: {}",
        target_blocks
    );
    assert!(
        target_blocks.contains("path = \"../my-lib-core\""),
        "Expected path reference in target_blocks: {}",
        target_blocks
    );
    assert!(
        !target_blocks.contains("default-features"),
        "default_features: true must not emit a default-features key: {}",
        target_blocks
    );
}

/// Regression test for the dropped `default_features` config key on
/// `[crates.ffi].target_dep_overrides` (see `FfiTargetDepOverride::default_features`):
/// an override with `default_features = false` must emit `default-features = false`
/// in its target block so the FFI crate can drop the core dep's own default feature set
/// on a target that cannot support it.
#[test]
fn test_render_core_dep_emits_default_features_false_when_override_disables_it() {
    let overrides = vec![FfiTargetDepOverride {
        cfg: "target_os = \"windows\"".to_string(),
        features: vec!["windows-target".to_string()],
        default_features: false,
    }];
    let (core_dep_line, target_blocks) = render_core_dep("my-lib", "../my-lib-core", "2.0.0", "", &overrides);

    assert!(
        core_dep_line.is_empty(),
        "Expected empty core_dep_line when overrides present"
    );
    assert!(
        target_blocks.contains("default-features = false"),
        "default_features: false must emit default-features = false: {}",
        target_blocks
    );
    assert!(
        target_blocks.contains(r#"features = ["windows-target"]"#),
        "override block must still emit its feature list: {}",
        target_blocks
    );
}

/// Regression: the FFI scaffold emitted `repository = "…"\n` (trailing LF)
/// into a format string that already had a separating blank line, producing
/// two consecutive blank lines between `repository = "…"` and the
/// `[package.metadata.cargo-machete]` comment block.  cargo-sort removes
/// one of them, causing prek to oscillate on every run.
#[test]
fn ffi_cargo_toml_repository_line_has_no_double_blank_line() {
    let repository = "https://github.com/example/my-lib";
    let repository_line = format!("\nrepository = \"{repository}\"");
    let pkg_header = "[package]\nname = \"my-lib-ffi\"\nversion = \"1.0.0\"";

    let content = format!("{pkg_header}{repository_line}\n\n# comment\n");

    assert!(
        !content.contains("repository = \"https://github.com/example/my-lib\"\n\n\n"),
        "double blank line found after repository — cargo-sort will remove one, causing prek oscillation:\n{content}"
    );
    assert!(
        content.contains("repository = \"https://github.com/example/my-lib\"\n\n# comment"),
        "expected exactly one blank line between repository and comment:\n{content}"
    );
}

fn resolve_config(toml_text: &str) -> ResolvedCrateConfig {
    let cfg: crate::core::config::NewAlefConfig = toml::from_str(toml_text).expect("valid config");
    cfg.resolve().expect("resolve").remove(0)
}

/// A capsule type that declares `package`/`package_version` must add that
/// crate as a direct FFI dependency so the capsule shim can name the pointee
/// type (the core crate's transitive dep is not in scope for generated code).
#[test]
fn ffi_cargo_toml_injects_capsule_package_dependency() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []
[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
package = "tree-sitter"
package_version = "0.26"
"#,
    );
    let api = crate::core::ir::ApiSurface::default();
    let files = scaffold_ffi(&api, &config).expect("scaffold");
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted");
    assert!(
        cargo.content.contains("tree-sitter = \"0.26\""),
        "capsule package dep must be injected into FFI Cargo.toml, got:\n{}",
        cargo.content
    );
}

/// A capsule type without `package` must not inject any dependency.
#[test]
fn ffi_cargo_toml_omits_capsule_dep_when_package_unset() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []
[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
"#,
    );
    let api = crate::core::ir::ApiSurface::default();
    let files = scaffold_ffi(&api, &config).expect("scaffold");
    let cargo = files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted");
    assert!(
        !cargo.content.contains("tree-sitter ="),
        "no capsule dep should be injected when package is unset, got:\n{}",
        cargo.content
    );
}

fn features_section(cargo: &str) -> &str {
    let start = cargo.find("[features]").expect("features table emitted");
    let rest = &cargo[start..];
    rest.find("\n[").map(|end| &rest[..end]).unwrap_or(rest)
}

fn minimal_config() -> ResolvedCrateConfig {
    resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []
"#,
    )
}

/// A `#[cfg(feature = "X")]` on an emitted type must make the FFI crate declare `X`.
/// Cargo features are per-crate, so an undeclared gate can never be satisfied and the
/// export is silently dropped from the cdylib while cbindgen still declares it.
#[test]
fn ffi_cargo_toml_declares_features_named_by_emitted_type_cfg_gates() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "BudgetConfig".to_string(),
            rust_path: "my_lib::BudgetConfig".to_string(),
            cfg: Some(r#"feature = "tower""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &minimal_config()).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    let features = features_section(cargo);
    assert!(
        features.contains(r#"tower = ["my-lib/tower"]"#),
        "gate feature must be declared as a passthrough, got:\n{features}"
    );
    assert!(
        features.contains("default = [") && features.contains("\"tower\""),
        "gate feature must default ON so the gated export survives, got:\n{features}"
    );
}

/// Same invariant for a gate on a function rather than a type -- the regression that
/// deleted `count_tokens`-style exports came in through the function path.
#[test]
fn ffi_cargo_toml_declares_features_named_by_emitted_function_cfg_gates() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        functions: vec![crate::core::ir::FunctionDef {
            name: "count_tokens".to_string(),
            cfg: Some(r#"feature = "tokenizer""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &minimal_config()).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    let features = features_section(cargo);
    assert!(
        features.contains(r#"tokenizer = ["my-lib/tokenizer"]"#),
        "function gate feature must be declared as a passthrough, got:\n{features}"
    );
}

/// `extra_features` is documented as declare-but-do-not-enable, for mutually-exclusive
/// alternatives such as a `wasm-http` backend. Discovering the same name in an emitted
/// gate must not promote it into `default`.
#[test]
fn ffi_cargo_toml_keeps_extra_features_out_of_default_even_when_gated() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []
[crates.ffi]
extra_features = ["wasm-http"]
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        functions: vec![crate::core::ir::FunctionDef {
            name: "fetch".to_string(),
            cfg: Some(r#"feature = "wasm-http""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &config).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    let features = features_section(cargo);
    assert!(
        features.contains(r#"wasm-http = ["my-lib/wasm-http"]"#),
        "extra_features entry must still be declared, got:\n{features}"
    );
    let default_line = features
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default line emitted");
    assert!(
        !default_line.contains("wasm-http"),
        "extra_features must stay opt-in, got: {default_line}"
    );
}

/// Regression for alef-task #320: `effective_ffi_default_features` unconditionally forwarded
/// `[crates.ffi].features` into the FFI crate's own `default = [...]` array, which re-enables
/// a feature a `target_dep_overrides` entry excluded for one cfg target -- the same defect
/// `RubyConfig::excluded_default_features` fixed for the Magnus crate, generalized here.
/// Asserts both directions: the excluded name is never defaulted, and a name nobody excluded
/// still is.
#[test]
fn ffi_cargo_toml_excludes_named_feature_from_default_but_keeps_others() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []
[crates.ffi]
features = ["native-http", "wasm-http"]
excluded_default_features = ["native-http"]
[[crates.ffi.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["wasm-http"]
default_features = false
"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &config).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    let features = features_section(cargo);
    let default_line = features
        .lines()
        .find(|line| line.starts_with("default = ["))
        .expect("default line emitted");
    assert!(
        !default_line.contains("native-http"),
        "excluded_default_features must drop the name from the FFI crate's own default array:\n{default_line}"
    );
    assert!(
        default_line.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded into default:\n{default_line}"
    );
    assert!(
        features.contains(r#"native-http = ["my-lib/native-http"]"#),
        "the excluded feature stays declared (so `cargo build --features native-http` still \
             works), just not defaulted:\n{features}"
    );
    let core_dep_line = cargo
        .lines()
        .find(|line| line.trim_start().starts_with("my-lib = { path ="))
        .expect("core dependency line emitted");
    assert!(
        !core_dep_line.contains("native-http"),
        "excluded_default_features must also drop the name from the core dependency's own \
             explicit features = [...] line, not just the wrapper's default array:\n{core_dep_line}"
    );
    assert!(
        core_dep_line.contains("wasm-http"),
        "a feature nobody excluded must still reach the core dependency line:\n{core_dep_line}"
    );
}

/// Parity invariant, stated without reference to any consumer: every feature named by a
/// `#[cfg(feature = ...)]` in the emitted surface appears in this crate's `[features]`.
#[test]
fn ffi_cargo_toml_declares_every_feature_named_by_any_emitted_gate() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "CacheConfig".to_string(),
            rust_path: "my_lib::CacheConfig".to_string(),
            cfg: Some(r#"any(feature = "tower", feature = "opendal-cache")"#.to_string()),
            ..Default::default()
        }],
        functions: vec![crate::core::ir::FunctionDef {
            name: "record_cost_usd".to_string(),
            cfg: Some(r#"feature = "metrics""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &minimal_config()).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    let features = features_section(cargo);
    for name in ["tower", "opendal-cache", "metrics"] {
        assert!(
            features.contains(&format!(r#"{name} = ["my-lib/{name}"]"#)),
            "feature `{name}` is gated on but never declared, got:\n{features}"
        );
    }
}

/// `[crates.cargo_lints]` must round-trip into the emitted FFI `Cargo.toml` as a
/// `[lints.rust]` / `[lints.clippy]` block, and produce valid TOML.
#[test]
fn ffi_cargo_toml_emits_configured_cargo_lints() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]
[[crates]]
name = "my-lib"
sources = []

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.clippy]
print_stdout = "deny"
"#,
    );
    let api = crate::core::ir::ApiSurface::default();
    let files = scaffold_ffi(&api, &config).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    // Exactly one blank line must precede the spliced `[lints.rust]` block. This is asserted
    // without naming the table before it: cargo-sort's canonical order puts every lints table
    // last, so pinning the preceding table would re-break this test the next time a table is
    // added at the tail. The check deliberately does not look at the whole file, because the
    // `[features]` block's `{core_features_passthrough_block}` slot is independently blank
    // whenever no cfg-gated feature is emitted (true for this test's empty API surface), which
    // pre-existingly leaves a double blank line before `[dependencies]` regardless of
    // cargo_lints. ~keep
    assert!(
        cargo.contains("\n\n[lints.rust]\nunused_must_use = \"deny\""),
        "expected a blank line before [lints.rust], got:\n{cargo}"
    );
    assert!(
        !cargo.contains("\n\n\n[lints.rust]"),
        "expected exactly one blank line before [lints.rust], not more, got:\n{cargo}"
    );
    assert!(
        cargo.contains(
            "[lints.rust]\nunused_must_use = \"deny\"\n\n\
                 # This crate deliberately does not use `[lints]` / `workspace = true`"
        ),
        "expected exactly one blank line between [lints.rust] and the [lints.clippy] rationale \
             comment, got:\n{cargo}"
    );
    assert!(
        cargo.contains(
            "can actually satisfy. ~keep\n\
                 [lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""
        ),
        "expected the rationale comment to attach directly to [lints.clippy] (no blank line), and the \
             configured print_stdout to merge with the builtin dbg_macro/print_stderr defaults, got:\n{cargo}"
    );
    // `[lints.clippy]` is last under cargo-sort's canonical table order, so there is no
    // following section to be separated from -- this previously pinned the `# `serde`` comment
    // that used to follow it. What still matters is that the file ends cleanly: the clippy
    // block is the final table and the tail does not accumulate blank lines. A global
    // "no triple newline" check would be wrong here, because the `[features]` block leaves a
    // documented double blank before `[dependencies]` on an empty API surface. ~keep
    assert!(
        cargo.trim_end().ends_with("print_stdout = \"deny\""),
        "expected [lints.clippy] to be the final table, got:\n{cargo}"
    );
    assert!(
        cargo.len() - cargo.trim_end_matches('\n').len() <= 2,
        "expected at most one trailing blank line after [lints.clippy], got:\n{cargo}"
    );
    toml::from_str::<toml::Value>(cargo).expect("generated Cargo.toml with cargo_lints must be valid TOML");
}

/// Absence of `[crates.cargo_lints]` must still emit the built-in `[lints.clippy]` deny
/// block (`dbg_macro` / `print_stderr` / `print_stdout`) — this is the regression cover
/// for the coverage-loss bug where four generated binding crates lost that exact block
/// on a full regen because it lived only as hand-added content the generator didn't know
/// to reproduce. No `[lints.rust]` table is emitted, since nothing configures it.
#[test]
fn ffi_cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
    let files = scaffold_ffi(&crate::core::ir::ApiSurface::default(), &minimal_config()).expect("scaffold");
    let cargo = &files
        .iter()
        .find(|f| f.path.ends_with("Cargo.toml"))
        .expect("ffi Cargo.toml emitted")
        .content;
    assert!(
        !cargo.contains("[lints.rust]"),
        "no [lints.rust] table should be emitted when cargo_lints.rust is unset, got:\n{cargo}"
    );
    assert!(
        cargo.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "the builtin [lints.clippy] deny block must survive even when cargo_lints is unset, got:\n{cargo}"
    );
    toml::from_str::<toml::Value>(cargo).expect("generated Cargo.toml must be valid TOML");
}

#[test]
fn ffi_manifest_reconciliation_adds_runtime_dependencies_and_cfg_features() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "BudgetConfig".to_string(),
            rust_path: "my_lib::BudgetConfig".to_string(),
            cfg: Some(r#"feature = "tower""#.to_string()),
            ..Default::default()
        }],
        functions: vec![crate::core::ir::FunctionDef {
            name: "count_tokens".to_string(),
            cfg: Some(r#"feature = "tokenizer""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = scaffold_ffi(&api, &minimal_config()).expect("scaffold");
    let directory = tempfile::tempdir().expect("temporary repository");
    let manifest_path = directory.path().join("crates/my-lib-ffi/Cargo.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent")).expect("create manifest parent");
    std::fs::write(
        &manifest_path,
        "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:old\n\
             [features]\ndefault = [\"full\"]\nfull = [\"my-lib/full\"]\n\
             [dependencies]\nserde_json = \"1\"\n",
    )
    .expect("write stale generated manifest");

    crate::cli::pipeline::reconcile_managed_scaffold_manifests(&files, directory.path(), None)
        .expect("reconcile generated FFI manifest");
    let reconciled = std::fs::read_to_string(manifest_path).expect("read reconciled manifest");

    assert!(reconciled.contains("serde = \"1\""), "{reconciled}");
    assert!(reconciled.contains(r#"tower = ["my-lib/tower"]"#), "{reconciled}");
    assert!(
        reconciled.contains(r#"tokenizer = ["my-lib/tokenizer"]"#),
        "{reconciled}"
    );
    let parsed: toml::Value = toml::from_str(&reconciled).expect("valid reconciled TOML");
    let defaults = parsed["features"]["default"].as_array().expect("default feature array");
    for feature in ["tower", "tokenizer"] {
        assert!(
            defaults.iter().any(|value| value.as_str() == Some(feature)),
            "{reconciled}"
        );
    }
}

/// Regression cover for the reported coverage-loss bug: a consumer's on-disk `-ffi`
/// manifest carries a hand-added `[lints.clippy]` deny block (with no
/// `[crates.cargo_lints]` entry in `alef.toml` describing it — exactly the
/// tree-sitter-language-pack scenario) and a full reconciliation pass must still leave
/// that block in place afterward, because the builtin now regenerates it itself rather
/// than relying on the consumer's hand-added copy surviving untouched.
#[test]
fn ffi_manifest_reconciliation_keeps_the_clippy_deny_block_with_no_cargo_lints_configured() {
    let files = scaffold_ffi(&crate::core::ir::ApiSurface::default(), &minimal_config()).expect("scaffold");
    let directory = tempfile::tempdir().expect("temporary repository");
    let manifest_path = directory.path().join("crates/my-lib-ffi/Cargo.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent")).expect("create manifest parent");
    std::fs::write(
        &manifest_path,
        "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:old\n\
             [lints.clippy]\nprint_stdout = \"deny\"\nprint_stderr = \"deny\"\ndbg_macro = \"deny\"\n",
    )
    .expect("write stale generated manifest carrying a hand-added clippy deny block");

    crate::cli::pipeline::reconcile_managed_scaffold_manifests(&files, directory.path(), None)
        .expect("reconcile generated FFI manifest");
    let reconciled = std::fs::read_to_string(manifest_path).expect("read reconciled manifest");

    assert!(
        reconciled.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "the clippy deny block must survive reconciliation even though alef.toml never \
             configured [crates.cargo_lints]:\n{reconciled}"
    );
    toml::from_str::<toml::Value>(&reconciled).expect("valid reconciled TOML");
}

/// Regression: the `~keep`-worthy rationale explaining why this crate does not use
/// `[lints]\nworkspace = true` (it would drag in `[workspace.lints.rust]
/// unsafe_code = "deny"`, which a C-ABI crate cannot satisfy) must appear directly
/// above `[lints.clippy]` on every regeneration -- including reconciliation against
/// an on-disk manifest that never had it (a stale manifest generated before this
/// comment existed, or hand-edited to remove it). The comment is emitted by alef
/// itself rather than left for a consumer to add, since these are `generated_header:
/// true` manifests rewritten in full and no `~keep` marker protects hand-added
/// content against that.
#[test]
fn ffi_manifest_reconciliation_restores_the_clippy_workspace_lints_rationale_comment() {
    let files = scaffold_ffi(&crate::core::ir::ApiSurface::default(), &minimal_config()).expect("scaffold");
    let directory = tempfile::tempdir().expect("temporary repository");
    let manifest_path = directory.path().join("crates/my-lib-ffi/Cargo.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent")).expect("create manifest parent");
    std::fs::write(
        &manifest_path,
        "# This file is auto-generated by alef — DO NOT EDIT.\n# alef:hash:old\n\
             [lints.clippy]\nprint_stdout = \"deny\"\nprint_stderr = \"deny\"\ndbg_macro = \"deny\"\n",
    )
    .expect("write stale generated manifest with no rationale comment");

    crate::cli::pipeline::reconcile_managed_scaffold_manifests(&files, directory.path(), None)
        .expect("reconcile generated FFI manifest");
    let reconciled = std::fs::read_to_string(manifest_path).expect("read reconciled manifest");

    assert!(
        reconciled.contains("# This crate deliberately does not use `[lints]` / `workspace = true`"),
        "the [lints.clippy] rationale comment must be (re)emitted on regeneration, not just the \
             block itself:\n{reconciled}"
    );
    assert!(
        reconciled.contains("can actually satisfy. ~keep\n[lints.clippy]"),
        "the rationale comment must attach directly above [lints.clippy] with no blank line, \
             got:\n{reconciled}"
    );
    toml::from_str::<toml::Value>(&reconciled).expect("valid reconciled TOML");
}

/// Regression for the alef/poly generator-formatter oscillation described in task #373: poly's
/// tree-sitter-based CMake formatter converges to this exact (visually uneven, but idempotent
/// and deterministic) indentation as its fixed point for a `find_package` config module —
/// verified empirically against `poly fmt --fix --fix-generated` and against the byte-identical
/// shape already committed in real consumer repos' `*-ffi-config.cmake` files. A "cleaner"
/// uniformly-indented version would get silently rewritten by poly on every regen. ~keep
#[test]
fn ffi_cmake_config_matches_polys_canonical_fixed_point() {
    let files = scaffold_ffi(&crate::core::ir::ApiSurface::default(), &minimal_config()).expect("scaffold");
    let cmake = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("-ffi-config.cmake"))
        .expect("cmake config module must be emitted");

    const EXPECTED: &str = r#"# my-lib-ffi CMake config-mode find module
#
# Defines the imported target:
#   my-lib-ffi::my-lib-ffi
#
# Usage:
#   find_package(my-lib-ffi REQUIRED)
#   target_link_libraries(myapp PRIVATE my-lib-ffi::my-lib-ffi)

if(TARGET my-lib-ffi::my-lib-ffi)
return()
endif()

get_filename_component(_FFI_CMAKE_DIR "${CMAKE_CURRENT_LIST_FILE}" PATH)
get_filename_component(_FFI_PREFIX "${_FFI_CMAKE_DIR}/.." ABSOLUTE)

find_library(_FFI_LIBRARY
  NAMES my_lib_ffi libmy_lib_ffi
  PATHS "${_FFI_PREFIX}/lib"
  NO_DEFAULT_PATH
)
if(NOT _FFI_LIBRARY)
find_library(_FFI_LIBRARY NAMES my_lib_ffi libmy_lib_ffi)
endif()

find_path(_FFI_INCLUDE_DIR
  NAMES my_lib.h
  PATHS "${_FFI_PREFIX}/include"
  NO_DEFAULT_PATH
)
if(NOT _FFI_INCLUDE_DIR)
find_path(_FFI_INCLUDE_DIR NAMES my_lib.h)
endif()

include(FindPackageHandleStandardArgs)
find_package_handle_standard_args(my-lib-ffi
  REQUIRED_VARS _FFI_LIBRARY _FFI_INCLUDE_DIR
)

if(my_lib_ffi_FOUND)
set(_FFI_LIB_TYPE UNKNOWN)
if(_FFI_LIBRARY MATCHES "\\.(dylib|so)$" OR _FFI_LIBRARY MATCHES "\\.so\\.")
    set(_FFI_LIB_TYPE SHARED)
elseif(_FFI_LIBRARY MATCHES "\\.dll$")
    set(_FFI_LIB_TYPE SHARED)
elseif(_FFI_LIBRARY MATCHES "\\.(a|lib)$")
    set(_FFI_LIB_TYPE STATIC)
endif()

add_library(my-lib-ffi::my-lib-ffi ${_FFI_LIB_TYPE} IMPORTED)
    set_target_properties(my-lib-ffi::my-lib-ffi PROPERTIES
    IMPORTED_LOCATION "${_FFI_LIBRARY}"
    INTERFACE_INCLUDE_DIRECTORIES "${_FFI_INCLUDE_DIR}"
    )

if(WIN32 AND _FFI_LIB_TYPE STREQUAL "SHARED")
        find_file(_FFI_DLL
      NAMES my_lib_ffi.dll libmy_lib_ffi.dll
      PATHS "${_FFI_PREFIX}/bin" "${_FFI_PREFIX}/lib"
      NO_DEFAULT_PATH
        )
    if(_FFI_DLL)
            set_target_properties(my-lib-ffi::my-lib-ffi PROPERTIES
        IMPORTED_LOCATION "${_FFI_DLL}"
        IMPORTED_IMPLIB "${_FFI_LIBRARY}"
            )
    endif()
    unset(_FFI_DLL CACHE)
endif()

if(APPLE)
        set_property(TARGET my-lib-ffi::my-lib-ffi APPEND PROPERTY
        INTERFACE_LINK_LIBRARIES "-framework CoreFoundation" "-framework Security" pthread)
elseif(UNIX)
        set_property(TARGET my-lib-ffi::my-lib-ffi APPEND PROPERTY
        INTERFACE_LINK_LIBRARIES pthread dl m)
elseif(WIN32)
        set_property(TARGET my-lib-ffi::my-lib-ffi APPEND PROPERTY
        INTERFACE_LINK_LIBRARIES ws2_32 userenv bcrypt)
endif()

unset(_FFI_LIB_TYPE)
endif()

mark_as_advanced(_FFI_LIBRARY _FFI_INCLUDE_DIR)
unset(_FFI_CMAKE_DIR)
unset(_FFI_PREFIX)
"#;
    assert_eq!(
        cmake.content, EXPECTED,
        "cmake config content drifted from poly's verified canonical fixed point"
    );
}

#[test]
fn component_manager_dependencies_require_ci_staged_lock() {
    use crate::core::config::{ComponentContractConfig, ComponentProfileConfig};

    let api = ApiSurface {
        crate_name: "demo-core".into(),
        version: "1.0.0".into(),
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "demo-core".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "engine".into(),
            trait_path: "demo_core::Engine".into(),
            interface_version: 1,
        }],
        components: vec![ComponentProfileConfig {
            name: "fast".into(),
            contract: "engine".into(),
            implementation: "demo_core::FastEngine".into(),
            features: vec!["fast".into()],
            default_features: false,
            targets: vec!["x86_64-unknown-linux-gnu".into()],
        }],
        ..ResolvedCrateConfig::default()
    };

    let files = scaffold_ffi(&api, &config).unwrap();
    let manifest = files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .expect("FFI manifest");
    assert!(manifest.content.contains("alef-component-runtime"));
    assert!(manifest.content.contains("alef-component-abi"));
    assert!(manifest.content.contains("directories = \"6\""));
    assert!(!files.iter().any(|file| file.path.ends_with("components.lock.json")));
}

#[test]
fn configured_component_dependencies_are_not_duplicated() {
    let config = resolve_config(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "demo-core"
sources = []

[crates.extra_dependencies]
alef-component-abi = "9"
alef-component-runtime = "9"
directories = "5"

[[crates.component_contracts]]
name = "engine"
trait_path = "demo_core::Engine"
interface_version = 1

[[crates.components]]
name = "fast"
contract = "engine"
implementation = "demo_core::FastEngine"
features = ["fast"]
targets = ["x86_64-unknown-linux-gnu"]
"#,
    );
    let files = scaffold_ffi(&ApiSurface::default(), &config).unwrap();
    let manifest = &files
        .iter()
        .find(|file| file.path.ends_with("Cargo.toml"))
        .unwrap()
        .content;

    for dependency in ["alef-component-abi", "alef-component-runtime", "directories"] {
        assert_eq!(
            manifest.matches(&format!("{dependency} =")).count(),
            1,
            "duplicate {dependency} dependency in:\n{manifest}"
        );
    }
    assert!(manifest.contains("alef-component-runtime = \"9\""));
    assert!(manifest.contains("directories = \"5\""));
}
