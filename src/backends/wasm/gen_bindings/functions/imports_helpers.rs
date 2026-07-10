//! Import-adjacent helper generation for WASM bindings.

pub(in crate::backends::wasm::gen_bindings) fn emit_rustdoc(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let sanitized =
        crate::codegen::doc_emission::sanitize_rust_idioms(doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::backends::wasm::template_env::render(
        "rustdoc",
        minijinja::context! {
            lines => sanitized.lines().collect::<Vec<_>>(),
        },
    )
}

/// Convert a `TypeRef` to its concrete Rust type string for use in serde deserialization
/// let-bindings. Unlike `WasmMapper::map_type`, this always returns a concrete Rust type
/// (e.g. `String`, `Vec<String>`) rather than `JsValue`. Used when emitting
pub(in crate::backends::wasm::gen_bindings) fn gen_env_shims(shim_names: &[String]) -> String {
    let mut out = String::from("// WASM environment shims for C scanner interop\n");

    for name in shim_names {
        let shim = match name.as_str() {
            "iswspace" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswspace(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_whitespace() as i32)\n",
                "}\n",
            ),
            "iswalnum" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalnum(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphanumeric() as i32)\n",
                "}\n",
            ),
            "towupper" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn towupper(c: u32) -> u32 {\n",
                "    char::from_u32(c).map_or(c, |ch| ch.to_uppercase().next().unwrap_or(ch) as u32)\n",
                "}\n",
            ),
            "iswalpha" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalpha(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphabetic() as i32)\n",
                "}\n",
            ),
            "iswlower" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswlower(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_lowercase() as i32)\n",
                "}\n",
            ),
            "iswupper" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswupper(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_uppercase() as i32)\n",
                "}\n",
            ),
            "iswxdigit" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswxdigit(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_ascii_hexdigit() as i32)\n",
                "}\n",
            ),
            "towlower" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn towlower(c: u32) -> u32 {\n",
                "    char::from_u32(c).map_or(c, |ch| ch.to_lowercase().next().unwrap_or(ch) as u32)\n",
                "}\n",
            ),
            "memchr" => concat!(
                "/// # Safety\n",
                "/// Caller must ensure `s` points to a buffer of at least `n` bytes.\n",
                "#[unsafe(no_mangle)]\n",
                "pub unsafe extern \"C\" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8 {\n",
                "    if s.is_null() { return core::ptr::null(); }\n",
                "    let needle = c as u8;\n",
                "    let slice = unsafe { core::slice::from_raw_parts(s, n) };\n",
                "    match slice.iter().position(|&b| b == needle) {\n",
                "        Some(idx) => unsafe { s.add(idx) },\n",
                "        None => core::ptr::null(),\n",
                "    }\n",
                "}\n",
            ),
            "strcmp" => concat!(
                "/// # Safety\n",
                "/// Caller must ensure both pointers are valid null-terminated C strings.\n",
                "#[unsafe(no_mangle)]\n",
                "pub unsafe extern \"C\" fn strcmp(a: *const u8, b: *const u8) -> i32 {\n",
                "    if a.is_null() || b.is_null() { return 0; }\n",
                "    let mut i = 0isize;\n",
                "    loop {\n",
                "        let ca = unsafe { *a.offset(i) };\n",
                "        let cb = unsafe { *b.offset(i) };\n",
                "        if ca != cb { return (ca as i32) - (cb as i32); }\n",
                "        if ca == 0 { return 0; }\n",
                "        i += 1;\n",
                "    }\n",
                "}\n",
            ),
            _ => continue,
        };
        out.push_str(shim);
    }

    out.trim_end_matches('\n').to_string()
}
