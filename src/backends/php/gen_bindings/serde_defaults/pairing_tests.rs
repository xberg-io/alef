//! Agreement test for the two emitters that decide, independently, whether a field gets a
//! `crate::serde_defaults::*` function.
//!
//! `gen_php_struct`'s field-attribute pass writes the `#[serde(default = "crate::serde_defaults::X")]`
//! *reference*; `gen_serde_defaults_module` writes the `pub fn X` *definition*. When the two
//! disagree the generated crate does not compile (`E0425: cannot find function ... in module
//! crate::serde_defaults`). This test drives both real emitters from one IR fixture through
//! `generate_bindings` and asserts every reference resolves. ~keep

use std::collections::BTreeSet;

use super::super::rust_bindings::generate_bindings;
use crate::core::config::resolved::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, DefaultValue, EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};

const SERDE_DEFAULT_REFERENCE: &str = "crate::serde_defaults::";
const SERDE_DEFAULT_MODULE_HEADER: &str = "mod serde_defaults {";

fn identifier_at(source: &str) -> String {
    let end = source
        .find(|character: char| !character.is_alphanumeric() && character != '_')
        .unwrap_or(source.len());
    source[..end].to_string()
}

fn referenced_default_fns(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = source;
    while let Some(index) = rest.find(SERDE_DEFAULT_REFERENCE) {
        rest = &rest[index + SERDE_DEFAULT_REFERENCE.len()..];
        names.insert(identifier_at(rest));
    }
    names
}

fn defined_default_fns(source: &str) -> BTreeSet<String> {
    let Some(start) = source.find(SERDE_DEFAULT_MODULE_HEADER) else {
        return BTreeSet::new();
    };
    let module = &source[start..];
    let module = match module.find("\n}") {
        Some(end) => &module[..end],
        None => module,
    };
    let mut names = BTreeSet::new();
    let mut rest = module;
    while let Some(index) = rest.find("pub fn ") {
        rest = &rest[index + "pub fn ".len()..];
        names.insert(identifier_at(rest));
    }
    names
}

/// A tagged data enum forces `has_serde` on the generated crate without probing a real
/// `Cargo.toml` from the test process (see `php_crate_requires_serde`). ~keep
fn serde_forcing_enum() -> EnumDef {
    EnumDef {
        name: "Shape".to_string(),
        rust_path: "sample_core::Shape".to_string(),
        variants: vec![EnumVariant {
            name: "Circle".to_string(),
            fields: vec![FieldDef {
                name: "radius".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..Default::default()
            }],
            ..Default::default()
        }],
        serde_tag: Some("type".to_string()),
        ..Default::default()
    }
}

/// Shaped like the field that broke a consumer's PHP binding crate: a non-optional integer whose
/// `#[serde(default = "…")]` names a function, and whose `Default` impl calls that same function
/// (so the recovered typed default is a call, not a literal). ~keep
fn function_path_int_field() -> FieldDef {
    FieldDef {
        name: "max_archive_depth".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Usize),
        optional: false,
        default: Some("serde(default = \"ArchiveOptions::default_depth\")".to_string()),
        typed_default: Some(DefaultValue::PublicFunctionCall(
            "sample_core::ArchiveOptions::default_depth".to_string(),
        )),
        ..Default::default()
    }
}

fn literal_int_field() -> FieldDef {
    FieldDef {
        name: "retry_limit".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::I64),
        optional: false,
        typed_default: Some(DefaultValue::IntLiteral(3)),
        ..Default::default()
    }
}

fn excluded_bool_field() -> FieldDef {
    FieldDef {
        name: "internal_flag".to_string(),
        ty: TypeRef::Primitive(PrimitiveType::Bool),
        optional: false,
        typed_default: Some(DefaultValue::BoolLiteral(true)),
        binding_excluded: true,
        ..Default::default()
    }
}

fn archive_options() -> TypeDef {
    TypeDef {
        name: "ArchiveOptions".to_string(),
        rust_path: "sample_core::ArchiveOptions".to_string(),
        has_default: true,
        fields: vec![function_path_int_field(), literal_int_field(), excluded_bool_field()],
        ..Default::default()
    }
}

fn fixture_api() -> ApiSurface {
    ApiSurface {
        types: vec![archive_options()],
        enums: vec![serde_forcing_enum()],
        ..Default::default()
    }
}

fn generated_lib_rs(api: &ApiSurface) -> String {
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let files = generate_bindings(api, &config).expect("php bindings generated");
    files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated lib.rs")
        .content
        .clone()
}

#[test]
fn every_referenced_serde_default_fn_is_defined() {
    let lib_rs = generated_lib_rs(&fixture_api());
    let referenced = referenced_default_fns(&lib_rs);
    let defined = defined_default_fns(&lib_rs);
    let dangling: Vec<&String> = referenced.difference(&defined).collect();

    assert!(
        dangling.is_empty(),
        "`#[serde(default = \"crate::serde_defaults::…\")]` references with no matching `pub fn` — \
         the generated crate cannot compile (E0425). dangling={dangling:?}\n\
         referenced={referenced:?}\ndefined={defined:?}"
    );
}

/// Pairing alone would also be satisfied by dropping the attribute. The generated function must
/// actually call the core function the source `#[serde(default = "…")]` named, cast to the
/// PHP-facing width, or the mirror silently deserializes to `0` where core yields the real
/// default. ~keep
#[test]
fn function_path_default_calls_the_resolved_core_function() {
    let lib_rs = generated_lib_rs(&fixture_api());
    assert!(
        lib_rs.contains(
            "pub fn archive_options_max_archive_depth() -> i64 \
             { sample_core::ArchiveOptions::default_depth() as i64 }"
        ),
        "expected the resolved core call with an `as i64` cast, got:\n{lib_rs}"
    );
}

/// The literal-default field must keep its function — narrowing the shared predicate must not
/// silently drop defaults that already worked. ~keep
#[test]
fn literal_default_field_still_gets_a_function() {
    let lib_rs = generated_lib_rs(&fixture_api());
    assert!(
        defined_default_fns(&lib_rs).contains("archive_options_retry_limit"),
        "expected `archive_options_retry_limit` in the generated module, got:\n{lib_rs}"
    );
}

/// A `binding_excluded` field is not emitted into the binding struct at all, so neither emitter
/// may produce anything for it. ~keep
#[test]
fn binding_excluded_field_gets_neither_reference_nor_definition() {
    let lib_rs = generated_lib_rs(&fixture_api());
    let name = "archive_options_internal_flag".to_string();
    assert!(
        !referenced_default_fns(&lib_rs).contains(&name),
        "binding-excluded field must not be referenced, got:\n{lib_rs}"
    );
    assert!(
        !defined_default_fns(&lib_rs).contains(&name),
        "binding-excluded field must not be defined, got:\n{lib_rs}"
    );
}

/// Shaped like `OutputFormat`: a `#[derive(Default)]` unit-variant enum with `#[default] Plain`
/// and `#[serde(rename_all = "lowercase")]`, mirrored to `String` in the PHP binding (see
/// `PhpMapper::named`). ~keep
fn output_format_enum() -> EnumDef {
    EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "sample_core::OutputFormat".to_string(),
        variants: vec![
            EnumVariant {
                name: "Plain".to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Markdown".to_string(),
                ..Default::default()
            },
        ],
        serde_rename_all: Some("lowercase".to_string()),
        has_default: true,
        ..Default::default()
    }
}

/// Shaped like `ExtractionConfig.output_format`: a bare `#[serde(default)]` on a named-enum
/// field whose `Default` impl resolves to the enum's `#[default]` variant name, not a wire
/// value. Regression coverage for the `invalid value "" for field 'output_format'` defect: the
/// bare sentinel used to be mirrored verbatim, defaulting the `String` mirror field to `""`. ~keep
fn output_format_field() -> FieldDef {
    FieldDef {
        name: "output_format".to_string(),
        ty: TypeRef::Named("OutputFormat".to_string()),
        optional: false,
        default: Some("/* serde(default) */".to_string()),
        typed_default: Some(DefaultValue::EnumVariant("OutputFormat::Plain".to_string())),
        ..Default::default()
    }
}

fn extraction_config() -> TypeDef {
    TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "sample_core::ExtractionConfig".to_string(),
        has_default: true,
        fields: vec![output_format_field()],
        ..Default::default()
    }
}

fn enum_default_fixture_api() -> ApiSurface {
    ApiSurface {
        types: vec![extraction_config()],
        // `serde_forcing_enum()` has nothing to do with `output_format` -- it is here only to
        // make `php_crate_requires_serde` true in this fixture-driven test, same as
        // `fixture_api()` above (no real `Cargo.toml` to probe `php_serde_available` from). ~keep
        enums: vec![output_format_enum(), serde_forcing_enum()],
        ..Default::default()
    }
}

/// The generated attribute must reference a named `crate::serde_defaults::…` function returning
/// the enum's wire value, never a bare `#[serde(default)]` (which would default the `String`
/// mirror field to `""`, the exact defect this test locks in as fixed).
#[test]
fn enum_default_field_gets_named_wire_value_function_not_bare_default() {
    let lib_rs = generated_lib_rs(&enum_default_fixture_api());
    assert!(
        lib_rs.contains("serde(default = \"crate::serde_defaults::extraction_config_output_format\")"),
        "expected a named serde_defaults reference for output_format, got:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("pub output_format: String,\n    #[serde(default)]")
            && !lib_rs.contains("#[serde(default)]\n    pub output_format"),
        "must not emit a bare `#[serde(default)]` on output_format, got:\n{lib_rs}"
    );
}

/// The defined function must return the enum's *wire* value (`"plain"`, from
/// `#[serde(rename_all = "lowercase")]`), not the raw variant name (`"Plain"`) — a mismatch would
/// pass pairing but still 500 on `from_json` when the enum's `TryFrom` rejects the un-lowered
/// string at deserialize time.
#[test]
fn enum_default_function_returns_exact_wire_value() {
    let lib_rs = generated_lib_rs(&enum_default_fixture_api());
    assert!(
        lib_rs.contains("pub fn extraction_config_output_format() -> String { \"plain\".to_string() }"),
        "expected the wire value \"plain\", got:\n{lib_rs}"
    );
}
