use crate::core::backend::Backend;
use crate::core::config::Language;

/// Get the backend for a given language.
///
/// Panics for languages that are not binding targets (Rust, C). Callers that
/// iterate every configured language — including docs-only or consumer
/// targets — should use [`try_get_backend`] instead.
pub fn get_backend(lang: Language) -> Box<dyn Backend> {
    try_get_backend(lang).unwrap_or_else(|| match lang {
        Language::Rust => panic!("Rust is a docs-only language target; it does not have a binding backend"),
        Language::C => panic!("C is an e2e test consumer target; it does not have a binding backend"),
        other => panic!("No backend registered for {other:?}"),
    })
}

/// Get the backend for a given language, returning `None` for languages
/// without a binding backend (Rust, C).
///
/// Use this from generic pipeline loops that iterate every configured
/// language, so docs-only and consumer targets can be skipped cleanly
/// instead of panicking.
pub fn try_get_backend(lang: Language) -> Option<Box<dyn Backend>> {
    let backend: Box<dyn Backend> = match lang {
        Language::Python => Box::new(crate::backends::pyo3::Pyo3Backend),
        Language::Node => Box::new(crate::backends::napi::NapiBackend),
        Language::Ruby => Box::new(crate::backends::magnus::MagnusBackend),
        Language::Php => Box::new(crate::backends::php::PhpBackend),
        Language::Elixir => Box::new(crate::backends::rustler::RustlerBackend),
        Language::Wasm => Box::new(crate::backends::wasm::WasmBackend),
        Language::Ffi => Box::new(crate::backends::ffi::FfiBackend),
        Language::Go => Box::new(crate::backends::go::GoBackend),
        Language::Java => Box::new(crate::backends::java::JavaBackend),
        Language::Csharp => Box::new(crate::backends::csharp::CsharpBackend),
        Language::R => Box::new(crate::backends::extendr::ExtendrBackend),
        Language::Rust | Language::C => return None,
        Language::Kotlin => Box::new(crate::backends::kotlin::KotlinBackend),
        Language::KotlinAndroid => Box::new(crate::backends::kotlin_android::KotlinAndroidBackend),
        Language::Swift => Box::new(crate::backends::swift::SwiftBackend),
        Language::Dart => Box::new(crate::backends::dart::DartBackend),
        Language::Gleam => Box::new(crate::backends::gleam::GleamBackend),
        Language::Zig => Box::new(crate::backends::zig::ZigBackend),
        Language::Jni => Box::new(crate::backends::jni::JniBackend),
    };
    Some(backend)
}
