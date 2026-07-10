//! Shared C-FFI consumer scaffolding for language backends.
//!
//! This module provides utilities for generating language bindings that consume
//! the C FFI layer produced by cbindgen. Each consumer backend (Go, Java, C#, Zig)
//! uses the same C interface:
//! - A C header file (`config.ffi_header_name()`)
//! - A library name (`config.ffi_lib_name()`)
//! - A symbol prefix (`config.ffi_prefix()`)
//! - Standard helper symbols: `{prefix}_free_string`, `{prefix}_last_error_code`, `{prefix}_last_error_context`

use crate::core::config::{ResolvedCrateConfig, resolve_output_dir};
use std::path::PathBuf;

/// Context capturing the shared FFI consumer inputs across all language backends.
pub struct CConsumerContext<'a> {
    /// Reference to the resolved crate configuration.
    pub config: &'a ResolvedCrateConfig,
    /// C header filename (e.g., "sample_markdown.h").
    pub header: String,
    /// C library name used for linking (e.g., "sample_markdown").
    pub lib_name: String,
    /// C symbol prefix for FFI functions (e.g., "htm").
    pub prefix: String,
}

impl<'a> CConsumerContext<'a> {
    /// Create a new CConsumerContext from the resolved crate configuration.
    pub fn from_config(config: &'a ResolvedCrateConfig) -> Self {
        Self {
            config,
            header: config.ffi_header_name(),
            lib_name: config.ffi_lib_name(),
            prefix: config.ffi_prefix(),
        }
    }
}

/// Return the C symbol name for freeing FFI-allocated strings.
///
/// Format: `{prefix}_free_string`
///
/// # Example
/// ```ignore
/// let sym = free_string_symbol("htm");
/// assert_eq!(sym, "htm_free_string");
/// ```
pub fn free_string_symbol(prefix: &str) -> String {
    format!("{prefix}_free_string")
}

/// Return the C symbol name for reading the thread-local last error code.
///
/// Format: `{prefix}_last_error_code`
///
/// # Example
/// ```ignore
/// let sym = last_error_code_symbol("krz");
/// assert_eq!(sym, "krz_last_error_code");
/// ```
pub fn last_error_code_symbol(prefix: &str) -> String {
    format!("{prefix}_last_error_code")
}

/// Return the C symbol name for reading the thread-local last error context message.
///
/// Format: `{prefix}_last_error_context`
///
/// # Example
/// ```ignore
/// let sym = last_error_context_symbol("krz");
/// assert_eq!(sym, "krz_last_error_context");
/// ```
pub fn last_error_context_symbol(prefix: &str) -> String {
    format!("{prefix}_last_error_context")
}

/// Resolve the per-backend output directory for generated files.
///
/// This helper wraps `resolve_output_dir` with a sensible default for C-FFI consumers,
/// allowing backends to pass a language-specific default (e.g., "packages/go/", "packages/java/src/main/java/").
///
/// # Arguments
/// - `config`: The Alef configuration.
/// - `default`: The backend-specific default output directory (e.g., "packages/go/").
///
/// # Returns
/// A PathBuf representing the resolved output directory.
pub fn default_output_dir(config: &ResolvedCrateConfig, default: &str) -> PathBuf {
    let resolved = resolve_output_dir(None, &config.name, default);
    PathBuf::from(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    fn make_config() -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn free_string_symbol_produces_expected_format() {
        assert_eq!(free_string_symbol("htm"), "htm_free_string");
    }

    #[test]
    fn last_error_code_symbol_produces_expected_format() {
        assert_eq!(last_error_code_symbol("krz"), "krz_last_error_code");
    }

    #[test]
    fn last_error_context_symbol_produces_expected_format() {
        assert_eq!(last_error_context_symbol("krz"), "krz_last_error_context");
    }

    #[test]
    fn from_config_reads_ffi_fields() {
        let config = make_config();
        let ctx = CConsumerContext::from_config(&config);
        assert!(!ctx.header.is_empty());
        assert!(!ctx.lib_name.is_empty());
        assert!(!ctx.prefix.is_empty());
    }

    #[test]
    fn default_output_dir_uses_provided_default() {
        let config = make_config();
        let dir = default_output_dir(&config, "packages/go/");
        assert!(dir.to_string_lossy().contains("go"));
    }
}
