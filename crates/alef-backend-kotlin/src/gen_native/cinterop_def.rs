//! Emit the cinterop `.def` file for a Kotlin/Native project.

use alef_core::config::AlefConfig;

/// Generate the contents of the `<crate>.def` cinterop definition file.
pub(super) fn emit_def_file(config: &AlefConfig) -> String {
    let header = config.ffi_header_name();
    let lib_name = config.ffi_lib_name();
    let prefix = config.ffi_prefix();

    format!(
        "headers = {header}\nheaderFilter = {prefix}_*\nlinkerOpts = -L../../../target/release -l{lib_name}\n"
    )
}
