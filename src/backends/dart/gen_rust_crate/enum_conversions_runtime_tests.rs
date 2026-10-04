use super::{
    enum_conversions::{emit_from_impl_for_enum, emit_from_mirror_to_core_enum},
    mirror_conversions::{emit_from_impl_for_struct, emit_from_mirror_to_core_struct},
    opaque::emit_enum_from_json_fn,
};
use crate::core::ir::{
    EnumDef, EnumVariant, FieldDef, NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, TypeDef, TypeRef,
};

fn unit_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        ..Default::default()
    }
}

#[test]
fn generated_json_decoder_returns_error_for_excluded_variant_at_runtime() {
    let en = EnumDef {
        name: "WorkflowStep".to_string(),
        rust_path: "mylib::WorkflowStep".to_string(),
        has_serde: true,
        variants: vec![unit_variant("Ready")],
        excluded_variants: vec![unit_variant("Internal")],
        ..Default::default()
    };
    let mut generated = String::new();
    emit_from_impl_for_enum(&mut generated, &en, "mylib", None);
    emit_enum_from_json_fn(&mut generated, &en, "mylib");
    let generated = generated.replace("#[frb]\n", "");
    let source = format!(
        "mod mylib {{\n    #[derive(serde::Deserialize)]\n    pub enum WorkflowStep {{ Ready, Internal }}\n}}\n\n#[derive(Debug, PartialEq)]\npub enum WorkflowStep {{ Ready }}\n\n#[allow(unreachable_patterns)]\n{generated}\nfn main() {{\n    assert_eq!(create_workflow_step_from_json(\"\\\"Ready\\\"\".to_string()), Ok(WorkflowStep::Ready));\n    let result = create_workflow_step_from_json(\"\\\"Internal\\\"\".to_string());\n    assert_eq!(result, Err(\"WorkflowStep contains a variant unavailable in the Dart binding\".to_string()));\n}}\n"
    );
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(temp.path().join("src")).expect("create src");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"dart-decoder-runtime\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .expect("write manifest");
    std::fs::write(temp.path().join("src/main.rs"), source).expect("write generated bridge source");
    let output = std::process::Command::new("cargo")
        .args(["run", "--quiet"])
        .env("CARGO_TARGET_DIR", temp.path().join("target"))
        // ~keep The real generated crate allows unreachable_patterns (emit_lib_rs), mirrored
        // only on the generated impl above. Keep every other warning fatal, including Cargo's
        // separate warning policy used by CI; removing RUSTFLAGS alone does not isolate it.
        .env("RUSTFLAGS", "-D warnings")
        .env("CARGO_BUILD_WARNINGS", "deny")
        .current_dir(temp.path())
        .output()
        .expect("run generated bridge crate");

    assert!(
        output.status.success(),
        "generated bridge crate must compile and return Err without panicking:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn transparent_string_enum_payload_uses_explicit_operations() {
    let en = EnumDef {
        name: "Authentication".to_string(),
        rust_path: "mylib::Authentication".to_string(),
        variants: vec![
            EnumVariant {
                name: "Bearer".to_string(),
                is_tuple: true,
                fields: vec![FieldDef {
                    name: "_0".to_string(),
                    ty: TypeRef::String,
                    newtype_wrapper: Some(NewtypeWrapper::encode_explicit(&[
                        NewtypeWrapperMetadata::transparent_string("mylib::SecretString", "from", "into_inner", vec![]),
                    ])),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Optional".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    newtype_wrapper: Some(NewtypeWrapper::encode_explicit(&[
                        NewtypeWrapperMetadata::transparent_string(
                            "mylib::SecretString",
                            "from",
                            "into_inner",
                            vec![crate::core::ir::NewtypeContainer::Optional],
                        ),
                    ])),
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Mixed".to_string(),
                fields: vec![FieldDef {
                    name: "segments".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Named("Segment".to_string())),
                    ))),
                    newtype_wrapper: Some(NewtypeWrapper::encode_explicit(&[
                        NewtypeWrapperMetadata::transparent_string(
                            "mylib::SecretString",
                            "from",
                            "into_inner",
                            vec![
                                crate::core::ir::NewtypeContainer::Vec,
                                crate::core::ir::NewtypeContainer::MapKey,
                            ],
                        ),
                    ])),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let mut to_core = String::new();
    emit_from_mirror_to_core_enum(&mut to_core, &en, "mylib", None);
    assert!(to_core.contains("mylib::SecretString::from(field0)"), "got:\n{to_core}");
    assert!(
        to_core.contains("if (value).is_empty() { None } else { Some(mylib::SecretString::from(value)) }"),
        "got:\n{to_core}"
    );
    assert!(
        to_core.contains("(mylib::SecretString::from(key), (value).into())"),
        "got:\n{to_core}"
    );
    assert_eq!(to_core.matches("segments).into_iter()").count(), 1, "got:\n{to_core}");

    let mut from_core = String::new();
    emit_from_impl_for_enum(&mut from_core, &en, "mylib", None);
    assert!(from_core.contains("(f0).into_inner()"), "got:\n{from_core}");
    assert!(
        from_core.contains("map(|value| (value).into_inner()).unwrap_or_default()"),
        "got:\n{from_core}"
    );
    assert!(
        from_core.contains("((key).into_inner(), Segment::from(value))"),
        "got:\n{from_core}"
    );
    assert_eq!(
        from_core.matches("segments).into_iter()").count(),
        1,
        "got:\n{from_core}"
    );
    assert!(!from_core.contains("to_string()"), "got:\n{from_core}");
}

#[test]
fn generated_optional_transparent_string_paths_compile() {
    let wrapper = |containers| {
        Some(NewtypeWrapper::encode_explicit(&[
            NewtypeWrapperMetadata::transparent_string("fixture::SecretString", "from", "into_inner", containers),
        ]))
    };
    let full_field = FieldDef {
        name: "full".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::String)),
        optional: true,
        newtype_wrapper: wrapper(vec![NewtypeContainer::Optional, NewtypeContainer::Optional]),
        ..Default::default()
    };
    let partial_field = FieldDef {
        name: "partial".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("Segment".to_string())),
        ))))),
        optional: true,
        newtype_wrapper: wrapper(vec![
            NewtypeContainer::Optional,
            NewtypeContainer::Optional,
            NewtypeContainer::Vec,
            NewtypeContainer::MapKey,
        ]),
        ..Default::default()
    };
    let typ = TypeDef {
        name: "Config".to_string(),
        rust_path: "fixture::Config".to_string(),
        fields: vec![full_field.clone(), partial_field.clone()],
        ..Default::default()
    };
    let en = EnumDef {
        name: "Choice".to_string(),
        rust_path: "fixture::Choice".to_string(),
        variants: vec![EnumVariant {
            name: "Values".to_string(),
            fields: vec![full_field, partial_field],
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut generated = String::new();
    emit_from_impl_for_struct(&mut generated, &typ, "fixture");
    emit_from_mirror_to_core_struct(&mut generated, &typ, "fixture");
    emit_from_impl_for_enum(&mut generated, &en, "fixture", None);
    emit_from_mirror_to_core_enum(&mut generated, &en, "fixture", None);

    let source = format!(
        r#"#![allow(dead_code)]
use std::collections::HashMap;

mod fixture {{
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, Eq, Hash)]
    pub struct SecretString(String);

    impl SecretString {{
        pub fn from(value: String) -> Self {{ Self(value) }}
        pub fn into_inner(self) -> String {{ self.0 }}
    }}

    pub struct Segment;

    pub struct Config {{
        pub full: Option<Option<SecretString>>,
        pub partial: Option<Option<Vec<HashMap<SecretString, Segment>>>>,
    }}

    pub enum Choice {{
        Values {{
            full: Option<Option<SecretString>>,
            partial: Option<Option<Vec<HashMap<SecretString, Segment>>>>,
        }},
    }}
}}

pub struct Segment;

impl From<fixture::Segment> for Segment {{
    fn from(_: fixture::Segment) -> Self {{ Self }}
}}

impl From<Segment> for fixture::Segment {{
    fn from(_: Segment) -> Self {{ Self }}
}}

pub struct Config {{
    pub full: Option<String>,
    pub partial: Option<Vec<HashMap<String, Segment>>>,
}}

pub enum Choice {{
    Values {{
        full: Option<String>,
        partial: Option<Vec<HashMap<String, Segment>>>,
    }},
}}

{generated}

fn require_all_directions(
    core_config: fixture::Config,
    mirror_config: Config,
    core_choice: fixture::Choice,
    mirror_choice: Choice,
) {{
    let _: Config = core_config.into();
    let _: fixture::Config = mirror_config.into();
    let _: Choice = core_choice.into();
    let _: fixture::Choice = mirror_choice.into();
}}

fn main() {{}}
"#
    );
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(temp.path().join("src")).expect("create src");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"dart-transparent-option-compile\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write manifest");
    std::fs::write(temp.path().join("src/main.rs"), source).expect("write generated bridge source");
    let output = std::process::Command::new("cargo")
        .args(["check", "--quiet"])
        .env("CARGO_TARGET_DIR", temp.path().join("target"))
        .current_dir(temp.path())
        .output()
        .expect("check generated Dart Rust conversions");

    assert!(
        output.status.success(),
        "generated Dart Rust optional wrapper paths must compile:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
