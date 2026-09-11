use super::Pyo3Backend;
use super::config::cfg_present_for_pyo3;
use super::mutex::{rewrite_to_tokio_mutex_impl, rewrite_to_tokio_mutex_struct};
use crate::backends::pyo3::type_map::Pyo3Mapper;
use crate::codegen::generators::gen_pyo3_data_enum_with_mapper;
use crate::core::backend::Backend;
use crate::core::config::Language;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeRef};

/// The production pyo3 data-enum path emits one `#[staticmethod]` constructor per data-carrying
/// struct variant, mapped through the real `Pyo3Mapper`. Proves the wiring at `gen_bindings/mod.rs`,
/// not just the generator helper.
#[test]
fn data_enum_emits_variant_constructors_through_pyo3_mapper() {
    let str_field = |name: &str| FieldDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..Default::default()
    };
    let def = EnumDef {
        name: "EmbeddingModelType".to_string(),
        rust_path: "crate::EmbeddingModelType".to_string(),
        serde_content: None,
        serde_tag: Some("type".to_string()),
        has_serde: true,
        variants: vec![
            EnumVariant {
                name: "Preset".to_string(),
                fields: vec![str_field("name")],
                ..Default::default()
            },
            EnumVariant {
                name: "Custom".to_string(),
                fields: vec![
                    str_field("model_id"),
                    FieldDef {
                        name: "dimensions".to_string(),
                        ty: TypeRef::Primitive(PrimitiveType::U32),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let generated = gen_pyo3_data_enum_with_mapper(&def, "core", Some(&Pyo3Mapper::new()));

    assert!(generated.contains("#[staticmethod]"), "{generated}");
    assert!(generated.contains(r#"#[pyo3(name = "preset")]"#), "{generated}");
    assert!(
        generated.contains("pub fn _factory_preset(name: String) -> Self"),
        "{generated}"
    );
    assert!(
        generated.contains("Self { inner: crate::EmbeddingModelType::Preset { name } }"),
        "{generated}"
    );
    assert!(generated.contains(r#"#[pyo3(name = "custom")]"#), "{generated}");
    assert!(
        generated.contains("pub fn _factory_custom(model_id: String, dimensions: u32) -> Self"),
        "{generated}"
    );
    assert!(
        generated.contains("Self { inner: crate::EmbeddingModelType::Custom { model_id, dimensions } }"),
        "{generated}"
    );
}

/// Pyo3Backend::name returns "pyo3".
#[test]
fn pyo3_backend_name_is_pyo3() {
    let b = Pyo3Backend;
    assert_eq!(b.name(), "pyo3");
}

/// Pyo3Backend::language returns Language::Python.
#[test]
fn pyo3_backend_language_is_python() {
    let b = Pyo3Backend;
    assert_eq!(b.language(), Language::Python);
}

/// rewrite_to_tokio_mutex_struct replaces std::sync::Mutex with tokio::sync::Mutex in struct.
#[test]
fn rewrite_tokio_mutex_struct_replaces_std_mutex() {
    let input = "pub inner: Arc<std::sync::Mutex<MyType>>";
    let result = rewrite_to_tokio_mutex_struct(input);
    assert_eq!(result, "pub inner: Arc<tokio::sync::Mutex<MyType>>");
}

/// rewrite_to_tokio_mutex_struct is a no-op when no std::sync::Mutex is present.
#[test]
fn rewrite_tokio_mutex_struct_noop_when_no_std_mutex() {
    let input = "pub inner: Arc<tokio::sync::Mutex<MyType>>";
    let result = rewrite_to_tokio_mutex_struct(input);
    assert_eq!(result, input);
}

/// rewrite_to_tokio_mutex_impl replaces all three patterns in impl block.
#[test]
fn rewrite_tokio_mutex_impl_replaces_all_patterns() {
    let input = concat!(
        "pub inner: Arc<std::sync::Mutex<MyType>>,\n",
        "Self { inner: Arc::new(std::sync::Mutex::new(val)) }\n",
        "let guard = self.inner.lock().unwrap();\n",
    );
    let result = rewrite_to_tokio_mutex_impl(input);
    assert!(result.contains("Arc<tokio::sync::Mutex<MyType>>"));
    assert!(result.contains("Arc::new(tokio::sync::Mutex::new(val))"));
    assert!(result.contains("self.inner.lock().await"));
}

/// rewrite_to_tokio_mutex_impl is a no-op when no std patterns are present.
#[test]
fn rewrite_tokio_mutex_impl_noop_when_already_tokio() {
    let input = concat!(
        "pub inner: Arc<tokio::sync::Mutex<MyType>>,\n",
        "Self { inner: Arc::new(tokio::sync::Mutex::new(val)) }\n",
        "let guard = self.inner.lock().await;\n",
    );
    let result = rewrite_to_tokio_mutex_impl(input);
    assert_eq!(result, input);
}

/// `cfg_present_for_pyo3` accepts `not(target_arch = "wasm32")` gates.
#[test]
fn cfg_present_for_pyo3_accepts_non_wasm_gate() {
    assert!(cfg_present_for_pyo3("not(target_arch = \"wasm32\")"));
    assert!(cfg_present_for_pyo3("not (target_arch = \"wasm32\")"));
}

/// `cfg_present_for_pyo3` accepts feature gates since pyo3 compiles with known features.
#[test]
fn cfg_present_for_pyo3_accepts_feature_gates() {
    assert!(cfg_present_for_pyo3("feature = \"pdf\""));
    assert!(cfg_present_for_pyo3("feature = \"html\""));
    assert!(cfg_present_for_pyo3("feature=\"tree-sitter\""));
    assert!(cfg_present_for_pyo3(
        "any(feature=\"keywords-yake\", feature=\"keywords-rake\")"
    ));
}

/// `cfg_present_for_pyo3` rejects unsupported gates.
#[test]
fn cfg_present_for_pyo3_rejects_unsupported_gates() {
    assert!(!cfg_present_for_pyo3("target_arch = \"wasm32\""));
    assert!(!cfg_present_for_pyo3("any(unix, windows)"));
    assert!(!cfg_present_for_pyo3("any(unix, feature=\"pdf\")"));
}

fn python_config() -> crate::core::config::ResolvedCrateConfig {
    let cfg: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "test_lib"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A `#[pyo3(get)]` field getter and a `#[pymethods]` wrapper with the same Python name both
/// bind that name on the class. pyo3 registers the method last, which silently kills the getter
/// and turns the documented attribute into a bound method. The field wins; the wrapper is
/// dropped, matching the ffi/wasm/go/ruby resolution.
#[test]
fn struct_impl_block_skips_method_colliding_with_a_field_getter() {
    use crate::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "test_lib::LlmConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "providers".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            methods: vec![MethodDef {
                name: "providers".to_string(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = Pyo3Backend.generate_bindings(&api, &python_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("pub providers:"),
        "the struct field must still be emitted:\n{content}"
    );
    let wrappers = content.matches("fn providers(&self)").count();
    assert_eq!(
        wrappers, 0,
        "the same-named #[pymethods] wrapper must be skipped, found {wrappers}:\n{content}"
    );
}

#[test]
fn from_json_deserializes_the_core_type_before_conversion() {
    use crate::core::ir::{ApiSurface, TypeDef};

    let temporary_directory = tempfile::tempdir().unwrap();
    let binding_directory = temporary_directory.path().join("test-lib-py");
    std::fs::create_dir_all(binding_directory.join("src")).unwrap();
    std::fs::write(
        binding_directory.join("Cargo.toml"),
        "[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\n",
    )
    .unwrap();
    let mut config = python_config();
    config
        .output_paths
        .insert("python".to_string(), binding_directory.join("src"));

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "ExtractionConfig".to_string(),
                rust_path: "test-lib::config::ExtractionConfig".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "chunking".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "BorrowedConfig".to_string(),
                rust_path: "test-lib::config::BorrowedConfig".to_string(),
                has_serde: true,
                has_lifetime_params: true,
                fields: vec![FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("serde_json::from_str::<test_lib::config::ExtractionConfig>(&json_str)"),
        "from_json must preserve the core serde contract before converting to the binding: {content}"
    );
    assert!(
        content.contains("serde_json::from_str::<test_lib::config::BorrowedConfig<'_>>(&json_str)"),
        "from_json must specialize lifetime-bearing core DTOs before deserializing: {content}"
    );
    assert!(
        content.contains(".map(Into::into)"),
        "from_json must convert the deserialized core value into the pyo3 wrapper: {content}"
    );
}

#[test]
fn lifetime_from_json_compiles_for_a_borrowing_core_dto() {
    use crate::core::ir::{ApiSurface, CoreWrapper, TypeDef};

    let temporary_directory = tempfile::tempdir().unwrap();
    let core_directory = temporary_directory.path().join("test-lib");
    let binding_directory = temporary_directory.path().join("test-lib-py");
    std::fs::create_dir_all(core_directory.join("src")).unwrap();
    std::fs::create_dir_all(binding_directory.join("src")).unwrap();
    std::fs::write(
        core_directory.join("Cargo.toml"),
        "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n",
    )
    .unwrap();
    std::fs::write(
        core_directory.join("src/lib.rs"),
        "use std::borrow::Cow;\n\
         #[derive(Clone, serde::Deserialize)]\n\
         pub struct BorrowedConfig<'a> {\n\
             #[serde(borrow)]\n\
             pub label: Cow<'a, str>,\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        binding_directory.join("Cargo.toml"),
        "[package]\nname = \"test-lib-py\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         [dependencies]\npyo3 = \"=0.29.2\"\nserde = { version = \"1\", features = [\"derive\"] }\n\
         serde_json = \"1\"\ntest-lib = { path = \"../test-lib\" }\n",
    )
    .unwrap();

    let mut config = python_config();
    config.python.as_mut().unwrap().module_name = Some("_test_lib".to_string());
    config
        .output_paths
        .insert("python".to_string(), binding_directory.join("src"));
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "BorrowedConfig".to_string(),
            rust_path: "test-lib::BorrowedConfig".to_string(),
            has_serde: true,
            has_lifetime_params: true,
            fields: vec![FieldDef {
                name: "label".to_string(),
                ty: TypeRef::String,
                core_wrapper: CoreWrapper::Cow,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let generated = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    std::fs::write(binding_directory.join("src/lib.rs"), &generated[0].content).unwrap();

    let output = std::process::Command::new("cargo")
        .args(["check", "--quiet", "--manifest-path"])
        .arg(binding_directory.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", temporary_directory.path().join("target"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "generated borrowing DTO binding must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Regression test for issue #380, exercised through the real `Pyo3Backend::generate_bindings`
/// path (not just the `gen_function` unit): a `&mut T` DTO parameter on a unit-returning free
/// function must render as a binding that returns the mutated intermediate.
#[test]
fn generate_bindings_writeback_free_function_returns_mutated_dto() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_record".to_string(),
            rust_path: "test_lib::tag_record".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Unit,
            ..Default::default()
        }],
        ..Default::default()
    };

    let files = Pyo3Backend.generate_bindings(&api, &python_config()).unwrap();
    let content = &files[0].content;

    assert!(
        content.contains("record_core.into()"),
        "expected the write-back tail through the real generate_bindings path:\n{content}"
    );
}

/// `reject_unsupported_writeback` must fire through the real `Pyo3Backend::generate_bindings`
/// path: a `&mut T` DTO param on a function that ALSO returns a value has no free return slot
/// for the write-back value, so generation must fail loudly (naming the function).
#[test]
fn generate_bindings_rejects_mut_dto_param_with_non_unit_return() {
    use crate::core::ir::{ApiSurface, FunctionDef, ParamDef};

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![crate::core::ir::TypeDef {
            name: "Record".to_string(),
            rust_path: "test_lib::Record".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "score".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "tag_and_count".to_string(),
            rust_path: "test_lib::tag_and_count".to_string(),
            params: vec![ParamDef {
                name: "record".to_string(),
                ty: TypeRef::Named("Record".to_string()),
                is_ref: true,
                is_mut: true,
                ..Default::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            ..Default::default()
        }],
        ..Default::default()
    };

    let error = Pyo3Backend
        .generate_bindings(&api, &python_config())
        .expect_err("a `&mut` DTO param plus a non-unit return must be rejected at generation time");
    let message = error.to_string();
    assert!(
        message.contains("tag_and_count"),
        "diagnostic must name the offending function:\n{message}"
    );
}
