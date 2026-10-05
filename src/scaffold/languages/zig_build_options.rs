pub(super) fn render_ffi_option_declarations(ffi_lib: &str, ffi_crate_path: &str) -> (String, String) {
    let path = format!(concat!(
        "    const ffi_path_option = b.option([]const u8, \"ffi_path\", ",
        "\"Path to directory containing lib{ffi_lib}.{{dylib,so,dll,a}}\") ",
        "orelse \"../../target/release\";"
    ),);
    let include = format!(concat!(
        "    const ffi_include_option = b.option([]const u8, \"ffi_include_path\", ",
        "\"Path to directory containing the FFI C header\") ",
        "orelse \"{ffi_crate_path}/include\";"
    ),);
    (path, include)
}
