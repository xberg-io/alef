use crate::core::backend::GeneratedFile;
use crate::core::config::{FfiTargetDepOverride, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

/// Render the `[target.'cfg(...)'.dependencies]` blocks for the JNI crate's
/// core-crate dependency when per-target overrides are configured.
///
/// Returns an empty string when `overrides` is empty (the core dep then lives
/// inline in `[dependencies]`). Otherwise emits a `cfg(not(any(...)))` default
/// branch carrying the full `features` set, plus one `cfg(<override.cfg>)`
/// block per override carrying that override's replacement feature set. This
/// mirrors the FFI scaffolder so the same Android/iOS/Windows gating applies to
/// the cross-compiled JNI shim. The dual-form `render_core_dep` keeps the
/// `version = "..."` requirement so the manifest still publishes cleanly.
fn render_jni_target_blocks(
    crate_name: &str,
    rel_path: &str,
    default_features: &str,
    version: &str,
    overrides: &[FfiTargetDepOverride],
) -> String {
    if overrides.is_empty() {
        return String::new();
    }

    let cfgs: Vec<&str> = overrides.iter().map(|o| o.cfg.as_str()).collect();
    let combined_cfg = if cfgs.len() == 1 {
        cfgs[0].to_owned()
    } else {
        format!("any({})", cfgs.join(", "))
    };

    let mut blocks = String::new();
    blocks.push_str(&format!(
        "\n[target.'cfg(not({combined_cfg}))'.dependencies]\n{}\n",
        crate::scaffold::render_core_dep(crate_name, rel_path, default_features, version)
    ));
    for override_ in overrides {
        let features_str = if override_.features.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = override_.features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(", features = [{}]", quoted.join(", "))
        };
        blocks.push_str(&format!(
            "\n[target.'cfg({})'.dependencies]\n{}\n",
            override_.cfg,
            crate::scaffold::render_core_dep(crate_name, rel_path, &features_str, version)
        ));
    }
    blocks
}

/// Scaffold the `<crate>-jni/Cargo.toml` for a JNI shim crate.
///
/// Emits a single `Cargo.toml` as a `cdylib` depending on `jni`, `tokio`,
/// `serde_json`, and `futures-util`.  The `<crate>` dependency path is
/// `../<core-crate-dir>` inside the same workspace; features come from
/// `[crates.kotlin_android] features` if present.
///
/// The output directory is `crates/<jni_crate_base>-jni/`, where
/// `jni_crate_base` is `[crates.jni] crate_dir` when explicitly set,
/// otherwise `config.name`.  This matches the path chosen by
/// `alef-backend-jni::gen_shims::jni_output_path` for `src/lib.rs`.
///
/// Consumers whose `config.name` carries a language suffix can set
/// `[crates.jni] crate_dir` to produce a suffix-free JNI crate — matching every other binding
/// crate — while the umbrella dep entry still uses `config.name` as the Cargo
/// package key with `path = "../<core_crate_dir>"` for the on-disk location.
///
/// When `core_crate_dir` (derived from `sources`) differs from `config.name`
/// — e.g. parser-pack's `name = "parser-language-pack"` with
/// `sources = ["crates/parser-core-core/src/lib.rs"]` — the path dependency on
/// the umbrella crate uses `core_crate_dir` (the directory) while the JNI
/// crate's own directory follows `jni_crate_base`.
pub(crate) fn scaffold_jni(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let core_crate_dir = config.core_crate_dir();
    let jni_crate_name = format!("{}-jni", config.jni_crate_base());
    let jni_lib_name = config.jni_lib_name(); // matches Kotlin Bridge loadLibrary

    // Prefer kotlin_android features, then empty.
    let features: Vec<String> = config
        .kotlin_android
        .as_ref()
        .and_then(|k| k.features.as_ref())
        .map(|f| f.iter().map(|s| format!("\"{s}\"")).collect())
        .unwrap_or_default();

    let features_str = if features.is_empty() {
        String::new()
    } else {
        format!(", features = [{}]", features.join(", "))
    };

    // The umbrella crate's cargo package name (`config.name`) and its
    // on-disk directory (`core_crate_dir`) can differ — e.g. parser-pack uses
    // `name = "parser-language-pack"` for sources under
    // `crates/parser-core-core/`. Cargo dep-table keys must use the package
    // name; the `path = ...` value must use the directory name.
    let umbrella_dep_name = &config.name;

    // Per-target dependency overrides: when configured (e.g. Android/iOS drop
    // ORT + native-C features, Windows uses `windows-target`), the core-crate
    // dependency moves out of the main `[dependencies]` table into per-cfg
    // `[target.'cfg(...)'.dependencies]` blocks. The default branch is wrapped
    // in `cfg(not(any(...)))` so exactly one variant matches on any build.
    // Mirrors the FFI scaffolder's `render_core_dep` (see scaffold/languages/ffi.rs).
    let target_overrides = config
        .jni
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let rel_path = format!("../{core_crate_dir}");

    // Collect `[dependencies]` entries then sort alphabetically so the emitted
    // Cargo.toml is cargo-sort canonical without a post-processing step. The
    // umbrella dep is named after `config.name` which is consumer-dependent,
    // so the sort must run at codegen time rather than baking a static order.
    let mut dep_lines: Vec<String> = vec![
        "base64 = \"0.22\"".to_owned(),
        "futures-util = \"0.3\"".to_owned(),
        "jni = \"0.22\"".to_owned(),
        "serde_json = \"1\"".to_owned(),
        "tokio = { version = \"1\", features = [\"rt-multi-thread\", \"macros\", \"sync\"] }".to_owned(),
    ];
    // When no overrides apply, the core-crate dep lives inline in `[dependencies]`.
    // Otherwise it is emitted in the per-target blocks below instead.
    if target_overrides.is_empty() {
        dep_lines.push(crate::scaffold::render_core_dep(
            umbrella_dep_name,
            &rel_path,
            &features_str,
            &api.version,
        ));
    }
    dep_lines.sort();
    let deps_section = dep_lines.join("\n");

    let target_blocks_section = render_jni_target_blocks(
        umbrella_dep_name,
        &rel_path,
        &features_str,
        &api.version,
        target_overrides,
    );

    // cargo-sort orders sub-tables of `[package]` (e.g.
    // `[package.metadata.cargo-machete]`) immediately after the `[package]`
    // section and before `[lib]` / `[dependencies]`. Emit in that order so
    // prek's cargo-sort hook does not rewrite the file on every regen.
    let content = format!(
        r#"# Generated by alef. Do not edit by hand.

[package]
name = "{jni_crate_name}"
version.workspace = true
edition.workspace = true
license.workspace = true

# `base64`, `futures-util`, `serde_json`, and `tokio` are emitted unconditionally below
# so the manifest is stable across regens (they are used when the umbrella
# crate declares binary top-level params, async fns, streaming adapters, or JSON-marshalled types),
# but for an umbrella crate that has none of those they are genuinely unused.
# List them here so `cargo machete` doesn't flag the no-async-no-streaming
# case as a real finding.
[package.metadata.cargo-machete]
ignored = ["base64", "futures-util", "serde_json", "tokio"]

[lib]
name = "{jni_lib_name}"
crate-type = ["cdylib"]

[dependencies]
{deps_section}
{target_blocks_section}"#,
        jni_crate_name = jni_crate_name,
        jni_lib_name = jni_lib_name,
        deps_section = deps_section,
        target_blocks_section = target_blocks_section,
    );

    let _ = api; // api not needed for Cargo.toml scaffold

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{jni_crate_name}/Cargo.toml")),
        content,
        generated_header: false,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::ApiSurface;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
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
        // Umbrella dep key is the Cargo package name (config.name), not the crate_dir.
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
    /// emitted in alphabetical order so the `cargo-sort` prek hook does not
    /// rewrite the file on every regen. The umbrella dep is named after
    /// `config.name`, so its placement depends on the consumer crate name.
    #[test]
    fn scaffold_jni_dependencies_are_alphabetically_sorted() {
        // Use a config.name that sorts in the middle of the static deps
        // (between `jni` and `serde_json`) to make sure the sort moves it
        // out of its naive first-position insertion slot.
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

        // Extract [dependencies] keys in emission order.
        let mut keys: Vec<&str> = Vec::new();
        let mut in_deps = false;
        for line in cargo_toml.lines() {
            if line.trim_start().starts_with('[') {
                in_deps = line.trim() == "[dependencies]";
                continue;
            }
            if in_deps && !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                if let Some(key) = line.split('=').next() {
                    let key = key.trim();
                    if !key.is_empty() {
                        keys.push(key);
                    }
                }
            }
        }
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(
            keys, sorted,
            "JNI Cargo.toml [dependencies] must be alphabetically sorted; got:\n{keys:?}\nin:\n{cargo_toml}"
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

        // The inline `[dependencies]` table must NOT carry the unconditional
        // core-crate dep when overrides apply.
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
        // One core-dep line per target branch: default + android + ios + windows.
        let core_dep_lines = cargo_toml.matches("demo-doc = {").count();
        assert_eq!(
            core_dep_lines, 4,
            "expected one core-dep line per target branch (default + 3 overrides); got {core_dep_lines}:\n{cargo_toml}"
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
}
