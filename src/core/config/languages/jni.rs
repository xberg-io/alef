use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::FfiTargetDepOverride;

/// Configuration for the JNI Rust shim crate emitter (`alef-backend-jni`).
///
/// Most identifiers are derived from the paired `[crates.kotlin_android]`
/// section (package, features, etc.).  Set `crate_dir` when the JNI crate
/// directory should differ from the default `<config.name>-jni/` — for
/// example when `config.name` carries a language-specific suffix (e.g.
/// `"sample-markdown-rs"`) but you want the JNI crate to live at
/// `crates/sample-markdown-jni/` to match every other binding crate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct JniConfig {
    /// Override the JNI crate directory name.
    ///
    /// When set, the JNI crate is placed at `crates/<crate_dir>-jni/` and the
    /// `[package] name` in the generated `Cargo.toml` is `<crate_dir>-jni`.
    /// When unset, both derive from `config.name` (the default, which matches
    /// the behavior used by `alef-backend-jni::gen_shims::jni_output_path`).
    #[serde(default)]
    pub crate_dir: Option<String>,
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated JNI `Cargo.toml`. Mirrors [`FfiConfig::target_dep_overrides`].
    ///
    /// The JNI shim crate is the unit that gets cross-compiled to Android NDK
    /// (and potentially iOS) targets, where native-C deps (libheif via `heic`)
    /// and ONNX Runtime (`ort-sys`, no Android prebuilt) cannot link. Without
    /// per-target gating the JNI crate pulls the unconditional `["full"]`
    /// feature set on every target, breaking the Kotlin-Android build.
    ///
    /// When this list is non-empty the scaffold emits
    /// `[target.'cfg(not(<any-cfg>))'.dependencies]` for the default branch
    /// plus one `[target.'cfg(<cfg>)'.dependencies]` block per override,
    /// instead of a single unconditional core-crate dependency line.
    ///
    /// [`FfiConfig::target_dep_overrides`]: super::FfiConfig::target_dep_overrides
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
}
