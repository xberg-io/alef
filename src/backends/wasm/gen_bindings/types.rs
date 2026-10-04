//! WASM struct and opaque type code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name, shared};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use heck::ToPascalCase;

use super::functions::{
    emit_rustdoc, format_param_unused, gen_wasm_unimplemented_body, wasm_call_args, wasm_mapped_param_type,
    wasm_mapped_return_type, wasm_newtype_param_bindings, wasm_wrap_return, wrap_jsvalue_mapped_return,
};
use super::methods::gen_method_with_type_paths;

#[path = "types_accessors.rs"]
mod types_accessors;
#[path = "types_helpers.rs"]
pub(in crate::backends::wasm::gen_bindings) mod types_helpers;
#[path = "types_unit_enum.rs"]
mod types_unit_enum;

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;

use types_accessors::{gen_clear_method, gen_getter, gen_setter};
pub(in crate::backends::wasm::gen_bindings) use types_helpers::filter_cfg_fields_for_features;
use types_helpers::{
    complex_newtype_field_uses_jsvalue, is_bare_tagged_data_enum, is_option_of_tagged_data_enum,
    is_vec_of_tagged_data_enum,
};

/// Generate an opaque wasm-bindgen struct with inner Arc or Arc<Mutex<>>.
pub(super) fn gen_opaque_struct(
    typ: &TypeDef,
    core_import: &str,
    prefix: &str,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    // ~keep See `methods::gen_method`: the override crate is the only one this binding depends
    // on, so the source-crate prefix the IR carries has to be rewritten before it is emitted.
    let core_path = crate::codegen::conversions::core_type_path_remapped(typ, core_import, source_crate_remaps);

    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));

    let mut out = String::with_capacity(256);
    out.push_str(&emit_rustdoc(&typ.doc));
    out.push_str(&crate::backends::wasm::template_env::render(
        "gen_opaque_struct",
        minijinja::context! {
            struct_name => js_name,
            unprefixed_name => typ.name,
            core_path => core_path,
            has_mut_methods => has_mut_methods,
        },
    ));
    out
}

/// Generate wasm-bindgen methods for an opaque struct.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    wasm_skipped_methods: &AHashSet<String>,
    trait_bridges: &[TraitBridgeConfig],
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);

    // under #[cfg(target_arch = "wasm32")], so guard its impl block identically
    let bridge_config = trait_bridges
        .iter()
        .find(|bridge| bridge.type_alias.as_deref() == Some(typ.name.as_str()));
    let is_bridge_type_alias = bridge_config.is_some();
    if is_bridge_type_alias {
        impl_builder.add_attr("cfg(target_arch = \"wasm32\")");
    }
    impl_builder.add_attr("wasm_bindgen");

    impl_builder.add_method(&gen_transfer_copy_method(typ, prefix));

    if is_bridge_type_alias && typ.methods.is_empty() {
        let bridge_config = bridge_config.expect("checked bridge alias");
        let module_name = crate::backends::wasm::trait_bridge::wasm_bridge_module_name(bridge_config);
        let bridge_struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Wasm", bridge_config);
        let constructor = crate::backends::wasm::template_env::render(
            "gen_visitor_handle_constructor",
            minijinja::context! {
                struct_name => js_name,
                module_name => module_name,
                bridge_struct_name => bridge_struct_name,
            },
        );
        impl_builder.add_method(&constructor);
    }

    for method in &typ.methods {
        if method.name == "default" {
            continue;
        }
        let method_key = format!("{}.{}", typ.name, method.name);
        if wasm_skipped_methods.contains(&method_key) {
            continue;
        }
        if method.is_static {
            impl_builder.add_method(&gen_opaque_static_method(
                method,
                OpaqueStaticMethodContext {
                    mapper,
                    type_name: &typ.name,
                    opaque_types,
                    core_import,
                    source_crate_remaps,
                    qualified_type_path: &crate::codegen::conversions::core_type_path_remapped(
                        typ,
                        core_import,
                        source_crate_remaps,
                    ),
                    type_paths,
                    prefix,
                    mutex_types,
                },
            ));
        } else {
            impl_builder.add_method(&gen_opaque_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
                core_import,
                source_crate_remaps,
                type_paths,
                prefix,
                adapter_bodies,
                mutex_types,
                streaming_item_types,
            ));
        }
    }

    impl_builder.build()
}

/// Generate a method for an opaque wasm-bindgen struct that delegates to self.inner.
#[allow(clippy::too_many_arguments)]
fn gen_opaque_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &AHashMap<String, String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
) -> String {
    let type_is_mutex_wrapped = mutex_types.contains(type_name);
    let is_ref_mut = matches!(method.receiver.as_ref(), Some(ReceiverKind::RefMut));

    let can_delegate_base = shared::can_auto_delegate(method, opaque_types);
    let can_delegate = if is_ref_mut && type_is_mutex_wrapped && method.trait_source.is_none() {
        !method.sanitized
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && shared::is_delegatable_param(&p.ty, opaque_types))
            && shared::is_opaque_delegatable_type(&method.return_type)
    } else {
        can_delegate_base
    };
    let adapter_key = format!("{type_name}.{}", method.name);
    let has_adapter = adapter_bodies.contains_key(&adapter_key);

    let params_unused = !can_delegate && !has_adapter && !method.is_async;
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = wasm_mapped_param_type(p, mapper);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, params_unused)
        })
        .collect();

    let adapter_key_for_stream = format!("{}.{}", type_name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        wasm_mapped_return_type(&method.return_type, &method.return_newtype_wrapper, mapper)
    };
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let async_kw = if method.is_async { "async " } else { "" };

    let needs_clone = matches!(method.receiver, Some(ReceiverKind::Owned));

    let body = if can_delegate {
        let let_bindings =
            wasm_newtype_param_bindings(&method.params, mapper, core_import, source_crate_remaps, type_paths);
        let call_args = wasm_call_args(&method.params, opaque_types, false);
        let core_call = if is_ref_mut {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else if needs_clone {
            if type_is_mutex_wrapped {
                format!("self.inner.lock().unwrap().clone().{}({})", method.name, call_args)
            } else {
                format!("(*self.inner).clone().{}({})", method.name, call_args)
            }
        } else if type_is_mutex_wrapped {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.is_async {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let result_wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            if method.error_type.is_some() {
                format!(
                    "{let_bindings}let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                     Ok({result_wrap})"
                )
            } else {
                format!("{let_bindings}let result = {core_call}.await;\n    {result_wrap}")
            }
        } else if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{let_bindings}{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
            } else {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        true,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                format!(
                    "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
                )
            }
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}{wrapped}")
        }
    } else if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let return_annotation = if has_adapter
        && adapter_bodies
            .get(&adapter_key)
            .is_some_and(|b| b.contains("js_sys::Array") || b.contains("serde_wasm_bindgen::to_value"))
    {
        "Result<JsValue, JsValue>".to_string()
    } else {
        return_annotation
    };

    let mut attrs = emit_rustdoc(&method.doc);
    if method.params.len() + 1 > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub {async_kw}fn {}(&self, {}) -> {} {{\n    \
         {body}\n}}",
        method.name,
        params.join(", "),
        return_annotation
    )
}

struct OpaqueStaticMethodContext<'a> {
    mapper: &'a WasmMapper,
    type_name: &'a str,
    opaque_types: &'a AHashSet<String>,
    core_import: &'a str,
    source_crate_remaps: &'a [(&'a str, &'a str)],
    qualified_type_path: &'a str,
    type_paths: &'a AHashMap<String, String>,
    prefix: &'a str,
    mutex_types: &'a AHashSet<String>,
}

/// Generate a static method for an opaque wasm-bindgen struct.
/// Static methods call CoreType::method() instead of self.inner.method().
fn gen_opaque_static_method(method: &MethodDef, context: OpaqueStaticMethodContext<'_>) -> String {
    let OpaqueStaticMethodContext {
        mapper,
        type_name,
        opaque_types,
        core_import,
        source_crate_remaps,
        qualified_type_path,
        type_paths,
        prefix,
        mutex_types,
    } = context;
    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = wasm_mapped_param_type(p, mapper);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate)
        })
        .collect();

    let return_type = wasm_mapped_return_type(&method.return_type, &method.return_newtype_wrapper, mapper);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let body = if can_delegate {
        let let_bindings =
            wasm_newtype_param_bindings(&method.params, mapper, core_import, source_crate_remaps, type_paths);
        let call_args = wasm_call_args(&method.params, opaque_types, false);
        let core_call = format!("{qualified_type_path}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            let await_suffix = if method.is_async { ".await" } else { "" };
            format!(
                "{let_bindings}let result = {core_call}{await_suffix}.map_err(|e| \
                 JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        } else if method.is_async {
            let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}let result = {core_call}.await;\n    {wrapped}")
        } else {
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            let wrapped = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                wasm_wrap_return(
                    &result,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                )
            });
            format!("{let_bindings}{wrapped}")
        }
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    let mut attrs = emit_rustdoc(&method.doc);
    if method.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    format!(
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub {async_kw}fn {method_name}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        async_kw = if method.is_async { "async " } else { "" },
        method_name = method.name,
        params = params.join(", "),
    )
}

/// Generate a wasm-bindgen struct definition with private fields.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
    source_crate_remaps: &[(&str, &str)],
    is_core_to_binding_convertible: bool,
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut out = String::with_capacity(512);
    out.push_str(&emit_rustdoc(&typ.doc));

    let mut fields = Vec::new();
    for field in shared::binding_fields(&typ.fields) {
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let is_option_tagged_enum =
            !is_vec_tagged_enum && is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let is_bare_tagged_enum = !is_vec_tagged_enum
            && !is_option_tagged_enum
            && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let field_type = if complex_newtype_field_uses_jsvalue(field) {
            if field.optional {
                "Option<JsValue>".to_string()
            } else {
                "JsValue".to_string()
            }
        } else if force_optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else if is_vec_tagged_enum {
            "JsValue".to_string()
        } else if is_option_tagged_enum || (is_bare_tagged_enum && field.optional) {
            "Option<JsValue>".to_string()
        } else if is_bare_tagged_enum {
            "JsValue".to_string()
        } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
            mapper.map_type(&field.ty)
        } else if field.optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        fields.push((field.name.clone(), field_type));
    }

    // When the type IS convertible, suppress #[derive(Default)] and emit the delegating
    let derives_default = !typ.has_default || !is_core_to_binding_convertible;
    out.push_str(&crate::backends::wasm::template_env::render(
        "gen_struct",
        minijinja::context! {
            struct_name => js_name,
            unprefixed_name => typ.name,
            derives_default => derives_default,
            fields => fields.iter().map(|(name, ty)| {
                minijinja::context! {
                    name => name,
                    field_type => ty,
                }
            }).collect::<Vec<_>>(),
        },
    ));
    if typ.has_default && is_core_to_binding_convertible {
        out.push_str(&generators::gen_delegating_default_impl(
            typ,
            core_import,
            prefix,
            source_crate_remaps,
        ));
    }
    out
}

/// Generate wasm-bindgen methods for a struct.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    opaque_types: &AHashSet<String>,
    api_enums: &[EnumDef],
    prefix: &str,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    untagged_ts_value_types: &AHashMap<String, String>,
    source_crate_remaps: &[(&str, &str)],
    api_types: &[TypeDef],
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();
    let mut type_paths = AHashMap::new();
    for api_type in api_types.iter().filter(|api_type| !api_type.is_trait) {
        type_paths.insert(
            api_type.name.clone(),
            crate::codegen::conversions::core_type_path_remapped(api_type, core_import, source_crate_remaps),
        );
    }
    for enum_def in api_enums {
        type_paths.insert(
            enum_def.name.clone(),
            crate::codegen::conversions::core_enum_path_remapped(enum_def, core_import, source_crate_remaps),
        );
    }
    // Mirrors the emission loop in `mod.rs`: every non-trait, non-excluded type becomes a
    // `#[wasm_bindgen]` class, whether it is an opaque wrapper or a field-carrying struct. ~keep
    let class_type_names: AHashSet<String> = api_types
        .iter()
        .filter(|t| !t.is_trait && !exclude_types.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();
    let tagged_data_enum_names: AHashSet<String> = api_enums
        .iter()
        .filter(|e| super::enums::is_tagged_data_enum(e, api_types))
        .map(|e| e.name.clone())
        .collect();
    // Every identifier this impl block already mints from the consumer's own API: an inherent
    // method name, and a field name (whose getter is `pub fn {field}`). `gen_clear_method`
    // stands down rather than collide with one as `E0592`. ~keep
    let reserved_idents: AHashSet<String> = typ
        .methods
        .iter()
        .map(|m| m.name.clone())
        .chain(typ.fields.iter().map(|f| f.name.clone()))
        .collect();

    if !typ.fields.is_empty() {
        impl_builder.add_method(&gen_new_method(
            typ,
            mapper,
            exclude_types,
            prefix,
            &tagged_data_enum_names,
            &class_type_names,
        ));
        // The wasm wrapper always has a Default impl — either #[derive(Default)] or the
        if !typ.methods.iter().any(|m| m.name == "default") {
            impl_builder.add_method(&gen_default_method(typ, prefix));
        }
    }

    impl_builder.add_method(&gen_transfer_copy_method(typ, prefix));

    let mut emitted_field_names: AHashSet<&str> = AHashSet::default();
    for field in shared::binding_fields(&typ.fields) {
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        emitted_field_names.insert(field.name.as_str());
        impl_builder.add_method(&gen_getter(
            field,
            mapper,
            &enum_names,
            &tagged_data_enum_names,
            typ.has_default,
            untagged_ts_value_types,
            &class_type_names,
        ));
        impl_builder.add_method(&gen_setter(
            field,
            mapper,
            &enum_names,
            typ.has_default,
            &tagged_data_enum_names,
            untagged_ts_value_types,
            &class_type_names,
        ));
        if let Some(clear) = gen_clear_method(field, mapper, &class_type_names, &reserved_idents) {
            impl_builder.add_method(&clear);
        }
    }

    if !exclude_types.contains(&typ.name) {
        for method in &typ.methods {
            // A field and an inherent method sharing the same name both mint a
            // `pub fn {name}` in this impl block — the getter above is emitted first and
            // wins, so the method wrapper is skipped to avoid `E0592: duplicate definitions`. ~keep
            if emitted_field_names.contains(method.name.as_str()) {
                continue;
            }
            let refs_excluded = method
                .params
                .iter()
                .any(|p| field_references_excluded_type(&p.ty, exclude_types))
                || field_references_excluded_type(&method.return_type, exclude_types);
            if refs_excluded {
                continue;
            }
            impl_builder.add_method(&gen_method_with_type_paths(
                method,
                mapper,
                &typ.name,
                core_import,
                opaque_types,
                prefix,
                typ,
                mutex_types,
                streaming_item_types,
                source_crate_remaps,
                &type_paths,
            ));
        }
    }

    impl_builder.build()
}

/// Convert snake_case parameter names to camelCase for JS-facing constructor signatures.
/// Also converts the assignments list to use explicit `field: param` syntax.
///
/// Assignment forms:
/// 1. Shorthand (required field): `"tool_call_id"` → `"tool_call_id: toolCallId"`
/// 2. Explicit passthrough: `"total_tokens: total_tokens"` → `"total_tokens: totalTokens"`
/// 3. Explicit with suffix: `"total_tokens: total_tokens.unwrap_or_default()"` →
///    `"total_tokens: totalTokens.unwrap_or_default()"` (leading ident renamed, suffix kept)
/// 4. Constant expressions (e.g. `"field: Default::default()"`): kept as-is.
fn convert_constructor_params_to_camel_case(
    param_list: &str,
    assignments: &str,
    field_names: &[String],
    borrowed: &AHashSet<String>,
) -> (String, String) {
    let field_to_camel: std::collections::HashMap<String, String> = field_names
        .iter()
        .map(|name| (name.clone(), to_node_name(name)))
        .collect();

    let is_multiline = param_list.contains('\n');
    let raw_camel_params: Vec<String> = param_list
        .split(',')
        .filter_map(|param| {
            let trimmed = param.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some((name, ty)) = trimmed.split_once(':') {
                let camel_name = to_node_name(name.trim());
                let borrow = if borrowed.contains(name.trim()) { "&" } else { "" };
                Some(format!("{}: {}{}", camel_name, borrow, ty.trim()))
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect();
    let camel_params = if is_multiline {
        format!("\n        {},\n    ", raw_camel_params.join(",\n        "))
    } else {
        raw_camel_params.join(", ")
    };

    let camel_assignments = assignments
        .split(", ")
        .map(|assignment| {
            if assignment.contains(':') {
                if let Some((field_name, rhs)) = assignment.split_once(':') {
                    let field_trimmed = field_name.trim();
                    let rhs_trimmed = rhs.trim();
                    let (leading_ident, suffix) = split_leading_ident(rhs_trimmed);
                    // A borrowed parameter is `&Wasm{Type}`, so the struct literal has to store
                    // a clone -- mirroring what `gen_class_field_setter.jinja` does. ~keep
                    let clone = if borrowed.contains(field_trimmed) && suffix.is_empty() {
                        ".clone()"
                    } else {
                        ""
                    };
                    if let Some(camel_rhs) = field_to_camel.get(leading_ident) {
                        format!("{}: {}{}{}", field_trimmed, camel_rhs, suffix, clone)
                    } else {
                        format!("{}: {}{}", field_trimmed, rhs_trimmed, clone)
                    }
                } else {
                    assignment.to_string()
                }
            } else {
                let field_name = assignment.trim();
                if borrowed.contains(field_name) {
                    let camel_name = field_to_camel.get(field_name).map_or(field_name, String::as_str);
                    return format!("{field_name}: {camel_name}.clone()");
                }
                match field_to_camel.get(field_name) {
                    // Only emit `field: camelVar` when the rename actually changes the
                    // identifier; single-word fields (snake == camel) stay shorthand. ~keep
                    Some(camel_name) if camel_name != field_name => format!("{}: {}", field_name, camel_name),
                    _ => assignment.to_string(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    (camel_params, camel_assignments)
}

/// Split a Rust expression into `(leading_identifier, rest_of_expression)`.
fn split_leading_ident(expr: &str) -> (&str, &str) {
    let end = expr
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(expr.len());
    (&expr[..end], &expr[end..])
}

fn preserve_optional_constructor_defaults(assignments: &str, fields: &[FieldDef]) -> (String, bool) {
    let mut rendered = assignments.to_string();
    let mut use_defaults = false;
    for field in fields
        .iter()
        .filter(|field| field.cfg.is_none() && (field.optional || matches!(field.ty, TypeRef::Optional(_))))
    {
        let direct = format!("{}: {}", field.name, field.name);
        let preserved = format!("{}: {}.or(defaults.{})", field.name, field.name, field.name);
        if rendered.contains(&direct) {
            rendered = rendered.replace(&direct, &preserved);
            use_defaults = true;
        } else {
            let mut replaced = false;
            rendered = rendered
                .split(", ")
                .map(|assignment| {
                    if assignment == field.name {
                        replaced = true;
                        preserved.as_str()
                    } else {
                        assignment
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            use_defaults |= replaced;
        }
    }
    (rendered, use_defaults)
}

/// Generate a constructor method with camelCase parameter names for JS consumers.
fn gen_new_method(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
    class_type_names: &AHashSet<String>,
) -> String {
    use super::field_references_excluded_type;
    use crate::codegen::shared::constructor_parts;

    let complex_newtype_types: Vec<&TypeRef> = typ
        .fields
        .iter()
        .filter(|field| complex_newtype_field_uses_jsvalue(field))
        .map(|field| &field.ty)
        .collect();
    let map_fn = |ty: &crate::core::ir::TypeRef| {
        if complex_newtype_types.contains(&ty)
            || is_vec_of_tagged_data_enum(ty, tagged_data_enum_names)
            || is_bare_tagged_data_enum(ty, tagged_data_enum_names)
        {
            "JsValue".to_string()
        } else if is_option_of_tagged_data_enum(ty, tagged_data_enum_names) {
            "Option<JsValue>".to_string()
        } else {
            mapper.map_type(ty)
        }
    };

    let filtered_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !f.binding_excluded)
        .filter(|f| !field_references_excluded_type(&f.ty, exclude_types))
        .cloned()
        .collect();

    let field_names: Vec<String> = filtered_fields.iter().map(|f| f.name.clone()).collect();

    let has_field_defaults = filtered_fields.iter().any(|field| field.has_bare_serde_enum_default());
    let (param_list, _, assignments) = if typ.has_default {
        crate::codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true, typ)
    } else if has_field_defaults {
        crate::codegen::shared::constructor_parts_with_field_defaults(&filtered_fields, &map_fn, typ)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };

    let (assignments, use_defaults) = if typ.has_default {
        preserve_optional_constructor_defaults(&assignments, &filtered_fields)
    } else {
        (assignments, false)
    };
    let borrowed = borrowable_constructor_fields(typ, &filtered_fields, mapper, class_type_names);
    let (param_list_camel, assignments_camel) =
        convert_constructor_params_to_camel_case(&param_list, &assignments, &field_names, &borrowed);

    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let mut attrs = Vec::new();
    if field_count > 7 {
        attrs.push("#[allow(clippy::too_many_arguments)]");
    }
    attrs.push("#[allow(non_snake_case)]");

    crate::backends::wasm::template_env::render(
        "gen_struct_constructor",
        minijinja::context! {
            doc_lines => consumed_argument_doc(&filtered_fields, mapper, class_type_names, &borrowed),
            attrs => attrs,
            param_list => param_list_camel,
            assignments => assignments_camel,
            struct_name => format!("{prefix}{}", typ.name),
            use_defaults => use_defaults,
        },
    )
    .trim_end()
    .to_string()
}

/// The fields whose constructor parameter can be taken as `&Wasm{Type}` instead of by value.
///
/// wasm-bindgen lowers a by-value exported-struct argument through `__destroy_into_raw()`, so
/// `new Options(palette, ...)` leaves the caller's `palette` handle dead (alef#472). A borrow is
/// passed as a plain `__wbg_ptr` read and leaves it alive.
///
/// Only a *required* parameter of a struct that does not derive `Default` qualifies, and that is
/// a wasm-bindgen limit rather than a conservative choice: `Option<&T>` has no
/// `OptionFromWasmAbi` impl, so an optional parameter cannot be borrowed at all -- and
/// `shared::config_constructor_parts_inner`, the arm taken when `typ.has_default`, types *every*
/// parameter as `Option<T>`, required ones included. A `#[derive(Default)]` options struct
/// therefore has no borrowable constructor parameter whatsoever. That remaining half of alef#472
/// is tracked separately; `consumed_argument_doc` is what tells the consumer about it. ~keep
fn borrowable_constructor_fields(
    typ: &TypeDef,
    fields: &[FieldDef],
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
) -> AHashSet<String> {
    if typ.has_default {
        return AHashSet::default();
    }
    fields
        .iter()
        .filter(|f| {
            !f.optional && !f.has_bare_serde_enum_default() && f.cfg.is_none() && !matches!(f.ty, TypeRef::Optional(_))
        })
        .filter(|f| types_helpers::class_backed_field_type(f, mapper, class_type_names).is_some())
        .map(|f| f.name.clone())
        .collect()
}

/// Rustdoc stating the ownership contract for each class-typed argument this constructor takes
/// by value, or nothing when it takes none.
///
/// These are exactly the class-typed parameters `borrowable_constructor_fields` could not take
/// by reference, and taking ownership of them is wasm-bindgen's ordinary lowering for a by-value
/// exported-struct argument, not a defect: passing a freshly built value is correct and is what
/// a constructor argument is normally for. It becomes a trap only for a handle the caller still
/// holds, where the failure is a `null pointer passed to rust` at the *next* use, far from this
/// call -- so the doc names the safe alternative rather than only the hazard.
///
/// This text is the resolution of alef#479: there is no non-breaking way to remove the
/// ownership transfer (`Option<&T>` has no `OptionFromWasmAbi` impl, and dropping the parameter
/// changes a published constructor signature), and `default()` plus the borrowed setters #470
/// added is already a complete construction path. Keep it accurate rather than deleting it. ~keep
fn consumed_argument_doc(
    fields: &[FieldDef],
    mapper: &WasmMapper,
    class_type_names: &AHashSet<String>,
    borrowed: &AHashSet<String>,
) -> Vec<String> {
    let consumed: Vec<String> = fields
        .iter()
        .filter(|f| !borrowed.contains(&f.name))
        .filter(|f| types_helpers::class_backed_field_type(f, mapper, class_type_names).is_some())
        .map(|f| format!("`{}`", to_node_name(&f.name)))
        .collect();
    let consumed_vectors: Vec<String> = fields
        .iter()
        .filter(|f| types_helpers::class_backed_vec_element_type(f, mapper, class_type_names).is_some())
        .map(|f| format!("`{}`", to_node_name(&f.name)))
        .collect();
    let mut lines = Vec::new();
    if !consumed.is_empty() {
        lines.extend([
            format!("Takes ownership of {}.", consumed.join(", ")),
            "wasm-bindgen lowers a by-value class argument through `__destroy_into_raw()`, so each".to_string(),
            "of those handles is dead once this returns. Passing a freshly built value is fine. To".to_string(),
            "keep a handle you already hold, build with `default()` and assign the property instead:".to_string(),
            "the generated setter borrows and leaves your handle alive.".to_string(),
        ]);
    }
    if !consumed_vectors.is_empty() {
        lines.extend([
            format!("Takes ownership of every element in {}.", consumed_vectors.join(", ")),
            "wasm-bindgen cannot borrow class values nested in an array. To keep those handles,".to_string(),
            "copy each retained element with `copyForTransfer()` before passing the array.".to_string(),
        ]);
    }
    lines
}

/// Generate a `default()` static factory method.
///
/// Provides an arg-free way to obtain a fresh instance for types whose constructor
/// requires positional arguments. Every wasm struct derives `Default`.
fn gen_default_method(typ: &TypeDef, prefix: &str) -> String {
    format!(
        "#[wasm_bindgen]\n#[allow(clippy::should_implement_trait)]\npub fn default() -> {prefix}{} {{\n    <{prefix}{} as ::core::default::Default>::default()\n}}",
        typ.name, typ.name
    )
}

/// Emit an independent JS handle that is safe to give to an ABI position which takes ownership.
/// wasm-bindgen cannot borrow class values nested in `Vec<T>`, so callers retain the original and
/// transfer this clone instead. The generated name is reserved for Alef's ownership adapter. ~keep
fn gen_transfer_copy_method(typ: &TypeDef, prefix: &str) -> String {
    format!(
        "/// Return an independent handle for an ownership-taking API.\n\
         ///\n\
         /// Use `copyForTransfer()` for each class value placed in an array when the original\n\
         /// handle must remain usable.\n\
         #[wasm_bindgen(js_name = \"copyForTransfer\")]\n\
         pub fn copy_for_transfer(&self) -> {prefix}{} {{\n    self.clone()\n}}",
        typ.name
    )
}
