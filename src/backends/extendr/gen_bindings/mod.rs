mod enum_conversions;
pub mod service_api;

use crate::codegen::builder::RustFileBuilder;
use crate::codegen::doc_emission::{parse_arguments_bullets, parse_rustdoc_sections};
use crate::codegen::generators::trait_bridge::find_bridge_field;
use crate::codegen::generators::{self, AsyncPattern, RustBindingConfig};
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier, wire_variant_value};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use ahash::AHashSet;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ExtendrBackend;

impl ExtendrBackend {
    fn binding_config<'a>(core_import: &'a str, lossy_skip_types: &'a [String]) -> RustBindingConfig<'a> {
        RustBindingConfig {
            struct_attrs: &[],
            field_attrs: &[],
            struct_derives: &["Clone"],
            // #[extendr] on impl blocks registers the struct as an R class, which enables
            // the ToVectorValue trait bound required for returning struct types from #[extendr]
            // free functions and for extendr_module! `impl Type;` declarations.
            method_block_attr: Some("extendr"),
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
            // The extendr backend uses a separate #[extendr] free-function kwargs constructor
            // (gen_extendr_kwargs_constructor) for R callers. The in-class impl-block `fn new()`
            // constructor is suppressed to avoid type conversion errors: extendr cannot convert
            // custom enum or struct parameters from Robj in the classic constructor signature.
            skip_impl_constructor: true,
            // R maps small ints (u8, u16, u32, i8, i16) to i32 and large ints (u64, usize,
            // isize) to f64. Cast these back to the core types in gen_lossy_binding_to_core_fields
            // so that method bodies that construct core structs compile correctly.
            cast_uints_to_i32: true,
            cast_large_ints_to_f64: true,
            // extendr's #[extendr] macro generates TryFrom<&Robj> for T (owned types only).
            // Free function parameters that are Named non-opaque structs must be declared as T
            // in the binding signature. The body uses .clone().into() to convert owned params.
            named_non_opaque_params_by_ref: false,
            // Flat data enums are output-only: no From<BindingType> impl exists for them.
            // Skip these types in gen_lossy_binding_to_core_fields so method bodies that
            // construct core structs emit Default::default() for those fields instead of
            // attempting .clone().into() which would fail to compile.
            lossy_skip_types,
            serializable_opaque_type_names: &[],
            never_skip_cfg_field_names: &[],
            emit_delegating_default_impl: false,
            // extendr's `#[extendr]` macro fails to expand impl blocks that contain
            // `compile_error!` bodies, breaking the whole binding crate. Skip
            // non-auto-delegatable methods (e.g. those whose signature contains
            // Vec<EnumVariant> or Result<Vec<NamedStruct>> shapes the extendr_api
            // converters do not implement) rather than emitting an unimplemented stub.
            skip_methods_when_not_delegatable: true,
        }
    }
}

impl TypeMapper for ExtendrBackend {
    fn primitive(&self, prim: &crate::core::ir::PrimitiveType) -> Cow<'static, str> {
        use crate::core::ir::PrimitiveType;
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
            supports_service_api: true,
            ..Capabilities::default()
        }
    }

    fn generate_bindings(&self, api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
        let core_import = config.core_import_name();
        // Compute flat data enum names first so binding_config and conversion config can use them.
        let flat_data_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| is_flat_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        // JSON-passthrough wrapper structs need an `impl Name;` entry in
        // `extendr_module!` so their `#[extendr] impl` block (with `default`,
        // `from_json`) is wired into the R package's class registry.
        let json_passthrough_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| is_json_passthrough_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        let cfg = Self::binding_config(&core_import, &flat_data_enum_names_vec);

        // Build adapter body map for method body substitution.
        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::R)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
        builder.add_import("extendr_api::prelude::*");
        // HashMap is needed for fields of type HashMap<K, V> (extendr prelude does not re-export it)
        builder.add_import("std::collections::HashMap");
        // Use extendr's Result<T> type alias (Result<T, Error>) in generated #[extendr] functions
        builder.add_import("extendr_api::Result");

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
        // Bridge handle aliases are synthesized through the configured trait bridge, not as
        // ordinary extendr classes. Skip those opaque aliases and any fields/methods that expose
        // them directly; other cfg-gated opaque types remain normal Arc-backed handles.
        let arc_incompatible_opaque: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| {
                t.is_opaque
                    && crate::codegen::generators::trait_bridge::is_bridge_handle_type_ref(
                        &TypeRef::Named(t.name.clone()),
                        &config.trait_bridges,
                    )
            })
            .map(|t| t.name.clone())
            .collect();
        let arc_incompatible_opaque_vec: Vec<String> = arc_incompatible_opaque.iter().cloned().collect();
        let mutex_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| t.is_opaque && generators::type_needs_mutex(t))
            .map(|t| t.name.clone())
            .collect();
        let enum_names: ahash::AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

        // Helper: returns true if a TypeRef references any arc-incompatible opaque type.
        let references_arc_incompatible = |ty: &crate::core::ir::TypeRef| -> bool {
            arc_incompatible_opaque_vec.iter().any(|n| {
                matches!(ty, crate::core::ir::TypeRef::Named(name) if name == n)
                    || matches!(ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(name) if name == n))
            })
        };
        // Helper: returns true if a method references any arc-incompatible opaque type in params or return.
        let method_references_arc_incompatible = |m: &crate::core::ir::MethodDef| -> bool {
            references_arc_incompatible(&m.return_type) || m.params.iter().any(|p| references_arc_incompatible(&p.ty))
        };

        // Helper: returns true if a TypeRef is (or contains) an enum type.
        let references_enum = |ty: &crate::core::ir::TypeRef| -> bool {
            match ty {
                crate::core::ir::TypeRef::Named(n) => enum_names.contains(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n) if enum_names.contains(n.as_str()))
                }
                _ => false,
            }
        };

        // Helper: returns true if a TypeRef references a non-opaque struct type by value.
        // Extendr generates TryFrom<&Robj> for &T (reference) but NOT TryFrom<&Robj> for T
        // (owned). Method parameters that take non-opaque structs by value therefore cannot
        // be converted from an incoming Robj. Such parameters trigger "T: TryFrom<&Robj> not
        // satisfied" compile errors. We exclude methods with such parameters so they are
        // omitted from the #[extendr] impl block.
        let param_is_owned_struct = |ty: &crate::core::ir::TypeRef| -> bool {
            let is_non_opaque_struct =
                |n: &str| !opaque_types.contains(n) && !enum_names.contains(n) && !arc_incompatible_opaque.contains(n);
            match ty {
                crate::core::ir::TypeRef::Named(n) => is_non_opaque_struct(n),
                crate::core::ir::TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n) if is_non_opaque_struct(n))
                }
                _ => false,
            }
        };

        // A method is incompatible with extendr if:
        //   • it references arc-incompatible types (arc-wrapped, Rc-based), OR
        //   • its return type is an enum (enums don't implement ToVectorValue), OR
        //   • any parameter takes a non-opaque struct by value (TryFrom<&Robj> for owned T
        //     is not generated by #[extendr] on the struct definition).
        let method_references_enum = |m: &crate::core::ir::MethodDef| -> bool {
            references_enum(&m.return_type)
                || m.params
                    .iter()
                    .any(|p| references_enum(&p.ty) || param_is_owned_struct(&p.ty))
        };

        // Helper: returns true if a TypeRef is (or contains) a Map type.
        // Extendr cannot marshal HashMap/BTreeMap directly (`HashMap<K, V>: ToVectorValue`
        // is not implemented). Methods returning or accepting Map types are excluded from
        // the impl block; callers access map data via the struct serialisation path
        // (R named list) instead.
        let references_map = |ty: &crate::core::ir::TypeRef| -> bool {
            match ty {
                crate::core::ir::TypeRef::Map(_, _) => true,
                crate::core::ir::TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Map(_, _))
                }
                _ => false,
            }
        };
        let method_references_map = |m: &crate::core::ir::MethodDef| -> bool {
            references_map(&m.return_type) || m.params.iter().any(|p| references_map(&p.ty))
        };

        // Helper: returns true if a TypeRef requires a type that extendr cannot automatically
        // convert from/to Robj.
        //
        // Extendr CANNOT handle:
        //   • Vec<T> where T is a non-opaque, non-enum, non-arc-incompatible struct —
        //     there is no automatic "R list → Vec<ExternalPtr<T>>" conversion.
        //   • Option<Vec<T>> of the same category.
        //
        // Extendr CAN handle:
        //   • Bare T (non-opaque struct) — wrapped/unwrapped via ExternalPtr.
        //   • Option<T> (non-opaque struct) — same, with NULL for None.
        //   • Vec<primitive/String> — native R vector types.
        //   • Enum types — ExternalPtr-backed after #[extendr] on enum.
        //   • Opaque types (Arc<T> wrappers) — ExternalPtr-backed.
        let is_extendr_native_incompatible = |ty: &crate::core::ir::TypeRef| -> bool {
            // A Vec<T> element type is "struct-like" (non-opaque, non-arc-incompatible)
            // Includes both structs AND enums — extendr can't auto-convert either from R lists
            let is_vec_element_incompatible =
                |n: &str| !opaque_types.contains(n) && !arc_incompatible_opaque.contains(n);
            match ty {
                // Vec<StructType> or Vec<Enum> — cannot convert from R list automatically.
                crate::core::ir::TypeRef::Vec(inner) => {
                    match inner.as_ref() {
                        // Vec<StructType> or Vec<Enum> — incompatible
                        crate::core::ir::TypeRef::Named(n) if is_vec_element_incompatible(n) => true,
                        // Vec<Vec<_>> — nested vectors not supported by extendr
                        crate::core::ir::TypeRef::Vec(_) => true,
                        _ => false,
                    }
                }
                // Option<Vec<StructType>> or Option<Vec<Enum>> — same.
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Vec(inner2) = inner.as_ref() {
                        match inner2.as_ref() {
                            // Option<Vec<StructType>> or Option<Vec<Enum>> — incompatible
                            crate::core::ir::TypeRef::Named(n) if is_vec_element_incompatible(n) => true,
                            // Option<Vec<Vec<_>>> — nested vectors not supported by extendr
                            crate::core::ir::TypeRef::Vec(_) => true,
                            _ => false,
                        }
                    } else {
                        false
                    }
                }
                // Bare Named types and Option<Named> are ExternalPtr-backed — compatible.
                _ => false,
            }
        };

        // Types that cannot be registered as extendr classes because their fields use types
        // that extendr cannot convert (Vec<T> where T is a non-opaque non-enum struct, etc.).
        // These types are still generated as Rust structs (for From impls), but are excluded
        // from #[extendr] impl block generation and from extendr_module! registration.
        let extendr_incompatible_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_opaque && !t.is_trait)
            .filter(|t| t.fields.iter().any(|f| is_extendr_native_incompatible(&f.ty)))
            .map(|t| t.name.clone())
            .collect();

        // Types that appear as function/method parameters — these are "input types" that
        // callers construct. Only generate from_json for input types whose core counterparts
        // implement serde::Deserialize. Output-only types (result/metadata structs) are excluded
        // so we avoid generating from_json for types whose core doesn't impl Deserialize.
        let input_type_names: ahash::AHashSet<String> = {
            fn collect_named(ty: &crate::core::ir::TypeRef, set: &mut ahash::AHashSet<String>) {
                match ty {
                    crate::core::ir::TypeRef::Named(n) => {
                        set.insert(n.clone());
                    }
                    crate::core::ir::TypeRef::Optional(inner) => collect_named(inner, set),
                    crate::core::ir::TypeRef::Vec(inner) => collect_named(inner, set),
                    _ => {}
                }
            }
            let mut set = ahash::AHashSet::new();
            for func in &api.functions {
                for p in &func.params {
                    collect_named(&p.ty, &mut set);
                }
            }
            for typ in &api.types {
                for m in &typ.methods {
                    for p in &m.params {
                        collect_named(&p.ty, &mut set);
                    }
                }
            }
            set
        };

        // Import Arc when there are arc-compatible opaque types.
        let has_arc_compatible = opaque_types.iter().any(|n| !arc_incompatible_opaque.contains(n));
        if has_arc_compatible {
            builder.add_import("std::sync::Arc");
        }

        // Generate type bindings
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                // Skip opaque types that cannot be wrapped in Arc (e.g. VisitorHandle with Rc).
                if arc_incompatible_opaque.contains(&typ.name) {
                    continue;
                }
                // Opaque types wrap the core type in Arc<T> and delegate methods to self.inner.
                // Applying #[extendr] to the struct generates TryFrom<&Robj> for &Foo and
                // From<Foo> for Robj via ExternalPtr — required for method dispatch.
                let opaque_struct = generators::gen_opaque_struct(typ, &cfg);
                builder.add_item(&format!("#[extendr]\n{opaque_struct}"));
                // Filter methods on opaque types that cannot be expressed in the extendr binding:
                //   • methods referencing arc-incompatible types (e.g. visitor() with VisitorHandle)
                //   • methods taking/returning enum types (ToVectorValue not implemented for enums)
                //   • methods taking non-opaque structs by value (TryFrom<&Robj> for owned T missing)
                let has_excluded_opaque_methods = typ.methods.iter().any(|m| {
                    method_references_arc_incompatible(m) || method_references_enum(m) || method_references_map(m)
                });
                let opaque_impl_typ: std::borrow::Cow<crate::core::ir::TypeDef> = if has_excluded_opaque_methods {
                    let filtered = crate::core::ir::TypeDef {
                        methods: typ
                            .methods
                            .iter()
                            .filter(|m| {
                                !method_references_arc_incompatible(m)
                                    && !method_references_enum(m)
                                    && !method_references_map(m)
                            })
                            .cloned()
                            .collect(),
                        ..typ.clone()
                    };
                    std::borrow::Cow::Owned(filtered)
                } else {
                    std::borrow::Cow::Borrowed(typ)
                };
                let impl_block = generators::gen_opaque_impl_block(
                    &opaque_impl_typ,
                    self,
                    &cfg,
                    &opaque_types,
                    &mutex_types,
                    &adapter_bodies,
                );
                if !impl_block.is_empty() {
                    builder.add_item(&impl_block);
                } else {
                    // extendr requires a #[extendr] impl block for every type listed in
                    // extendr_module! — even opaque types with no methods need one so the
                    // meta__TypeName function is generated by the #[extendr] proc-macro.
                    builder.add_item(&format!("#[extendr]\nimpl {} {{}}", typ.name));
                }
            } else {
                // If this type has fields referencing arc-incompatible opaque types, generate the
                // struct with those fields removed. They are skipped in the binding because the
                // opaque wrapper struct (e.g. VisitorHandle) was not generated.
                // Also filter methods that take or return enum types — extendr enums don't
                // automatically implement ToVectorValue so they cannot be used as extendr
                // method params/return types in non-opaque impl blocks.
                let has_excluded_fields = typ.fields.iter().any(|f| references_arc_incompatible(&f.ty));
                let has_excluded_methods = typ.methods.iter().any(|m| {
                    method_references_arc_incompatible(m) || method_references_enum(m) || method_references_map(m)
                });
                let struct_typ: std::borrow::Cow<crate::core::ir::TypeDef> =
                    if has_excluded_fields || has_excluded_methods {
                        let filtered = crate::core::ir::TypeDef {
                            fields: typ
                                .fields
                                .iter()
                                .filter(|f| !references_arc_incompatible(&f.ty))
                                .cloned()
                                .collect(),
                            methods: typ
                                .methods
                                .iter()
                                .filter(|m| {
                                    !method_references_arc_incompatible(m)
                                        && !method_references_enum(m)
                                        && !method_references_map(m)
                                })
                                .cloned()
                                .collect(),
                            ..typ.clone()
                        };
                        std::borrow::Cow::Owned(filtered)
                    } else {
                        std::borrow::Cow::Borrowed(typ)
                    };

                // gen_struct already emits #[derive(Default)] for all structs.
                // Emitting gen_struct_default_impl here would produce a conflicting
                // `impl Default` compile error. The derive covers all types where
                // can_generate_default_impl is true (all field types implement Default).

                if extendr_incompatible_types.contains(&struct_typ.name) {
                    // This type has fields (e.g. Vec<HeaderMetadata>) that extendr cannot
                    // convert from/to Robj. Do NOT register it as an extendr class — the
                    // #[extendr] impl and extendr_module! entry are omitted. The struct is
                    // still generated so From impls compile. Callers receive these types
                    // serialised to R named lists by the custom types.rs module.
                    builder.add_item(&generators::gen_struct(&struct_typ, self, &cfg));
                } else {
                    // Applying #[extendr] to the struct generates the conversions needed
                    // for extendr class registration:
                    //   • impl TryFrom<&Robj> for &Foo  (ExternalPtr unwrap)
                    //   • impl TryFrom<&mut Robj> for &mut Foo
                    //   • impl From<Foo> for Robj        (ExternalPtr wrap)
                    // Without this the free-function kwargs constructors cannot return Foo
                    // (Robj::from requires From<Foo> for Robj) and method &self params
                    // cannot be extracted from the incoming Robj.
                    let struct_item = generators::gen_struct(&struct_typ, self, &cfg);
                    builder.add_item(&format!("#[extendr]\n{struct_item}"));
                    // The impl block uses the full struct_typ (with real fields) so that method
                    // bodies (e.g. gen_lossy_binding_to_core_fields) emit correct field
                    // assignments. Constructor generation is suppressed via skip_impl_constructor
                    // in the binding config — the kwargs free-function constructor handles R
                    // object creation instead.
                    // Build from_json method body when the type has serde + has_default.
                    // from_json lets R callers construct typed ExternalPtrs from JSON strings
                    // (e.g. ExtractionConfig$from_json(jsonlite::toJSON(list(...)))).
                    // Generate from_json only for input types (function/method parameters)
                    // whose core counterparts implement serde::Deserialize. Output-only types
                    // (metadata/result structs) are excluded: their core types may not impl
                    // Deserialize and callers never need to construct them from JSON.
                    let from_json_method = if struct_typ.has_default
                        && !struct_typ.fields.is_empty()
                        && input_type_names.contains(&struct_typ.name)
                    {
                        let type_name = &struct_typ.name;
                        let core_path = struct_typ.rust_path.replace('-', "_");
                        // Deserialize via the core type so its serde setup (enum string variants,
                        // #[serde(default)] on all fields, etc.) applies. The binding has
                        // From<CoreType> generated by alef so conversion is always available.
                        format!(
                            "    pub fn from_json(json: String) -> extendr_api::Result<{type_name}> {{\n        \
                             let core: {core_path} = serde_json::from_str(&json)\n            \
                             .map_err(|e| extendr_api::Error::Other(e.to_string()))?;\n        \
                             Ok(core.into())\n    \
                             }}\n"
                        )
                    } else {
                        String::new()
                    };

                    let impl_block =
                        generators::gen_impl_block(&struct_typ, self, &cfg, &adapter_bodies, &opaque_types);
                    if !impl_block.is_empty() {
                        // Inject from_json (if any) into the existing #[extendr] impl block
                        // before the closing `}`. extendr only allows one #[extendr] impl per type.
                        let final_impl = if from_json_method.is_empty() {
                            impl_block
                        } else if let Some(pos) = impl_block.rfind('}') {
                            format!("{}{}{}", &impl_block[..pos], &from_json_method, &impl_block[pos..])
                        } else {
                            impl_block
                        };
                        builder.add_item(&final_impl);
                    } else {
                        // extendr requires a #[extendr] impl block for every type listed in
                        // extendr_module! — without it, the module macro cannot register the type.
                        let empty_or_from_json = if from_json_method.is_empty() {
                            format!("#[extendr]\nimpl {} {{}}", struct_typ.name)
                        } else {
                            format!("#[extendr]\nimpl {} {{\n{}}}", struct_typ.name, from_json_method)
                        };
                        builder.add_item(&empty_or_from_json);
                    }
                    // Generate kwargs config constructor if type has Default.
                    if struct_typ.has_default && !struct_typ.fields.is_empty() {
                        let map_fn = |ty: &crate::core::ir::TypeRef| self.map_type(ty);
                        let config_fn = crate::codegen::config_gen::gen_extendr_kwargs_constructor(
                            &struct_typ,
                            &map_fn,
                            &enum_names,
                        );
                        builder.add_item(&config_fn);
                    }
                }
            }
        }

        // Generate enum bindings.
        for e in &api.enums {
            if is_flat_data_enum(e) {
                // Data enums with all-tuple variants become flat structs so bindings can
                // access variant data with dot notation (e.g. result$format$excel$sheet_count).
                // The #[extendr] attribute registers it as a class; the impl block satisfies
                // extendr_module! even though the struct has no methods.
                let flat_struct = gen_extendr_flat_data_enum_struct(e, self, &cfg);
                builder.add_item(&format!("#[extendr]\n{flat_struct}"));
                builder.add_item(&format!("#[extendr]\nimpl {} {{}}", e.name));
            } else if is_json_passthrough_data_enum(e) {
                // Tagged data enums with struct variants (e.g. `EmbeddingModelType`
                // with `Preset { name: String }`) cannot be expressed losslessly as
                // flat unit-only enums — the variant payload is dropped on round
                // trip. Generate a newtype struct whose serde representation defers
                // entirely to the core enum, so nested deserialization through parent
                // structs preserves the full tagged payload across the FFI boundary.
                builder.add_item(&gen_extendr_json_passthrough_enum_struct(e, &core_import));
            } else {
                builder.add_item(&generators::gen_enum(e, &cfg));
            }
        }

        // Emit binding↔core From impls so generated bodies can use `.into()` /
        // `Type::from(core)` to bridge between the extendr-facing binding types and
        // the core Rust types.  Without these impls the generated `convert` and
        // builder methods fail with E0277 unsatisfied trait bound errors.
        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api);
        let input_types = crate::codegen::conversions::input_type_names(api);

        // Collect all types that appear in the R binding surface: parameters, return types,
        // and fields. This ensures conversion impls are emitted for every type referenced
        // from an exported function or method, even if it only appears in return types.
        let mut all_surface_types: AHashSet<String> = input_types.clone();

        // Add types from function return types
        for func in &api.functions {
            collect_named_types_into(&func.return_type, &mut all_surface_types);
        }

        // Add types from method return types
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                collect_named_types_into(&method.return_type, &mut all_surface_types);
            }
        }

        // Transitive closure: add all types referenced by surface types
        let mut changed = true;
        while changed {
            changed = false;
            let snapshot: Vec<String> = all_surface_types.iter().cloned().collect();
            for name in &snapshot {
                if let Some(typ) = api.types.iter().find(|t| t.name == *name) {
                    for field in &typ.fields {
                        let mut field_types = AHashSet::new();
                        collect_named_types_into(&field.ty, &mut field_types);
                        for field_type in field_types {
                            if all_surface_types.insert(field_type) {
                                changed = true;
                            }
                        }
                    }
                }
                if let Some(e) = api.enums.iter().find(|e| e.name == *name) {
                    for variant in &e.variants {
                        for field in &variant.fields {
                            let mut field_types = AHashSet::new();
                            collect_named_types_into(&field.ty, &mut field_types);
                            for field_type in field_types {
                                if all_surface_types.insert(field_type) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Flat data enums whose tuple variant data types are all primitive/String can round-trip.
        // Those that have complex output-only types (e.g. FormatMetadata → DocxMetadata) cannot.
        let non_round_trip_flat_enums: Vec<String> = api
            .enums
            .iter()
            .filter(|e| is_flat_data_enum(e) && !can_flat_data_enum_round_trip(e))
            .map(|e| e.name.clone())
            .collect();
        let extendr_conversion_cfg = crate::codegen::conversions::ConversionConfig {
            cast_uints_to_i32: true,
            cast_large_ints_to_f64: true,
            // Exclude arc-incompatible opaque types (e.g. VisitorHandle) from conversion
            // generation so that struct fields referencing them are skipped in From impls.
            exclude_types: &arc_incompatible_opaque_vec,
            // Only skip flat data enums that are output-only (complex variant data types).
            // Round-trip-safe ones (e.g. OutputFormat with only String data) have a
            // From<BindingStruct> for CoreEnum impl generated and don't need skipping.
            from_binding_skip_types: &non_round_trip_flat_enums,
            // The extendr binding crate doesn't carry sample_core feature flags into its
            // own Cargo.toml, so cfg-gated core fields are dropped from the binding struct
            // (see `gen_struct` skip rule).  Mirror that in conversions: skip cfg-gated
            // fields and let `..Default::default()` pad the core struct slot.
            strip_cfg_fields_from_binding_struct: true,
            ..crate::codegen::conversions::ConversionConfig::default()
        };
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            // binding→core: emit when type is used as input and all fields are
            // convertible (mirrors pyo3/magnus emission paths).
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &extendr_conversion_cfg,
                ));
            }
            // core→binding: emit whenever the conversion can be generated.  Allows
            // `core_value.into()` in return positions.
            if crate::codegen::conversions::can_generate_conversion(typ, &core_to_binding) {
                builder.add_item(&crate::codegen::conversions::gen_from_core_to_binding_cfg(
                    typ,
                    &core_import,
                    &opaque_types,
                    &extendr_conversion_cfg,
                ));
            }
        }
        for e in &api.enums {
            if is_flat_data_enum(e) {
                // Flat data enum: always generate From<core> impl so containing structs can
                // convert core values. Struct variant data is lost (extendr can only represent
                // tuple variants as fields), but this is acceptable for output-only types like
                // FormatMetadata or policy enums like VlmFallbackPolicy.
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    // Generate dedicated From<core::Enum> impl.
                    builder.add_item(&gen_extendr_flat_data_enum_from_core(e, &core_import));
                    // Also generate the reverse for flat data enums whose tuple variant fields are
                    // all primitive/String types (so binding→core round-trip works).
                    // Output-only enums like FormatMetadata have complex output-only variant types
                    // (PdfMetadata, DocxMetadata, ...) and are excluded.
                    if can_flat_data_enum_round_trip(e) {
                        builder.add_item(&gen_extendr_flat_data_enum_to_core(e, &core_import));
                    }
                }
                // binding→core is only generated for round-trip-safe flat data enums above.
            } else if is_json_passthrough_data_enum(e) {
                // JSON-passthrough wrapper struct already emits its own `From<core>`
                // and `From<binding>` impls in `gen_extendr_json_passthrough_enum_struct`.
                // Skip the generic enum conversion which would emit a lossy unit-variant
                // mapping that conflicts with the wrapper struct definition.
                continue;
            } else {
                // Extendr emits enums as flat (unit-only) variants regardless of whether the
                // core enum has data — emit lossy From impls so containing structs can call
                // `.into()`.  Data is discarded across the boundary; the binding enum keeps
                // only the variant tag.
                // Emit binding→core for any enum in the surface (not just input params).
                if all_surface_types.contains(&e.name) && crate::codegen::conversions::can_generate_enum_conversion(e) {
                    builder.add_item(&enum_conversions::gen_from_binding_to_core(e, &core_import));
                }
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&enum_conversions::gen_from_core_to_binding(e, &core_import));
                }
            }
        }

        // Collect non-excluded bridges for function param matching.
        let active_bridges: Vec<_> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "r" || l == "extendr"))
            .cloned()
            .collect();

        // Names emitted by the trait-bridge generator; skip them in the free-function
        // pass below to avoid duplicate `#[extendr]` definitions (Rust E0428).
        let bridge_fn_names = collect_trait_bridge_fn_names(config);

        // Functions to exclude from R binding generation.
        let r_exclude_functions: ahash::AHashSet<String> = config
            .r
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        // Generate function bindings
        for func in &api.functions {
            if bridge_fn_names.contains(&func.name) {
                continue;
            }
            if r_exclude_functions.contains(&func.name) {
                continue;
            }
            let bridge_param = crate::backends::extendr::trait_bridge::find_bridge_param(func, &active_bridges);
            let bridge_field = find_bridge_field(func, &api.types, &active_bridges);

            if let Some((param_idx, bridge_cfg)) = bridge_param {
                builder.add_item(&crate::backends::extendr::trait_bridge::gen_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    self,
                    &opaque_types,
                    &core_import,
                ));
            } else if let Some(bm) = bridge_field {
                // Function has a bridge field binding (e.g., visitor on options)
                builder.add_item(&gen_extendr_bridge_field_function(api, func, &bm, &core_import));
            } else {
                // Detect functions whose return type or parameter types are incompatible
                // with extendr's automatic Robj conversions. These need JSON bridging.
                let func_return_needs_json = return_type_needs_json(
                    &func.return_type,
                    &extendr_incompatible_types,
                    &enum_names,
                    &opaque_types,
                );
                let func_params_need_json = func.params.iter().any(|p| {
                    is_extendr_native_incompatible(&p.ty)
                        || matches!(&p.ty, crate::core::ir::TypeRef::Named(n)
                            if extendr_incompatible_types.contains(n.as_str()) || enum_names.contains(n.as_str()))
                        || matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
                            if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n)
                                if enum_names.contains(n.as_str())))
                        // Bare non-opaque structs (not enums, not extendr_incompatible, but still not opaque)
                        // need JSON bridging because extendr can't auto-convert them from Robj
                        || matches!(&p.ty, crate::core::ir::TypeRef::Named(n)
                            if !opaque_types.contains(n.as_str())
                                && !enum_names.contains(n.as_str())
                                && !extendr_incompatible_types.contains(n.as_str())
                                && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n))
                        || matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
                            if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n)
                                if !opaque_types.contains(n.as_str())
                                    && !enum_names.contains(n.as_str())
                                    && !extendr_incompatible_types.contains(n.as_str())
                                    && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n)))
                });
                if func_return_needs_json || func_params_need_json {
                    builder.add_item(&gen_extendr_json_bridged_function(
                        func,
                        self,
                        &core_import,
                        &opaque_types,
                        &cfg,
                        &extendr_incompatible_types,
                        &enum_names,
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
        }

        // Trait bridge wrappers — generate extendr bridge structs that delegate to R list objects
        let mut emitted_send_robj_helper = false;
        for bridge_cfg in &config.trait_bridges {
            // Skip bridges explicitly excluded for this language.
            if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
                continue;
            }
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
                // Emit the shared SendRobj wrapper once before the first bridge struct so async
                // bridge methods can move `Robj` clones into `tokio::spawn_blocking` closures.
                if !emitted_send_robj_helper {
                    builder.add_item(crate::backends::extendr::trait_bridge::gen_send_robj_helper());
                    emitted_send_robj_helper = true;
                }
                let bridge = crate::backends::extendr::trait_bridge::gen_trait_bridge(
                    trait_type,
                    bridge_cfg,
                    &core_import,
                    &config.error_type_name(),
                    &config.error_constructor_expr(),
                    api,
                )?;
                for imp in &bridge.imports {
                    builder.add_import(imp);
                }
                builder.add_item(&bridge.code);
            }
        }

        // Module registration — only include types that were actually generated.
        // Arc-incompatible opaque types (e.g. VisitorHandle) were skipped above and
        // must be omitted from the module so the linker/R doesn't expect them.
        let module_name = config.r_package_name().replace('-', "_");
        let module_items = format!(
            "extendr_module! {{\n    mod {module};\n{types}{flat_enums}{json_enums}{funcs}}}\n",
            module = module_name,
            types = api
                .types
                .iter()
                .filter(|t| {
                    !t.is_trait
                        && !arc_incompatible_opaque.contains(&t.name)
                        && !extendr_incompatible_types.contains(&t.name)
                })
                .map(|t| format!("    impl {};\n", t.name))
                .collect::<String>(),
            flat_enums = flat_data_enum_names_vec
                .iter()
                .map(|n| format!("    impl {n};\n"))
                .collect::<String>(),
            json_enums = json_passthrough_enum_names_vec
                .iter()
                .map(|n| format!("    impl {n};\n"))
                .collect::<String>(),
            funcs = api
                .functions
                .iter()
                .filter(|f| !bridge_fn_names.contains(&f.name))
                .filter(|f| !r_exclude_functions.contains(&f.name))
                .map(|f| format!("    fn {};\n", f.name))
                .collect::<String>()
                + &collect_trait_bridge_functions(config)
                    .iter()
                    .map(|tb| format!("    fn {};\n", tb.name))
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
        // The NAMESPACE / DESCRIPTION sit one directory above R/ (i.e. packages/r/).
        let r_pkg_dir = r_wrapper_dir.trim_end_matches("R/").trim_end_matches("R");

        let mut files = Vec::new();

        // {package_name}.R: only @useDynLib stub. The actual call wrappers and class
        // dispatchers live in extendr-wrappers.R which is regenerated below.
        let mut pkg_content = hash::header(CommentStyle::Hash);
        pkg_content.push('\n');
        pkg_content.push_str(&crate::backends::extendr::template_env::render(
            "r_use_dyn_lib.jinja",
            minijinja::context! { package_name => package_name },
        ));
        pkg_content.push_str("NULL\n");
        files.push(GeneratedFile {
            path: PathBuf::from(&r_wrapper_dir).join(format!("{package_name}.R")),
            content: pkg_content,
            generated_header: false,
        });

        // extendr-wrappers.R: R-side bindings for every `#[extendr]` function and method
        // registered in the `extendr_module!` macro. Without this file, R callers cannot
        // invoke the native `wrap__<symbol>` entry points exported by the .so.
        // Historically `rextendr::document()` produced this at package-development time;
        // alef now emits it directly so install-time builds (which never run rextendr)
        // still expose the public API.
        let input_type_names = crate::codegen::conversions::input_type_names(api);
        let trait_bridge_fns = collect_trait_bridge_functions(config);
        let r_exclude_functions: ahash::AHashSet<String> = config
            .r
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();
        let wrappers_content = gen_extendr_wrappers_r(
            api,
            &package_name,
            &input_type_names,
            &trait_bridge_fns,
            &r_exclude_functions,
            &config.trait_bridges,
        );
        files.push(GeneratedFile {
            path: PathBuf::from(&r_wrapper_dir).join("extendr-wrappers.R"),
            content: wrappers_content,
            generated_header: false,
        });

        // NAMESPACE: regenerated each run so that newly added `#[extendr]` functions and
        // methods are exported. Scaffolding only writes a useDynLib bootstrap on init.
        let namespace_content = gen_namespace(
            api,
            &package_name,
            &trait_bridge_fns,
            &r_exclude_functions,
            &config.trait_bridges,
        );
        files.push(GeneratedFile {
            path: PathBuf::from(r_pkg_dir).join("NAMESPACE"),
            content: namespace_content,
            generated_header: false,
        });

        // options.R: generated from the configured/options-like IR type so all fields are present.
        if let Some(opts_type) = find_r_options_type(api, config) {
            let options_r = gen_conversion_options_r(opts_type);
            files.push(GeneratedFile {
                path: PathBuf::from(&r_wrapper_dir).join("options.R"),
                content: options_r,
                generated_header: true,
            });
        }

        // options.rs (Rust-side): generated decode_options function that handles ExternalPtr-wrapped
        // binding types from the configured/options-like type's default() and similar constructors.
        if let Some(opts_type) = find_r_options_type(api, config) {
            let core_import = config.core_import_name();
            let options_rs = gen_options_rs(api, opts_type, &core_import);
            let rust_output_path =
                resolve_output_dir(config.output_paths.get("r"), &config.name, "packages/r/src/rust/src");
            files.push(GeneratedFile {
                path: PathBuf::from(&rust_output_path).join("options.rs"),
                content: options_rs,
                generated_header: true,
            });
        }

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
            crate_suffix: "-extendr",
            build_dep: BuildDependency::None,
            post_build: vec![],
        })
    }
}

fn find_r_options_type<'a>(api: &'a ApiSurface, config: &ResolvedCrateConfig) -> Option<&'a TypeDef> {
    config
        .trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|bridge| bridge.options_type.as_deref())
        .find_map(|type_name| api.types.iter().find(|t| t.name == type_name && !t.is_trait))
        .or_else(|| find_r_options_type_from_api(api))
}

fn find_r_options_type_from_api(api: &ApiSurface) -> Option<&TypeDef> {
    let input_type_names = crate::codegen::conversions::input_type_names(api);
    api.types
        .iter()
        .find(|t| !t.is_trait && t.has_default && input_type_names.contains(&t.name))
}

/// Generate the `options.R` file for the R package from the configured options IR type.
///
/// Produces a roxygen-documented `conversion_options()` helper function with one parameter per
/// field (all defaulting to `NULL`). R callers use named arguments to override individual
/// settings; unset parameters remain `NULL` and are omitted from the resulting list so that the
/// Rust side applies its own defaults.
fn gen_conversion_options_r(opts_type: &TypeDef) -> String {
    use crate::core::ir::PrimitiveType;

    // Build parameter list
    let params: Vec<String> = opts_type
        .fields
        .iter()
        .map(|f| format!("{} = NULL", f.name.trim_start_matches('_')))
        .collect();

    // Build field info for template
    let fields: Vec<minijinja::Value> = opts_type
        .fields
        .iter()
        .map(|field| {
            let rname = field.name.trim_start_matches('_');
            let doc_text = if field.doc.is_empty() {
                rname.to_string()
            } else {
                let first = field.doc.lines().next().unwrap_or(rname);
                first.trim_end_matches('.').to_string()
            };

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

            minijinja::context! {
                rname => rname,
                doc => doc_text,
                cfg => field.cfg.is_some(),
                assign_val => assign_val,
            }
        })
        .collect();

    crate::backends::extendr::template_env::render(
        "conversion_options.jinja",
        minijinja::context! {
            params => params,
            fields => fields,
        },
    )
}

/// Generate the Rust-side `options.rs` module with `decode_options` function.
///
/// The `decode_options` function handles input from R in three main forms:
/// 1. ExternalPtr<T> (from $default() / builder methods) — unwraps and converts to core
/// 2. NULL — uses the configured options type's default
/// 3. Named list with field names matching struct fields — decodes field by field
///
/// This allows R callers to pass `OptionsType$default()`, NULL, or a named list.
fn gen_options_rs(api: &ApiSurface, opts_type: &TypeDef, _core_import: &str) -> String {
    let mut code = String::new();
    code.push_str("//! Option decoding for R bindings.\n\n");
    code.push_str("use extendr_api::prelude::*;\n\n");

    let type_defs: std::collections::HashMap<_, _> = api.types.iter().map(|t| (t.name.as_str(), t)).collect();
    let enum_defs: std::collections::HashMap<_, _> = api.enums.iter().map(|e| (e.name.as_str(), e)).collect();

    // Generate enum and nested struct decoders by inspecting field types.
    let mut enum_decoders = std::collections::BTreeSet::new();
    let mut struct_decoders = std::collections::BTreeSet::new();
    for field in &opts_type.fields {
        collect_option_decoder_types(
            &field.ty,
            opts_type.name.as_str(),
            &type_defs,
            &enum_defs,
            &mut enum_decoders,
            &mut struct_decoders,
        );
    }

    // Helper function for list access
    code.push_str("/// Helper: extract and convert a value from an R list by name.\n");
    code.push_str("fn list_get(list: &List, key: &str) -> Option<Robj> {\n");
    code.push_str("    list.iter().find(|(n, _)| *n == key).map(|(_, v)| v)\n");
    code.push_str("}\n\n");

    // Generate enum-specific decoders
    for enum_name in enum_decoders {
        if let Some(enum_def) = enum_defs.get(enum_name.as_str()) {
            gen_enum_decoder(&mut code, enum_def);
        }
    }

    for struct_name in struct_decoders {
        if let Some(struct_def) = type_defs.get(struct_name.as_str()) {
            gen_struct_decoder(&mut code, struct_def, &enum_defs, &type_defs);
        }
    }

    // Main decode_options function
    code.push_str("/// Decode an R ExternalPtr, NULL, or named list into ");
    code.push_str(&opts_type.name);
    code.push_str(".\n");
    code.push_str("///\n");
    code.push_str("/// Accepts:\n");
    code.push_str("/// - ExternalPtr of the configured options type (from $default() or builder methods) — unwraps and converts\n");
    code.push_str("/// - NULL — returns the configured options type's default\n");
    code.push_str("/// - Named list with field names matching struct fields — decodes field by field\n");
    code.push_str("///\n");
    code.push_str("/// Fields are optional: omitted fields retain their defaults. Unknown fields are ignored.\n");
    code.push_str("pub fn decode_options(options: Robj) -> std::result::Result<crate::");
    code.push_str(&opts_type.name);
    code.push_str(", String> {\n");
    code.push_str("    if options.is_null() {\n");
    code.push_str("        return Ok(crate::");
    code.push_str(&opts_type.name);
    code.push_str("::default());\n");
    code.push_str("    }\n\n");

    code.push_str("    // Accept the wrapper struct returned by the options type's default() / builder methods,\n");
    code.push_str("    // which extendr exposes as an `ExternalPtr`. The binding struct is returned directly\n");
    code.push_str("    // from the #[extendr] impl methods, so unwrap it as the binding type.\n");
    code.push_str("    if let Ok(ext) = ExternalPtr::<crate::");
    code.push_str(&opts_type.name);
    code.push_str(">::try_from(&options) {\n");
    code.push_str("        // Clone the binding struct and convert to core type via the generated From impl\n");
    code.push_str("        return Ok((*ext).clone().into());\n");
    code.push_str("    }\n\n");

    code.push_str("    // Try to decode as a named list\n");
    code.push_str("    let list = List::try_from(&options)\n");
    code.push_str("        .map_err(|e| format!(\"options must be NULL, ExternalPtr, or named list: {e}\"))?;\n");
    code.push_str("    let mut opts = crate::");
    code.push_str(&opts_type.name);
    code.push_str("::default();\n\n");

    // Generate field decoders
    for field in &opts_type.fields {
        gen_field_decoder(&mut code, field, &enum_defs, &type_defs);
    }

    code.push_str(
        "    // Note: visitor field is skipped — R has no visitor concept, so it remains at default None\n\n",
    );
    code.push_str("    Ok(opts)\n");
    code.push_str("}\n");

    code
}

fn collect_option_decoder_types(
    ty: &TypeRef,
    root_type_name: &str,
    type_defs: &std::collections::HashMap<&str, &TypeDef>,
    enum_defs: &std::collections::HashMap<&str, &EnumDef>,
    enum_decoders: &mut std::collections::BTreeSet<String>,
    struct_decoders: &mut std::collections::BTreeSet<String>,
) {
    let TypeRef::Named(name) = ty else {
        if let TypeRef::Optional(inner) = ty {
            collect_option_decoder_types(
                inner,
                root_type_name,
                type_defs,
                enum_defs,
                enum_decoders,
                struct_decoders,
            );
        }
        return;
    };
    if enum_defs.contains_key(name.as_str()) {
        enum_decoders.insert(name.clone());
        return;
    }
    let Some(type_def) = type_defs.get(name.as_str()) else {
        return;
    };
    if type_def.name == root_type_name || type_def.is_opaque || type_def.is_trait {
        return;
    }
    if struct_decoders.insert(type_def.name.clone()) {
        for field in &type_def.fields {
            collect_option_decoder_types(
                &field.ty,
                root_type_name,
                type_defs,
                enum_defs,
                enum_decoders,
                struct_decoders,
            );
        }
    }
}

/// Generate an enum decoder function for the given enum definition.
fn gen_enum_decoder(code: &mut String, enum_def: &EnumDef) {
    if enum_def.variants.iter().any(|variant| !variant.fields.is_empty()) {
        return;
    }

    let enum_name = &enum_def.name;
    let field_name_snake = r_function_component(enum_name);
    let fn_name = format!("decode_{}", field_name_snake);

    code.push_str("/// Decode a ");
    code.push_str(&field_name_snake.replace('_', " "));
    code.push_str(" enum from its string representation.\n");
    code.push_str("fn ");
    code.push_str(&fn_name);
    code.push_str("(val: Robj) -> std::result::Result<crate::");
    code.push_str(enum_name);
    code.push_str(", String> {\n");
    code.push_str("    let s = String::try_from(&val).map_err(|e| format!(\"");
    code.push_str(&field_name_snake);
    code.push_str(": {e}\"))?;\n");
    code.push_str("    match s.as_str() {\n");

    for variant in &enum_def.variants {
        code.push_str("        \"");
        code.push_str(&variant.name);
        code.push_str("\" => Ok(crate::");
        code.push_str(enum_name);
        code.push_str("::");
        code.push_str(&variant.name);
        code.push_str("),\n");
    }

    code.push_str("        _ => Err(format!(\"");
    code.push_str(&field_name_snake);
    code.push_str(": unknown variant '{}'\", s)),\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");
}

/// Generate decoder for a nested options struct.
fn gen_struct_decoder(
    code: &mut String,
    typ: &TypeDef,
    enum_defs: &std::collections::HashMap<&str, &EnumDef>,
    type_defs: &std::collections::HashMap<&str, &TypeDef>,
) {
    let decoder_name = format!("decode_{}", r_function_component(&typ.name));
    let label = r_function_component(&typ.name);
    code.push_str("/// Decode ");
    code.push_str(&typ.name);
    code.push_str(" from an R list.\n");
    code.push_str("fn ");
    code.push_str(&decoder_name);
    code.push_str("(val: Robj) -> std::result::Result<crate::");
    code.push_str(&typ.name);
    code.push_str(", String> {\n");
    code.push_str("    if val.is_null() {\n");
    code.push_str("        return Ok(crate::");
    code.push_str(&typ.name);
    code.push_str("::default());\n");
    code.push_str("    }\n");
    code.push_str("    let list = List::try_from(&val).map_err(|e| format!(\"");
    code.push_str(&label);
    code.push_str(": {e}\"))?;\n");
    code.push_str("    let mut opts = crate::");
    code.push_str(&typ.name);
    code.push_str("::default();\n\n");

    for field in &typ.fields {
        gen_field_decoder(code, field, enum_defs, type_defs);
    }

    code.push_str("    Ok(opts)\n");
    code.push_str("}\n\n");
}

/// Map a core type to its binding type for the R extendr backend.
/// This applies the type transformations used in the binding layer:
/// - u64, i64, usize, isize -> f64
/// - Other primitives stay the same
/// - Optional wrapping is preserved
fn map_type_to_binding(ty: &crate::core::ir::TypeRef) -> crate::core::ir::TypeRef {
    use crate::core::ir::{PrimitiveType, TypeRef};

    match ty {
        TypeRef::Primitive(
            _prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
        ) => {
            // These are mapped to f64 in the binding layer
            TypeRef::Primitive(PrimitiveType::F64)
        }
        TypeRef::Optional(inner) => {
            // Recursively map the inner type and re-wrap in Optional
            TypeRef::Optional(Box::new(map_type_to_binding(inner)))
        }
        other => other.clone(),
    }
}

/// Generate field decoding logic for a single field.
fn gen_field_decoder(
    code: &mut String,
    field: &crate::core::ir::FieldDef,
    enum_defs: &std::collections::HashMap<&str, &EnumDef>,
    type_defs: &std::collections::HashMap<&str, &TypeDef>,
) {
    use crate::core::ir::{PrimitiveType, TypeRef};

    // Skip visitor field — R has no visitor concept; it remains at default None
    if field.name == "visitor" {
        return;
    }

    // Map the core field type to the binding type before generating decoder logic
    let binding_ty = map_type_to_binding(&field.ty);

    let field_name = &field.name;
    let field_name_trim = field_name.trim_start_matches('_');

    match &binding_ty {
        TypeRef::Primitive(PrimitiveType::Bool) => {
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = bool::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::String => {
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = String::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Char => {
            // In the binding layer, char is mapped to String, so just assign the string directly
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = String::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Primitive(
            prim @ (PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32),
        ) => {
            let ty = match prim {
                PrimitiveType::U8 => "u8",
                PrimitiveType::U16 => "u16",
                PrimitiveType::U32 => "u32",
                PrimitiveType::I8 => "i8",
                PrimitiveType::I16 => "i16",
                PrimitiveType::I32 => "i32",
                _ => unreachable!(),
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = ");
            code.push_str(ty);
            code.push_str("::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Primitive(
            prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
        ) => {
            // R maps these types to f64 in the binding layer because R has no native u64/usize.
            // The core field, however, still uses the original integer type, so the f64 value
            // read from R must be cast back to the core type when assigning to `opts.{field}`.
            //
            // `field.optional == true` means the core field is `Option<T>` even though
            // `field.ty` was stripped of its `Option` wrapper by the IR extractor.
            let core_ty = match prim {
                PrimitiveType::U64 => "u64",
                PrimitiveType::I64 => "i64",
                PrimitiveType::Usize => "usize",
                PrimitiveType::Isize => "isize",
                _ => unreachable!(),
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            if field.optional {
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val as ");
                code.push_str(core_ty);
                code.push_str(");\n");
                code.push_str("        }\n");
            } else {
                code.push_str("        let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = f64_val as ");
                code.push_str(core_ty);
                code.push_str(";\n");
            }
            code.push_str("    }\n");
        }
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => {
            let ty = match &binding_ty {
                TypeRef::Primitive(PrimitiveType::F32) => "f32",
                _ => "f64",
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            if field.optional {
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = ");
                code.push_str(ty);
                code.push_str("::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val);\n");
                code.push_str("        }\n");
            } else {
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = ");
                code.push_str(ty);
                code.push_str("::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
            }
            code.push_str("    }\n");
        }
        TypeRef::Vec(inner) => {
            if matches!(inner.as_ref(), TypeRef::String) {
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        let strings = Strings::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("        let vec: Vec<String> = strings\n");
                code.push_str("            .iter()\n");
                code.push_str("            .map(|s| s.to_string())\n");
                code.push_str("            .collect();\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = vec;\n");
                code.push_str("    }\n");
            }
        }
        TypeRef::Named(enum_name)
            if (enum_defs.contains_key(enum_name.as_str()) || type_defs.contains_key(enum_name.as_str())) =>
        {
            let fn_name = format!("decode_{}", r_function_component(enum_name));
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = ");
            code.push_str(&fn_name);
            code.push_str("(v)?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::Named(name) if enum_defs.contains_key(name.as_str()) => {
                    let fn_name = format!("decode_{}", r_function_component(name));
                    code.push_str("    if let Some(v) = list_get(&list, \"");
                    code.push_str(field_name_trim);
                    code.push_str("\") {\n");
                    code.push_str("        opts.");
                    code.push_str(field_name);
                    code.push_str(" = Some(");
                    code.push_str(&fn_name);
                    code.push_str("(v)?);\n");
                    code.push_str("    }\n");
                }
                TypeRef::Named(name) if type_defs.contains_key(name.as_str()) => {
                    let fn_name = format!("decode_{}", r_function_component(name));
                    code.push_str("    if let Some(v) = list_get(&list, \"");
                    code.push_str(field_name_trim);
                    code.push_str("\") {\n");
                    code.push_str("        opts.");
                    code.push_str(field_name);
                    code.push_str(" = Some(");
                    code.push_str(&fn_name);
                    code.push_str("(v)?);\n");
                    code.push_str("    }\n");
                }
                TypeRef::Primitive(
                    prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
                ) => {
                    // R maps these types to Option<f64> in the binding layer, but the core
                    // field uses the original integer type, so cast f64 back to the core type.
                    let core_ty = match prim {
                        PrimitiveType::U64 => "u64",
                        PrimitiveType::I64 => "i64",
                        PrimitiveType::Usize => "usize",
                        PrimitiveType::Isize => "isize",
                        _ => unreachable!(),
                    };
                    code.push_str("    if let Some(v) = list_get(&list, \"");
                    code.push_str(field_name_trim);
                    code.push_str("\") {\n");
                    code.push_str("        if !v.is_null() {\n");
                    code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                    code.push_str(field_name_trim);
                    code.push_str(": {e}\"))?;\n");
                    code.push_str("            opts.");
                    code.push_str(field_name);
                    code.push_str(" = Some(f64_val as ");
                    code.push_str(core_ty);
                    code.push_str(");\n");
                    code.push_str("        }\n");
                    code.push_str("    }\n");
                }
                TypeRef::Primitive(PrimitiveType::F64) => {
                    // Option<f64> is used as-is in the binding layer
                    code.push_str("    if let Some(v) = list_get(&list, \"");
                    code.push_str(field_name_trim);
                    code.push_str("\") {\n");
                    code.push_str("        if !v.is_null() {\n");
                    code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                    code.push_str(field_name_trim);
                    code.push_str(": {e}\"))?;\n");
                    code.push_str("            opts.");
                    code.push_str(field_name);
                    code.push_str(" = Some(f64_val);\n");
                    code.push_str("        }\n");
                    code.push_str("    }\n");
                }
                _ => {} // Skip other Option types
            }
        }
        _ => {} // Skip other types
    }
}

/// Convert a CamelCase type name to snake_case for function names.
fn r_function_component(name: &str) -> String {
    public_host_identifier(Language::R, PublicIdentifierKind::Function, name)
}

/// Returns true if the function return type cannot be handled by extendr's `#[extendr]` macro
/// automatically, and therefore requires JSON bridging.
///
/// Extendr cannot handle:
///   - Named types in `extendr_incompatible_types` (structs with Vec<T> fields)
///   - Vec<Named> where Named is extendr-incompatible
///   - Vec<Vec<_>> (no Robj impl for nested vectors)
///   - Option<Enum> (enums don't implement ToVectorValue, so Option<Enum> fails)
///   - Option<non-opaque struct> (Option<ExternalPtr<T>> doesn't implement ToVectorValue)
///
/// Returns true if `e` is a data enum where every data-carrying variant is either a
/// single-field tuple variant or a single-field struct variant. Such enums are
/// represented as flat structs in the R binding so that callers can access variant
/// data with dot notation (e.g. `result$format$excel$sheet_count`).
///
/// Single-field struct variants like `Preset { name: String }` are treated
/// identically to single-field tuple variants `Preset(String)` — both expose one
/// scalar field per variant in the flat struct.
/// Recursively collect all Named type names from a TypeRef into a set.
fn collect_named_types_into(ty: &TypeRef, out: &mut AHashSet<String>) {
    match ty {
        TypeRef::Named(name) => {
            out.insert(name.clone());
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => collect_named_types_into(inner, out),
        TypeRef::Map(k, v) => {
            collect_named_types_into(k, out);
            collect_named_types_into(v, out);
        }
        _ => {}
    }
}

fn is_flat_data_enum(e: &crate::core::ir::EnumDef) -> bool {
    let has_data = e.variants.iter().any(|v| !v.fields.is_empty());
    has_data
        && e.variants
            .iter()
            .filter(|v| !v.fields.is_empty())
            .all(|v| v.fields.len() == 1)
}

/// Returns true if a flat data enum can safely generate a binding→core From impl.
/// Only enums whose tuple variant data is String or Option<String> are safe — complex
/// output-only struct types (DocxMetadata, PdfMetadata, etc.) have no reverse conversion.
fn can_flat_data_enum_round_trip(e: &crate::core::ir::EnumDef) -> bool {
    e.variants.iter().all(|v| {
        if v.fields.is_empty() {
            return true; // unit variants always safe
        }
        if v.is_tuple && v.fields.len() == 1 {
            let ty = &v.fields[0].ty;
            matches!(ty, crate::core::ir::TypeRef::String)
                || matches!(ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::String))
        } else {
            false
        }
    })
}

/// Returns true if `e` is a tagged data enum (i.e. has `serde_tag`) that cannot be
/// represented as a flat struct, but can be safely round-tripped through serde JSON
/// — at least one variant has data and `is_flat_data_enum` returns false. These
/// enums get a JSON-passthrough binding (newtype around the core type's serde
/// JSON encoding) so the variant payload survives the FFI boundary.
///
/// The core type must implement `Serialize`/`Deserialize` consistently with the wire
/// format. Tagged enums in supported source crates derive both unconditionally, so this is safe.
fn is_json_passthrough_data_enum(e: &crate::core::ir::EnumDef) -> bool {
    if is_flat_data_enum(e) {
        return false;
    }
    if e.serde_tag.is_none() {
        return false;
    }
    e.variants.iter().any(|v| !v.fields.is_empty())
}

/// Generate a JSON-passthrough wrapper struct for a tagged data enum.
///
/// The wrapper carries the serde-JSON encoding of the core enum value in a private
/// `__inner` field. `#[serde(from, into)]` plugs the wrapper into serde so nested
/// deserialization through parent binding structs (e.g. `EmbeddingConfig::from_json`)
/// preserves the inner variant data transparently — the parent's serde derives drive
/// the bridge with no extra glue.
///
/// The struct exposes `from_json(json: String)` (for direct construction from R) and
/// `default()`. From/Into impls bridge to the core type via serde round-trip.
fn gen_extendr_json_passthrough_enum_struct(enum_def: &crate::core::ir::EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    format!(
        r#"#[extendr]
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "{core_path}", into = "{core_path}")]
pub struct {name} {{
    /// Serde-JSON encoding of the underlying core enum value. Preserves the
    /// tagged-variant payload across the FFI boundary so round trips don't drop
    /// inner field data. The field is private-by-convention (double-underscore
    /// prefix) and not surfaced in R; construction goes through `from_json`.
    #[serde(skip)]
    pub __inner: String,
}}

impl From<{core_path}> for {name} {{
    fn from(value: {core_path}) -> Self {{
        Self {{
            __inner: serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string()),
        }}
    }}
}}

impl From<{name}> for {core_path} {{
    fn from(value: {name}) -> Self {{
        if value.__inner.is_empty() {{
            return <{core_path}>::default();
        }}
        serde_json::from_str(&value.__inner).unwrap_or_default()
    }}
}}

#[extendr]
impl {name} {{
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> {name} {{
        <{core_path}>::default().into()
    }}
    pub fn from_json(json: String) -> extendr_api::Result<{name}> {{
        let core: {core_path} =
            serde_json::from_str(&json).map_err(|e| extendr_api::Error::Other(e.to_string()))?;
        Ok(core.into())
    }}
}}
"#
    )
}

/// Generate an extendr function with bridge field binding support.
///
/// For R, the function accepts the options as an Robj (R list), extracts the bridge field
/// from it, creates the bridge, injects it into the decoded options struct, and calls the
/// core function. This is similar to PyO3's gen_bridge_field_function but tailored to R.
fn gen_extendr_bridge_field_function(
    api: &ApiSurface,
    func: &FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    core_import: &str,
) -> String {
    let func_name = &func.name;
    let options_param = &bridge_match.param_name;
    let field_name = &bridge_match.field_name;
    let handle_path =
        crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_match.bridge, core_import);
    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("R", bridge_match.bridge);

    // Build the param list for the Rust function signature.
    // Non-options params are emitted with the closest extendr-convertible Rust type
    // so the call site can hand them to the core function with the expected reference shape.
    let mut param_parts = Vec::new();
    for param in &func.params {
        if param.name == *options_param {
            // Options param is always Robj in the bridge_field case (decoded by decode_options).
            param_parts.push(format!("{}: Robj", param.name));
        } else {
            match &param.ty {
                TypeRef::String => param_parts.push(format!("{}: String", param.name)),
                _ => param_parts.push(format!("{}: Robj", param.name)),
            }
        }
    }
    let params_str = param_parts.join(", ");

    // Return type
    let return_type = "Result<Robj>";

    // Build the core function call. For `String` params we pass `&name` (deref-coerces to `&str`);
    // for `Robj` fall back to passing by reference as before. `opts` is already the core type.
    let mut call_args = Vec::new();
    for param in &func.params {
        if param.name == *options_param {
            call_args.push("Some(opts)".to_string());
        } else {
            call_args.push(format!("&{}", param.name));
        }
    }

    crate::backends::extendr::template_env::render(
        "bridge_field_function.jinja",
        minijinja::context! {
            func_name => func_name,
            params_str => params_str,
            return_type => return_type,
            field_name => field_name,
            options_param => options_param,
            handle_path => handle_path,
            struct_name => struct_name,
            core_import => core_import,
            options_type => &bridge_match.options_type,
            call_args_str => call_args.join(", "),
        },
    )
}

/// Generate a flat Rust struct for a data enum with all-tuple variants.
///
/// The struct has a discriminator field (from `serde_tag`, defaulting to `"format_type"`)
/// plus one `Option<T>` field per data-carrying variant. The variant field name is the
/// snake_case form of the variant name (e.g. `Excel` → `excel`).
///
/// `#[derive(Default)]` is required so `From` impls can use `..Default::default()`.
/// `serde::Serialize`/`Deserialize` are required so the JSON bridge produces and consumes
/// the nested representation.
fn gen_extendr_flat_data_enum_struct(
    enum_def: &crate::core::ir::EnumDef,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    cfg: &crate::codegen::generators::RustBindingConfig,
) -> String {
    let name = &enum_def.name;
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(1024);

    // Build derives: start with the binding config derives (e.g. ["Clone"]), then add
    // Default (for ..Default::default() in From impls) and serde derives for JSON bridge.
    let mut derives: Vec<&str> = cfg.struct_derives.to_vec();
    derives.push("Default");
    derives.push("serde::Serialize");
    derives.push("serde::Deserialize");
    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_derive.jinja",
        minijinja::context! {
            derives => derives.join(", "),
        },
    ));

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_struct_header.jinja",
        minijinja::context! {
            name => name,
        },
    ));
    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_discriminator_field.jinja",
        minijinja::context! {
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        if !variant.fields.is_empty() && variant.is_tuple {
            if let Some(first_field) = variant.fields.first() {
                let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
                let inner_ty = mapper.map_type(&first_field.ty);
                out.push_str(&crate::backends::extendr::template_env::render(
                    "flat_enum_variant_field.jinja",
                    minijinja::context! {
                        field_name => &field_name,
                        inner_ty => &inner_ty,
                    },
                ));
            }
        }
    }

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_struct_footer.jinja",
        minijinja::context! {},
    ));
    out
}

/// Generate a `From<core::EnumName> for FlatStruct` impl for flat data enums.
///
/// The generic `gen_enum_from_core_to_binding` generates enum→enum arm matching which does
/// not apply to flat structs. This function generates the correct struct-init form.
fn gen_extendr_flat_data_enum_from_core(enum_def: &crate::core::ir::EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_from_core_impl.jinja",
        minijinja::context! {
            core_path => &core_path,
            name => name,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::extendr::template_env::render(
                "flat_enum_from_core_variant_unit.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                },
            ));
        } else if variant.is_tuple {
            let first_field = variant.fields.first().unwrap();
            let is_boxed = first_field.is_boxed;
            let is_sanitized_to_string =
                first_field.sanitized && matches!(first_field.ty, crate::core::ir::TypeRef::String);
            let data_expr: String = if is_sanitized_to_string {
                if is_boxed {
                    "format!(\"{:?}\", *_0)".to_string()
                } else {
                    "format!(\"{:?}\", _0)".to_string()
                }
            } else if is_boxed {
                "(*_0).into()".to_string()
            } else {
                "_0.into()".to_string()
            };
            out.push_str(&crate::backends::extendr::template_env::render(
                "flat_enum_from_core_variant_tuple.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                    fname => &field_name,
                    expr => &data_expr,
                },
            ));
        } else {
            // Struct variant: data is lost in the flat binding (extendr can only
            // represent tuple variants as struct fields). Pattern match with .. to discard.
            out.push_str(&crate::backends::extendr::template_env::render(
                "flat_enum_from_core_variant_struct.jinja",
                minijinja::context! {
                    core_path => &core_path,
                    vname => &variant.name,
                    disc => discriminator,
                    wire => &wire_name,
                },
            ));
        }
    }

    // Always emit a `_ => Self::default()` catch-all. The IR-extraction crate may build
    // with a different feature set than the binding crate, and source variants gated by
    // `#[cfg(feature = "...")]` are silently absent from the extracted IR. `EnumVariant`
    // does not carry the cfg expression, so we cannot conditionally emit. The catch-all
    // is annotated with `#[allow(unreachable_patterns)]` to silence the warning for
    // enums whose binding match happens to be fully exhaustive at compile time.
    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_from_core_impl_catch_all.jinja",
        minijinja::context! {},
    ));

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_from_core_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

fn gen_extendr_flat_data_enum_to_core(enum_def: &crate::core::ir::EnumDef, core_import: &str) -> String {
    let name = &enum_def.name;
    let core_path = format!("{core_import}::{name}");
    let discriminator = enum_def.serde_tag.as_deref().unwrap_or("format_type");
    let mut out = String::with_capacity(512);

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_from_binding_impl.jinja",
        minijinja::context! {
            name => name,
            core_path => &core_path,
            discriminator => discriminator,
        },
    ));

    for variant in &enum_def.variants {
        let field_name = heck::AsSnakeCase(variant.name.as_str()).to_string();
        let wire_name = wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if variant.fields.is_empty() {
            out.push_str(&crate::backends::extendr::template_env::render(
                "flat_enum_from_binding_variant_unit.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    vname => &variant.name,
                },
            ));
        } else if variant.is_tuple {
            out.push_str(&crate::backends::extendr::template_env::render(
                "flat_enum_from_binding_variant_tuple.jinja",
                minijinja::context! {
                    wire => &wire_name,
                    vname => &variant.name,
                    fname => &field_name,
                },
            ));
        }
    }

    out.push_str(&crate::backends::extendr::template_env::render(
        "flat_enum_from_binding_impl_footer.jinja",
        minijinja::context! {},
    ));
    out
}

fn return_type_needs_json(
    ret: &TypeRef,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
    opaque_types: &AHashSet<String>,
) -> bool {
    match ret {
        TypeRef::Named(n) => {
            // Bare Enum: extendr enums don't impl From<T> for Robj
            if enum_names.contains(n.as_str()) {
                return true;
            }
            extendr_incompatible_types.contains(n.as_str())
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                // Vec<Enum>: extendr-generated enums don't impl From<Vec<T>> for Robj
                if enum_names.contains(n.as_str()) {
                    return true;
                }
                // Vec<OpaqueDTO>: opaque types don't impl From<Vec<T>> for Robj
                if opaque_types.contains(n.as_str()) {
                    return true;
                }
                extendr_incompatible_types.contains(n.as_str())
            }
            // Vec<Vec<_>> has no From<Vec<Vec<T>>> for Robj regardless of T
            TypeRef::Vec(_) => true,
            _ => false,
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            // Option<Enum>: extendr enums don't implement ToVectorValue
            TypeRef::Named(n) if enum_names.contains(n.as_str()) => true,
            // Option<non-opaque struct>: Option<ExternalPtr<T>> doesn't implement
            // ToVectorValue — bridge via JSON so R receives NULL or a JSON string.
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) && !enum_names.contains(n.as_str()) => true,
            // Option<Vec<Enum>>: Vec doesn't impl From<...> for Robj, and Enum doesn't impl ToVectorValue
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) => {
                    enum_names.contains(n.as_str())
                        || opaque_types.contains(n.as_str())
                        || extendr_incompatible_types.contains(n.as_str())
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Generate a JSON-bridged `#[extendr]` free function.
///
/// When a function's return type or parameter types cannot be handled by extendr's automatic
/// Robj conversions, this generates a wrapper that:
///   - For incompatible return types (ExtractionResult, Vec<ExtractionResult>, Vec<Vec<f32/f64>>,
///     Option<Enum>): serializes the Rust result to a JSON string via serde_json.
///   - For incompatible parameter types (Vec<Struct>): takes a JSON `String` and deserializes it.
///   - Async functions use the TokioBlockOn pattern (no `async fn`).
fn gen_extendr_json_bridged_function(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    cfg: &RustBindingConfig,
    extendr_incompatible_types: &AHashSet<String>,
    enum_names: &AHashSet<String>,
) -> String {
    use crate::codegen::generators::binding_helpers::gen_call_args_cfg;

    let err_map = ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))";
    let rt_new = format!("tokio::runtime::Runtime::new(){err_map}?");

    // Build the parameter list. For Vec<Struct> params (extendr-incompatible),
    // take `String` and deserialize from JSON.
    let mut sig_params: Vec<String> = Vec::new();
    let mut body_preamble = String::new();

    for param in &func.params {
        // Vec<T> or Option<Vec<T>> needs JSON bridging if T is:
        // - An enum (Vec<Enum>): extendr enums don't auto-convert
        // - An opaque type (Vec<OpaqueDTO>): no TryFrom<&Robj> impl
        // - A non-opaque struct (Vec<Struct>): in extendr_incompatible_types
        let needs_json_vec = match &param.ty {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) => {
                    enum_names.contains(n.as_str())
                        || opaque_types.contains(n.as_str())
                        || extendr_incompatible_types.contains(n.as_str())
                }
                _ => false,
            },
            TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                    TypeRef::Named(n) => {
                        enum_names.contains(n.as_str())
                            || opaque_types.contains(n.as_str())
                            || extendr_incompatible_types.contains(n.as_str())
                    }
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        };
        // Bare Named enum params: extendr enums don't have TryFrom<&Robj> impl, must use JSON.
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        // Bare Named struct params: types that lack a `#[extendr]` impl (they live in
        // `extendr_incompatible_types`) OR non-opaque non-enum structs that still can't be
        // auto-converted from Robj by extendr. Both must cross the boundary as JSON strings.
        // Note: enum check must come first since enums are also Named types.
        let needs_json_struct = !needs_json_enum
            && (matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str())
                || (!opaque_types.contains(n.as_str())
                    && !enum_names.contains(n.as_str())
                    && !extendr_incompatible_types.contains(n.as_str())))
                || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str()))));
        if needs_json_vec {
            // Take JSON string, deserialize to core Vec<T> or Option<Vec<T>>.
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                    _ => unreachable!(),
                },
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                        TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            // If param.ty is Option<Vec<T>>, the function signature always takes Option<String>
            // because it's already optional in the param type.
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_vec_optional_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_vec_required_preamble.jinja",
                    minijinja::context! {
                        name => &param.name,
                        ty => &core_ty_path,
                        err_map => &err_map,
                        mut_kw => &mut_kw,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else if needs_json_struct {
            // Take JSON string, deserialize to core type directly.
            // May be TypeRef::Named(n) or TypeRef::Optional(Named(n))
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_struct_optional_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_struct_required_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else if needs_json_enum {
            // Take JSON string, deserialize to core enum type directly or Option<EnumType>.
            let (core_ty_path, is_optional) = match &param.ty {
                TypeRef::Named(n) => (format!("{core_import}::{n}"), false),
                TypeRef::Optional(opt_inner) => match opt_inner.as_ref() {
                    TypeRef::Named(n) => (format!("{core_import}::{n}"), true),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let mut_kw = if param.is_mut { "mut " } else { "" };
            let param_is_optional = param.optional || is_optional;
            if param_is_optional {
                sig_params.push(format!("{}: Option<String>", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_struct_optional_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            } else {
                sig_params.push(format!("{}: String", param.name));
                body_preamble.push_str(&crate::backends::extendr::template_env::render(
                    "json_struct_required_preamble.jinja",
                    minijinja::context! {
                        mut_kw => mut_kw,
                        name => &param.name,
                        ty => &core_ty_path,
                        err => &err_map,
                    },
                ));
                body_preamble.push_str("    ");
            }
        } else {
            // Use the standard binding type. Named opaque structs have `#[extendr]` impls and
            // therefore generate `TryFrom<&Robj> for &T`; bare scalar/primitive types convert
            // directly.
            let ty_str = mapper.map_type(&param.ty);
            let sig_ty = if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                if param.optional {
                    format!("extendr_api::Nullable<&{ty_str}>")
                } else {
                    format!("&{ty_str}")
                }
            } else if param.optional {
                format!("Option<{ty_str}>")
            } else {
                ty_str
            };
            sig_params.push(format!("{}: {sig_ty}", param.name));
        }
    }

    // Determine the core function path.
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    // Handle named non-opaque struct params that need let bindings (for &T conversion).
    let mut named_let_bindings = String::new();
    for param in &func.params {
        let needs_json = matches!(&param.ty, TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n) if !opaque_types.contains(n.as_str())));
        // Skip bare Named extendr-incompatible structs, enums, and other non-opaque structs
        // already emit their `_core` binding via the JSON preamble in the param loop above.
        let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
            if enum_names.contains(n.as_str()))
            || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
        let needs_json_struct = !needs_json_enum
            && (matches!(&param.ty, TypeRef::Named(n)
            if extendr_incompatible_types.contains(n.as_str())
                || (!opaque_types.contains(n.as_str())
                    && !enum_names.contains(n.as_str())
                    && !extendr_incompatible_types.contains(n.as_str())))
                || matches!(&param.ty, TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::Named(n)
                    if !opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str()))));
        if !needs_json && !needs_json_struct && !needs_json_enum {
            if let TypeRef::Named(n) = &param.ty {
                if !opaque_types.contains(n.as_str()) {
                    if param.optional {
                        // Nullable<&T>: use into_option() then map to core
                        named_let_bindings.push_str(&crate::backends::extendr::template_env::render(
                            "named_let_optional_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    } else {
                        named_let_bindings.push_str(&crate::backends::extendr::template_env::render(
                            "named_let_required_binding.jinja",
                            minijinja::context! {
                                name => &param.name,
                                ci => core_import,
                                n => n,
                            },
                        ));
                        named_let_bindings.push_str("    ");
                    }
                }
            }
        }
    }

    // Adjust call_args for named let binding params.
    let final_call_args: Vec<String> = func
        .params
        .iter()
        .map(|param| {
            // Re-check needs_json using the updated logic
            let needs_json = match &param.ty {
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(n) => {
                        enum_names.contains(n.as_str())
                            || opaque_types.contains(n.as_str())
                            || extendr_incompatible_types.contains(n.as_str())
                    }
                    _ => false,
                },
                _ => false,
            };
            let needs_json_enum = matches!(&param.ty, TypeRef::Named(n)
                if enum_names.contains(n.as_str()))
                || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())));
            let needs_json_struct = !needs_json_enum
                && (matches!(&param.ty, TypeRef::Named(n)
                if extendr_incompatible_types.contains(n.as_str())
                    || (!opaque_types.contains(n.as_str())
                        && !enum_names.contains(n.as_str())
                        && !extendr_incompatible_types.contains(n.as_str())))
                    || matches!(&param.ty, TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::Named(n)
                        if !opaque_types.contains(n.as_str())
                            && !enum_names.contains(n.as_str())
                            && !extendr_incompatible_types.contains(n.as_str()))));
            if needs_json {
                // Vec<T> or Option<Vec<T>> JSON deserialized: pass as Vec/slice.
                if param.optional {
                    format!("{}_core.as_deref().unwrap_or_default()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else {
                    // Convert Vec<T> to &[T] for functions expecting slices
                    format!("{}_core.as_slice()", param.name)
                }
            } else if needs_json_struct || needs_json_enum {
                if param.optional && param.is_ref {
                    format!("{}_core.as_ref()", param.name)
                } else if param.optional {
                    format!("{}_core", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else if param.is_ref {
                    format!("&{}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else if matches!(&param.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())) {
                if param.optional {
                    format!("{}_core.as_ref()", param.name)
                } else if param.is_mut {
                    format!("&mut {}_core", param.name)
                } else {
                    format!("{}_core", param.name)
                }
            } else {
                gen_call_args_cfg(
                    std::slice::from_ref(param),
                    opaque_types,
                    cfg.cast_uints_to_i32,
                    cfg.cast_large_ints_to_f64,
                )
            }
        })
        .collect();
    let final_call_args_str = final_call_args.join(", ");

    // When the preamble emits `?` for JSON deserialization, the wrapping function must
    // return a `Result` even if the core function itself is infallible.
    let params_need_json_deserialize = func.params.iter().any(|p| match &p.ty {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => {
                enum_names.contains(n.as_str())
                    || opaque_types.contains(n.as_str())
                    || extendr_incompatible_types.contains(n.as_str())
            }
            _ => false,
        },
        TypeRef::Named(n) => {
            enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str())
        }
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n)
            if enum_names.contains(n.as_str())
                || extendr_incompatible_types.contains(n.as_str())
                || !opaque_types.contains(n.as_str())),
        _ => false,
    });
    let effectively_fallible = func.error_type.is_some() || params_need_json_deserialize;

    // Generate the return type — always String (JSON) for JSON-bridged functions,
    // or Option<String> for Option<Enum> returns.
    let (ret_type, result_convert) = match &func.return_type {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            // Option<Enum>: return Option<String> (or Result<Option<String>> if fallible).
            // `.transpose()` converts Option<Result<S,E>> → Result<Option<S>,E> for the
            // Result case; for the non-Result case it converts Option<Result<S,E>> → Result<Option<S>,E>
            // which we then `?`-unwrap to Option<String>.
            if effectively_fallible {
                // Last expression must be Result<Option<String>, E> — use transpose() without `?`
                let ser = format!(
                    "result.map(|v| serde_json::to_string(&v){err_map}).transpose()",
                    err_map = err_map
                );
                ("Result<Option<String>>".to_string(), ser)
            } else {
                // No error type: result is Option<Enum>, serialize each Some variant
                let ser = "result.map(|v| serde_json::to_string(&v).expect(\"serialization failed\"))".to_string();
                ("Option<String>".to_string(), ser)
            }
        }
        _ => {
            // All other incompatible types: return String (JSON)
            // For Result<String> functions, the last expr must be Result<String, E> — use map_err
            // without `?` (the `?` would unwrap to String, not match Result<String>).
            if effectively_fallible {
                let ser = format!("serde_json::to_string(&result){err_map}");
                ("Result<String>".to_string(), ser)
            } else {
                (
                    "String".to_string(),
                    "serde_json::to_string(&result).expect(\"serialization failed\")".to_string(),
                )
            }
        }
    };

    // Determine if the core result must be converted to a binding type before serialization.
    // Named types in extendr_incompatible_types have full core→binding From impls (e.g.
    // ExtractionResult). Serializing the binding type instead of the core type ensures the
    // JSON uses the flat-struct representation for data enums (e.g. FormatMetadata).
    let binding_conversion: Option<String> = match &func.return_type {
        TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => {
            Some(format!("let result: {n} = result.into();"))
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if extendr_incompatible_types.contains(n.as_str()) => Some(format!(
                "let result: Vec<{n}> = result.into_iter().map(Into::into).collect();"
            )),
            _ => None,
        },
        _ => None,
    };
    let convert = binding_conversion.as_deref().unwrap_or("");

    // Build the function body.
    let core_call = format!("{core_fn_path}({final_call_args_str})");

    // `core_suffix` is what we append to the core call. When the core function itself is
    // fallible we use `?` to propagate (with map_err); when only the wrapper is fallible
    // (because of JSON-deserialize preamble) the core call is infallible and we just bind.
    let core_call_with_err = if func.error_type.is_some() {
        format!("{core_call}{err_map}?")
    } else {
        core_call.clone()
    };

    let body = if func.is_async {
        // Async: use TokioBlockOn (no async fn for extendr)
        if func.error_type.is_some() {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await{err_map} }})?;\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                err_map = err_map,
                convert = convert,
                result_convert = result_convert,
            )
        } else {
            format!(
                "{body_preamble}{named_let_bindings}\
                 let rt = {rt_new};\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 {convert}\n    \
                 {result_convert}",
                body_preamble = body_preamble,
                named_let_bindings = named_let_bindings,
                rt_new = rt_new,
                core_call = core_call,
                convert = convert,
                result_convert = result_convert,
            )
        }
    } else {
        // Sync — the call expression already includes `?` and `map_err` when core is fallible.
        format!(
            "{body_preamble}{named_let_bindings}\
             let result = {core_call_with_err};\n    \
             {convert}\n    \
             {result_convert}",
            body_preamble = body_preamble,
            named_let_bindings = named_let_bindings,
            core_call_with_err = core_call_with_err,
            convert = convert,
            result_convert = result_convert,
        )
    };

    // Assemble the full function.
    let params_str = sig_params.join(", ");
    let allow = if effectively_fallible {
        "#[allow(clippy::missing_errors_doc)]\n"
    } else {
        ""
    };
    format!(
        "{allow}#[extendr]\npub fn {}({params_str}) -> {ret_type} {{\n    {body}\n}}",
        func.name
    )
}

/// Return the set of type names that are excluded from extendr class registration.
///
/// Mirrors the filters applied in `generate_bindings`:
///   • Trait types — never registered (no concrete class).
///   • Arc-incompatible opaque types (Rc-based, cfg-feature-gated) — skipped.
///   • Extendr-incompatible types: structs whose fields contain `Vec<T>` where T is a
///     non-opaque, non-enum named type. Extendr cannot convert these from R lists.
///
/// The returned set is used by wrapper-file generation to skip class env emission for
/// types that are not present in `extendr_module!`.
/// A trait-bridge function (register / unregister / clear) that must be wired into
/// `extendr_module!`, `extendr-wrappers.R`, and `NAMESPACE` alongside ordinary
/// free functions emitted from `api.functions`.
///
/// The IR (`ApiSurface`) does not contain these symbols because they are synthesised
/// by `gen_trait_bridge` from `TraitBridgeConfig` rather than parsed from Rust source.
/// Each entry records the name and the R-visible parameters so the R-side wrappers
/// can call `.Call("wrap__<name>", <args>, PACKAGE = ...)` with a matching signature.
pub(crate) struct TraitBridgeFn {
    pub(crate) name: String,
    /// Parameter names in R-visible order. R is dynamically typed so the type is
    /// erased — `register_fn` takes an R object (named list of closures), `unregister_fn`
    /// takes a plugin name, `clear_fn` takes nothing.
    pub(crate) params: Vec<String>,
}

/// Collect the set of free-function names that the trait-bridge generator will emit
/// (`register_<trait>` / `unregister_<trait>` / `clear_<trait>`). Used to filter
/// `api.functions` so a free function with the same name as a trait-bridge fn is
/// not emitted twice in `lib.rs` (which would be a Rust `E0428` duplicate
/// definition). Honours `exclude_languages` so excluded bridges don't shadow real
/// free functions.
///
/// Example: `clear_text_backends` is defined both as `pub fn` in
/// `crates/sample_core/src/plugins/ocr.rs` (so it appears in `api.functions`) AND
/// synthesised by the trait-bridge generator for the `TextBackend` trait. The
/// trait-bridge form is the canonical one — it resolves to the
/// `sample_core::plugins::text_backend::clear_text_backends` path module rather than
/// the top-level alias — so emit it from the bridge generator and skip the
/// duplicate from `api.functions`.
pub(crate) fn collect_trait_bridge_fn_names(config: &ResolvedCrateConfig) -> ahash::AHashSet<String> {
    let mut names = ahash::AHashSet::new();
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
            continue;
        }
        if let Some(name) = bridge_cfg.register_fn.as_deref() {
            names.insert(name.to_string());
        }
        if let Some(name) = bridge_cfg.unregister_fn.as_deref() {
            names.insert(name.to_string());
        }
        if let Some(name) = bridge_cfg.clear_fn.as_deref() {
            names.insert(name.to_string());
        }
    }
    names
}

/// Collect every trait-bridge register / unregister / clear function that the
/// extendr backend will emit for this crate, honouring `exclude_languages`.
///
/// The order matches `gen_trait_bridge` so the resulting extendr_module! entries
/// line up with the `#[extendr]` items in `lib.rs`.
pub(crate) fn collect_trait_bridge_functions(config: &ResolvedCrateConfig) -> Vec<TraitBridgeFn> {
    let mut out = Vec::new();
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
            continue;
        }
        // register_fn(r_backend: Robj) — the R caller passes a named list of closures.
        if let Some(name) = bridge_cfg.register_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: vec!["r_backend".to_string()],
            });
        }
        // unregister_fn(name: String) — the R caller passes the plugin name.
        if let Some(name) = bridge_cfg.unregister_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: vec!["name".to_string()],
            });
        }
        // clear_fn() — no arguments; clears every registered backend of this type.
        if let Some(name) = bridge_cfg.clear_fn.as_deref() {
            out.push(TraitBridgeFn {
                name: name.to_string(),
                params: Vec::new(),
            });
        }
    }
    out
}

fn collect_bridge_handle_aliases(bridges: &[TraitBridgeConfig]) -> ahash::AHashSet<String> {
    bridges.iter().filter_map(|bridge| bridge.type_alias.clone()).collect()
}

fn collect_excluded_class_types(api: &ApiSurface, bridges: &[TraitBridgeConfig]) -> ahash::AHashSet<String> {
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let enum_names: ahash::AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let bridge_handle_aliases = collect_bridge_handle_aliases(bridges);
    let arc_incompatible: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && bridge_handle_aliases.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    let is_struct_like =
        |n: &str| -> bool { !opaque_types.contains(n) && !enum_names.contains(n) && !arc_incompatible.contains(n) };
    let is_native_incompatible = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if is_struct_like(n) => true,
                TypeRef::Vec(_) => true, // Vec<Vec<_>> not supported
                _ => false,
            },
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Vec(inner2) => match inner2.as_ref() {
                    TypeRef::Named(n) if is_struct_like(n) => true,
                    TypeRef::Vec(_) => true, // Option<Vec<Vec<_>>> not supported
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        }
    };

    let mut excluded: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_trait)
        .map(|t| t.name.clone())
        .collect();
    for t in &arc_incompatible {
        excluded.insert(t.clone());
    }
    for t in &api.types {
        if t.is_opaque || t.is_trait {
            continue;
        }
        if t.fields.iter().any(|f| is_native_incompatible(&f.ty)) {
            excluded.insert(t.name.clone());
        }
    }
    excluded
}

/// Return true if the method should be filtered out of an emitted impl block.
///
/// Mirrors `method_references_arc_incompatible` and `method_references_enum` from
/// `generate_bindings`. Used by wrapper-file generation to skip wrapper entries for
/// methods that the Rust impl block will not contain.
fn method_is_excluded_from_impl(
    method: &crate::core::ir::MethodDef,
    api: &ApiSurface,
    bridges: &[TraitBridgeConfig],
) -> bool {
    let opaque_types: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let enum_names: ahash::AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let bridge_handle_aliases = collect_bridge_handle_aliases(bridges);
    let arc_incompatible: ahash::AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && bridge_handle_aliases.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    let references_arc_incompatible = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Named(n) => arc_incompatible.contains(n),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if arc_incompatible.contains(n)),
            _ => false,
        }
    };
    let references_enum = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Named(n) => enum_names.contains(n.as_str()),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if enum_names.contains(n.as_str())),
            _ => false,
        }
    };
    let param_is_owned_struct = |ty: &TypeRef| -> bool {
        let is_non_opaque_struct =
            |n: &str| !opaque_types.contains(n) && !enum_names.contains(n) && !arc_incompatible.contains(n);
        match ty {
            TypeRef::Named(n) => is_non_opaque_struct(n),
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(n) if is_non_opaque_struct(n)),
            _ => false,
        }
    };

    if references_arc_incompatible(&method.return_type)
        || method.params.iter().any(|p| references_arc_incompatible(&p.ty))
    {
        return true;
    }
    if references_enum(&method.return_type)
        || method
            .params
            .iter()
            .any(|p| references_enum(&p.ty) || param_is_owned_struct(&p.ty))
    {
        return true;
    }
    // Map return/param types: extendr cannot marshal HashMap/BTreeMap directly
    // (`HashMap<K, V>: ToVectorValue` is not implemented). Exclude any method
    // whose surface uses Map types; callers must access map fields via the struct
    // serialisation path (R named list) instead of through a method getter.
    let references_map = |ty: &TypeRef| -> bool {
        match ty {
            TypeRef::Map(_, _) => true,
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Map(_, _)),
            _ => false,
        }
    };
    if references_map(&method.return_type) || method.params.iter().any(|p| references_map(&p.ty)) {
        return true;
    }
    if method.sanitized {
        return true;
    }
    false
}

/// Human-readable R type description for a `TypeRef`, used to populate
/// `@param` / `@return` lines in the generated roxygen2 doc blocks. Returns
/// a sentence-cased phrase ending in a period (e.g. "Raw vector of bytes.").
fn r_type_description(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Bytes => "Raw vector of bytes.".to_string(),
        TypeRef::String => "Character string.".to_string(),
        TypeRef::Char => "Single-character string.".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "Logical (TRUE/FALSE).".to_string(),
            crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "Numeric.".to_string(),
            _ => "Integer.".to_string(),
        },
        TypeRef::Optional(inner) => {
            let inner_desc = r_type_description(inner);
            let trimmed = inner_desc.trim_end_matches('.');
            // Lower-case the first letter so "Character string" becomes "character string"
            // after the "Optional " prefix — but only for natural-language descriptions.
            // Named types (e.g. `ExtractionConfig object`) keep their proper-noun casing.
            let body = if matches!(**inner, TypeRef::Named(_)) {
                trimmed.to_string()
            } else {
                match trimmed.chars().next() {
                    Some(c) => {
                        let mut s = c.to_lowercase().collect::<String>();
                        s.push_str(&trimmed[c.len_utf8()..]);
                        s
                    }
                    None => String::new(),
                }
            };
            format!("Optional {body}. Defaults to NULL.")
        }
        TypeRef::Vec(inner) => {
            let inner_desc = r_type_description(inner);
            let trimmed = inner_desc.trim_end_matches('.');
            format!("List of {}.", trimmed.to_lowercase())
        }
        TypeRef::Map(_, _) => "Named list.".to_string(),
        TypeRef::Named(name) => format!("{name} object (list with class attribute)."),
        TypeRef::Path => "File path as character string.".to_string(),
        TypeRef::Unit => "Invisible NULL.".to_string(),
        TypeRef::Json => "JSON-serializable value.".to_string(),
        TypeRef::Duration => "Numeric duration in seconds.".to_string(),
    }
}

/// Convert the first character of `s` to upper-case while leaving the rest untouched.
/// Returns an empty string when `s` is empty.
fn title_case_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Append `text` to `block` so multi-line content stays inside the current
/// roxygen tag (`@param` / `@return`). The first line is appended as-is;
/// every subsequent line is prefixed with `#'   ` so R's parser still sees
/// the line as a comment rather than parsing it as code.
fn push_roxygen_inline_multiline(block: &mut String, text: &str) {
    let mut lines = text.lines();
    if let Some(first) = lines.next() {
        block.push_str(first.trim_end());
    }
    for line in lines {
        block.push('\n');
        block.push_str("#'   ");
        block.push_str(line.trim_end());
    }
}

/// Build the roxygen2 doc block for a free R wrapper function.
///
/// The block carries a title line (derived from the first line of `doc`, or
/// the function name as a fallback), optional description paragraphs, one
/// `@param` per parameter, an `@return`, and the `@export` tag. Every output
/// line is prefixed with `#'` — callers prepend the block directly above the
/// `name <- function(...) ...` definition.
fn r_roxygen_block(func_name: &str, doc: &str, params: &[ParamDef], return_type: &TypeRef) -> String {
    let mut block = String::with_capacity(256);
    let trimmed_doc = doc.trim();
    // Parse the rustdoc into sections so `# Arguments` / `# Returns` / `# Errors` /
    // `# Example` are surfaced as native roxygen2 tags instead of being emitted as raw
    // markdown headings in the description body.
    let sections = parse_rustdoc_sections(trimmed_doc);
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (func_name.to_string(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    // Build a name → description map from the `# Arguments` bullets, if any.
    // Falls back to the type-based description when no entry is present.
    let mut param_docs: HashMap<String, String> = HashMap::new();
    if let Some(args_body) = sections.arguments.as_deref() {
        for (name, desc) in parse_arguments_bullets(args_body) {
            if !desc.is_empty() {
                param_docs.insert(name, desc);
            }
        }
    }
    for param in params {
        block.push_str("#' @param ");
        block.push_str(&param.name);
        block.push(' ');
        if let Some(desc) = param_docs.get(&param.name) {
            push_roxygen_inline_multiline(&mut block, desc);
            if !desc.trim_end().ends_with('.') {
                block.push('.');
            }
        } else {
            block.push_str(&r_type_description(&param.ty));
        }
        block.push('\n');
    }
    block.push_str("#' @return ");
    if let Some(ret) = sections.returns.as_deref() {
        let ret = ret.trim();
        push_roxygen_inline_multiline(&mut block, ret);
        if !ret.ends_with('.') {
            block.push('.');
        }
    } else {
        block.push_str(&r_type_description(return_type));
    }
    block.push('\n');
    if let Some(err) = sections.errors.as_deref() {
        block.push_str("#'\n#' @section Errors:\n");
        for line in err.trim().lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    block.push_str("#' @export\n");
    block
}

/// Build a one-line description for a struct field, derived from the field's
/// `doc` comment. Falls back to the field name when the IR carries no docs.
///
/// R's roxygen2 `@field` tag is single-line per field; multi-paragraph rustdoc
/// must be collapsed. We take the first paragraph (lines up to the first blank
/// line), trim, and join with a single space.
fn r_field_one_liner(field_name: &str, doc: &str) -> String {
    let trimmed = doc.trim();
    if trimmed.is_empty() {
        return field_name.to_string();
    }
    let paragraph: Vec<&str> = trimmed
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect();
    if paragraph.is_empty() {
        field_name.to_string()
    } else {
        let mut result = paragraph.join(" ");
        // Enforce 120-char line limit for roxygen2. The format is:
        // #' @field <field_name> <description>
        // which is 10 + len(field_name) + 1 + len(description) = 120 max
        // So description can be at most 109 - len(field_name).
        let max_desc_len = 109_usize.saturating_sub(field_name.len());
        if result.len() > max_desc_len {
            result.truncate(max_desc_len);
            // Remove trailing partial words by finding the last space.
            if let Some(last_space) = result.rfind(' ') {
                result.truncate(last_space);
            }
        }
        result
    }
}

/// Build the roxygen2 doc block for a class env (one per registered struct).
///
/// Layout: title (first line of `typ.doc`, falling back to the class name),
/// optional description body, one `#' @field <name> <description>` per public
/// field, and the `#' @export` tag. The block is prepended to the class env
/// definition via the `r_type_class_env.jinja` template.
fn r_class_roxygen_block(typ: &TypeDef) -> String {
    let mut block = String::with_capacity(256);
    let sections = parse_rustdoc_sections(typ.doc.trim());
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (typ.name.clone(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    for field in &typ.fields {
        if field.binding_excluded {
            continue;
        }
        let rname = field.name.trim_start_matches('_');
        block.push_str("#' @field ");
        block.push_str(rname);
        block.push(' ');
        block.push_str(&r_field_one_liner(rname, &field.doc));
        block.push('\n');
    }
    block.push_str("#' @export\n");
    block
}

/// Build the roxygen2 doc block for a flat data enum class env.
///
/// Like `r_class_roxygen_block` but uses enum variants as fields — the flat
/// representation exposes one scalar field per variant (see
/// [`is_flat_data_enum`]). For JSON-passthrough enums (`is_json_passthrough_data_enum`),
/// the `@field` list is omitted because callers interact with the opaque
/// `__inner` JSON blob rather than typed variant fields.
fn r_enum_roxygen_block(enum_def: &EnumDef, include_variants_as_fields: bool) -> String {
    let mut block = String::with_capacity(256);
    let sections = parse_rustdoc_sections(enum_def.doc.trim());
    let summary = sections.summary.trim();
    let (title, description) = if summary.is_empty() {
        (enum_def.name.clone(), String::new())
    } else {
        let mut parts = summary.splitn(2, '\n');
        let raw_title = parts.next().unwrap_or("").trim().trim_end_matches('.');
        let title = title_case_first(raw_title);
        let description = parts.next().map(str::trim).unwrap_or("").to_string();
        (title, description)
    };
    block.push_str("#' ");
    block.push_str(&title);
    block.push('\n');
    if !description.is_empty() {
        block.push_str("#'\n");
        for line in description.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                block.push_str("#'\n");
            } else {
                block.push_str("#' ");
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if include_variants_as_fields {
        for variant in &enum_def.variants {
            block.push_str("#' @field ");
            block.push_str(&variant.name);
            block.push(' ');
            block.push_str(&r_field_one_liner(&variant.name, &variant.doc));
            block.push('\n');
        }
    }
    block.push_str("#' @export\n");
    block
}

/// Generate `extendr-wrappers.R` — the R-side bindings for every `#[extendr]` symbol
/// registered in the generated `extendr_module!` macro.
///
/// The output mirrors what `rextendr::document()` would produce at package-development
/// time, but is written directly from the alef IR so it is always present at install time.
///
/// Layout:
///   1. Free-function wrappers: `name <- function(...) .Call("wrap__name", ..., PACKAGE = "<pkg>")`.
///      Exported via `#' @export` (paired with explicit `export(name)` lines in NAMESPACE).
///   2. One `<TypeName> <- new.env(parent = emptyenv())` block per registered class, with:
///      • static methods bound as `Type$method <- function(...) .Call("wrap__Type__method", ...)`,
///      • instance methods bound as `Type$method <- function(...) .Call("wrap__Type__method", self, ...)`,
///      • dispatch operators (`$.Type`, `[[.Type`) so callers can write `instance$method(...)`.
fn gen_extendr_wrappers_r(
    api: &ApiSurface,
    package_name: &str,
    input_type_names: &ahash::AHashSet<String>,
    trait_bridge_fns: &[TraitBridgeFn],
    r_exclude_functions: &ahash::AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(8 * 1024);
    out.push_str("# Generated by extendr: Do not edit by hand\n");
    out.push_str("#\n");
    out.push_str("# This file is regenerated by alef on every `alef generate` run.\n");
    out.push_str("# It mirrors the output of `rextendr::document()` and binds every\n");
    out.push_str("# wrap__<symbol> entry registered in extendr_module! to an R-callable\n");
    out.push_str("# function or class env.\n\n");

    out.push_str(&crate::backends::extendr::template_env::render(
        "r_use_dyn_lib.jinja",
        minijinja::context! { package_name => package_name },
    ));
    out.push_str("NULL\n\n");

    // Names emitted by the trait-bridge generator; skip them in the free-function
    // pass so the wrapper file does not define the same R wrapper twice.
    let bridge_fn_names: ahash::AHashSet<&str> = trait_bridge_fns.iter().map(|tb| tb.name.as_str()).collect();

    // Free functions. Every entry in `api.functions` is registered in extendr_module!.
    for func in &api.functions {
        if bridge_fn_names.contains(func.name.as_str()) {
            continue;
        }
        if r_exclude_functions.contains(&func.name) {
            continue;
        }
        let params: Vec<String> = func.params.iter().map(|p| sanitize_r_param_name(&p.name)).collect();
        let params_sig = r_wrapper_params_signature(&func.params, api);
        let mut call_args = vec![format!("\"wrap__{}\"", func.name)];
        for p in &params {
            call_args.push(p.clone());
        }
        call_args.push(format!("PACKAGE = \"{package_name}\""));
        let call_args_str = call_args.join(", ");

        let roxygen_block = r_roxygen_block(&func.name, &func.doc, &func.params, &func.return_type);

        out.push_str(&crate::backends::extendr::template_env::render(
            "r_free_function_wrapper.jinja",
            minijinja::context! {
                func_name => &func.name,
                params_sig => params_sig,
                call_args_str => call_args_str,
                roxygen_block => roxygen_block,
            },
        ));
    }

    // Trait-bridge functions (register_<trait> / unregister_<trait> / clear_<trait>).
    // These are synthesised from `[trait_bridges]` in alef.toml rather than parsed from
    // Rust source, so they are absent from `api.functions` but are still registered in
    // `extendr_module!` (see `collect_trait_bridge_functions`). Without these R wrappers
    // callers cannot reach the `wrap__register_<trait>` entry points.
    //
    // The R-side surface is intentionally minimal: `register_<trait>` accepts an R object
    // (typically a named list of closures), `unregister_<trait>` accepts a name string,
    // `clear_<trait>` accepts nothing. Type checking happens on the Rust side via extendr's
    // `Robj` introspection — R's dynamic typing makes static signatures unnecessary.
    for bridge_fn in trait_bridge_fns {
        let params_sig = bridge_fn.params.join(", ");
        let mut call_args = vec![format!("\"wrap__{}\"", bridge_fn.name)];
        for p in &bridge_fn.params {
            call_args.push(p.clone());
        }
        call_args.push(format!("PACKAGE = \"{package_name}\""));
        let call_args_str = call_args.join(", ");

        // Hand-crafted roxygen block — we cannot reuse `r_roxygen_block` because
        // trait-bridge functions are not represented as `FunctionDef` in the IR.
        let kind = if bridge_fn.name.starts_with("register_") {
            "register"
        } else if bridge_fn.name.starts_with("unregister_") {
            "unregister"
        } else if bridge_fn.name.starts_with("clear_") {
            "clear"
        } else {
            ""
        };
        let roxygen_block = crate::backends::extendr::template_env::render(
            "r_trait_bridge_roxygen.jinja",
            minijinja::context! {
                name => &bridge_fn.name,
                kind => kind,
            },
        );

        out.push_str(&crate::backends::extendr::template_env::render(
            "r_free_function_wrapper.jinja",
            minijinja::context! {
                func_name => &bridge_fn.name,
                params_sig => params_sig,
                call_args_str => call_args_str,
                roxygen_block => roxygen_block,
            },
        ));
    }

    // Collect S3 method pairs once — used both for per-type forwarder emission below and
    // for the trailing generic block at the end of the file.
    let s3_pairs = collect_s3_methods(api, trait_bridge_fns, bridges);
    let s3_pairs_by_type: ahash::AHashMap<String, Vec<String>> = {
        let mut map: ahash::AHashMap<String, Vec<String>> = ahash::AHashMap::new();
        for (method_name, type_name) in &s3_pairs {
            map.entry(type_name.clone()).or_default().push(method_name.clone());
        }
        map
    };

    // Class env blocks. One per non-trait, non-extendr-incompatible type — matching the
    // set registered in `extendr_module! { impl Type; ... }`.
    let excluded = collect_excluded_class_types(api, bridges);
    for typ in &api.types {
        if typ.is_trait || excluded.contains(&typ.name) {
            continue;
        }

        let class_roxygen = r_class_roxygen_block(typ);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => &typ.name,
                roxygen_block => class_roxygen,
            },
        ));

        // Emit method bindings. Skip methods that are filtered out of the Rust impl
        // block — they have no `wrap__Type__method` symbol.
        for method in &typ.methods {
            if method_is_excluded_from_impl(method, api, bridges) {
                continue;
            }
            let params: Vec<String> = method.params.iter().map(|p| sanitize_r_param_name(&p.name)).collect();
            let params_sig = if method.is_static {
                params.join(", ")
            } else if params.is_empty() {
                "self".to_string()
            } else {
                format!("self, {}", params.join(", "))
            };
            let mut call_args = vec![format!(
                "\"wrap__{type_name}__{method_name}\"",
                type_name = typ.name,
                method_name = method.name,
            )];
            if !method.is_static {
                call_args.push("self".to_string());
            }
            for p in &params {
                call_args.push(p.clone());
            }
            call_args.push(format!("PACKAGE = \"{package_name}\""));
            let call_args_str = call_args.join(", ");

            out.push_str(&crate::backends::extendr::template_env::render(
                "r_method_binding.jinja",
                minijinja::context! {
                    type_name => &typ.name,
                    method_name => &method.name,
                    params_sig => params_sig,
                    call_args_str => call_args_str,
                },
            ));
        }

        // Synthetic from_json static factory: generated for every has_default non-opaque struct.
        // The Rust impl block adds `pub fn from_json(json: String) -> Result<Self>` which extendr
        // registers as `wrap__TypeName__from_json`. We emit the R wrapper here since from_json is
        // not part of the IR and gen_extendr_wrappers_r would otherwise skip it.
        if typ.has_default && !typ.fields.is_empty() && input_type_names.contains(&typ.name) {
            out.push_str(&crate::backends::extendr::template_env::render(
                "r_from_json_factory.jinja",
                minijinja::context! {
                    type_name => &typ.name,
                    package_name => package_name,
                },
            ));
        }

        // Dispatch operators: `instance$method` and `instance[["method"]]` resolve via
        // the class env. The dispatcher captures `self` so instance methods see it.
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => &typ.name },
        ));

        // S3 method forwarders: `is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)`.
        // Lets callers write `is_valid(meta)` instead of the env-class form `meta$is_valid()`,
        // hiding the extendr implementation detail behind idiomatic R generic dispatch.
        if let Some(method_names) = s3_pairs_by_type.get(&typ.name) {
            for method_name in method_names {
                out.push_str(&crate::backends::extendr::template_env::render(
                    "r_s3_method.jinja",
                    minijinja::context! { name => method_name, type_name => &typ.name },
                ));
            }
        }
    }

    // Flat data enum class env blocks — data enums are registered as structs in
    // extendr_module! and therefore need the same dispatch operator setup so R can
    // access fields via `instance$field_name`.
    for e in &api.enums {
        if !is_flat_data_enum(e) {
            continue;
        }
        let type_name = &e.name;
        let enum_roxygen = r_enum_roxygen_block(e, true);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => type_name,
                roxygen_block => enum_roxygen,
            },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
    }

    // Unit enum wrapper functions — simple enums with no data variants that are not
    // registered as extendr classes. Emit a function that returns the default variant.
    // R callers can write `ProcessingStage()` to get the default variant.
    for e in &api.enums {
        if is_flat_data_enum(e) || is_json_passthrough_data_enum(e) {
            continue;
        }
        // Only emit for unit enums (no data in any variant)
        let is_unit_enum = e.variants.iter().all(|v| v.fields.is_empty());
        if !is_unit_enum {
            continue;
        }

        let enum_name = &e.name;

        // Emit a simple wrapper function that returns the default variant as a list with class attribute.
        // This mirrors how structs are constructed via `TypeName$default()` in R.
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_unit_enum_wrapper.jinja",
            minijinja::context! { enum_name => enum_name },
        ));
    }

    // JSON-passthrough data enum class env blocks — these enums are also
    // registered as structs in extendr_module! with `default` and `from_json`
    // static methods. Emit the class env + method bindings + dispatchers so R
    // callers can write `EnumType$from_json("...")`.
    for e in &api.enums {
        if !is_json_passthrough_data_enum(e) {
            continue;
        }
        let type_name = &e.name;
        let enum_roxygen = r_enum_roxygen_block(e, false);
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_type_class_env.jinja",
            minijinja::context! {
                type_name => type_name,
                roxygen_block => enum_roxygen,
            },
        ));
        // `default` and `from_json` are emitted by `gen_extendr_json_passthrough_enum_struct`
        // as `pub fn` items in the `#[extendr] impl` block, so extendr registers them as
        // `wrap__<EnumType>__default` and `wrap__<EnumType>__from_json`. Bind them in R
        // by name so callers can use `EnumType$default()` / `EnumType$from_json(json)`.
        for method_name in ["default", "from_json"] {
            let params_sig = if method_name == "from_json" { "json" } else { "" };
            let mut call_args = vec![format!("\"wrap__{type_name}__{method_name}\"")];
            if method_name == "from_json" {
                call_args.push("json".to_string());
            }
            call_args.push(format!("PACKAGE = \"{package_name}\""));
            let call_args_str = call_args.join(", ");
            out.push_str(&crate::backends::extendr::template_env::render(
                "r_method_binding.jinja",
                minijinja::context! {
                    type_name => type_name,
                    method_name => method_name,
                    params_sig => params_sig,
                    call_args_str => call_args_str,
                },
            ));
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_dollar_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_bracket_dispatch.jinja",
            minijinja::context! { type_name => type_name },
        ));
    }

    // S3 generics: one `name <- function(x, ...) UseMethod("name")` per unique instance
    // method name across every emitted class. Emit last so all class methods they dispatch
    // over are already defined in source order.
    for generic_name in unique_s3_generic_names(&s3_pairs) {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_s3_generic.jinja",
            minijinja::context! { name => generic_name },
        ));
    }

    out
}

/// Sanitize a Rust parameter name for use in R code.
/// R identifiers cannot start with underscore, so we strip any leading underscore.
fn sanitize_r_param_name(name: &str) -> String {
    name.trim_start_matches('_').to_string()
}

fn r_wrapper_params_signature(params: &[ParamDef], api: &ApiSurface) -> String {
    let default_types: ahash::AHashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.has_default)
        .map(|t| t.name.as_str())
        .collect();
    params
        .iter()
        .map(|p| {
            let sanitized_name = sanitize_r_param_name(&p.name);
            if let TypeRef::Named(name) = &p.ty
                && default_types.contains(name.as_str())
            {
                format!("{} = {}$default()", sanitized_name, name)
            } else if p.optional || matches!(p.ty, TypeRef::Optional(_)) {
                format!("{} = NULL", sanitized_name)
            } else {
                sanitized_name
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Collect S3 (method_name, type_name) pairs for instance methods.
///
/// Instance methods get idiomatic R S3 wrappers — `is_valid(meta)` instead of `meta$is_valid()`
/// — so callers don't have to think about the env-class implementation detail. Static methods
/// (factories like `from_json`, `default`) are intentionally excluded: they're accessed
/// directly off the class env (`Type$from_json(json)`) and don't need a generic.
///
/// Method names that collide with free functions or trait-bridge functions are skipped to
/// avoid clobbering them with a generic that calls `UseMethod`.
fn collect_s3_methods(
    api: &ApiSurface,
    trait_bridge_fns: &[TraitBridgeFn],
    bridges: &[TraitBridgeConfig],
) -> Vec<(String, String)> {
    let excluded_types = collect_excluded_class_types(api, bridges);
    let mut reserved: ahash::AHashSet<String> = api.functions.iter().map(|f| f.name.clone()).collect();
    for bridge_fn in trait_bridge_fns {
        reserved.insert(bridge_fn.name.clone());
    }

    let mut pairs: Vec<(String, String)> = Vec::new();
    for typ in &api.types {
        if typ.is_trait || excluded_types.contains(&typ.name) {
            continue;
        }
        for method in &typ.methods {
            if method.is_static || method_is_excluded_from_impl(method, api, bridges) {
                continue;
            }
            if reserved.contains(&method.name) {
                continue;
            }
            pairs.push((method.name.clone(), typ.name.clone()));
        }
    }
    pairs
}

/// Unique generic names (sorted for deterministic emission) from a list of S3 method pairs.
fn unique_s3_generic_names(pairs: &[(String, String)]) -> Vec<String> {
    let mut names: Vec<String> = pairs.iter().map(|(name, _)| name.clone()).collect();
    names.sort();
    names.dedup();
    names
}

/// Generate `NAMESPACE` from the alef IR.
///
/// Lists every free function and every class dispatch operator (`$.Type`, `[[.Type`)
/// emitted by `gen_extendr_wrappers_r`. Without explicit `export()` entries, R loads
/// the wrapper file but treats the symbols as internal — calling code receives
/// `could not find function`.
fn gen_namespace(
    api: &ApiSurface,
    package_name: &str,
    trait_bridge_fns: &[TraitBridgeFn],
    r_exclude_functions: &ahash::AHashSet<String>,
    bridges: &[TraitBridgeConfig],
) -> String {
    let mut out = String::with_capacity(2 * 1024);
    out.push_str("# Generated by alef — do not edit.\n\n");
    // NAMESPACE requires the bare `useDynLib(...)` directive. The roxygen2 form
    // (`#' @useDynLib ...`) only takes effect when present in `.R` source files
    // processed by roxygen2 — emitting it directly into NAMESPACE leaves the
    // shared library unloaded and every `.Call` site fails at runtime.
    out.push_str(&crate::backends::extendr::template_env::render(
        "r_namespace_use_dyn_lib.jinja",
        minijinja::context! { package_name => package_name },
    ));
    out.push('\n');

    // Names emitted by the trait-bridge generator; skip them in the free-function
    // export pass to avoid duplicate `export(...)` lines in NAMESPACE.
    let bridge_fn_names: ahash::AHashSet<&str> = trait_bridge_fns.iter().map(|tb| tb.name.as_str()).collect();

    for func in &api.functions {
        if bridge_fn_names.contains(func.name.as_str()) {
            continue;
        }
        if r_exclude_functions.contains(&func.name) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &func.name },
        ));
    }

    // Trait-bridge functions need explicit NAMESPACE exports so that callers can use
    // them directly (e.g. `sample_core::register_text_backend(...)`). Without an `export()`
    // entry, R restricts the wrapper to internal-only visibility and `:: ` lookups fail.
    for bridge_fn in trait_bridge_fns {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &bridge_fn.name },
        ));
    }

    // Export the options helper function if an options-like input type exists.
    if find_r_options_type_from_api(api).is_some() {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => "conversion_options" },
        ));
    }

    let excluded = collect_excluded_class_types(api, bridges);
    for typ in &api.types {
        if typ.is_trait || excluded.contains(&typ.name) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &typ.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &typ.name },
        ));
    }

    // Flat data enums are registered as classes in extendr_module! and need exports too.
    for e in &api.enums {
        if !is_flat_data_enum(e) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &e.name },
        ));
    }

    // JSON-passthrough data enums also need NAMESPACE exports for the class
    // env and the `$`/`[[` dispatch operators.
    for e in &api.enums {
        if !is_json_passthrough_data_enum(e) {
            continue;
        }
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "$", name => &e.name },
        ));
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method.jinja",
            minijinja::context! { method_type => "[[", name => &e.name },
        ));
    }

    // S3 generics emitted in extendr-wrappers.R are exposed through the same NAMESPACE.
    // Without `export(name)` + `S3method(name, Type)` entries R loads the wrappers but
    // refuses to dispatch — `is_valid(meta)` raises `could not find function "is_valid"`.
    let s3_pairs = collect_s3_methods(api, trait_bridge_fns, bridges);
    for generic_name in unique_s3_generic_names(&s3_pairs) {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_export.jinja",
            minijinja::context! { name => &generic_name },
        ));
    }
    for (method_name, type_name) in &s3_pairs {
        out.push_str(&crate::backends::extendr::template_env::render(
            "r_namespace_s3method_named.jinja",
            minijinja::context! { method_name => method_name, type_name => type_name },
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::ExtendrBackend;
    use crate::core::backend::Backend;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::ir::*;

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
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,

                has_lifetime_params: false,
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
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
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
        // Expect: <package>.R (useDynLib stub), extendr-wrappers.R, NAMESPACE.
        let paths: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
        assert!(
            paths.iter().any(|p| p.ends_with("testlib.R")),
            "public API file must include {{package_name}}.R, got {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("extendr-wrappers.R")),
            "public API file must include extendr-wrappers.R, got {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("NAMESPACE")),
            "public API file must include NAMESPACE, got {paths:?}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_function_call_binding() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        assert!(
            wrappers.content.contains("process <- function()"),
            "free function must produce a wrapper: {}",
            wrappers.content
        );
        assert!(
            wrappers.content.contains(".Call(\"wrap__process\""),
            "wrapper must invoke the wrap__ symbol: {}",
            wrappers.content
        );
        assert!(
            wrappers.content.contains("Config <- new.env(parent = emptyenv())"),
            "non-trait class must be registered as an env: {}",
            wrappers.content
        );
    }

    #[test]
    fn extendr_wrappers_emits_roxygen_doc_block_for_free_functions() {
        // Regression: prior to roxygen2 doc emission, every free function in
        // extendr-wrappers.R carried only `#' @export` — `?<fn>` in an R REPL
        // returned an empty .Rd. The wrapper emitter must now derive a title
        // line + description from the Rust doc comment and emit `@param` /
        // `@return` lines from the IR's type information.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "extract_bytes".to_string(),
                rust_path: "test_lib::extract_bytes".to_string(),
                original_rust_path: String::new(),
                params: vec![
                    ParamDef {
                        name: "bytes".to_string(),
                        ty: TypeRef::Bytes,
                        optional: false,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "mime_type".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        optional: true,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                    ParamDef {
                        name: "config".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionConfig".to_string()))),
                        optional: true,
                        default: None,
                        sanitized: false,
                        typed_default: None,
                        is_ref: false,
                        is_mut: false,
                        newtype_wrapper: None,
                        original_type: None,
                        map_is_ahash: false,
                        map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                    },
                ],
                return_type: TypeRef::Named("ExtractionResult".to_string()),
                is_async: false,
                error_type: None,
                doc: "Extract text from raw bytes.\n\nDetect the MIME type of the input bytes\nand run the appropriate extractor.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
                unsupported_public_items: Vec::new(),
};
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;

        assert!(
            content.contains("#' Extract text from raw bytes"),
            "title line derived from Rust doc comment must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' Detect the MIME type of the input bytes"),
            "description from Rust doc comment must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' @param bytes Raw vector of bytes."),
            "@param for bytes must describe the type:\n{content}"
        );
        assert!(
            content.contains("#' @param mime_type Optional character string."),
            "@param for optional string must include `Optional` qualifier:\n{content}"
        );
        assert!(
            content.contains("#' @param config Optional ExtractionConfig object"),
            "@param for named optional type must reference the named type:\n{content}"
        );
        assert!(
            content.contains("extract_bytes <- function(bytes, mime_type = NULL, config = NULL)"),
            "R wrapper must allow README-style omitted optional config/mime args:\n{content}"
        );
        assert!(
            content.contains("#' @return ExtractionResult object"),
            "@return must describe the return type:\n{content}"
        );
        assert!(
            content.contains("#' @export"),
            "@export tag must be preserved:\n{content}"
        );
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("#' @param ") {
                let mut parts = rest.splitn(2, ' ');
                let _name = parts.next();
                let description = parts.next().unwrap_or("").trim();
                assert!(
                    !description.is_empty(),
                    "@param line must include a description, got: {line:?}\nfull content:\n{content}"
                );
            }
        }
    }

    #[test]
    fn extendr_wrappers_default_required_config_objects_in_r() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "ExtractionConfig".to_string(),
                rust_path: "test_lib::ExtractionConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,

                has_lifetime_params: false,
            }],
            functions: vec![FunctionDef {
                name: "extract_bytes".to_string(),
                rust_path: "test_lib::extract_bytes".to_string(),
                original_rust_path: String::new(),
                params: vec![
                    ParamDef {
                        name: "bytes".to_string(),
                        ty: TypeRef::Bytes,
                        ..Default::default()
                    },
                    ParamDef {
                        name: "config".to_string(),
                        ty: TypeRef::Named("ExtractionConfig".to_string()),
                        ..Default::default()
                    },
                ],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;

        assert!(
            content.contains("extract_bytes <- function(bytes, config = ExtractionConfig$default())"),
            "R wrapper must synthesize default objects instead of advertising NULL for required config:\n{content}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_placeholder_title_when_doc_is_empty() {
        // Functions with no Rust doc comment must still produce a complete
        // roxygen block — title falls back to the function name, description
        // is omitted, @param/@return lines are still emitted.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("#' process"),
            "fallback title (function name) must be emitted when doc is empty:\n{content}"
        );
        assert!(
            content.contains("#' @return Character string."),
            "@return must be emitted even without a doc comment:\n{content}"
        );
    }

    #[test]
    fn namespace_exports_functions_and_classes() {
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let namespace = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
            .expect("NAMESPACE must be generated");
        assert!(
            namespace.content.contains("export(process)"),
            "free function must be exported: {}",
            namespace.content
        );
        assert!(
            namespace.content.contains("export(Config)"),
            "class env must be exported: {}",
            namespace.content
        );
        assert!(
            namespace.content.contains("S3method(\"$\", Config)"),
            "S3 dispatch operator must be registered: {}",
            namespace.content
        );
        // NAMESPACE must use the bare `useDynLib(...)` directive — the roxygen2
        // form (`#' @useDynLib ...`) is silently ignored by R when placed in
        // NAMESPACE, leaving the .so unloaded and every `.Call` unresolved.
        assert!(
            namespace.content.contains("useDynLib(testlib, .registration = TRUE)"),
            "NAMESPACE must contain bare useDynLib directive: {}",
            namespace.content
        );
        assert!(
            !namespace.content.contains("#' @useDynLib"),
            "NAMESPACE must not contain roxygen2 useDynLib form: {}",
            namespace.content
        );
    }

    fn make_instance_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: TypeRef::Primitive(PrimitiveType::Bool),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            sanitized: false,
            receiver: Some(ReceiverKind::Ref),
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    fn make_api_with_instance_method() -> ApiSurface {
        ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "HeaderMetadata".to_string(),
                rust_path: "test_lib::HeaderMetadata".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("level", TypeRef::Primitive(PrimitiveType::U32), false)],
                methods: vec![make_instance_method("is_valid")],
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
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,

                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    #[test]
    fn extendr_wrappers_emits_s3_generic_and_method_for_instance_methods() {
        // Regression: bare env-class form `meta$is_valid()` leaks the extendr implementation
        // detail. Generate an S3 generic + class method so callers can write `is_valid(meta)`.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_with_instance_method();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("is_valid <- function(x, ...) UseMethod(\"is_valid\")"),
            "S3 generic must be emitted for instance methods:\n{content}"
        );
        assert!(
            content.contains("is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)"),
            "S3 class method must forward to the env-class binding:\n{content}"
        );
    }

    #[test]
    fn extendr_wrappers_skips_s3_wrappers_for_static_methods() {
        // Static factories like `default` / `from_json` are accessed off the class env
        // (`Type$from_json(json)`) — no `self`, no S3 forwarding needed.
        let backend = ExtendrBackend;
        let config = make_config();
        let mut api = make_api_with_instance_method();
        let static_method = MethodDef {
            is_static: true,
            ..make_instance_method("default")
        };
        api.types[0].methods.push(static_method);
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            !content.contains("default <- function(x, ...) UseMethod"),
            "must not emit S3 generic for static methods:\n{content}"
        );
        assert!(
            !content.contains("default.HeaderMetadata <-"),
            "must not emit S3 class method for static methods:\n{content}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_one_generic_per_unique_method_name() {
        // Two classes both expose `is_valid` — only one generic should be emitted to
        // avoid `UseMethod` being clobbered by a second definition.
        let backend = ExtendrBackend;
        let config = make_config();
        let mut api = make_api_with_instance_method();
        let second_type = TypeDef {
            name: "LinkMetadata".to_string(),
            rust_path: "test_lib::LinkMetadata".to_string(),
            methods: vec![make_instance_method("is_valid")],
            ..api.types[0].clone()
        };
        api.types.push(second_type);
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        let generic_count = content.matches("is_valid <- function(x, ...) UseMethod").count();
        assert_eq!(
            generic_count, 1,
            "exactly one S3 generic per unique method name, got {generic_count}:\n{content}"
        );
        assert!(
            content.contains("is_valid.HeaderMetadata <- function(x, ...) x$is_valid(...)"),
            "S3 method for HeaderMetadata must be emitted:\n{content}"
        );
        assert!(
            content.contains("is_valid.LinkMetadata <- function(x, ...) x$is_valid(...)"),
            "S3 method for LinkMetadata must be emitted:\n{content}"
        );
    }

    #[test]
    fn namespace_exports_s3_generics_and_methods_for_instance_methods() {
        // S3 generics + class methods emitted into extendr-wrappers.R need matching
        // `export(name)` + `S3method(name, Type)` NAMESPACE entries. Without them R
        // refuses to dispatch `is_valid(meta)` even though the function is loaded.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = make_api_with_instance_method();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let namespace = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
            .expect("NAMESPACE must be generated");
        let content = &namespace.content;
        assert!(
            content.contains("export(is_valid)"),
            "S3 generic must be exported by name: {content}"
        );
        assert!(
            content.contains("S3method(is_valid, HeaderMetadata)"),
            "S3 class method must be registered: {content}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_roxygen_class_block_with_field_lines_for_struct() {
        // Class envs (`Type <- new.env(parent = emptyenv())`) must carry a roxygen2
        // block derived from the struct's Rust doc comment: a title line, an
        // optional description, one `@field` per public field (with the field's
        // own doc comment as the description), and an `@export` tag.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "ServerConfig".to_string(),
                rust_path: "test_lib::ServerConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![
                    FieldDef {
                        doc: "TCP port the server binds to.".to_string(),
                        ..make_field("port", TypeRef::Primitive(PrimitiveType::U32), false)
                    },
                    FieldDef {
                        doc: "Maximum number of in-flight requests.\n\nApplies to all listener sockets.".to_string(),
                        ..make_field("max_connections", TypeRef::Primitive(PrimitiveType::U32), false)
                    },
                ],
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
                doc: "Server configuration.\n\nHolds tunable parameters for the network listener.".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,

                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("#' Server configuration"),
            "class title from struct doc must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' Holds tunable parameters for the network listener."),
            "class description must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' @field port TCP port the server binds to."),
            "@field with single-line doc must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' @field max_connections Maximum number of in-flight requests."),
            "@field must collapse multi-paragraph doc to the first paragraph:\n{content}"
        );
        // The class env line must follow the roxygen block.
        assert!(
            content.contains("ServerConfig <- new.env(parent = emptyenv())"),
            "class env definition must still be emitted:\n{content}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_param_doc_from_arguments_section_for_function() {
        // When a free function's Rust doc carries a `# Arguments` section, the
        // per-param description from the bullet list must override the default
        // type-based description on the `#' @param` line, and the `# Returns`
        // section must drive the `#' @return` line.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![FunctionDef {
                name: "render".to_string(),
                rust_path: "test_lib::render".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "template".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    sanitized: false,
                    typed_default: None,
                    is_ref: false,
                    is_mut: false,
                    newtype_wrapper: None,
                    original_type: None,
                    map_is_ahash: false,
                    map_key_is_cow: false,
            vec_inner_is_ref: false,
                    map_is_btree: false,
                    core_wrapper: crate::core::ir::CoreWrapper::None,
                }],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "Render a template to a string.\n\n# Arguments\n\n* `template` - Mustache template source.\n\n# Returns\n\nThe fully interpolated output.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
                unsupported_public_items: Vec::new(),
};
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("#' @param template Mustache template source."),
            "@param must use description from `# Arguments` bullet:\n{content}"
        );
        assert!(
            content.contains("#' @return The fully interpolated output."),
            "@return must use prose from `# Returns` section:\n{content}"
        );
        // The raw `# Arguments` / `# Returns` headings must not leak into the
        // description body now that they're rendered as roxygen tags.
        assert!(
            !content.contains("#' # Arguments"),
            "raw `# Arguments` heading must not appear in roxygen output:\n{content}"
        );
        assert!(
            !content.contains("#' # Returns"),
            "raw `# Returns` heading must not appear in roxygen output:\n{content}"
        );
    }

    #[test]
    fn extendr_wrappers_emits_roxygen_block_for_flat_data_enum_with_variant_fields() {
        // Flat data enums (single-field tuple variants) are surfaced in R as
        // class envs with one scalar field per variant. The class env must
        // carry roxygen with one `@field` per variant carrying the variant's
        // Rust doc as description.
        let backend = ExtendrBackend;
        let config = make_config();
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![EnumDef {
                name: "Payload".to_string(),
                rust_path: "test_lib::Payload".to_string(),
                original_rust_path: String::new(),
                variants: vec![
                    EnumVariant {
                        name: "Text".to_string(),
                        fields: vec![make_field("inner", TypeRef::String, false)],
                        doc: "UTF-8 encoded text payload.".to_string(),
                        is_default: false,
                        serde_rename: None,
                        is_tuple: true,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        originally_had_data_fields: false,
                    },
                    EnumVariant {
                        name: "Binary".to_string(),
                        fields: vec![make_field("inner", TypeRef::String, false)],
                        doc: "Base64-encoded binary payload.".to_string(),
                        is_default: false,
                        serde_rename: None,
                        is_tuple: true,
                        binding_excluded: false,
                        binding_exclusion_reason: None,
                        originally_had_data_fields: false,
                    },
                ],
                doc: "Wire payload variants.".to_string(),
                cfg: None,
                is_copy: false,
                has_serde: false,
                serde_tag: None,
                serde_untagged: false,
                serde_rename_all: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                excluded_variants: vec![],
            }],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("#' Wire payload variants"),
            "enum title from Rust doc must be emitted:\n{content}"
        );
        assert!(
            content.contains("#' @field Text UTF-8 encoded text payload."),
            "@field per variant must carry the variant's doc:\n{content}"
        );
        assert!(
            content.contains("#' @field Binary Base64-encoded binary payload."),
            "every variant must produce a `@field` line:\n{content}"
        );
        assert!(
            content.contains("Payload <- new.env(parent = emptyenv())"),
            "enum class env must still be emitted:\n{content}"
        );
    }

    fn trait_bridge_config_for_tests() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "test_lib::Plugin"
registry_getter = "test_lib::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
"#,
        )
    }

    #[test]
    fn extendr_module_registers_trait_bridge_register_unregister_clear() {
        // Regression: register_<trait> / unregister_<trait> / clear_<trait> are emitted
        // as `#[extendr]` functions by the trait-bridge generator but were missing from
        // the `extendr_module!` block, so the wrap__<symbol> entry points never reached
        // the .so and R callers could not invoke them.
        let backend = ExtendrBackend;
        let config = trait_bridge_config_for_tests();
        let api = make_api_surface();
        let files = backend.generate_bindings(&api, &config).unwrap();
        let lib_rs = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
            .expect("lib.rs must be generated");
        for sym in ["register_ocr_backend", "unregister_ocr_backend", "clear_ocr_backends"] {
            assert!(
                lib_rs.content.contains(&format!("fn {sym};")),
                "extendr_module! must register `{sym}`:\n{}",
                lib_rs.content
            );
        }
    }

    #[test]
    fn extendr_wrappers_emits_trait_bridge_register_unregister_clear() {
        // Regression: extendr-wrappers.R only iterated `api.functions` and so omitted
        // the trait-bridge register/unregister/clear functions. R callers had no way to
        // invoke `wrap__register_text_backend` because no R wrapper existed.
        let backend = ExtendrBackend;
        let config = trait_bridge_config_for_tests();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;
        assert!(
            content.contains("register_ocr_backend <- function(r_backend) .Call(\"wrap__register_ocr_backend\""),
            "register wrapper must accept an R object and call wrap__register_ocr_backend:\n{content}"
        );
        assert!(
            content.contains("unregister_ocr_backend <- function(name) .Call(\"wrap__unregister_ocr_backend\""),
            "unregister wrapper must accept a name and call wrap__unregister_ocr_backend:\n{content}"
        );
        assert!(
            content.contains("clear_ocr_backends <- function() .Call(\"wrap__clear_ocr_backends\""),
            "clear wrapper must take no arguments:\n{content}"
        );
    }

    #[test]
    fn namespace_exports_trait_bridge_register_unregister_clear() {
        // Regression: without explicit `export()` entries in NAMESPACE, the
        // trait-bridge wrappers would be loaded internally but unreachable via
        // `pkg::register_<trait>(...)`.
        let backend = ExtendrBackend;
        let config = trait_bridge_config_for_tests();
        let api = make_api_surface();
        let files = backend.generate_public_api(&api, &config).unwrap();
        let namespace = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
            .expect("NAMESPACE must be generated");
        for sym in ["register_ocr_backend", "unregister_ocr_backend", "clear_ocr_backends"] {
            assert!(
                namespace.content.contains(&format!("export({sym})")),
                "NAMESPACE must export `{sym}`:\n{}",
                namespace.content
            );
        }
    }

    #[test]
    fn extendr_excludes_trait_bridge_functions_when_language_excluded() {
        // The bridge structs already honour `exclude_languages`. Their register / unregister /
        // clear free functions must follow the same gate so the module/wrappers/namespace stay in sync.
        let config = resolved_one(
            r#"
[workspace]
languages = ["r"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.r]
package_name = "testlib"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "test_lib::Plugin"
registry_getter = "test_lib::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
unregister_fn = "unregister_ocr_backend"
clear_fn = "clear_ocr_backends"
exclude_languages = ["r"]
"#,
        );
        let collected = super::collect_trait_bridge_functions(&config);
        assert!(
            collected.is_empty(),
            "no trait-bridge entries should be collected when r is excluded: {:?}",
            collected.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn regression_namespace_exports_functions_types_enums() {
        // Regression test: Verify that NAMESPACE exports ALL functions, types, and enums.
        // A bug caused NAMESPACE to only contain `useDynLib(...)` with no exports.
        let backend = ExtendrBackend;
        let config = make_config();
        let mut api = make_api_surface();
        // Add extra exported types and enums to exercise namespace completeness.
        api.types.push(TypeDef {
            name: "DocumentMetadata".to_string(),
            rust_path: "test_lib::DocumentMetadata".to_string(),
            original_rust_path: String::new(),
            fields: vec![make_field("title", TypeRef::String, true)],
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
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,

            has_lifetime_params: false,
        });
        // Add a flat data enum (has variant with data, single field)
        api.enums.push(EnumDef {
            name: "ConversionResult".to_string(),
            rust_path: "test_lib::ConversionResult".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Ok".to_string(),
                    fields: vec![make_field("content", TypeRef::String, false)],
                    is_default: false,
                    serde_rename: None,
                    is_tuple: true,
                    doc: String::new(),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                },
                EnumVariant {
                    name: "Err".to_string(),
                    fields: vec![make_field("msg", TypeRef::String, false)],
                    is_default: false,
                    serde_rename: None,
                    is_tuple: true,
                    doc: String::new(),
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                    originally_had_data_fields: false,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            excluded_variants: vec![],
        });
        let files = backend.generate_public_api(&api, &config).unwrap();
        let namespace = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("NAMESPACE"))
            .expect("NAMESPACE must be generated");
        let content = &namespace.content;
        // Check for the useDynLib line
        assert!(
            content.contains("useDynLib(testlib, .registration = TRUE)"),
            "NAMESPACE must have useDynLib: {content}"
        );
        // Check for function exports
        assert!(
            content.contains("export(process)"),
            "NAMESPACE must export free functions, got: {content}"
        );
        // Check for type exports
        assert!(
            content.contains("export(Config)"),
            "NAMESPACE must export types like Config: {content}"
        );
        assert!(
            content.contains("export(DocumentMetadata)"),
            "NAMESPACE must export DocumentMetadata: {content}"
        );
        // Check for enum exports (flat data enums)
        assert!(
            content.contains("export(ConversionResult)"),
            "NAMESPACE must export flat data enums: {content}"
        );
        // Make sure NAMESPACE is NOT just 2 lines (the bug symptom)
        let line_count = content.lines().count();
        assert!(
            line_count > 10,
            "NAMESPACE should have many more than 10 lines, got {line_count}: {content}"
        );
    }

    #[test]
    fn r_field_long_descriptions_are_truncated_to_fit_120_char_lines() {
        // Ensure roxygen2 @field lines don't exceed 120 chars to satisfy lintr.
        // Each @field line has format: "#' @field <name> <description>"
        // which is 10 + len(name) + 1 + len(description) chars.
        // So description must be truncated to fit within 120 total.
        let backend = ExtendrBackend;
        let config = make_config();
        let long_doc = "Open Graph metadata (og:* properties) for social media Keys like \"title\", \"description\", \"image\", \"url\", etc.";
        let api = ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![TypeDef {
                name: "DocumentMetadata".to_string(),
                rust_path: "test_lib::DocumentMetadata".to_string(),
                original_rust_path: String::new(),
                fields: vec![FieldDef {
                    doc: long_doc.to_string(),
                    ..make_field("open_graph", TypeRef::String, true)
                }],
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
                doc: "Document metadata".to_string(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,

                has_lifetime_params: false,
            }],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let files = backend.generate_public_api(&api, &config).unwrap();
        let wrappers = files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("extendr-wrappers.R"))
            .expect("extendr-wrappers.R must be generated");
        let content = &wrappers.content;

        // Find the @field line and verify it's under 120 chars.
        for line in content.lines() {
            if line.contains("@field open_graph") {
                assert!(
                    line.len() <= 120,
                    "@field line must be <= 120 chars, got {} chars: {}",
                    line.len(),
                    line
                );
                // Also verify it's not just truncated to empty — should have real description.
                assert!(
                    line.contains("Open Graph metadata"),
                    "@field description was over-truncated: {}",
                    line
                );
                return;
            }
        }
        panic!("Could not find @field open_graph line in:\n{}", content);
    }
}
