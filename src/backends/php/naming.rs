//! PHP-specific naming helpers for `ResolvedCrateConfig`.

use crate::core::config::ResolvedCrateConfig;

/// Get the PHP Composer autoload namespace.
///
/// If `[crates.php] namespace` is configured, uses that verbatim.
/// Otherwise, derives the namespace from the extension name (e.g. `sample_crate` -> `Sample\\Crate`).
pub fn php_autoload_namespace(config: &ResolvedCrateConfig) -> String {
    use heck::ToPascalCase;

    if let Some(php_cfg) = &config.php
        && let Some(ns) = &php_cfg.namespace
    {
        return ns.clone();
    }

    let ext = config.php_extension_name();
    if ext.contains('_') {
        ext.split('_')
            .map(|p| p.to_pascal_case())
            .collect::<Vec<_>>()
            .join("\\")
    } else {
        ext.to_pascal_case()
    }
}

/// Get the ext-php-rs facade class name that exposes crate-level free functions as static
/// methods (e.g. `sample_crate` -> `SampleCrateApi`).
///
/// The php-ext backend never emits free functions as global `#[php_function]` items: ext-php-rs's
/// `#[php_impl]` registration derive walks every method in a fixed `impl` block and unconditionally
/// references it by Rust identifier, so free functions are placed as static methods on this facade
/// class instead (see `gen_bindings/rust_bindings.rs`). Callers that need to invoke a free function
/// through the generated extension — including the php_ext e2e smoke-app generator — must go
/// through `{php_autoload_namespace}\{php_ext_api_class_name}::{method}`, never a bare global
/// function name.
/// The PHP class holding the generated public static methods (e.g. `sample_crate` ->
/// `SampleCrate`). Distinct from [`php_ext_api_class_name`]: this is the hand-facing wrapper the
/// consumer calls, which forwards to the `…Api` extension class. ~keep
pub fn php_public_class_name(extension_name: &str) -> String {
    use heck::ToPascalCase;

    extension_name.to_pascal_case()
}

/// The PHP method name for a configured trait-bridge `register_fn`/`unregister_fn`/`clear_fn`.
///
/// The same transform names the `#[php(name = "…")]` on the extension class and the static
/// method on the public wrapper, so `PhpBackend::trait_bridge_registration_surface` reports
/// through it rather than restating the casing rule. ~keep
pub fn php_bridge_method_name(configured_fn: &str) -> String {
    use heck::ToLowerCamelCase;

    configured_fn.to_lower_camel_case()
}

pub fn php_ext_api_class_name(extension_name: &str) -> String {
    use heck::ToPascalCase;

    format!("{}Api", extension_name.to_pascal_case())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn php_autoload_namespace_converts_snake_to_pascal_parts() {
        let r = minimal();
        assert_eq!(php_autoload_namespace(&r), "Test\\Lib");
    }

    #[test]
    fn php_autoload_namespace_no_underscore_returns_single_pascal() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(php_autoload_namespace(&r), "Mylib");
    }

    #[test]
    fn php_autoload_namespace_explicit_extension_name() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "sample_markdown_rs"
"#,
        );
        assert_eq!(php_autoload_namespace(&r), "Sample\\Markdown\\Rs");
    }

    #[test]
    fn php_autoload_namespace_explicit_namespace_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample-markdown"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "sample_markdown_rs"
namespace = "SampleMarkdown"
"#,
        );
        assert_eq!(php_autoload_namespace(&r), "SampleMarkdown");
    }
}

/// The name of the project-level exception class every typed error extends, for example
/// `LiterLlmException` for the extension `liter_llm`. The stub and the runtime use the same name.
pub(crate) fn php_root_exception_name(extension_name: &str) -> String {
    format!("{}Exception", heck::ToPascalCase::to_pascal_case(extension_name))
}
