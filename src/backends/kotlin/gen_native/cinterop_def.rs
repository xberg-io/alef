//! Emit the cinterop `.def` file for a Kotlin/Native project.

use crate::core::config::ResolvedCrateConfig;

/// Generate the contents of the `<crate>.def` cinterop definition file.
pub(super) fn emit_def_file(config: &ResolvedCrateConfig) -> String {
    let header = config.ffi_header_name();
    let lib_name = config.ffi_lib_name();
    let prefix = config.ffi_prefix();

    format!("headers = {header}\nheaderFilter = {prefix}_*\nlinkerOpts = -L../../../target/release -l{lib_name}\n")
}
