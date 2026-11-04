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
            private_fields: vec![FieldDef {
                name: "attributes".to_string(),
                ty: map_ty(),
                ..Default::default()
            }],
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
         [crates.python]\nmodule_name = \"test_lib\"\nstubs = {{ output = \"stubs\" }}\n\n[crates.php]\nstubs = {{ output = \"stubs\" }}\n\n[crates.ruby]\ngem_name = \"test_lib\"\nstubs = {{ output = \"sig\" }}\n"
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
fn accessor_without_a_constructor_accepting_its_value_keeps_its_method() {
    let mut typ = node_context_api().types.remove(0);
    typ.methods.retain(|method| method.name != "with_owned_attributes");
    promote_private_accessors(&mut typ);
    assert!(typ.fields.iter().all(|field| field.name != "attributes"));
    assert!(typ.methods.iter().any(|method| method.name == "attributes"));
}

#[test]
fn plain_getter_on_a_type_without_a_constructor_keeps_its_method() {
    let mut typ = node_context_api().types.remove(0);
    typ.has_lifetime_params = false;
    promote_private_accessors(&mut typ);
    assert!(
        typ.fields.iter().all(|field| field.name != "attributes"),
        "no constructor the conversion calls takes the value"
    );
    assert!(typ.methods.iter().any(|method| method.name == "attributes"));
}

#[test]
fn type_with_a_static_new_carries_the_value_through_it() {
    let mut api = node_context_api();
    let typ = &mut api.types[0];
    typ.has_lifetime_params = false;
    let constructor = typ
        .methods
        .iter_mut()
        .find(|method| method.name == "with_owned_attributes")
        .unwrap();
    constructor.name = "new".to_string();
    let content = Pyo3Backend.generate_bindings(&api, &config("python")).unwrap()[0]
        .content
        .clone();
    let to_core = impl_block(&content, "impl From<NodeContext> for test_lib::NodeContext {");
    let call = &to_core[to_core.find("Self::new(").expect("calls the static new")..];
    let position = |needle: &str| {
        call.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` is not an argument:\n{to_core}"))
    };
    assert!(
        position("val.tag_name") < position("val.attributes") && position("val.attributes") < position("val.depth"),
        "arguments follow `new(tag_name, attributes, depth)`:\n{to_core}"
    );
    assert!(!call.contains("Default::default()"), "{to_core}");
}

#[test]
fn constructor_taking_the_value_by_reference_does_not_carry_it() {
    let mut typ = node_context_api().types.remove(0);
    typ.methods[1].params[1].is_ref = true;
    promote_private_accessors(&mut typ);
    assert!(
        typ.fields.iter().all(|field| field.name != "attributes"),
        "the conversion passes an owned value, which a `&BTreeMap` parameter rejects (E0308)"
    );
    assert!(typ.methods.iter().any(|method| method.name == "attributes"));
}

#[test]
fn promotion_validates_the_constructor_the_conversion_selects() {
    let mut typ = node_context_api().types.remove(0);
    let mut wrong_type = typ.methods[1].clone();
    wrong_type.name = "from_parts".to_string();
    wrong_type.params[1].ty = TypeRef::String;
    typ.methods.insert(1, wrong_type);
    let mut selected = typ.clone();
    selected.fields.push(FieldDef {
        name: "attributes".to_string(),
        ty: map_ty(),
        ..Default::default()
    });
    assert_eq!(
        crate::codegen::conversions::select_constructor(&selected).map(|method| method.name.as_str()),
        Some("from_parts"),
        "fixture: the conversion would call the first covering constructor"
    );
    promote_private_accessors(&mut typ);
    assert!(
        typ.fields.iter().all(|field| field.name != "attributes"),
        "a later constructor with the right type must not vouch for the one the conversion calls"
    );
}

#[test]
fn borrowing_constructor_does_not_shadow_the_owning_one() {
    let mut typ = node_context_api().types.remove(0);
    let mut borrowing = typ.methods[1].clone();
    borrowing.name = "with_borrowed_attributes".to_string();
    borrowing.params[1].is_ref = true;
    typ.methods.insert(1, borrowing);
    promote_private_accessors(&mut typ);
    assert!(
        typ.fields.iter().any(|field| field.name == "attributes"),
        "the conversion prefers the non-borrowing constructor, which takes the value by value"
    );
}

#[test]
fn accessor_materializing_a_differently_stored_field_is_carried_with_its_return_type() {
    let mut typ = node_context_api().types.remove(0);
    typ.private_fields[0].ty = TypeRef::Named("AttributesSource".to_string());
    promote_private_accessors(&mut typ);
    let carried = typ.fields.iter().find(|field| field.name == "attributes").unwrap();
    assert_eq!(carried.ty, map_ty());
}

#[test]
fn optional_accessor_is_carried_as_an_optional_field() {
    let mut typ = node_context_api().types.remove(0);
    let optional_map = TypeRef::Optional(Box::new(map_ty()));
    typ.methods[0].return_type = optional_map.clone();
    typ.methods[1].params[1].ty = optional_map;
    promote_private_accessors(&mut typ);
    let carried = typ.fields.iter().find(|field| field.name == "attributes").unwrap();
    assert_eq!((&carried.ty, carried.optional), (&map_ty(), true));
}

#[test]
fn constructor_parameter_of_another_type_does_not_carry_the_value() {
    let mut typ = node_context_api().types.remove(0);
    let constructor = typ
        .methods
        .iter_mut()
        .find(|method| method.name == "with_owned_attributes")
        .unwrap();
    constructor.params[1].ty = TypeRef::String;
    promote_private_accessors(&mut typ);
    assert!(typ.fields.iter().all(|field| field.name != "attributes"));
}

#[test]
fn serde_skipped_private_field_is_not_promoted() {
    let mut typ = node_context_api().types.remove(0);
    typ.private_fields[0].serde_skip = true;
    promote_private_accessors(&mut typ);
    assert!(typ.fields.iter().all(|field| field.name != "attributes"));
}

#[test]
fn serde_renamed_private_field_keeps_its_wire_name() {
    let mut api = node_context_api();
    api.types[0].private_fields[0].serde_rename = Some("attrs".to_string());
    let mut typ = api.types[0].clone();
    promote_private_accessors(&mut typ);
    let carried = typ.fields.iter().find(|field| field.name == "attributes").unwrap();
    assert_eq!(carried.serde_rename.as_deref(), Some("attrs"));
    let go = crate::backends::go::GoBackend
        .generate_bindings(&api, &config("go"))
        .unwrap()
        .iter()
        .map(|file| file.content.clone())
        .collect::<String>();
    assert!(go.contains("`json:\"attrs\"`"), "{go}");
}

#[test]
fn carried_value_without_a_renderable_zero_gets_no_default() {
    let mut typ = node_context_api().types.remove(0);
    typ.methods[0].return_type = TypeRef::Named("Other".to_string());
    typ.methods[1].params[1].ty = TypeRef::Named("Other".to_string());
    promote_private_accessors(&mut typ);
    let carried = typ.fields.iter().find(|field| field.name == "attributes").unwrap();
    assert_eq!(carried.typed_default, None);
}

#[test]
fn accessor_that_is_not_a_plain_getter_is_left_alone() {
    type MakeUnplain = fn(&mut TypeDef);
    let not_plain_getters: [(&str, MakeUnplain); 7] = [
        ("takes an argument", |typ| {
            typ.methods[0].params.push(ParamDef::default())
        }),
        ("is async", |typ| typ.methods[0].is_async = true),
        ("can fail", |typ| typ.methods[0].error_type = Some("Error".to_string())),
        ("is sanitized", |typ| typ.methods[0].sanitized = true),
        ("returns unit", |typ| typ.methods[0].return_type = TypeRef::Unit),
        ("takes &mut self", |typ| {
            typ.methods[0].receiver = Some(ReceiverKind::RefMut)
        }),
        ("shares a public field's name", |typ| {
            typ.fields.push(FieldDef {
                name: "attributes".to_string(),
                ty: map_ty(),
                ..Default::default()
            });
        }),
    ];
    for (case, make_unplain) in not_plain_getters {
        let mut typ = node_context_api().types.remove(0);
        make_unplain(&mut typ);
        let before = typ.clone();
        promote_private_accessors(&mut typ);
        assert_eq!(
            typ.fields.len(),
            before.fields.len(),
            "an accessor that {case} was promoted"
        );
        assert_eq!(typ.methods.len(), before.methods.len(), "{case}");
    }
    let mut typ = node_context_api().types.remove(0);
    promote_private_accessors(&mut typ);
    assert!(
        typ.fields.iter().any(|field| field.name == "attributes"),
        "the unmodified fixture is promoted, so each case above fails on its own guard"
    );
}

/// A type whose private `field` of type `ty` is returned by `fn field(&self) -> <ty or &ty>`.
fn getter_type(ty: TypeRef, returns_ref: bool) -> TypeDef {
    TypeDef {
        name: "T".to_string(),
        methods: vec![MethodDef {
            name: "field".to_string(),
            return_type: ty,
            receiver: Some(ReceiverKind::Ref),
            returns_ref,
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn accessor_reads_rewrite_only_whole_field_references() {
    let typ = getter_type(map_ty(), true);
    assert_eq!(
        read_through_accessor(
            "field: val.field.iter().count(), x: val.field_len",
            "val",
            &typ,
            "field"
        ),
        "field: val.field().clone().iter().count(), x: val.field_len"
    );
    assert_eq!(
        read_through_accessor("v.field.into()", "v", &getter_type(map_ty(), false), "field"),
        "v.field().into()"
    );
}

#[test]
fn accessor_reads_own_the_value_by_the_accessor_return_type() {
    let read = |ty: TypeRef, returns_ref: bool, conversion: &str| {
        read_through_accessor(conversion, "val", &getter_type(ty, returns_ref), "field")
    };
    let optional = |ty: TypeRef| TypeRef::Optional(Box::new(ty));
    let usize_ty = TypeRef::Primitive(PrimitiveType::Usize);
    let cow = |ty: TypeRef| {
        let mut typ = getter_type(ty, false);
        typ.methods[0].returns_cow = true;
        read_through_accessor("f: val.field", "val", &typ, "field")
    };
    assert_eq!(cow(TypeRef::String), "f: val.field().into_owned()");
    assert_eq!(
        cow(optional(TypeRef::String)),
        "f: val.field().map(std::borrow::Cow::into_owned)"
    );
    assert_eq!(read(TypeRef::String, true, "f: val.field"), "f: val.field().to_owned()");
    assert_eq!(
        read(TypeRef::String, true, "f: val.field.clone()"),
        "f: val.field().to_owned()"
    );
    assert_eq!(
        read(TypeRef::Vec(Box::new(TypeRef::String)), true, "f: val.field"),
        "f: val.field().to_owned()"
    );
    assert_eq!(
        read(usize_ty.clone(), true, "f: val.field as i64"),
        "f: *val.field() as i64"
    );
    assert_eq!(
        read(usize_ty.clone(), true, "f: val.field.into()"),
        "f: (*val.field()).into()"
    );
    assert_eq!(read(usize_ty.clone(), false, "f: val.field"), "f: val.field()");
    assert_eq!(
        read(optional(TypeRef::String), true, "f: val.field"),
        "f: val.field().map(ToOwned::to_owned)"
    );
    assert_eq!(
        read(optional(usize_ty), true, "f: val.field"),
        "f: val.field().copied()"
    );
    assert_eq!(
        read(optional(TypeRef::Named("X".to_string())), true, "f: val.field"),
        "f: val.field().cloned()"
    );
}

#[test]
fn owned_string_field_before_the_carried_value_moves_after_the_accessor_read() {
    let mut api = node_context_api();
    api.types[0].fields.insert(
        0,
        FieldDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        },
    );
    api.types[0].methods[1].params.push(ParamDef {
        name: "label".to_string(),
        ty: TypeRef::String,
        ..Default::default()
    });
    let content = Pyo3Backend.generate_bindings(&api, &config("python")).unwrap()[0]
        .content
        .clone();
    let from_core = impl_block(&content, "impl From<test_lib::NodeContext<'_>> for NodeContext");
    let accessor_read = from_core
        .find("val.attributes()")
        .unwrap_or_else(|| panic!("no accessor read:\n{from_core}"));
    let first_move = regex::Regex::new(r"\bval\.(label|tag_name|depth)\b")
        .unwrap()
        .find(from_core)
        .unwrap_or_else(|| panic!("no field read:\n{from_core}"))
        .start();
    assert!(
        accessor_read < first_move,
        "the accessor borrows `val` before any field moves out of it (E0382):\n{from_core}"
    );
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
fn magnus_rbs_takes_the_carried_value_as_an_optional_keyword() {
    let rbs = crate::backends::magnus::MagnusBackend
        .generate_type_stubs(&node_context_api(), &config("ruby"))
        .unwrap()
        .iter()
        .map(|file| file.content.clone())
        .collect::<String>();
    assert!(rbs.contains("?attributes: Hash[String, String]"), "{rbs}");
    assert!(rbs.contains("attr_reader attributes: Hash[String, String]"), "{rbs}");
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
    let accessor_read = from_core.find("v.attributes()").expect("accessor read");
    let field_move = from_core.find("v.tag_name").expect("field read");
    assert!(
        accessor_read < field_move,
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
        swift.contains("serde_json::to_string(self.0.attributes())"),
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
    let map = getter_type(map_ty(), true);
    let number = getter_type(TypeRef::Primitive(PrimitiveType::Usize), true);
    assert_eq!(
        borrow_through_accessor("for (k, v) in &ctx.field {", "ctx", &map, "field"),
        "for (k, v) in ctx.field() {"
    );
    assert_eq!(
        borrow_through_accessor("x(ctx.field as f64, &ctx.field_map)", "ctx", &number, "field"),
        "x((*ctx.field()) as f64, &ctx.field_map)"
    );
    assert_eq!(
        borrow_through_accessor(
            "map(env, &ctx.field\n    .iter()\n    .collect())",
            "ctx",
            &map,
            "field"
        ),
        "map(env, &ctx.field()\n    .iter()\n    .collect())"
    );
    assert_eq!(
        borrow_through_accessor("&ctx.field", "ctx", &getter_type(map_ty(), false), "field"),
        "&ctx.field()"
    );
    assert_eq!(
        borrow_through_accessor(
            "self.0.field.clone()",
            "self.0",
            &getter_type(TypeRef::String, true),
            "field"
        ),
        "self.0.field().to_owned()"
    );
}
