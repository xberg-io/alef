use super::*;
use crate::core::config::NewAlefConfig;
use crate::core::ir::ApiSurface;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A tree with no reachable `[workspace.package]` must get explicit `[package]` values.
///
/// The JNI manifest used to hard-code `version.workspace = true` / `edition.workspace = true`
/// / `license.workspace = true`. In a root-flat emitted tree there is no workspace root to
/// inherit from, so cargo refused the manifest outright ("error inheriting `edition` from
/// workspace root manifest") and every downstream command over the JNI crate -- clippy,
/// build, test -- failed before compiling anything. Every sibling scaffolder already routed
/// this through `detect_workspace_inheritance_for_crate`; only this one asserted. ~keep
#[test]
fn scaffold_jni_cargo_toml_writes_explicit_package_fields_without_a_workspace() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );

    let cargo_toml = &scaffold_jni(&ApiSurface::default(), &config).unwrap()[0].content;

    assert!(
        cargo_toml.contains("edition = \"2024\""),
        "without a workspace root the edition must be written explicitly; got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains(".workspace = true"),
        "no [package] field may claim workspace inheritance when no workspace is reachable; got:\n{cargo_toml}"
    );
    // The same tree shape breaks the core dependency for the same reason: `../{core}` names
    // a `crates/<core>` sibling that a root-flat tree does not have, so cargo could not read
    // the manifest it points at. ~keep
    let expected = config.core_crate_dep_path(std::path::Path::new("crates/demo-llm-jni"));
    assert_eq!(expected, "../..", "sanity: root-flat core crate sits two levels up");
    assert!(
        cargo_toml.contains(&format!(r#"path = "{expected}""#)),
        "the core dep path must be derived from the emitted layout; got:\n{cargo_toml}"
    );
}

/// The JNI trait-bridge glue (`trait_bridge_method_body.rs.jinja`) logs swallowed
/// host-callback failures via `tracing::warn!`. Since this Cargo.toml is scaffolded
/// once (`generated_header: false`) and not regenerated once trait bridges are
/// configured later, `tracing` must be declared unconditionally up front.
#[test]
fn scaffold_jni_cargo_toml_declares_tracing_dependency() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains(&format!(
            "tracing = \"{}\"",
            crate::core::template_versions::cargo::TRACING
        )),
        "JNI Cargo.toml must declare the tracing dependency; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("\"tracing\""),
        "JNI Cargo.toml must ignore tracing in cargo-machete since it may be unused until trait bridges are configured; got:\n{cargo_toml}"
    );
}

/// `[crates.cargo_lints]` must round-trip into the emitted JNI `Cargo.toml` as a
/// `[lints.rust]` / `[lints.clippy]` block, with a configured `clippy` entry for a
/// non-builtin key surviving alongside the builtin deny defaults. The JNI template
/// builds the rest of its manifest from a hand-written literal, so this exercises the
/// splice against a differently shaped template than the other binding crates.
#[test]
fn scaffold_jni_cargo_toml_emits_configured_cargo_lints() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.clippy]
unwrap_used = "warn"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains("[lints.rust]\nunused_must_use = \"deny\""),
        "expected [lints.rust] block, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(
            "[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\n\
             print_stdout = \"deny\"\nunwrap_used = \"warn\""
        ),
        "expected the configured clippy entry to merge with the builtin deny defaults, got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated Cargo.toml with cargo_lints must be valid TOML");
}

/// Absence of `[crates.cargo_lints]` must still emit the built-in `[lints.clippy]` deny
/// block; no `[lints.rust]` table is emitted since nothing configures it.
#[test]
fn scaffold_jni_cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        !cargo_toml.contains("[lints.rust]"),
        "no [lints.rust] table should be emitted when cargo_lints.rust is unset, got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "the builtin [lints.clippy] deny block must survive even when cargo_lints is unset, got:\n{cargo_toml}"
    );
}

#[test]
fn scaffold_jni_inherits_declared_workspace_dependencies() {
    let workspace = tempfile::tempdir().expect("create workspace");
    std::fs::write(
        workspace.path().join("Cargo.toml"),
        r#"
[workspace]
members = []

[workspace.dependencies]
base64 = "0.22"
jni = "0.22"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
"#,
    )
    .expect("write workspace manifest");
    let mut config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );
    config.workspace_root = Some(workspace.path().to_path_buf());

    let files = scaffold_jni(&ApiSurface::default(), &config).expect("scaffold JNI");
    let cargo_toml = &files[0].content;

    assert!(cargo_toml.contains("base64.workspace = true"));
    assert!(cargo_toml.contains("jni.workspace = true"));
    assert!(cargo_toml.contains("tokio.workspace = true"));
    assert!(cargo_toml.contains("async-trait = \"0.1\""));
    assert!(!cargo_toml.contains("base64 = \"0.22\""));
}

/// The scaffolded `[lib] name` must match what the Kotlin Bridge emits in
/// `System.loadLibrary(...)`.  When `[ffi] prefix` is set, both must use the
/// prefix-derived name rather than the snake-cased package name.
#[test]
fn scaffold_jni_lib_name_uses_ffi_prefix() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demoffi"

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains("name = \"demoffi_jni\""),
        "expected `name = \"demoffi_jni\"` but got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("name = \"demo_llm_jni\""),
        "cdylib name must not fall back to snake-cased crate name when prefix is set; got:\n{cargo_toml}"
    );
}

/// When `config.name` differs from `core_crate_dir()` (e.g. parser-pack's
/// `name = "parser-language-pack"` with sources under
/// `crates/parser-core-core/`), the JNI scaffold must place its output at
/// `crates/<config.name>-jni/Cargo.toml` to match the path that
/// `alef-backend-jni::gen_shims` uses for `src/lib.rs`, and the umbrella
/// dep entry must use the cargo package name as the dep key while the
/// `path = "../..."` value references the on-disk directory.
#[test]
fn scaffold_jni_path_uses_config_name_not_core_crate_dir() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "sample-language-pack"
sources = ["crates/sample-pack-core/src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.sample_language_pack.android"
namespace = "dev.sample_crate.sample_language_pack.android"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    let path = files[0].path.to_string_lossy();
    let cargo_toml = &files[0].content;

    assert!(files[0].generated_header, "generated JNI manifest must be Alef-owned");
    assert_eq!(
        path, "crates/sample-language-pack-jni/Cargo.toml",
        "JNI scaffold path must follow config.name, not core_crate_dir; got: {path}"
    );
    assert!(
        cargo_toml.contains("name = \"sample-language-pack-jni\""),
        "[package] name must follow config.name; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("sample-language-pack = { path = \"../sample-pack-core\""),
        "umbrella dep key must be cargo package name with path = ../<core_crate_dir>; got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("sample-pack-core = { path = \"../sample-pack-core\""),
        "umbrella dep key must NOT be the directory name; got:\n{cargo_toml}"
    );
}

/// Without an explicit `[ffi] prefix`, the lib name must still be the
/// snake-cased crate name (regression guard for the default case).
#[test]
fn scaffold_jni_lib_name_defaults_to_snake_case_crate_name() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "plain-pkg"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.plain"
namespace = "dev.sample_crate.plain"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains("name = \"plain_pkg_jni\""),
        "expected `name = \"plain_pkg_jni\"` for default case; got:\n{cargo_toml}"
    );
}

/// When `[crates.jni] crate_dir` is set, the JNI scaffold uses the
/// override for both the crate directory and `[package] name`, while the
/// umbrella dep key remains `config.name` (the Cargo package name) with
/// `path = "../<core_crate_dir>"`.
///
/// This covers a suffixed package name with a suffix-free core crate directory:
/// the JNI crate lands at the configured `crate_dir` path rather than keeping
/// the package suffix in its own crate name.
#[test]
fn scaffold_jni_crate_dir_override_controls_output_path() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-render-rs"
sources = ["crates/demo-render/src/lib.rs"]

[crates.jni]
crate_dir = "demo-render"

[crates.kotlin_android]
package = "dev.example.demo_render.android"
namespace = "dev.example.demo_render.android"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    assert_eq!(files.len(), 1);
    let path = files[0].path.to_string_lossy();
    let cargo_toml = &files[0].content;

    assert_eq!(
        path, "crates/demo-render-jni/Cargo.toml",
        "JNI scaffold path must follow [crates.jni] crate_dir override; got: {path}"
    );
    assert!(
        cargo_toml.contains("name = \"demo-render-jni\""),
        "[package] name must follow crate_dir override; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("demo-render-rs = { path = \"../demo-render\""),
        "umbrella dep key must be cargo package name, path must be core_crate_dir; got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("demo-render = { path = \"../demo-render\""),
        "umbrella dep key must NOT be the crate_dir override; got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("demo-render-rs-jni"),
        "crate name must NOT contain the -rs suffix; got:\n{cargo_toml}"
    );
}

/// Regression guard: the JNI `Cargo.toml` `[dependencies]` table must be
/// emitted in the order `cargo sort --check` requires, so the `cargo-sort`
/// prek hook does not rewrite the file on every regen. The umbrella dep is
/// named after `config.name`, so its placement depends on the consumer crate
/// name.
///
/// The check goes through `assert_dependency_keys_sorted`, which re-parses the
/// manifest with `toml_edit` and so runs cargo-sort's own comparison. An
/// earlier version of this test derived the key itself with
/// `line.split('=').next()`, which yields the DOTTED key `tracing.workspace` --
/// the same key the emitter was sorting on -- making the assertion a tautology
/// that kept passing while consumers failed. ~keep
#[test]
fn scaffold_jni_dependency_keys_are_in_cargo_sort_order() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "sample_stream"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.sample_stream"
namespace = "dev.example.sample_stream"
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        crate::test_support::cargo_sort_order::assert_dependency_keys_sorted("jni Cargo.toml", cargo_toml) > 0,
        "the JNI manifest must carry dependency keys to compare:\n{cargo_toml}"
    );
}

/// When `[crates.jni] target_dep_overrides` are configured, the core-crate
/// dependency must move out of the inline `[dependencies]` table into per-cfg
/// `[target.'cfg(...)'.dependencies]` blocks: a `cfg(not(any(...)))` default
/// branch carrying the full feature set, plus one block per override carrying
/// its replacement features. Mirrors the FFI gating so the cross-compiled JNI
/// shim drops ORT / native-C features on Android, iOS, and Windows.
#[test]
fn scaffold_jni_emits_target_dep_overrides() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "ios"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    let deps_start = cargo_toml.find("[dependencies]").expect("missing [dependencies]");
    let first_target = cargo_toml
        .find("[target.")
        .expect("expected at least one [target.*] block");
    let inline_deps = &cargo_toml[deps_start..first_target];
    assert!(
        !inline_deps.contains("demo-doc ="),
        "core-crate dep must not appear inline when overrides are present; got:\n{cargo_toml}"
    );

    // Default branch is gated on cfg(not(any(...))) and carries `full`.
    assert!(
        cargo_toml.contains(
            r#"[target.'cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))'.dependencies]"#
        ),
        "default branch must be gated on cfg(not(any(...))); got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"[target.'cfg(target_os = "android")'.dependencies]"#)
            && cargo_toml.contains(r#"features = ["android-target"]"#),
        "android override block must carry android-target; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains(r#"[target.'cfg(target_os = "windows")'.dependencies]"#)
            && cargo_toml.contains(r#"features = ["windows-target"]"#),
        "windows override block must carry windows-target; got:\n{cargo_toml}"
    );
    let core_dep_lines = cargo_toml.matches("demo-doc = {").count();
    assert_eq!(
        core_dep_lines, 4,
        "expected one core-dep line per target branch (default + 3 overrides); got {core_dep_lines}:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
}

/// Regression test for the dropped `default_features` config key on
/// `[crates.jni.target_dep_overrides]` (see `FfiTargetDepOverride::default_features`):
/// `render_jni_target_blocks` iterates the per-target overrides but, unlike the FFI
/// scaffolder's own `render_core_dep` and `scaffold::render_core_dep_with_overrides`, never
/// read `override.default_features` -- a consumer's per-target `default_features = false`
/// vanished from the generated JNI Cargo.toml. An override with `default_features = false`
/// must emit `default-features = false` in its target block.
#[test]
fn scaffold_jni_target_dep_override_disables_default_features() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]
default_features = false
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains(r#"[target.'cfg(target_os = "windows")'.dependencies]"#),
        "expected a windows override block; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("default-features = false"),
        "override.default_features = false must reach the rendered JNI manifest as \
         `default-features = false`; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
}

/// The default-branch and `default_features = true` override paths must NOT emit
/// `default-features = false` -- only an explicit `default_features = false` override does.
/// Paired with the test above so neither direction of this boolean is vacuously true.
#[test]
fn scaffold_jni_target_dep_override_keeps_default_features_when_true() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]
default_features = true
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        !cargo_toml.contains("default-features = false"),
        "override.default_features = true must not emit `default-features = false`; got:\n{cargo_toml}"
    );
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders
/// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
/// predicate string (plain byte-wise comparison), NOT with the default
/// `cfg(not(any(...)))` branch always first. An `all(...)` override (e.g.
/// the macOS-Intel target a downstream consumer actually configures) sorts
/// *before* `not(...)` — `'a'` < `'n'` — so emitting the default branch
/// unconditionally first produces an unsorted manifest that
/// `cargo sort --check` rejects. This reproduces a downstream consumer's
/// real `crates/acme-jni/Cargo.toml` override set (android/ios/windows plus a
/// macOS-Intel `all(...)` override) and asserts the `all(...)` block comes
/// before the `not(...)` default block in the emitted text.
#[test]
fn scaffold_jni_target_dep_overrides_sort_all_before_not() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "android"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "ios"'
features = ["android-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'target_os = "windows"'
features = ["windows-target"]

[[crates.jni.target_dep_overrides]]
cfg = 'all(target_os = "macos", target_arch = "x86_64")'
features = ["macos-intel-target"]
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    let all_pos = cargo_toml
        .find(r#"[target.'cfg(all(target_os = "macos", target_arch = "x86_64"))'.dependencies]"#)
        .expect("expected the macOS-Intel `all(...)` override block");
    let not_pos = cargo_toml
        .find("[target.'cfg(not(any(")
        .expect("expected the default `not(any(...))` block");
    let android_pos = cargo_toml
        .find(r#"[target.'cfg(target_os = "android")'.dependencies]"#)
        .expect("expected the android override block");

    assert!(
        all_pos < not_pos,
        "the `all(...)` override must sort BEFORE the `not(...)` default branch \
         (cargo-sort compares raw cfg predicate strings byte-wise: 'a' < 'n'); got:\n{cargo_toml}"
    );
    assert!(
        not_pos < android_pos,
        "the `not(...)` default branch must sort before `target_os = \"android\"` \
         ('n' < 't'); got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
}

/// Without `target_dep_overrides`, the core-crate dep stays inline in
/// `[dependencies]` and no `[target.*]` blocks are emitted (regression guard
/// for the default path).
#[test]
fn scaffold_jni_no_target_blocks_without_overrides() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-doc"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"
features = ["full"]
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;
    assert!(
        !cargo_toml.contains("[target."),
        "no [target.*] blocks without overrides; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("demo-doc = {") && cargo_toml.contains(r#"features = ["full"]"#),
        "core-crate dep must stay inline with full features; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
}

/// Regression guard: cargo-sort orders sub-tables of `[package]`
/// (`[package.metadata.cargo-machete]`) directly after the `[package]`
/// section and before `[lib]` / `[dependencies]`. Emitting `[lib]` or
/// `[dependencies]` before the metadata sub-table causes cargo-sort to
/// rewrite the file on every regen.
#[test]
fn scaffold_jni_section_order_matches_cargo_sort() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );
    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    let pkg = cargo_toml.find("[package]").expect("missing [package]");
    let meta = cargo_toml
        .find("[package.metadata.cargo-machete]")
        .expect("missing [package.metadata.cargo-machete]");
    let lib = cargo_toml.find("[lib]").expect("missing [lib]");
    let deps = cargo_toml.find("[dependencies]").expect("missing [dependencies]");
    assert!(
        pkg < meta && meta < lib && lib < deps,
        "section order must be [package] < [package.metadata.cargo-machete] < [lib] < [dependencies]; got:\n{cargo_toml}"
    );
}

/// The generated JNI shim calls whatever the core crate's configured feature set exposes,
/// so its manifest must request that same set. Reading `[crates.kotlin_android] features`
/// alone left the dependency on the core crate's defaults whenever the paired section
/// omitted the key, and the shim then failed to compile against feature-gated modules.
#[test]
fn scaffold_jni_core_dep_inherits_top_level_features_when_kotlin_android_omits_them() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]
features = ["native-http", "full"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains(r#"features = ["native-http", "full"]"#),
        "core dep must inherit the top-level feature set; got:\n{cargo_toml}"
    );
}

/// An explicit `[crates.kotlin_android] features` still wins over the top-level set, so
/// the inheritance above is a fallback rather than an override.
#[test]
fn scaffold_jni_core_dep_prefers_explicit_kotlin_android_features() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-llm"
sources = ["src/lib.rs"]
features = ["native-http", "full"]

[crates.kotlin_android]
package = "dev.sample_crate.demo"
namespace = "dev.sample_crate.demo"
features = ["android-http"]
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        cargo_toml.contains(r#"features = ["android-http"]"#),
        "explicit kotlin_android features must win; got:\n{cargo_toml}"
    );
    assert!(
        !cargo_toml.contains("native-http"),
        "top-level features must not leak in when overridden; got:\n{cargo_toml}"
    );
}

/// Regression test for alef task #145. A capsule type configured in BOTH
/// `[crates.ffi.capsule_types]` and `[crates.kotlin_android.capsule_types]` used to make
/// `scaffold_jni` declare the capsule's backing crate (e.g. `tree-sitter`) as a direct
/// `[dependencies]` entry, on the premise that the JNI shim emitted an explicit
/// `as *const {into_raw_type}` cast referencing it. That cast was later dropped as a
/// same-type cast tripping `clippy::unnecessary_cast` (see
/// `capsule_returns_transfer_the_pointer_without_a_redundant_cast` in `gen_shims::tests`),
/// so the JNI shim now transfers a capsule return purely by `.into_raw()` type inference
/// and never spells the crate's path in generated source. Declaring the dependency anyway
/// left it genuinely unused; `cargo machete` correctly flagged it and stripped it from a
/// real consumer's manifest during `poly lint --fix`, which then fought the next
/// `alef generate`. The manifest must never declare a capsule package dependency.
#[test]
fn scaffold_jni_never_declares_a_capsule_package_dependency() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo-lang-pack"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.demo"
namespace = "dev.example.demo"

[crates.kotlin_android.capsule_types.Language]
host_type = "org.example.TSLanguage"

[crates.ffi.capsule_types.Language]
into_raw_type = "tree_sitter::ffi::TSLanguage"
c_return_type = "TSLanguage"
package = "tree-sitter"
package_version = "0.26"
"#,
    );

    let api = ApiSurface::default();
    let files = scaffold_jni(&api, &config).unwrap();
    let cargo_toml = &files[0].content;

    assert!(
        !cargo_toml.contains("tree-sitter"),
        "the JNI manifest must never declare a capsule package as a dependency: the JNI \
         shim transfers capsule returns via `.into_raw()` type inference alone and never \
         names the crate; got:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(cargo_toml).expect("generated JNI Cargo.toml must be valid TOML");
}

/// Regression: the JNI shim names each core error type in a `jni_error_parts_*` function. An
/// error that lives in a source crate other than the umbrella crate (`demo_graphql::GraphQLError`)
/// needs that crate as a direct dependency, or the shim fails with E0433. Only the crates the
/// shim names are declared, so an unreferenced extra dependency is not flagged as unused.
#[test]
fn scaffold_jni_declares_the_source_crates_of_error_types() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.extra_dependencies]
demo-graphql = { path = "../demo-graphql" }
demo-http = { path = "../demo-http" }

[crates.kotlin_android]
package = "dev.demo"
namespace = "dev.demo"
"#,
    );
    let error = |name: &str, rust_path: &str| crate::core::ir::ErrorDef {
        exception_base: None,
        name: name.into(),
        rust_path: rust_path.into(),
        original_rust_path: String::new(),
        variants: Vec::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let api = ApiSurface {
        errors: vec![
            error("AppError", "demo::AppError"),
            error("GraphQLError", "demo_graphql::GraphQLError"),
        ],
        ..Default::default()
    };

    let cargo_toml = &scaffold_jni(&api, &config).unwrap()[0].content;
    let manifest: toml::Table = toml::from_str(cargo_toml).unwrap();
    let dependencies = manifest["dependencies"].as_table().unwrap();

    assert_eq!(
        dependencies.get("demo-graphql").and_then(|dep| dep.get("path")),
        Some(&toml::Value::String("../demo-graphql".into())),
        "the error type's source crate must be a dependency; got:\n{cargo_toml}"
    );
    assert!(
        !dependencies.contains_key("demo-http"),
        "an extra dependency the shim never names must not be declared; got:\n{cargo_toml}"
    );
    assert!(dependencies.contains_key("demo"), "{cargo_toml}");
}
