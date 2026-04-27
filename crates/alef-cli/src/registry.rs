use alef_core::backend::Backend;
use alef_core::config::Language;

/// Get the backend for a given language.
pub fn get_backend(lang: Language) -> Box<dyn Backend> {
    match lang {
        Language::Python => Box::new(alef_backend_pyo3::Pyo3Backend),
        Language::Node => Box::new(alef_backend_napi::NapiBackend),
        Language::Ruby => Box::new(alef_backend_magnus::MagnusBackend),
        Language::Php => Box::new(alef_backend_php::PhpBackend),
        Language::Elixir => Box::new(alef_backend_rustler::RustlerBackend),
        Language::Wasm => Box::new(alef_backend_wasm::WasmBackend),
        Language::Ffi => Box::new(alef_backend_ffi::FfiBackend),
        Language::Go => Box::new(alef_backend_go::GoBackend),
        Language::Java => Box::new(alef_backend_java::JavaBackend),
        Language::Csharp => Box::new(alef_backend_csharp::CsharpBackend),
        Language::R => Box::new(alef_backend_extendr::ExtendrBackend),
        Language::Rust => panic!("Rust is a docs-only language target; it does not have a binding backend"),
        Language::Kotlin => Box::new(alef_backend_kotlin::KotlinBackend),
        Language::Swift => Box::new(alef_backend_swift::SwiftBackend),
        Language::Dart => Box::new(alef_backend_dart::DartBackend),
        Language::Gleam => Box::new(alef_backend_gleam::GleamBackend),
        Language::Zig => Box::new(alef_backend_zig::ZigBackend),
    }
}
