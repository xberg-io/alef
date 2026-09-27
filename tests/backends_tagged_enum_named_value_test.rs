use alef::backends::csharp::CsharpBackend;
use alef::backends::dart::DartBackend;
use alef::backends::extendr::ExtendrBackend;
use alef::backends::ffi::FfiBackend;
use alef::backends::gleam::GleamBackend;
use alef::backends::go::GoBackend;
use alef::backends::java::JavaBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::backends::kotlin_android::KotlinAndroidBackend;
use alef::backends::magnus::MagnusBackend;
use alef::backends::napi::NapiBackend;
use alef::backends::php::PhpBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::backends::rustler::RustlerBackend;
use alef::backends::swift::SwiftBackend;
use alef::backends::wasm::WasmBackend;
use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

fn named_value_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_owned(),
        fields: vec![FieldDef {
            name: "value".to_owned(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..EnumVariant::default()
    }
}

fn tagged_enum_surface() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "fixture_core".to_owned(),
        version: "1.0.0".to_owned(),
        enums: vec![EnumDef {
            name: "MatchRule".to_owned(),
            rust_path: "fixture_core::MatchRule".to_owned(),
            serde_tag: Some("kind".to_owned()),
            serde_content: None,
            serde_rename_all: Some("snake_case".to_owned()),
            has_serde: true,
            variants: vec![
                named_value_variant("Exact"),
                named_value_variant("Suffix"),
                named_value_variant("Pattern"),
            ],
            ..EnumDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn binding_backends() -> Vec<(&'static str, Box<dyn Backend>, &'static [&'static str])> {
    const FULL_SURFACE: &[&str] = &["MatchRule", "Exact", "Suffix", "Pattern", "value"];
    const SNAKE_CASE_VARIANTS: &[&str] = &["MatchRule", "exact", "suffix", "pattern"];
    vec![
        ("pyo3", Box::new(Pyo3Backend), FULL_SURFACE),
        ("napi", Box::new(NapiBackend), FULL_SURFACE),
        ("magnus", Box::new(MagnusBackend), FULL_SURFACE),
        ("php", Box::new(PhpBackend), FULL_SURFACE),
        ("wasm", Box::new(WasmBackend), FULL_SURFACE),
        ("ffi", Box::new(FfiBackend), FULL_SURFACE),
        ("go", Box::new(GoBackend), FULL_SURFACE),
        ("java", Box::new(JavaBackend), FULL_SURFACE),
        // ~keep Kotlin aliases Java DTOs; JNI only emits native call shims and has no public DTO surface.
        ("kotlin", Box::new(KotlinBackend), &["MatchRule"]),
        ("kotlin_android", Box::new(KotlinAndroidBackend), FULL_SURFACE),
        ("csharp", Box::new(CsharpBackend), FULL_SURFACE),
        ("dart", Box::new(DartBackend), FULL_SURFACE),
        ("swift", Box::new(SwiftBackend), FULL_SURFACE),
        ("zig", Box::new(ZigBackend), SNAKE_CASE_VARIANTS),
        ("gleam", Box::new(GleamBackend), FULL_SURFACE),
        ("rustler", Box::new(RustlerBackend), FULL_SURFACE),
        ("extendr", Box::new(ExtendrBackend), FULL_SURFACE),
    ]
}

fn binding_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "fixture_core"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.example.fixture"
namespace = "dev.example.fixture"

[crates.package_metadata]
repository = "https://example.com/fixture-core"
license = "MIT"
"#,
    )
    .expect("fixture config should parse");
    config.resolve().expect("fixture config should resolve").remove(0)
}

#[test]
fn internally_tagged_named_value_enum_is_present_in_every_binding() {
    let api = tagged_enum_surface();
    let config = binding_config();

    for (name, backend, expected_items) in binding_backends() {
        let files = backend
            .generate_bindings(&api, &config)
            .unwrap_or_else(|error| panic!("{name}: generation failed: {error:#}"));
        let output = files
            .iter()
            .map(|file| file.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!output.is_empty(), "{name}: generated no binding output");
        for expected in expected_items {
            assert!(
                output.contains(expected),
                "{name}: generated output omitted `{expected}` from MatchRule:\n{output}"
            );
        }
    }
}
