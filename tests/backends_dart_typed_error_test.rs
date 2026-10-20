//! A fallible Dart bridge call returns the mirrored error, so Dart receives the typed error with
//! its variant and fields instead of the error text.

use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{
    ApiSurface, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
    TypeDef, TypeRef,
};

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]
"#,
    )
    .expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> ErrorVariant {
    ErrorVariant {
        name: name.to_string(),
        fields,
        ..Default::default()
    }
}

fn api_error(excluded: bool) -> ErrorDef {
    ErrorDef {
        name: "ApiErr".to_string(),
        rust_path: "demo_crate::ApiErr".to_string(),
        variants: vec![
            variant(
                "RateLimit",
                vec![
                    field("message", TypeRef::String, false),
                    field("retry_after", TypeRef::Duration, true),
                ],
            ),
            variant(
                "Status",
                vec![field("status", TypeRef::Primitive(PrimitiveType::U16), false)],
            ),
            ErrorVariant {
                name: "Timeout".to_string(),
                is_unit: true,
                ..Default::default()
            },
            ErrorVariant {
                name: "Serialization".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    binding_excluded: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: excluded,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn send_function() -> FunctionDef {
    FunctionDef {
        name: "send".to_string(),
        rust_path: "demo_crate::send".to_string(),
        params: vec![ParamDef {
            name: "raw".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: alef::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::String,
        error_type: Some("ApiErr".to_string()),
        ..Default::default()
    }
}

fn client_with_fallible_method() -> TypeDef {
    TypeDef {
        name: "Client".to_string(),
        rust_path: "demo_crate::Client".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "fetch".to_string(),
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("ApiErr".to_string()),
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated_lib(errors: Vec<ErrorDef>) -> String {
    let api = ApiSurface {
        crate_name: "demo-crate".to_string(),
        version: "0.1.0".to_string(),
        types: vec![client_with_fallible_method()],
        functions: vec![send_function()],
        errors,
        ..Default::default()
    };
    let files = DartBackend.generate_bindings(&api, &config()).unwrap();
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("packages/dart/rust/src/lib.rs"))
        .map(|f| f.content.clone())
        .expect("lib.rs not found")
}

#[test]
fn fallible_bridge_fn_returns_the_mirrored_error_not_a_string() {
    let lib = generated_lib(vec![api_error(false)]);

    assert!(
        lib.contains("pub fn send(raw: String) -> Result<String, ApiErr>"),
        "the bridge must return the mirrored error: {lib}"
    );
    assert!(
        lib.contains(".map_err(ApiErr::from)"),
        "the bridge must convert the core error into the mirror: {lib}"
    );
    assert!(
        !lib.contains(".map_err(|e| e.to_string())"),
        "no error text conversion may remain on a mirrored error: {lib}"
    );
}

#[test]
fn fallible_opaque_method_returns_the_mirrored_error_not_a_string() {
    let lib = generated_lib(vec![api_error(false)]);

    assert!(
        lib.contains("-> Result<String, ApiErr> {") && lib.contains("pub async fn fetch(&self"),
        "the opaque method must return the mirrored error: {lib}"
    );
}

#[test]
fn core_error_converts_into_every_mirror_variant_shape() {
    let lib = generated_lib(vec![api_error(false)]);

    assert!(
        lib.contains("impl From<demo_crate::ApiErr> for ApiErr"),
        "the core error must convert into the mirror: {lib}"
    );
    assert!(
        lib.contains("retry_after: f_retry_after.map(|v| v.as_millis() as i64)"),
        "an optional duration must stay optional in the conversion: {lib}"
    );
    assert!(
        lib.contains("demo_crate::ApiErr::Status { status: f_status, .. } => ApiErr::Status { status: f_status as _ }"),
        "an integer field must widen in the conversion: {lib}"
    );
    assert!(
        lib.contains("demo_crate::ApiErr::Timeout => ApiErr::Timeout,"),
        "a unit variant must convert to the unit variant: {lib}"
    );
    assert!(
        lib.contains("demo_crate::ApiErr::Serialization { .. } => ApiErr::Serialization { field0: e.to_string() }"),
        "a variant with only excluded fields must carry the message: {lib}"
    );
}

#[test]
fn error_excluded_from_the_binding_keeps_the_string_error() {
    let lib = generated_lib(vec![api_error(true)]);

    assert!(
        lib.contains("pub fn send(raw: String) -> Result<String, String>"),
        "an error with no mirror reports its text: {lib}"
    );
    assert!(lib.contains(".map_err(|e| e.to_string())"), "{lib}");
    assert!(!lib.contains("impl From<demo_crate::ApiErr> for ApiErr"), "{lib}");
}
