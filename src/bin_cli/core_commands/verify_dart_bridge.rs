//! `alef verify`'s Dart-bridge-field-drift check (alef #437).
//!
//! ~keep Own module rather than a function inline in `verify.rs`: that file sits at the repo's
//! 1,000-line cap (`file-modularization` in CLAUDE.md), and adding this there crossed it --
//! mirroring `verify_flags.rs`'s identical reason for existing as its own file.

/// `config`'s Dart-side FRB bridge (`lib.dart`), as `[<crate>] <TypeName>.<field_name>` per
/// facade field it is missing, if any.
///
/// `flutter_rust_bridge_codegen` writes `lib.dart` itself -- alef never generates it -- so unlike
/// `verify::frb_generated_drift` (which recomputes a canonical form alef's own `CarryFrbCfgGates`
/// rewrite owns) there is no fixed point to compare against here. What IS checkable read-only,
/// without shelling out to the real tool, is field coverage: does every named field
/// `packages/dart/rust/src/lib.rs` currently declares have a matching property on the bridge's
/// Dart class? [`crate::backends::dart::missing_bridge_struct_fields`] answers exactly that, and
/// is the verify-time counterpart to the generate-time `VerifyFrbBridgeCoverage` post-build step,
/// which asks the identical question about free functions (alef #135) but never ran this check
/// for struct fields at all -- see alef #437, where a public field changed and the Dart bridge
/// class silently kept its old shape because nothing, at either generate time or verify time,
/// ever looked at whether the bridge's OWN fields still matched the facade.
///
/// Empty when the crate does not target Dart's FRB style, when either file is missing (nothing
/// generated yet is not drift), or when the bridge class for a given facade struct cannot be
/// found at all -- see [`crate::backends::dart::missing_bridge_struct_fields`]'s doc for why a
/// missing class is a different, out-of-scope claim from a missing field on an existing one. See
/// `verify::run`'s call site. ~keep
pub(super) fn frb_dart_bridge_field_drift(
    config: &crate::core::config::ResolvedCrateConfig,
    base_dir: &std::path::Path,
) -> Vec<String> {
    let Some((lib_rs_path, _)) = crate::backends::dart::frb_rust_facade_paths(config) else {
        return Vec::new();
    };
    let Some(bridge_dart_path) = crate::backends::dart::frb_dart_bridge_path(config) else {
        return Vec::new();
    };
    let lib_rs_path = base_dir.join(lib_rs_path);
    let bridge_dart_path = base_dir.join(bridge_dart_path);
    let (Ok(lib_rs), Ok(bridge_dart)) = (
        std::fs::read_to_string(&lib_rs_path),
        std::fs::read_to_string(&bridge_dart_path),
    ) else {
        return Vec::new();
    };

    let exclude_types = config
        .dart
        .as_ref()
        .map(|dart| dart.exclude_types.clone())
        .unwrap_or_default();
    crate::backends::dart::missing_bridge_struct_fields(&lib_rs, &bridge_dart, &exclude_types)
        .into_iter()
        .map(|missing_field| format!("[{}] {} ({})", config.name, missing_field, bridge_dart_path.display()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::frb_dart_bridge_field_drift;

    /// A minimal single-crate, Dart-targeting config -- same fixture shape as
    /// `verify::frb_generated_drift_tests::dart_config`, duplicated locally rather than shared so
    /// each drift check's test module stays self-contained.
    fn dart_config(name: &str) -> crate::core::config::ResolvedCrateConfig {
        let toml_text = format!(
            "[workspace]\nlanguages = [\"dart\"]\n\n[[crates]]\nname = \"{name}\"\nsources = [\"src/lib.rs\"]\n"
        );
        let cfg: crate::core::config::new_config::NewAlefConfig =
            toml::from_str(&toml_text).expect("test config must parse");
        cfg.resolve().expect("test config must resolve").remove(0)
    }

    /// Writes `lib.rs` (the facade) and `lib.dart` (the bridge) at the exact paths
    /// `frb_rust_facade_paths`/`frb_dart_bridge_path` compute for `config`, rooted under
    /// `base_dir`.
    fn write_bridge_fixture(
        config: &crate::core::config::ResolvedCrateConfig,
        base_dir: &std::path::Path,
        lib_rs: &str,
        lib_dart: &str,
    ) {
        let (lib_rs_path, _) =
            crate::backends::dart::frb_rust_facade_paths(config).expect("dart FRB style always has facade paths");
        let bridge_dart_path =
            crate::backends::dart::frb_dart_bridge_path(config).expect("dart FRB style always has a bridge path");
        let lib_rs_path = base_dir.join(lib_rs_path);
        let bridge_dart_path = base_dir.join(bridge_dart_path);
        std::fs::create_dir_all(lib_rs_path.parent().expect("facade path has a parent dir"))
            .expect("create facade src dir");
        std::fs::create_dir_all(bridge_dart_path.parent().expect("bridge path has a parent dir"))
            .expect("create bridge dir");
        std::fs::write(&lib_rs_path, lib_rs).expect("write facade lib.rs");
        std::fs::write(&bridge_dart_path, lib_dart).expect("write bridge lib.dart");
    }

    const PAGE_RESULT_LIB_RS: &str = "\
#[frb(mirror(CrawlPageResult))]
pub struct CrawlPageResult {
    /// The page URL.
    pub url: String,
    /// Redirect hops taken to reach `final_url` from `url`.
    pub redirect_count: i64,
}
";

    /// alef #437's reported shape: a public field (`redirect_count`) exists in the current
    /// facade but the committed bridge class predates it -- exactly what a skipped
    /// `flutter_rust_bridge_codegen` run leaves behind.
    #[test]
    fn flags_a_bridge_class_missing_a_facade_field_as_drift() {
        let config = dart_config("sample-lib");
        let base_dir = tempfile::tempdir().expect("tempdir");
        let stale_bridge = "\
class CrawlPageResult {
  final String url;

  const CrawlPageResult({required this.url});
}
";
        write_bridge_fixture(&config, base_dir.path(), PAGE_RESULT_LIB_RS, stale_bridge);

        let drift = frb_dart_bridge_field_drift(&config, base_dir.path());
        assert_eq!(
            drift.len(),
            1,
            "a bridge class missing a field the facade declares must be reported as drift: {drift:?}"
        );
        assert!(
            drift[0].contains("CrawlPageResult.redirect_count"),
            "the report must name the missing field: {drift:?}"
        );
    }

    /// THE CONTROL. Once the bridge class already declares every facade field, nothing may be
    /// reported -- otherwise every up-to-date consumer repo would fail `alef verify` on this
    /// check forever.
    #[test]
    fn is_silent_once_the_bridge_class_has_every_facade_field() {
        let config = dart_config("sample-lib");
        let base_dir = tempfile::tempdir().expect("tempdir");
        let up_to_date_bridge = "\
class CrawlPageResult {
  final String url;

  /// Redirect hops taken to reach `final_url` from `url`.
  final PlatformInt64 redirectCount;

  const CrawlPageResult({required this.url, required this.redirectCount});
}
";
        write_bridge_fixture(&config, base_dir.path(), PAGE_RESULT_LIB_RS, up_to_date_bridge);

        let drift = frb_dart_bridge_field_drift(&config, base_dir.path());
        assert!(
            drift.is_empty(),
            "an up-to-date bridge class must not be reported as drift: {drift:?}"
        );
    }
}
