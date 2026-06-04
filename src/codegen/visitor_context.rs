use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};

pub(crate) enum VisitorContextBackend {
    Napi,
    Wasm,
    Pyo3,
    Magnus,
    Extendr,
    Rustler,
}

pub(crate) struct VisitorContextHelper {
    pub type_path: String,
    pub field_lines: String,
}

pub(crate) fn visitor_context_helper(
    api: &ApiSurface,
    bridge_cfg: &TraitBridgeConfig,
    core_crate: &str,
    backend: VisitorContextBackend,
) -> anyhow::Result<VisitorContextHelper> {
    let context_type = bridge_cfg.context_type.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "trait bridge `{}` must configure context_type for visitor context conversion",
            bridge_cfg.trait_name
        )
    })?;
    let context_def = api
        .types
        .iter()
        .find(|type_def| type_def.name == context_type)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "trait bridge `{}` configures context_type `{context_type}`, but no matching type exists in the API surface",
                bridge_cfg.trait_name
            )
        })?;
    let type_path = non_empty_path(&context_def.rust_path, core_crate, context_type);
    let field_lines = context_def
        .fields
        .iter()
        .map(|field| context_field_line(api, context_def, field, &backend))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("\n");
    Ok(VisitorContextHelper { type_path, field_lines })
}

fn context_field_line(
    api: &ApiSurface,
    context_def: &TypeDef,
    field: &FieldDef,
    backend: &VisitorContextBackend,
) -> anyhow::Result<String> {
    let shape = field_shape(api, field).ok_or_else(|| {
        anyhow::anyhow!(
            "trait bridge context type `{}` field `{}` has unsupported type `{}`",
            context_def.name,
            field.name,
            type_ref_label(&field.ty)
        )
    })?;
    Ok(match backend {
        VisitorContextBackend::Napi => napi_field_line(field, shape),
        VisitorContextBackend::Wasm => wasm_field_line(field, shape),
        VisitorContextBackend::Pyo3 => pyo3_field_line(field, shape),
        VisitorContextBackend::Magnus => magnus_field_line(field, shape),
        VisitorContextBackend::Extendr => extendr_field_line(field, shape),
        VisitorContextBackend::Rustler => rustler_field_line(field, shape),
    })
}

#[derive(Clone, Copy)]
enum FieldShape {
    String,
    OptionalString,
    Bool,
    Number,
    Enum,
    StringMap,
    StringVec,
}

fn field_shape(api: &ApiSurface, field: &FieldDef) -> Option<FieldShape> {
    let optional = field.optional || matches!(field.ty, TypeRef::Optional(_));
    let ty = match &field.ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            if optional {
                Some(FieldShape::OptionalString)
            } else {
                Some(FieldShape::String)
            }
        }
        TypeRef::Primitive(PrimitiveType::Bool) => Some(FieldShape::Bool),
        TypeRef::Primitive(_) | TypeRef::Duration => Some(FieldShape::Number),
        TypeRef::Named(name) if api.enums.iter().any(|enum_def| enum_def.name == *name) => Some(FieldShape::Enum),
        TypeRef::Map(key, value) if matches!((key.as_ref(), value.as_ref()), (TypeRef::String, TypeRef::String)) => {
            Some(FieldShape::StringMap)
        }
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => Some(FieldShape::StringVec),
        _ => None,
    }
}

fn napi_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_node_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => {
            format!(r#"    obj.set_named_property("{host_name}", env.create_string(&ctx.{name})?)?;"#)
        }
        FieldShape::OptionalString => format!(
            r#"    let {name}_value = match &ctx.{name} {{
        Some(value) => env.create_string(value)?.to_unknown(),
        None => {{
            let raw = unsafe {{ napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), napi::bindgen_prelude::Null)? }};
            unsafe {{ napi::bindgen_prelude::Unknown::from_raw_unchecked(env.raw(), raw) }}
        }}
    }};
    obj.set_named_property("{host_name}", {name}_value)?;"#
        ),
        FieldShape::Bool => format!(r#"    obj.set_named_property("{host_name}", ctx.{name})?;"#),
        FieldShape::Number => {
            format!(r#"    obj.set_named_property("{host_name}", env.create_double(ctx.{name} as f64)?)?;"#)
        }
        FieldShape::Enum => {
            format!(
                r#"    obj.set_named_property("{host_name}", env.create_string(&format!("{{:?}}", ctx.{name}))?)?;"#
            )
        }
        FieldShape::StringMap => format!(
            r#"    let mut {name}_obj = napi::bindgen_prelude::Object::new(env)?;
    for (key, value) in &ctx.{name} {{
        {name}_obj.set_named_property(key, env.create_string(value)?)?;
    }}
    obj.set_named_property("{host_name}", {name}_obj)?;"#
        ),
        FieldShape::StringVec => format!(
            r#"    let mut {name}_array = env.create_array_with_length(ctx.{name}.len())?;
    for (index, value) in ctx.{name}.iter().enumerate() {{
        {name}_array.set_element(index as u32, env.create_string(value)?)?;
    }}
    obj.set_named_property("{host_name}", {name}_array)?;"#
        ),
    }
}

fn wasm_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_node_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => reflect_set(&host_name, &format!("wasm_bindgen::JsValue::from_str(&ctx.{name})")),
        FieldShape::OptionalString => format!(
            r#"    let {name}_value = match &ctx.{name} {{
        Some(value) => wasm_bindgen::JsValue::from_str(value),
        None => wasm_bindgen::JsValue::null(),
    }};
{}"#,
            reflect_set(&host_name, &format!("{name}_value"))
        ),
        FieldShape::Bool => reflect_set(&host_name, &format!("wasm_bindgen::JsValue::from_bool(ctx.{name})")),
        FieldShape::Number => reflect_set(
            &host_name,
            &format!("wasm_bindgen::JsValue::from_f64(ctx.{name} as f64)"),
        ),
        FieldShape::Enum => reflect_set(
            &host_name,
            &format!("wasm_bindgen::JsValue::from_str(&format!(\"{{:?}}\", ctx.{name}))"),
        ),
        FieldShape::StringMap | FieldShape::StringVec => reflect_set(
            &host_name,
            &format!("serde_wasm_bindgen::to_value(&ctx.{name}).unwrap_or(wasm_bindgen::JsValue::NULL)"),
        ),
    }
}

fn pyo3_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_python_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => format!(r#"    d.set_item("{host_name}", &ctx.{name}).unwrap_or(());"#),
        FieldShape::OptionalString => format!(r#"    d.set_item("{host_name}", ctx.{name}.as_deref()).unwrap_or(());"#),
        FieldShape::Bool | FieldShape::Number => format!(r#"    d.set_item("{host_name}", ctx.{name}).unwrap_or(());"#),
        FieldShape::Enum => format!(r#"    d.set_item("{host_name}", format!("{{:?}}", ctx.{name})).unwrap_or(());"#),
        FieldShape::StringMap | FieldShape::StringVec => {
            format!(r#"    d.set_item("{host_name}", &ctx.{name}).unwrap_or(());"#)
        }
    }
}

fn magnus_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_ruby_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => format!(r#"    h.aset(ruby.to_symbol("{host_name}"), ctx.{name}.as_str()).ok();"#),
        FieldShape::OptionalString => format!(
            r#"    h.aset(ruby.to_symbol("{host_name}"), ctx.{name}.as_deref().map(|value| ruby.str_new(value).as_value())).ok();"#
        ),
        FieldShape::Bool => format!(r#"    h.aset(ruby.to_symbol("{host_name}"), ctx.{name}).ok();"#),
        FieldShape::Number => format!(r#"    h.aset(ruby.to_symbol("{host_name}"), ctx.{name} as i64).ok();"#),
        FieldShape::Enum => {
            format!(r#"    h.aset(ruby.to_symbol("{host_name}"), format!("{{:?}}", ctx.{name})).ok();"#)
        }
        FieldShape::StringMap => format!(
            r#"    let {name}_hash = ruby.hash_new();
    for (key, value) in &ctx.{name} {{
        {name}_hash.aset(ruby.str_new(key), ruby.str_new(value)).ok();
    }}
    h.aset(ruby.to_symbol("{host_name}"), {name}_hash).ok();"#
        ),
        FieldShape::StringVec => format!(
            r#"    let {name}_array = ruby.ary_new_capa(ctx.{name}.len());
    for value in &ctx.{name} {{
        let _ = {name}_array.push(ruby.str_new(value));
    }}
    h.aset(ruby.to_symbol("{host_name}"), {name}_array).ok();"#
        ),
    }
}

fn extendr_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_python_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => {
            format!(r#"    pairs.push(("{host_name}", extendr_api::Robj::from(ctx.{name}.as_str())));"#)
        }
        FieldShape::OptionalString => format!(
            r#"    pairs.push(("{host_name}", match ctx.{name}.as_deref() {{
        Some(value) => extendr_api::Robj::from(value),
        None => extendr_api::Robj::from(extendr_api::NULL),
    }}));"#
        ),
        FieldShape::Bool => format!(r#"    pairs.push(("{host_name}", extendr_api::Robj::from(ctx.{name})));"#),
        FieldShape::Number => {
            format!(r#"    pairs.push(("{host_name}", extendr_api::Robj::from(ctx.{name} as f64)));"#)
        }
        FieldShape::Enum => format!(
            r#"    pairs.push(("{host_name}", extendr_api::Robj::from(format!("{{:?}}", ctx.{name}).as_str())));"#
        ),
        FieldShape::StringMap => format!(
            r#"    let {name}_pairs: Vec<(&str, extendr_api::Robj)> = ctx.{name}.iter()
        .map(|(key, value)| (key.as_str(), extendr_api::Robj::from(value.as_str())))
        .collect();
    pairs.push(("{host_name}", extendr_api::prelude::List::from_pairs({name}_pairs).into()));"#
        ),
        FieldShape::StringVec => {
            format!(r#"    pairs.push(("{host_name}", extendr_api::Robj::from(ctx.{name}.clone())));"#)
        }
    }
}

fn rustler_field_line(field: &FieldDef, shape: FieldShape) -> String {
    let host_name = crate::codegen::naming::to_elixir_name(&field.name);
    let name = &field.name;
    match shape {
        FieldShape::String => rustler_pair(&host_name, &format!("ctx.{name}.encode(env)")),
        FieldShape::OptionalString => {
            let value_expr = format!(
                r#"match ctx.{name}.as_deref() {{
        Some(value) => value.encode(env),
        None => rustler::types::atom::Atom::from_str(env, "nil").unwrap().to_term(env),
    }}"#
            );
            rustler_pair(&host_name, &value_expr)
        }
        FieldShape::Bool => rustler_pair(&host_name, &format!("ctx.{name}.encode(env)")),
        FieldShape::Number => rustler_pair(&host_name, &format!("(ctx.{name} as i64).encode(env)")),
        FieldShape::Enum => rustler_pair(&host_name, &format!("format!(\"{{:?}}\", ctx.{name}).encode(env)")),
        FieldShape::StringMap => {
            let value_expr = format!(
                r#"rustler::Term::map_from_pairs(
        env,
        &ctx.{name}
            .iter()
            .map(|(key, value)| (key.encode(env), value.encode(env)))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, "nil").unwrap().to_term(env))"#
            );
            rustler_pair(&host_name, &value_expr)
        }
        FieldShape::StringVec => rustler_pair(&host_name, &format!("ctx.{name}.encode(env)")),
    }
}

fn reflect_set(host_name: &str, value_expr: &str) -> String {
    format!(r#"    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str("{host_name}"), &{value_expr}).ok();"#)
}

fn rustler_pair(host_name: &str, value_expr: &str) -> String {
    format!(
        r#"    pairs.push((rustler::types::atom::Atom::from_str(env, "{host_name}").unwrap().to_term(env), {value_expr}));"#
    )
}

fn type_ref_label(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(primitive) => format!("{primitive:?}"),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "bytes".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_label(inner)),
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_label(inner)),
        TypeRef::Map(key, value) => format!("Map<{}, {}>", type_ref_label(key), type_ref_label(value)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Path => "Path".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "Json".to_string(),
        TypeRef::Duration => "Duration".to_string(),
    }
}

fn non_empty_path(path: &str, core_crate: &str, type_name: &str) -> String {
    let normalized = path.replace('-', "_");
    if normalized.is_empty() {
        format!("{core_crate}::{type_name}")
    } else {
        normalized
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{
        ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
    };

    pub(crate) fn neutral_visitor_fixture() -> (ApiSurface, TypeDef, TraitBridgeConfig) {
        let context = TypeDef {
            name: "TraversalState".to_string(),
            rust_path: "sample_core::walk::TraversalState".to_string(),
            fields: vec![
                field("kind", TypeRef::Named("TraversalKind".to_string()), false),
                field("display_name", TypeRef::String, false),
                field("depth", TypeRef::Primitive(PrimitiveType::Usize), false),
                field("enabled", TypeRef::Primitive(PrimitiveType::Bool), false),
                field("parent_label", TypeRef::Optional(Box::new(TypeRef::String)), true),
            ],
            doc: String::new(),
            ..TypeDef::default()
        };
        let trait_type = TypeDef {
            name: "DocumentWalker".to_string(),
            rust_path: "sample_core::walk::DocumentWalker".to_string(),
            methods: vec![MethodDef {
                name: "inspect_node".to_string(),
                params: vec![
                    ParamDef {
                        name: "state".to_string(),
                        ty: TypeRef::Named("TraversalState".to_string()),
                        is_ref: true,
                        ..ParamDef::default()
                    },
                    ParamDef {
                        name: "label".to_string(),
                        ty: TypeRef::String,
                        is_ref: true,
                        ..ParamDef::default()
                    },
                ],
                return_type: TypeRef::Named("WalkOutcome".to_string()),
                is_async: false,
                is_static: false,
                error_type: None,
                doc: String::new(),
                receiver: None,
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: true,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_trait: true,
            doc: String::new(),
            ..TypeDef::default()
        };
        let api = ApiSurface {
            crate_name: "sample_core".to_string(),
            types: vec![context, trait_type.clone()],
            enums: vec![
                EnumDef {
                    name: "TraversalKind".to_string(),
                    rust_path: "sample_core::walk::TraversalKind".to_string(),
                    variants: vec![EnumVariant {
                        name: "Section".to_string(),
                        ..EnumVariant::default()
                    }],
                    ..EnumDef::default()
                },
                EnumDef {
                    name: "WalkOutcome".to_string(),
                    rust_path: "sample_core::walk::WalkOutcome".to_string(),
                    variants: vec![
                        EnumVariant {
                            name: "KeepGoing".to_string(),
                            is_default: true,
                            ..EnumVariant::default()
                        },
                        EnumVariant {
                            name: "StopHere".to_string(),
                            ..EnumVariant::default()
                        },
                    ],
                    ..EnumDef::default()
                },
            ],
            ..ApiSurface::default()
};
        let bridge = TraitBridgeConfig {
            trait_name: "DocumentWalker".to_string(),
            type_alias: Some("DocumentWalkerHandle".to_string()),
            context_type: Some("TraversalState".to_string()),
            result_type: Some("WalkOutcome".to_string()),
            ..TraitBridgeConfig::default()
        };
        (api, trait_type, bridge)
    }

    pub(crate) fn assert_neutral_visitor_output(code: &str) {
        assert!(code.contains("DocumentWalker"));
        assert!(code.contains("TraversalState"));
        assert!(code.contains("WalkOutcome::KeepGoing"));
        assert!(code.contains("display"));
        assert!(!code.contains("HtmlVisitor"));
        assert!(!code.contains("NodeContext"));
        assert!(!code.contains("VisitResult::Continue"));
    }

    fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: Default::default(),
            vec_inner_core_wrapper: Default::default(),
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        }
    }
}
