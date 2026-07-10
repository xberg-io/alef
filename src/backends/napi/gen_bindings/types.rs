//! NAPI-RS struct, opaque type, and static method code generation.

use crate::backends::napi::type_map::NapiMapper;
use crate::codegen::builder::{ImplBuilder, StructBuilder};
use crate::codegen::generators::{self, RustBindingConfig};
use crate::codegen::naming::to_node_name;
use crate::codegen::shared::{binding_fields, can_auto_delegate, function_params, partition_methods};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToPascalCase, ToSnakeCase};

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
    let has_serde_with_field = has_serde
        && binding_fields(&typ.fields).any(|f| match &f.ty {
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
    struct_builder.add_attr(&format!("napi(object, js_name = \"{}\")", typ.name));
    if has_serde && has_serde_with_field {
        struct_builder.add_attr("serde_with::serde_as");
    }
    struct_builder.add_derive("Clone");
    struct_builder.add_derive("Default");
    if has_serde {
        struct_builder.add_derive("serde::Serialize");
        struct_builder.add_derive("serde::Deserialize");
    }

    let _ = never_skip_cfg_field_names;
    for field in binding_fields(&typ.fields) {
        // Opaque NAPI classes (e.g. JsVisitorHandle) cannot be embedded in `#[napi(object)]`
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
        let field_type = if (field.optional || typ.has_default) && !already_optional {
            format!("Option<{base_type}>")
        } else {
            base_type
        };
        // Honor `#[serde(rename = "...")]` on the core field so JS callers see the wire
        let js_name = field.serde_rename.clone().unwrap_or_else(|| to_node_name(&field.name));
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

        // When js_name differs from field.name, add #[serde(rename = "js_name")] so serde
        if has_serde && js_name != field.name {
            attrs.push(format!("serde(rename = \"{}\")", js_name));
        }

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
                    attrs.push("serde_as(as = \"HashMap<_, serde_with::Bytes>\")".to_string());
                }
                _ => {}
            }
        }

        let is_opaque_field = match &field.ty {
            TypeRef::Named(name) if opaque_types.contains(name) => true,
            TypeRef::Optional(inner) => {
                matches!(inner.as_ref(), TypeRef::Named(name) if opaque_types.contains(name))
            }
            _ => false,
        };
        // Emit `#[serde(skip)]` for opaque fields and cfg-gated trait-bridge fields (their
        let skip_cfg_bridge_field = field.cfg.is_some() && !never_skip_cfg_field_names.contains(&field.name);
        if has_serde && (is_opaque_field || skip_cfg_bridge_field) {
            attrs.push("serde(skip)".to_string());
        }
        let sanitized_field_doc = if field.doc.is_empty() {
            String::new()
        } else {
            crate::codegen::doc_emission::sanitize_rust_idioms(
                &field.doc,
                crate::codegen::doc_emission::DocTarget::TsDoc,
            )
        };
        struct_builder.add_field_with_doc(&field.name, &field_type, attrs, &sanitized_field_doc);
    }

    let body = struct_builder.build();
    let mut out = String::new();
    let sanitized_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&typ.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut out, &sanitized_doc, "");
    out.push_str(&body);
    out
}

/// Generate NAPI methods for an opaque struct (delegates to self.inner).
///
/// NOTE: NAPI-RS (TypeScript/Node.js) intentionally does NOT emit fluent builder classes
/// for DTO types (non-opaque structs with #[napi(object)]). TypeScript idiomatically uses
/// object literals for construction: `convert(html, { headingStyle: 'atx', wrap: true })`
/// rather than fluent builders like `new ConversionOptionsBuilder().headingStyle(...).build()`.
/// This contrasts with the Java backend, which nests builders inside records for idiomatic
/// Java construction patterns. DTO methods (withers returning Self) are exposed as free functions
/// accepting the instance as the first parameter (see gen_dto_method_fns).
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    adapter_bodies: &crate::adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    capsule_type_names: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&format!("{prefix}{}", typ.name));
    impl_builder.add_attr("napi");

    let (instance, statics) = partition_methods(&typ.methods);

    for method in &instance {
        let adapter_key = format!("{}.{}", typ.name, method.name);
        if method.sanitized && !adapter_bodies.contains_key(&adapter_key) {
            continue;
        }
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
        // FromNapiValue and cannot appear as plain `#[napi]` method params. These methods (e.g.
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
    adapter_bodies: &crate::adapters::AdapterBodies,
    streaming_item_types: &ahash::AHashMap<String, String>,
    mutex_types: &AHashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
) -> String {
    let params = function_params(&method.params, &|ty| {
        if let crate::core::ir::TypeRef::Named(name) = ty {
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

    let type_name = &typ.name;
    let is_owned_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::Owned));
    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut)));

    let self_ref_return =
        method.returns_ref && matches!(&method.return_type, crate::core::ir::TypeRef::Named(n) if n == type_name);

    let call_args = napi_gen_call_args(&method.params, opaque_types);

    let has_named_ref_param = method
        .params
        .iter()
        .any(|p| crate::codegen::shared::is_named_ref_param_pub(p, opaque_types));
    let opaque_can_delegate = !method.sanitized
        && (!is_ref_mut_receiver || has_mut_methods)
        && (!is_owned_receiver || typ.is_clone)
        && !has_named_ref_param
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type);

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
        if cfg.has_serde
            && !method.sanitized
            && generators::has_named_params(&method.params, opaque_types)
            && method.error_type.is_some()
            && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type)
        {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let serde_bindings =
                generators::gen_serde_let_bindings(&method.params, opaque_types, cfg.core_import, err_conv, "        ");
            let serde_call_args =
                generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types);
            let core_call = if has_mut_methods {
                format!("self.inner.lock().unwrap().{}({serde_call_args})", method.name)
            } else {
                format!("self.inner.{}({serde_call_args})", method.name)
            };
            let await_suffix = if method.is_async { ".await" } else { "" };
            if matches!(method.return_type, TypeRef::Unit) {
                format!("{serde_bindings}{core_call}{await_suffix}{err_conv}?;\n    Ok(())")
            } else if self_ref_return {
                format!(
                    "{serde_bindings}{core_call}{await_suffix}{err_conv}?;\n    Ok(Self {{ inner: self.inner.clone() }})"
                )
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
                format!("{serde_bindings}let result = {core_call}{await_suffix}{err_conv}?;\n    Ok({wrap})")
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
        let use_let_bindings = generators::has_named_params(&method.params, opaque_types);
        let (let_bindings, call_args_for_call) = if use_let_bindings {
            let bindings = generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import);
            let args = napi_apply_primitive_casts_to_call_args(
                &generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types),
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
            } else if self_ref_return {
                format!("{let_bindings}{core_call}{err_conv}?;\n    Ok(Self {{ inner: self.inner.clone() }})")
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
        } else if self_ref_return {
            format!("{let_bindings}{core_call};\n    Self {{ inner: self.inner.clone() }}")
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
    let sanitized_method_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&method.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
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
    let sanitized_method_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&method.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
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
        "{attrs}#[napi{js_name_attr}]\npub {async_kw}fn {}({params}) -> {return_annotation} {{\n    \
         {body}\n}}",
        method.name
    )
}

/// Generate standalone `#[napi]` free functions for DTO (non-opaque) impl methods.
///
/// `#[napi(object)]` structs cannot have `#[napi]` impl blocks — NAPI-RS enforces this.
/// The idiomatic workaround is to surface the methods as module-level free functions
/// whose `js_name` encodes the type name as a namespace prefix.  NAPI-RS emits them in
/// `index.d.ts` as top-level exports; callers group them via a TypeScript value-namespace
/// declaration (`export const ProcessConfig = { all, minimal, default: defaultConfig, … }`).
///
/// Naming convention:
/// - static (no receiver): `{type_name}_{method_name}` → camelCase JS `{typeName}{MethodName}`
/// - instance wither (&self, → Self): `{type_name}_{method_name}` same scheme but receives
///   the instance as an extra first param (JS: `processConfigWithChunking(cfg, n)`)
pub(super) fn gen_dto_method_fns(
    typ: &TypeDef,
    mapper: &NapiMapper,
    cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    prefix: &str,
    mutex_types: &ahash::AHashSet<String>,
    api: &crate::core::ir::ApiSurface,
) -> String {
    let methods: Vec<&crate::core::ir::MethodDef> = typ
        .methods
        .iter()
        .filter(|m| !m.binding_excluded && !m.sanitized)
        .collect();
    if methods.is_empty() {
        return String::new();
    }

    let core_type_path = typ.rust_path.replace('-', "_");
    let type_js_name = to_node_name(&typ.name);
    let mut out = String::new();

    for method in methods {
        let returns_self = matches!(&method.return_type, TypeRef::Named(n) if n == &typ.name);
        if !returns_self {
            continue;
        }

        let is_static = method.receiver.is_none();
        let method_js = to_node_name(&method.name);
        let js_name_upper = {
            let mut s = method_js.clone();
            if let Some(first) = s.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            s
        };
        let full_js_name = format!("{type_js_name}{js_name_upper}");
        let full_rust_name = format!("{}_{}", typ.name.to_snake_case(), method.name.to_snake_case());

        let binding_type = format!("{prefix}{}", typ.name);
        let return_annotation = mapper.map_type(&method.return_type).to_string();

        let mut param_parts: Vec<String> = vec![];
        if !is_static {
            param_parts.push(format!("cfg: {binding_type}"));
        }
        for p in &method.params {
            let ptype = mapper.map_type(&p.ty);
            let ptype = if p.optional { format!("Option<{ptype}>") } else { ptype };
            param_parts.push(format!("{}: {}", p.name, ptype));
        }
        let params_str = param_parts.join(", ");

        let use_let_bindings = generators::has_named_params(&method.params, opaque_types);
        let (call_args_str, dto_let_bindings) = if use_let_bindings {
            (
                napi_apply_primitive_casts_to_call_args(
                    &generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types),
                    &method.params,
                ),
                generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import),
            )
        } else {
            (napi_gen_call_args(&method.params, opaque_types), String::new())
        };

        let body = if is_static {
            let core_call = if method.name == "from" && method.params.len() == 1 {
                let param = &method.params[0];
                let param_expr = napi_gen_call_args(std::slice::from_ref(param), opaque_types);

                let core_param_type = match &param.ty {
                    crate::core::ir::TypeRef::Named(param_type_name) => api
                        .types
                        .iter()
                        .find(|t| t.name == *param_type_name)
                        .map(|t| t.rust_path.replace('-', "_"))
                        .unwrap_or_else(|| param_type_name.clone()),
                    other_ty => mapper.map_type(other_ty).to_string(),
                };

                format!("let _arg: {core_param_type} = {param_expr};\n    {core_type_path}::from(_arg)",)
            } else {
                format!("{core_type_path}::{}({call_args_str})", method.name)
            };
            if method.error_type.is_some() {
                let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
                let wrapped = napi_wrap_return(
                    "val",
                    &method.return_type,
                    &typ.name,
                    opaque_types,
                    false,
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
                    &typ.name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    prefix,
                    mutex_types,
                )
            }
        } else {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            if cfg.has_serde {
                crate::backends::napi::template_env::render(
                    "struct_wither_serde_body.jinja",
                    minijinja::context! {
                        err_conv => err_conv,
                        core_type_path => core_type_path,
                        method_name => method.name,
                        call_args => call_args_str,
                    },
                )
            } else {
                format!(
                    "    Err(napi::Error::new(napi::Status::GenericFailure, \
                     \"wither {} requires serde to round-trip through JSON\"))",
                    method.name
                )
            }
        };

        let body = if is_static && !dto_let_bindings.is_empty() {
            format!("{dto_let_bindings}{body}")
        } else {
            body
        };

        let mut attrs = String::new();
        let sanitized_method_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
            &method.doc,
            crate::codegen::doc_emission::DocTarget::TsDoc,
        );
        crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
        if method.error_type.is_some() || !is_static {
            attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
        }
        let returns_result = method.error_type.is_some() || !is_static;
        let final_return_ann = if returns_result && !return_annotation.starts_with("napi::Result") {
            format!("napi::Result<{return_annotation}>")
        } else {
            return_annotation.clone()
        };

        out.push_str(&crate::backends::napi::template_env::render(
            "struct_static_method_wrapper.jinja",
            minijinja::context! {
                attrs => attrs,
                js_name => full_js_name,
                rust_name => full_rust_name,
                params => params_str,
                return_annotation => final_return_ann,
                body => body,
            },
        ));
    }

    out
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
    fn struct_gen_function_exists() {}
}
