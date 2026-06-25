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

    // One `_factory_<name>` staticmethod per struct variant, each exposed under its snake_case
    // name and building the core variant struct literal directly.
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
