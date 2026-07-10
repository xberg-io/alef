//! WASM struct and opaque type code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::builder::ImplBuilder;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name, shared};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToPascalCase;

use super::functions::{emit_rustdoc, format_param_unused, gen_wasm_unimplemented_body, wasm_wrap_return};
use super::methods::gen_method;

#[path = "types_helpers.rs"]
mod types_helpers;
#[path = "types_unit_enum.rs"]
mod types_unit_enum;

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;

pub(in crate::backends::wasm::gen_bindings) use types_helpers::filter_cfg_fields_for_features;
use types_helpers::{
    is_bare_tagged_data_enum, is_copy_type, is_option_of_tagged_data_enum, is_vec_of_tagged_data_enum, optional_inner,
};
use types_unit_enum::{is_vec_of_unit_enum, vec_unit_enum_inner_name};

/// Generate an opaque wasm-bindgen struct with inner Arc or Arc<Mutex<>>.
pub(super) fn gen_opaque_struct(typ: &TypeDef, core_import: &str, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);

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
                mapper,
                &typ.name,
                opaque_types,
                core_import,
                prefix,
                mutex_types,
            ));
        } else {
            impl_builder.add_method(&gen_opaque_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
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
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, params_unused)
        })
        .collect();

    let adapter_key_for_stream = format!("{}.{}", type_name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        mapper.map_type(&method.return_type)
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
        let call_args = generators::gen_call_args(&method.params, opaque_types);
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
            let result_wrap = wasm_wrap_return(
                "result",
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
                mutex_types,
            );
            if method.error_type.is_some() {
                format!(
                    "let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                     Ok({result_wrap})"
                )
            } else {
                format!("let result = {core_call}.await;\n    Ok({result_wrap})")
            }
        } else if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
            } else {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                );
                format!("let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})")
            }
        } else {
            wasm_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
                mutex_types,
            )
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

/// Generate a static method for an opaque wasm-bindgen struct.
/// Static methods call CoreType::method() instead of self.inner.method().
fn gen_opaque_static_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate)
        })
        .collect();

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let core_call = format!("{core_import}::{type_name}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let wrap = wasm_wrap_return(
                "result",
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
                mutex_types,
            );
            format!("let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})")
        } else {
            wasm_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
                prefix,
                mutex_types,
            )
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
        "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
         {body}\n}}",
        method.name,
        params.join(", "),
        return_annotation
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
        let field_type = if force_optional {
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
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();
    let tagged_data_enum_names: AHashSet<String> = api_enums
        .iter()
        .filter(|e| super::enums::is_tagged_data_enum(e))
        .map(|e| e.name.clone())
        .collect();

    if !typ.fields.is_empty() {
        impl_builder.add_method(&gen_new_method(
            typ,
            mapper,
            exclude_types,
            prefix,
            &tagged_data_enum_names,
        ));
        // The wasm wrapper always has a Default impl — either #[derive(Default)] or the
        if !typ.methods.iter().any(|m| m.name == "default") {
            impl_builder.add_method(&gen_default_method(typ, prefix));
        }
    }

    for field in shared::binding_fields(&typ.fields) {
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        impl_builder.add_method(&gen_getter(
            field,
            mapper,
            &enum_names,
            &tagged_data_enum_names,
            typ.has_default,
        ));
        impl_builder.add_method(&gen_setter(
            field,
            mapper,
            &enum_names,
            typ.has_default,
            &tagged_data_enum_names,
        ));
    }

    if !exclude_types.contains(&typ.name) {
        for method in &typ.methods {
            let refs_excluded = method
                .params
                .iter()
                .any(|p| field_references_excluded_type(&p.ty, exclude_types))
                || field_references_excluded_type(&method.return_type, exclude_types);
            if refs_excluded {
                continue;
            }
            impl_builder.add_method(&gen_method(
                method,
                mapper,
                &typ.name,
                core_import,
                opaque_types,
                prefix,
                typ,
                mutex_types,
                streaming_item_types,
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
                Some(format!("{}: {}", camel_name, ty.trim()))
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
                    if let Some(camel_rhs) = field_to_camel.get(leading_ident) {
                        format!("{}: {}{}", field_trimmed, camel_rhs, suffix)
                    } else {
                        format!("{}: {}", field_trimmed, rhs_trimmed)
                    }
                } else {
                    assignment.to_string()
                }
            } else {
                let field_name = assignment.trim();
                if let Some(camel_name) = field_to_camel.get(field_name) {
                    format!("{}: {}", field_name, camel_name)
                } else {
                    assignment.to_string()
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

/// Generate a constructor method with camelCase parameter names for JS consumers.
fn gen_new_method(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
) -> String {
    use super::field_references_excluded_type;
    use crate::codegen::shared::constructor_parts;

    let map_fn = |ty: &crate::core::ir::TypeRef| {
        if is_vec_of_tagged_data_enum(ty, tagged_data_enum_names)
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

    let (param_list, _, assignments) = if typ.has_default {
        crate::codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };

    let (param_list_camel, assignments_camel) =
        convert_constructor_params_to_camel_case(&param_list, &assignments, &field_names);

    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let allow_attrs = if field_count > 7 {
        "#[allow(clippy::too_many_arguments)]\n#[allow(non_snake_case)]\n"
    } else {
        "#[allow(non_snake_case)]\n"
    };

    format!(
        "{allow_attrs}#[wasm_bindgen(constructor)]\npub fn new({param_list_camel}) -> {prefix}{} {{\n    {prefix}{} {{ {assignments_camel} }}\n}}",
        typ.name, typ.name
    )
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

/// Generate a getter method for a field.
fn gen_getter(
    field: &FieldDef,
    mapper: &WasmMapper,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
    has_default: bool,
) -> String {
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let field_type = if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        mapper.map_type(&field.ty)
    } else if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    let inner_ty = optional_inner(&field.ty);
    let is_optional_enum = field.optional
        && matches!(inner_ty, TypeRef::Named(n) if enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    let is_required_enum = !field.optional
        && matches!(field.ty, TypeRef::Named(ref n) if enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    let is_required_vec_tagged_enum = !field.optional && is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_required_bare_tagged_enum = !field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_optional_tagged_enum = field.optional
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names));
    let is_vec_unit_enum = !field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
    let is_optional_vec_unit_enum =
        field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
    let is_optional_vec_of_struct = field.optional
        && matches!(
            inner_ty,
            TypeRef::Vec(elem) if matches!(elem.as_ref(), TypeRef::Named(n) if !enum_names.contains(n))
        )
        && !is_vec_of_tagged_data_enum(inner_ty, tagged_data_enum_names);

    let (field_type, return_expr) = if is_vec_unit_enum {
        let expr = format!(
            "self.{}.iter().map(|v| v.to_api_str().to_owned()).collect()",
            field.name
        );
        ("Vec<String>".to_string(), expr)
    } else if is_optional_vec_unit_enum {
        let expr = format!(
            "self.{}.as_ref().map(|v| v.iter().map(|x| x.to_api_str().to_owned()).collect())",
            field.name
        );
        ("Option<Vec<String>>".to_string(), expr)
    } else if is_required_vec_tagged_enum || is_required_bare_tagged_enum {
        ("JsValue".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_tagged_enum {
        ("Option<JsValue>".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_enum {
        let expr = format!("self.{}.map(|v| v.to_api_str().to_owned())", field.name);
        ("Option<String>".to_string(), expr)
    } else if is_required_enum {
        let expr = format!("self.{}.to_api_str().to_owned()", field.name);
        ("String".to_string(), expr)
    } else if is_optional_vec_of_struct {
        let expr = format!(
            "self.{f}.as_ref().map(|items| {{\n        \
             let arr = js_sys::Array::new();\n        \
             for item in items {{\n            \
             arr.push(&JsValue::from(item.clone()));\n        \
             }}\n        \
             arr\n    }})",
            f = field.name
        );
        ("Option<js_sys::Array>".to_string(), expr)
    } else {
        let copy_enum_names: AHashSet<String> = enum_names
            .iter()
            .filter(|n| !tagged_data_enum_names.contains(*n))
            .cloned()
            .collect();
        let expr = if is_copy_type(&field.ty, &copy_enum_names) {
            format!("self.{}", field.name)
        } else {
            format!("self.{}.clone()", field.name)
        };
        (field_type, expr)
    };

    format!(
        "#[wasm_bindgen(getter{js_name_attr})]\npub fn {}(&self) -> {} {{\n    {}\n}}",
        field.name, field_type, return_expr
    )
}

/// Generate a setter method for a field.
fn gen_setter(
    field: &FieldDef,
    mapper: &WasmMapper,
    enum_names: &AHashSet<String>,
    has_default: bool,
    tagged_data_enum_names: &AHashSet<String>,
) -> String {
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_option_tagged_enum = !is_vec_tagged_enum
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || (field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names)));
    let is_bare_tagged_enum =
        !is_vec_tagged_enum && !is_option_tagged_enum && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_vec_unit_enum = !field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
    let is_optional_vec_unit_enum =
        field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);

    let js_name = to_node_name(&field.name);
    let js_name_attr = if js_name != field.name {
        format!(", js_name = \"{}\"", js_name)
    } else {
        String::new()
    };

    if is_vec_unit_enum {
        let inner = vec_unit_enum_inner_name(&field.ty, enum_names, tagged_data_enum_names, &mapper.prefix)
            .expect("is_vec_of_unit_enum implied inner is a named unit enum");
        return format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Vec<String>) {{\n    \
             self.{name} = value.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect();\n}}",
            name = field.name,
            inner = inner,
        );
    }

    if is_optional_vec_unit_enum {
        let inner = vec_unit_enum_inner_name(&field.ty, enum_names, tagged_data_enum_names, &mapper.prefix)
            .expect("is_vec_of_unit_enum implied inner is a named unit enum");
        return format!(
            "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{name}(&mut self, value: Option<Vec<String>>) {{\n    \
             self.{name} = value.map(|v| v.into_iter().filter_map(|s| {inner}::from_api_str(&s)).collect());\n}}",
            name = field.name,
            inner = inner,
        );
    }

    let field_type = if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else if is_vec_tagged_enum || is_bare_tagged_enum {
        "JsValue".to_string()
    } else if is_option_tagged_enum {
        "Option<JsValue>".to_string()
    } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        mapper.map_type(&field.ty)
    } else if field.optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else {
        mapper.map_type(&field.ty)
    };

    format!(
        "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{}(&mut self, value: {}) {{\n    self.{} = value;\n}}",
        field.name, field_type, field.name
    )
}
