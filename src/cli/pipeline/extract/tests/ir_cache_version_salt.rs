//! End-to-end coverage for the defect that made an alef upgrade a no-op in the field.
//!
//! `.alef/<crate>/ir.json` holds the extracted `ApiSurface`, and its key was built from the
//! Rust sources, the consumer crate's own version, and the config — never from alef itself.
//! A newer alef therefore replayed an older alef's surface verbatim, and generated from it,
//! and `alef verify` (which re-enters this same `extract`) agreed. Nothing reported anything.
//!
//! These tests drive the real [`super::super::extract`] over a real fixture rather than
//! testing the key builder in isolation, because both halves have to hold at once and the
//! interesting failures are on opposite sides of each other:
//!
//! - [`ir_cache_is_actually_consulted_within_one_alef_version`] is the anti-vacuity half. If
//!   `extract` stopped reading the cache at all, the staleness test below would pass while
//!   proving nothing — the shape this whole investigation exists to reject.
//! - [`ir_cache_written_by_another_alef_version_is_not_replayed`] is the regression itself.
//!
//! The planted surface is the real extraction plus one extra type, so it stays valid for the
//! fixture's `include` list. That matters: a surface that failed `validate_extracted_api`
//! would make `extract` error out, and an error is not the same observation as a re-extraction.
//! ~keep

use std::path::{Path, PathBuf};

use crate::core::config::{IncludeConfig, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;

/// A type name no extractor can produce from [`FIXTURE_SOURCE`]; its presence in a returned
/// surface means that surface came off disk rather than out of the extractor.
const SENTINEL_TYPE: &str = "PlantedByAPreviousAlefRelease";

/// A version string that is not, and will never be, this crate's version.
const PREVIOUS_ALEF_VERSION: &str = "0.0.0-previous-alef-release";

const FIXTURE_SOURCE: &str = "pub struct Record {\n    pub value: String,\n}\n";

struct Fixture {
    _dir: tempfile::TempDir,
    _cwd: crate::test_support::CwdGuard,
    config: ResolvedCrateConfig,
    config_path: PathBuf,
    root: PathBuf,
}

impl Fixture {
    /// `extract` writes its cache under a CWD-relative `.alef/`, so the guard is what keeps
    /// this test's cache in this test's tempdir — the same hazard, and the same fix, as
    /// `external_type_roots::extract_with_external_type_roots_keeps_host_sources_and_field_type`.
    /// ~keep
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create fixture directory");
        let cwd = crate::test_support::CwdGuard::enter(dir.path());
        let root = std::env::current_dir().expect("read fixture directory");

        let manifest = root.join("Cargo.toml");
        let source = root.join("lib.rs");
        std::fs::write(&manifest, "[package]\nname = \"sample\"\nversion = \"1.4.0\"\n").expect("write manifest");
        std::fs::write(&source, FIXTURE_SOURCE).expect("write fixture source");

        let config = ResolvedCrateConfig {
            name: "sample".to_string(),
            sources: vec![source],
            version_from: manifest.to_string_lossy().into_owned(),
            include: IncludeConfig {
                types: vec!["Record".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };

        Self {
            config_path: root.join("alef.toml"),
            root,
            config,
            _dir: dir,
            _cwd: cwd,
        }
    }

    fn extract(&self) -> ApiSurface {
        super::super::extract(&self.config, &self.config_path, false).expect("extract fixture surface")
    }

    fn ir_hash_path(&self) -> PathBuf {
        self.root.join(".alef").join(&self.config.name).join("ir.hash")
    }

    fn ir_json_path(&self) -> PathBuf {
        self.root.join(".alef").join(&self.config.name).join("ir.json")
    }

    /// Overwrite the cached surface with one carrying [`SENTINEL_TYPE`], leaving the key
    /// alone. Models an older alef having extracted something the current one would not.
    fn plant_sentinel_surface(&self, base: &ApiSurface) {
        let mut planted = base.clone();
        let mut sentinel = base.types.first().expect("fixture surface has a type").clone();
        sentinel.name = SENTINEL_TYPE.to_string();
        sentinel.rust_path = format!("sample::{SENTINEL_TYPE}");
        planted.types.push(sentinel);
        std::fs::write(
            self.ir_json_path(),
            serde_json::to_string_pretty(&planted).expect("serialize planted surface"),
        )
        .expect("plant cached surface");
    }

    fn plant_malformed_newtype_metadata(&self, base: &ApiSurface) {
        let mut planted = base.clone();
        planted.types[0].fields[0].newtype_wrapper = Some("alef:newtype-conversions:v1:not-json".to_string());
        std::fs::write(
            self.ir_json_path(),
            serde_json::to_string_pretty(&planted).expect("serialize planted surface"),
        )
        .expect("plant cached surface");
    }
}

fn has_sentinel(api: &ApiSurface) -> bool {
    api.types.iter().any(|ty| ty.name == SENTINEL_TYPE)
}

fn read_key(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read cached IR key")
}

/// Without this, the regression test below is satisfiable by an `extract` that never reads
/// the cache at all — which would pass while leaving every warm run paying full extraction.
#[test]
fn ir_cache_is_actually_consulted_within_one_alef_version() {
    let fixture = Fixture::new();
    let first = fixture.extract();
    assert!(
        first.types.iter().any(|ty| ty.name == "Record"),
        "fixture must extract its one public type before anything is planted"
    );
    assert!(!has_sentinel(&first), "the extractor cannot invent {SENTINEL_TYPE}");

    fixture.plant_sentinel_surface(&first);
    let replayed = fixture.extract();

    assert!(
        has_sentinel(&replayed),
        "a second extract with the same inputs and the same alef build must serve the cached \
         surface; if it does not, the staleness test in this file proves nothing"
    );
}

#[test]
fn malformed_cached_newtype_metadata_is_rejected_before_generation() {
    let fixture = Fixture::new();
    let first = fixture.extract();
    fixture.plant_malformed_newtype_metadata(&first);

    let error = super::super::extract(&fixture.config, &fixture.config_path, false)
        .expect_err("invalid cached metadata must fail validation");
    let message = format!("{error:#}");

    assert!(message.contains("invalid_newtype_metadata"), "{message}");
    assert!(message.contains("invalid transparent newtype metadata"), "{message}");
}

/// The regression: a cache entry written by a different alef release must not be served.
///
/// The stale key is built by `extract`'s own key builder with only the alef version changed,
/// so this asserts against the real composition rather than a test-local restatement of it.
#[test]
fn ir_cache_written_by_another_alef_version_is_not_replayed() {
    let fixture = Fixture::new();
    let first = fixture.extract();
    fixture.plant_sentinel_surface(&first);

    let current_key = read_key(&fixture.ir_hash_path());
    let stale_key = super::super::ir_cache_key(&fixture.config, &fixture.config_path, PREVIOUS_ALEF_VERSION)
        .expect("build the previous release's IR cache key");
    assert_ne!(
        stale_key.as_str(),
        current_key,
        "the IR cache key must differ between alef releases for identical extraction inputs; \
         equal keys are the bug — a newer alef silently generating from an older one's surface"
    );

    std::fs::write(fixture.ir_hash_path(), stale_key.as_str()).expect("plant the previous release's key");
    let refreshed = fixture.extract();

    assert!(
        !has_sentinel(&refreshed),
        "a surface cached under another alef release's key must be re-extracted, not replayed"
    );
    assert!(
        refreshed.types.iter().any(|ty| ty.name == "Record"),
        "re-extraction must produce the real surface"
    );
    assert_eq!(
        read_key(&fixture.ir_hash_path()),
        current_key,
        "re-extraction must re-key the cache to the running alef build"
    );
}
