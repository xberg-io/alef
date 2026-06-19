use crate::core::backend::GeneratedFile;
use crate::core::config::{AdapterPattern, FfiTargetDepOverride, Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions as tv;
use crate::{
    scaffold::cargo_package_header, scaffold::core_dep_features, scaffold::detect_workspace_inheritance,
    scaffold::render_extra_deps, scaffold::scaffold_meta,
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
    core_crate_dir: &str,
    version: &str,
    default_features: &str,
    overrides: &[FfiTargetDepOverride],
) -> (String, String) {
    if overrides.is_empty() {
        let line =
            format!("{crate_name} = {{ path = \"../{core_crate_dir}\", version = \"{version}\"{default_features} }}");
        return (line, String::new());
    }

    let cfgs: Vec<String> = overrides.iter().map(|o| o.cfg.clone()).collect();
    let combined_cfg = if cfgs.len() == 1 {
        cfgs[0].clone()
    } else {
        format!("any({})", cfgs.join(", "))
    };

    let mut blocks = String::new();
    blocks.push_str(&format!(
        "[target.'cfg(not({combined_cfg}))'.dependencies]\n{crate_name} = {{ path = \"../{core_crate_dir}\", version = \"{version}\"{default_features} }}\n"
    ));
    for override_ in overrides {
        let features_str = if override_.features.is_empty() {
            String::new()
        } else {
            let quoted: Vec<String> = override_.features.iter().map(|f| format!("\"{f}\"")).collect();
            format!(", features = [{}]", quoted.join(", "))
        };
        blocks.push_str(&format!(
            "\n[target.'cfg({})'.dependencies]\n{crate_name} = {{ path = \"../{core_crate_dir}\", version = \"{version}\"{features_str} }}\n",
            override_.cfg
        ));
    }
    (String::new(), blocks)
}

pub(crate) fn scaffold_ffi(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(&format!("{core_crate_dir}-ffi"), version, "2021", &meta, &ws);

    // FFI Cargo.toml dependency policy:
    // - `serde_json` and `tokio` are pinned to major versions; cargo will pick
    //   the latest compatible release on `cargo update`.
    // - `cbindgen` is pinned to a specific minor since header generation
    //   output changes between minors and we want reproducible headers.
    //   Bump intentionally and verify generated headers in CI when updating.
    // - `[crate.extra_dependencies]` from alef.toml is merged in. Required for
    //   projects whose public surface spans multiple workspace crates (e.g.
    //   mylib-core, mylib-http, mylib-extra) — the FFI bindings codegen
    //   emits qualified paths like `mylib_http::ServerConfig` and needs each
    //   referenced crate as a direct dependency.
    // `render_extra_deps` injects the resolved workspace version into path-only
    // member tables (`{ path = "../<lib>-core", version = "X" }`) so the
    // generated FFI/umbrella manifest publishes cleanly — `cargo publish`
    // rejects path-only deps with "all dependencies must have a version
    // requirement specified when publishing". External deps (e.g. `anyhow =
    // "1.0"`) and already-versioned tables pass through unchanged. This mirrors
    // how the core-crate dep and the swift/dart bridge crates are handled.
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
    let has_streaming = config
        .adapters
        .iter()
        .any(|a| matches!(a.pattern, AdapterPattern::Streaming));
    if has_streaming && !extra_dep_lines.iter().any(|l| l.starts_with("futures-util")) {
        extra_dep_lines.push("futures-util = \"0.3\"".to_string());
    }
    // Inject the crate that provides each capsule type's `into_raw_type` pointee
    // (e.g. `tree-sitter` for `tree_sitter::ffi::TSLanguage`). The capsule shim
    // names that type directly, but the FFI crate only depends on it transitively
    // via the core crate, so it is not in scope without a direct dependency.
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
    extra_dep_lines.sort();

    // Build the cargo-machete ignored list. `serde_json` and `tokio` are
    // always emitted unconditionally above so they are always ignored.
    // Conditional deps (`async-trait` for trait bridges, `futures-util` for
    // streaming) are appended only when the scaffold actually adds them to
    // `[dependencies]`, so cargo-machete doesn't flap on umbrellas whose API
    // surface doesn't exercise the trait-bridge / streaming codepath.
    let mut machete_ignored: Vec<&str> = vec!["ahash", "serde_json", "tokio"];
    if has_trait_bridges {
        machete_ignored.push("async-trait");
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
    let (core_dep_line, target_blocks) = render_core_dep(
        &config.name,
        &core_crate_dir,
        version,
        &core_dep_features(config, Language::Ffi),
        target_overrides,
    );

    // FFI source uses `#[cfg(feature = "X")]` to gate code paths driven by core-crate
    // features (metadata, visitor, inline-images, testkit, …). The cfg matches against
    // THIS crate's feature flags, not the core dep's, so we must declare matching
    // passthrough features here. Without them, `RUSTFLAGS="-D warnings"` fails CI Rust
    // with `unexpected cfg condition value` errors on every gated function.
    let ffi_core_features = config.features_for_language(Language::Ffi);
    let passthrough_feature_names: Vec<&str> = ffi_core_features
        .iter()
        .map(|f| f.as_str())
        .filter(|f| *f != "serde") // serde is not a core feature here; it's a passthrough already enabled elsewhere when needed
        .collect();
    let core_features_default_list = passthrough_feature_names
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut core_features_passthrough_block = if passthrough_feature_names.is_empty() {
        String::new()
    } else {
        passthrough_feature_names
            .iter()
            .map(|f| format!("{f} = [\"{}/{f}\"]", config.name))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // Emit an `android-target` aggregate feature when the core crate defines one,
    // so consumers can build the FFI crate for Android with
    // `--no-default-features --features android-target` (ORT-free, libheif-free).
    // It enables the core dep's `android-target` plus every binding passthrough
    // feature that is a member of the core aggregate (the FFI exports are gated
    // by the binding's own passthrough features, not the core dep's).
    if let Some(line) = crate::scaffold::android_target_feature_line(config, &passthrough_feature_names) {
        if core_features_passthrough_block.is_empty() {
            core_features_passthrough_block = line;
        } else {
            core_features_passthrough_block.push('\n');
            core_features_passthrough_block.push_str(&line);
        }
    }
    // Declare any `extra_features` from [crates.ffi] as passthrough features that
    // are NOT enabled by default. The FFI source may reference a non-default core
    // feature in a `#[cfg(feature = "X")]` gate (e.g. a `wasm-http` backend that is
    // mutually exclusive with the default `native-http`); without a matching
    // declaration, `RUSTFLAGS="-D warnings"` fails with `unexpected cfg condition
    // value`. These are declared but excluded from `default` so the alternative
    // never activates alongside the default backend.
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
    // Separate the main [dependencies] table from any per-target tables.
    let target_blocks_section = if target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{target_blocks}\n")
    };

    // Build [dependencies] block, alphabetically sorted to match cargo-sort:
    // ahash, async-trait?, futures-util?, <core-crate>, serde_json, tokio, …
    // (cargo-sort orders dependencies by key name). core_dep_line is the
    // core crate dependency, which is sorted with other deps by crate name.
    // extra_dep_lines come from
    // [crate.extra_dependencies] in alef.toml and are pre-sorted above.
    let mut dep_entries: Vec<String> = vec![
        "ahash = \"0.8\"".to_string(),
        "serde_json = \"1\"".to_string(),
        "tokio = { version = \"1\", features = [\"full\"] }".to_string(),
    ];
    if !core_dep_line.is_empty() {
        dep_entries.push(core_dep_line.clone());
    }
    for line in &extra_dep_lines {
        dep_entries.push(line.clone());
    }
    dep_entries.sort();
    let dep_block = dep_entries.join("\n");
    let repository_line = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("\nrepository = \"{repository}\""))
        .unwrap_or_default();

    let content = format!(
        r#"{pkg_header}{repository_line}

# `serde_json`, `ahash`, and `tokio` are emitted unconditionally above so the
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
"#,
        pkg_header = pkg_header,
        repository_line = repository_line,
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

    let cmake_content = format!(
        r#"# {ffi_name} CMake config-mode find module
#
# Defines the imported target:
#   {ffi_name}::{ffi_name}
#
# Usage:
#   find_package({ffi_name} REQUIRED)
#   target_link_libraries(myapp PRIVATE {ffi_name}::{ffi_name})

if(TARGET {ffi_name}::{ffi_name})
  return()
endif()

get_filename_component(_FFI_CMAKE_DIR "${{CMAKE_CURRENT_LIST_FILE}}" PATH)
get_filename_component(_FFI_PREFIX "${{_FFI_CMAKE_DIR}}/.." ABSOLUTE)

find_library(_FFI_LIBRARY
  NAMES {lib_name} lib{lib_name}
  PATHS "${{_FFI_PREFIX}}/lib"
  NO_DEFAULT_PATH
)
if(NOT _FFI_LIBRARY)
  find_library(_FFI_LIBRARY NAMES {lib_name} lib{lib_name})
endif()

find_path(_FFI_INCLUDE_DIR
  NAMES {header_name}
  PATHS "${{_FFI_PREFIX}}/include"
  NO_DEFAULT_PATH
)
if(NOT _FFI_INCLUDE_DIR)
  find_path(_FFI_INCLUDE_DIR NAMES {header_name})
endif()

include(FindPackageHandleStandardArgs)
find_package_handle_standard_args({ffi_name}
  REQUIRED_VARS _FFI_LIBRARY _FFI_INCLUDE_DIR
)

if({ffi_name_under}_FOUND)
  set(_FFI_LIB_TYPE UNKNOWN)
  if(_FFI_LIBRARY MATCHES "\\.(dylib|so)$" OR _FFI_LIBRARY MATCHES "\\.so\\.")
    set(_FFI_LIB_TYPE SHARED)
  elseif(_FFI_LIBRARY MATCHES "\\.dll$")
    set(_FFI_LIB_TYPE SHARED)
  elseif(_FFI_LIBRARY MATCHES "\\.(a|lib)$")
    set(_FFI_LIB_TYPE STATIC)
  endif()

  add_library({ffi_name}::{ffi_name} ${{_FFI_LIB_TYPE}} IMPORTED)
  set_target_properties({ffi_name}::{ffi_name} PROPERTIES
    IMPORTED_LOCATION "${{_FFI_LIBRARY}}"
    INTERFACE_INCLUDE_DIRECTORIES "${{_FFI_INCLUDE_DIR}}"
  )

  if(WIN32 AND _FFI_LIB_TYPE STREQUAL "SHARED")
    find_file(_FFI_DLL
      NAMES {lib_name}.dll lib{lib_name}.dll
      PATHS "${{_FFI_PREFIX}}/bin" "${{_FFI_PREFIX}}/lib"
      NO_DEFAULT_PATH
    )
    if(_FFI_DLL)
      set_target_properties({ffi_name}::{ffi_name} PROPERTIES
        IMPORTED_LOCATION "${{_FFI_DLL}}"
        IMPORTED_IMPLIB "${{_FFI_LIBRARY}}"
      )
    endif()
    unset(_FFI_DLL CACHE)
  endif()

  if(APPLE)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES "-framework CoreFoundation" "-framework Security" pthread)
  elseif(UNIX)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES pthread dl m)
  elseif(WIN32)
    set_property(TARGET {ffi_name}::{ffi_name} APPEND PROPERTY
      INTERFACE_LINK_LIBRARIES ws2_32 userenv bcrypt)
  endif()

  unset(_FFI_LIB_TYPE)
endif()

mark_as_advanced(_FFI_LIBRARY _FFI_INCLUDE_DIR)
unset(_FFI_CMAKE_DIR)
unset(_FFI_PREFIX)
"#,
        ffi_name = ffi_name,
        ffi_name_under = ffi_name_under,
        lib_name = lib_name,
        header_name = header_name,
    );

    Ok(vec![
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
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_core_dep_includes_version_in_default_line() {
        let (core_dep_line, target_blocks) =
            render_core_dep("my-lib", "my-lib-core", "1.2.3", ", features = [\"foo\"]", &[]);

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
        }];
        let (core_dep_line, target_blocks) = render_core_dep("my-lib", "my-lib-core", "2.0.0", "", &overrides);

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
    }

    /// Regression: the FFI scaffold emitted `repository = "…"\n` (trailing LF)
    /// into a format string that already had a separating blank line, producing
    /// two consecutive blank lines between `repository = "…"` and the
    /// `[package.metadata.cargo-machete]` comment block.  cargo-sort removes
    /// one of them, causing prek to oscillate on every run.
    #[test]
    fn ffi_cargo_toml_repository_line_has_no_double_blank_line() {
        // Simulate the repository_line computation from scaffold_ffi to confirm
        // the format string produces exactly one blank line between the
        // repository entry and the cargo-machete comment.
        let repository = "https://github.com/example/my-lib";
        let repository_line = format!("\nrepository = \"{repository}\"");
        let pkg_header = "[package]\nname = \"my-lib-ffi\"\nversion = \"1.0.0\"";

        // Reproduce the exact format pattern used in scaffold_ffi.
        let content = format!("{pkg_header}{repository_line}\n\n# comment\n");

        // There must be exactly one blank line between repository and the comment.
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
}
