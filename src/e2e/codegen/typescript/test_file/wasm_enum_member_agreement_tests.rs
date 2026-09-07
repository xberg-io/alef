//! Cross-surface agreement between the two generators that both name a WASM enum member.
//!
//! One surface DECLARES the member (`backends::wasm::gen_bindings::enums::gen_enum`, whose
//! `#[wasm_bindgen] pub enum` becomes the TypeScript enum the generated package ships); the other
//! REFERENCES it (`ts_builder_expression`, whose output is spliced into every generated snippet
//! under `snippets-generated/wasm/**`). A generated snippet compiles against the generated binding
//! only if the second is a subset of the first.
//!
//! Every assertion here compares the two EMITTED strings against each other. Asserting either one
//! against a hardcoded expectation is what let them drift apart in the first place: both sides
//! independently "knew" that a wire value re-cased to UpperCamelCase was the member name, and a
//! per-side expectation would have been updated in lockstep with whichever side changed. ~keep

use super::builders::ts_builder_expression;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

const WASM_TYPE_PREFIX: &str = "Wasm";
const CORE_CRATE: &str = "sample_core";
const OWNER_TYPE: &str = "RenderOptions";
const OWNER_FIELD: &str = "output_format";

fn unit_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        ..Default::default()
    }
}

fn renamed_unit_variant(name: &str, serde_rename: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        serde_rename: Some(serde_rename.to_string()),
        ..Default::default()
    }
}

fn payload_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "0".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A payload variant carrying its own `#[serde(untagged)]`, which makes the whole enum a
/// string-literal union rather than a nominal type -- so the binding declares no member at all.
fn untagged_payload_variant(name: &str) -> EnumVariant {
    EnumVariant {
        serde_untagged: true,
        ..payload_variant(name)
    }
}

fn enum_def(name: &str, variants: Vec<EnumVariant>, rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("{CORE_CRATE}::{name}"),
        variants,
        serde_rename_all: rename_all.map(str::to_string),
        ..Default::default()
    }
}

fn owner_type_def(enum_name: &str) -> TypeDef {
    TypeDef {
        name: OWNER_TYPE.to_string(),
        fields: vec![FieldDef {
            name: OWNER_FIELD.to_string(),
            ty: TypeRef::Named(enum_name.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Surface 1: the member identifiers the WASM binding actually declares.
///
/// Empty when `gen_enum` did not emit a C-style enum at all — a data-carrying enum is emitted as
/// a discriminator `pub struct` instead, which declares no members for a snippet to reference.
///
/// The `is_json_passthrough_data_enum` guard asks the same authority `mod.rs`'s enum loop asks
/// before it calls `gen_enum` at all. Without it this helper called `gen_enum` directly and
/// reported the C-style members that function still returns in isolation -- members the pipeline
/// suppresses and the package therefore never ships. That made this whole test blind to exactly
/// the drift it exists to catch: a variant-level untagged enum passed while the real generated
/// snippet referenced `WasmOutputFormat.Markdown` against a binding that declares no such
/// member. ~keep
fn declared_members(enum_def: &EnumDef) -> Vec<String> {
    if crate::backends::wasm::gen_bindings::enums::is_json_passthrough_data_enum(enum_def) {
        return Vec::new();
    }
    let source = crate::backends::wasm::gen_bindings::enums::gen_enum(
        enum_def,
        WASM_TYPE_PREFIX,
        CORE_CRATE,
        &std::collections::HashSet::new(),
    );
    let header = format!("pub enum {WASM_TYPE_PREFIX}{} {{", enum_def.name);
    let Some(start) = source.find(&header) else {
        return Vec::new();
    };
    let body = &source[start + header.len()..];
    let body = &body[..body.find("\n}").expect("wasm enum declaration must close")];
    body.lines()
        .filter_map(|line| line.trim().split_once(" = "))
        .map(|(member, _)| member.to_string())
        .collect()
}

/// Surface 2: the member identifier the generated snippet references, if any.
fn referenced_member(expression: &str, enum_name: &str) -> Option<String> {
    let needle = format!("{WASM_TYPE_PREFIX}{enum_name}.");
    let start = expression.find(&needle)? + needle.len();
    let rest = &expression[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn snippet_expression(enum_def: &EnumDef, wire_value: &str) -> String {
    ts_builder_expression(
        serde_json::json!({ "output_format": wire_value })
            .as_object()
            .expect("fixture input is an object"),
        &format!("{WASM_TYPE_PREFIX}{OWNER_TYPE}"),
        &Default::default(),
        "wasm",
        &Default::default(),
        &Default::default(),
        &[owner_type_def(&enum_def.name)],
        std::slice::from_ref(enum_def),
        WASM_TYPE_PREFIX,
        &[],
        &mut Default::default(),
    )
}

/// The generated snippet may never name a member the generated binding does not declare.
///
/// `TS2339: Property 'X' does not exist on type 'typeof WasmY'` is exactly this invariant
/// failing, and it is unreachable from either generator alone.
#[test]
fn wasm_snippet_enum_members_are_declared_by_the_wasm_binding() {
    // (case, enum under test, the wire value a fixture supplies)
    let cases: Vec<(&str, EnumDef, &str)> = vec![
        (
            "single-word unit variant",
            enum_def(
                "OutputFormat",
                vec![unit_variant("Markdown"), unit_variant("Html")],
                None,
            ),
            "markdown",
        ),
        (
            "multi-word unit variant under snake_case rename_all",
            enum_def(
                "OutputFormat",
                vec![unit_variant("Markdown"), unit_variant("PlainText")],
                Some("snake_case"),
            ),
            "plain_text",
        ),
        (
            "multi-word unit variant under lowercase rename_all",
            enum_def(
                "OutputFormat",
                vec![unit_variant("Markdown"), unit_variant("PlainText")],
                Some("lowercase"),
            ),
            "plaintext",
        ),
        (
            "unit variant carrying an explicit serde rename",
            enum_def(
                "OutputFormat",
                vec![renamed_unit_variant("Markdown", "md"), unit_variant("Html")],
                None,
            ),
            "md",
        ),
        (
            "externally tagged enum carrying a payload variant",
            enum_def(
                "OutputFormat",
                vec![unit_variant("Markdown"), payload_variant("Custom")],
                None,
            ),
            "markdown",
        ),
        (
            "variant-level untagged payload variant",
            enum_def(
                "OutputFormat",
                vec![unit_variant("Markdown"), untagged_payload_variant("Custom")],
                None,
            ),
            "markdown",
        ),
    ];

    for (case, enum_under_test, wire_value) in cases {
        let declared = declared_members(&enum_under_test);
        let expression = snippet_expression(&enum_under_test, wire_value);
        let referenced = referenced_member(&expression, &enum_under_test.name);

        match referenced {
            Some(member) => assert!(
                declared.contains(&member),
                "{case}: the generated snippet references `{WASM_TYPE_PREFIX}{}.{member}`, but the \
                 generated WASM binding declares {declared:?}.\nsnippet expression:\n{expression}",
                enum_under_test.name,
            ),
            None => assert!(
                declared.is_empty(),
                "{case}: the generated WASM binding declares {declared:?}, but the generated \
                 snippet references no member of `{WASM_TYPE_PREFIX}{}` at all.\nsnippet \
                 expression:\n{expression}",
                enum_under_test.name,
            ),
        }
    }
}
