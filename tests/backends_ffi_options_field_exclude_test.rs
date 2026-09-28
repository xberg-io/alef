//! `exclude_languages` must suppress **every** options-field emission in the FFI crate, not
//! just the bridge struct (alef #476).
//!
//! Before this, three of the four sites -- the "has an options-field bridge" flag, the
//! per-function wrapper, and the `{prefix}_options_set_{field}` setter -- scanned the bridge
//! list unfiltered while only the bridge struct honoured the exclusion. The FFI crate then
//! referenced a `{Prefix}{Trait}Bridge` and a `{prefix}_{trait}_bridge_new` that nothing
//! generated, which breaks every backend that links through FFI (go, java, csharp, kotlin,
//! kotlin_android, jni, zig, and dart in FFI style), not only C. ~keep

use std::path::Path;

use alef::backends::ffi::FfiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

const SOURCE: &str = r#"
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

pub trait ProgressListener: Send + Sync {
    fn on_step(&self, label: String);
}

pub type ProgressHandle = Arc<Mutex<dyn ProgressListener + Send + Sync>>;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RunOptions {
    pub retries: u32,
    #[serde(skip)]
    pub on_progress: Option<ProgressHandle>,
}

pub fn inspect(settings: RunOptions) -> Result<String, String> {
    let _ = settings;
    Ok(String::new())
}
"#;

fn extract(dir: &Path) -> ApiSurface {
    let src = dir.join("lib.rs");
    std::fs::write(&src, SOURCE).expect("write fixture source");
    alef::extract::extractor::extract(&[src.as_path()], "sample_lib", "0.1.0", None).expect("extract fixture")
}

fn config_with(bridge_extra: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "ProgressListener"
type_alias = "ProgressHandle"
param_name = "on_progress"
bind_via = "options_field"
options_type = "RunOptions"
options_field = "on_progress"
{bridge_extra}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("parse alef.toml");
    cfg.resolve().expect("resolve config").remove(0)
}

fn lib_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    FfiBackend
        .generate_bindings(api, config)
        .expect("generate ffi bindings")
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .expect("lib.rs")
        .content
}

/// Negative control for the assertions below: with no exclusion the symbols really are there,
/// so their absence in the excluded case cannot be a test that examined nothing. ~keep
#[test]
fn an_active_bridge_emits_the_setter_and_the_bridge_struct() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with(""));

    assert!(
        lib.contains("_options_set_on_progress"),
        "an active options-field bridge must emit its setter; lib.rs:\n{lib}"
    );
    assert!(
        lib.contains("ProgressListenerBridge"),
        "an active options-field bridge must emit its bridge struct; lib.rs:\n{lib}"
    );
}

#[test]
fn excluding_c_suppresses_every_options_field_emission() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let lib = lib_rs(&api, &config_with("exclude_languages = [\"c\"]"));

    assert!(
        !lib.contains("_options_set_on_progress"),
        "an excluded bridge must emit no options setter; lib.rs:\n{lib}"
    );
    assert!(
        !lib.contains("ProgressListenerBridge"),
        "an excluded bridge must leave no reference to a bridge struct no pass wrote; lib.rs:\n{lib}"
    );
    assert!(
        !lib.contains("_bridge_new"),
        "an excluded bridge must emit no bridge constructor; lib.rs:\n{lib}"
    );
}

#[test]
fn the_backend_spelling_excludes_the_same_output_as_the_language_spelling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let api = extract(dir.path());
    let by_language = lib_rs(&api, &config_with("exclude_languages = [\"c\"]"));
    let by_backend = lib_rs(&api, &config_with("exclude_languages = [\"ffi\"]"));
    assert_eq!(
        by_language, by_backend,
        "`c` and `ffi` are the same target's two spellings and must suppress identically"
    );
}
