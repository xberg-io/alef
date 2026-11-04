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
    std::fs::write("/private/tmp/claude-501/-Users-naamanhirschfeld-workspace-xberg-io/5d69f932-4d49-440f-8720-636c64e3adba/scratchpad/napi.rs", content).unwrap();
    syn::parse_file(content).unwrap_or_else(|error| panic!("{error}:\n{content}"));
    assert!(content.contains("pub attributes: HashMap<String, String>"), "{content}");
    let from_core = impl_block(content, "impl From<test_lib::NodeContext<'_>> for JsNodeContext");
    assert!(from_core.contains("val.attributes()"), "{from_core}");
    assert!(!content.contains("BTreeMap::new()"), "{content}");
}

#[test]
fn php_carries_the_accessor_value_into_the_method_round_trip() {
    let files = PhpBackend
        .generate_bindings(&node_context_api(), &config("php"))
        .unwrap();
    let content = &files[0].content;
    std::fs::write("/private/tmp/claude-501/-Users-naamanhirschfeld-workspace-xberg-io/5d69f932-4d49-440f-8720-636c64e3adba/scratchpad/php.rs", content).unwrap();
    syn::parse_file(content).unwrap_or_else(|error| panic!("{error}:\n{content}"));
    assert!(!content.contains("BTreeMap::new()"), "{content}");
    assert!(content.contains("val.attributes()"), "{content}");
    assert!(content.contains("pub fn attributes(&self)"), "{content}");
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
            "attributes",
            true
        ),
        "attributes: val.attributes().clone().iter().count(), x: val.attributes_len"
    );
    assert_eq!(read_through_accessor("a: val.a", "a", false), "a: val.a()");
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
    std::fs::write("/private/tmp/claude-501/-Users-naamanhirschfeld-workspace-xberg-io/5d69f932-4d49-440f-8720-636c64e3adba/scratchpad/php.stub", &stub).unwrap();
    std::fs::write("/private/tmp/claude-501/-Users-naamanhirschfeld-workspace-xberg-io/5d69f932-4d49-440f-8720-636c64e3adba/scratchpad/node.dts", &dts).unwrap();
    std::fs::write("/private/tmp/claude-501/-Users-naamanhirschfeld-workspace-xberg-io/5d69f932-4d49-440f-8720-636c64e3adba/scratchpad/py.pyi", &pyi).unwrap();
    assert!(stub.contains("attributes"), "{stub}");
}
