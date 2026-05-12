use crate::naming::gleam_nif_module;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use alef_core::ir::ApiSurface;
use std::collections::BTreeSet;
use std::path::PathBuf;

mod helpers;
mod nif_external;
mod trait_bridge;
mod variant_collision;

use nif_external::{emit_enum, emit_error_type, emit_from_json_fn, emit_function, emit_method, emit_resource_type, emit_type};
use trait_bridge::{emit_trait_bridge_shims, emit_trait_support_nifs};
use variant_collision::build_collision_set;

fn gleam_module_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

pub struct GleamBackend;

impl Backend for GleamBackend {
    fn name(&self) -> &str {
        "gleam"
    }

    fn language(&self) -> Language {
        Language::Gleam
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_async: false,
            supports_classes: false,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            supports_callbacks: false,
            supports_streaming: false,
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let module_name = gleam_module_name(&config.name);
        let nif_module = gleam_nif_module(config);

        let exclude_functions: std::collections::HashSet<&str> = config
            .gleam
            .as_ref()
            .map(|c| c.exclude_functions.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let exclude_types: std::collections::HashSet<&str> = config
            .gleam
            .as_ref()
            .map(|c| c.exclude_types.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let mut imports: BTreeSet<&'static str> = BTreeSet::new();
        let mut body = String::new();

        // Emit regular (data/DTO) types here. Types that will be emitted as
        // opaque NIF resource handles below (non-trait types with methods) are
        // skipped to avoid duplicate type definitions.
        for ty in api
            .types
            .iter()
            .filter(|t| !(exclude_types.contains(t.name.as_str()) || !t.is_trait && !t.methods.is_empty()))
        {
            emit_type(ty, &mut body, &mut imports);
            body.push('\n');
        }

        let collisions = build_collision_set(api);

        for en in api.enums.iter().filter(|e| !exclude_types.contains(e.name.as_str())) {
            emit_enum(en, &collisions, &mut body, &mut imports);
            body.push('\n');
        }

        for err in &api.errors {
            emit_error_type(err, &collisions, &mut body, &mut imports);
            body.push('\n');
        }

        let declared_errors: Vec<String> = api.errors.iter().map(|e| e.name.clone()).collect();
        for f in api
            .functions
            .iter()
            .filter(|f| !exclude_functions.contains(f.name.as_str()))
        {
            emit_function(f, &nif_module, &declared_errors, &mut body, &mut imports);
            body.push('\n');
        }

        // Emit from_json NIF externals for every non-opaque serde-capable struct type.
        for ty in api.types.iter().filter(|t| {
            !t.is_trait
                && !t.is_opaque
                && !t.fields.is_empty()
                && t.has_serde
                && !exclude_types.contains(t.name.as_str())
        }) {
            emit_from_json_fn(ty, &nif_module, &mut body);
            body.push('\n');
        }

        // Emit opaque resource types and their instance methods (e.g. DefaultClient).
        // Only non-trait types with at least one method that are not excluded get emitted.
        for ty in api
            .types
            .iter()
            .filter(|t| !t.is_trait && !exclude_types.contains(t.name.as_str()) && !t.methods.is_empty())
        {
            emit_resource_type(ty, &mut body, &mut imports);
            body.push('\n');
            for method in &ty.methods {
                emit_method(method, &ty.name, &nif_module, &declared_errors, &mut body, &mut imports);
                body.push('\n');
            }
        }

        // Emit trait bridge shims for each configured bridge not excluded from Gleam.
        let active_bridges: Vec<&TraitBridgeConfig> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "gleam"))
            .collect();

        // Set of struct/enum type names that have a corresponding generated Gleam type.
        // Trait method signatures referring to any other Named type (e.g. excluded
        // internal types like `InternalDocument`) are substituted with `String`.
        let visible_type_names: std::collections::HashSet<&str> = api
            .types
            .iter()
            .filter(|t| !t.is_trait)
            .map(|t| t.name.as_str())
            .chain(api.enums.iter().map(|e| e.name.as_str()))
            .collect();
        let mut support_nifs_emitted = false;
        for bridge_cfg in &active_bridges {
            let trait_type = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name);
            emit_trait_bridge_shims(
                bridge_cfg,
                trait_type,
                &nif_module,
                &declared_errors,
                &visible_type_names,
                &mut body,
                &mut imports,
            );
            body.push('\n');

            if !support_nifs_emitted {
                emit_trait_support_nifs(&nif_module, &mut body);
                support_nifs_emitted = true;
            }
        }

        let mut content = String::new();
        content.push_str("// Generated by alef. Do not edit by hand.\n\n");
        for import in &imports {
            content.push_str(&crate::template_env::render(
                "import_line.jinja",
                minijinja::context! {
                    import => import,
                },
            ));
        }
        if !imports.is_empty() {
            content.push('\n');
        }
        content.push_str(&body);

        let dir = resolve_output_dir(None, &config.name, "packages/gleam/src");
        let path = PathBuf::from(dir).join(format!("{module_name}.gleam"));

        Ok(vec![GeneratedFile {
            path,
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "gleam",
            crate_suffix: "",
            build_dep: BuildDependency::Rustler,
            post_build: vec![],
        })
    }
}
