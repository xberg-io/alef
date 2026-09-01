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

/// Ownership contract for the native pointer wrapped by a host capsule.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointerOwnership {
    /// The host wrapper owns and may destroy the pointer.
    #[default]
    Owned,
    /// The pointer has static lifetime and must never be destroyed by the host wrapper.
    BorrowedStatic,
    /// The pointer participates in a reference-counted ownership protocol.
    Refcounted,
    /// The pointer belongs to a WebAssembly runtime rather than the host native runtime.
    Wasm,
}

/// Destructor behavior of the host wrapper around a capsule pointer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostDestructor {
    /// The wrapper invokes a destructor from the same native runtime.
    #[default]
    SharedRuntime,
    /// The wrapper has no destructor path for the pointer.
    None,
    /// The wrapper invokes an ABI-stable destructor that is explicitly a no-op.
    AbiNoop,
}

/// Host-native capsule config for a single type in one C-ABI family backend.
///
/// TOML form (Go example):
/// ```toml
/// [crates.go.capsule_types.Language]
/// host_type = "*my_pkg.Language"
/// package = "github.com/example/go-my-lib"
/// package_version = "v1.0.0"
/// construct_expr = "my_pkg.NewLanguage(unsafe.Pointer({ptr}))"
/// pointer_ownership = "borrowed_static"
/// host_destructor = "none"
/// abi_compatible = true
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
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
    /// Ownership contract for the native pointer. Defaults to `owned`, which requires a
    /// shared native runtime unless a stronger safe contract is declared.
    #[serde(default)]
    pub pointer_ownership: PointerOwnership,
    /// Destructor behavior of the host wrapper. Defaults to `shared_runtime`.
    #[serde(default)]
    pub host_destructor: HostDestructor,
    /// Affirms that the host wrapper's pointer representation is ABI-compatible with the
    /// pointer returned by Alef's FFI boundary.
    #[serde(default)]
    pub abi_compatible: bool,
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

pub fn require_shared_native_runtime(
    capsule_types: &std::collections::HashMap<String, HostCapsuleTypeConfig>,
    shares_native_runtime: bool,
    backend: &str,
) -> Result<(), anyhow::Error> {
    if shares_native_runtime || capsule_types.is_empty() {
        return Ok(());
    }

    let mut unsafe_capsules: Vec<_> = capsule_types
        .iter()
        .filter_map(|(type_name, config)| borrowed_static_contract_error(type_name, config))
        .collect();
    if unsafe_capsules.is_empty() {
        return Ok(());
    }
    unsafe_capsules.sort_unstable();
    anyhow::bail!(
        "capsule configuration in backend `{backend}` cannot safely wrap native pointers: {}; \
         declare a complete borrowed-static ABI-compatible no-destructor contract for every listed capsule, \
         or set `[crates.{backend}].shares_native_runtime = true` when every host wrapper shares one native \
         runtime and ownership contract",
        unsafe_capsules.join("; ")
    )
}

fn borrowed_static_contract_error(type_name: &str, config: &HostCapsuleTypeConfig) -> Option<String> {
    let mut reasons = Vec::new();
    if config.pointer_ownership != PointerOwnership::BorrowedStatic {
        reasons.push("`pointer_ownership = \"borrowed_static\"` is required");
    }
    if !config.abi_compatible {
        reasons.push("`abi_compatible = true` is required");
    }
    if !matches!(config.host_destructor, HostDestructor::None | HostDestructor::AbiNoop) {
        reasons.push("`host_destructor = \"none\"` or `host_destructor = \"abi_noop\"` is required");
    }
    (!reasons.is_empty()).then(|| format!("capsule type `{type_name}`: {}", reasons.join(", ")))
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
    let qualified = host_type.split_whitespace().find(|token| token.contains('.'))?;
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
mod capsule_backend_coverage_tests {
    use crate::core::backend::Backend;
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;

    const UNDECLARED_CAPSULE: &str = "[capsule_types.Language]\nhost_type = \"Placeholder\"\n";

    const GATE_ERROR: &str = "cannot safely wrap native pointers";

    fn go_config(shares_native_runtime: bool) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            go: Some(parse_language_config(UNDECLARED_CAPSULE, shares_native_runtime)),
            ..Default::default()
        }
    }

    fn swift_config(shares_native_runtime: bool) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            swift: Some(parse_language_config(UNDECLARED_CAPSULE, shares_native_runtime)),
            ..Default::default()
        }
    }

    fn zig_config(shares_native_runtime: bool) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            zig: Some(parse_language_config(UNDECLARED_CAPSULE, shares_native_runtime)),
            ..Default::default()
        }
    }

    fn parse_language_config<T: serde::de::DeserializeOwned>(capsule: &str, shares_native_runtime: bool) -> T {
        let toml = format!("shares_native_runtime = {shares_native_runtime}\n{capsule}");
        toml::from_str(&toml).expect("language config fixture parses")
    }

    fn gate_error_of(result: anyhow::Result<Vec<crate::core::backend::GeneratedFile>>) -> Option<String> {
        result
            .err()
            .map(|error| error.to_string())
            .filter(|m| m.contains(GATE_ERROR))
    }

    #[test]
    fn go_rejects_an_undeclared_capsule_contract() {
        let backend = crate::backends::go::GoBackend;
        let result = backend.generate_bindings(&ApiSurface::default(), &go_config(false));
        assert!(
            gate_error_of(result).is_some(),
            "the go backend must enforce the capsule gate"
        );
    }

    #[test]
    fn swift_rejects_an_undeclared_capsule_contract() {
        let backend = crate::backends::swift::SwiftBackend;
        let result = backend.generate_bindings(&ApiSurface::default(), &swift_config(false));
        assert!(
            gate_error_of(result).is_some(),
            "the swift backend must enforce the capsule gate"
        );
    }

    #[test]
    fn zig_rejects_an_undeclared_capsule_contract() {
        let backend = crate::backends::zig::ZigBackend;
        let result = backend.generate_bindings(&ApiSurface::default(), &zig_config(false));
        assert!(
            gate_error_of(result).is_some(),
            "the zig backend must enforce the capsule gate"
        );
    }

    #[test]
    fn shares_native_runtime_clears_the_gate_for_every_newly_gated_backend() {
        let go = crate::backends::go::GoBackend.generate_bindings(&ApiSurface::default(), &go_config(true));
        let swift = crate::backends::swift::SwiftBackend.generate_bindings(&ApiSurface::default(), &swift_config(true));
        let zig = crate::backends::zig::ZigBackend.generate_bindings(&ApiSurface::default(), &zig_config(true));
        assert_eq!(gate_error_of(go), None, "go ignored `shares_native_runtime = true`");
        assert_eq!(
            gate_error_of(swift),
            None,
            "swift ignored `shares_native_runtime = true`"
        );
        assert_eq!(gate_error_of(zig), None, "zig ignored `shares_native_runtime = true`");
    }
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
            ..Default::default()
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
    fn shared_runtime_contract_is_explicit_and_contextual() {
        let capsule_types =
            std::collections::HashMap::from([("Language".to_string(), make_cfg("Language", "new Language({ptr})"))]);
        let error = require_shared_native_runtime(&capsule_types, false, "java").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("[crates.java].shares_native_runtime = true"));
        assert!(message.contains("Language"));
        assert!(message.contains("borrowed-static ABI-compatible no-destructor contract"));
        assert!(message.contains("shares one native runtime and ownership contract"));

        require_shared_native_runtime(&capsule_types, true, "java").unwrap();
    }

    #[test]
    fn defaults_require_shared_native_runtime() {
        let cfg: HostCapsuleTypeConfig = toml::from_str(r#"host_type = "Language""#).unwrap();
        assert_eq!(cfg.pointer_ownership, PointerOwnership::Owned);
        assert_eq!(cfg.host_destructor, HostDestructor::SharedRuntime);
        assert!(!cfg.abi_compatible);

        let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);
        let message = require_shared_native_runtime(&capsule_types, false, "java")
            .unwrap_err()
            .to_string();
        assert!(message.contains("Language"));
        assert!(message.contains("pointer_ownership = \"borrowed_static\""));
    }

    #[test]
    fn borrowed_static_without_destructor_and_abi_compatible_is_safe_without_shared_runtime() {
        let cfg: HostCapsuleTypeConfig = toml::from_str(
            r#"
host_type = "Language"
pointer_ownership = "borrowed_static"
host_destructor = "none"
abi_compatible = true
"#,
        )
        .unwrap();
        let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);

        require_shared_native_runtime(&capsule_types, false, "java").unwrap();
    }

    #[test]
    fn borrowed_static_with_abi_noop_destructor_is_safe_without_shared_runtime() {
        let mut cfg = make_cfg("Language", "new Language({ptr})");
        cfg.pointer_ownership = PointerOwnership::BorrowedStatic;
        cfg.host_destructor = HostDestructor::AbiNoop;
        cfg.abi_compatible = true;
        let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);

        require_shared_native_runtime(&capsule_types, false, "csharp").unwrap();
    }

    #[test]
    fn borrowed_static_without_abi_compatibility_is_rejected() {
        let mut cfg = make_cfg("Language", "new Language({ptr})");
        cfg.pointer_ownership = PointerOwnership::BorrowedStatic;
        cfg.host_destructor = HostDestructor::None;
        let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);

        let message = require_shared_native_runtime(&capsule_types, false, "java")
            .unwrap_err()
            .to_string();
        assert!(message.contains("abi_compatible = true"));
    }

    #[test]
    fn shared_runtime_destructor_is_rejected_without_shared_runtime() {
        let mut cfg = make_cfg("Language", "new Language({ptr})");
        cfg.pointer_ownership = PointerOwnership::BorrowedStatic;
        cfg.abi_compatible = true;
        let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);

        let message = require_shared_native_runtime(&capsule_types, false, "java")
            .unwrap_err()
            .to_string();
        assert!(message.contains("host_destructor = \"none\"") || message.contains("host_destructor = \"abi_noop\""));
    }

    #[test]
    fn unsafe_mixed_map_names_only_the_unsafe_capsule() {
        let mut safe = make_cfg("Language", "new Language({ptr})");
        safe.pointer_ownership = PointerOwnership::BorrowedStatic;
        safe.host_destructor = HostDestructor::None;
        safe.abi_compatible = true;
        let unsafe_capsule = make_cfg("Query", "new Query({ptr})");
        let capsule_types =
            std::collections::HashMap::from([("Language".to_string(), safe), ("Query".to_string(), unsafe_capsule)]);

        let message = require_shared_native_runtime(&capsule_types, false, "java")
            .unwrap_err()
            .to_string();
        assert!(message.contains("Query"));
        assert!(!message.contains("capsule type `Language`"));
    }

    #[test]
    fn all_ownership_contract_values_parse_and_unsafe_values_fail_closed() {
        for ownership in ["owned", "refcounted", "wasm"] {
            let cfg: HostCapsuleTypeConfig = toml::from_str(&format!(
                r#"
host_type = "Language"
pointer_ownership = "{ownership}"
host_destructor = "none"
abi_compatible = true
"#
            ))
            .unwrap();
            let capsule_types = std::collections::HashMap::from([("Language".to_string(), cfg)]);
            let message = require_shared_native_runtime(&capsule_types, false, "java")
                .unwrap_err()
                .to_string();
            assert!(
                message.contains("borrowed_static"),
                "unexpected error for {ownership}: {message}"
            );
        }
    }

    #[test]
    fn shares_native_runtime_preserves_legacy_capsule_acceptance() {
        let capsule_types =
            std::collections::HashMap::from([("Language".to_string(), make_cfg("Language", "new Language({ptr})"))]);

        require_shared_native_runtime(&capsule_types, true, "java").unwrap();
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
