//! The node handle-config renderer's agreement with the binding it generates against.
//!
//! `build_args_and_setup`'s own `mod tests` covers argument shapes; these cover the one value that
//! had no typed renderer at all — a `handle` argument's config object — and the two ways the
//! untyped dump it used to take was wrong about what the napi binding accepts.

use super::*;

fn fixture() -> crate::e2e::fixture::Fixture {
    crate::e2e::fixture::Fixture {
        id: "crawl".to_string(),
        description: "Crawl".to_string(),
        ..Default::default()
    }
}

fn crawl_config_type_def() -> TypeDef {
    TypeDef {
        name: "CrawlConfig".into(),
        fields: vec![
            crate::core::ir::FieldDef {
                name: "content_filter".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ContentFilterKind".into()))),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "max_depth".into(),
                ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                ..Default::default()
            },
            crate::core::ir::FieldDef {
                name: "custom_headers".into(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn content_filter_kind_enum_def() -> EnumDef {
    EnumDef {
        name: "ContentFilterKind".into(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Bm25".into(),
            ..Default::default()
        }],
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    }
}

fn create_engine_function_def() -> crate::core::ir::FunctionDef {
    crate::core::ir::FunctionDef {
        name: "create_engine".into(),
        rust_path: "my_crate::create_engine".into(),
        params: vec![crate::core::ir::ParamDef {
            name: "config".into(),
            ty: TypeRef::Named("CrawlConfig".into()),
            optional: true,
            ..Default::default()
        }],
        return_type: TypeRef::Named("CrawlEngineHandle".into()),
        ..Default::default()
    }
}

fn engine_handle_arg() -> ArgMapping {
    ArgMapping {
        name: "engine".into(),
        field: "config".into(),
        arg_type: "handle".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// Regression for a real downstream crate's `E2E (node)` failure: five `tests/filter.test.ts` cases threw
/// `value "bm25" does not match any variant of enum JsContentFilterKind on
/// JsCrawlConfig.contentFilter` at run time.
///
/// A `handle` argument's config object was the one node value with no typed renderer — it
/// went through `json_to_js_camel`, which re-cases KEYS and copies VALUES verbatim. The
/// fixture holds serde's wire value (`"bm25"`), but napi re-cases variant names with
/// `convert_case`, which splits the letter-to-digit boundary serde does not, so the only
/// value the binding accepts is `'bm_25'` (the same fact `backends::napi::gen_bindings::
/// enums::apply_napi_case` already documents and the emitted `.d.ts` already declares).
/// Nothing checked that the string the e2e passed was one the binding produced.
///
/// The emitted form must be the binding's own declared member, not any re-derivation of the
/// literal, so it cannot drift from whatever string napi assigns the variant.
#[test]
fn node_handle_config_enum_value_renders_the_binding_declared_member() {
    let enums = [content_filter_kind_enum_def()];
    let type_defs = [crawl_config_type_def()];
    let functions = [create_engine_function_def()];
    let fixture = fixture();
    let input = serde_json::json!({ "config": { "content_filter": "bm25", "max_depth": 1 } });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig::default();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        false,
        &mut referenced_enums,
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr {
            functions: &functions,
            type_defs: &type_defs,
        },
    );

    assert_eq!(
        setup_lines[0], "const engineConfig = { contentFilter: ContentFilterKind.Bm25, maxDepth: 1 };",
        "the handle config must name the variant the napi binding declares, not the serde wire value"
    );
    assert!(
        !setup_lines[0].contains("\"bm25\""),
        "the serde wire value is not a literal any napi string enum accepts: {}",
        setup_lines[0]
    );
    assert!(
        referenced_enums.contains("ContentFilterKind"),
        "the referenced member must be registered so the import block carries it: {referenced_enums:?}"
    );
}

/// The typed route is taken only when the IR determines the constructor's config struct.
/// Without a `create_<handle>` signature to resolve there is nothing to type the object
/// against, and the untyped dump every non-node backend still uses must survive untouched.
#[test]
fn handle_config_without_a_resolvable_constructor_keeps_the_untyped_dump() {
    let enums = [content_filter_kind_enum_def()];
    let type_defs = [crawl_config_type_def()];
    let fixture = fixture();
    let input = serde_json::json!({ "config": { "content_filter": "bm25" } });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        false,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr::default(),
    );

    assert_eq!(
        setup_lines[0], "const engineConfig = { contentFilter: \"bm25\" };",
        "an unresolvable config type must leave the pre-existing untyped rendering in place"
    );
}
/// A map-typed field's entries are DATA, not field names: `custom_headers` is keyed
/// `Accept-Language`, which is not a bare JS identifier. The untyped `json_to_js_camel` dump
/// quoted such a key via `js_object_key`; the typed node renderer formatted it bare, so
/// routing the handle config through the typed path emitted `{ Accept-Language: "en-US" }` —
/// a JS syntax error that fails the whole file, not just one assertion. Both node
/// object-literal emitters now key through `node_field_public_key`, which quotes.
#[test]
fn node_handle_config_map_keys_that_are_not_identifiers_stay_quoted() {
    let enums = [content_filter_kind_enum_def()];
    let type_defs = [crawl_config_type_def()];
    let functions = [create_engine_function_def()];
    let fixture = fixture();
    let input = serde_json::json!({ "config": { "custom_headers": { "Accept-Language": "en-US" } } });
    let args = [engine_handle_arg()];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let (setup_lines, _call_args) = build_args_and_setup(
        &input,
        &args,
        None,
        &fixture,
        &Default::default(),
        "node",
        &Default::default(),
        &Default::default(),
        None,
        &type_defs,
        &enums,
        "",
        &config,
        false,
        &mut Default::default(),
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        crate::e2e::codegen::call_ir::CallIr {
            functions: &functions,
            type_defs: &type_defs,
        },
    );

    assert_eq!(
        setup_lines[0], "const engineConfig = { customHeaders: { \"Accept-Language\": \"en-US\" } };",
        "a map key that is not a bare JS identifier must stay quoted"
    );
}
