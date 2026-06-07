use super::Pyo3Backend;
use super::config::cfg_present_for_pyo3;
use super::mutex::{rewrite_to_tokio_mutex_impl, rewrite_to_tokio_mutex_struct};
use crate::core::backend::Backend;
use crate::core::config::Language;

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
