use super::write_format_config_prepass;
use crate::core::config::{Language, ResolvedCrateConfig};

/// The two claims this pre-pass exists to satisfy: it writes `rustfmt.toml` (so
/// `format_rust_content`'s bounded lookup has something real to find ahead of the bindings
/// stage), and it deliberately leaves `poly.toml` alone (that file is a merge target the real
/// scaffold stage owns -- see this module's doc for why writing it here would record a merge
/// baseline twice).
///
/// Also proves the "silent no-op later" design claim directly: writing the exact same
/// `rustfmt.toml` content through the real scaffold writer a second time must change nothing,
/// which is what makes the real scaffold stage's later write of the identical content a no-op
/// rather than a second, redundant write.
#[test]
fn prepass_writes_rustfmt_toml_and_not_poly_toml() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let config = ResolvedCrateConfig::default();
    let languages = [Language::Rust];

    let report = write_format_config_prepass(&config, &languages, base).expect("prepass write");

    assert!(
        base.join("rustfmt.toml").is_file(),
        "the pre-pass must write rustfmt.toml ahead of the real scaffold stage"
    );
    assert!(
        !base.join("poly.toml").exists(),
        "the pre-pass must never write poly.toml -- it is a merge target the real scaffold \
         stage owns, and pre-writing it would record a merge baseline twice in the same run"
    );
    assert_eq!(
        report.changed_count(),
        1,
        "exactly one file (rustfmt.toml) is expected to change on a clean tree with no C FFI \
         target configured (no .clang-format), got: {:?}",
        report.changed_paths
    );

    let second_report = write_format_config_prepass(&config, &languages, base).expect("second prepass write");
    assert_eq!(
        second_report.changed_count(),
        0,
        "writing the identical rustfmt.toml content a second time must be a silent no-op -- \
         this is what makes the real scaffold stage's later write of the same content a no-op \
         too, instead of a second, redundant write"
    );
}
