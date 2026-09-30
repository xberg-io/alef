//! The Rust reference page documents the Rust crate itself, so it must show canonical Rust --
//! borrows, `&mut`, and the real source type names -- rather than the binding-normalized shape
//! every other language page is built from.

use super::*;
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, RustDocType};

fn borrowed_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        is_ref: true,
        ..make_param(name, TypeRef::Named(type_name.to_string()), false)
    }
}

fn mutably_borrowed_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        is_ref: true,
        is_mut: true,
        ..make_param(name, TypeRef::Named(type_name.to_string()), false)
    }
}

fn rust_and_python_pages(api: &ApiSurface) -> (String, String) {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "rust"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let files = generate_docs(api, &config, &[Language::Python, Language::Rust], "out").unwrap();
    (
        doc_content(&files, "api-rust").to_string(),
        doc_content(&files, "api-python").to_string(),
    )
}

fn api_with_borrowed_function() -> ApiSurface {
    let mut api = make_minimal_api("1.0.0");
    api.types = vec![empty_type("TextOptions")];
    api.functions = vec![make_function(
        "convert",
        vec![
            make_param("input", TypeRef::String, false),
            borrowed_param("options", "TextOptions"),
        ],
        TypeRef::String,
        false,
        None,
    )];
    api
}

#[test]
fn rust_reference_renders_a_borrowed_function_param_as_a_reference() {
    let (rust, _) = rust_and_python_pages(&api_with_borrowed_function());
    assert!(
        rust.contains("options: &TextOptions"),
        "rust reference must borrow the param the source borrows; got:\n{rust}"
    );
    assert!(
        !rust.contains("options: TextOptions"),
        "rust reference must not document the by-value binding shape; got:\n{rust}"
    );
}

#[test]
fn binding_reference_still_normalizes_a_borrowed_function_param() {
    let (_, python) = rust_and_python_pages(&api_with_borrowed_function());
    assert!(
        python.contains("options: TextOptions"),
        "the python page binds by value and must keep doing so; got:\n{python}"
    );
    assert!(
        !python.contains('&'),
        "no borrow marker may leak into a binding page; got:\n{python}"
    );
}

fn api_with_mutating_trait_method() -> ApiSurface {
    let mut api = make_minimal_api("1.0.0");
    let mut processor = empty_type("DocumentProcessor");
    processor.is_trait = true;
    let mut process = MethodDef {
        receiver: Some(ReceiverKind::Ref),
        ..make_method(
            "process",
            vec![
                mutably_borrowed_param("document", "Document"),
                borrowed_param("options", "TextOptions"),
            ],
            TypeRef::Unit,
            false,
            false,
            None,
        )
    };
    process.doc = "Rewrites `document` in place.".to_string();
    processor.methods = vec![process];
    api.types = vec![empty_type("Document"), empty_type("TextOptions"), processor];
    api
}

#[test]
fn rust_reference_renders_a_mutably_borrowed_trait_method_param() {
    let (rust, _) = rust_and_python_pages(&api_with_mutating_trait_method());
    assert!(
        rust.contains("document: &mut Document"),
        "an in-place trait method must document its `&mut` param; got:\n{rust}"
    );
    assert!(
        rust.contains("options: &TextOptions"),
        "a shared-borrow trait method param must stay borrowed; got:\n{rust}"
    );
    assert!(
        !rust.contains("document: Document"),
        "the owned binding shape contradicts the in-place prose; got:\n{rust}"
    );
}

#[test]
fn rust_reference_example_passes_borrowed_params_borrowed() {
    let (rust, _) = rust_and_python_pages(&api_with_mutating_trait_method());
    assert!(
        rust.contains("instance.process(&mut Document::default(), &TextOptions::default());"),
        "the example must compile against the signature printed above it; got:\n{rust}"
    );

    let (rust, python) = rust_and_python_pages(&api_with_borrowed_function());
    assert!(
        rust.contains(r#"convert("value", &TextOptions::default())"#),
        "a free function's example must borrow what its signature borrows; got:\n{rust}"
    );
    assert!(
        python.contains(r#"convert("value", TextOptions())"#),
        "the python example must keep passing by value; got:\n{python}"
    );
}

#[test]
fn rust_reference_renders_a_mutable_self_receiver() {
    let mut api = make_minimal_api("1.0.0");
    let mut session = empty_type("Session");
    session.is_opaque = true;
    session.methods = vec![
        MethodDef {
            receiver: Some(ReceiverKind::RefMut),
            ..make_method("reset", vec![], TypeRef::Unit, false, false, None)
        },
        MethodDef {
            receiver: Some(ReceiverKind::Owned),
            ..make_method("finish", vec![], TypeRef::String, false, false, None)
        },
    ];
    api.types = vec![session];

    let (rust, _) = rust_and_python_pages(&api);
    assert!(
        rust.contains("pub fn reset(&mut self)"),
        "a `&mut self` method must not be documented as `&self`; got:\n{rust}"
    );
    assert!(
        rust.contains("pub fn finish(self) -> String"),
        "a by-value receiver must not be documented as `&self`; got:\n{rust}"
    );
}

fn api_with_rust_only_field() -> ApiSurface {
    let mut api = make_minimal_api("1.0.0");
    let mut config_type = empty_type("TextOptions");
    let mut visible = make_field("width", TypeRef::Primitive(PrimitiveType::U32), false, None);
    visible.doc = "Wrap width.".to_string();
    // `PoolSettings` is not part of the binding surface, so the extract pipeline's
    // `sanitize_unknown_types` pass rewrites the field's leaf type to `String`. ~keep
    let mut rust_only = make_field("pool", TypeRef::String, true, None);
    rust_only.doc = "Rust-only pool settings.".to_string();
    rust_only.sanitized = true;
    rust_only.original_type = Some("PoolSettings".to_string());
    rust_only.binding_excluded = true;
    rust_only.binding_exclusion_reason = Some("alef(skip)".to_string());
    config_type.fields = vec![visible, rust_only];
    api.types = vec![config_type];
    api
}

#[test]
fn rust_reference_renders_a_rust_only_field_with_its_source_type() {
    let (rust, _) = rust_and_python_pages(&api_with_rust_only_field());
    assert!(
        rust.contains("Option<PoolSettings>"),
        "a sanitized-away field type must be restored on the rust page; got:\n{rust}"
    );
    assert!(
        !rust.contains("Option<String>"),
        "the sanitized placeholder type must not reach the rust page; got:\n{rust}"
    );
}

#[test]
fn binding_reference_still_omits_a_rust_only_field() {
    let (_, python) = rust_and_python_pages(&api_with_rust_only_field());
    assert!(python.contains("width"), "the bound field must stay; got:\n{python}");
    assert!(
        !python.contains("pool"),
        "an `alef(skip)` field must never reach a binding page; got:\n{python}"
    );
}

fn api_with_rust_only_enum_param() -> ApiSurface {
    let mut api = make_minimal_api("1.0.0");
    api.retain_rust_doc_type(RustDocType {
        name: "CaptionAltTextPolicy".to_string(),
        rust_path: "mylib::CaptionAltTextPolicy".to_string(),
        has_default: true,
        default_unit_variant: Some("Preserve".to_string()),
        unit_variants: vec!["Preserve".to_string(), "Replace".to_string()],
    });
    api.retain_rust_doc_type(RustDocType {
        name: "FallbackPolicy".to_string(),
        rust_path: "mylib::FallbackPolicy".to_string(),
        has_default: false,
        default_unit_variant: None,
        unit_variants: vec!["First".to_string(), "Second".to_string()],
    });
    api.retain_rust_doc_type(RustDocType {
        name: "FieldOnlyPolicy".to_string(),
        rust_path: "mylib::FieldOnlyPolicy".to_string(),
        has_default: true,
        default_unit_variant: None,
        unit_variants: Vec::new(),
    });
    api.retain_rust_doc_type(RustDocType {
        name: "RustOnlyOptions".to_string(),
        rust_path: "mylib::RustOnlyOptions".to_string(),
        has_default: true,
        default_unit_variant: None,
        unit_variants: Vec::new(),
    });
    api.retain_rust_doc_type(RustDocType {
        name: "OpaqueToken".to_string(),
        rust_path: "mylib::OpaqueToken".to_string(),
        has_default: false,
        default_unit_variant: None,
        unit_variants: Vec::new(),
    });
    api.retain_rust_doc_type(RustDocType {
        name: "QualifiedPolicy".to_string(),
        rust_path: "mylib::one::QualifiedPolicy".to_string(),
        has_default: false,
        default_unit_variant: Some("One".to_string()),
        unit_variants: vec!["One".to_string()],
    });
    api.retain_rust_doc_type(RustDocType {
        name: "QualifiedPolicy".to_string(),
        rust_path: "mylib::two::QualifiedPolicy".to_string(),
        has_default: false,
        default_unit_variant: Some("Two".to_string()),
        unit_variants: vec!["Two".to_string()],
    });

    let mut policy_param = make_param("policy", TypeRef::String, false);
    policy_param.sanitized = true;
    policy_param.original_type = Some("CaptionAltTextPolicy".to_string());

    let mut function = make_function(
        "extract_with_caption_alt_text_policy",
        vec![policy_param],
        TypeRef::Unit,
        true,
        None,
    );
    function.binding_excluded = true;

    let mut optional_policy_param = make_param("policy", TypeRef::String, true);
    optional_policy_param.sanitized = true;
    optional_policy_param.original_type = Some("CaptionAltTextPolicy".to_string());
    let mut optional_function = make_function(
        "serve_with_caption_alt_text_policy",
        vec![optional_policy_param],
        TypeRef::Unit,
        true,
        None,
    );
    optional_function.binding_excluded = true;

    let mut borrowed_policy = make_param("policy", TypeRef::String, false);
    borrowed_policy.sanitized = true;
    borrowed_policy.original_type = Some("CaptionAltTextPolicy".to_string());
    borrowed_policy.is_ref = true;

    let cases = [
        ("borrow_policy", borrowed_policy),
        ("fallback_policy", sanitized_param("policy", "FallbackPolicy", false)),
        ("field_only_policy", sanitized_param("policy", "FieldOnlyPolicy", false)),
        (
            "rust_only_options",
            sanitized_param("options", "RustOnlyOptions", false),
        ),
        ("opaque_token", sanitized_param("token", "OpaqueToken", false)),
        (
            "qualified_policy",
            sanitized_param("policy", "mylib::two::QualifiedPolicy", false),
        ),
        ("ambiguous_policy", sanitized_param("policy", "QualifiedPolicy", false)),
    ];

    api.functions = vec![function, optional_function];
    api.functions.extend(cases.into_iter().map(|(name, param)| {
        let mut function = make_function(name, vec![param], TypeRef::Unit, false, None);
        function.binding_excluded = true;
        function
    }));
    api
}

fn sanitized_param(name: &str, original_type: &str, optional: bool) -> ParamDef {
    let mut param = make_param(name, TypeRef::String, optional);
    param.sanitized = true;
    param.original_type = Some(original_type.to_string());
    param
}

#[test]
fn rust_reference_restores_a_rust_only_enum_function_param_and_example() {
    let (rust, _) = rust_and_python_pages(&api_with_rust_only_enum_param());
    assert!(
        rust.contains("pub async fn extract_with_caption_alt_text_policy(policy: CaptionAltTextPolicy)"),
        "the Rust signature must use the source enum type; got:\n{rust}"
    );
    assert!(
        rust.contains("extract_with_caption_alt_text_policy(CaptionAltTextPolicy::Preserve).await;"),
        "the Rust example must pass a real enum variant; got:\n{rust}"
    );
    assert!(
        rust.contains("pub async fn serve_with_caption_alt_text_policy(policy: Option<CaptionAltTextPolicy>)"),
        "the Rust signature must preserve optionality around the source enum; got:\n{rust}"
    );
    assert!(
        rust.contains("serve_with_caption_alt_text_policy(Some(CaptionAltTextPolicy::Preserve)).await;"),
        "the Rust example must wrap an optional enum value in Some; got:\n{rust}"
    );
    assert!(
        rust.contains("borrow_policy(&CaptionAltTextPolicy::Preserve);"),
        "a borrowed Rust-only enum sample must borrow the real variant; got:\n{rust}"
    );
    assert!(
        rust.contains("fallback_policy(FallbackPolicy::First);")
            && rust.contains("field_only_policy(FieldOnlyPolicy::default());"),
        "enum examples must prefer a default unit, then a fallback unit, then Default; got:\n{rust}"
    );
    assert!(
        rust.contains("rust_only_options(RustOnlyOptions::default());") && rust.contains("opaque_token(todo!());"),
        "non-enum source types need a valid default or typed placeholder; got:\n{rust}"
    );
    assert!(
        rust.contains("qualified_policy(QualifiedPolicy::Two);"),
        "a qualified original type must select the exact retained path before the ambiguous short name; got:\n{rust}"
    );
    assert!(
        rust.contains("ambiguous_policy(todo!());"),
        "an ambiguous short type name must not select arbitrary construction metadata; got:\n{rust}"
    );
    assert!(
        !rust.contains("policy: String") && !rust.contains("extract_with_caption_alt_text_policy(\"value\")"),
        "binding placeholders must not reach the Rust reference; got:\n{rust}"
    );
}

#[test]
fn binding_reference_still_omits_a_rust_only_enum_function() {
    let (_, python) = rust_and_python_pages(&api_with_rust_only_enum_param());
    assert!(
        !python.contains("extract_with_caption_alt_text_policy"),
        "a binding-excluded Rust function must stay absent from binding docs; got:\n{python}"
    );
}
