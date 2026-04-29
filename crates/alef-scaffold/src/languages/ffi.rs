use crate::{cargo_package_header, core_dep_features, detect_workspace_inheritance, scaffold_meta};
use alef_core::backend::GeneratedFile;
use alef_core::config::{AlefConfig, Language};
use alef_core::ir::ApiSurface;
use alef_core::template_versions as tv;
use std::path::PathBuf;

pub(crate) fn scaffold_ffi(api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let core_crate_dir = config.core_crate_dir();
    let ws = detect_workspace_inheritance(config.crate_config.workspace_root.as_deref());
    let pkg_header = cargo_package_header(
        &format!("{core_crate_dir}-ffi"),
        version,
        "2021",
        &meta.license,
        &meta.description,
        &meta.keywords,
        &ws,
    );

    // FFI Cargo.toml dependency policy:
    // - `serde_json` and `tokio` are pinned to major versions; cargo will pick
    //   the latest compatible release on `cargo update`.
    // - `cbindgen` is pinned to a specific minor since header generation
    //   output changes between minors and we want reproducible headers.
    //   Bump intentionally and verify generated headers in CI when updating.
    // - `[crate.extra_dependencies]` from alef.toml is merged in. Required for
    //   projects whose public surface spans multiple workspace crates (e.g.
    //   spikard-core, spikard-http, spikard-graphql) — the FFI bindings codegen
    //   emits qualified paths like `spikard_http::ServerConfig` and needs each
    //   referenced crate as a direct dependency.
    let extra_deps = config.extra_deps_for_language(Language::Ffi);
    let mut extra_dep_lines: Vec<String> = extra_deps
        .iter()
        .map(|(name, value)| {
            if let Some(s) = value.as_str() {
                format!("{name} = \"{s}\"")
            } else {
                format!("{name} = {value}")
            }
        })
        .collect();
    extra_dep_lines.sort();
    let extra_deps_block = if extra_dep_lines.is_empty() {
        String::new()
    } else {
        format!("\n{}", extra_dep_lines.join("\n"))
    };

    let content = format!(
        r#"{pkg_header}
repository = "{repository}"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
{crate_name} = {{ path = "../{core_crate_dir}"{features} }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}{extra_deps_block}

[features]
default = []

[build-dependencies]
cbindgen = "{cbindgen}"

[dev-dependencies]
tempfile = "{tempfile}"
"#,
        pkg_header = pkg_header,
        repository = meta.repository,
        crate_name = &config.crate_config.name,
        core_crate_dir = core_crate_dir,
        features = core_dep_features(config, Language::Ffi),
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
