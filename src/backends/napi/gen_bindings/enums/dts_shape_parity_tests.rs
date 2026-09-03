//! Cross-generator parity between the compiled `#[napi]` runtime shape ([`gen_enum`], via
//! [`super::gen_tagged_enum_as_object`]) and the declared `.d.ts` shape (`errors::gen_dts`) for
//! the SAME `EnumDef`.
//!
//! An enum with no explicit `#[serde(tag/content/untagged)]` but with a data-carrying variant is
//! the exact shape that regressed: `enums::gen_enum` (the single authority, see
//! [`super::is_tagged_data_enum`]) has always routed it to the tagged-OBJECT emitter, because a
//! `#[napi(string_enum)]` cannot hold a payload. `errors::gen_dts`'s `Decl::Enum` dispatch used to
//! re-derive the same routing decision locally as `e.serde_tag.is_some()`, which is strictly
//! narrower -- it never covers this shape -- so the `.d.ts` declared a plain string enum for a
//! type the compiled extension actually returns as `{ type: "...", ... }`. `tsc` cannot catch
//! this: the declaration and a snippet checked against it agree with each other and both disagree
//! with the runtime value. Only running the generated test fails, and it fails the same way every
//! time (`String({type:"Function",...}) === "[object Object]"`).
//!
//! These tests call the two real production entry points -- [`gen_enum`] and
//! `errors::gen_dts` -- on one shared `EnumDef` fixture and assert their exact rendered output,
//! so a future re-introduction of a second, narrower shape derivation on either side fails here
//! instead of shipping.

use super::gen_enum;
use crate::backends::napi::gen_bindings::errors::gen_dts;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

/// A default-representation (no `serde_tag`, no `serde_content`, not `serde_untagged`) enum with
/// one data-carrying tuple variant and one unit variant -- the shape that must route through the
/// tagged-object emitter on both the runtime and `.d.ts` sides, even without an explicit
/// `#[serde(tag = "...")]`.
fn sample_kind_enum() -> EnumDef {
    EnumDef {
        name: "SampleKind".to_string(),
        rust_path: "test_core::SampleKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "Function".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Idle".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// The compiled `#[napi]` runtime shape for [`sample_kind_enum`] is a tagged OBJECT, not a
/// `#[napi(string_enum)]` -- pins the runtime half of the parity this module protects.
#[test]
fn runtime_struct_is_tagged_object_for_default_tagged_data_enum() {
    let enum_def = sample_kind_enum();
    let runtime = gen_enum(&enum_def, "Js", false, "test_core", None);

    let expected = "\
#[derive(Clone)]
#[napi(object, js_name = \"SampleKind\")]
pub struct JsSampleKind {
    #[napi(js_name = \"type\")]
    pub type_tag: String,
    pub function: Option<String>,
}

impl Default for JsSampleKind {
    fn default() -> Self { Self { type_tag: \"Function\".to_string(), function: None } }
}";
    assert_eq!(runtime, expected);
}

/// The generated `.d.ts` declaration for [`sample_kind_enum`] is a discriminated union of objects
/// -- the TypeScript description of the exact struct [`runtime_struct_is_tagged_object_for_default_tagged_data_enum`]
/// pins -- never a plain `export declare enum`. This is the regression this module exists to
/// catch: before the fix, `errors::gen_dts` re-derived its own narrower "is this a tagged data
/// enum" check (`e.serde_tag.is_some()`) instead of asking the same authority `gen_enum` uses, so
/// this exact fixture declared `export declare enum SampleKind { Function = "Function", Idle =
/// "Idle" }` -- a string enum -- while the runtime returned an object. `find(...).expect(...)`
/// fails loudly if that regresses, rather than silently comparing against the wrong block.
#[test]
fn dts_declaration_is_discriminated_union_for_default_tagged_data_enum() {
    let enum_def = sample_kind_enum();
    let api = ApiSurface {
        enums: vec![enum_def],
        ..Default::default()
    };
    let dts = gen_dts(
        &api,
        "Js",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );

    let start = dts
        .find("export type SampleKind =")
        .expect("gen_dts must declare SampleKind as a discriminated union type, not a plain enum");
    let declaration = dts[start..].trim_end();

    assert_eq!(
        declaration,
        "export type SampleKind =\n  | { type: 'Function'; function: string }\n  | { type: 'Idle' }"
    );
}

/// The runtime struct's discriminant `js_name` and payload field name must literally match the
/// keys the `.d.ts` union declares for the same fixture -- computed from each side's real output
/// (not two independently hand-typed literals), so this fails if either generator's naming ever
/// drifts from the other even when both still agree on "object, not enum".
#[test]
fn dts_and_runtime_agree_on_discriminant_and_payload_field_names() {
    let enum_def = sample_kind_enum();

    let runtime = gen_enum(&enum_def, "Js", false, "test_core", None);
    let runtime_tag_line = runtime
        .lines()
        .find(|l| l.trim_start().starts_with("#[napi(js_name ="))
        .expect("runtime struct must declare a js_name for its discriminant field");
    assert_eq!(runtime_tag_line.trim(), "#[napi(js_name = \"type\")]");
    let runtime_payload_line = runtime
        .lines()
        .find(|l| l.trim_start().starts_with("pub function:"))
        .expect("runtime struct must declare the Function variant's payload field");
    assert_eq!(runtime_payload_line.trim(), "pub function: Option<String>,");

    let api = ApiSurface {
        enums: vec![enum_def],
        ..Default::default()
    };
    let dts = gen_dts(
        &api,
        "Js",
        &Default::default(),
        &[],
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        "",
        None,
    );
    let start = dts
        .find("export type SampleKind =")
        .expect("gen_dts must declare SampleKind as a discriminated union type, not a plain enum");
    let function_member = dts[start..]
        .lines()
        .find(|l| l.trim_start().starts_with("| { type: 'Function';"))
        .expect(".d.ts must declare the Function variant's member shape");
    assert_eq!(function_member.trim(), "| { type: 'Function'; function: string }");
}
