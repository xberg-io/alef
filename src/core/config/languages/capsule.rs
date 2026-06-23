//! Shared host-native capsule (Language-passthrough) config for the C-ABI family backends.
//!
//! Every C-ABI binding (Go, Java, C#, Swift, Dart, Zig, Kotlin Android) links the same C
//! symbol emitted by the FFI backend, which returns the host runtime's raw grammar pointer
//! for capsule types instead of an opaque alef handle. Each binding then wraps that raw
//! pointer in its own ecosystem's native `Language` type.
//!
//! This struct captures the per-backend host construction: the host type name to annotate the
//! return as, the package/module to depend on, its version, and the construction expression.
//! The `{ptr}` placeholder in `construct_expr` is replaced with the raw pointer expression at
//! the FFI boundary in the target language.
//!
//! `host_type` and `construct_expr` are **required** at emission time — backends call
//! [`HostCapsuleTypeConfig::construct_required`] and check [`Self::required_host_type`] which
//! return descriptive errors when the fields are missing.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Host-native capsule config for a single type in one C-ABI family backend.
///
/// TOML form (Go example):
/// ```toml
/// [crates.go.capsule_types.Language]
/// host_type = "*my_pkg.Language"
/// package = "github.com/example/go-my-lib"
/// package_version = "v1.0.0"
/// construct_expr = "my_pkg.NewLanguage(unsafe.Pointer({ptr}))"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct HostCapsuleTypeConfig {
    /// The host ecosystem's `Language` type, used as the return-type annotation in the
    /// generated binding (e.g. `"*my_pkg.Language"` for Go, `"MyLib.Language"` for Swift).
    /// **Required** — backends error at emission time when this is empty.
    pub host_type: String,
    /// The host package/module identifier to depend on. Injected into the backend's
    /// package manifest by the scaffold layer. Empty string disables injection.
    #[serde(default)]
    pub package: String,
    /// The version constraint for `package`. Format is backend-specific and passed through
    /// verbatim to the manifest.
    #[serde(default)]
    pub package_version: String,
    /// The construction expression that wraps the raw FFI pointer in the host `Language`.
    /// The `{ptr}` placeholder is substituted with the raw-pointer expression produced at
    /// the FFI boundary. **Required** — backends error at emission time when this is empty.
    #[serde(default)]
    pub construct_expr: String,
}

impl HostCapsuleTypeConfig {
    /// Returns the construction expression with `{ptr}` substituted by `ptr_expr`,
    /// falling back to `default_expr` (also `{ptr}`-templated) when `construct_expr` is empty.
    ///
    /// Prefer [`Self::construct_required`] when there is no sensible language-generic
    /// default — that method errors instead of silently substituting a fallback.
    pub fn construct(&self, ptr_expr: &str, default_expr: &str) -> String {
        let template = if self.construct_expr.is_empty() {
            default_expr
        } else {
            self.construct_expr.as_str()
        };
        template.replace("{ptr}", ptr_expr)
    }

    /// Returns the construction expression with `{ptr}` substituted by `ptr_expr`.
    ///
    /// Errors with a descriptive message when `construct_expr` is empty, naming the
    /// `type_name` and `backend` for easy diagnosis from `alef.toml`. Use this in
    /// backends where there is no acceptable generic default — callers MUST supply
    /// `construct_expr` in their `alef.toml`.
    pub fn construct_required(&self, ptr_expr: &str, type_name: &str, backend: &str) -> Result<String, anyhow::Error> {
        if self.construct_expr.is_empty() {
            anyhow::bail!(
                "capsule type `{type_name}` in backend `{backend}`: \
                 `construct_expr` is required but not set in alef.toml — \
                 add `construct_expr = \"<expr using {{ptr}}>\"` under \
                 `[crates.{backend}.capsule_types.{type_name}]`"
            );
        }
        Ok(self.construct_expr.replace("{ptr}", ptr_expr))
    }

    /// Returns `host_type`, or errors with a descriptive message when it is empty.
    pub fn required_host_type(&self, type_name: &str, backend: &str) -> Result<&str, anyhow::Error> {
        if self.host_type.is_empty() {
            anyhow::bail!(
                "capsule type `{type_name}` in backend `{backend}`: \
                 `host_type` is required but not set in alef.toml — \
                 add `host_type = \"<language type>\"` under \
                 `[crates.{backend}.capsule_types.{type_name}]`"
            );
        }
        Ok(&self.host_type)
    }
}

/// Extract the Zig import name from a capsule `host_type` expression.
///
/// For a `host_type` like `"?*const tree_sitter.Language"` the import name is
/// `"tree_sitter"` — the first dotted identifier that is not a pointer/optional
/// sigil. The caller emits `const {name} = @import("{name}");`.
///
/// Returns `None` when the host_type contains no dotted qualified name (e.g. a
/// bare type with no module prefix).
pub fn zig_capsule_import_name(host_type: &str) -> Option<&str> {
    // Find the first whitespace-separated token that contains a dot (e.g. `my_mod.Language`).
    // This skips leading sigil tokens like `?`, `*`, `?*const`, etc.
    // e.g. `"?*const my_mod.Language"` → token `"my_mod.Language"` → `"my_mod"`.
    let qualified = host_type.split_whitespace().find(|token| token.contains('.'))?;
    // Strip any leading `?` or `*` sigils from the token itself.
    let qualified = qualified.trim_start_matches(['?', '*']);
    if qualified.is_empty() || !qualified.contains('.') {
        return None;
    }
    qualified.split('.').next()
}

/// Collect the distinct Zig module import names for every capsule type that declares a
/// non-empty `package`. Shared by the scaffolded in-tree `build.zig` and the published
/// distributable `build.zig` so their `b.dependency`/`addImport` wiring stays in sync —
/// a published tarball missing this wiring fails consumers with
/// `no module named '<name>' available within module '<module>'`.
pub fn zig_capsule_import_names(
    capsule_types: &std::collections::HashMap<String, HostCapsuleTypeConfig>,
) -> std::collections::BTreeSet<String> {
    capsule_types
        .values()
        .filter(|cap| !cap.package.is_empty())
        .filter_map(|cap| zig_capsule_import_name(&cap.host_type).map(|s| s.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cfg(host_type: &str, construct_expr: &str) -> HostCapsuleTypeConfig {
        HostCapsuleTypeConfig {
            host_type: host_type.to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: construct_expr.to_string(),
        }
    }

    #[test]
    fn construct_required_substitutes_ptr_placeholder() {
        let cfg = make_cfg("*my_pkg.Language", "my_pkg.NewLanguage(unsafe.Pointer({ptr}))");
        assert_eq!(
            cfg.construct_required("ptr", "Language", "go").unwrap(),
            "my_pkg.NewLanguage(unsafe.Pointer(ptr))"
        );
    }

    #[test]
    fn construct_required_errors_when_construct_expr_empty() {
        let cfg = make_cfg("*my_pkg.Language", "");
        let err = cfg.construct_required("ptr", "Language", "go").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("construct_expr"), "error must mention the field: {msg}");
        assert!(msg.contains("Language"), "error must name the type: {msg}");
        assert!(msg.contains("go"), "error must name the backend: {msg}");
    }

    #[test]
    fn required_host_type_returns_value_when_set() {
        let cfg = make_cfg("my_pkg.Language", "my_pkg.NewLanguage({ptr})");
        assert_eq!(cfg.required_host_type("Language", "go").unwrap(), "my_pkg.Language");
    }

    #[test]
    fn required_host_type_errors_when_empty() {
        let cfg = make_cfg("", "my_pkg.NewLanguage({ptr})");
        let err = cfg.required_host_type("Language", "swift").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("host_type"), "error must mention the field: {msg}");
        assert!(msg.contains("Language"), "error must name the type: {msg}");
        assert!(msg.contains("swift"), "error must name the backend: {msg}");
    }

    #[test]
    fn zig_capsule_import_name_extracts_module_from_qualified_type() {
        assert_eq!(zig_capsule_import_name("?*const my_module.Language"), Some("my_module"));
    }

    #[test]
    fn zig_capsule_import_name_returns_none_for_unqualified_type() {
        assert_eq!(zig_capsule_import_name("Language"), None);
    }
}
