//! Resolve-time validation coverage for new-style config, split from tests.rs.

use super::*;
use crate::core::config::extras::Language;

/// A NuGet collision key is folded per package ID, not shared globally -- two crates with
/// distinct package IDs must resolve cleanly even though both target `csharp`.
#[test]
fn new_alef_config_resolve_allows_distinct_nuget_package_ids() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "crate-a"
sources = ["src/lib.rs"]

[crates.csharp]
package_id = "MyLib"

[[crates]]
name = "crate-b"
sources = ["src/other.rs"]

[crates.csharp]
package_id = "OtherLib"
"#,
    )
    .unwrap();
    assert!(cfg.resolve().is_ok());
}

#[test]
fn new_alef_config_resolve_per_crate_languages_overrides_workspace() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "x"
sources = ["src/lib.rs"]
languages = ["node"]
"#,
    )
    .unwrap();
    let resolved = cfg.resolve().unwrap();
    assert_eq!(resolved[0].languages, vec![Language::Node]);
}

#[test]
fn resolve_inherits_workspace_language_config() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.python]
module_name = "workspace_module"

[[crates]]
name = "sample"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().unwrap();

    assert_eq!(
        resolved[0]
            .python
            .as_ref()
            .and_then(|python| python.module_name.as_deref()),
        Some("workspace_module")
    );
}

#[test]
fn resolve_crate_language_config_overrides_workspace_language_config() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[workspace.python]
module_name = "workspace_module"

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.python]
module_name = "crate_module"
"#,
    )
    .unwrap();

    let resolved = cfg.resolve().unwrap();

    assert_eq!(
        resolved[0]
            .python
            .as_ref()
            .and_then(|python| python.module_name.as_deref()),
        Some("crate_module")
    );
}

/// Regression: the plain `kotlin` backend (not `kotlin_android`) splices
/// `[crates.java].package` verbatim into generated `.kt` source (see
/// `new_config::java_package_is_consumed`'s doc comment), so a package segment that is a
/// Kotlin hard keyword but not a Java one must be rejected once `kotlin` is enabled, even
/// though the very same value passes the Java grammar on its own.
#[test]
fn resolve_rejects_java_package_that_is_a_kotlin_keyword_when_kotlin_is_enabled() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.fun"
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("[crates.java].package") && msg.contains("dev.fun")),
        "expected InvalidConfig naming the offending java package, got: {err:?}"
    );
}

/// The same package segment is a legal Java identifier and must still resolve cleanly when
/// only `java` (no `kotlin`) is enabled -- the Kotlin-grammar check must not leak into crates
/// that never generate Kotlin source from this value.
#[test]
fn resolve_accepts_java_package_that_is_only_a_kotlin_keyword_when_kotlin_is_disabled() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.fun"
"#,
    )
    .unwrap();
    assert!(cfg.resolve().is_ok());
}

#[test]
fn resolve_rejects_unknown_skip_languages_in_adapter() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.adapters]]
name = "stream_data"
pattern = "streaming"
core_path = "my_crate::stream_data"
skip_languages = ["wasm32"]
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("wasm32")),
        "expected InvalidConfig error mentioning the bad name, got: {err:?}"
    );
}

#[test]
fn resolve_accepts_valid_skip_languages_in_adapter() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.adapters]]
name = "stream_data"
pattern = "streaming"
core_path = "my_crate::stream_data"
skip_languages = ["wasm", "kotlin"]
"#,
    )
    .unwrap();
    let resolved = cfg.resolve().expect("valid skip_languages should not fail");
    assert_eq!(resolved[0].adapters[0].skip_languages, vec!["wasm", "kotlin"]);
}

#[test]
fn resolve_rejects_unknown_language_in_registration_variant() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.handler_contracts]]
trait_name = "Handler"
dispatch_method = "call"

[[crates.services]]
owner_type = "App"

[[crates.services.registrations]]
method = "add_route"
callback_param = "handler"
callback_bound = "IntoHandler"
callback_contract = "Handler"

[[crates.services.registrations.variants]]
name = "get"
fixed = { method = "GET" }

[crates.services.registrations.variants.languages.knotlin]
method_prefix = "Map"
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("knotlin")),
        "expected InvalidConfig error mentioning the bad name, got: {err:?}"
    );
}

#[test]
fn resolve_accepts_valid_language_in_registration_variant() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.handler_contracts]]
trait_name = "Handler"
dispatch_method = "call"

[[crates.services]]
owner_type = "App"

[[crates.services.registrations]]
method = "add_route"
callback_param = "handler"
callback_bound = "IntoHandler"
callback_contract = "Handler"

[[crates.services.registrations.variants]]
name = "get"
fixed = { method = "GET" }

[crates.services.registrations.variants.languages.kotlin]
method_prefix = "Map"
"#,
    )
    .unwrap();
    let resolved = cfg.resolve().expect("valid variant language should not fail");
    assert!(
        resolved[0].services[0].registrations[0].variants[0]
            .languages
            .contains_key("kotlin")
    );
}

#[test]
fn resolve_rejects_unknown_language_in_trait_bridge_exclude_languages() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["wasm32"]
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("wasm32")),
        "expected InvalidConfig error mentioning the bad name, got: {err:?}"
    );
}

#[test]
fn resolve_accepts_valid_trait_bridge_exclude_languages() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["wasm", "elixir"]
"#,
    )
    .unwrap();
    let resolved = cfg.resolve().expect("valid exclude_languages should not fail");
    assert_eq!(resolved[0].trait_bridges[0].exclude_languages, vec!["wasm", "elixir"]);
}

#[test]
fn resolve_rejects_unknown_language_in_callback_unsupported_mode() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["php-zts:callbacks"]
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("php-zts:callbacks") && msg.contains("exclude_languages")),
        "expected InvalidConfig naming the field and bad language, got: {err:?}"
    );
}

#[test]
fn resolve_rejects_callback_unsupported_mode_for_non_php_language() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["python:callbacks"]
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    assert!(
        matches!(&err, ResolveError::InvalidConfig(msg) if msg.contains("python:callbacks") && msg.contains("exclude_languages")),
        "expected InvalidConfig naming the unsupported mode and field, got: {err:?}"
    );
}

/// Every backend answers to its own name as well as its language's, and
/// `bridge_targets_language` documents exactly that -- but resolution used to reject the
/// backend spelling, so the documented form was unusable. ~keep
#[test]
fn resolve_accepts_a_backend_spelling_in_trait_bridge_exclude_languages() {
    for spelling in ["pyo3", "napi", "magnus", "rustler", "extendr"] {
        let cfg: NewAlefConfig = toml::from_str(&format!(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["{spelling}"]
"#
        ))
        .unwrap();
        let resolved = cfg
            .resolve()
            .unwrap_or_else(|e| panic!("`{spelling}` is a documented backend spelling: {e:?}"));
        assert!(!resolved[0].trait_bridges[0].is_active_for(spelling));
    }
}

#[test]
fn rejected_exclude_language_error_lists_both_name_families() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["nodejs"]
"#,
    )
    .unwrap();
    let err = cfg.resolve().unwrap_err();
    let ResolveError::InvalidConfig(message) = &err else {
        panic!("expected InvalidConfig, got {err:?}");
    };
    assert!(
        message.contains("`nodejs`"),
        "must quote the offending entry: {message}"
    );
    assert!(
        message.contains("kotlin_android") && message.contains("pyo3"),
        "must list the language names and the backend spellings: {message}"
    );
}

/// `resolve()` must not eagerly resolve `[[crates.source_crates]]` with `from_registry = true` --
/// that shells out to `cargo metadata` (see `crate::core::config::registry`), a cost (and a
/// failure mode) every subcommand paid for before this fix, including `alef publish package`,
/// which never reads `source_crates` at all. `workspace_root` here points at a directory with no
/// `Cargo.toml`, which would make `cargo metadata` fail immediately (no network access needed to
/// prove this deterministically) -- so `resolve()` succeeding is direct proof the resolution
/// never ran. ~keep
#[test]
fn resolve_does_not_invoke_cargo_for_a_from_registry_source_crate() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample_router"
sources = ["src/lib.rs"]
workspace_root = "/definitely/does/not/exist/nowhere"

[[crates.source_crates]]
name = "sample_router-core"
sources = ["src/http.rs"]
from_registry = true
"#,
    )
    .unwrap();

    let resolved = cfg
        .resolve()
        .expect("resolve() must not eagerly resolve source_crates against a nonexistent workspace_root");
    let sample_router = &resolved[0];

    // The entry is preserved verbatim, unrebased -- further proof no cargo call happened: a
    // successful rebase would have rewritten `sources` to an absolute registry path.
    assert_eq!(sample_router.source_crates.len(), 1);
    assert!(sample_router.source_crates[0].from_registry);
    assert_eq!(
        sample_router.source_crates[0].sources,
        vec![std::path::PathBuf::from("src/http.rs")]
    );

    // Only on first access from a codegen/hash-style caller does resolution actually run -- and
    // it correctly surfaces the cargo failure as an error rather than panicking.
    let err = sample_router
        .resolved_source_crates()
        .expect_err("a nonexistent workspace_root must fail to resolve, proving the lazy path is wired");
    assert!(
        matches!(err, ResolveError::RegistryResolution(_)),
        "unexpected error variant: {err:?}"
    );
}
