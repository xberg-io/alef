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

#[path = "types_unit_enum.rs"]
mod types_unit_enum;

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;

use types_unit_enum::{is_vec_of_unit_enum, vec_unit_enum_inner_name};

/// Returns `true` when `ty` is `Vec<Named>` where `Named` is a tagged-data enum.
///
/// These fields are stored as `JsValue` in the wasm binding struct so that plain JS object
/// literals (e.g. `{ role: "user", content: "..." }`) can be passed without constructing
/// explicit wasm-bindgen class instances.
fn is_vec_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Returns `true` when `ty` is a bare `Named` that is a tagged-data enum (not wrapped in Option or Vec).
///
/// Bare tagged-data enum fields are stored as `JsValue` in the wasm binding struct so that plain
/// JS object literals can be assigned without constructing an explicit wasm-bindgen class instance.
fn is_bare_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Named(n) if tagged_data_enum_names.contains(n))
}

/// Returns `true` when `ty` is `Option<Named>` where `Named` is a tagged-data enum.
///
/// Optional tagged-data enum fields are stored as `Option<JsValue>` in the wasm binding struct.
fn is_option_of_tagged_data_enum(ty: &TypeRef, tagged_data_enum_names: &AHashSet<String>) -> bool {
    matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if tagged_data_enum_names.contains(n)))
}

/// Check if a TypeRef is a Copy type that shouldn't be cloned.
/// `enum_names` contains the set of enum type names that derive Copy.
pub(super) fn is_copy_type(ty: &TypeRef, enum_names: &AHashSet<String>) -> bool {
    match ty {
        TypeRef::Primitive(_) => true, // All primitives are Copy
        TypeRef::Duration => true,     // Duration maps to u64 (secs), which is Copy
        TypeRef::String | TypeRef::Char | TypeRef::Bytes | TypeRef::Path | TypeRef::Json => false,
        TypeRef::Optional(inner) => is_copy_type(inner, enum_names), // Option<Copy> is Copy
        TypeRef::Vec(_) | TypeRef::Map(_, _) => false,
        TypeRef::Named(n) => enum_names.contains(n), // WASM enums derive Copy
        TypeRef::Unit => true,
    }
}

/// Generate an opaque wasm-bindgen struct with inner Arc or Arc<Mutex<>>.
pub(super) fn gen_opaque_struct(typ: &TypeDef, core_import: &str, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let core_path = crate::codegen::conversions::core_type_path(typ, core_import);

    // Check if any method takes &mut self, requiring Arc<Mutex<T>>
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

    // Bridge handle modules (__alef_wasm_bridge_*) are only emitted
    // under #[cfg(target_arch = "wasm32")], so guard its impl block identically
    // to avoid "unresolved module" errors when compiling on host targets.
    let bridge_config = trait_bridges
        .iter()
        .find(|bridge| bridge.type_alias.as_deref() == Some(typ.name.as_str()));
    let is_bridge_type_alias = bridge_config.is_some();
    if is_bridge_type_alias {
        impl_builder.add_attr("cfg(target_arch = \"wasm32\")");
    }
    impl_builder.add_attr("wasm_bindgen");

    // Special handling for bridge handles: add a constructor if no methods exist.
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
        // Skip the `default()` method — we don't emit synthetic factories for opaque types
        if method.name == "default" {
            continue;
        }
        // Skip methods whose corresponding streaming adapter has "wasm" in
        // skip_languages. The parameter types for those methods are only
        // generated by the streaming adapter body path; omitting the whole
        // method is simpler than emitting a stub that references missing types.
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
    // Whether the parent opaque type's inner is `Arc<Mutex<T>>` (it has at least one `&mut self`
    // method). RefMut methods on Mutex-wrapped types ARE delegatable (lock yields `&mut T`),
    // contra `shared::can_auto_delegate`'s blanket exclusion. `&self` methods on Mutex-wrapped
    // types must also lock; otherwise the call dispatches against `Arc<Mutex<T>>`.
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

    // Params are "unused" only when we can't delegate AND there's no adapter body
    // that references them. Async methods also use params in their generated bodies.
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
        // For streaming methods, return the iterator struct (not the item type).
        // The iterator struct name is {PascalCaseMethodName}Iterator.
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

    // Check if the core method takes ownership (Owned receiver) or mutable reference.
    // For Owned: clone out of Arc (or Arc<Mutex<>>) since wasm_bindgen methods take &self.
    // For RefMut: lock the Mutex and get &mut, then call the method.
    let needs_clone = matches!(method.receiver, Some(ReceiverKind::Owned));

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let core_call = if is_ref_mut {
            // RefMut: inner is Arc<Mutex<T>>, lock and call &mut method
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else if needs_clone {
            if type_is_mutex_wrapped {
                format!("self.inner.lock().unwrap().clone().{}({})", method.name, call_args)
            } else {
                format!("(*self.inner).clone().{}({})", method.name, call_args)
            }
        } else if type_is_mutex_wrapped {
            // `&self` method on a Mutex-wrapped opaque type: dispatch through .lock().unwrap()
            // since `self.inner: Arc<Mutex<T>>` does not expose the inner type's methods.
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args)
        } else {
            format!("self.inner.{}({})", method.name, call_args)
        };
        if method.is_async {
            // WASM async: native async fn becomes a Promise automatically
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

    // Streaming adapters return JsValue (via js_sys::Array or serde_wasm_bindgen), override the IR return type
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
    // Per-item clippy suppression: too_many_arguments when >7 params (including &self)
    if method.params.len() + 1 > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning methods
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    // Per-item clippy suppression: should_implement_trait for trait-conflicting names
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
    // Per-item clippy suppression: too_many_arguments when >7 params
    if method.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning methods
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    // Per-item clippy suppression: should_implement_trait for trait-conflicting names
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
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut out = String::with_capacity(512);
    out.push_str(&emit_rustdoc(&typ.doc));
    // Binding types derive Clone and Default.
    // Default: enables using unwrap_or_default() in constructors.
    // Note: Do NOT derive Serialize/Deserialize on WASM types. wasm-bindgen handles conversion
    // across the JS boundary, and many WASM struct fields (like JsValue) don't implement Serialize.

    // Build filtered and typed fields
    let mut fields = Vec::new();
    for field in shared::binding_fields(&typ.fields) {
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        // On has_default types, non-optional Duration fields are stored as Option<u64> so the
        // wasm constructor can omit them and the From conversion falls back to the core default.
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        // Tagged-data enum fields (Vec<T>, Option<T>, or bare T) must be stored as JsValue /
        // Option<JsValue> so that plain JS object literals can be assigned directly without
        // constructing explicit wasm-bindgen class instances (wasm-bindgen would reject plain
        // objects with "expected instance of WasmFoo").
        //
        // IR shape variants for a tagged-data enum T:
        //   - required bare:    field.optional=false, ty=Named(T)  → JsValue
        //   - optional bare:    field.optional=true,  ty=Named(T)  → Option<JsValue>
        //   - explicit optional: field.optional=true, ty=Optional(Named(T)) → Option<JsValue>
        //   - required Vec:     field.optional=false, ty=Vec(Named(T)) → JsValue
        let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        // Optional via the ty=Optional(Named) IR form.
        let is_option_tagged_enum =
            !is_vec_tagged_enum && is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
        // Bare Named that is a tagged-data enum — may be optional or required via field.optional.
        let is_bare_tagged_enum = !is_vec_tagged_enum
            && !is_option_tagged_enum
            && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
        let field_type = if force_optional {
            // Duration field forced to Option<u64>: map_type returns "u64", wrap in Option<>.
            mapper.optional(&mapper.map_type(&field.ty))
        } else if is_vec_tagged_enum {
            // Vec<TaggedDataEnum>: use JsValue for the field so JS callers can pass plain objects.
            "JsValue".to_string()
        } else if is_option_tagged_enum || (is_bare_tagged_enum && field.optional) {
            // Option<TaggedDataEnum> (either via ty=Optional(Named) or field.optional=true with ty=Named):
            // use Option<JsValue> so JS callers can pass plain objects or null.
            "Option<JsValue>".to_string()
        } else if is_bare_tagged_enum {
            // Required bare TaggedDataEnum: use JsValue so JS callers can pass plain objects.
            "JsValue".to_string()
        } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
            // Field is already Optional in the IR: map_type returns "Option<X>". Using
            // mapper.optional() would yield Option<Option<X>>, which wasm-bindgen can't handle
            // (OptionIntoWasmAbi is not implemented for Option<Option<T>>). Use the mapped
            // type directly.
            mapper.map_type(&field.ty)
        } else if field.optional {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        fields.push((field.name.clone(), field_type));
    }

    out.push_str(&crate::backends::wasm::template_env::render(
        "gen_struct",
        minijinja::context! {
            struct_name => js_name,
            unprefixed_name => typ.name,
            fields => fields.iter().map(|(name, ty)| {
                minijinja::context! {
                    name => name,
                    field_type => ty,
                }
            }).collect::<Vec<_>>(),
        },
    ));
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

    // Collect enum names for Copy detection in getters.
    // Use unprefixed names since TypeRef::Named stores the original name without Js prefix.
    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();
    // Tagged data enums are emitted as wasm-bindgen structs, not C-enums — they do NOT have
    // a `to_api_str()` method and they are not Copy. Track them separately so the getter
    // path treats `Option<WasmAuthConfig>` like any other Optional<Struct> (clone-and-return).
    // Also used for constructor generation: Vec<TaggedDataEnum> parameters become JsValue.
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
        // Skip synthetic Default factory when the IR already exposes an
        // explicit static method named `default` (it will be emitted below
        // through the methods loop and would otherwise conflict).
        if !typ.methods.iter().any(|m| m.name == "default") {
            impl_builder.add_method(&gen_default_method(typ, prefix));
        }
    }

    for field in shared::binding_fields(&typ.fields) {
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
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
            // Skip methods whose params or return type reference excluded types
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

/// Extract the inner type of an `Optional` wrapper, or return the type itself.
/// `FieldDef::optional` + `FieldDef::ty` can be:
/// - `optional=true, ty=Named(X)`      → inner is Named(X)  (Optional<> added by mapping)
/// - `optional=true, ty=Optional(X)`   → inner is X         (Optional already in IR)
fn optional_inner(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

/// Generate a getter method for a field.
fn gen_getter(
    field: &FieldDef,
    mapper: &WasmMapper,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
    has_default: bool,
) -> String {
    // On has_default types, non-optional Duration fields are stored as Option<u64>.
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    let field_type = if force_optional {
        mapper.optional(&mapper.map_type(&field.ty))
    } else if field.optional && matches!(field.ty, TypeRef::Optional(_)) {
        // Already Optional in IR: map_type returns "Option<X>". Don't double-wrap.
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

    // Fix A: enum fields must return String (or Option<String>) so JS receives the serde wire
    // string (e.g. "stop", "tool_calls") instead of a numeric discriminant.
    // Fix B: optional Vec-of-struct fields must return Option<js_sys::Array> so JS can
    // access prototype methods on each element (e.g. [0].function.name).
    let inner_ty = optional_inner(&field.ty);
    // Tagged data enums are emitted as wasm-bindgen structs (no `to_api_str()`); treat them
    // like any other Named struct field — clone and return the binding wrapper directly.
    let is_optional_enum = field.optional
        && matches!(inner_ty, TypeRef::Named(n) if enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    let is_required_enum = !field.optional
        && matches!(field.ty, TypeRef::Named(ref n) if enum_names.contains(n) && !tagged_data_enum_names.contains(n));
    // Vec<TaggedDataEnum> is stored as JsValue — return it directly by clone.
    let is_required_vec_tagged_enum = !field.optional && is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    // Bare TaggedDataEnum (required, field.optional=false) is also stored as JsValue.
    let is_required_bare_tagged_enum = !field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    // Option<TaggedDataEnum> — either via ty=Optional(Named) or field.optional=true with ty=Named.
    // Both cases are stored as Option<JsValue>.
    let is_optional_tagged_enum = field.optional
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names));
    // Vec<UnitEnum>: return Vec<String> via per-element to_api_str() so JS sees serde-wire strings
    // (matching the setter that accepts Vec<String>). wasm-bindgen does not bridge Vec<UnitEnum>
    // ↔ JS arrays of strings on its own.
    //
    // Two IR shapes both reach here:
    //   - required `Vec<UnitEnum>`:  field.optional=false, field.ty=Vec(Named(E))
    //     → stored as `Vec<WasmE>`,        getter returns `Vec<String>`.
    //   - optional `Option<Vec<UnitEnum>>`: field.optional=true,  field.ty=Vec(Named(E))
    //     → stored as `Option<Vec<WasmE>>`, getter returns `Option<Vec<String>>` and must
    //       flatten the Option before iterating (`.as_ref().map(...)`).
    let is_vec_unit_enum = !field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
    let is_optional_vec_unit_enum =
        field.optional && is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
    let is_optional_vec_of_struct = field.optional
        && matches!(
            inner_ty,
            TypeRef::Vec(elem) if matches!(elem.as_ref(), TypeRef::Named(n) if !enum_names.contains(n))
        )
        // Do not apply the js_sys::Array path for Vec<TaggedDataEnum> — those are stored as
        // JsValue and must be returned as JsValue (not reconstructed element-by-element).
        && !is_vec_of_tagged_data_enum(inner_ty, tagged_data_enum_names);

    let (field_type, return_expr) = if is_vec_unit_enum {
        // Vec<UnitEnum> stored as Vec<WasmFoo>: convert each via to_api_str() to a Vec<String>.
        let expr = format!(
            "self.{}.iter().map(|v| v.to_api_str().to_owned()).collect()",
            field.name
        );
        ("Vec<String>".to_string(), expr)
    } else if is_optional_vec_unit_enum {
        // Option<Vec<UnitEnum>> stored as Option<Vec<WasmFoo>>: flatten the Option before
        // iterating, then convert each element via to_api_str(). Iterating the Option directly
        // would yield a `&Vec<WasmFoo>` element that lacks `to_api_str()` (E0599).
        let expr = format!(
            "self.{}.as_ref().map(|v| v.iter().map(|x| x.to_api_str().to_owned()).collect())",
            field.name
        );
        ("Option<Vec<String>>".to_string(), expr)
    } else if is_required_vec_tagged_enum || is_required_bare_tagged_enum {
        // Vec<TaggedDataEnum> or bare TaggedDataEnum stored as JsValue: return the JsValue directly.
        ("JsValue".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_tagged_enum {
        // Option<TaggedDataEnum> stored as Option<JsValue>: return it directly by clone.
        ("Option<JsValue>".to_string(), format!("self.{}.clone()", field.name))
    } else if is_optional_enum {
        // Return Option<String> using the generated to_api_str() method.
        let expr = format!("self.{}.map(|v| v.to_api_str().to_owned())", field.name);
        ("Option<String>".to_string(), expr)
    } else if is_required_enum {
        // Return String directly using the generated to_api_str() method.
        let expr = format!("self.{}.to_api_str().to_owned()", field.name);
        ("String".to_string(), expr)
    } else if is_optional_vec_of_struct {
        // Return Option<js_sys::Array> so JS can call prototype methods on each element.
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
        // Default: only clone non-Copy types; Copy types are returned directly.
        // Tagged data enums are emitted as Clone (non-Copy) structs, so exclude them from the
        // Copy set we pass to `is_copy_type`.
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
    // On has_default types, non-optional Duration fields are stored as Option<u64>.
    let force_optional = has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
    // Tagged-data enum fields (Vec<T>, Option<T>, bare T) are stored as JsValue / Option<JsValue>
    // so plain JS object literals can be assigned without explicit wasm-bindgen class instances.
    // IR shape: field.optional=true with ty=Named is an optional bare enum → Option<JsValue>.
    let is_vec_tagged_enum = is_vec_of_tagged_data_enum(&field.ty, tagged_data_enum_names);
    let is_option_tagged_enum = !is_vec_tagged_enum
        && (is_option_of_tagged_data_enum(&field.ty, tagged_data_enum_names)
            || (field.optional && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names)));
    let is_bare_tagged_enum =
        !is_vec_tagged_enum && !is_option_tagged_enum && is_bare_tagged_data_enum(&field.ty, tagged_data_enum_names);
    // Vec<UnitEnum>: accept Vec<String> and convert each via from_api_str(). Silently drop
    // unknown variants — matches the getter that returns Vec<String> via to_api_str().
    //
    // Required vs optional has the same IR shape split as the getter:
    //   - required `Vec<UnitEnum>`:        field.optional=false → storage Vec<WasmE>,        setter takes Vec<String>.
    //   - optional `Option<Vec<UnitEnum>>`: field.optional=true  → storage Option<Vec<WasmE>>, setter takes Option<Vec<String>>
    //     and must wrap the collected Vec in `Some(...)` to match the field type.
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
        // Already Optional in IR: map_type returns "Option<X>". Don't double-wrap.
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
