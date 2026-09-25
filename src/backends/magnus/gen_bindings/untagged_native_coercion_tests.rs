//! An `#[serde(untagged)]` enum represented as a native `#[magnus::wrap]` class must still accept
//! the wire forms its own `Deserialize` accepts.
//!
//! The native representation exists to preserve variant identity across the Ruby boundary, and its
//! `from_<variant>`/`<variant>` accessors are the reason a caller can hold one at all. But a
//! discriminator-free enum's wire form *is* the payload: a unit-variant payload arrives as a bare
//! String, a record payload as a Hash, and both are what alef's own e2e generator renders from a
//! fixture (`e2e::codegen::ruby::values::json_to_ruby` maps a JSON string to a Ruby String literal
//! and a JSON object to a Ruby Hash — it has no notion of a wrapped class). A `TryConvert` that
//! accepts only an already-wrapped instance rejects every one of those callers with
//! `TypeError: no implicit conversion`, and the generated spec cannot construct the argument at
//! all.
//!
//! These run the REAL `MagnusBackend::generate_bindings` path rather than `gen_enum_with_module`
//! directly, because the conversion a field of this type actually performs is decided by the whole
//! emitted crate — the wrap attribute, the suppressed JSON `IntoValue`, and the `TryConvert` body
//! have to agree, and only the assembled `lib.rs` shows whether they do.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

const ENUM_NAME: &str = "Selector";
const MODE_PAYLOAD: &str = "SelectorMode";
const RECORD_PAYLOAD: &str = "SelectorTarget";

fn magnus_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.ruby]\ngem_name = \"test_lib\"\n",
    )
    .expect("fixture alef.toml parses");
    cfg.resolve().expect("fixture alef.toml resolves").remove(0)
}

fn named_variant(name: &str, payload: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::Named(payload.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Every variant is a single `TypeRef::Named` tuple payload and the enum is untagged with no cfg:
/// that is precisely `is_native_payload_enum`'s predicate, so this fixture — and nothing weaker —
/// reaches the native `#[magnus::wrap]` emitter under test. ~keep
fn native_payload_enum() -> EnumDef {
    EnumDef {
        name: ENUM_NAME.to_string(),
        rust_path: format!("test_lib::{ENUM_NAME}"),
        serde_untagged: true,
        has_default: true,
        variants: vec![
            named_variant("Mode", MODE_PAYLOAD),
            named_variant("Target", RECORD_PAYLOAD),
        ],
        ..Default::default()
    }
}

/// The unit enum behind the String-shaped variant. Its own wire form is a bare scalar, which is
/// why the owning untagged enum has to read a bare Ruby String.
fn mode_enum() -> EnumDef {
    EnumDef {
        name: MODE_PAYLOAD.to_string(),
        rust_path: format!("test_lib::{MODE_PAYLOAD}"),
        serde_rename_all: Some("lowercase".to_string()),
        has_default: true,
        variants: vec![
            EnumVariant {
                name: "Automatic".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Mandatory".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn record_payload_type() -> TypeDef {
    TypeDef {
        name: RECORD_PAYLOAD.to_string(),
        rust_path: format!("test_lib::{RECORD_PAYLOAD}"),
        has_default: true,
        has_serde: true,
        is_clone: true,
        fields: vec![FieldDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![native_payload_enum(), mode_enum()],
        types: vec![
            record_payload_type(),
            TypeDef {
                name: "SelectorRequest".to_string(),
                rust_path: "test_lib::SelectorRequest".to_string(),
                has_default: true,
                has_serde: true,
                is_clone: true,
                fields: vec![
                    FieldDef {
                        name: "model".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    },
                    FieldDef {
                        name: "selector".to_string(),
                        ty: TypeRef::Optional(Box::new(TypeRef::Named(ENUM_NAME.to_string()))),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn generated_lib_rs() -> String {
    let files = MagnusBackend
        .generate_bindings(&api(), &magnus_config())
        .expect("magnus bindings generate");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content
        .clone()
}

/// Returns the `impl magnus::TryConvert for <name>` block alone, so an assertion can never read a
/// sibling type's conversion body by accident — several types in the emitted crate carry a
/// `TryConvert` impl and most of them legitimately contain the serde reader. ~keep
fn try_convert_body<'a>(lib_rs: &'a str, name: &str) -> &'a str {
    let needle = format!("impl magnus::TryConvert for {name} {{");
    let start = lib_rs
        .find(&needle)
        .unwrap_or_else(|| panic!("the generated crate must convert `{name}`:\n{lib_rs}"));
    let rest = &lib_rs[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{name}`'s TryConvert impl must close:\n{rest}"));
    &rest[..end]
}

/// Guards the premise of the tests below: this fixture really does take the native
/// `#[magnus::wrap]` path and really does keep its typed accessors. If the shape predicate ever
/// stopped matching, the coercion assertions would pass for the wrong reason — an ordinary
/// JSON-backed enum has always coerced. ~keep
#[test]
fn untagged_named_payload_enum_is_emitted_as_a_native_class_with_accessors() {
    let lib_rs = generated_lib_rs();

    assert!(
        lib_rs.contains(&format!("#[magnus::wrap(class = \"TestLib::{ENUM_NAME}\")]")),
        "fixture premise broken: the enum is no longer emitted as a native wrapped class:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("pub fn from_mode(value: {MODE_PAYLOAD}) -> Self")),
        "the native class's typed factory must survive:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains(&format!("pub fn target(&self) -> Option<{RECORD_PAYLOAD}>")),
        "the native class's typed accessor must survive:\n{lib_rs}"
    );
}

/// The fix. A wrapped instance still wins, and everything else falls through to the serde reader,
/// which is what turns a bare Ruby String into the unit-payload variant and a Ruby Hash into the
/// record variant. Restoring the `<&Self as TryConvert>::try_convert(val).cloned()` one-liner as
/// the whole body fails every assertion here.
#[test]
fn native_class_conversion_falls_back_to_the_serde_reader() {
    let lib_rs = generated_lib_rs();
    let body = try_convert_body(&lib_rs, ENUM_NAME);

    let wrapped = body
        .find("<&Self as magnus::TryConvert>::try_convert(val)")
        .unwrap_or_else(|| panic!("a wrapped instance must still convert directly:\n{body}"));
    let serde = body
        .find("serde_json::from_str(&json_str)")
        .unwrap_or_else(|| panic!("a non-wrapped value must reach the serde reader:\n{body}"));
    assert!(
        wrapped < serde,
        "the wrapped instance must be tried before the serde reader, or a wrapped argument would \
         be round-tripped through `to_json`:\n{body}"
    );
    assert!(
        body.contains("<String as magnus::TryConvert>::try_convert(val)"),
        "a bare Ruby String must be read as the scalar wire form, not stringified:\n{body}"
    );
    assert!(
        body.contains("serde_json::from_str(&format!(\"\\\"{json_str}\\\"\"))"),
        "an unquoted scalar such as a unit-variant name must be requoted into JSON before the \
         untagged reader sees it:\n{body}"
    );
    assert!(
        body.contains("val.funcall::<_, _, String>(\"to_json\", ())"),
        "a Ruby Hash must reach the reader through `to_json`:\n{body}"
    );
}

/// The emitted crate must still compile. The native branch suppresses the JSON `IntoValue` impl
/// (the wrap attribute supplies one), so a fallback spliced into the wrong branch would leave the
/// body without a tail expression or duplicate the impl — both parse-visible.
#[test]
fn generated_crate_with_a_native_class_fallback_is_valid_rust() {
    let lib_rs = generated_lib_rs();

    syn::parse_file(&lib_rs).expect("the emitted magnus crate must be valid Rust");
    assert_eq!(
        lib_rs
            .matches(&format!("impl magnus::TryConvert for {ENUM_NAME} {{"))
            .count(),
        1,
        "exactly one conversion impl may be emitted for the native class:\n{lib_rs}"
    );
    assert!(
        !try_convert_body(&lib_rs, ENUM_NAME).contains("json_to_ruby"),
        "the native class keeps its wrapped egress; the fallback is ingress only:\n{lib_rs}"
    );
}
