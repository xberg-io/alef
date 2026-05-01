use alef_codegen::builder::RustFileBuilder;
use alef_codegen::doc_emission;
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{AlefConfig, Language, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;
use std::borrow::Cow;
use std::path::PathBuf;

pub struct ExtendrBackend;

impl ExtendrBackend {
    fn binding_config(core_import: &str) -> RustBindingConfig<'_> {
        RustBindingConfig {
            struct_attrs: &[],
            field_attrs: &[],
            struct_derives: &["Clone"],
            method_block_attr: None,
            constructor_attr: "",
            static_attr: None,
            function_attr: "#[extendr]",
            enum_attrs: &[],
            enum_derives: &["Clone", "PartialEq"],
            needs_signature: false,
            signature_prefix: "",
            signature_suffix: "",
            core_import,
            async_pattern: AsyncPattern::TokioBlockOn,
            has_serde: true,
            type_name_prefix: "",
            option_duration_on_defaults: false,
            opaque_type_names: &[],
        }
    }
}

impl TypeMapper for ExtendrBackend {
    fn primitive(&self, prim: &alef_core::ir::PrimitiveType) -> Cow<'static, str> {
        use alef_core::ir::PrimitiveType;
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("bool"),
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => Cow::Borrowed("i32"),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                Cow::Borrowed("f64")
            }
            PrimitiveType::F32 | PrimitiveType::F64 => Cow::Borrowed("f64"),
        }
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}

impl Backend for ExtendrBackend {
    fn name(&self) -> &str {
        "extendr"
    }

    fn language(&self) -> Language {
        Language::R
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // R is single-threaded; async funcs are blocked on a per-call tokio runtime.
            supports_async: true,
            supports_classes: true,
            supports_enums: true,
            supports_option: true,
            supports_result: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let core_import = config.core_import();
        let cfg = Self::binding_config(&core_import);

        // Build adapter body map for method body substitution
        let adapter_bodies = alef_adapters::build_adapter_bodies(config, Language::R)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
        builder.add_import("extendr_api::prelude::*");

        // Import traits needed for trait method dispatch
        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

        // Custom module declarations
        let custom_mods = config.custom_modules.for_language(Language::R);
        for module in custom_mods {
            builder.add_item(&format!("pub mod {module};"));
        }

        let opaque_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque)
            .map(|t| t.name.clone())
            .collect();
        let mutex_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();

        // Import Arc when there are opaque types (builder-pattern types use Arc<CoreType>).
        if !opaque_types.is_empty() {
            builder.add_import("std::sync::Arc");
        }

        // Generate type bindings
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                // Opaque types wrap the core type in Arc<T> and delegate methods to self.inner.
                builder.add_item(&generators::gen_opaque_struct(typ, &cfg));
                let impl_block =
                    generators::gen_opaque_impl_block(typ, self, &cfg, &opaque_types, &mutex_types, &adapter_bodies);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
            } else {
                // gen_struct already emits #[derive(Default)] for all structs.
                // Emitting gen_struct_default_impl here would produce a conflicting
                // `impl Default` compile error. The derive covers all types where
                // can_generate_default_impl is true (all field types implement Default).
                builder.add_item(&generators::gen_struct(typ, self, &cfg));
                let impl_block = generators::gen_impl_block(typ, self, &cfg, &adapter_bodies, &opaque_types);
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                }
                // Generate config constructor if type has Default
                if typ.has_default && !typ.fields.is_empty() {
                    let map_fn = |ty: &alef_core::ir::TypeRef| self.map_type(ty);
                    let config_fn = alef_codegen::config_gen::gen_extendr_kwargs_constructor(typ, &map_fn);
                    builder.add_item(&config_fn);
                }
            }
        }

        // Generate enum bindings
        for e in &api.enums {
            builder.add_item(&generators::gen_enum(e, &cfg));
        }

        // Generate function bindings
        for func in &api.functions {
            let bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges);
            if let Some((param_idx, bridge_cfg)) = bridge_param {
                builder.add_item(&crate::trait_bridge::gen_bridge_function(
                    func,
                    param_idx,
                    bridge_cfg,
                    self,
                    &opaque_types,
                    &core_import,
                ));
            } else {
                builder.add_item(&generators::gen_function(
                    func,
                    self,
                    &cfg,
                    &adapter_bodies,
                    &opaque_types,
                ));
            }
        }

        // Trait bridge wrappers — generate extendr bridge structs that delegate to R list objects
        for bridge_cfg in &config.trait_bridges {
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                let bridge = crate::trait_bridge::gen_trait_bridge(
                    trait_type,
                    bridge_cfg,
                    &core_import,
                    &config.error_type(),
                    &config.error_constructor(),
                    api,
                );
                for imp in &bridge.imports {
                    builder.add_import(imp);
                }
                builder.add_item(&bridge.code);
            }
        }

        // Module registration
        let module_name = config.r_package_name().replace('-', "_");
        let module_items = format!(
            "extendr_module! {{\n    mod {module};\n{types}{funcs}}}\n",
            module = module_name,
            types = api
                .types
                .iter()
                .map(|t| format!("    impl {};\n", t.name))
                .collect::<String>(),
            funcs = api
                .functions
                .iter()
                .map(|f| format!("    fn {};\n", f.name))
                .collect::<String>(),
        );
        builder.add_item(&module_items);

        let output_path = resolve_output_dir(
            config.output.r.as_ref(),
            &config.crate_config.name,
            "packages/r/src/rust/src",
        );

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_path).join("lib.rs"),
            content: builder.build(),
            generated_header: false,
        }])
    }

    fn generate_public_api(&self, api: &ApiSurface, config: &AlefConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let package_name = config.r_package_name();
        let prefix = config.ffi_prefix();

        // Generate R namespace file with wrapper functions
        let mut content = hash::header(CommentStyle::Hash);
        content.push('\n');

        // Add useDynLib directive
        content.push_str(&format!("#' @useDynLib {}, .registration = TRUE\n", package_name));
        content.push_str("NULL\n\n");

        // Generate wrapper functions for all API functions
        for func in &api.functions {
            // Emit roxygen documentation
            doc_emission::emit_roxygen(&mut content, &func.doc);
            // Add @export tag for public functions
            content.push_str("#' @export\n");

            content.push_str(&format!("{} <- function(", func.name));

            // Parameters with default values
            let params: Vec<String> = func
                .params
                .iter()
                .map(|p| {
                    if p.optional {
                        format!("{} = NULL", p.name)
                    } else {
                        p.name.clone()
                    }
                })
                .collect();
            content.push_str(&params.join(", "));

            content.push_str(") {\n");

            // Call the native function
            let param_names: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();
            content.push_str(&format!(
                "  .Call(\"{}_{}\"{})\n",
                prefix,
                func.name,
                if param_names.is_empty() {
                    String::new()
                } else {
                    format!(", {}", param_names.join(", "))
                }
            ));
            content.push_str("}\n\n");
        }

        // The R wrapper file always goes into the package's R/ directory (e.g. packages/r/R/).
        // We derive this from the rust output path: strip the conventional Rust-source suffix
        // (src/rust/src) and append R/, falling back to the hardcoded default.
        let r_wrapper_dir = if let Some(rust_out) = config.output.r.as_ref() {
            let rust_str = rust_out.to_string_lossy();
            // Strip trailing separator variants of "src/rust/src"
            let suffixes = ["src/rust/src/", "src/rust/src"];
            let base = suffixes
                .iter()
                .find_map(|s| rust_str.strip_suffix(s))
                .unwrap_or_else(|| rust_str.as_ref());
            format!("{base}R/")
        } else {
            "packages/r/R/".to_string()
        };

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&r_wrapper_dir).join(format!("{}.R", package_name)),
            content,
            generated_header: false,
        }])
    }

    fn build_config(&self) -> Option<BuildConfig> {
        Some(BuildConfig {
            tool: "cargo",
            crate_suffix: "-extendr",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}
