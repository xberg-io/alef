//! A public reply dataclass must have the instance methods of the native class it shadows.
//!
//! `options.py` publishes a reply type as a frozen `@dataclass` with the fields of the native
//! `#[pyclass]`. The `.pyi` stub, the native class and every other language binding also have the
//! instance methods (`text()`, `output_images()`), so a caller who parsed a reply and called
//! `message.text()` got an `AttributeError`. The dataclass now forwards each qualifying method
//! through the `_to_rust_*` converter of `api.py`.

use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

fn python_config(dto_section: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
{dto_section}
"#
    ))
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

fn typed_dict_config() -> ResolvedCrateConfig {
    python_config("\n[crates.dto]\npython = \"typed-dict\"\n")
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn instance_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    }
}

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        ..Default::default()
    }
}

/// A reply message with the methods of a chat reply, one method of each kind
/// the generator must leave out, and a reply type (`ImageUrl`) that a method returns.
fn reply_surface() -> ApiSurface {
    let named = |name: &str| TypeRef::Named(name.to_string());
    let mut set_name = instance_method("set_name", vec![param("name", TypeRef::String, false)], TypeRef::Unit);
    set_name.receiver = Some(ReceiverKind::RefMut);
    let mut from_text = instance_method(
        "from_text",
        vec![param("value", TypeRef::String, false)],
        named("Message"),
    );
    from_text.receiver = None;
    from_text.is_static = true;
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ImageUrl".to_string(),
                rust_path: "test_lib::ImageUrl".to_string(),
                has_default: true,
                is_return_type: true,
                fields: vec![field("url", TypeRef::String)],
                ..Default::default()
            },
            TypeDef {
                name: "Usage".to_string(),
                rust_path: "test_lib::Usage".to_string(),
                is_return_type: true,
                fields: vec![field("tokens", TypeRef::Primitive(PrimitiveType::U32))],
                ..Default::default()
            },
            TypeDef {
                name: "Message".to_string(),
                rust_path: "test_lib::Message".to_string(),
                has_default: true,
                is_return_type: true,
                fields: vec![
                    field("name", TypeRef::Optional(Box::new(TypeRef::String))),
                    field("text", TypeRef::String),
                ],
                methods: vec![
                    instance_method("refusal_text", vec![], TypeRef::Optional(Box::new(TypeRef::String))),
                    instance_method("output_images", vec![], TypeRef::Vec(Box::new(named("ImageUrl")))),
                    instance_method(
                        "describe",
                        vec![
                            param("suffix", TypeRef::Optional(Box::new(TypeRef::String)), true),
                            param("prefix", TypeRef::String, false),
                        ],
                        TypeRef::String,
                    ),
                    instance_method("usage", vec![], named("Usage")),
                    instance_method("validate", vec![], TypeRef::Unit),
                    // Each of these needs a conversion the generator does not have, or it
                    // would shadow a field, so the dataclass must not declare it.
                    instance_method("text", vec![], TypeRef::String),
                    instance_method(
                        "with_image",
                        vec![param("image", named("ImageUrl"), false)],
                        TypeRef::Unit,
                    ),
                    instance_method("kind", vec![], named("Kind")),
                    instance_method("from_json", vec![], TypeRef::String),
                    set_name,
                    from_text,
                ],
                ..Default::default()
            },
        ],
        enums: vec![EnumDef {
            name: "Kind".to_string(),
            variants: vec![EnumVariant {
                name: "Plain".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn render(api: &ApiSurface, config: &ResolvedCrateConfig) -> (String, String) {
    let files = crate::backends::pyo3::Pyo3Backend
        .generate_public_api(api, config)
        .expect("public API generation succeeds");
    let find = |suffix: &str| {
        files
            .iter()
            .find(|file| file.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} is generated"))
            .content
            .clone()
    };
    (find("options.py"), find("api.py"))
}

/// The class body of `name` in `options.py`, up to the next top-level statement.
fn class_body<'a>(options_py: &'a str, name: &str) -> &'a str {
    let start = options_py
        .find(&format!("class {name}:"))
        .unwrap_or_else(|| panic!("options.py defines {name}:\n{options_py}"));
    let rest = &options_py[start..];
    let end = rest[1..]
        .find("\n@dataclass")
        .or_else(|| rest[1..].find("\ndef "))
        .map_or(rest.len(), |offset| offset + 1);
    &rest[..end]
}

#[test]
fn should_forward_instance_methods_through_the_api_converter() {
    let (options_py, api_py) = render(&reply_surface(), &typed_dict_config());
    let message = class_body(&options_py, "Message");

    for expected in [
        "    def refusal_text(self) -> str | None:",
        "        from .api import _to_rust_message  # noqa: PLC0415",
        "        return _to_rust_message(self).refusal_text()",
        "    def validate(self) -> None:",
        "        _to_rust_message(self).validate()",
        "    def usage(self) -> Usage:",
        "        return _to_rust_message(self).usage()",
    ] {
        assert!(
            message.contains(expected),
            "missing `{expected}` in the Message dataclass:\n{message}"
        );
    }
    assert!(
        api_py.contains("def _to_rust_message("),
        "the forwarded methods call `_to_rust_message`, so api.py must define it:\n{api_py}"
    );
}

#[test]
fn should_convert_reply_types_that_a_forwarded_method_returns() {
    let (options_py, _) = render(&reply_surface(), &typed_dict_config());
    let message = class_body(&options_py, "Message");

    assert!(
        message.contains("    def output_images(self) -> list[ImageUrl]:"),
        "{message}"
    );
    assert!(
        message.contains("        result = _to_rust_message(self).output_images()"),
        "{message}"
    );
    assert!(
        message.contains("        return [_from_native_image_url(__v) for __v in result]"),
        "a published reply type must come back as the dataclass, not the native object:\n{message}"
    );
}

#[test]
fn should_name_native_only_result_types_for_the_type_checker() {
    let (options_py, _) = render(&reply_surface(), &typed_dict_config());

    let imports = options_py
        .split_once("if TYPE_CHECKING:")
        .map(|(_, rest)| rest.split("\n\n").next().unwrap_or_default())
        .expect("a native-only result type needs a TYPE_CHECKING import");
    assert!(
        imports.contains("Usage"),
        "`usage()` is annotated with the native class, so options.py must import it:\n{options_py}"
    );
}

#[test]
fn should_keep_the_parameter_order_and_defaults_of_the_native_stub() {
    let (options_py, _) = render(&reply_surface(), &typed_dict_config());
    let message = class_body(&options_py, "Message");

    assert!(
        message.contains("    def describe(self, prefix: str, suffix: str | None = None) -> str:"),
        "required parameters come first, as in the stub:\n{message}"
    );
    assert!(
        message.contains("_to_rust_message(self).describe(suffix=suffix, prefix=prefix)"),
        "the native call passes every parameter by name:\n{message}"
    );
}

#[test]
fn should_not_forward_methods_that_need_a_conversion_the_generator_lacks() {
    let (options_py, _) = render(&reply_surface(), &typed_dict_config());
    let message = class_body(&options_py, "Message");

    for absent in [
        "def set_name(",
        "def from_text(",
        "def with_image(",
        "def kind(",
        "def text(",
    ] {
        assert!(
            !message.contains(absent),
            "`{absent}` must not be forwarded:\n{message}"
        );
    }
    assert_eq!(
        message.matches("def from_json(").count(),
        0,
        "a method named from_json is the staticmethod's job, never a forwarded instance method:\n{message}"
    );
}

#[test]
fn should_not_forward_methods_of_a_type_the_package_keeps_native() {
    let (options_py, api_py) = render(&reply_surface(), &python_config(""));

    assert!(
        !options_py.contains("def refusal_text("),
        "with the default output style a reply type stays the native class:\n{options_py}"
    );
    assert!(!api_py.contains("def _to_rust_message("), "{api_py}");
}

#[test]
fn should_list_the_forwarded_methods_for_the_generated_end_to_end_suite() {
    let api = reply_surface();
    let config = typed_dict_config();
    let forwarded = crate::backends::pyo3::gen_bindings::forwarded_method_names(&api, &config.dto, &[]);

    assert_eq!(
        forwarded,
        vec![(
            "Message".to_string(),
            ["describe", "output_images", "refusal_text", "usage", "validate"]
                .map(String::from)
                .to_vec()
        )]
    );
}
