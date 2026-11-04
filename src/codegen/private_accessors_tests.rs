//! A public `&self` accessor over a private core field (html-to-markdown's
//! `NodeContext::attributes()`, xberg-io/alef#865) must carry its value through every mirroring
//! binding instead of reading it from a core value rebuilt with an empty default.

use super::*;
use crate::backends::napi::NapiBackend;
use crate::backends::php::PhpBackend;
use crate::backends::pyo3::Pyo3Backend;
use crate::core::backend::Backend;
use crate::core::ir::{CoreWrapper, ParamDef, PrimitiveType};

fn map_ty() -> TypeRef {
    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String))
}

fn node_context_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "NodeContext".to_string(),
            rust_path: "test_lib::NodeContext".to_string(),
            has_serde: true,
            is_clone: true,
            has_lifetime_params: true,
            has_private_fields: true,
            private_fields: vec!["attributes".to_string()],
            fields: vec![
                FieldDef {
                    name: "tag_name".to_string(),
                    ty: TypeRef::String,
                    core_wrapper: CoreWrapper::Cow,
                    ..Default::default()
                },
                FieldDef {
                    name: "depth".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::Usize),
                    ..Default::default()
                },
            ],
            methods: vec![
                MethodDef {
                    name: "attributes".to_string(),
                    return_type: map_ty(),
                    receiver: Some(ReceiverKind::Ref),
                    returns_ref: true,
                    doc: "Return a reference to the attribute map.".to_string(),
                    ..Default::default()
                },
                MethodDef {
                    name: "with_owned_attributes".to_string(),
                    params: vec![
                        ParamDef {
                            name: "tag_name".to_string(),
                            ty: TypeRef::String,
                            ..Default::default()
                        },
                        ParamDef {
                            name: "attributes".to_string(),
                            ty: map_ty(),
                            ..Default::default()
                        },
                        ParamDef {
                            name: "depth".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::Usize),
                            ..Default::default()
                        },
                    ],
                    return_type: TypeRef::Named("NodeContext".to_string()),
                    is_static: true,
                    receiver: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn config(language: &str) -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(&format!(
        "[workspace]\nlanguages = [\"{language}\"]\n\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\n\
         [crates.python]\nmodule_name = \"test_lib\"\nstubs = {{ output = \"stubs\" }}\n\n[crates.php]\nstubs = {{ output = \"stubs\" }}\n"
    ))
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn impl_block<'a>(content: &'a str, header: &str) -> &'a str {
    let start = content
        .find(header)
        .unwrap_or_else(|| panic!("missing `{header}`:\n{content}"));
    let rest = &content[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |end| end + 3);
    &rest[..end]
}

#[test]
fn pyo3_carries_the_accessor_value_and_keeps_the_method() {
    let files = Pyo3Backend
        .generate_bindings(&node_context_api(), &config("python"))
        .unwrap();
    let content = &files[0].content;
    syn::parse_file(content).unwrap_or_else(|error| panic!("{error}:\n{content}"));

    let to_core = impl_block(content, "impl From<NodeContext> for test_lib::NodeContext<'_>");
    assert!(
        !to_core.contains("BTreeMap::new()"),
        "the accessor's data must not be replaced by an empty default:\n{to_core}"
    );
    assert!(to_core.contains("val.attributes"), "{to_core}");

    let from_core = impl_block(content, "impl From<test_lib::NodeContext<'_>> for NodeContext");
    assert!(
        from_core.contains("val.attributes()"),
        "the binding must read the value through the public accessor:\n{from_core}"
    );

    assert!(
        content.contains("pub fn attributes(&self)"),
        "the accessor stays a Python method:\n{content}"
    );
    assert!(
        content.contains("    pub attributes: HashMap<String, String>,")
            && !content.contains("#[pyo3(get)]\n    pub attributes"),
        "the binding carries the value without a getter that would shadow the method:\n{content}"
    );
    assert!(
        content.contains("attributes: Default::default()"),
        "the Python constructor keeps its signature and starts the carried value empty:\n{content}"
    );
}

#[test]
fn napi_exposes_the_accessor_value_as_an_object_property() {
    let files = NapiBackend
        .generate_bindings(&node_context_api(), &config("node"))
        .unwrap();
    let content = &files[0].content;
    syn::parse_file(content).unwrap_or_else(|error| panic!("{error}:\n{content}"));
    assert!(content.contains("pub attributes: HashMap<String, String>"), "{content}");
    let from_core = impl_block(content, "impl From<test_lib::NodeContext<'_>> for JsNodeContext");
    assert!(from_core.contains("val.attributes()"), "{from_core}");
    assert!(!content.contains("BTreeMap::new()"), "{content}");
}

#[test]
fn php_serde_constructor_defaults_the_carried_value() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("test-lib-php/src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        directory.path().join("test-lib-php/Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .unwrap();
    let mut php = config("php");
    php.output_paths.insert("php".to_string(), source);
    let content = &PhpBackend.generate_bindings(&node_context_api(), &php).unwrap()[0].content;
    assert!(
        content.contains("pub fn from_json(json: String)"),
        "fixture takes the serde branch:\n{content}"
    );
    let constructor = impl_block(content, "    #[php(constructor)]");
    assert!(
        constructor.contains("attributes: Option<HashMap<String, String>>")
            && constructor.contains("attributes: attributes.unwrap_or_default()"),
        "{constructor}"
    );
}

#[test]
fn php_carries_the_accessor_value_into_the_method_round_trip() {
    let files = PhpBackend
        .generate_bindings(&node_context_api(), &config("php"))
        .unwrap();
    let content = &files[0].content;
    syn::parse_file(content).unwrap_or_else(|error| panic!("{error}:\n{content}"));
    assert!(!content.contains("BTreeMap::new()"), "{content}");
    assert!(content.contains("val.attributes()"), "{content}");
    assert!(content.contains("pub fn attributes(&self)"), "{content}");
    assert!(
        content.contains("attributes: Option<HashMap<String, String>>")
            && content.contains("attributes: attributes.unwrap_or_default()"),
        "the PHP constructor takes the carried value as an optional parameter:\n{content}"
    );
}

#[test]
fn accessor_without_a_constructor_accepting_its_value_is_dropped() {
    let mut typ = node_context_api().types.remove(0);
    typ.methods.retain(|method| method.name != "with_owned_attributes");
    promote_private_accessors(&mut typ);
    assert!(typ.fields.iter().all(|field| field.name != "attributes"));
    assert!(typ.methods.iter().all(|method| method.name != "attributes"));
}

#[test]
fn accessor_with_arguments_or_over_a_public_name_is_left_alone() {
    let mut typ = node_context_api().types.remove(0);
    typ.private_fields.clear();
    let before = typ.clone();
    promote_private_accessors(&mut typ);
    assert_eq!(typ.fields.len(), before.fields.len());
    assert_eq!(typ.methods.len(), before.methods.len());
}

#[test]
fn accessor_reads_rewrite_only_whole_field_references() {
    assert_eq!(
        read_through_accessor(
            "attributes: val.attributes.iter().count(), x: val.attributes_len",
            "val",
            "attributes",
            true
        ),
        "attributes: val.attributes().clone().iter().count(), x: val.attributes_len"
    );
    assert_eq!(read_through_accessor("v.a.into()", "v", "a", false), "v.a().into()");
}

#[test]
fn type_stubs_match_the_generated_bindings() {
    let python = Pyo3Backend
        .generate_type_stubs(&node_context_api(), &config("python"))
        .unwrap();
    let pyi = python.iter().map(|file| file.content.as_str()).collect::<String>();
    assert!(pyi.contains("def attributes(self)"), "{pyi}");
    assert!(
        pyi.contains("    depth: int\n    def __init__(self, tag_name: str, depth: int) -> None: ..."),
        "the carried value is neither a stub attribute nor a constructor parameter:\n{pyi}"
    );

    let node = NapiBackend
        .generate_type_stubs(&node_context_api(), &config("node"))
        .unwrap();
    let dts = node.iter().map(|file| file.content.as_str()).collect::<String>();
    assert!(dts.contains("attributes"), "{dts}");

    let php = PhpBackend
        .generate_type_stubs(&node_context_api(), &config("php"))
        .unwrap();
    let stub = php.iter().map(|file| file.content.as_str()).collect::<String>();
    assert!(stub.contains("public readonly ?array $attributes = null"), "{stub}");
}

fn generated(backend: &dyn Backend, language: &str) -> String {
    let mut files = backend
        .generate_bindings(&node_context_api(), &config(language))
        .unwrap();
    files.extend(
        backend
            .generate_public_api(&node_context_api(), &config(language))
            .unwrap(),
    );
    files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_rust_round_trip(content: &str, from_core_header: &str, read: &str) {
    assert!(
        !content.contains("std::collections::BTreeMap::new(),"),
        "the accessor's data must not be replaced by an empty default:\n{content}"
    );
    let from_core = impl_block(content, from_core_header);
    assert!(
        from_core.contains(read),
        "the binding must read through the accessor:\n{from_core}"
    );
}

#[test]
fn wasm_carries_the_accessor_value_behind_the_method() {
    let content = generated(&crate::backends::wasm::WasmBackend, "wasm");
    assert_rust_round_trip(
        &content,
        "impl From<test_lib::NodeContext<'_>> for WasmNodeContext",
        "val.attributes()",
    );
    assert!(
        content.contains("serde_wasm_bindgen::from_value(val.attributes.clone())"),
        "the carried JsValue decodes back into the core constructor:\n{content}"
    );
    assert!(content.contains("pub fn attributes(&self) -> JsValue"), "{content}");
    assert!(
        !content.contains("#[wasm_bindgen(getter)]\n    pub fn attributes"),
        "{content}"
    );
    assert!(
        content.contains("pub fn new(tagName: String, depth: usize)")
            && content.contains("attributes: Default::default()"),
        "the JS constructor keeps its signature:\n{content}"
    );
}

#[test]
fn magnus_carries_the_accessor_value_as_the_attributes_method() {
    let content = generated(&crate::backends::magnus::MagnusBackend, "ruby");
    assert_rust_round_trip(
        &content,
        "impl From<test_lib::NodeContext<'_>> for NodeContext",
        "val.attributes()",
    );
    assert_eq!(
        content.matches("define_method(\"attributes\"").count(),
        1,
        "Ruby keeps exactly one `attributes` method:\n{content}"
    );
}

#[test]
fn rustler_carries_the_accessor_value_in_the_struct() {
    let content = generated(&crate::backends::rustler::RustlerBackend, "elixir");
    assert_rust_round_trip(
        &content,
        "impl From<test_lib::NodeContext<'_>> for NodeContext",
        "val.attributes()",
    );
    assert!(content.contains("pub fn nodecontext_attributes("), "{content}");
    assert!(
        content.contains("attributes: %{}"),
        "the Elixir struct declares the key:\n{content}"
    );
}

#[test]
fn extendr_carries_the_accessor_value_in_the_struct() {
    let content = generated(&crate::backends::extendr::ExtendrBackend, "r");
    assert_rust_round_trip(
        &content,
        "impl From<test_lib::NodeContext<'_>> for NodeContext",
        "val.attributes()",
    );
    assert!(content.contains("pub attributes: HashMap<String, String>"), "{content}");
}

#[test]
fn dart_mirror_reads_the_accessor_value() {
    let content = generated(&crate::backends::dart::DartBackend, "dart");
    assert_rust_round_trip(
        &content,
        "impl From<test_lib::NodeContext<'_>> for NodeContext",
        "v.attributes()",
    );
    let from_core = impl_block(&content, "impl From<test_lib::NodeContext<'_>> for NodeContext");
    assert!(
        from_core.find("v.attributes()") < from_core.find("v.tag_name"),
        "the accessor borrows `v` before any field moves out of it:\n{from_core}"
    );
    assert!(
        content.contains("pub attributes: std::collections::HashMap<String, String>"),
        "{content}"
    );
}

#[test]
fn json_dto_backends_carry_the_accessor_value_on_the_wire() {
    let go = generated(&crate::backends::go::GoBackend, "go");
    assert!(
        go.contains("Attributes map[string]string `json:\"attributes\"`"),
        "{go}"
    );

    let kotlin = generated(&crate::backends::kotlin_android::KotlinAndroidBackend, "kotlin_android");
    assert!(
        kotlin.contains("val attributes: Map<String, String> = emptyMap(),"),
        "{kotlin}"
    );

    let swift = generated(&crate::backends::swift::SwiftBackend, "swift");
    assert!(
        swift.contains("serde_json::to_string(&self.0.attributes())"),
        "the bridge getter reads through the accessor:\n{swift}"
    );
    assert!(swift.contains("attributes: [String: String] = [:]"), "{swift}");
}

#[test]
fn visitor_callback_contexts_read_the_accessor_in_place() {
    use crate::codegen::visitor_context::{VisitorContextBackend, visitor_context_helper};
    let api = node_context_api().with_private_accessor_fields();
    let bridge = crate::core::config::TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        context_type: Some("NodeContext".to_string()),
        ..crate::core::config::TraitBridgeConfig::default()
    };
    let direct_read = regex::Regex::new(r"ctx\.attributes\b[^(]").unwrap();
    for backend in [
        VisitorContextBackend::Napi,
        VisitorContextBackend::Wasm,
        VisitorContextBackend::Pyo3,
        VisitorContextBackend::Magnus,
        VisitorContextBackend::Extendr,
        VisitorContextBackend::Rustler,
    ] {
        let lines = visitor_context_helper(&api, &bridge, "test_lib", backend)
            .unwrap()
            .field_lines;
        assert!(lines.contains("ctx.attributes()"), "{lines}");
        assert!(
            !lines.contains("&(*ctx.attributes())"),
            "clippy::borrow_deref_ref:\n{lines}"
        );
        assert!(
            !direct_read.is_match(&lines),
            "every read goes through the accessor:\n{lines}"
        );
    }
}

#[test]
fn borrowing_reads_keep_their_meaning_through_a_reference_accessor() {
    assert_eq!(
        borrow_through_accessor("for (k, v) in &ctx.attributes {", "ctx", "attributes", true),
        "for (k, v) in ctx.attributes() {"
    );
    assert_eq!(
        borrow_through_accessor("x(ctx.depth as f64, &ctx.depth_map)", "ctx", "depth", true),
        "x((*ctx.depth()) as f64, &ctx.depth_map)"
    );
    assert_eq!(
        borrow_through_accessor(
            "map(env, &ctx.attributes\n    .iter()\n    .collect())",
            "ctx",
            "attributes",
            true
        ),
        "map(env, &ctx.attributes()\n    .iter()\n    .collect())"
    );
    assert_eq!(borrow_through_accessor("&ctx.a", "ctx", "a", false), "&ctx.a()");
}
