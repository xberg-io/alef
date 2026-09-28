use heck::ToSnakeCase;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Node,
    Ruby,
    Php,
    Elixir,
    Wasm,
    Ffi,
    Go,
    Java,
    Csharp,
    R,
    Rust,
    Kotlin,
    #[serde(rename = "kotlin_android", alias = "kotlin-android")]
    KotlinAndroid,
    Swift,
    Dart,
    Gleam,
    Zig,
    /// C consumer of the FFI layer — e2e test target, not a generated binding.
    C,
    /// Rust JNI shim crate emitter — paired with kotlin-android.
    /// Emits `Java_*` symbols that mirror the Kotlin Bridge `external fun` declarations.
    #[serde(rename = "jni")]
    Jni,
}

impl Language {
    /// ~keep Every `Language` variant, so name lists can be derived from the enum
    /// rather than hand-maintained alongside it. Enumerated by hand because
    /// `Language` has no derive-based variant iterator.
    pub const ALL: [Language; 20] = [
        Self::Python,
        Self::Node,
        Self::Ruby,
        Self::Php,
        Self::Elixir,
        Self::Wasm,
        Self::Ffi,
        Self::Go,
        Self::Java,
        Self::Csharp,
        Self::R,
        Self::Rust,
        Self::Kotlin,
        Self::KotlinAndroid,
        Self::Swift,
        Self::Dart,
        Self::Gleam,
        Self::Zig,
        Self::C,
        Self::Jni,
    ];

    /// The spellings `exclude_languages` accepts for this target: its own name, plus the
    /// backend crate that emits it where those differ.
    ///
    /// Mirrors each backend's own `TARGET_SPELLINGS` const. A caller that has a `Language`
    /// rather than a specific backend -- the generation pipeline's per-language prune is the
    /// motivating one -- cannot reach those consts, and asking only the language name would
    /// miss `exclude_languages = ["pyo3"]`. Kept in lock-step with the backends by
    /// `tests/trait_bridge_exclude_language_spellings.rs`, which scans the consts out of the
    /// source rather than trusting this match. ~keep
    pub fn bridge_spellings(self) -> &'static [&'static str] {
        match self {
            Self::Python => &["python", "pyo3"],
            Self::Node => &["node", "napi"],
            Self::Ruby => &["ruby", "magnus"],
            Self::Elixir => &["elixir", "rustler"],
            Self::R => &["r", "extendr"],
            Self::Ffi => &["ffi", "c"],
            Self::Php => &["php"],
            Self::Wasm => &["wasm"],
            Self::Go => &["go"],
            Self::Java => &["java"],
            Self::Csharp => &["csharp"],
            Self::Rust => &["rust"],
            Self::Kotlin => &["kotlin"],
            Self::KotlinAndroid => &["kotlin_android"],
            Self::Swift => &["swift"],
            Self::Dart => &["dart"],
            Self::Gleam => &["gleam"],
            Self::Zig => &["zig"],
            Self::C => &["c"],
            Self::Jni => &["jni"],
        }
    }

    /// Comma-joined canonical names of every `Language` variant, derived from [`Self::ALL`]
    /// and [`Display`](std::fmt::Display) rather than a second hand-typed list. Used to build
    /// "valid names are: ..." validation messages so they cannot drift from the enum they
    /// describe.
    #[must_use]
    pub fn all_names_joined() -> String {
        Self::ALL.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Python => write!(f, "python"),
            Self::Node => write!(f, "node"),
            Self::Ruby => write!(f, "ruby"),
            Self::Php => write!(f, "php"),
            Self::Elixir => write!(f, "elixir"),
            Self::Wasm => write!(f, "wasm"),
            Self::Ffi => write!(f, "ffi"),
            Self::Go => write!(f, "go"),
            Self::Java => write!(f, "java"),
            Self::Csharp => write!(f, "csharp"),
            Self::R => write!(f, "r"),
            Self::Rust => write!(f, "rust"),
            Self::Kotlin => write!(f, "kotlin"),
            Self::KotlinAndroid => write!(f, "kotlin_android"),
            Self::Swift => write!(f, "swift"),
            Self::Dart => write!(f, "dart"),
            Self::Gleam => write!(f, "gleam"),
            Self::Zig => write!(f, "zig"),
            Self::C => write!(f, "c"),
            Self::Jni => write!(f, "jni"),
        }
    }
}

/// A parameter in an adapter function.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterParam {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub optional: bool,
}

/// The kind of adapter pattern.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdapterPattern {
    SyncFunction,
    AsyncMethod,
    CallbackBridge,
    Streaming,
}

/// Configuration for a single adapter.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    pub name: String,
    pub pattern: AdapterPattern,
    /// Full Rust path to the core function/method (e.g., "sample_markdown_rs::convert")
    pub core_path: String,
    /// Parameters
    #[serde(default)]
    pub params: Vec<AdapterParam>,
    /// Return type name
    pub returns: Option<String>,
    /// Error type name
    pub error_type: Option<String>,
    /// For async_method/streaming: the owning type name
    pub owner_type: Option<String>,
    /// For streaming: the item type
    pub item_type: Option<String>,
    /// For Python: release GIL during call
    #[serde(default)]
    pub gil_release: bool,
    /// For callback_bridge: the Rust trait to implement (e.g., "MyHandler")
    #[serde(default)]
    pub trait_name: Option<String>,
    /// For callback_bridge: the trait method name (e.g., "handle")
    #[serde(default)]
    pub trait_method: Option<String>,
    /// For callback_bridge: whether to detect async callbacks at construction time
    #[serde(default)]
    pub detect_async: bool,
    /// For streaming (FFI backend): full Rust type path of the request payload
    /// deserialised from JSON (e.g. `"my_crate::ChatCompletionRequest"`).
    /// Required when generating FFI streaming bodies — codegen will hard-fail
    /// with a clear error if this field is absent on a streaming adapter.
    #[serde(default)]
    pub request_type: Option<String>,
    /// Language backends for which this adapter should NOT be emitted.
    ///
    /// Mirrors the same field on `[[crates.e2e.calls.*]]`. Useful when a
    /// consumer's core crate cannot compile on a given target (e.g.
    /// a streaming crate on `wasm32-unknown-unknown` which has no working async
    /// runtime for streaming). The adapter remains declared for every backend
    /// where it works, with explicit per-backend opt-out rather than removing
    /// the adapter entirely.
    ///
    /// Values must match a canonical [`Language`] name (see [`Language::ALL`]).
    /// An unknown name fails at config-resolve time.
    ///
    /// Example: `skip_languages = ["wasm", "kotlin"]`
    #[serde(default)]
    pub skip_languages: Vec<String>,
}

/// Returns `true` when `lang_str` is a recognised canonical language name.
///
/// Derived from [`Language::ALL`] rather than a second hand-typed name list, so a variant
/// added to the enum is recognised here without a separate edit to keep in sync.
pub fn is_known_language(lang_str: &str) -> bool {
    Language::ALL.iter().any(|language| language.to_string() == lang_str)
}

/// Returns whether `adapter.owner_type` names `owner_type` exactly.
pub(crate) fn adapter_owner_matches(adapter: &AdapterConfig, owner_type: &str) -> bool {
    adapter.owner_type.as_deref() == Some(owner_type)
}

/// Returns whether `adapter.core_path`'s final `::`-delimited segment names `method_name`.
///
/// `core_path` is documented on [`AdapterConfig::core_path`] as the *fully-qualified* Rust path
/// to the core function/method (e.g. `sample_markdown_rs::convert`), so a consumer following
/// that contract writes a qualified path such as `sample_stream_rs::DefaultClient::chat_stream` — not a
/// bare method name. Comparing the whole string against a bare method name therefore never
/// matches a config written the way the field's own doc comment tells consumers to write it;
/// stripping to the last segment is what honours the documented contract. Both sides go through
/// `to_snake_case` so a `core_path` segment spelled in a different case style still matches the
/// IR's snake_case method name.
pub(crate) fn adapter_method_name_matches(adapter: &AdapterConfig, method_name: &str) -> bool {
    let method_name = method_name.to_snake_case();
    adapter
        .core_path
        .rsplit("::")
        .next()
        .is_some_and(|segment| segment.to_snake_case() == method_name)
}

/// Returns whether `adapter` is the `[[crates.adapters]]` entry that handles
/// `owner_type::method_name`.
///
/// This is the shared predicate every "does this adapter cover this method" call site must go
/// through: `mark_adapter_handled_methods` (`cli/pipeline/extract/services.rs`) and
/// `adapter_binds_method_for_language` (`e2e/snippets/exclusions.rs`) previously each answered
/// this question with their own exact-tuple-equality comparison, which never matched a
/// fully-qualified `core_path` as documented above.
pub(crate) fn adapter_covers_method(adapter: &AdapterConfig, owner_type: &str, method_name: &str) -> bool {
    adapter_owner_matches(adapter, owner_type) && adapter_method_name_matches(adapter, method_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_config_skip_languages_deserializes() {
        let toml_str = r#"
            name = "crawl_stream"
            pattern = "streaming"
            core_path = "sample_crawler_core::crawl_stream"
            owner_type = "CrawlEngine"
            item_type = "CrawlResult"
            skip_languages = ["wasm", "kotlin"]
        "#;
        let config: AdapterConfig = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.skip_languages, vec!["wasm", "kotlin"]);
        assert_eq!(config.name, "crawl_stream");
    }

    #[test]
    fn adapter_config_skip_languages_defaults_to_empty() {
        let toml_str = r#"
            name = "crawl_stream"
            pattern = "streaming"
            core_path = "sample_crawler_core::crawl_stream"
        "#;
        let config: AdapterConfig = toml::from_str(toml_str).expect("deserialization failed");
        assert!(config.skip_languages.is_empty());
    }

    /// Iterates `Language::ALL` rather than a hand-typed name list, so this test stays coupled
    /// to the enum instead of drifting alongside a second copy of it: a variant whose canonical
    /// name `is_known_language` fails to recognise is caught here without editing the test.
    #[test]
    fn is_known_language_accepts_every_all_variant() {
        for language in Language::ALL {
            let name = language.to_string();
            assert!(is_known_language(&name), "{name} should be recognised");
        }
    }

    fn adapter_for_parity(name: &str, owner_type: &str, core_path: &str) -> AdapterConfig {
        AdapterConfig {
            name: name.to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: core_path.to_string(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some(owner_type.to_string()),
            item_type: None,
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }
    }

    /// Table-driven parity guard for [`adapter_covers_method`], the single predicate
    /// `mark_adapter_handled_methods` (`cli/pipeline/extract/services.rs`) and
    /// `adapter_binds_method_for_language` (`e2e/snippets/exclusions.rs`) both call
    /// unconditionally, and that `docs::language_pages::streaming::streaming_adapter_matches_method`
    /// calls as one arm of its own match (the other being a docs-only `adapter.name` fallback).
    /// Before this predicate existed, each of those three sites carried its own
    /// string-comparison logic and disagreed on every row below except the bare-name case. This
    /// is the guard against them diverging again: any future edit to one caller that stops
    /// routing through this function is invisible to this test, but any edit to the predicate
    /// itself that breaks one of these documented cases is caught here.
    #[test]
    fn adapter_covers_method_parity_table() {
        struct Case {
            description: &'static str,
            core_path: &'static str,
            owner_type: &'static str,
            queried_owner: &'static str,
            queried_method: &'static str,
            expected: bool,
        }
        let cases = [
            Case {
                description: "bare core_path equal to the method name",
                core_path: "chat_stream",
                owner_type: "DefaultClient",
                queried_owner: "DefaultClient",
                queried_method: "chat_stream",
                expected: true,
            },
            Case {
                description: "fully-qualified core_path, as AdapterConfig::core_path documents",
                core_path: "sample_stream_rs::DefaultClient::chat_stream",
                owner_type: "DefaultClient",
                queried_owner: "DefaultClient",
                queried_method: "chat_stream",
                expected: true,
            },
            Case {
                description: "qualified core_path naming the right method but a different owner",
                core_path: "sample_stream_rs::DefaultClient::chat_stream",
                owner_type: "DefaultClient",
                queried_owner: "OtherClient",
                queried_method: "chat_stream",
                expected: false,
            },
            Case {
                description: "non-matching method name entirely",
                core_path: "sample_stream_rs::DefaultClient::chat_stream",
                owner_type: "DefaultClient",
                queried_owner: "DefaultClient",
                queried_method: "other_method",
                expected: false,
            },
        ];

        for case in cases {
            let adapter = adapter_for_parity("chat_stream", case.owner_type, case.core_path);
            let actual = adapter_covers_method(&adapter, case.queried_owner, case.queried_method);
            assert_eq!(
                actual, case.expected,
                "{}: adapter_covers_method({:?}, {:?}, {:?}) should be {}",
                case.description, adapter, case.queried_owner, case.queried_method, case.expected
            );
        }
    }

    #[test]
    fn is_known_language_rejects_unknown() {
        assert!(!is_known_language("wasm32"));
        assert!(!is_known_language("kotlin-android"));
        assert!(!is_known_language(""));
    }

    #[test]
    fn all_lists_every_variant() {
        for language in Language::ALL {
            // ~keep: exhaustive over Language, so adding a variant stops this file
            // compiling until it is also added to ALL — nothing else enforces that.
            match language {
                Language::Python
                | Language::Node
                | Language::Ruby
                | Language::Php
                | Language::Elixir
                | Language::Wasm
                | Language::Ffi
                | Language::Go
                | Language::Java
                | Language::Csharp
                | Language::R
                | Language::Rust
                | Language::Kotlin
                | Language::KotlinAndroid
                | Language::Swift
                | Language::Dart
                | Language::Gleam
                | Language::Zig
                | Language::C
                | Language::Jni => {}
            }
        }
        let distinct: std::collections::HashSet<String> = Language::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(
            distinct.len(),
            Language::ALL.len(),
            "Language::ALL must not repeat a variant"
        );
    }
}
