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
    extra_dep_lines.sort();
    let extra_deps_block = if extra_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_dep_lines.join("\n"))
    };

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
    // Prepend a newline when the core-crate dep stays inside `[dependencies]`
    // so the generated TOML matches the historical layout exactly.
    let core_dep_line_block = if core_dep_line.is_empty() {
        String::new()
    } else {
        format!("{core_dep_line}\n")
    };
    // Separate the main [dependencies] table from any per-target tables.
    let target_blocks_section = if target_blocks.is_empty() {
        String::new()
    } else {
        format!("\n{target_blocks}\n")
    };

    let content = format!(
        r#"{pkg_header}
repository = "{repository}"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{core_dep_line_block}ahash = "0.8"
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}{extra_deps_block}
{target_blocks_section}
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

[features]
default = []

[build-dependencies]
cbindgen = "{cbindgen}"

[dev-dependencies]
tempfile = "{tempfile}"
"#,
        pkg_header = pkg_header,
        repository = meta.repository,
        core_dep_line_block = core_dep_line_block,
        target_blocks_section = target_blocks_section,
        cbindgen = tv::cargo::CBINDGEN,
        tempfile = tv::cargo::TEMPFILE,
        extra_deps_block = extra_deps_block,
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
}
