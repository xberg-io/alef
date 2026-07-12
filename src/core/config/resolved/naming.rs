//! Language-specific naming methods for `ResolvedCrateConfig`.

use super::ResolvedCrateConfig;

impl ResolvedCrateConfig {
    /// Get the Python module name.
    pub fn python_module_name(&self) -> String {
        self.python
            .as_ref()
            .and_then(|p| p.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("_{}", self.name.replace('-', "_")))
    }

    /// Get the Node package name.
    pub fn node_package_name(&self) -> String {
        self.node
            .as_ref()
            .and_then(|n| n.package_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }

    /// Get the WASM npm package name.
    pub fn wasm_package_name(&self) -> String {
        self.wasm
            .as_ref()
            .and_then(|w| w.package_name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                let node_pkg = self.node_package_name();
                let base = node_pkg.strip_suffix("-node").unwrap_or(node_pkg.as_str());
                format!("{base}-wasm")
            })
    }

    /// Get the wasm-pack targets to build and publish for the WASM package.
    /// Defaults to every wasm-pack target when unset.
    pub fn wasm_targets(&self) -> Vec<String> {
        self.wasm
            .as_ref()
            .map(|w| w.targets.clone())
            .unwrap_or_else(crate::core::config::languages::default_wasm_targets)
    }

    /// Get the Ruby gem name.
    pub fn ruby_gem_name(&self) -> String {
        self.ruby
            .as_ref()
            .and_then(|r| r.gem_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the PHP extension name.
    pub fn php_extension_name(&self) -> String {
        self.php
            .as_ref()
            .and_then(|p| p.extension_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the PHP binding Cargo crate name (used for deriving the shared library filename).
    pub fn php_cargo_crate_name(&self) -> Option<&str> {
        self.php.as_ref().and_then(|p| p.cargo_crate_name.as_deref())
    }

    /// Get the Elixir app name.
    pub fn elixir_app_name(&self) -> String {
        self.elixir
            .as_ref()
            .and_then(|e| e.app_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the Zig module name.
    pub fn zig_module_name(&self) -> String {
        self.zig
            .as_ref()
            .and_then(|z| z.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the Dart pubspec package name.
    ///
    /// Returns `[dart] pubspec_name` if set, otherwise derives a snake_case
    /// name from the crate name by replacing hyphens with underscores.
    pub fn dart_pubspec_name(&self) -> String {
        self.dart
            .as_ref()
            .and_then(|d| d.pubspec_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the Dart FRB bridge class name.
    ///
    /// Converts `[dart] lib_name` (falling back to `dart_pubspec_name()`) to
    /// PascalCase and appends `"Bridge"`, matching the FRB v2 convention.
    /// E.g. `lib_name = "sample-widget"` → `"SampleWidgetBridge"`.
    pub fn dart_bridge_class_name(&self) -> String {
        use heck::ToUpperCamelCase;
        let lib_name = self
            .dart
            .as_ref()
            .and_then(|d| d.lib_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.dart_pubspec_name());
        format!("{}Bridge", lib_name.to_upper_camel_case())
    }

    /// Get the Swift module name.
    ///
    /// Returns `[swift] module_name` if configured, otherwise derives a PascalCase
    /// name from the crate name (e.g. `"my-lib"` → `"MyLib"`).
    pub fn swift_module(&self) -> String {
        self.swift
            .as_ref()
            .and_then(|s| s.module_name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                use heck::ToUpperCamelCase;
                self.name.to_upper_camel_case()
            })
    }

    /// Get the R package name.
    pub fn r_package_name(&self) -> String {
        self.r
            .as_ref()
            .and_then(|r| r.package_name.as_ref())
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }

    /// Get the WASM type name prefix (e.g. "Wasm" produces `WasmConversionOptions`).
    /// Defaults to `"Wasm"`.
    pub fn wasm_type_prefix(&self) -> String {
        self.wasm
            .as_ref()
            .and_then(|w| w.type_prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| "Wasm".to_string())
    }

    /// Get the Node/NAPI type name prefix (e.g. "Js" produces `JsConversionOptions`).
    /// Defaults to `"Js"`.
    pub fn node_type_prefix(&self) -> String {
        self.node
            .as_ref()
            .and_then(|n| n.type_prefix.as_ref())
            .cloned()
            .unwrap_or_else(|| "Js".to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::core::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> super::super::ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn python_module_name_defaults_to_underscore_prefix() {
        let r = minimal();
        assert_eq!(r.python_module_name(), "_test_lib");
    }

    #[test]
    fn python_module_name_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "mymod"
"#,
        );
        assert_eq!(r.python_module_name(), "mymod");
    }

    #[test]
    fn node_package_name_defaults_to_crate_name() {
        let r = minimal();
        assert_eq!(r.node_package_name(), "test-lib");
    }

    #[test]
    fn wasm_package_name_defaults_from_node_package_name() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["node", "wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.node]
package_name = "@scope/test-lib-node"
"#,
        );
        assert_eq!(r.wasm_package_name(), "@scope/test-lib-wasm");
    }

    #[test]
    fn wasm_package_name_uses_explicit_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
package_name = "@scope/explicit-wasm"
"#,
        );
        assert_eq!(r.wasm_package_name(), "@scope/explicit-wasm");
    }

    #[test]
    fn ruby_gem_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(r.ruby_gem_name(), "test_lib");
    }

    #[test]
    fn php_extension_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(r.php_extension_name(), "test_lib");
    }

    #[test]
    fn elixir_app_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(r.elixir_app_name(), "test_lib");
    }

    #[test]
    fn zig_module_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(r.zig_module_name(), "test_lib");
    }

    #[test]
    fn dart_pubspec_name_replaces_hyphens() {
        let r = minimal();
        assert_eq!(r.dart_pubspec_name(), "test_lib");
    }

    #[test]
    fn swift_module_is_pascal_case() {
        let r = minimal();
        assert_eq!(r.swift_module(), "TestLib");
    }

    #[test]
    fn r_package_name_defaults_to_crate_name() {
        let r = minimal();
        assert_eq!(r.r_package_name(), "test-lib");
    }

    #[test]
    fn wasm_type_prefix_defaults_to_wasm() {
        let r = minimal();
        assert_eq!(r.wasm_type_prefix(), "Wasm");
    }

    #[test]
    fn node_type_prefix_defaults_to_js() {
        let r = minimal();
        assert_eq!(r.node_type_prefix(), "Js");
    }
}
