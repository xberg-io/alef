//! Regression coverage for the swift backend's package-root derivation.
//!
//! `SwiftBackend::generate_bindings` used to pick the SwiftPM package root (where the shared
//! `Sources/RustBridge{,C}` targets live) by walking the resolved output dir's ancestors for one
//! that already had a `Sources/` directory ON DISK, so the same `(IR, config)` input could resolve
//! to a different package root depending on what else had run before it -- observed in practice as
//! `Sources/RustBridge/...` on one call and `packages/swift/Sources/RustBridge/...` on the next,
//! while a concurrent scaffold test created and removed those directories. Codegen output must be a
//! pure function of `(IR, config)`; this test proves the generated paths do not move when the only
//! thing that changes between two calls is what happens to exist on disk.

use alef::backends::swift::SwiftBackend;
use alef::core::backend::Backend;
use alef::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Serializes every test in this file that mutates the process-global current directory.
/// `cargo test` runs every `#[test]` as a thread in one process, so an unguarded
/// `std::env::set_current_dir` call would race every other test in the binary.
static CWD_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that enters `dir` as the process current directory and restores the original
/// directory on drop, including when the guarded scope panics.
struct CwdGuard {
    _lock: MutexGuard<'static, ()>,
    original: PathBuf,
}

impl CwdGuard {
    fn enter(dir: &Path) -> Self {
        let lock = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let original = std::env::current_dir().expect("read current directory");
        std::env::set_current_dir(dir).expect("enter fixture directory");
        Self { _lock: lock, original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

fn resolved_swift_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.swift]
"#,
    )
    .expect("test config must parse");
    let mut config = cfg.resolve().expect("test config must resolve").remove(0);
    config.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Renderer".to_string(),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("renderer".to_string()),
        ..Default::default()
    }];
    config
}

fn api_with_options_field_trait_bridge() -> ApiSurface {
    let trait_type = TypeDef {
        name: "Renderer".to_string(),
        rust_path: "test_lib::Renderer".to_string(),
        methods: vec![MethodDef {
            name: "render_text".to_string(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            ..Default::default()
        }],
        is_trait: true,
        ..Default::default()
    };
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![trait_type],
        ..ApiSurface::default()
    }
}

/// Generates the swift bindings and returns every produced file path, sorted so the comparison is
/// over the SET of paths rather than whatever incidental order the backend returned them in.
fn generated_paths(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = SwiftBackend
        .generate_bindings(api, config)
        .expect("swift generation must succeed")
        .into_iter()
        .map(|file| file.path)
        .collect();
    paths.sort();
    paths
}

#[test]
fn generated_paths_are_identical_across_different_ambient_directory_state() {
    let api = api_with_options_field_trait_bridge();
    let config = resolved_swift_config();

    // Clean state: nothing exists on disk anywhere the old ancestor-probe could have looked.
    let clean_dir = tempfile::tempdir().expect("create clean fixture directory");
    let paths_from_clean_state = {
        let _cwd = CwdGuard::enter(clean_dir.path());
        generated_paths(&api, &config)
    };

    // Dirty state: a `packages/swift/Sources` directory already exists, e.g. left behind by a
    // previous `alef build` or a concurrent scaffold run. The old ancestor-probing logic picked a
    // DIFFERENT package root depending on exactly this kind of leftover state.
    let dirty_dir = tempfile::tempdir().expect("create dirty fixture directory");
    std::fs::create_dir_all(dirty_dir.path().join("packages/swift/Sources"))
        .expect("pre-create leftover Sources directory");
    let paths_from_dirty_state = {
        let _cwd = CwdGuard::enter(dirty_dir.path());
        generated_paths(&api, &config)
    };

    assert_eq!(
        paths_from_clean_state, paths_from_dirty_state,
        "generated file paths must be a pure function of (IR, config); they must not depend on \
         what directories happen to already exist on disk"
    );

    // Direct regression guard: both runs must land under the canonical `packages/swift/Sources/`
    // prefix. The historical bug's empty-package-root fallback produced a bare `Sources/...` path
    // instead -- two runs could still agree with EACH OTHER while both being wrong in that way, so
    // pin the expected prefix explicitly rather than only comparing the two runs to each other.
    assert!(
        paths_from_clean_state
            .iter()
            .any(|path| path.starts_with(Path::new("packages").join("swift").join("Sources").join("RustBridge"))),
        "expected a packages/swift/Sources/RustBridge/... file, got: {paths_from_clean_state:?}"
    );
    assert!(
        paths_from_clean_state
            .iter()
            .all(|path| !path.starts_with(Path::new("Sources"))),
        "no generated path should fall back to a bare Sources/... prefix, got: {paths_from_clean_state:?}"
    );
}
