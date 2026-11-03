use crate::backends::dart::type_map::{DartMapper, dart_primitive_is_bigint};
use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::codegen::type_mapper::TypeMapper;
use crate::core::config::Language;
use crate::core::config::{AdapterConfig, ResolvedCrateConfig};
use crate::core::ir::{ErrorDef, PrimitiveType, TypeDef, TypeRef};
use quote::ToTokens;
use std::collections::HashMap;

pub(super) fn emit_streaming_sink_method(
    out: &mut String,
    method_name: &str,
    adapter: &AdapterConfig,
    errors: &[ErrorDef],
    config: &ResolvedCrateConfig,
) -> anyhow::Result<()> {
    let params = adapter
        .params
        .iter()
        .map(|param| {
            let ty = if param.optional {
                format!("Option<{}>", param.ty)
            } else {
                param.ty.clone()
            };
            format!(", {}: {ty}", param.name)
        })
        .collect::<String>();
    let typed_error = error_definition(adapter, config, errors).is_some();
    let (body, _) = crate::adapters::streaming::dart_body(adapter, config, typed_error)?;
    let item_type = if typed_error {
        crate::adapters::streaming::dart_transport_names(adapter).carrier
    } else {
        adapter.item_type.clone().unwrap_or("()".into())
    };
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_streaming_sink_method.rs.jinja",
        crate::alef_context! {
            method_name => method_name,
            params_str => params,
            item_type => item_type,
            body => body,
        },
    ));
    Ok(())
}

fn error_definition<'a>(
    adapter: &AdapterConfig,
    config: &ResolvedCrateConfig,
    errors: &'a [ErrorDef],
) -> Option<&'a ErrorDef> {
    let error_type = adapter.error_type.as_deref();
    super::error_bridge::typed_error_definition(error_type, errors).or_else(|| {
        let prefix = format!("{}::", config.core_import_name());
        let local = error_type.and_then(|name| name.strip_prefix(&prefix));
        super::error_bridge::typed_error_definition(local, errors)
    })
}

pub(super) fn emit_transports(
    out: &mut String,
    ty: &TypeDef,
    stub_methods: &[String],
    adapters: &HashMap<String, &AdapterConfig>,
    config: &ResolvedCrateConfig,
    errors: &[ErrorDef],
) {
    for method in &ty.methods {
        if !method.sanitized || stub_methods.contains(&method.name) {
            continue;
        }
        let Some(adapter) = adapters.get(&format!("{}.{}", ty.name, method.name)) else {
            continue;
        };
        let Some(error) = error_definition(adapter, config, errors) else {
            continue;
        };
        emit_transport(out, adapter, error, config, ty.cfg.as_deref().unwrap_or(""));
    }
}

fn emit_transport(
    out: &mut String,
    adapter: &AdapterConfig,
    error: &ErrorDef,
    config: &ResolvedCrateConfig,
    owner_cfg: &str,
) {
    let transport = crate::adapters::streaming::dart_transport_names(adapter);
    let item_type = adapter.item_type.as_deref().unwrap_or("()");
    let (wire_item_type, item_conversion) = stream_wire_item(item_type);
    let dart_wire = public_host_identifier(Language::Dart, PublicIdentifierKind::Type, &transport.wire);
    let arms = [
        if transport.unit_item {
            format!("{dart_wire}_Item() => null")
        } else {
            format!("{dart_wire}_Item(:final value) => value")
        },
        format!("{dart_wire}_Error(:final error) => throw error"),
        format!("{dart_wire}_NativeFailure(:final message) => throw StateError(message)"),
    ];
    let decode = format!("switch ({{}}) {{ {} }}", arms.join(", "));
    let factory = public_host_identifier(Language::Dart, PublicIdentifierKind::Method, "Item");
    let encode = if transport.unit_item {
        format!("{dart_wire}.{factory}()")
    } else {
        format!("{dart_wire}.{factory}(value: {{}})")
    };
    out.push_str(&crate::backends::dart::template_env::render(
        "rust_streaming_transport.rs.jinja",
        crate::alef_context! {
            transport => &transport,
            source_cfg => super::helpers::widen_opaque_wrapper_cfg(owner_cfg),
            item_type => item_type,
            wire_item_type => wire_item_type,
            item_conversion => item_conversion,
            error_type => &error.name,
            core_error_type => super::error_bridge::core_error_path(error, &config.core_import_name()),
            dart_type => dart_item_type(adapter.item_type.as_deref().unwrap_or("()")),
            decode_code => format!("{decode:?}"),
            encode_code => format!("{encode:?}"),
        },
    ));
}

fn stream_wire_item(item_type: &str) -> (String, String) {
    let Ok(mut ty) = syn::parse_str::<syn::Type>(item_type) else {
        return (item_type.into(), "value".into());
    };
    let resolved = crate::extract::type_resolver::resolve_type(&ty);
    if !widen_stream_usize(&mut ty) {
        return (item_type.into(), "value".into());
    }
    (
        ty.to_token_stream().to_string(),
        super::newtype_conversions::from_core_value("value", &resolved),
    )
}

fn widen_stream_usize(ty: &mut syn::Type) -> bool {
    let syn::Type::Path(path) = ty else { return false };
    if path.path.is_ident("usize") {
        let wire = super::conversions::frb_rust_type_inner(&TypeRef::Primitive(PrimitiveType::Usize));
        let ident = &mut path.path.segments[0].ident;
        *ident = syn::Ident::new(&wire, ident.span());
        return true;
    }
    let Some(segment) = path.path.segments.last_mut() else {
        return false;
    };
    if !matches!(
        segment.ident.to_string().as_str(),
        "Option" | "Vec" | "HashMap" | "BTreeMap"
    ) {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &mut segment.arguments else {
        return false;
    };
    let mut changed = false;
    for argument in &mut arguments.args {
        if let syn::GenericArgument::Type(inner) = argument {
            changed |= widen_stream_usize(inner);
        }
    }
    changed
}

fn dart_item_type(item_type: &str) -> String {
    let ty = syn::parse_str::<syn::Type>(item_type)
        .map(|ty| crate::extract::type_resolver::resolve_type(&ty))
        .unwrap_or_else(|_| TypeRef::Named(item_type.into()));
    native_dart_type(&ty)
}

fn native_dart_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::Isize) => "PlatformInt64".into(),
        TypeRef::Optional(inner) => format!("{}?", native_dart_type(inner)),
        TypeRef::Vec(inner) => native_typed_list(inner)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("List<{}>", native_dart_type(inner))),
        TypeRef::Map(key, value) => format!("Map<{}, {}>", native_dart_type(key), native_dart_type(value)),
        _ => DartMapper.map_type(ty),
    }
}

fn native_typed_list(ty: &TypeRef) -> Option<&'static str> {
    Some(match ty {
        TypeRef::Primitive(PrimitiveType::U8) => "Uint8List",
        TypeRef::Primitive(PrimitiveType::I8) => "Int8List",
        TypeRef::Primitive(PrimitiveType::U16) => "Uint16List",
        TypeRef::Primitive(PrimitiveType::I16) => "Int16List",
        TypeRef::Primitive(PrimitiveType::U32) => "Uint32List",
        TypeRef::Primitive(PrimitiveType::I32) => "Int32List",
        TypeRef::Primitive(prim) if dart_primitive_is_bigint(prim) => "Uint64List",
        TypeRef::Primitive(PrimitiveType::I64 | PrimitiveType::Isize | PrimitiveType::Usize) => "Int64List",
        TypeRef::Primitive(PrimitiveType::F32) => "Float32List",
        TypeRef::Primitive(PrimitiveType::F64) => "Float64List",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn stream_codec_uses_dart_types_for_native_items() {
        for (rust, dart) in [
            ("Chunk", "Chunk"),
            ("String", "String"),
            ("u32", "int"),
            ("f64", "double"),
            ("bool", "bool"),
            ("u64", "BigInt"),
            ("i64", "PlatformInt64"),
            ("Option<u64>", "BigInt?"),
            ("Vec<u32>", "Uint32List"),
            ("Vec<f32>", "Float32List"),
            ("Vec<Option<u64>>", "List<BigInt?>"),
            ("()", "void"),
        ] {
            assert_eq!(super::dart_item_type(rust), dart, "{rust}");
        }
    }
}

#[cfg(test)]
#[path = "streaming_layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "streaming_type_tests.rs"]
mod type_tests;
