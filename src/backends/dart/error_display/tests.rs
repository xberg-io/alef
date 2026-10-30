use super::*;
use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef, TypeRef};

fn variant(template: &str, fields: &[&str]) -> ErrorVariant {
    ErrorVariant {
        name: "Rejected".into(),
        message_template: Some(template.into()),
        fields: fields
            .iter()
            .map(|name| FieldDef {
                name: (*name).into(),
                ty: TypeRef::String,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn metadata_for(variant: ErrorVariant) -> String {
    let error = ErrorDef {
        name: "SampleError".into(),
        variants: vec![variant],
        exception_base: None,
        rust_path: "sample::SampleError".into(),
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    metadata_comments(&[&error])
}

#[test]
fn metadata_uses_only_visible_scalar_fields_and_retains_unsupported_fallbacks() {
    assert!(!metadata_for(variant("rejected: {raw_message}", &["raw_message"])).is_empty());
    for template in ["{missing}", "{raw_message:?}", "{raw_message:>8}", "{", "}"] {
        assert!(metadata_for(variant(template, &["raw_message"])).is_empty());
    }
    for kind in ["hidden", "optional", "sanitized", "json"] {
        let mut case = variant("{message}", &["message"]);
        match kind {
            "hidden" => case.fields[0].binding_excluded = true,
            "optional" => case.fields[0].optional = true,
            "sanitized" => case.fields[0].sanitized = true,
            _ => case.fields[0].ty = TypeRef::Json,
        }
        assert!(metadata_for(case).is_empty(), "{kind}");
    }
    let mut tuple = variant("{1}", &["_0", "_1"]);
    tuple.is_tuple = true;
    tuple.fields[0].binding_excluded = true;
    assert!(metadata_for(tuple.clone()).contains("\"field\":0"));
    tuple.message_template = Some("{0}".into());
    assert!(metadata_for(tuple).is_empty());
    let unit = ErrorVariant {
        is_unit: true,
        ..variant("literal", &[])
    };
    assert!(
        metadata_for(unit).is_empty(),
        "all-unit plain Dart enums remain unchanged"
    );
}

const BRIDGE: &str =
    "sealed class SampleError {\n const factory SampleError.rejected({required String renamedLabel}) = ActualImpl;\n}";
const FREEZED: &str = "class ActualImpl extends SampleError {\n const ActualImpl({required this.renamedLabel});\n final String renamedLabel;\n @override\n String toString() {\n return 'SampleError.rejected(renamedLabel: $renamedLabel)';\n }\n}\n";

#[test]
fn actual_factory_aliases_and_bound_fields_are_rewritten_idempotently() {
    let metadata = metadata_for(variant("prefix {{ {raw_message} }} $literal", &["raw_message"]));
    let result = rewrite(FREEZED, BRIDGE, &metadata).unwrap();
    assert!(result.contains("renamedLabel.toString()"), "{result}");
    assert!(result.contains("\\$literal"), "{result}");
    assert_eq!(rewrite(&result, BRIDGE, &metadata).unwrap(), result);
    assert!(!result.contains("SampleError.rejected(renamedLabel:"));
    for (source, bridge) in [
        ("".to_string(), BRIDGE.to_string()),
        (FREEZED.repeat(2), BRIDGE.to_string()),
        (FREEZED.to_string(), BRIDGE.repeat(2)),
        (FREEZED.replace("toString", "otherMethod"), BRIDGE.to_string()),
        (
            FREEZED.replace("this.renamedLabel", "this.renamedLabel, required this.extra"),
            BRIDGE.to_string(),
        ),
    ] {
        assert!(
            rewrite(&source, &bridge, &metadata).is_err(),
            "missing/duplicate layout must reject"
        );
    }
    assert!(rewrite(FREEZED, BRIDGE, &(metadata.clone() + "\n" + &metadata)).is_err());
    assert_eq!(rewrite(FREEZED, BRIDGE, "unrelated mirror").unwrap(), FREEZED);
}

#[test]
fn lexical_bounds_ignore_strings_nested_interpolations_comments_and_generic_commas() {
    let source = r##"{ /* class Fake { /* nested */ } */ var x = r'\'; var y = '''}'''; var z = '${f("}")}'; }"##;
    let masked = lexical::mask(source).unwrap();
    assert_eq!(masked.len(), source.len());
    assert!(!masked.contains("class Fake"));
    assert_eq!(lexical::closing(source, 0).unwrap(), source.len() - 1);
    assert_eq!(
        lexical::parameters("{required Map<String, List<int>> values, required this.label}").unwrap(),
        ["values", "label"]
    );
    for malformed in ["{", "{]", "'unclosed", "/* unclosed"] {
        assert!(
            lexical::mask(malformed)
                .and_then(|_| lexical::closing(malformed, 0))
                .is_err()
        );
    }
}

#[path = "native.rs"]
mod native;

#[test]
fn reserved_error_uses_actual_frb_owner_through_display_restoration() {
    use crate::backends::dart::DartBackend;
    use crate::core::backend::Backend;
    use crate::core::config::NewAlefConfig;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        r#"#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Language '{0}' not found")]
    LanguageNotFound(String),
}
pub fn load() -> Result<(), Error> { Err(Error::LanguageNotFound("missing".into())) }
"#,
    )
    .unwrap();
    let api = crate::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.errors.len(), 1);
    assert_eq!(api.errors[0].variants.len(), 1);
    let config: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['dart']\n[[crates]]\nname='sample'\nsources=[]\n").unwrap();
    let config = config.resolve().unwrap().remove(0);
    let files = DartBackend.generate_bindings(&api, &config).unwrap();
    let facades = files
        .iter()
        .filter(|file| file.content.contains("alef-error-display:"))
        .collect::<Vec<_>>();
    assert_eq!(facades.len(), 1);
    let facade = &facades[0].content;
    assert!(facade.contains("pub enum Error {"));
    let bridge = "sealed class Error {\n const factory Error.languageNotFound({required String field0}) = Error_LanguageNotFound;\n}";
    let freezed = "class Error_LanguageNotFound extends Error {\n const Error_LanguageNotFound({required this.field0});\n final String field0;\n @override\n String toString() {\n return 'Error.languageNotFound(field0: $field0)';\n }\n}\n";
    let restored = rewrite(freezed, bridge, facade).expect("metadata must address the actual FRB factory");
    assert!(restored.contains("field0.toString()"), "{restored}");
    assert_eq!(rewrite(&restored, bridge, facade).unwrap(), restored);
    let records = read_records(facade).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].owner, "Error");
    assert_eq!(records[0].factory, "languageNotFound");
    assert!(
        facade.contains("AlefBridgeErrorWire_Error(:final error) => error"),
        "{facade}"
    );
    assert!(
        facade.contains("Error error => AlefBridgeErrorWire.error(error: error)"),
        "{facade}"
    );
    assert!(!facade.contains("ErrorNode"), "{facade}");
}
