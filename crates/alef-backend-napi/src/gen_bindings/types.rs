//! NAPI-RS struct, opaque type, and static method code generation.

use crate::type_map::NapiMapper;
use ahash::AHashSet;
use alef_codegen::builder::{ImplBuilder, StructBuilder};
use alef_codegen::generators::{self, RustBindingConfig};
use alef_codegen::naming::to_node_name;
use alef_codegen::shared::{can_auto_delegate, function_params, partition_methods};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};

use super::functions::{napi_apply_primitive_casts_to_call_args, napi_gen_call_args, napi_wrap_return};

/// Map a struct-field `TypeRef` containing `TypeRef::Bytes` (Rust `Vec<u8>`) to the TS
/// type the generated `JsBytes` wrapper accepts at runtime.
fn ts_type_for_bytes_field(ty: &TypeRef) -> Option<String> {
    fn inner(ty: &TypeRef) -> Option<String> {
        match ty {
            TypeRef::Bytes => Some("Uint8Array | Buffer | Array<number>".to_string()),
            TypeRef::Optional(i) => inner(i).map(|s| format!("{s} | null | undefined")),
            TypeRef::Vec(i) => inner(i).map(|s| format!("Array<{s}>")),
            TypeRef::Map(_k, v) => inner(v).map(|s| format!("Record<string, {s}>")),
            _ => None,
        }
    }
    inner(ty)
}

pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &NapiMapper,
    prefix: &str,
    has_serde: bool,
    opaque_types: &ahash::AHashSet<String>,
    never_skip_cfg_field_names: &[String],
) -> String {
    // Pre-check if any field uses serde_with (HashMap<_, Vec<u8>>) so we can add struct-level attr.
    // The IR represents `Vec<u8>` as TypeRef::Bytes (not Vec(Bytes)); accept both wrappers for safety.
    let has_serde_with_field = has_serde
        && typ.fields.iter().any(|f| match &f.ty {
            TypeRef::Map(_k, v) => {
                matches!(v.as_ref(), TypeRef::Bytes)
                    || matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Bytes))
            }
            TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Map(_k, v)
                if matches!(v.as_ref(), TypeRef::Bytes)
                    || matches!(v.as_ref(), TypeRef::Vec(vi) if matches!(vi.as_ref(), TypeRef::Bytes))),
            _ => false,
        });

    let mut struct_builder = StructBuilder::new(&format!("{prefix}{}", typ.name));
    // Use napi(object, js_name = "Foo") so NAPI-RS exports the unprefixed name in the
    // generated .d.ts while the Rust struct retains its JsFoo identifier internally.
    struct_builder.add_attr(&format!("napi(object, js_name = \"{}\")", typ.name));
    if has_serde && has_serde_with_field {
        struct_builder.add_attr("serde_with::serde_as");
    }
    struct_builder.add_derive("Clone");
    // Binding types always derive Default, Serialize, and Deserialize.
    // Default: enables using unwrap_or_default() in constructors for types with has_default.
    // Serialize/Deserialize: required for FFI/type conversion across binding boundaries.
    struct_builder.add_derive("Default");
    // Only derive serde traits when the binding crate has serde as a dependency.
    // Generating these derives unconditionally causes compile errors in crates
    // that don't list serde in their Cargo.toml.
    if has_serde {
        struct_builder.add_derive("serde::Serialize");
        struct_builder.add_derive("serde::Deserialize");
    }

    // Suppress unused-variable warning when no field uses it.
    let _ = never_skip_cfg_field_names;
    for field in &typ.fields {
        // Opaque NAPI classes (e.g. JsVisitorHandle) cannot be embedded in `#[napi(object)]`
        // structs because they don't implement `FromNapiValue`. Use a raw JavaScript object
        // (`napi::bindgen_prelude::Object<'static>`) as the field type instead — the convert
        // function bridges the JS object to the Rust opaque type at call time.
        //
        // Returns (base_type, already_optional) where already_optional means the base_type
        // already includes the Option<> wrapper (either from TypeRef::Optional or opaque handling).
        //
        // IMPORTANT: For struct fields, `Bytes` maps to `JsBytes` rather than raw `Vec<u8>`.
        // `JsBytes` provides custom NAPI conversion for Buffer, Uint8Array, and Array<number>
        // while still deriving Clone/serde traits for object structs.
        let map_bytes_field_type = |ty: &TypeRef| -> String {
            fn replace_bytes(ty: &TypeRef, mapper: &NapiMapper) -> String {
                match ty {
                    TypeRef::Bytes => "JsBytes".to_string(),
                    TypeRef::Optional(inner) => format!("Option<{}>", replace_bytes(inner, mapper)),
                    TypeRef::Map(k, v) => {
                        format!("HashMap<{}, {}>", replace_bytes(k, mapper), replace_bytes(v, mapper))
                    }
                    TypeRef::Vec(inner) => format!("Vec<{}>", replace_bytes(inner, mapper)),
                    other => mapper.map_type(other),
                }
            }
            replace_bytes(ty, mapper)
        };
        let (base_type, already_optional): (String, bool) = match &field.ty {
            TypeRef::Named(name) if opaque_types.contains(name) => {
                ("napi::bindgen_prelude::Object<'static>".to_string(), false)
            }
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if opaque_types.contains(name) {
                        // Optional<OpaqueClass> → Option<Object<'static>>
                        ("Option<napi::bindgen_prelude::Object<'static>>".to_string(), true)
                    } else {
                        (map_bytes_field_type(&field.ty), true)
                    }
                } else {
                    (map_bytes_field_type(&field.ty), true)
                }
            }
            _ => (map_bytes_field_type(&field.ty), false),
        };
        // For types with Default, make all fields optional so JS callers
        // can pass partial objects (missing fields get defaults).
        let field_type = if (field.optional || typ.has_default) && !already_optional {
            format!("Option<{base_type}>")
        } else {
            base_type
        };
        // Honor `#[serde(rename = "...")]` on the core field so JS callers see the wire
        // name (e.g. core `tool_type` with rename `"type"` is exposed to JS as `type`).
        let js_name = field.serde_rename.clone().unwrap_or_else(|| to_node_name(&field.name));
        // Override the d.ts type to match the runtime contract. The override covers Option,
        // Map, and Vec wrappers that ultimately bottom out at bytes.
        let ts_type_override = ts_type_for_bytes_field(&field.ty);
        let napi_attr_inner: Vec<String> = {
            let mut v = vec![];
            if js_name != field.name {
                v.push(format!("js_name = \"{}\"", js_name));
            }
            if let Some(ts) = &ts_type_override {
                v.push(format!("ts_type = \"{}\"", ts));
            }
            v
        };
        let mut attrs = if !napi_attr_inner.is_empty() {
            vec![format!("napi({})", napi_attr_inner.join(", "))]
        } else {
            vec![]
        };

        // For HashMap<_, Vec<u8>>, keep serde_with's Bytes helper for map values.
        // Bare/optional byte fields use JsBytes and do not need serde_bytes attributes.
        fn contains_vec_u8(ty: &TypeRef) -> bool {
            match ty {
                TypeRef::Bytes => true,
                TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
                TypeRef::Optional(inner) => contains_vec_u8(inner),
                TypeRef::Map(_k, v) => contains_vec_u8(v),
                _ => false,
            }
        }
        let has_vec_u8 = contains_vec_u8(&field.ty);
        if has_serde && has_vec_u8 {
            match &field.ty {
                TypeRef::Map(_k, v)
                    if matches!(v.as_ref(), TypeRef::Bytes)
                        || matches!(v.as_ref(), TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Bytes)) =>
                {
                    // HashMap<K, Vec<u8>>: use serde_with's Bytes helper for map values.
                    attrs.push("serde_as(as = \"HashMap<_, serde_with::Bytes>\")".to_string());
                }
                _ => {}
            }
        }

        // Opaque NAPI types (e.g. JsVisitorHandle) are stored as Object<'static>, which also
        // does NOT impl Serialize/Deserialize. Skip them too so serde derives still compile.
        let is_opaque_field = match &field.ty {
            TypeRef::Named(name) if opaque_types.contains(name) => true,
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(name) if opaque_types.contains(name))
            }
            _ => false,
        };
        // Emit `#[serde(skip)]` for opaque fields and cfg-gated trait-bridge fields (their
        // wrapper types don't impl serde). Other cfg-gated fields remain serializable.
        let skip_cfg_bridge_field = field.cfg.is_some() && never_skip_cfg_field_names.contains(&field.name);
        if has_serde && (is_opaque_field || skip_cfg_bridge_field) {
            attrs.push("serde(skip)".to_string());
        }
        struct_builder.add_field(&field.name, &field_type, attrs);
    }

    

    struct_builder.build()
}

/// Generate NAPI methods for an opaque struct (delegates to self.inner).
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    capsule_type_names: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, alef_core::config::NodeCapsuleTypeConfig>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&format!("{prefix}{}", typ.name));
    impl_builder.add_attr("napi");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        // Skip sanitized methods that have no adapter override — they cannot be delegated
        // and emitting an unimplemented stub pollutes the public API with dead placeholders.
        let adapter_key = format!("{}.{}", typ.name, method.name);
        if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        // Skip methods whose return type is a capsule type — the capsule shim for
        // free functions emits a JsObject/External, but the method codegen path
        // here would emit `Result<Js<Capsule>>` referencing a suppressed wrapper
        // class that no longer exists. The free function alternative covers the
        // same API. Tracked as a known limitation in alef-backend-napi.
        let returns_capsule = match &method.return_type {
            TypeRef::Named(name) => capsule_type_names.contains(name),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(name) => capsule_type_names.contains(name),
                _ => false,
            },
            _ => false,
        };
        if returns_capsule {
            continue;
        }
        // Skip methods that accept opaque-typed params by value — NAPI class types don't implement
        // FromNapiValue and cannot appear as plain `#[napi]` method params. These methods (e.g.
        // ConversionOptionsBuilder::visitor) require custom adapter code or bridge patterns.
        let has_opaque_by_value_param = method.params.iter().any(|p| {
            let inner_ty = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            matches!(inner_ty, TypeRef::Named(name) if opaque_types.contains(name) && !p.is_ref)
        });
        if has_opaque_by_value_param && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        impl_builder.add_method(&gen_opaque_instance_method(
            method,
            mapper,
            typ,
            cfg,
            opaque_types,
            prefix,
            adapter_bodies,
            streaming_item_types,
            mutex_types,
            capsule_types,
        ));
    }
    for method in &statics {
        // Skip sanitized static methods that have no adapter override.
        let adapter_key = format!("{}.{}", typ.name, method.name);
        if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
        impl_builder.add_method(&gen_static_method(
            method,
            mapper,
            typ,
            cfg,
            opaque_types,
            prefix,
            mutex_types,
        ));
    }

    impl_builder.build()
}

/// Generate an opaque instance method that delegates to self.inner.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_instance_method(
    method: &MethodDef,
    mapper: &NapiMapper,
    typ: &TypeDef,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, alef_core::config::NodeCapsuleTypeConfig>,
) -> String {
    let params = function_params(&method.params, &|ty| {
        // For capsule types in method params, use fully-qualified names
        if let alef_core::ir::TypeRef::Named(name) = ty {
            if let Some(capsule_cfg) = capsule_types.get(name) {
                return format!(
                    "{}::{}",
                    capsule_cfg.from_module.replace('-', "_"),
                    capsule_cfg.type_name
                );
            }
        }
        mapper.map_type(ty)
    });
    let adapter_key_for_stream = format!("{}.{}", typ.name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if let Some(item) = stream_item {
        format!("Vec<{prefix}{item}>")
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

    let type_name = &typ.name;
    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::RefMut));

    // Check if the type has any RefMut methods (which means inner is wrapped in Mutex).
    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(alef_core::ir::ReceiverKind::RefMut)));

    let call_args = napi_gen_call_args(&method.params, opaque_types);

    // Use the shared can_auto_delegate check for opaque instance methods.
    // RefMut methods can be delegated if the type is Mutex-wrapped (has_mut_methods).
    // Arc<T> doesn't support &mut T directly, but Arc<Mutex<T>> does via lock().
    let opaque_can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || has_mut_methods)
        && (!is_owned_receiver || typ.is_clone)
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && alef_codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type);

    let make_async_core_call = |method_name: &str| -> String {
        if has_mut_methods && !is_ref_mut_receiver {
            format!("inner.lock().unwrap().{method_name}({call_args})")
        } else {
            format!("inner.{method_name}({call_args})")
        }
    };

    let async_result_wrap = napi_wrap_return(
        "result",
        &method.return_type,
        type_name,
        opaque_types,
        true,
        method.returns_ref,
        prefix,
        mutex_types,
    );

    let adapter_key = format!("{type_name}.{}", method.name);
    let body = if let Some(adapter_body) = adapter_bodies.get(&adapter_key) {
        adapter_body.clone()
    } else if !opaque_can_delegate {
        // Try serde-based param conversion for methods with non-opaque Named params
        if cfg.has_serde
            && !method.sanitized
            && generators::has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && alef_codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let serde_bindings =
                generators::gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            let serde_call_args = generators::gen_call_args_with_let_bindings(&method.params, opaque_types);
            let core_call = if has_mut_methods {
                format!("self.inner.lock().unwrap().{}({serde_call_args})", method.name)
            } else {
                format!("self.inner.{}({serde_call_args})", method.name)
            };
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{err_conv}?;\n    Ok(())")
            } else {
                let wrap = napi_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                );
                format!("{serde_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})")
            }
        } else {
            generators::gen_unimplemented_body(
                &method.return_type,
                &format!("{type_name}.{}", method.name),
                method.error_type.is_some(),
                cfg,
                &method.params,
                opaque_types,
            )
        }
    } else if method.is_async {
        let inner_clone_line = "let inner = self.inner.clone();\n    ";
        let core_call_str = make_async_core_call(&method.name);
        generators::gen_async_body(
            &core_call_str,
            cfg,
            method.error_type.is_some(),
            &async_result_wrap,
            true,
            inner_clone_line,
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        // When any non-opaque Named param has is_ref=true, generate let-bindings before the call
        // to avoid E0716 ("temporary value dropped while borrowed"). The inline `.into()` pattern
        // creates a temporary that Rust can't borrow for the duration of the call expression.
        let use_let_bindings = generators::has_named_params(&method.params, opaque_types);
        let (let_bindings, call_args_for_call) = if use_let_bindings {
            let bindings = generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import);
            let args = napi_apply_primitive_casts_to_call_args(
                &generators::gen_call_args_with_let_bindings(&method.params, opaque_types),
                &method.params,
            );
            (bindings, args)
        } else {
            (String::new(), napi_gen_call_args(&method.params, opaque_types))
        };
        let core_call = if is_owned_receiver {
            format!("(*self.inner).clone().{}({})", method.name, call_args_for_call)
        } else if has_mut_methods {
            format!("self.inner.lock().unwrap().{}({})", method.name, call_args_for_call)
        } else {
            format!("self.inner.{}({})", method.name, call_args_for_call)
        };
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{let_bindings}{core_call}{err_conv}?;\n    Ok(())")
            } else {
                let wrap = napi_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                );
                format!("{let_bindings}let result = {core_call}{err_conv}?;\n    Ok({wrap})")
            }
        } else {
            format!(
                "{let_bindings}{}",
                napi_wrap_return(
                    &core_call,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                )
            )
        }
    };

    let mut attrs = String::new();
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
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}(&self, {params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}

/// Generate a static method binding.
pub(super) fn gen_static_method(
    method: &MethodDef,
    mapper: &NapiMapper,
    typ: &TypeDef,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let type_name = &typ.name;
    let core_type_path = typ.rust_path.replace('-', "_");
    let call_args = napi_gen_call_args(&method.params, opaque_types);
    let can_delegate_static = can_auto_delegate(method, opaque_types);

    let async_kw = if method.is_async { "async " } else { "" };

    let body = if !can_delegate_static {
        generators::gen_unimplemented_body(
            &method.return_type,
            &format!("{type_name}::{}", method.name),
            method.error_type.is_some(),
            cfg,
            &method.params,
            opaque_types,
        )
    } else if method.is_async {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        let return_wrap = napi_wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            typ.is_opaque,
            method.returns_ref,
            prefix,
            mutex_types,
        );
        generators::gen_async_body(
            &core_call,
            cfg,
            method.error_type.is_some(),
            &return_wrap,
            false,
            "",
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let wrapped = napi_wrap_return(
                "val",
                &method.return_type,
                type_name,
                opaque_types,
                typ.is_opaque,
                method.returns_ref,
                prefix,
                mutex_types,
            );
            if wrapped == "val" {
                format!("{core_call}{err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            napi_wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                typ.is_opaque,
                method.returns_ref,
                prefix,
                mutex_types,
            )
        }
    };

    let mut attrs = String::new();
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
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}

/// Generate a NAPI enum definition using string_enum with Js prefix.
/// Generate a NAPI enum definition.
/// For simple enums (no variant fields): generates `#[napi(string_enum)]`.
/// For tagged enums with data fields: generates a flattened `#[napi(object)]` struct
/// with a discriminant field and all variant fields as optional.
#[cfg(test)]
mod tests {
    /// gen_struct (pub(super)) is accessible from mod.rs — smoke test via trait.
    /// The actual output is tested via the integration test (gen_bindings_test.rs).
    #[test]
    fn struct_gen_function_exists() {
        // Compilation check: if this module compiles, gen_struct is correctly defined.
    }
}
