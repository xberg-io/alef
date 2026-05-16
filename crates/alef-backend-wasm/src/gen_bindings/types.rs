//! WASM struct and opaque type code generation.

use crate::type_map::WasmMapper;
use ahash::AHashSet;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::type_mapper::TypeMapper;
use alef_codegen::{generators, naming::to_node_name, shared};
use alef_core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

use super::functions::{emit_rustdoc, format_param_unused, gen_wasm_unimplemented_body, wasm_wrap_return};
use super::methods::gen_method;

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

/// Returns `true` when `ty` is `Vec<Named>` where `Named` is a unit enum (in `enum_names`
/// but NOT in `tagged_data_enum_names`).
///
/// JS callers pass these fields as arrays of serde-wire strings (e.g. `["image", "document"]`);
/// the setter must convert via `from_api_str` and the getter must emit `to_api_str` so the
/// JS surface is symmetric. wasm-bindgen does not transparently bridge `Vec<UnitEnum>` with
/// `Vec<String>` — we have to emit explicit conversions on both sides.
fn is_vec_of_unit_enum(
    ty: &TypeRef,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
) -> bool {
    matches!(
        ty,
        TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n)
                if enum_names.contains(n) && !tagged_data_enum_names.contains(n))
    )
}

/// Resolve the prefixed binding name for a unit-enum element inside a `Vec<UnitEnum>` field.
/// Returns `Some(WasmFoo)` for `Vec<Foo>` when `Foo` is a unit enum.
fn vec_unit_enum_inner_name(
    ty: &TypeRef,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
    prefix: &str,
) -> Option<String> {
    if let TypeRef::Vec(inner) = ty {
        if let TypeRef::Named(n) = inner.as_ref() {
            if enum_names.contains(n) && !tagged_data_enum_names.contains(n) {
                return Some(format!("{prefix}{n}"));
            }
        }
    }
    None
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
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);

    // Check if any method takes &mut self, requiring Arc<Mutex<T>>
    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(ReceiverKind::RefMut)));

    let mut out = String::with_capacity(256);
    out.push_str(&emit_rustdoc(&typ.doc));
    out.push_str(&crate::template_env::render(
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
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &WasmMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    prefix: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
    mutex_types: &AHashSet<String>,
) -> String {
    let js_name = format!("{prefix}{}", typ.name);
    let mut impl_builder = ImplBuilder::new(&js_name);

    // The VisitorHandle bridge module (__alef_wasm_bridge_*) is only emitted
    // under #[cfg(target_arch = "wasm32")], so guard its impl block identically
    // to avoid "unresolved module" errors when compiling on host targets.
    if typ.name == "VisitorHandle" {
        impl_builder.add_attr("cfg(target_arch = \"wasm32\")");
    }
    impl_builder.add_attr("wasm_bindgen");

    // Special handling for VisitorHandle: add a constructor if no methods exist.
    if typ.name == "VisitorHandle" && typ.methods.is_empty() {
        let constructor = crate::template_env::render(
            "gen_visitor_handle_constructor",
            minijinja::context! {
                struct_name => js_name,
            },
        );
        impl_builder.add_method(&constructor);
    }

    for method in &typ.methods {
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
    mutex_types: &AHashSet<String>,
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

    let return_type = mapper.map_type(&method.return_type);
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
    for field in &typ.fields {
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

    out.push_str(&crate::template_env::render(
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

    for field in &typ.fields {
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
            ));
        }
    }

    impl_builder.build()
}

/// Convert snake_case parameter names to camelCase for JS-facing constructor signatures.
/// Also converts the assignments list to use explicit field: param syntax.
/// Input: ("foo_bar: String, baz_qux: Option<u32>", "foo_bar: String, baz_qux")
/// Output: (camel_params, camel_assignments) where assignments use explicit syntax mapping renamed params to original field names.
fn convert_constructor_params_to_camel_case(param_list: &str, assignments: &str, field_names: &[String]) -> (String, String) {
    // Build a map from snake_case field names to their camelCase equivalents.
    let field_to_camel: std::collections::HashMap<String, String> = field_names
        .iter()
        .map(|name| (name.clone(), to_node_name(name)))
        .collect();

    // Rename parameter declarations: "foo_bar: String" → "fooBar: String"
    let camel_params = param_list
        .split(", ")
        .map(|param| {
            if let Some((name, ty)) = param.split_once(':') {
                let name_trimmed = name.trim();
                let ty_trimmed = ty.trim();
                let camel_name = to_node_name(name_trimmed);
                format!("{}: {}", camel_name, ty_trimmed)
            } else {
                param.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    // Rewrite assignments to use explicit field: param syntax.
    // E.g. "foo_bar, baz_qux" becomes "foo_bar: foo_bar, baz_qux: baz_qux"
    // (where the RHS is now the camelCase parameter name).
    let camel_assignments = assignments
        .split(", ")
        .map(|assignment| {
            // Check if this is already an explicit assignment (e.g. "field: Default::default()")
            if assignment.contains(':') {
                // Already explicit: keep it, but if RHS is a field name, apply camelCase rename
                if let Some((field_name, rhs)) = assignment.split_once(':') {
                    let field_trimmed = field_name.trim();
                    let rhs_trimmed = rhs.trim();
                    // If the RHS matches a field name, rename it to camelCase
                    if let Some(camel_rhs) = field_to_camel.get(rhs_trimmed) {
                        format!("{}: {}", field_trimmed, camel_rhs)
                    } else {
                        assignment.to_string()
                    }
                } else {
                    assignment.to_string()
                }
            } else {
                // Shorthand: "foo_bar" → "foo_bar: foo_bar" (where RHS is camelCase param)
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

/// Generate a constructor method.
fn gen_new_method(
    typ: &TypeDef,
    mapper: &WasmMapper,
    exclude_types: &[String],
    prefix: &str,
    tagged_data_enum_names: &AHashSet<String>,
) -> String {
    use super::field_references_excluded_type;
    use alef_codegen::shared::constructor_parts;

    // Tagged-data enum fields (Vec<T>, Option<T>, bare T) are stored as JsValue / Option<JsValue>
    // in the struct; the constructor must accept the same types so callers can pass plain JS
    // object literals directly.
    // Note: for optional bare tagged enums (field.optional=true, ty=Named), the constructor
    // parameter type is determined by `config_constructor_parts_with_options` / `constructor_parts`
    // which wraps optional fields via `mapper.optional()`. Since `map_fn` maps the ty (Named) to
    // JsValue, the optional wrapper produces Option<JsValue> automatically.
    let map_fn = |ty: &alef_core::ir::TypeRef| {
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

    // Filter out fields whose types reference excluded types (the Js* wrapper won't exist).
    // Cfg-gated fields are retained: the struct body keeps them (with #[serde(skip)]) and the
    // shared `constructor_parts*` helpers emit a `Default::default()` initializer so the
    // struct literal stays complete; the host caller cannot supply trait-bridge wrappers.
    let filtered_fields: Vec<_> = typ
        .fields
        .iter()
        .filter(|f| !field_references_excluded_type(&f.ty, exclude_types))
        .cloned()
        .collect();

    // Collect field names for camelCase conversion.
    let field_names: Vec<String> = filtered_fields.iter().map(|f| f.name.clone()).collect();

    // For types with has_default, generate optional kwargs-style constructor.
    // Pass option_duration_on_defaults=true so Duration fields are Option<u64> params,
    // matching the Option<u64> field type emitted by gen_struct for has_default types.
    let (param_list, _, assignments) = if typ.has_default {
        alef_codegen::shared::config_constructor_parts_with_options(&filtered_fields, &map_fn, true)
    } else {
        constructor_parts(&filtered_fields, &map_fn)
    };

    // Convert parameter and assignment names to camelCase for JS consumers.
    let (param_list_camel, assignments_camel) = convert_constructor_params_to_camel_case(&param_list, &assignments, &field_names);

    // Suppress too_many_arguments when the constructor has >7 params
    let field_count = filtered_fields.iter().filter(|f| f.cfg.is_none()).count();
    let allow_attr = if field_count > 7 {
        "#[allow(clippy::too_many_arguments)]\n"
    } else {
        ""
    };

    format!(
        "{allow_attr}#[wasm_bindgen(constructor)]\npub fn new({param_list_camel}) -> {prefix}{} {{\n    {prefix}{} {{ {assignments_camel} }}\n}}",
        typ.name, typ.name
    )
}

/// Generate a `default()` static factory method.
///
/// wasm-bindgen's `#[wasm_bindgen(constructor)]` follows the Rust constructor's
/// arity, so types with required (non-Optional) fields expose a JS constructor
/// with required positional args. Test codegen and other JS callers want an
/// arg-free way to obtain a fresh instance and then drive it via setters; the
/// inherent `default()` factory (delegating to the derived `Default` impl)
/// supplies that without disturbing the constructor signature. Every wasm
/// struct derives `Default` (see `gen_struct.jinja`), so the factory can be
/// emitted unconditionally for structs with fields.
fn gen_default_method(typ: &TypeDef, prefix: &str) -> String {
    // `#[allow(clippy::should_implement_trait)]` is required because `default()` conflicts with
    // `Default::default()`. Renaming would change the JS-visible API; the allow is correct.
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
    let is_vec_unit_enum = is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);
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
    let is_vec_unit_enum = is_vec_of_unit_enum(&field.ty, enum_names, tagged_data_enum_names);

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
