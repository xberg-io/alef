//! The synthetic consumer crate that `generated_output_downstream_gate` emits bindings for.
//!
//! Split out of the parent file to keep it under the repo's 1,000-line cap for
//! `tests/**/*.rs`; the parent `mod`s this in the same way it already `mod`s
//! `poly_fmt_exclusions`.

// ---------------------------------------------------------------------------
// Fixture: a synthetic consumer, deliberately nobody's real crate
// ---------------------------------------------------------------------------

pub(crate) const FIXTURE_SOURCE: &str = r#"
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Segment {
    pub index: u32,
    pub text: String,
}

/// A private-field wrapper whose `Display` representation could be redacted. The generated
/// bindings must use only these explicit lossless operations, including below containers and in
/// enum payloads; the full-language emitted tree is the compile oracle for that contract. ~keep
#[cfg_attr(alef, alef(transparent_string(from = "from", into = "into_inner")))]
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretString(String);

impl SecretString {
    pub fn from(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

#[cfg_attr(alef, alef(transparent_string(from = "from", into = "into_inner")))]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OrderedSecret(String);

impl OrderedSecret {
    pub fn from(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Authentication {
    Bearer(SecretString),
    Header { value: SecretString },
    Optional { value: Option<SecretString> },
    Pairs {
        values: std::collections::HashMap<SecretString, SecretString>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub total: u64,
    pub segments: Vec<Segment>,
    pub attachment: Attachment,
    pub credential: SecretString,
    pub optional_credential: Option<SecretString>,
    pub nested_optional_credential: Option<Option<SecretString>>,
    pub secret_headers: std::collections::HashMap<String, SecretString>,
    pub secret_chain: Vec<Option<SecretString>>,
    pub secret_lookup: std::collections::HashMap<SecretString, String>,
    pub secret_pairs: std::collections::HashMap<SecretString, Vec<SecretString>>,
    pub ordered_secret_pairs: std::collections::BTreeMap<OrderedSecret, OrderedSecret>,
    pub nested_optional_segments:
        Option<Option<Vec<std::collections::HashMap<SecretString, Segment>>>>,
    pub authentication: Authentication,
}

pub fn echo_secret(value: SecretString) -> SecretString {
    value
}

pub fn optional_secret(value: Option<SecretString>) -> Option<SecretString> {
    value
}

pub async fn async_secret(value: SecretString) -> SecretString {
    value
}

pub fn fallible_secret(value: SecretString) -> Result<SecretString, String> {
    Ok(value)
}

/// `Chunked` is gated on a feature this crate declares but does not default-enable (see
/// `FIXTURE_CARGO_TOML`'s `[features]` table) -- alef forwards it into every binding crate's own
/// manifest and turns it on by default there, so a clippy-lane binding crate always compiles
/// with it active. Regression coverage for two defects at once: (1) the forwarding row must
/// exist at all -- pyo3 previously declared no such row, so this gate's own `#[cfg(feature =
/// "chunking-tokenizers")]` re-emission tripped `unexpected_cfgs` under `-D warnings`; (2) the
/// resulting `From<Binding> for Mode` / `From<Mode> for Binding` match must NOT gain a trailing
/// `_ => Default::default()` merely because a variant is cfg-gated -- the arm carries the same
/// gate as the variant, so with the feature always active in this gate's own build the catch-all
/// is unreachable and trips `unreachable_patterns` under `-D warnings`. ~keep
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    Fast,
    Thorough,
    #[cfg(feature = "chunking-tokenizers")]
    Chunked,
}

/// A manual (not derived) `Default` impl: `DocumentProcessor::preferred_mode` returns `Mode`
/// from a Rust-defaulted, infallible sync method, and every backend's sync trait-bridge body
/// substitutes `Default::default()` on a failure path (missing JS property, unparseable
/// return, ...) regardless of whether that path is ever exercised at runtime -- the substitution
/// is source code, so `Mode: Default` is a compile-time requirement independent of it. A manual
/// impl (rather than `#[derive(Default)]` on the enum above) keeps that derive list -- and the
/// `unreachable_patterns` catch-all coverage it documents -- untouched. ~keep
impl Default for Mode {
    fn default() -> Self {
        Mode::Fast
    }
}

/// The crate's uniform trait-bridge error type: bridge codegen constructs every bridged-method
/// failure via `{core_import}::{error_type}::from({msg})` (see
/// `ResolvedCrateConfig::error_constructor_expr`) and requires each bridged trait's fallible
/// methods to return exactly this configured type -- not an arbitrary per-method error, since
/// the generated `impl Trait for Wrapper` block must match the real trait declaration. Plain
/// (not `#[derive(thiserror::Error)]`), so it is never extracted into `ErrorDef`/`api.errors`
/// and triggers no codegen beyond `DocumentProcessor` itself. ~keep
#[derive(Debug)]
pub struct Error(pub String);

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error(message)
    }
}

/// A free function gated the same way, covering the function-position half of the same
/// forwarding requirement (`Mode::Chunked` above only covers the enum-variant position).
#[cfg(feature = "chunking-tokenizers")]
pub fn count_tokens(text: String) -> Result<u64, String> {
    Ok(text.len() as u64)
}

/// A data-carrying enum, covering both variant shapes swift's `from_string` reconstruction
/// helper cannot handle: a wire string carries only a variant's discriminant, never its
/// field data. Regression coverage for the bug where `alef generate` emitted a bare
/// `EnumName::Variant` path for every variant of every enum regardless of fields, which
/// does not type-check against a tuple or struct variant (E0308 / E0533). ~keep
#[derive(Clone, Default, Serialize, Deserialize)]
pub enum Attachment {
    #[default]
    None,
    Url(String),
    Inline {
        mime_type: String,
        bytes_len: u64,
    },
}

/// No public fields, so the extractor keeps this opaque and every backend has to emit a
/// handle type for it. The handle is what the JNI emitter turns into a `jlong` round
/// trip — the surface the redundant-cast regression lived on. ~keep
pub struct Session {
    token: String,
}

impl Session {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub fn token(&self) -> String {
        self.token.clone()
    }

    pub fn replace_secret(&mut self, value: SecretString) -> SecretString {
        SecretString::from(std::mem::replace(&mut self.token, value.into_inner()))
    }

    pub async fn static_async_secret(value: SecretString) -> SecretString {
        value
    }

    pub async fn static_fallible_secret(value: SecretString) -> Result<SecretString, String> {
        Ok(value)
    }

    pub fn analyze(&self, input: String, mode: Mode) -> Result<Report, String> {
        let _ = (input, mode);
        Err("unimplemented".to_string())
    }

    /// Infallible counterpart to `analyze`: same fieldless-enum parameter (`Mode`), but no
    /// `Result` return of its own. `analyze` alone cannot catch a declaration/implementation
    /// fallibility mismatch on a type method: a fieldless enum parameter is reconstructed from
    /// its wire string by a fallible helper, which forces a binding's shim to become
    /// `Result<_, String>` even when the method itself is not -- but `analyze` was ALREADY
    /// `Result`-returning, so both sides "agreeing on Result" proved nothing about whether the
    /// forced-fallibility case is tracked at all. This method is the infallible pairing that
    /// does exercise it. ~keep
    pub fn mode_name(&self, mode: Mode) -> String {
        let _ = mode;
        self.token.clone()
    }
}

pub fn summarize(input: String) -> Result<Report, String> {
    let _ = input;
    Err("unimplemented".to_string())
}

/// No cast should be emitted on either side of the JNI boundary: `f64`'s JNI wire type
/// (`jni::sys::jdouble`) is a type alias for `f64` itself. Regression coverage for the bug
/// where JNI's `primitive_cast` (param unmarshalling) and `emit_return_marshal_with_indent`
/// (return marshalling) assumed every primitive needs a cast to its own wire type and cast an
/// already-`f64` value to `f64`, tripping `clippy::unnecessary_cast` under `-D warnings`
/// (alef commit c82f8f117). Unit coverage for the underlying helpers already existed; this
/// free function is what lets the live JNI crate this gate compiles actually contain the
/// generated call-site and return-site casts, so a regression here fails `cargo clippy`. ~keep
pub fn round_trip_cost(cost_usd: f64) -> f64 {
    cost_usd
}

/// The pointee type a capsule (host-native passthrough) function returns. Kept out of the
/// binding surface itself (`alef(skip)`) since only its fully-qualified path matters to the
/// FFI backend, mirroring how a real capsule pointee (e.g. tree-sitter's `TSLanguage`) lives
/// in a crate alef never parses. ~keep
#[cfg_attr(alef, alef(skip))]
pub struct RawLanguage {
    pub value: u64,
}

/// A capsule-configured type (see `[crates.ffi.capsule_types.Language]` in
/// `FIXTURE_ALEF_TOML`): the FFI backend returns `into_raw()`'s own pointer verbatim instead
/// of boxing it, so the exported C function's declared return and `into_raw()`'s real return
/// are the same `*const RawLanguage` by construction. Regression coverage for the bug where
/// `capsule_into_raw_expr` appended a redundant `as *const RawLanguage` to an
/// already-`*const RawLanguage` expression, tripping `clippy::unnecessary_cast` under
/// `-D warnings` (alef commit c82f8f117). Unit coverage for `capsule_into_raw_expr` already
/// existed; this type and `get_language` are what let the live FFI crate this gate compiles
/// actually contain the generated capsule return, so a regression here fails `cargo clippy`.
/// ~keep
pub struct Language {
    handle: u64,
}

impl Language {
    #[cfg_attr(alef, alef(skip))]
    pub fn into_raw(self) -> *const RawLanguage {
        Box::into_raw(Box::new(RawLanguage { value: self.handle })) as *const RawLanguage
    }
}

pub fn get_language(name: String) -> Result<Language, String> {
    if name.is_empty() {
        return Err("empty language name".to_string());
    }
    Ok(Language { handle: 1 })
}

// A crate whose public API takes and returns a foreign type re-exports it, or no Rust caller
// could name the argument without depending on `foreign_core` directly. Alef's emitted bindings
// reach a `[[crates.source_crates]]` type through the core crate path (`toolkit::Swatch`), so
// without this the generated pyo3 crate fails `E0425: cannot find type Swatch in crate toolkit`
// -- a fixture defect, not a codegen one. ~keep
pub use foreign_core::Swatch;

/// Exercises `Swatch` (see `FOREIGN_CRATE_SOURCE`) in both directions: as a parameter, so
/// `impl From<BindingEnum> for foreign_core::Swatch` is actually generated (`input_type_names`
/// only sees a type used as a parameter, not merely returned), and as a return type, so `impl
/// From<foreign_core::Swatch> for BindingEnum` is generated too. See
/// `foreign_cfg_gate::write_foreign_crate` for how `foreign_core` reaches this crate's
/// dependency graph, and `[[crates.source_crates]]` below for how `Swatch` reaches the IR. ~keep
pub fn recolor(swatch: foreign_core::Swatch) -> foreign_core::Swatch {
    swatch
}

/// A napi-only trait bridge (`[[crates.trait_bridges]]` below restricts it to `node` via
/// `exclude_languages` -- every other GATE_LANGUAGES backend has its own trait-bridge generator,
/// untouched by xberg#1636's fix, and is out of scope here). Covers the method shapes the
/// rewritten napi generator (`NapiBridgeGenerator`) distinguishes:
/// - `transform`: async, a `Bytes` param, a native-struct (`Report`) return.
/// - `describe`: async, a `Path` param, Rust-defaulted (forwardable).
/// - `refine`: async, `Unit` return, a single `&mut` native-struct param -- the
///   `PostProcessor::process` write-back shape.
/// - `cost`: sync, infallible, required -- the `TokenizerBackend::count_tokens` shape.
/// - `label`: sync, fallible, Rust-defaulted.
/// - `preferred_mode`: sync, an enum return.
/// - `supported_mime_types`: sync, *required* (no Rust default -- a defaulted one routes through
///   the presence-check delegate instead and does not exercise this), returning a borrowed `&[&str]`. This is the one shape whose
///   emitted body is not used as-is: `trait_impl` collects it into a `Vec<String>` and leaks it
///   into a `&'static [&'static str]`. It is covered here because the napi sync bridge returns
///   from inside its reply loop rather than ending in a tail expression, and a plain block
///   around such a body lets those `return`s escape the trait method with the unconverted
///   `Vec<String>` (xberg#1636 follow-up).
#[async_trait::async_trait]
pub trait DocumentProcessor: Send + Sync {
    async fn transform(&self, content: Vec<u8>) -> Result<Report, Error>;

    async fn describe(&self, path: std::path::PathBuf) -> String {
        let _ = path;
        "unchanged".to_string()
    }

    async fn refine(&self, report: &mut Report) -> Result<(), Error>;

    fn cost(&self, text: &str) -> u64 {
        text.len() as u64
    }

    fn label(&self, mode: Mode) -> Result<String, Error> {
        let _ = mode;
        Ok("default".to_string())
    }

    fn preferred_mode(&self) -> Mode {
        Mode::Fast
    }

    fn supported_mime_types(&self) -> &[&str];
}

/// A SECOND bridged trait, for the `options_field` binding mode, and deliberately not
/// `DocumentProcessor`: that one returns a native enum (`preferred_mode -> Mode`) from a
/// bridged method, which the pyo3 trait-bridge generator cannot convert -- see the `~keep`
/// note on `exclude_languages` in `FIXTURE_ALEF_TOML`. Reusing it here would fail with that
/// unrelated E0277 long before any options-field behaviour was reached. One method, unit
/// return, one `String` parameter: the smallest shape pyo3 can bridge end to end. ~keep
pub trait ProgressListener: Send + Sync {
    fn on_step(&self, label: String);
}

/// The handle the bridge installs on `RunOptions::on_progress`.
pub type ProgressHandle = std::sync::Arc<std::sync::Mutex<dyn ProgressListener + Send + Sync>>;

/// The config struct the bridge attaches to. `on_progress` is `#[serde(skip)]` because a
/// trait object has no serde representation; the binding still has to carry the field, which
/// is what makes this the `options_field` shape rather than a plain parameter. ~keep
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RunOptions {
    pub retries: u32,
    #[serde(skip)]
    pub on_progress: Option<ProgressHandle>,
}

/// The ASYNC half of the options-field coverage. Every other public free function in this
/// fixture is synchronous, so without this the emitted wrapper's `pyo3_async_runtimes`
/// `future_into_py` path -- and the `.await` inside it -- is never compiled at all. The bug
/// this catches returned the un-awaited future from a function declared to return
/// `PyResult<Bound<'py, PyAny>>` (alef #476). ~keep
pub async fn execute(input: String, settings: RunOptions) -> Result<String, String> {
    let _ = (input, settings);
    Ok("done".to_string())
}

/// The SYNC half, taking the same **required** (not `Option<...>`) config. The wrapper used
/// to pattern-match `Some`/`None` against this parameter regardless of the signature it had
/// just emitted, an E0308 the clippy lane is the only thing that catches. ~keep
pub fn inspect(settings: RunOptions) -> Result<String, String> {
    let _ = settings;
    Ok("ok".to_string())
}
"#;

// A FOREIGN crate (a `[[crates.source_crates]].roots` merge target, not a file the `toolkit`
// crate itself is built from) providing `Swatch`: an enum whose cfg-gated variant is owned by a
// crate other than the one every binding wraps. `foreign_cfg_gate.rs` writes this alongside
// `toolkit` in the fixture workspace and sabotages the emitted pyo3 conversion to prove the
// downstream gate actually compiles this shape -- see that module's doc for the full rationale
// (alef commit f9795aea9's E0004 fix and its `unreachable_patterns` opposite). ~keep
pub(crate) const FOREIGN_CRATE_CARGO_TOML: &str = "[package]\nname = \"foreign_core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n\n[features]\nspot-colors = []\n";

pub(crate) const FOREIGN_CRATE_SOURCE: &str = r#"
/// A fieldless enum standing in for a dependency-owned type merged into `toolkit`'s binding
/// surface via `[[crates.source_crates]].roots`. `Spot` is gated on a feature only THIS crate
/// declares -- `toolkit` never enables it, directly or via `extra_dependencies`, so every
/// binding crate compiles `foreign_core` with `spot-colors` off and `Spot` genuinely absent
/// from the type this enum compiles to. Fieldless only: a data-carrying enum never reaches the
/// codegen path this fixture exercises (see `Pyo3Backend`'s own `enum_has_data_variants` skip).
/// `Default` is required: the binding-to-core direction's `_ => Default::default()` catch-all
/// (kept for the E0004 shape this fixture exists to reproduce) needs `Self: Default`.
/// `Serialize`/`Deserialize` are required too, and NOT incidentally: the jni backend crosses the
/// boundary by serializing through `serde_json::to_string`, so a bound type that does not derive
/// serde fails `E0277` in the emitted `-jni` crate even though pyo3 compiles fine. The gate found
/// that on its first full run -- every type alef binds must be serde-serializable, and a fixture
/// whose foreign enum was not is the unrealistic party here, not the jni codegen. ~keep
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub enum Swatch {
    #[default]
    Base,
    Accent,
    #[cfg(feature = "spot-colors")]
    Spot,
}
"#;

// The fixture's own core crate derives serde, so it needs the dependency to compile. It went
// unnoticed until the core crate became reachable from the emitted binding crates: before that
// nothing ever built it, so an uncompilable fixture still passed every lane. ~keep
//
// `[lints.rust] unexpected_cfgs`'s `check-cfg` allowlist is required, not decorative: the
// fixture source above uses alef's own `#[cfg_attr(alef, alef(skip))]` exclusion marker (see
// `RawLanguage` and `Language::into_raw`), and `cfg(alef)` is never a real, declared cfg --
// there is no `alef` proc-macro crate this fixture depends on, so the attribute inside each
// `cfg_attr` never actually runs at real compile time. Without this allowlist, rustc's
// `unexpected_cfgs` lint fires on both call sites and the clippy lane's `-D warnings` denies
// it: this crate is a path dependency of every clippy-lane binding crate, so an unconfigured
// core manifest fails builds it does not otherwise participate in. `alef scaffold` patches
// this same allowlist into `[workspace.lints.rust]` when the root manifest already declares a
// `[workspace]` (see `cli::pipeline::workspace_lints`), but this fixture's root is a plain
// `[package]` manifest -- the common case for a small, pre-existing consumer crate -- so it
// carries the allowlist itself, exactly as a real consumer in that shape has to today. ~keep
// `foreign_core` is alphabetized ahead of `serde` in `[dependencies]` -- `cargo sort --check`
// runs over this manifest too (it is reachable from every clippy-lane binding crate), so an
// out-of-order fixture-authored dependency would fail a lane this file has nothing to do with.
// The path itself is a placeholder: `foreign_cfg_gate::write_foreign_crate` substitutes an
// absolute path at fixture-write time, so it resolves regardless of how deep the clippy-lane
// binding crate that pulls `toolkit` in as a path dependency happens to sit. ~keep
pub(crate) const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"toolkit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                                              [features]\nchunking-tokenizers = []\n\n\
                                              [dependencies]\nasync-trait = \"0.1\"\n\
                                              foreign_core = { path = \"__FOREIGN_CORE_DEP_PATH__\" }\n\
                                              serde = { version = \"1\", features = [\"derive\"] }\n\n\
                                              [lints.rust]\nunexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(alef)'] }\n";

// `java` and `elixir` scaffolders bail `alef generate` outright when repository/license/
// authors are unset (`scaffold::languages::java`, `scaffold::languages::elixir`), and both
// are gate languages, so this metadata is required for `alef generate` to succeed over the
// fixture at all -- not merely to exercise a "configured" code path. ~keep
pub(crate) const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
alef_version = "__ALEF_VERSION__"
languages = [__LANGUAGES__]

[[crates]]
name = "toolkit"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.generate]
public_api = true

[crates.package_metadata]
repository = "https://github.com/example/toolkit"
license = "MIT"
authors = ["Example Author <author@example.invalid>"]

# ~keep `execute` is async and takes `RunOptions`, whose `on_progress` field node no longer
# carries: this bridge excludes node, and the per-language carrier prune (alef #480) drops the
# field from every language the bridge does not target. That is what makes `JsRunOptions`
# `Send` again, which napi requires of an async function's arguments. Until #480 this fixture
# had to remove `execute` from the node surface entirely; the gate compiling it here is the
# proof the prune reaches napi, so do not reintroduce a `[crates.node] exclude_functions`
# workaround without re-checking that.

[crates.ffi.capsule_types.Language]
into_raw_type = "toolkit::RawLanguage"
c_return_type = "RawLanguage"

[crates.extra_dependencies]
foreign_core = { path = "__FOREIGN_CORE_DEP_PATH__" }

[[crates.source_crates]]
name = "foreign_core"
sources = ["__FOREIGN_CORE_SOURCE_PATH__"]
roots = ["Swatch"]

# napi-only: every other GATE_LANGUAGES backend has its own trait-bridge generator, untouched
# by this fixture's reason for existing (the napi async/sync trait-bridge rewrite), and
# exercising them here is out of scope. See `DocumentProcessor`'s doc in `FIXTURE_SOURCE`.
#
# ~keep Adding "python" here does NOT currently work, and the reason is not this fixture: the
# pyo3 trait-bridge generator cannot convert a native enum return (`preferred_mode -> Mode`),
# so the emitted crate fails with E0277 before any shared-`trait_impl` behaviour is reached.
# Widening this gate to pyo3 has to wait on that generator gap.
[[crates.trait_bridges]]
trait_name = "DocumentProcessor"
# Written in poly's own canonical TOML array layout -- one element per line at two spaces.
# The poly lane checks the whole emitted tree, this fixture's own input file included, so a
# hand-wrapped array here reports as unformatted alef output and the lane fails on the test's
# input rather than on anything alef produced. Same reason `fixture_language_list` wraps and
# `FIXTURE_SOURCE` is written with `trim_start`. ~keep
exclude_languages = [
  "ffi",
  "python",
  "wasm",
  "jni",
  "kotlin_android",
  "java",
  "ruby",
  "php",
  "elixir",
  "swift",
  "go",
  "csharp",
]

# The options-field bridge, python-only. Two things ride on this entry being here:
#
# 1. `execute` and `inspect` both take `RunOptions`, so the pyo3 crate this gate compiles
#    contains the emitted `#[pyfunction]` wrapper for an async AND a sync function whose
#    config parameter is required -- the exact pair whose wrapper did not type-check.
# 2. Every other gate language is excluded, so the ffi crate the gate ALSO compiles must
#    contain no `toolkit_options_set_on_progress` and no `ToolkitProgressListenerBridge`.
#    Three of the four ffi emission sites ignored `exclude_languages` and emitted a setter
#    calling a bridge no pass wrote, which is a link failure for every backend that goes
#    through ffi, not just C. ~keep
[[crates.trait_bridges]]
trait_name = "ProgressListener"
type_alias = "ProgressHandle"
param_name = "on_progress"
bind_via = "options_field"
options_type = "RunOptions"
options_field = "on_progress"
exclude_languages = [
  "ffi",
  "node",
  "wasm",
  "jni",
  "kotlin_android",
  "java",
  "ruby",
  "php",
  "elixir",
  "swift",
  "go",
  "csharp",
]
"#;
