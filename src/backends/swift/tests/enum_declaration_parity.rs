//! Cross-surface parity for a cfg-gated enum variant: the public Swift `enum`'s `case` list and
//! the swift-bridge mirror enum's variant list must always name the same variants.
//!
//! These assertions deliberately compare the TWO EMITTED SURFACES to each other rather than to a
//! hardcoded expected string. A test that pins each side to its own literal passes just as
//! happily when both sides drift together, which is the failure mode this file exists to catch:
//! `gen_rust_crate::enums::emit_enum_wrapper` consulted
//! `codegen::conversions::enum_variant_declaration` and dropped a proven-unreachable FOREIGN
//! cfg-gated variant, while `gen_bindings::enums::emit_enum` walked `en.variants` with no cfg
//! awareness at all and emitted a Swift `case` for it. The facade advertised `.extra` even though
//! the mirror's `to_string` match (built from the mirror's own variant list) could never produce
//! it and its `__alef_*_from_swift_string` arm — dropped unconditionally for any foreign
//! cfg-gated variant — could never accept it.
//!
//! Both surfaces come out of a single `SwiftBackend::generate_bindings` call, so nothing here can
//! diverge from what a real `alef build` writes.

use crate::backends::swift::SwiftBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

const ENUM_NAME: &str = "RoutingStrategy";
const GATING_FEATURE: &str = "extra-tier";

fn swift_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.swift]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A first path segment other than the crate's own name (`test_lib`) is what
/// `is_host_owned_rust_path` reads to classify this enum — and its cfg-gated variant — as FOREIGN.
/// `has_serde` is what routes the facade into the native-Swift-enum branch of `emit_enum` rather
/// than the `typealias` shortcut, so the `case` list actually exists to compare. ~keep
fn foreign_cfg_enum_api(rust_path: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: ENUM_NAME.to_string(),
            rust_path: rust_path.to_string(),
            has_serde: true,
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extra".to_string(),
                    cfg: Some(format!(r#"feature = "{GATING_FEATURE}""#)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The one generated file of extension `suffix` that declares `marker`.
///
/// Selected by content, not by filename: `generate_bindings` returns several `.swift` files and
/// more than one `.rs`, and picking the first by path would silently look at the wrong surface. ~keep
fn file_declaring<'a>(files: &'a [GeneratedFile], suffix: &str, marker: &str) -> &'a str {
    let matches: Vec<&GeneratedFile> = files
        .iter()
        .filter(|f| f.path.to_string_lossy().ends_with(suffix) && f.content.contains(marker))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one generated {suffix} file containing {marker:?}, found {} ({:?})",
        matches.len(),
        files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>()
    );
    &matches[0].content
}

/// Every variant name the swift-bridge mirror `pub enum {ENUM_NAME} { ... }` declares, in order.
fn mirror_variants(lib_rs: &str) -> Vec<String> {
    let header = format!("pub enum {ENUM_NAME} {{\n");
    let start = lib_rs
        .find(&header)
        .map(|i| i + header.len())
        .unwrap_or_else(|| panic!("bridge lib.rs must declare the mirror enum, got:\n{lib_rs}"));
    let body = &lib_rs[start..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("mirror enum declaration must close, got:\n{body}"));
    body[..end]
        .lines()
        .map(|line| line.trim().trim_end_matches(',').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Every `case` name the public Swift `enum {ENUM_NAME}` declares, in order. Handles both the
/// bare `case x` and the `case x = "Wire"` raw-value spellings `emit_enum` chooses between.
fn swift_cases(swift_src: &str) -> Vec<String> {
    let header = format!("public enum {ENUM_NAME}: String");
    let start = swift_src
        .find(&header)
        .unwrap_or_else(|| panic!("Swift source must declare the public enum, got:\n{swift_src}"));
    let body = &swift_src[start..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("Swift enum declaration must close, got:\n{body}"));
    body[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("case "))
        .map(|rest| {
            rest.split('=')
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('`')
                .to_string()
        })
        .collect()
}

/// The Swift spelling `emit_enum` derives a `case` name from, so the two lists can be compared as
/// the same set of variants rather than as raw strings.
fn as_swift_case(variant_name: &str) -> String {
    crate::codegen::naming::public_host_identifier(
        crate::core::config::Language::Swift,
        crate::codegen::naming::PublicIdentifierKind::EnumVariant,
        variant_name,
    )
    .trim_matches('`')
    .to_string()
}

fn emitted_surfaces(configured_feature: Option<&str>, rust_path: &str) -> (Vec<String>, Vec<String>) {
    let api = foreign_cfg_enum_api(rust_path);
    let config = swift_config_with_feature(configured_feature);
    let files = SwiftBackend.generate_bindings(&api, &config).unwrap();
    let mirror_header = format!("pub enum {ENUM_NAME} {{\n");
    let swift_header = format!("public enum {ENUM_NAME}: String");
    let mirror = mirror_variants(file_declaring(&files, ".rs", &mirror_header));
    let cases = swift_cases(file_declaring(&files, ".swift", &swift_header));
    (mirror, cases)
}

fn assert_surfaces_agree(configured_feature: Option<&str>, rust_path: &str) -> Vec<String> {
    let (mirror, cases) = emitted_surfaces(configured_feature, rust_path);
    let expected: Vec<String> = mirror.iter().map(|v| as_swift_case(v)).collect();
    assert_eq!(
        cases, expected,
        "the public Swift enum's `case` list and the swift-bridge mirror enum's variant list must \
         name the same variants; mirror declared {mirror:?}, Swift declared {cases:?}"
    );
    mirror
}

/// The gating feature is NOT configured, so the dependency never compiles `Extra` in: the mirror
/// drops it, and the Swift facade must drop it too.
#[test]
fn swift_case_list_matches_mirror_when_foreign_variant_is_proven_unreachable() {
    let mirror = assert_surfaces_agree(None, "dep_crate::RoutingStrategy");
    assert_eq!(
        mirror,
        vec!["Primary".to_string()],
        "sanity check on the fixture itself: with the gating feature off the mirror must have \
         dropped the foreign variant, otherwise this test proves nothing"
    );
}

/// Positive control: with the gating feature configured the foreign variant is NOT proven
/// unreachable, so BOTH surfaces must keep it. Without this, dropping every cfg-gated variant
/// from the Swift facade would also satisfy the test above.
#[test]
fn swift_case_list_matches_mirror_when_foreign_variant_is_reachable() {
    let mirror = assert_surfaces_agree(Some(GATING_FEATURE), "dep_crate::RoutingStrategy");
    assert_eq!(
        mirror,
        vec!["Primary".to_string(), "Extra".to_string()],
        "with the gating feature configured the foreign variant is not proven unreachable, so the \
         mirror must still declare it"
    );
}

/// A HOST-owned cfg-gated variant is never dropped by `enum_variant_declaration` — the wrapper
/// crate declares the feature itself and defers to the compiler. Both surfaces must keep it even
/// with the feature off, so this fix cannot be mistaken for "drop every cfg-gated variant". ~keep
#[test]
fn swift_case_list_matches_mirror_for_a_host_owned_cfg_gated_variant() {
    let mirror = assert_surfaces_agree(None, "test_lib::RoutingStrategy");
    assert_eq!(
        mirror,
        vec!["Primary".to_string(), "Extra".to_string()],
        "a host-owned cfg-gated variant must stay on the mirror declaration regardless of the \
         configured feature set"
    );
}
