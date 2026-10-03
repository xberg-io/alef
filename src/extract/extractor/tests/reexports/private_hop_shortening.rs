use super::*;

/// Reproduces alef issue #466: a type declared behind a chain of PRIVATE modules must be
/// emitted at the shallowest path that is public at every segment (here, the crate root),
/// never at its private declaration path.
///
/// Fixture layout mirrors the reported consumer crate exactly:
///
/// ```text
/// src/lib.rs            mod types;              (private)
///                        pub use types::{Reexported};
/// src/types/mod.rs       mod config;             (private)
///                        pub use config::{Reexported};
/// src/types/config.rs    mod primitives;         (private, 2018 sibling-file layout)
///                        pub use primitives::{Reexported};
/// src/types/config/primitives.rs
///                        pub struct Reexported { .. }
/// ```
///
/// Contrasted, in the same fixture, with the PUBLIC-hop shape that already works (mirrors
/// `net/ssrf.rs` from the report):
///
/// ```text
/// src/net/mod.rs         pub mod ssrf;           (PUBLIC)
/// src/net/ssrf.rs         mod policy;             (private)
///                        pub use policy::{FlatOk};
/// src/net/ssrf/policy.rs pub struct FlatOk { .. }
/// ```
///
/// Both `config.rs` and `ssrf.rs` are 2018-edition "sibling" module files: each owns a
/// same-named subdirectory (`config/`, `ssrf/`) holding its own nested `mod` declarations,
/// rather than a `mod.rs` inside that directory. This is the shape that
/// `extract_module`'s external-file candidate search resolved against the wrong base
/// directory (see `paths::module_children_dir`), silently failing to find the nested file at
/// all -- not merely emitting the wrong path for it.
fn write_private_hop_fixture(tmp: &std::path::Path) {
    std::fs::create_dir_all(tmp.join("src/types/config")).unwrap();
    std::fs::create_dir_all(tmp.join("src/net/ssrf")).unwrap();

    std::fs::write(
        tmp.join("src/lib.rs"),
        r#"
mod types;
pub use types::Reexported;

pub mod net;
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/types/mod.rs"),
        r#"
mod config;
pub use config::Reexported;
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/types/config.rs"),
        r#"
mod primitives;
pub use primitives::Reexported;
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/types/config/primitives.rs"),
        r#"
pub struct Reexported {
    pub value: u32,
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/net/mod.rs"),
        r#"
pub mod ssrf;
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/net/ssrf.rs"),
        r#"
mod policy;
pub use policy::FlatOk;
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.join("src/net/ssrf/policy.rs"),
        r#"
pub struct FlatOk {
    pub allowed: bool,
}
"#,
    )
    .unwrap();
}

#[test]
fn test_private_module_chain_shortens_to_crate_root_public_reexport() {
    let tmp = std::env::temp_dir().join("alef_test_private_hop_chain_466");
    let _ = std::fs::remove_dir_all(&tmp);
    write_private_hop_fixture(&tmp);

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "crawlberg_fixture", "0.1.0", None).unwrap();

    let reexported = surface.types.iter().find(|t| t.name == "Reexported").expect(
        "Reexported must still be extracted: it is declared 3 private-module hops deep in a \
             2018-edition sibling-file layout (types.rs analog -> config.rs -> config/primitives.rs), \
             which extract_module's external-file candidate search previously resolved against the \
             wrong base directory and so never found at all",
    );
    assert_eq!(
        reexported.rust_path, "crawlberg_fixture::Reexported",
        "a type reached only through a chain of private modules, each bridged by the enclosing \
         module's own `pub use`, must be emitted at its shallowest path that is public at every \
         segment (the crate root), never at its private declaration path; got {}",
        reexported.rust_path
    );

    // The public-hop shape (mirrors `net/ssrf.rs`) must stay correct and unaffected by the fix:
    // `net` and `ssrf` are both genuinely `pub mod`, so `net::ssrf::FlatOk` is already a fully
    // public, reachable path -- it only needs the private `policy` segment dropped.
    let flat_ok = surface
        .types
        .iter()
        .find(|t| t.name == "FlatOk")
        .expect("FlatOk must still be extracted from the same sibling-file layout under a public mod chain");
    assert_eq!(
        flat_ok.rust_path, "crawlberg_fixture::net::ssrf::FlatOk",
        "a re-export reached through PUBLIC module hops must keep shortening exactly as before \
         (dropping only the bridged private `policy` segment); got {}",
        flat_ok.rust_path
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Isolates the previously-broken private-hop chain from the fixture above, independent of the
/// contrasting public-hop branch, so a regression in one shape cannot hide inside the other.
#[test]
fn test_types_config_primitives_chain_alone_reaches_crate_root() {
    let tmp = std::env::temp_dir().join("alef_test_private_hop_types_only_466");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/types/config")).unwrap();
    std::fs::write(tmp.join("src/lib.rs"), "mod types;\npub use types::Reexported;\n").unwrap();
    std::fs::write(
        tmp.join("src/types/mod.rs"),
        "mod config;\npub use config::Reexported;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/types/config.rs"),
        "mod primitives;\npub use primitives::Reexported;\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/types/config/primitives.rs"),
        "pub struct Reexported { pub value: u32 }\n",
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "types_only_fixture", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1, "exactly one type should be extracted");
    assert_eq!(surface.types[0].rust_path, "types_only_fixture::Reexported");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Isolates the public-hop, sibling-file chain (mirrors `net/ssrf.rs`) alone.
#[test]
fn test_net_ssrf_policy_chain_alone_reaches_public_path() {
    let tmp = std::env::temp_dir().join("alef_test_private_hop_net_only_466");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src/net/ssrf")).unwrap();
    std::fs::write(tmp.join("src/lib.rs"), "pub mod net;\n").unwrap();
    std::fs::write(tmp.join("src/net/mod.rs"), "pub mod ssrf;\n").unwrap();
    std::fs::write(tmp.join("src/net/ssrf.rs"), "mod policy;\npub use policy::FlatOk;\n").unwrap();
    std::fs::write(
        tmp.join("src/net/ssrf/policy.rs"),
        "pub struct FlatOk { pub allowed: bool }\n",
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "net_only_fixture", "0.1.0", None).unwrap();

    assert_eq!(surface.types.len(), 1, "exactly one type should be extracted");
    assert_eq!(surface.types[0].rust_path, "net_only_fixture::net::ssrf::FlatOk");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn private_module_error_reexport_uses_public_path_in_php_converter() {
    let tmp = std::env::temp_dir().join("alef_test_private_error_reexport");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src/lib.rs"), "mod faults;\npub use faults::SampleError;\n").unwrap();
    std::fs::write(
        tmp.join("src/faults.rs"),
        r#"
#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("invalid sample")]
    InvalidSample,
}
"#,
    )
    .unwrap();

    let lib_rs = tmp.join("src/lib.rs");
    let sources: Vec<&std::path::Path> = vec![lib_rs.as_path()];
    let surface = super::extract(&sources, "sample_core", "0.1.0", None).unwrap();

    assert_eq!(surface.errors.len(), 1, "exactly one error should be extracted");
    let error = &surface.errors[0];
    assert_eq!(error.rust_path, "sample_core::SampleError");

    let converter = crate::codegen::error_gen::gen_php_error_converter(error, "sample_core");
    assert!(
        converter.contains("fn sample_error_to_php_err(e: sample_core::SampleError)"),
        "converter must accept the public re-export path: {converter}"
    );
    assert!(
        converter.contains("sample_core::SampleError::InvalidSample"),
        "converter patterns must use the public re-export path: {converter}"
    );
    assert!(
        !converter.contains("sample_core::faults::SampleError"),
        "converter must not expose the private declaration module: {converter}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Direct unit coverage for `paths::validate_no_private_path_leaks`: proves the loud-failure
/// path actually fires (rather than silently letting an unreachable path through) when an
/// item's `rust_path` still runs through a module recorded as private, and proves it stays
/// silent for paths that never cross a recorded-private segment.
#[test]
fn test_validate_no_private_path_leaks_fires_for_unreachable_path() {
    let mut surface = ApiSurface {
        crate_name: "leaky_crate".into(),
        ..ApiSurface::default()
    };
    surface.types.push(TypeDef {
        name: "Unreachable".into(),
        rust_path: "leaky_crate::config::primitives::Unreachable".into(),
        ..Default::default()
    });

    let mut private_module_paths = ahash::AHashSet::new();
    private_module_paths.insert("config::primitives".to_string());

    let err = paths::validate_no_private_path_leaks(&surface, "leaky_crate", &private_module_paths)
        .expect_err("a rust_path that still runs through a recorded-private module must be rejected");
    let message = err.to_string();
    assert!(
        message.contains("leaky_crate::config::primitives::Unreachable"),
        "error must name the offending path; got: {message}"
    );
    assert!(
        message.contains("config::primitives"),
        "error must name the offending private module; got: {message}"
    );
}

#[test]
fn test_validate_no_private_path_leaks_is_silent_for_public_path() {
    let mut surface = ApiSurface {
        crate_name: "clean_crate".into(),
        ..ApiSurface::default()
    };
    surface.types.push(TypeDef {
        name: "Reachable".into(),
        rust_path: "clean_crate::Reachable".into(),
        ..Default::default()
    });
    surface.types.push(TypeDef {
        name: "AlsoReachable".into(),
        rust_path: "clean_crate::net::ssrf::AlsoReachable".into(),
        ..Default::default()
    });

    // "config::primitives" was private in some other extraction; neither path above runs
    // through it, so it must not cause a false positive here.
    let mut private_module_paths = ahash::AHashSet::new();
    private_module_paths.insert("config::primitives".to_string());

    paths::validate_no_private_path_leaks(&surface, "clean_crate", &private_module_paths)
        .expect("a path that never crosses a recorded-private module must not be rejected");
}
