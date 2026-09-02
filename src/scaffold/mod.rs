//! Package scaffolding generator for alef.

use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

mod cargo_config;
mod cargo_deps;
mod cargo_lints;
mod core_features;
mod generated_files;
mod languages;
mod manifest_header;
pub(crate) mod naming;
mod repair;
mod template_env;
mod text_helpers;
pub(crate) mod version_floor;

use generated_files::{scaffold_gitattributes, scaffold_license_files};
pub(crate) use repair::repair_missing_cfg_binding_features;

pub use languages::{
    PUBLISHED_RUNTIME_IDENTIFIERS, render_csharp_csproj, render_csharp_runtime_csproj,
    render_csharp_runtime_json_template,
};
pub(crate) use languages::{
    elixir_native_crate_dir, migrate_build_zig_test_target, migrate_dart_placeholder_test, migrate_dart_pubignore,
    migrate_java_checkstyle_line_length, migrate_kotlin_build_gradle, migrate_node_package_json_service_export,
    migrate_php_composer_phpunit_constraint, migrate_poly_toml_drop_snippet_hook,
    migrate_poly_toml_drop_unrunnable_snapshot_hooks, migrate_swift_placeholder_test,
    migrate_wasm_cargo_config_allow_multiple_definition, migrate_wasm_package_json,
    migrate_zig_build_ffi_include_default, migrate_zig_example, ruby_native_manifest_path,
};

pub use manifest_header::{ScaffoldMeta, scaffold_meta};
pub(crate) use manifest_header::{
    WorkspacePackageInheritance, cargo_package_header, detect_workspace_inheritance,
    detect_workspace_inheritance_for_crate, readme_language_configured,
};

pub(crate) use text_helpers::capitalize_first;
pub use text_helpers::{parse_author, xml_escape};

pub(crate) use cargo_deps::{
    cargo_dependency_declared, dependency_sort_key, join_sorted_target_dep_blocks, render_core_dep,
    render_core_dep_with_overrides, render_extra_deps, render_workspace_dep_or, sort_dependency_lines,
};

pub(crate) use cargo_lints::{cargo_lints_clippy_block_with_rationale, cargo_lints_section};

pub(crate) use core_features::{
    android_target_feature_line, android_target_feature_line_for_dep, core_crate_manifest_path, core_dep_features,
    core_dep_features_excluding, core_feature_closure,
};

pub use cargo_config::render_cargo_config;
pub(crate) use cargo_config::rust_toolchain_file;

pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = vec![];
    for &lang in languages {
        files.extend(scaffold_language(api, config, lang)?);
    }
    // Every binding manifest above is `generated_header: true` and therefore rewritten in
    // full, so a dependency literal that has fallen behind the consumer's own tree is
    // written back over their bump on every run. Raise each requirement to the committed
    // one before anything else sees these files, so the whole pipeline -- write, `diff`,
    // `verify` -- agrees that a version alef would have lowered is not a difference. ~keep
    version_floor::apply_version_floors(&mut files, config);
    files.extend(scaffold_poly_config(config, languages));

    // LICENSE sync — copy the workspace-root LICENSE into every per-language
    // package directory so ecosystems like pub.dev (Dart) that require a LICENSE
    // LICENSE file is present at the workspace root.
    files.extend(scaffold_license_files(config, languages));

    if !std::path::Path::new("rust-toolchain.toml").exists() {
        files.push(rust_toolchain_file(languages));
    }

    if let Some(cargo) = config.scaffold.as_ref().and_then(|s| s.cargo.as_ref()) {
        files.push(GeneratedFile {
            path: std::path::PathBuf::from(".cargo/config.toml"),
            content: render_cargo_config(cargo),
            generated_header: true,
        });
    } else if languages.contains(&Language::Wasm) && !std::path::Path::new(".cargo/config.toml").exists() {
        files.push(wasm_cargo_config_file());
    }

    files.extend(scaffold_gitattributes(config, languages));

    Ok(files)
}

use languages::{
    scaffold_csharp, scaffold_dart, scaffold_elixir, scaffold_elixir_cargo, scaffold_ffi, scaffold_gleam, scaffold_go,
    scaffold_java, scaffold_jni, scaffold_kotlin, scaffold_node, scaffold_node_cargo, scaffold_php, scaffold_php_cargo,
    scaffold_poly_config, scaffold_python, scaffold_python_cargo, scaffold_r, scaffold_r_cargo, scaffold_ruby,
    scaffold_ruby_cargo, scaffold_swift, scaffold_wasm, scaffold_zig, wasm_cargo_config_file,
};

fn scaffold_language(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    lang: Language,
) -> anyhow::Result<Vec<GeneratedFile>> {
    match lang {
        Language::Python => {
            let mut files = scaffold_python(api, config)?;
            files.extend(scaffold_python_cargo(api, config)?);
            Ok(files)
        }
        Language::Node => {
            let mut files = scaffold_node(api, config)?;
            files.extend(scaffold_node_cargo(api, config)?);
            Ok(files)
        }
        Language::Ffi => scaffold_ffi(api, config),
        Language::Go => scaffold_go(api, config),
        Language::Java => scaffold_java(api, config),
        Language::Csharp => scaffold_csharp(api, config),
        Language::Ruby => {
            let mut files = scaffold_ruby(api, config)?;
            files.extend(scaffold_ruby_cargo(api, config)?);
            Ok(files)
        }
        Language::Php => {
            let mut files = scaffold_php(api, config)?;
            files.extend(scaffold_php_cargo(api, config)?);
            Ok(files)
        }
        Language::Elixir => {
            let mut files = scaffold_elixir(api, config)?;
            files.extend(scaffold_elixir_cargo(api, config)?);
            Ok(files)
        }
        Language::Wasm => scaffold_wasm(api, config),
        Language::R => {
            let mut files = scaffold_r(api, config)?;
            files.extend(scaffold_r_cargo(api, config)?);
            Ok(files)
        }
        Language::Rust | Language::C => Ok(vec![]),
        Language::Jni => scaffold_jni(api, config),
        Language::Kotlin => scaffold_kotlin(api, config),
        Language::KotlinAndroid => Ok(vec![]),
        Language::Gleam => scaffold_gleam(api, config),
        Language::Zig => scaffold_zig(api, config),
        Language::Dart => scaffold_dart(api, config),
        Language::Swift => scaffold_swift(api, config),
    }
}

#[cfg(test)]
mod tests;
