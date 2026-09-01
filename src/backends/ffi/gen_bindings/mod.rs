mod capsule;
mod field_ownership;
mod functions;
mod helpers;
mod lib_rs;
mod lib_setup;
mod rust_literal;
pub(crate) mod service_api;
mod types;

use crate::core::backend::{
    Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile, TraitBridgeRegistrationSurface,
};
use crate::core::config::{Language, OutputLayout, ResolvedCrateConfig};
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
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
        api.validate_error_taxonomy()?;
        let prefix = config.ffi_prefix();
        let header_name = config.ffi_header_name();
        let lib_name = config.ffi_lib_name();

        let output_dir = config
            .output_for("ffi")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("crates/{}-ffi/src/", config.name));

        validate_custom_modules_exist(config, &output_dir, config.custom_modules.for_language(Language::Ffi))?;

        // `cbindgen.toml` and `build.rs` belong to the FFI crate, so they are placed against
        // its root rather than against `output_dir`'s parent: a crate-root-shaped output path
        // has no `src` to strip, and stripping one anyway drops both files into the directory
        // that holds every *other* language's package. ~keep
        let crate_root = OutputLayout::from_output_dir(&output_dir).root;

        let go_output_dir = if config.targets(Language::Go) {
            config.output_paths.get("go").map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };

        let ffi_capsule_types: std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig> =
            config.ffi.as_ref().map(|c| c.capsule_types.clone()).unwrap_or_default();
        let cbindgen_exclude_types = cbindgen_exclude_type_names(api, config);

        let files = vec![
            GeneratedFile {
                path: PathBuf::from(&output_dir).join("lib.rs"),
                content: lib_rs::gen_lib_rs(api, &prefix, config)?,
                generated_header: false,
            },
            GeneratedFile {
                path: crate_root.join("cbindgen.toml"),
                content: gen_cbindgen_toml(&prefix, api, &ffi_capsule_types, &cbindgen_exclude_types),
                generated_header: false,
            },
            GeneratedFile {
                path: crate_root.join("build.rs"),
                content: gen_build_rs(
                    &header_name,
                    &format!("lib{lib_name}"),
                    &crate_root.to_string_lossy(),
                    go_output_dir.as_deref(),
                    &prefix,
                    &ffi_capsule_types,
                )?,
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
        // Order the IR once, before anything reads it: every emission loop below concatenates
        // api.types/enums/functions/errors into a single generated file in Vec order. ~keep
        let sorted_api = crate::backends::ir_order::with_sorted_items(api);
        let api = &sorted_api;
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

    fn trait_bridge_registration_surface(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> Vec<TraitBridgeRegistrationSurface> {
        crate::backends::ffi::trait_bridge::registration_surface(api, config)
    }
}

/// Validate that every `[crates.custom_modules] ffi = [...]` entry resolves to a
/// hand-written source file that already exists under the FFI crate's `src/`
/// directory.
///
/// `custom_modules.ffi` only *declares* the module (`pub mod <name>;`) in the
/// generated `lib.rs` (see `lib_rs::gen_lib_rs`) — it never generates the
/// module's contents, by design: this is the mechanism for compiling a
/// hand-authored file (e.g. an opaque-handle wrapper around a core type
/// marked `alef(skip)`) into the generated crate. A configured name with no
/// matching file compiles to `error[E0583]: file not found for module
/// <name>`, reported by rustc against the *generated* `lib.rs` with no
/// pointer back to the config key that caused it. Catching it here, before
/// that file is even written, turns it into an alef-level error that names
/// the crate, the module, and the exact paths that were checked.
fn validate_custom_modules_exist(
    config: &ResolvedCrateConfig,
    output_dir: &str,
    modules: &[String],
) -> anyhow::Result<()> {
    let src_dir = std::path::Path::new(output_dir);
    for module in modules {
        let flat = src_dir.join(format!("{module}.rs"));
        let nested = src_dir.join(module).join("mod.rs");
        if !flat.exists() && !nested.exists() {
            anyhow::bail!(
                "crate `{}`: `[crates.custom_modules] ffi` names module `{module}`, but neither `{}` nor `{}` \
                 exists. Write the module by hand — `custom_modules.ffi` only declares it.",
                config.name,
                flat.display(),
                nested.display(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
