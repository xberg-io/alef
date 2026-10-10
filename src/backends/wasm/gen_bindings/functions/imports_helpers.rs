//! Import-adjacent helper generation for WASM bindings.

pub(in crate::backends::wasm::gen_bindings) fn emit_rustdoc(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let sanitized =
        crate::codegen::doc_emission::sanitize_rust_idioms(doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::backends::wasm::template_env::render(
        "rustdoc",
        crate::alef_context! {
            lines => sanitized.lines().collect::<Vec<_>>(),
        },
    )
}

/// Convert a `TypeRef` to its concrete Rust type string for use in serde deserialization
/// let-bindings. Unlike `WasmMapper::map_type`, this always returns a concrete Rust type
/// (e.g. `String`, `Vec<String>`) rather than `JsValue`. Used when emitting
pub(in crate::backends::wasm::gen_bindings) fn gen_env_shims(shim_names: &[String]) -> String {
    const SUPPORTED_SHIMS: &[&str] = &[
        "iswspace",
        "iswalnum",
        "towupper",
        "iswalpha",
        "iswlower",
        "iswupper",
        "iswxdigit",
        "towlower",
        "memchr",
        "strcmp",
    ];
    let shims = shim_names
        .iter()
        .filter(|name| SUPPORTED_SHIMS.contains(&name.as_str()))
        .collect::<Vec<_>>();
    if shims.is_empty() {
        return String::new();
    }
    crate::backends::wasm::template_env::render("env_shims", crate::alef_context! { shims => shims })
        .trim_end_matches('\n')
        .to_string()
}
