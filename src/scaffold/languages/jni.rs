use crate::core::backend::GeneratedFile;
use crate::core::config::{FfiTargetDepOverride, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
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

    let mut entries: Vec<(String, String)> = vec![(
        format!("not({combined_cfg})"),
        crate::scaffold::render_core_dep(crate_name, rel_path, default_features, version),
    )];
    for override_ in overrides {
        // `FfiTargetDepOverride::default_features` (false by default) must reach this manifest
        // the same way it reaches the FFI crate's own `render_core_dep` and
        // `scaffold::render_core_dep_with_overrides`: a per-target `default_features = false`
        // that isn't read here silently vanishes from the generated JNI Cargo.toml. ~keep
        let default_block = if override_.default_features {
            String::new()
        } else {
            ", default-features = false".to_string()
        };
        let features_str = if override_.features.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = override_.features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(", features = [{}]", quoted.join(", "))
        };
        let override_features = format!("{default_block}{features_str}");
        entries.push((
            override_.cfg.clone(),
            crate::scaffold::render_core_dep(crate_name, rel_path, &override_features, version),
        ));
    }
    // See `crate::scaffold::join_sorted_target_dep_blocks`: cargo-sort orders
    // `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
    // predicate string, so the default `not(...)` branch is not always first. ~keep
    let joined = crate::scaffold::join_sorted_target_dep_blocks(entries);
    format!("\n{joined}")
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
    let jni_crate_name = format!("{}-jni", config.jni_crate_base());
    let jni_lib_name = config.jni_lib_name();

    // Route through `core_dep_features` rather than reading `[crates.kotlin_android] features`
    // directly: that lookup falls back to the top-level `features` when the paired section
    // omits them, which every other binding scaffolder already gets. Reading the section alone
    // left the JNI manifest on the core crate's default features while the generated shim
    // called into feature-gated modules, so the crate could not compile. ~keep
    let features_str = crate::scaffold::core_dep_features(config, Language::KotlinAndroid);

    let umbrella_dep_name = &config.name;

    // `[target.'cfg(...)'.dependencies]` blocks. The default branch is wrapped
    // in `cfg(not(any(...)))` so exactly one variant matches on any build.
    let target_overrides = config
        .jni
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    // Same derivation as the manifest's own location, rather than assuming the core crate is a
    // `crates/` sibling: in a root-flat tree `../{core_crate_dir}` names a directory the emitted
    // tree does not contain, and cargo fails to read it before compiling anything. ~keep
    let rel_path = config.core_crate_dep_path(std::path::Path::new(&format!("crates/{jni_crate_name}")));

    let mut dep_lines: Vec<String> = vec![
        crate::scaffold::render_workspace_dep_or(config, "async-trait", &format!("\"{}\"", tv::cargo::ASYNC_TRAIT)),
        crate::scaffold::render_workspace_dep_or(config, "base64", &format!("\"{}\"", tv::cargo::BASE64)),
        crate::scaffold::render_workspace_dep_or(config, "futures-util", &format!("\"{}\"", tv::cargo::FUTURES_UTIL)),
        crate::scaffold::render_workspace_dep_or(config, "jni", &format!("\"{}\"", tv::cargo::JNI)),
        crate::scaffold::render_workspace_dep_or(config, "serde_json", &format!("\"{}\"", tv::cargo::SERDE_JSON)),
        crate::scaffold::render_workspace_dep_or(
            config,
            "tokio",
            "{ version = \"1\", features = [\"rt-multi-thread\", \"macros\", \"sync\"] }",
        ),
        crate::scaffold::render_workspace_dep_or(config, "tracing", &format!("\"{}\"", tv::cargo::TRACING)),
    ];
    if target_overrides.is_empty() {
        dep_lines.push(crate::scaffold::render_core_dep(
            umbrella_dep_name,
            &rel_path,
            &features_str,
            &api.version,
        ));
    }

    // Capsule types (e.g. tree-sitter's `Language`) used to make the JNI shim emit
    // `value.into_raw() as *const {into_raw_type}` casts, which would have required the
    // crate backing `into_raw_type` as a direct dependency here. `method_capsule_return.rs.jinja`
    // no longer emits that cast: `into_raw()` already returns the pointee type verbatim, so an
    // explicit `as *const T` was a same-type cast tripping `clippy::unnecessary_cast` (see
    // `capsule_returns_transfer_the_pointer_without_a_redundant_cast` in `gen_shims::tests`).
    // The JNI shim now calls `.into_raw()` through type inference alone and never spells the
    // capsule crate's path in generated source, so it must not be added as a direct dependency
    // of this manifest -- it is unused there, reachable only transitively through the umbrella
    // core crate. Adding it unconditionally made `cargo machete` correctly flag it as unused
    // (alef task #145). ~keep

    crate::scaffold::sort_dependency_lines(&mut dep_lines);
    let deps_section = dep_lines.join("\n");

    let target_blocks_section = render_jni_target_blocks(
        umbrella_dep_name,
        &rel_path,
        &features_str,
        &api.version,
        target_overrides,
    );

    let lints_section = crate::scaffold::cargo_lints_section(config);
    // Every sibling scaffolder (ffi, python, php, ruby) asks
    // `detect_workspace_inheritance_for_crate` whether a `[workspace.package]` is actually
    // reachable before writing `*.workspace = true`; this one used to assert it. In a
    // root-flat emitted tree there is no workspace root, so `edition.workspace = true` makes
    // the manifest unparseable ("failed to find a workspace root") and every downstream
    // cargo command over the JNI crate dies before compiling a line. ~keep
    let crate_dir = format!("crates/{jni_crate_name}");
    let ws = crate::scaffold::detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let cargo_filters = crate::scaffold::cargo_package_file_filters(config);
    let package_header = crate::scaffold::cargo_package_header(
        &jni_crate_name,
        &api.version,
        "2024",
        &crate::scaffold::scaffold_meta(config),
        &ws,
        cargo_filters,
    );
    let content = format!(
        r#"# Generated by alef. Do not edit by hand.

{package_header}

# `base64`, `futures-util`, `serde_json`, and `tokio` are emitted unconditionally below
# so the manifest is stable across regens (they are used when the umbrella
# crate declares binary top-level params, async fns, streaming adapters, or JSON-marshalled types),
# but for an umbrella crate that has none of those they are genuinely unused.
# List them here so `cargo machete` doesn't flag the no-async-no-streaming
# case as a real finding.
[package.metadata.cargo-machete]
ignored = ["async-trait", "base64", "futures-util", "serde_json", "tokio", "tracing"]

[lib]
name = "{jni_lib_name}"
crate-type = ["cdylib"]

[dependencies]
{deps_section}
{target_blocks_section}{lints_section}"#,
        package_header = package_header,
        jni_lib_name = jni_lib_name,
        lints_section = lints_section,
        deps_section = deps_section,
        target_blocks_section = target_blocks_section,
    );

    let _ = api;

    Ok(vec![GeneratedFile {
        path: PathBuf::from(format!("crates/{jni_crate_name}/Cargo.toml")),
        content,
        generated_header: true,
    }])
}

#[cfg(test)]
#[path = "jni/tests.rs"]
mod tests;
