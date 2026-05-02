//! WASM struct and opaque type code generation.

use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::type_mapper::TypeMapper;
use alef_codegen::{generators, naming::to_node_name, shared};
use alef_core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::fmt::Write;

use super::functions::{emit_rustdoc, format_param_unused, gen_wasm_unimplemented_body, wasm_wrap_return};
use super::methods::gen_method;

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

/// Generate an opaque wasm-bindgen struct with inner Arc.
pub(super) fn gen_opaque_struct(typ: &TypeDef, core_import: &str, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", typ.name);

    // We can't use StructBuilder for private fields, so build manually
    let mut out = String::with_capacity(256);
    out.push_str(&emit_rustdoc(&typ.doc));
    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, "#[wasm_bindgen]").ok();
    writeln!(out, "pub struct {} {{", js_name).ok();
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    writeln!(out, "    inner: Arc<{}>,", core_path).ok();
    write!(out, "}}").ok();
    out
}

/// Generate wasm-bindgen methods for an opaque struct.
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    prefix: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    for method in &typ.methods {
        if method.is_static {
            impl_builder.add_method(&gen_opaque_static_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
                core_import,
                prefix,
            ));
        } else {
            impl_builder.add_method(&gen_opaque_method(
                method,
                mapper,
                &typ.name,
                opaque_types,
                prefix,
                adapter_bodies,
            ));
        }
    }

    impl_builder.build()
}

/// Generate a method for an opaque wasm-bindgen struct that delegates to self.inner.
fn gen_opaque_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
) -> String {
    let can_delegate = shared::can_auto_delegate(method, opaque_types);
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

    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let async_kw = if method.is_async { "async " } else { "" };

    // Check if the core method takes ownership (Owned receiver).
    // If so, we must clone out of Arc since wasm_bindgen methods take &self.
    let needs_clone = matches!(method.receiver, Some(ReceiverKind::Owned));

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let core_call = if needs_clone {
            format!("(*self.inner).clone().{}({})", method.name, call_args)
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
            )
        }
    } else if let Some(body) = adapter_bodies.get(&adapter_key) {
        body.clone()
    } else {
        gen_wasm_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };

    // Streaming adapters return JsValue (via serde_wasm_bindgen), override the IR return type
    let return_annotation = if has_adapter
        && adapter_bodies
            .get(&adapter_key)
            .is_some_and(|b| b.contains("serde_wasm_bindgen::to_value"))
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
pub(super) fn gen_struct(typ: &TypeDef, mapper: &WasmMapper, exclude_types: &[String], prefix: &str) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut out = String::with_capacity(512);
    out.push_str(&emit_rustdoc(&typ.doc));
    // Binding types derive Clone and Default.
    // Default: enables using unwrap_or_default() in constructors.
    // Note: Do NOT derive Serialize/Deserialize on WASM types. wasm-bindgen handles conversion
    // across the JS boundary, and many WASM struct fields (like JsValue) don't implement Serialize.
    writeln!(out, "#[derive(Clone, Default)]").ok();
    writeln!(out, "#[wasm_bindgen]").ok();
    writeln!(out, "pub struct {} {{", js_name).ok();

    for field in &typ.fields {
        // Skip cfg-gated fields — they depend on features that may not be enabled
        if field.cfg.is_some() {
            continue;
        }
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        // On has_default types, non-optional Duration fields are stored as Option<u64> so the
        // wasm constructor can omit them and the From conversion falls back to the core default.
        let force_optional = typ.has_default && !field.optional && matches!(field.ty, TypeRef::Duration);
        let field_type = if force_optional {
            // Duration field forced to Option<u64>: map_type returns "u64", wrap in Option<>.
            mapper.optional(&mapper.map_type(&field.ty))
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
        // Fields are private (no pub)
        writeln!(out, "    {}: {},", field.name, field_type).ok();
    }

    writeln!(out, "}}").ok();
    out
}

/// Generate wasm-bindgen methods for a struct.
pub(super) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    core_import: &str,
    opaque_types: &AHashSet<String>,
    api_enums: &[EnumDef],
    prefix: &str,
) -> String {
    use super::field_references_excluded_type;

    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);
    impl_builder.add_attr("wasm_bindgen");

    if !typ.fields.is_empty() {
        impl_builder.add_method(&gen_new_method(typ, mapper, exclude_types, prefix));
    }

    // Collect enum names for Copy detection in getters.
    // Use unprefixed names since TypeRef::Named stores the original name without Js prefix.
    let enum_names: AHashSet<String> = api_enums.iter().map(|e| e.name.clone()).collect();

    for field in &typ.fields {
        // Skip fields whose type references an excluded type (the Js* wrapper won't exist)
        if field_references_excluded_type(&field.ty, exclude_types) {
            continue;
        }
        impl_builder.add_method(&gen_getter(field, mapper, &enum_names, typ.has_default));
        impl_builder.add_method(&gen_setter(field, mapper, typ.has_default));
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
            ));
        }
    }

    impl_builder.build()
}

/// Generate a constructor method.
fn gen_new_method(typ: &TypeDef, mapper: &WasmMapper, exclude_types: &[String], prefix: &str) -> String {
    use super::field_references_excluded_type;
    use alef_codegen::shared::constructor_parts;

    let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);

    // Filter out fields whose types reference excluded types
    let filtered_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !field_references_excluded_type(&f.ty, exclude_types))
        .cloned()
        .collect();

    // For types with has_default, generate optional kwargs-style constructor.
    // Pass option_duration_on_defaults=true so Duration fields are Option<u64> params,
    // matching the Option<u64> field type emitted by gen_struct for has_default types.
    let (param_list, _, assignments) = if typ.has_default {
        alef_codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };

    // Suppress too_many_arguments when the constructor has >7 params
    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let allow_attr = if field_count > 7 {
        "#[allow(clippy::too_many_arguments)]\n"
    } else {
        ""
    };

    format!(
        "{allow_attr}#[wasm_bindgen(constructor)]\npub fn new({param_list}) -> {prefix}{} {{\n    {prefix}{} {{ {assignments} }}\n}}",
        typ.name, typ.name
    )
}

/// Generate a getter method for a field.
fn gen_getter(field: &FieldDef, mapper: &WasmMapper, enum_names: &AHashSet<String>, has_default: bool) -> String {
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

    // Only clone non-Copy types; Copy types are returned directly.
    let return_expr = if is_copy_type(&field.ty, enum_names) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };

    format!(
        "#[wasm_bindgen(getter{js_name_attr})]\npub fn {}(&self) -> {} {{\n    {}\n}}",
        field.name, field_type, return_expr
    )
}

/// Generate a setter method for a field.
fn gen_setter(field: &FieldDef, mapper: &WasmMapper, has_default: bool) -> String {
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

    format!(
        "#[wasm_bindgen(setter{js_name_attr})]\npub fn set_{}(&mut self, value: {}) {{\n    self.{} = value;\n}}",
        field.name, field_type, field.name
    )
}
