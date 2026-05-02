use alef_codegen::builder::RustFileBuilder;
use alef_codegen::generators::{self, AsyncPattern, RustBindingConfig};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use alef_core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, TypeDef, TypeRef};
use std::borrow::Cow;
use std::fmt::Write as FmtWrite;
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

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let core_import = config.core_import_name();
        let cfg = Self::binding_config(&core_import);

        // Build adapter body map for method body substitution.
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

        // Emit binding↔core From impls so generated bodies can use `.into()` /
        // `Type::from(core)` to bridge between the extendr-facing binding types and
        // the core Rust types.  Without these impls the generated `convert` and
        // builder methods fail with E0277 unsatisfied trait bound errors.
        let binding_to_core = alef_codegen::conversions::convertible_types(api);
        let core_to_binding = alef_codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = alef_codegen::conversions::input_type_names(api);
        let extendr_conversion_cfg = alef_codegen::conversions::ConversionConfig::default();
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            // binding→core: emit when type is used as input and all fields are
            // convertible (mirrors pyo3/magnus emission paths).
            if input_types.contains(&typ.name)
                && alef_codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&alef_codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &extendr_conversion_cfg,
                ));
            }
            // core→binding: emit whenever the conversion can be generated.  Allows
            // `core_value.into()` in return positions.
            if alef_codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&alef_codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &extendr_conversion_cfg,
                ));
            }
        }
        for e in &api.enums {
            // Extendr emits enums as flat (unit-only) variants regardless of whether the
            // core enum has data — emit lossy From impls so containing structs can call
            // `.into()`.  Data is discarded across the boundary; the binding enum keeps
            // only the variant tag.
            if input_types.contains(&e.name) && alef_codegen::conversions::can_generate_enum_conversion(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_binding_to_core(
                    e,
                    &core_import,
                ));
            }
            if alef_codegen::conversions::can_generate_enum_conversion_from_core(e) {
                builder.add_item(&alef_codegen::conversions::gen_enum_from_core_to_binding(
                    e,
                    &core_import,
                ));
            }
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
                    &config.error_type_name(),
                    &config.error_constructor_expr(),
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

        let output_path = resolve_output_dir(config.output_paths.get("r"), &config.name, "packages/r/src/rust/src");

        Ok(vec![GeneratedFile {
            path: PathBuf::from(&output_path).join("lib.rs"),
            content: builder.build(),
            generated_header: false,
        }])
    }

    fn generate_public_api(
        &self,
        api: &ApiSurface,
        config: &ResolvedCrateConfig,
    ) -> anyhow::Result<Vec<GeneratedFile>> {
        let package_name = config.r_package_name();

        // The R wrapper file always goes into the package's R/ directory (e.g. packages/r/R/).
        // We derive this from the rust output path: strip the conventional Rust-source suffix
        // (src/rust/src) and append R/, falling back to the hardcoded default.
        let r_wrapper_dir = if let Some(rust_out) = config.output_paths.get("r") {
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

        let mut files = Vec::new();

        // {package_name}.R: only @useDynLib — no function wrappers.
        // extendr auto-generates extendr-wrappers.R at build time with the correct
        // `wrap__<funcname>` symbol references. Emitting a second convert() here would
        // create a duplicate export and call the wrong (non-existent) symbol.
        let mut pkg_content = hash::header(CommentStyle::Hash);
        pkg_content.push('\n');
        pkg_content.push_str(&format!("#' @useDynLib {package_name}, .registration = TRUE\n"));
        pkg_content.push_str("NULL\n");
        files.push(GeneratedFile {
            path: PathBuf::from(&r_wrapper_dir).join(format!("{package_name}.R")),
            content: pkg_content,
            generated_header: false,
        });

        // options.R: generated from ConversionOptions IR so all fields are present.
        if let Some(opts_type) = api.types.iter().find(|t| t.name == "ConversionOptions" && !t.is_trait) {
            let options_r = gen_conversion_options_r(opts_type);
            files.push(GeneratedFile {
                path: PathBuf::from(&r_wrapper_dir).join("options.R"),
                content: options_r,
                generated_header: true,
            });
        }

        Ok(files)
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

/// Generate the `options.R` file for the R package from the `ConversionOptions` IR type.
///
/// Produces a roxygen-documented `conversion_options()` helper function with one parameter per
/// field (all defaulting to `NULL`). R callers use named arguments to override individual
/// settings; unset parameters remain `NULL` and are omitted from the resulting list so that the
/// Rust side applies its own defaults.
fn gen_conversion_options_r(opts_type: &TypeDef) -> String {
    use alef_core::ir::PrimitiveType;

    let mut out = String::with_capacity(2048);

    // Function-level roxygen header
    writeln!(
        out,
        "#' Create a ConversionOptions list for HTML-to-Markdown conversion"
    )
    .ok();
    writeln!(out, "#'").ok();
    writeln!(
        out,
        "#' All parameters default to `NULL`, which means the Rust default is used."
    )
    .ok();
    writeln!(out, "#' Pass named arguments to override individual settings.").ok();
    writeln!(out, "#'").ok();

    // Per-field roxygen @param lines. Strip leading underscore from names —
    // Rust uses `_ctx` etc. for unused default-impl params; R callers write without the prefix.
    for field in &opts_type.fields {
        let rname = field.name.trim_start_matches('_');
        let doc_text = if field.doc.is_empty() {
            rname.to_string()
        } else {
            let first = field.doc.lines().next().unwrap_or(rname);
            first.trim_end_matches('.').to_string()
        };
        if field.cfg.is_some() {
            writeln!(out, "#' @param {rname} (feature-gated) {doc_text}").ok();
        } else {
            writeln!(out, "#' @param {rname} {doc_text}").ok();
        }
    }

    writeln!(
        out,
        "#' @return A named list suitable for the `options` argument of [convert()]."
    )
    .ok();
    writeln!(out, "#' @export").ok();

    // Function signature: all fields as NULL-defaulted params
    let params: Vec<String> = opts_type
        .fields
        .iter()
        .map(|f| format!("{} = NULL", f.name.trim_start_matches('_')))
        .collect();
    writeln!(out, "conversion_options <- function(").ok();
    for (i, param) in params.iter().enumerate() {
        if i + 1 < params.len() {
            writeln!(out, "  {param},").ok();
        } else {
            writeln!(out, "  {param}").ok();
        }
    }
    writeln!(out, ") {{").ok();

    // Body: collect non-NULL values into a list
    writeln!(out, "  opts <- list()").ok();
    for field in &opts_type.fields {
        let rname = field.name.trim_start_matches('_');
        // Integer coercion for numeric fields that map to Rust integer types
        let needs_int = matches!(
            &field.ty,
            TypeRef::Primitive(PrimitiveType::U8)
                | TypeRef::Primitive(PrimitiveType::U16)
                | TypeRef::Primitive(PrimitiveType::U32)
                | TypeRef::Primitive(PrimitiveType::U64)
                | TypeRef::Primitive(PrimitiveType::I8)
                | TypeRef::Primitive(PrimitiveType::I16)
                | TypeRef::Primitive(PrimitiveType::I32)
                | TypeRef::Primitive(PrimitiveType::I64)
                | TypeRef::Primitive(PrimitiveType::Usize)
        );
        let assign_val = if needs_int {
            format!("as.integer({rname})")
        } else {
            rname.to_string()
        };
        writeln!(out, "  if (!is.null({rname})) opts${rname} <- {assign_val}").ok();
    }
    writeln!(out, "  opts").ok();
    writeln!(out, "}}").ok();

    out
}

#[cfg(test)]
mod tests {
    use super::ExtendrBackend;
    use alef_core::backend::Backend;
    use alef_core::config::ResolvedCrateConfig;
    use alef_core::config::new_config::NewAlefConfig;
    use alef_core::ir::*;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"
"#,
        )
    }

    fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    fn make_api_surface() -> ApiSurface {
        ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "Config".to_string(),
                rust_path: "test_lib::Config".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("timeout", TypeRef::Primitive(PrimitiveType::U32), false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: false,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: false,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
            }],
            functions: vec![FunctionDef {
                name: "process".to_string(),
                rust_path: "test_lib::process".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: String::new(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
            }],
            enums: vec![],
            errors: vec![],
        }
    }

    #[test]
    fn generates_extendr_module_registration() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        assert_eq!(files.len(), 1);
        let content = &files[0].content;
        assert!(content.contains("extendr_module!"), "must emit extendr_module! macro");
        assert!(content.contains("mod testlib"), "module name must match r_package_name");
    }

    #[test]
    fn generates_extendr_function_attribute() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        let content = &files[0].content;
        assert!(
            content.contains("#[extendr]"),
            "functions must carry #[extendr] attribute"
        );
        assert!(content.contains("fn process"), "process function must be generated");
    }

    #[test]
    fn r_package_name_drives_output_path() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        // Output should go to packages/r/src/rust/src/lib.rs (default path)
        assert!(
            files[0].path.to_string_lossy().ends_with("lib.rs"),
            "output file must be lib.rs"
        );
    }

    #[test]
    fn generate_public_api_uses_r_package_name() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        assert_eq!(files.len(), 1);
        let path_str = files[0].path.to_string_lossy();
        assert!(
            path_str.ends_with("testlib.R"),
            "public API file must be {{package_name}}.R"
        );
    }
}
