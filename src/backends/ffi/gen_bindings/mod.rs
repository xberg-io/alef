mod capsule;
mod functions;
mod helpers;
mod lib_rs;
mod lib_setup;
mod service_api;
mod types;

use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

use helpers::{cbindgen_exclude_type_names, gen_build_rs, gen_cbindgen_toml};

pub struct FfiBackend;

impl FfiBackend {}

impl Backend for FfiBackend {
    fn name(&self) -> &str {
        "ffi"
    }

    fn language(&self) -> Language {
        Language::Ffi
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let prefix = config.ffi_prefix();
        let header_name = config.ffi_header_name();
        let lib_name = config.ffi_lib_name();

        let output_dir = config
            .output_for("ffi")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("crates/{}-ffi/src/", config.name));

        let parent_dir = PathBuf::from(&output_dir)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();

        let go_output_dir = if config.targets(Language::Go) {
            config.output_paths.get("go").map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };

        // Capsule (Language-passthrough) types configured under `[crates.ffi.capsule_types]`.
        // Drives both the cbindgen forward declarations and the opaque-handle suppression in lib.rs.
        let ffi_capsule_types: std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig> =
            config.ffi.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();
        let cbindgen_exclude_types = cbindgen_exclude_type_names(api, config);

        let files = vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join("lib.rs"),
                content: lib_rs::gen_lib_rs(api, &prefix, config),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("cbindgen.toml"),
                content: gen_cbindgen_toml(&prefix, api, &ffi_capsule_types, &cbindgen_exclude_types),
                generated_header: false,
            },
            GeneratedFile {
                path: parent_dir.join("build.rs"),
                content: gen_build_rs(
                    &header_name,
                    &format!("lib{lib_name}"),
                    go_output_dir.as_deref(),
                    &prefix,
                    &ffi_capsule_types,
                ),
                generated_header: false,
            },
        ];

        Ok(files)
    }

    fn generate_service_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        service_api::generate(api, config)
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-ffi",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// lib.rs generation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
