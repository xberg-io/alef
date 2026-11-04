use super::super::containers::ContainerContext;
use super::super::types::gen_opaque_instance_method;
use crate::backends::napi::type_map::NapiMapper;
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef,
};
use quote::ToTokens;

/// A DTO whose binding converts to core through `TryFrom` (it holds a JSON-passthrough enum).
fn fallible_request_api() -> ApiSurface {
    ApiSurface {
        types: vec![TypeDef {
            name: "ChatRequest".into(),
            rust_path: "sample_core::ChatRequest".into(),
            fields: vec![FieldDef {
                name: "policy".into(),
                ty: TypeRef::Named("Policy".into()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        enums: vec![EnumDef {
            name: "Policy".into(),
            rust_path: "sample_core::Policy".into(),
            serde_untagged: true,
            variants: vec![EnumVariant {
                name: "Text".into(),
                fields: vec![FieldDef {
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn identifiers(block: &syn::Block) -> Vec<String> {
    block
        .to_token_stream()
        .into_iter()
        .filter_map(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => Some(ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Regression: an adapter-backed async method receives its body from the shared adapter
/// template (`let core_req = req.into();`), while the NAPI input pass converted the fallible DTO
/// into a separate `req_core` local. `req.into()` then needs `From<JsChatRequest>`, which a
/// `TryFrom`-converted DTO does not implement (E0277). The converted value must be what the
/// adapter body consumes.
#[test]
fn adapter_body_consumes_the_fallibly_converted_input() {
    let api = fallible_request_api();
    let context = ContainerContext::new(&api, "sample_core");
    let method = MethodDef {
        name: "chat".into(),
        params: vec![ParamDef {
            name: "req".into(),
            ty: TypeRef::Named("ChatRequest".into()),
            ..Default::default()
        }],
        return_type: TypeRef::Named("ChatResponse".into()),
        error_type: Some("Error".into()),
        receiver: Some(ReceiverKind::Ref),
        is_async: true,
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Client".into(),
        rust_path: "sample_core::Client".into(),
        is_opaque: true,
        methods: vec![method.clone()],
        ..Default::default()
    };
    let adapter_body = crate::codegen::template_env::render(
        "adapters/async_node_request.rs.jinja",
        crate::alef_context! {
            call_str => "req.into()",
            core_call => "self.inner.chat(core_req)",
            map_expr => ".map(JsChatResponse::from)",
        },
    );
    let mut adapter_bodies = crate::adapters::AdapterBodies::new();
    adapter_bodies.insert("Client.chat".into(), adapter_body.trim_end().to_owned());

    let code = gen_opaque_instance_method(
        &method,
        &NapiMapper::new("Js".into()),
        &typ,
        &super::super::NapiBackend::binding_config("sample_core", "Js", true),
        &ahash::AHashSet::from(["Client".to_string()]),
        "Js",
        &adapter_bodies,
        &ahash::AHashMap::new(),
        &ahash::AHashSet::new(),
        &std::collections::HashMap::new(),
        Some(&context),
    );

    let function: syn::ImplItemFn = syn::parse_str(&code).expect("generated method must parse");
    let statements = &function.block.stmts;
    let syn::Stmt::Local(converted) = &statements[0] else {
        panic!("the fallible conversion must come first, got:\n{code}")
    };
    // `try_into()` has no target type to infer from the adapter's `req.into()`, so the
    // binding must name the core type. ~keep
    let syn::Pat::Type(annotated) = &converted.pat else {
        panic!("conversion must annotate its core type, got:\n{code}")
    };
    let annotation = annotated.ty.to_token_stream().to_string();
    assert!(annotation.starts_with("sample_core ::"), "{annotation}\n{code}");
    let syn::Pat::Ident(converted_name) = annotated.pat.as_ref() else {
        panic!("conversion must bind a plain identifier, got:\n{code}")
    };
    assert_eq!(
        converted_name.ident, "req",
        "the converted value must shadow the parameter the adapter body reads, got:\n{code}"
    );
    let initializer = converted.init.as_ref().unwrap().expr.to_token_stream().to_string();
    assert!(initializer.contains("try_into"), "{initializer}");
    let syn::Stmt::Local(adapter_local) = &statements[1] else {
        panic!("the adapter body must follow the conversion, got:\n{code}")
    };
    assert_eq!(
        adapter_local.init.as_ref().unwrap().expr.to_token_stream().to_string(),
        "req . into ()",
        "{code}"
    );
    assert!(
        !identifiers(&function.block).iter().any(|ident| ident == "req_core"),
        "no unused separately-named conversion may remain, got:\n{code}"
    );
}

/// Regression: a DTO that no function takes as input is emitted by the fallback conversion pass,
/// which rendered a plain `From` with `.into()` on its fields. When a field holds a DTO whose
/// conversion is fallible, that `From` needed `From<JsChatRequest>`, which a `TryFrom`-converted
/// DTO does not implement (E0277). The fallback must convert through the same fallible policy.
#[test]
fn fallback_conversion_of_a_dto_holding_a_fallible_dto_is_fallible() {
    use crate::core::backend::Backend;

    let mut api = fallible_request_api();
    api.crate_name = "sample-core".into();
    api.version = "0.1.0".into();
    api.types.push(TypeDef {
        name: "Batch".into(),
        rust_path: "sample_core::Batch".into(),
        fields: vec![FieldDef {
            name: "requests".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named("ChatRequest".into()))),
            ..Default::default()
        }],
        ..Default::default()
    });
    let config: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"sample-core\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = super::super::NapiBackend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap();
    let lib = &files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap().content;
    let compact: String = lib.chars().filter(|character| !character.is_whitespace()).collect();
    for name in ["ChatRequest", "Batch"] {
        assert!(
            compact.contains(&format!("implTryFrom<Js{name}>forsample_core::{name}")),
            "{name} must convert fallibly:\n{lib}"
        );
        assert!(
            !compact.contains(&format!("implFrom<Js{name}>forsample_core::{name}")),
            "{name} must not also convert infallibly:\n{lib}"
        );
    }
}
