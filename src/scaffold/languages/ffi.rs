use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, FfiTargetDepOverride, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::detect_workspace_inheritance_for_crate, scaffold::render_extra_deps,
    scaffold::scaffold_meta,
};
use std::path::PathBuf;

/// Render the core-crate dependency portion of the FFI Cargo.toml.
///
/// Returns a tuple of `(core_dep_line, target_blocks)` where:
/// - `core_dep_line` is the single TOML line that goes inside the main
///   `[dependencies]` table (empty string when target overrides are in use).
/// - `target_blocks` contains any `[target.'cfg(...)'.dependencies]` sections
///   that follow the main `[dependencies]` table (empty string by default).
///
/// When `overrides` is empty the behaviour matches the historical output:
/// a single `{crate} = { path = ..., features = [...] }` line lives next to
/// `serde_json` / `tokio` inside `[dependencies]`. When overrides are
/// present the core-crate dependency moves out into per-cfg target blocks;
/// the default branch is wrapped in `cfg(not(any(<cfg1>, <cfg2>, ...)))` so
/// that exactly one variant matches on any given build (most importantly
/// `x86_64-linux-android`, which has no ONNX Runtime prebuilt).
fn render_core_dep(
    crate_name: &str,
    rel_path: &str,
    version: &str,
    default_features: &str,
    overrides: &[FfiTargetDepOverride],
) -> (String, String) {
    if overrides.is_empty() {
        let line = format!("{crate_name} = {{ path = \"{rel_path}\", version = \"{version}\"{default_features} }}");
        return (line, String::new());
    }

    let cfgs: Vec<String> = overrides.iter().map(|o| o.cfg.clone()).collect();
    let combined_cfg = if cfgs.len() == 1 {
        cfgs[0].clone()
    } else {
        format!("any({})", cfgs.join(", "))
    };

    let mut entries: Vec<(String, String)> = vec![(
        format!("not({combined_cfg})"),
        format!("{crate_name} = {{ path = \"{rel_path}\", version = \"{version}\"{default_features} }}"),
    )];
    for override_ in overrides {
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
        entries.push((
            override_.cfg.clone(),
            format!("{crate_name} = {{ path = \"{rel_path}\", version = \"{version}\"{default_block}{features_str} }}"),
        ));
    }
    // See `crate::scaffold::join_sorted_target_dep_blocks`: cargo-sort orders
    // `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
    // predicate string, so the default `not(...)` branch is not always first. ~keep
    (String::new(), crate::scaffold::join_sorted_target_dep_blocks(entries))
}

pub(crate) fn scaffold_ffi(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let crate_dir = format!("crates/{core_crate_dir}-ffi");
    let ws = detect_workspace_inheritance_for_crate(config.workspace_root.as_deref(), &crate_dir);
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-ffi"), version, "2024", &meta, &ws);

    let rendered_extra_deps = render_extra_deps(config, Language::Ffi);
    let mut extra_dep_lines: Vec<String> = if rendered_extra_deps.is_empty() {
        Vec::new()
    } else {
        rendered_extra_deps.lines().map(str::to_string).collect()
    };
    let has_trait_bridges = !config.trait_bridges.is_empty();
    if has_trait_bridges && !extra_dep_lines.iter().any(|l| l.starts_with("async-trait")) {
        extra_dep_lines.push(format!("async-trait = \"{}\"", tv::cargo::ASYNC_TRAIT));
    }
    if has_trait_bridges && !extra_dep_lines.iter().any(|l| l.starts_with("tracing")) {
        extra_dep_lines.push(format!("tracing = \"{}\"", tv::cargo::TRACING));
    }
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    if has_streaming && !extra_dep_lines.iter().any(|l| l.starts_with("futures-util")) {
        extra_dep_lines.push("futures-util = \"0.3\"".to_string());
    }
    if let Some(ffi) = config.ffi.as_ref() {
        for capsule in ffi.capsule_types.values() {
            let (Some(package), Some(version)) = (capsule.package.as_ref(), capsule.package_version.as_ref()) else {
                continue;
            };
            let dep_prefix = format!("{package} ");
            if !extra_dep_lines.iter().any(|l| l.starts_with(&dep_prefix)) {
                extra_dep_lines.push(format!("{package} = \"{version}\""));
            }
        }
    }
    if !config.components.is_empty() {
        let alef_version = env!("CARGO_PKG_VERSION");
        for (name, dependency) in [
            ("alef-component-abi", format!("alef-component-abi = \"{alef_version}\"")),
            (
                "alef-component-runtime",
                format!("alef-component-runtime = \"{alef_version}\""),
            ),
            ("directories", "directories = \"6\"".to_string()),
        ] {
            let key = format!("{name} =");
            if !extra_dep_lines.iter().any(|line| line.starts_with(&key)) {
                extra_dep_lines.push(dependency);
            }
        }
    }
    crate::scaffold::sort_dependency_lines(&mut extra_dep_lines);

    let mut machete_ignored: Vec<&str> = vec!["ahash", "serde", "serde_json", "tokio"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
        machete_ignored.push("tracing");
    }
    if has_streaming {
        machete_ignored.push("futures-util");
    }
    let machete_ignored_str = machete_ignored
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect::<Vec<_>>()
        .join(", ");

    let target_overrides: &[FfiTargetDepOverride] = config
        .ffi
        .as_ref()
        .map(|c| c.target_dep_overrides.as_slice())
        .unwrap_or(&[]);
    let excluded_default_features: std::collections::HashSet<&str> = config
        .ffi
        .as_ref()
        .map(|c| c.excluded_default_features.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let core_dep_path = config.core_crate_dep_path(std::path::Path::new(&crate_dir));
    let (core_dep_line, target_blocks) = render_core_dep(
        &config.name,
        &core_dep_path,
        version,
        &crate::scaffold::core_dep_features_excluding(config, Language::Ffi, &excluded_default_features),
        target_overrides,
    );

    // FFI source uses `#[cfg(feature = "X")]` to gate code paths driven by core-crate
    let ffi_core_features = config.features_for_language(Language::Ffi);
    let passthrough_feature_names: Vec<&str> = ffi_core_features
        .iter()
        .map(|f| f.as_str())
        .filter(|f| *f != "serde")
        .collect();
    // Cargo features are per-crate: `full = ["<core>/full"]` enabling `X` on the dependency
    // does NOT create feature `X` here, so a `#[cfg(feature = "X")]` the codegen emits into
    // this crate is unsatisfiable unless this crate declares `X` itself. An undeclared gate is
    // never true, which silently drops the export from the cdylib while cbindgen still declares
    // it in the header -- a link failure for every C-ABI consumer. Declare every feature the
    // emitted surface actually gates on. ~keep
    //
    // `default_feature_names` is `effective_ffi_default_features` -- the ONE derivation of what
    // the compiled FFI cdylib builds with by default. `warn_on_ffi_feature_drift` compares
    // against this exact same derivation instead of re-deriving it from `passthrough_feature_names`
    // alone, so the two can no longer disagree about what "the FFI crate's feature set" means
    // (see github.com/xberg-io/alef/issues/257). ~keep
    let default_feature_names_owned = crate::codegen::cfg::effective_ffi_default_features(api, config);
    let default_feature_names: Vec<&str> = default_feature_names_owned.iter().map(String::as_str).collect();
    let core_features_default_list = default_feature_names
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut core_features_passthrough_block = if default_feature_names.is_empty() {
        String::new()
    } else {
        default_feature_names
            .iter()
            .map(|f| format!("{f} = [\"{}/{f}\"]", config.name))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if let Some(line) = crate::scaffold::android_target_feature_line(config, &default_feature_names) {
        if core_features_passthrough_block.is_empty() {
            core_features_passthrough_block = line;
        } else {
            core_features_passthrough_block.push('\n');
            core_features_passthrough_block.push_str(&line);
        }
    }
    // feature in a `#[cfg(feature = "X")]` gate (e.g. a `wasm-http` backend that is
    if let Some(extra) = config.ffi.as_ref().map(|c| c.extra_features.as_slice()) {
        for feat in extra {
            if feat.is_empty() || passthrough_feature_names.contains(&feat.as_str()) {
                continue;
            }
            let line = format!("{feat} = [\"{}/{feat}\"]", config.name);
            if core_features_passthrough_block.is_empty() {
                core_features_passthrough_block = line;
            } else {
                core_features_passthrough_block.push('\n');
                core_features_passthrough_block.push_str(&line);
            }
        }
    }
    // `effective_ffi_default_features` already drops `excluded_default_features` names from
    // `default_feature_names`, so `cargo build --features <name>` would fail to declare the
    // feature at all without this: a name listed there stays a *declared* opt-in flag, just
    // never defaulted, the same tradeoff `extra_features` makes above. ~keep
    for feat in &excluded_default_features {
        // Deliberately checked against `default_feature_names` alone, not
        // `passthrough_feature_names`: a name in `excluded_default_features` is stripped out of
        // `default_feature_names` by `effective_ffi_default_features` even when it IS present in
        // `passthrough_feature_names` (i.e. explicitly configured), so that unfiltered list can
        // no longer prove a declare-only row already exists for it. ~keep
        if default_feature_names.contains(feat) {
            continue;
        }
        let line = format!("{feat} = [\"{}/{feat}\"]", config.name);
        if core_features_passthrough_block.is_empty() {
            core_features_passthrough_block = line;
        } else {
            core_features_passthrough_block.push('\n');
            core_features_passthrough_block.push_str(&line);
        }
    }
    let target_blocks_section = if target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{target_blocks}\n")
    };

    let mut dep_entries: Vec<String> = vec![
        "ahash = \"0.8\"".to_string(),
        format!("serde = \"{}\"", tv::cargo::SERDE),
        "serde_json = \"1\"".to_string(),
        "tokio = { version = \"1\", features = [\"full\"] }".to_string(),
    ];
    if !core_dep_line.is_empty() {
        dep_entries.push(core_dep_line.clone());
    }
    for line in &extra_dep_lines {
        dep_entries.push(line.clone());
    }
    crate::scaffold::sort_dependency_lines(&mut dep_entries);
    let dep_block = dep_entries.join("\n");
    let repository_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("\nrepository = \"{repository}\""))
        .unwrap_or_default();
    let lints_section = crate::scaffold::cargo_lints_section(config);

    let content = format!(
        r#"{pkg_header}{repository_line}

# `serde`, `serde_json`, `ahash`, and `tokio` are emitted unconditionally above so the
# manifest is stable across regens (and so the C FFI codegen can pull them in
# when an async / Result-typed function appears in the API surface), but for
# umbrella crates with no async fns and no JSON-marshalled return types they
# are genuinely unused. The conditional `async-trait` / `futures-util` deps
# are similarly flagged when the umbrella has trait-bridge / streaming adapters
# configured but no actual async-trait / async-stream callsite in the generated
# FFI shim.
[package.metadata.cargo-machete]
ignored = [{machete_ignored_str}]

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[features]
default = [{core_features_default_list}]
{core_features_passthrough_block}

[dependencies]
{dep_block}
{target_blocks_section}
[build-dependencies]
cbindgen = "{cbindgen}"

[dev-dependencies]
tempfile = "{tempfile}"
{lints_section}"#,
        pkg_header = pkg_header,
        repository_line = repository_line,
        lints_section = lints_section,
        dep_block = dep_block,
        target_blocks_section = target_blocks_section,
        cbindgen = tv::cargo::CBINDGEN,
        tempfile = tv::cargo::TEMPFILE,
        machete_ignored_str = machete_ignored_str,
        core_features_default_list = core_features_default_list,
        core_features_passthrough_block = core_features_passthrough_block,
    );

    let ffi_name = format!("{core_crate_dir}-ffi");
    let header_name = config.ffi_header_name();
    let lib_name = config.ffi_lib_name();
    let ffi_name_under = ffi_name.replace('-', "_");

    // The odd, non-uniform indentation below (top-level `if`/`endif` bodies flush left,
    // nested blocks stepping by 4 spaces instead of consistently by 2) is not a style
    // choice — it is poly's actual fixed point for `.cmake` files under its tree-sitter-based
    // formatter (verified empirically: `poly fmt --fix --fix-generated` on a conventionally
    // 2-space-indented equivalent converges to exactly this shape and is then stable under a
    // repeated `--check`). Matching it here avoids the generate/format oscillation that a
    // "cleaner" hand-indented version would trigger on every regen. ~keep
    let cmake_content = crate::scaffold::template_env::render(
        "ffi_config.cmake.jinja",
        minijinja::context! {
            ffi_name => ffi_name,
            ffi_name_under => ffi_name_under,
            lib_name => lib_name,
            header_name => header_name,
        },
    );

    let files = vec![
        GeneratedFile {
            path: PathBuf::from(format!("crates/{}-ffi/Cargo.toml", core_crate_dir)),
            content,
            generated_header: true,
        },
        GeneratedFile {
            path: PathBuf::from(format!(
                "crates/{}-ffi/cmake/{}-ffi-config.cmake",
                core_crate_dir, core_crate_dir
            )),
            content: cmake_content,
            generated_header: true,
        },
    ];
    Ok(files)
}

#[cfg(test)]
#[path = "ffi/tests.rs"]
mod tests;
