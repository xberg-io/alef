//! NAPI-RS struct, opaque type, and static method code generation.

use crate::backends::napi::type_map::NapiMapper;
use crate::codegen::builder::{ImplBuilder, StructBuilder};
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::generators::{
    self, RustBindingConfig, gen_delegating_deserialize_impl, struct_wants_deserialize_delegation,
};
use crate::codegen::naming::to_node_name;
use crate::codegen::shared::{binding_fields, can_auto_delegate, function_params, partition_methods};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{EnumDef, FieldDef, MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::{ToPascalCase, ToSnakeCase};

use super::enums::string_enum_js_values;
use super::functions::{
    napi_apply_primitive_casts_to_call_args, napi_gen_call_args, napi_newtype_param_bindings, napi_wrap_return,
    promoted_required, wrap_promoted_required_body,
};

mod opaque_methods;
#[cfg(test)]
pub(super) use opaque_methods::gen_opaque_instance_method;
pub(super) use opaque_methods::{
    gen_opaque_struct_methods, opaque_instance_method_is_dropped, opaque_static_method_is_dropped,
};

/// Whether the generated NAPI binding declares `field` of `owner` as `Option<T>`, i.e. whether
/// the emitted `.d.ts` spells it `name?: T` and a JS caller may read `undefined` from it.
///
/// ~keep In addition to fields that are inherently optional, a type implementing `Default`
/// widens every field to `Option<T>`, while a bare field-level serde default widens only that
/// field. e2e snippet codegen has to reach the same verdict as this emitter or it can generate
/// an unconditional property access against a `?`-typed member and fail `tsc` with `TS18048`,
/// so `FieldResolver::ir_result_field_facts` calls this named predicate too.
pub(crate) fn napi_field_is_optional(field: &FieldDef, owner: &TypeDef) -> bool {
    matches!(field.ty, TypeRef::Optional(_))
        || field.optional
        || owner.has_default
        || field.has_bare_serde_enum_default()
}

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

/// Map a struct-field `TypeRef` naming a `#[napi(string_enum)]` to a TS string-literal union.
///
/// napi-rs emits a string enum as a nominal TS `enum`, so a caller writing the plain literal the
/// enum accepts at runtime — `{ kind: "uri" }` — is rejected with *"Type '\"uri\"' is not
/// assignable to type 'ExtractInputKind'"*, even though the value is exactly what the binding
/// expects. Declaring the field as the union of the enum's own runtime values keeps the nominal
/// type usable while also accepting the literals, which is how the enum is documented and how
/// every JS caller naturally writes it.
///
/// The union comes from [`string_enum_js_values`], which mirrors the enum emitter, so the two
/// cannot drift. Enums that are not emitted as string enums yield `None` and keep the nominal type.
///
/// ~keep That mirroring covers WHICH variants exist, not just how each one spells its value:
/// `string_enum_js_values` filters through `codegen::conversions::enum_variant_declaration`, so a
/// foreign cfg-gated variant `gen_enum` proves unreachable and omits from the emitted Rust enum
/// never reaches this attribute either. Emitting it here made `ts_type` advertise a literal the
/// Rust side cannot represent.
fn ts_type_for_string_enum_field(
    ty: &TypeRef,
    enums: &[EnumDef],
    core_import: &str,
    configured_features: Option<&std::collections::HashSet<&str>>,
    types: &[TypeDef],
) -> Option<String> {
    fn inner(
        ty: &TypeRef,
        enums: &[EnumDef],
        core_import: &str,
        configured_features: Option<&std::collections::HashSet<&str>>,
        types: &[TypeDef],
    ) -> Option<String> {
        match ty {
            TypeRef::Named(name) => {
                let enum_def = enums.iter().find(|e| e.name == *name)?;
                let is_host_enum = crate::codegen::cfg::is_host_owned_rust_path(core_import, &enum_def.rust_path);
                let values = string_enum_js_values(enum_def, is_host_enum, configured_features, types)?;
                Some(format!(
                    "{} | {}",
                    name,
                    values
                        .iter()
                        .map(|value| format!("'{value}'"))
                        .collect::<Vec<_>>()
                        .join(" | ")
                ))
            }
            TypeRef::Optional(i) => {
                inner(i, enums, core_import, configured_features, types).map(|s| format!("{s} | null | undefined"))
            }
            TypeRef::Vec(i) => inner(i, enums, core_import, configured_features, types).map(|s| format!("Array<{s}>")),
            TypeRef::Map(_k, v) => {
                inner(v, enums, core_import, configured_features, types).map(|s| format!("Record<string, {s}>"))
            }
            _ => None,
        }
    }
    inner(ty, enums, core_import, configured_features, types)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &NapiMapper,
    prefix: &str,
    has_serde: bool,
    opaque_types: &ahash::AHashSet<String>,
    never_skip_cfg_field_names: &[String],
    enums: &[EnumDef],
    core_import: &str,
    core_to_binding_convertible: &ahash::AHashSet<String>,
    configured_features: Option<&std::collections::HashSet<&str>>,
    types: &[TypeDef],
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
    // napi has no per-language "serializable opaque" carve-out (unlike pyo3's
    // `serializable_opaque_type_names`), so an opaque field always disqualifies delegation here.
    let opaque_type_names: Vec<String> = opaque_types.iter().cloned().collect();
    let delegate_deserialize = has_serde
        && core_to_binding_convertible.contains(&typ.name)
        && struct_wants_deserialize_delegation(typ, &opaque_type_names, &[]);
    if has_serde {
        struct_builder.add_derive("serde::Serialize");
        if !delegate_deserialize {
            struct_builder.add_derive("serde::Deserialize");
        }
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
        let field_type = if napi_field_is_optional(field, typ) && !already_optional {
            format!("Option<{base_type}>")
        } else {
            base_type
        };
        // The napi `js_name` is the public JS identifier and must come from casing policy
        // (`to_node_name`) alone, never from `field.serde_rename` -- that field is the JSON
        // wire name (set by `#[serde(rename = "...")]` on the core struct for reasons that
        // can be entirely unrelated to this napi binding, e.g. an external HTTP API's field
        // name) and is a separate name surface. Reading `alef::codegen::naming`'s
        // "serde_rename/serde_rename_all define wire names only; they must not be used as
        // host-language public identifier casing rules" -- bleeding the wire rename into
        // `js_name` here previously made the compiled `.node` artifact expose e.g. `max_chars`
        // while `gen_dts` (this backend's `.d.ts` generator, a few files over) independently
        // computed `maxCharacters` for the very same field from the very same IR, since it
        // never consulted `serde_rename` in the first place. The two generators disagreed
        // about one field on one struct; this makes the wire name irrelevant to `js_name` so
        // they can't diverge again. ~keep
        let js_name = to_node_name(&field.name);
        let wire_name = crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            typ.serde_rename_all.as_deref(),
        );
        let ts_type_override = ts_type_for_bytes_field(&field.ty)
            .or_else(|| ts_type_for_string_enum_field(&field.ty, enums, core_import, configured_features, types));
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

        // Wire (JSON) renaming is independent of the JS-visible `js_name` above -- a field can
        // be `maxCharacters` in JS while still serializing as `max_chars` over JSON (or vice
        // versa). Emit `#[serde(rename = ...)]` from the field's own wire name only.
        if has_serde && wire_name != field.name {
            attrs.push(format!("serde(rename = \"{}\")", wire_name));
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
    if delegate_deserialize {
        out.push_str(&gen_delegating_deserialize_impl(typ, core_import, prefix, &[]));
    }
    out
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
    let has_promoted_required = !promoted_required(&method.params).is_empty();
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some() || has_promoted_required);

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
        let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
        let return_wrap = napi_wrap_return(
            &result,
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
            &napi_newtype_param_bindings(&method.params),
            matches!(method.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_type_path}::{}({call_args})", method.name);
        if method.error_type.is_some() {
            let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";
            let value = apply_return_newtype_unwrap("val", &method.return_newtype_wrapper);
            let wrapped = napi_wrap_return(
                &value,
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
            let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
            napi_wrap_return(
                &result,
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

    let body = if can_delegate_static && !method.is_async {
        format!("{}{body}", napi_newtype_param_bindings(&method.params))
    } else {
        body
    };

    let body = wrap_promoted_required_body(
        body,
        &method.params,
        method.error_type.is_some(),
        method.is_async,
        matches!(method.return_type, TypeRef::Unit),
    );

    let mut attrs = String::new();
    let sanitized_method_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&method.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
    if method.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() || has_promoted_required {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }
    // See `gen_opaque_instance_method` for why the gate is re-emitted rather than filtered. ~keep
    attrs.push_str(&method.rust_cfg_attribute());
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
        let has_promoted_required = !promoted_required(&method.params).is_empty();

        let mut param_parts: Vec<String> = vec![];
        if !is_static {
            param_parts.push(format!("cfg: {binding_type}"));
        }
        let method_params = function_params(&method.params, &|ty| mapper.map_type(ty));
        if !method_params.is_empty() {
            param_parts.push(method_params);
        }
        let params_str = param_parts.join(", ");

        let use_let_bindings = generators::has_named_params(&method.params, opaque_types);
        let (call_args_str, mut dto_let_bindings) = if use_let_bindings {
            (
                napi_apply_primitive_casts_to_call_args(
                    &if has_promoted_required {
                        generators::gen_call_args_with_let_bindings_mutex_no_promote(
                            &method.params,
                            opaque_types,
                            mutex_types,
                        )
                    } else {
                        generators::gen_call_args_with_let_bindings_mutex(&method.params, opaque_types, mutex_types)
                    },
                    &method.params,
                ),
                if has_promoted_required {
                    generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, cfg.core_import)
                } else {
                    generators::gen_named_let_bindings_pub(&method.params, opaque_types, cfg.core_import)
                },
            )
        } else {
            (napi_gen_call_args(&method.params, opaque_types), String::new())
        };
        dto_let_bindings.push_str(&napi_newtype_param_bindings(&method.params));

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
                let value = apply_return_newtype_unwrap("val", &method.return_newtype_wrapper);
                let wrapped = napi_wrap_return(
                    &value,
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
                let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
                napi_wrap_return(
                    &result,
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

        let body = if !dto_let_bindings.is_empty() {
            format!("{dto_let_bindings}{body}")
        } else {
            body
        };
        let body = wrap_promoted_required_body(
            body,
            &method.params,
            method.error_type.is_some() || !is_static,
            method.is_async,
            matches!(method.return_type, TypeRef::Unit),
        );

        let mut attrs = String::new();
        let sanitized_method_doc = crate::codegen::doc_emission::sanitize_rust_idioms(
            &method.doc,
            crate::codegen::doc_emission::DocTarget::TsDoc,
        );
        crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_method_doc, "");
        if method.error_type.is_some() || !is_static || has_promoted_required {
            attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
        }
        // DTO methods become standalone `#[napi]` free functions, so the gate lands on the same
        // kind of item `support::prepend_cfg` already gates for top-level functions. ~keep
        attrs.push_str(&method.rust_cfg_attribute());
        let returns_result = method.error_type.is_some() || !is_static || has_promoted_required;
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
mod tests;
