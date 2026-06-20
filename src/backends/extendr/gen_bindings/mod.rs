mod bridges;
mod enum_conversions;
mod options;
mod r_wrappers;
pub mod service_api;
mod trait_bridge_wrappers;
mod type_mapping;

use self::trait_bridge_wrappers::{collect_trait_bridge_fn_names, collect_trait_bridge_functions};
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::generators::trait_bridge::find_bridge_field;
use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeRef};
use ahash::AHashSet;
use std::path::PathBuf;

/// Prepend `#[cfg(<pred>)]` to a code item when the source symbol carries a cfg predicate.
fn prepend_cfg(cfg: Option<&str>, item: String) -> String {
    match cfg {
        Some(pred) if !pred.is_empty() => format!("#[cfg({pred})]\n{item}"),
        _ => item,
    }
}

pub struct ExtendrBackend;

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
        // extendr emits a single compiled Rust surface with no Rust-cfg gating, so same-named
        // cfg-variant functions must collapse to one definition to avoid E0428 (defined multiple
        // times). See codegen::fn_dedup.
        let deduped_api = api.with_deduped_functions();
        let api = &deduped_api;
        let core_import = config.core_import_name();
        // Build type path map for resolving fully-qualified paths to enums not re-exported at crate root
        let type_paths = build_type_path_lookup(api);
        // Compute flat data enum names first so binding_config and conversion config can use them.
        let flat_data_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| bridges::is_flat_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        // JSON-passthrough wrapper structs need an `impl Name;` entry in
        // `extendr_module!` so their `#[extendr] impl` block (with `default`,
        // `from_json`) is wired into the R package's class registry.
        let json_passthrough_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| bridges::is_json_passthrough_data_enum(e))
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

        // Helper: returns true if a method's RETURN type cannot be auto-converted into `Robj` by
        // extendr, so the method must be excluded from the `#[extendr]` impl block.
        //
        // Extendr's automatic return conversions cover bare structs/enums/opaque handles (via
        // ExternalPtr), primitives, String, and `Vec<primitive/String>`. They do NOT cover:
        //   • `Option<Named>` — there is no `From<Option<ExternalPtr<T>>> for Robj`; the auto-impl
        //     requires `T: ToVectorValue`, which structs/opaque handles don't implement
        //     (e.g. `Registry::get -> Option<Preset>`).
        //   • `Vec<Named>` — no `From<Vec<LocalStruct>> for Robj` (e.g. `summaries -> Vec<PresetSummary>`).
        //   • `Option<Vec<_>>` — `Option<Vec<u8>>`/`Option<Vec<primitive>>`/`Option<Vec<Named>>` all
        //     fail `ToVectorValue` (e.g. `Registry::sample_bytes -> Option<Vec<u8>>`).
        // These returns are dropped rather than JSON-bridged because method delegation through the
        // shared opaque/struct impl generators has no JSON-return path; callers reach the same data
        // via the free-function / serialised-struct surfaces.
        let method_return_unsupported = |m: &crate::core::ir::MethodDef| -> bool {
            match &m.return_type {
                // Vec<Named> (struct/enum element) — no R-list conversion.
                crate::core::ir::TypeRef::Vec(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(_))
                }
                crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                    // Option<Named> — Option<ExternalPtr<T>> has no Robj conversion.
                    crate::core::ir::TypeRef::Named(_) => true,
                    // Option<Vec<_>>/Option<Bytes> — Vec<u8>/Vec<primitive>/Vec<Named> all fail
                    // ToVectorValue. `Vec<u8>` is modelled as `TypeRef::Bytes` in the IR.
                    crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Bytes => true,
                    _ => false,
                },
                _ => false,
            }
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
                //   • methods returning Option<Named>/Vec<Named>/Option<Vec<_>> (no Robj conversion)
                let has_excluded_opaque_methods = typ.methods.iter().any(|m| {
                    method_references_arc_incompatible(m)
                        || method_references_enum(m)
                        || method_references_map(m)
                        || method_return_unsupported(m)
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
                                    && !method_return_unsupported(m)
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
                    method_references_arc_incompatible(m)
                        || method_references_enum(m)
                        || method_references_map(m)
                        || method_return_unsupported(m)
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
                                        && !method_return_unsupported(m)
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
            if bridges::is_flat_data_enum(e) {
                // Data enums with all-tuple variants become flat structs so bindings can
                // access variant data with dot notation (e.g. result$format$excel$sheet_count).
                // The #[extendr] attribute registers it as a class; the impl block satisfies
                // extendr_module! even though the struct has no methods.
                let flat_struct = bridges::gen_extendr_flat_data_enum_struct(e, self, &cfg);
                builder.add_item(&format!("#[extendr]\n{flat_struct}"));
                builder.add_item(&format!("#[extendr]\nimpl {} {{}}", e.name));
            } else if bridges::is_json_passthrough_data_enum(e) {
                // Tagged data enums with struct variants (e.g. `EmbeddingModelType`
                // with `Preset { name: String }`) cannot be expressed losslessly as
                // flat unit-only enums — the variant payload is dropped on round
                // trip. Generate a newtype struct whose serde representation defers
                // entirely to the core enum, so nested deserialization through parent
                // structs preserves the full tagged payload across the FFI boundary.
                builder.add_item(&bridges::gen_extendr_json_passthrough_enum_struct(e, &core_import));
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
            bridges::collect_named_types_into(&func.return_type, &mut all_surface_types);
        }

        // Add types from method return types
        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                bridges::collect_named_types_into(&method.return_type, &mut all_surface_types);
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
                        bridges::collect_named_types_into(&field.ty, &mut field_types);
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
                            bridges::collect_named_types_into(&field.ty, &mut field_types);
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
            .filter(|e| bridges::is_flat_data_enum(e) && !bridges::can_flat_data_enum_round_trip(e))
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
            if bridges::is_flat_data_enum(e) {
                // Flat data enum: always generate From<core> impl so containing structs can
                // convert core values. Struct variant data is lost (extendr can only represent
                // tuple variants as fields), but this is acceptable for output-only types like
                // FormatMetadata or policy enums like VlmFallbackPolicy.
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    // Generate dedicated From<core::Enum> impl.
                    builder.add_item(&bridges::gen_extendr_flat_data_enum_from_core(e, &core_import));
                    // Also generate the reverse for flat data enums whose tuple variant fields are
                    // all primitive/String types (so binding→core round-trip works).
                    // Output-only enums like FormatMetadata have complex output-only variant types
                    // (PdfMetadata, DocxMetadata, ...) and are excluded.
                    if bridges::can_flat_data_enum_round_trip(e) {
                        builder.add_item(&bridges::gen_extendr_flat_data_enum_to_core(e, &core_import));
                    }
                }
                // binding→core is only generated for round-trip-safe flat data enums above.
            } else if bridges::is_json_passthrough_data_enum(e) {
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
                    builder.add_item(&enum_conversions::gen_from_binding_to_core(
                        e,
                        &core_import,
                        &type_paths,
                    ));
                }
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&enum_conversions::gen_from_core_to_binding(
                        e,
                        &core_import,
                        &type_paths,
                    ));
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
                let item = crate::backends::extendr::trait_bridge::gen_bridge_function(
                    api,
                    func,
                    param_idx,
                    bridge_cfg,
                    self,
                    &opaque_types,
                    &core_import,
                );
                let item = prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else if let Some(bm) = bridge_field {
                // Function has a bridge field binding (e.g., visitor on options)
                let item = bridges::gen_extendr_bridge_field_function(api, func, &bm, &core_import);
                let item = prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else {
                // Detect functions whose return type or parameter types are incompatible
                // with extendr's automatic Robj conversions. These need JSON bridging.
                let func_return_needs_json = bridges::return_type_needs_json(
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
                        // Required bare non-opaque structs need JSON bridging only when
                        // named_non_opaque_params_by_ref is false. When true, extendr can convert &Robj → &T.
                        // Optional non-opaque structs still need JSON bridging since gen_function has a bug
                        // with Optional(Named) types (generates Option<Option<T>>) until that's fixed.
                        || (!cfg.named_non_opaque_params_by_ref && (
                            matches!(&p.ty, crate::core::ir::TypeRef::Named(n)
                                if !opaque_types.contains(n.as_str())
                                    && !enum_names.contains(n.as_str())
                                    && !extendr_incompatible_types.contains(n.as_str())
                                    && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n))
                            || matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
                                if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n)
                                    if !opaque_types.contains(n.as_str())
                                        && !enum_names.contains(n.as_str())
                                        && !extendr_incompatible_types.contains(n.as_str())
                                        && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n)))))
                        || (cfg.named_non_opaque_params_by_ref && matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
                            if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n)
                                if !opaque_types.contains(n.as_str())
                                    && !enum_names.contains(n.as_str())
                                    && !extendr_incompatible_types.contains(n.as_str())
                                    && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n))))
                });
                if func_return_needs_json || func_params_need_json {
                    let item = bridges::gen_extendr_json_bridged_function(
                        func,
                        self,
                        &core_import,
                        &opaque_types,
                        &cfg,
                        &extendr_incompatible_types,
                        &enum_names,
                    );
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
                } else {
                    let item = generators::gen_function(func, self, &cfg, &adapter_bodies, &opaque_types);
                    let item = prepend_cfg(func.cfg.as_deref(), item);
                    builder.add_item(&item);
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
                // The `extendr_module!` macro parser accepts ONLY bare `mod`/`fn`/`impl`/`use`
                // entries — it rejects any `#[cfg(...)]` attribute with "expected mod, fn or impl"
                // (extendr-macros `Module::parse`). The R crate enables every gated feature, so the
                // cfg-gated fn definitions always exist; the module entry is therefore emitted
                // unconditionally regardless of the function's cfg.
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
        let wrappers_content = r_wrappers::gen_extendr_wrappers_r(
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
        let namespace_content = r_wrappers::gen_namespace(
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
        if let Some(opts_type) = options::find_r_options_type(api, config) {
            let options_r = options::gen_conversion_options_r(opts_type);
            files.push(GeneratedFile {
                path: PathBuf::from(&r_wrapper_dir).join("options.R"),
                content: options_r,
                generated_header: true,
            });
        }

        // options.rs (Rust-side): generated decode_options function that handles ExternalPtr-wrapped
        // binding types from the configured/options-like type's default() and similar constructors.
        if let Some(opts_type) = options::find_r_options_type(api, config) {
            let core_import = config.core_import_name();
            let options_rs = options::gen_options_rs(api, opts_type, &core_import);
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

#[cfg(test)]
mod tests;
