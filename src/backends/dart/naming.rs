//! Dart-specific naming helpers for `ResolvedCrateConfig`.

use crate::core::config::{DartStyle, ResolvedCrateConfig};
use crate::core::template_versions;

/// Get the Dart bridging style (`frb` or `ffi`).
pub fn dart_style(config: &ResolvedCrateConfig) -> DartStyle {
    config.dart.as_ref().map(|d| d.style).unwrap_or_default()
}

/// Get the flutter_rust_bridge version to pin.
///
/// Returns the per-crate override from `[crates.dart] frb_version` when set;
/// otherwise falls back to the compiled-in default constant.
pub fn dart_frb_version(config: &ResolvedCrateConfig) -> String {
    config
        .dart
        .as_ref()
        .and_then(|d| d.frb_version.as_ref())
        .cloned()
        .unwrap_or_else(|| template_versions::cargo::FLUTTER_RUST_BRIDGE.to_string())
}

// ~keep FRB owns these declarations and preserves Rust type names, including Dart core names.
pub(crate) fn frb_dart_type_name(name: &str) -> String {
    crate::codegen::naming::public_casing(
        crate::core::config::Language::Dart,
        crate::codegen::naming::PublicIdentifierKind::Type,
        name,
    )
}

pub(crate) fn dart_error_variant_class(error_name: &str, variant_name: &str) -> String {
    format!(
        "{}_{}",
        frb_dart_type_name(error_name),
        frb_dart_type_name(variant_name)
    )
}

pub(crate) fn dart_error_is_simple(error: &crate::core::ir::ErrorDef) -> bool {
    error.variants.iter().all(|variant| {
        variant.is_unit || (!variant.is_tuple && variant.fields.iter().all(|field| field.binding_excluded))
    })
}

pub(crate) fn dart_simple_error_value(error_name: &str, variant_name: &str) -> String {
    use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
    use crate::core::config::Language;
    format!(
        "{}.{}",
        frb_dart_type_name(error_name),
        public_host_identifier(Language::Dart, PublicIdentifierKind::Method, variant_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::new_config::NewAlefConfig;
    use crate::core::template_versions;

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn error_references_preserve_the_type_names_owned_by_frb() {
        for name in ["Error", "Object", "List", "StateError", "RDFaChunk", "SampleError"] {
            assert_eq!(
                dart_error_variant_class(name, "LanguageNotFound"),
                format!("{name}_LanguageNotFound")
            );
            assert_eq!(dart_simple_error_value(name, "Missing"), format!("{name}.missing"));
        }
        assert_eq!(dart_error_variant_class("SampleError", "Error"), "SampleError_Error");
    }

    #[test]
    fn dart_style_defaults_to_frb() {
        let r = minimal();
        assert_eq!(dart_style(&r), DartStyle::Frb);
    }

    #[test]
    fn dart_style_explicit_ffi() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.dart]
style = "ffi"
"#,
        );
        assert_eq!(dart_style(&r), DartStyle::Ffi);
    }

    #[test]
    fn dart_frb_version_defaults_to_constant() {
        let r = minimal();
        assert_eq!(
            dart_frb_version(&r),
            template_versions::cargo::FLUTTER_RUST_BRIDGE,
            "should fall back to compiled-in default"
        );
    }

    #[test]
    fn dart_frb_version_explicit_override_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.dart]
frb_version = "2.9.0"
"#,
        );
        assert_eq!(
            dart_frb_version(&r),
            "2.9.0",
            "explicit frb_version override should win"
        );
    }
}
