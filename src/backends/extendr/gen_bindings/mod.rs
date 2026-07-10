mod bridges;
mod cfg_registration;
mod enum_conversions;
mod options;
mod r_package;
mod r_wrappers;
pub mod service_api;
mod trait_bridge_wrappers;
mod type_mapping;

use self::cfg_registration::{always_registered, apply_r_cfg_field_policy, effective_r_cfg_features, prepend_cfg};
use self::trait_bridge_wrappers::{collect_trait_bridge_fn_names, collect_trait_bridge_functions};
use crate::codegen::builder::RustFileBuilder;
use crate::codegen::generators;
use crate::codegen::generators::trait_bridge::find_bridge_field;
use crate::codegen::generators::type_paths::build_type_path_lookup;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::backend::{Backend, BuildConfig, BuildDependency, Capabilities, GeneratedFile};
use crate::core::config::{Language, ResolvedCrateConfig, resolve_output_dir};
use crate::core::ir::{ApiSurface, TypeRef};
use std::path::PathBuf;

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
        let deduped_api = api.with_deduped_functions();
        let enabled_features = effective_r_cfg_features(&deduped_api, config);
        let r_cfg_api = apply_r_cfg_field_policy(&deduped_api, &enabled_features);
        let api = &r_cfg_api;
        let core_import = config.core_import_name();
        let type_paths = build_type_path_lookup(api);
        let flat_data_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| bridges::is_flat_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        // `extendr_module!` so their `#[extendr] impl` block (with `default`,
        let json_passthrough_enum_names_vec: Vec<String> = api
            .enums
            .iter()
            .filter(|e| bridges::is_json_passthrough_data_enum(e))
            .map(|e| e.name.clone())
            .collect();
        let cfg = Self::binding_config(&core_import, &flat_data_enum_names_vec);

        let adapter_bodies = crate::adapters::build_adapter_bodies(config, Language::R)?;

        let mut builder = RustFileBuilder::new().with_generated_header();
        builder.add_inner_attribute("allow(dead_code, unused_imports, unused_variables)");
        builder.add_inner_attribute("allow(clippy::too_many_arguments, clippy::let_unit_value, clippy::needless_borrow, clippy::map_identity, clippy::just_underscores_and_digits, clippy::unused_unit, clippy::unnecessary_cast, clippy::unwrap_or_default, clippy::derivable_impls, clippy::needless_borrows_for_generic_args, clippy::unnecessary_fallible_conversions)");
        if let Some(extra_attr) = crate::codegen::shared::format_extra_clippy_allows(
            &config.extra_clippy_allows,
            builder.inner_attributes_text(),
        ) {
            builder.add_inner_attribute(&extra_attr);
        }
        builder.add_import("extendr_api::prelude::*");
        builder.add_import("std::collections::HashMap");
        // Use extendr's Result<T> type alias (Result<T, Error>) in generated #[extendr] functions
        builder.add_import("extendr_api::Result");

        for trait_path in generators::collect_trait_imports(api) {
            builder.add_import(&trait_path);
        }

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

        let references_arc_incompatible = |ty: &crate::core::ir::TypeRef| -> bool {
            arc_incompatible_opaque_vec.iter().any(|n| {
                matches!(ty, crate::core::ir::TypeRef::Named(name) if name == n)
                    || matches!(ty, crate::core::ir::TypeRef::Optional(inner) if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(name) if name == n))
            })
        };
        let method_references_arc_incompatible = |m: &crate::core::ir::MethodDef| -> bool {
            references_arc_incompatible(&m.return_type) || m.params.iter().any(|p| references_arc_incompatible(&p.ty))
        };

        let references_enum = |ty: &crate::core::ir::TypeRef| -> bool {
            match ty {
                crate::core::ir::TypeRef::Named(n) => enum_names.contains(n.as_str()),
                crate::core::ir::TypeRef::Optional(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n) if enum_names.contains(n.as_str()))
                }
                _ => false,
            }
        };

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

        //     is not generated by #[extendr] on the struct definition).
        let method_references_enum = |m: &crate::core::ir::MethodDef| -> bool {
            references_enum(&m.return_type)
                || m.params
                    .iter()
                    .any(|p| references_enum(&p.ty) || param_is_owned_struct(&p.ty))
        };

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

        // extendr, so the method must be excluded from the `#[extendr]` impl block.
        let method_return_unsupported = |m: &crate::core::ir::MethodDef| -> bool {
            match &m.return_type {
                crate::core::ir::TypeRef::Vec(inner) => {
                    matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(_))
                }
                crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
                    crate::core::ir::TypeRef::Named(_) => true,
                    crate::core::ir::TypeRef::Vec(_) | crate::core::ir::TypeRef::Bytes => true,
                    _ => false,
                },
                _ => false,
            }
        };

        //   • Enum types — ExternalPtr-backed after #[extendr] on enum.
        let is_extendr_native_incompatible = |ty: &crate::core::ir::TypeRef| -> bool {
            let is_vec_element_incompatible =
                |n: &str| !opaque_types.contains(n) && !arc_incompatible_opaque.contains(n);
            match ty {
                crate::core::ir::TypeRef::Vec(inner) => match inner.as_ref() {
                    crate::core::ir::TypeRef::Named(n) if is_vec_element_incompatible(n) => true,
                    crate::core::ir::TypeRef::Vec(_) => true,
                    _ => false,
                },
                crate::core::ir::TypeRef::Optional(inner) => {
                    if let crate::core::ir::TypeRef::Vec(inner2) = inner.as_ref() {
                        match inner2.as_ref() {
                            crate::core::ir::TypeRef::Named(n) if is_vec_element_incompatible(n) => true,
                            crate::core::ir::TypeRef::Vec(_) => true,
                            _ => false,
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            }
        };

        // A method cannot be exposed in a #[extendr] impl if any parameter is a type extendr
        let method_params_use_extendr_incompatible =
            |m: &crate::core::ir::MethodDef| -> bool { m.params.iter().any(|p| is_extendr_native_incompatible(&p.ty)) };

        // from #[extendr] impl block generation and from extendr_module! registration.
        let extendr_incompatible_types: ahash::AHashSet<String> = api
            .types
            .iter()
            .filter(|t| !t.is_opaque && !t.is_trait)
            .filter(|t| t.fields.iter().any(|f| is_extendr_native_incompatible(&f.ty)))
            .map(|t| t.name.clone())
            .collect();

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

        let has_arc_compatible = opaque_types.iter().any(|n| !arc_incompatible_opaque.contains(n));
        if has_arc_compatible {
            builder.add_import("std::sync::Arc");
        }

        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if typ.is_opaque {
                if arc_incompatible_opaque.contains(&typ.name) {
                    continue;
                }
                // Applying #[extendr] to the struct generates TryFrom<&Robj> for &Foo and
                let opaque_struct = generators::gen_opaque_struct(typ, &cfg);
                builder.add_item(&format!("#[extendr]\n{opaque_struct}"));
                let has_excluded_opaque_methods = typ.methods.iter().any(|m| {
                    method_references_arc_incompatible(m)
                        || method_references_enum(m)
                        || method_references_map(m)
                        || method_return_unsupported(m)
                        || method_params_use_extendr_incompatible(m)
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
                                    && !method_params_use_extendr_incompatible(m)
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
                    // meta__TypeName function is generated by the #[extendr] proc-macro.
                    builder.add_item(&format!("#[extendr]\nimpl {} {{}}", typ.name));
                }
            } else {
                let has_excluded_fields = typ.fields.iter().any(|f| references_arc_incompatible(&f.ty));
                let has_excluded_methods = typ.methods.iter().any(|m| {
                    method_references_arc_incompatible(m)
                        || method_references_enum(m)
                        || method_references_map(m)
                        || method_return_unsupported(m)
                        || method_params_use_extendr_incompatible(m)
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
                                        && !method_params_use_extendr_incompatible(m)
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

                if extendr_incompatible_types.contains(&struct_typ.name) {
                    // #[extendr] impl and extendr_module! entry are omitted. The struct is
                    builder.add_item(&generators::gen_struct(&struct_typ, self, &cfg));
                } else {
                    // Applying #[extendr] to the struct generates the conversions needed
                    let struct_item = generators::gen_struct(&struct_typ, self, &cfg);
                    builder.add_item(&format!("#[extendr]\n{struct_item}"));
                    let from_json_method = if struct_typ.has_default
                        && !struct_typ.fields.is_empty()
                        && input_type_names.contains(&struct_typ.name)
                    {
                        let type_name = &struct_typ.name;
                        let core_path = struct_typ.rust_path.replace('-', "_");
                        // #[serde(default)] on all fields, etc.) applies. The binding has
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
                        let empty_or_from_json = if from_json_method.is_empty() {
                            format!("#[extendr]\nimpl {} {{}}", struct_typ.name)
                        } else {
                            format!("#[extendr]\nimpl {} {{\n{}}}", struct_typ.name, from_json_method)
                        };
                        builder.add_item(&empty_or_from_json);
                    }
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

        for e in &api.enums {
            if bridges::is_flat_data_enum(e) {
                // The #[extendr] attribute registers it as a class; the impl block satisfies
                let flat_struct = bridges::gen_extendr_flat_data_enum_struct(e, self, &cfg);
                builder.add_item(&format!("#[extendr]\n{flat_struct}"));
                builder.add_item(&format!("#[extendr]\nimpl {} {{}}", e.name));
            } else if bridges::is_json_passthrough_data_enum(e) {
                builder.add_item(&bridges::gen_extendr_json_passthrough_enum_struct(
                    e,
                    self,
                    &core_import,
                ));
            } else {
                builder.add_item(&generators::gen_enum(e, &cfg));
            }
        }

        let binding_to_core = crate::codegen::conversions::convertible_types(api);
        let core_to_binding = crate::codegen::conversions::core_to_binding_convertible_types(api, &[]);
        let input_types = crate::codegen::conversions::input_type_names(api);

        let mut all_surface_types: ahash::AHashSet<String> = input_types.clone();

        for func in &api.functions {
            bridges::collect_named_types_into(&func.return_type, &mut all_surface_types);
        }

        for typ in api.types.iter().filter(|t| !t.is_trait) {
            for method in &typ.methods {
                bridges::collect_named_types_into(&method.return_type, &mut all_surface_types);
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            let snapshot: Vec<String> = all_surface_types.iter().cloned().collect();
            for name in &snapshot {
                if let Some(typ) = api.types.iter().find(|t| t.name == *name) {
                    for field in &typ.fields {
                        let mut field_types = ahash::AHashSet::new();
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
                            let mut field_types = ahash::AHashSet::new();
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

        let non_round_trip_flat_enums: Vec<String> = api
            .enums
            .iter()
            .filter(|e| bridges::is_flat_data_enum(e) && !bridges::can_flat_data_enum_round_trip(e))
            .map(|e| e.name.clone())
            .collect();
        let extendr_conversion_cfg = crate::codegen::conversions::ConversionConfig {
            cast_uints_to_i32: true,
            cast_large_ints_to_f64: true,
            exclude_types: &arc_incompatible_opaque_vec,
            from_binding_skip_types: &non_round_trip_flat_enums,
            strip_cfg_fields_from_binding_struct: true,
            ..crate::codegen::conversions::ConversionConfig::default()
        };
        for typ in api.types.iter().filter(|typ| !typ.is_trait) {
            if input_types.contains(&typ.name)
                && crate::codegen::conversions::can_generate_conversion(typ, &binding_to_core)
            {
                builder.add_item(&crate::codegen::conversions::gen_from_binding_to_core_cfg(
                    typ,
                    &core_import,
                    &extendr_conversion_cfg,
                ));
            }
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
                if crate::codegen::conversions::can_generate_enum_conversion_from_core(e) {
                    builder.add_item(&bridges::gen_extendr_flat_data_enum_from_core(e, &core_import));
                    if bridges::can_flat_data_enum_round_trip(e) {
                        builder.add_item(&bridges::gen_extendr_flat_data_enum_to_core(e, &core_import));
                    }
                }
            } else if bridges::is_json_passthrough_data_enum(e) {
                continue;
            } else {
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

        let active_bridges: Vec<_> = config
            .trait_bridges
            .iter()
            .filter(|b| !b.exclude_languages.iter().any(|l| l == "r" || l == "extendr"))
            .cloned()
            .collect();

        // pass below to avoid duplicate `#[extendr]` definitions (Rust E0428).
        let bridge_fn_names = collect_trait_bridge_fn_names(config);

        let r_exclude_functions: ahash::AHashSet<String> = config
            .r
            .as_ref()
            .map(|c| c.exclude_functions.iter().cloned().collect())
            .unwrap_or_default();

        // Function wrapper definitions keep their source `#[cfg(...)]` gate. The generated crate
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
                let item = bridges::gen_extendr_bridge_field_function(api, func, &bm, &core_import);
                let item = prepend_cfg(func.cfg.as_deref(), item);
                builder.add_item(&item);
            } else {
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
                        || (!cfg.named_non_opaque_params_by_ref
                            && (matches!(&p.ty, crate::core::ir::TypeRef::Named(n)
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
                        || (cfg.named_non_opaque_params_by_ref
                            && matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
                            if matches!(inner.as_ref(), crate::core::ir::TypeRef::Named(n)
                                if !opaque_types.contains(n.as_str())
                                    && !enum_names.contains(n.as_str())
                                    && !extendr_incompatible_types.contains(n.as_str())
                                    && api.types.iter().any(|t| !t.is_opaque && !t.is_trait && t.name == *n))))
                        || (cfg.named_non_opaque_params_by_ref
                            && matches!(&p.ty, crate::core::ir::TypeRef::Optional(inner)
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

        let mut emitted_send_robj_helper = false;
        for bridge_cfg in &config.trait_bridges {
            if bridge_cfg.exclude_languages.iter().any(|l| l == "r" || l == "extendr") {
                continue;
            }
            if let Some(trait_type) = api.types.iter().find(|t| t.is_trait && t.name == bridge_cfg.trait_name) {
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
                // entries — it rejects any `#[cfg(...)]` attribute with "expected mod, fn or impl"
                // registered. Feature-gated wrappers (whose `#[extendr]` `meta__`/`wrap__` symbols
                .filter(|f| always_registered(f.cfg.as_deref()))
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
        r_package::generate_public_api(api, config)
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
