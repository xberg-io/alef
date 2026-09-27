//! Per-fixture, per-language exclusion checks the snippet driver's `expected` gate consults
//! before a coverage cell is ever pushed.
//!
//! Split out of `mod.rs` to keep that file under the repo's per-file line cap: these three
//! predicates share no state with the rest of the driver beyond [`super::SnippetRenderContext`]
//! and [`super::DocumentationLanguage`], which they take as ordinary parameters.

use super::{DocumentationLanguage, SnippetRenderContext, parse_language};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::e2e::fixture::Fixture;

/// Whether the function a fixture's call resolves to for `language` is excluded for that
/// language, and therefore can never be rendered into a snippet.
///
/// Reuses [`crate::docs::language_pages::excludes::language_excludes`] -- the accessor the
/// docs generator already consults for the same question -- rather than re-deriving the
/// per-language `exclude_functions` union here. A second copy of that rule is exactly how a
/// ledger and its emitter drift apart: one path evolves (a language gains an override, a new
/// per-language config field is added) and the other silently keeps checking the old shape.
/// [`CallConfig::core_lookup_name`] gives the Rust-spelled identity `exclude_functions`
/// entries are keyed by, matching every built-in snippet recipe's own resolution (see
/// `e2e/codegen/go/snippet.rs`, `kotlin/snippet.rs`, `php/snippet.rs`, `ruby/snippet.rs`, and
/// the WASM-specific `rust_identity_for_wasm_symbol`, which resolves the same identity for the
/// one target that also accepts the JS spelling of an override). ~keep
pub(super) fn function_excluded_for_language(
    fixture: &Fixture,
    language: &str,
    generator_language_name: &str,
    context: &SnippetRenderContext<'_>,
) -> bool {
    let Some(DocumentationLanguage::Binding(lang)) = parse_language(generator_language_name) else {
        return false;
    };
    let docs_fixture = fixture.docs_call_fixture();
    let call = context.e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let Some(function_name) = call.core_lookup_name(language) else {
        return false;
    };
    let (excluded_functions, _) = crate::docs::language_pages::excludes::language_excludes(context.crate_config, lang);
    excluded_functions.contains(function_name.as_ref())
}

/// Whether `fixture` exercises the fixture engine's generic visitor/trait-bridge entry point
/// ([`Fixture::visitor`]) and this language excludes it via the
/// [`crate::e2e::fixture::VISITOR_EXCLUDE_FUNCTION_NAME`] convention.
///
/// `exclude_functions` normally names a real Rust function, so
/// [`function_excluded_for_language`] cannot catch this case: a visitor fixture's *call*
/// resolves to some ordinary function (e.g. `convert`), while the visitor itself attaches
/// through a trait-bridge parameter or options-struct field that never has its own IR function
/// name. `e2e::codegen::kotlin_android::project` already applies this exact rule when deciding
/// whether to emit a fixture's Kotlin-Android e2e test or fall back to
/// `ExcludedBindingsTest.kt`; without the matching check here, snippet generation rendered real
/// code against a visitor API the binding never exposed for that language. ~keep
pub(super) fn visitor_excluded_for_language(
    fixture: &Fixture,
    generator_language_name: &str,
    context: &SnippetRenderContext<'_>,
) -> bool {
    if fixture.visitor.is_none() {
        return false;
    }
    let Some(DocumentationLanguage::Binding(lang)) = parse_language(generator_language_name) else {
        return false;
    };
    let (excluded_functions, _) = crate::docs::language_pages::excludes::language_excludes(context.crate_config, lang);
    excluded_functions.contains(crate::e2e::fixture::VISITOR_EXCLUDE_FUNCTION_NAME)
}

/// Whether the function or method a fixture's call resolves to for `language` is
/// `binding_excluded` in the IR -- i.e. marked `#[alef::skip]`/`#[doc(hidden)]` at
/// extraction time, which excludes it from every generated binding regardless of what
/// `alef.toml` configures.
///
/// This is deliberately separate from [`function_excluded_for_language`]:  that helper
/// only ever consults `alef.toml`-configured `exclude_functions` lists via
/// [`crate::docs::language_pages::excludes::language_excludes`], which never reads
/// `FunctionDef::binding_excluded` / `MethodDef::binding_excluded` at all -- see
/// `src/snippets/gaps.rs`'s `LedgerExpectations` doc comment, which documented this exact
/// gap before this function closed it. Without this check a function a Rust author
/// explicitly opted out of every binding still entered `coverage.expected` for every
/// non-Rust language, and the snippet coverage ledger reported the resulting absence as a
/// gap the consumer has no `alef.toml` knob to silence -- it was never a gap.
///
/// Mirrors the `lang == Language::Rust || !binding_excluded` rule
/// `docs::language_pages::mod::generate_lang_doc` already applies for the same flag: the
/// Rust documentation page still lists a `binding_excluded` item (it exists in Rust
/// source, it is just not exposed to other-language bindings), so `"rust"` is carved out
/// here too and never treated as excluded.
///
/// Resolution mirrors [`crate::e2e::codegen::call_ir::CallIr::signature`]'s
/// free-function-first, then agreeing-methods fallback, but answers the `binding_excluded`
/// question instead of a signature: a free function of the resolved name wins
/// unambiguously (a crate has at most one `pub fn` of a given path), and when only methods
/// match, every same-named method across every type must agree on the flag or this
/// answers `false` (not excluded) rather than guessing. This mirrors `CallIr::signature`'s
/// conservatism on disagreement -- "learned nothing" -- deliberately: treating
/// disagreement as excluded would drop a cell that is still fully bindable through at
/// least one of the disagreeing types, which is worse than occasionally letting a
/// genuinely-excluded cell surface as a coverage gap a human can then triage with a
/// `docs.coverage_exceptions` entry. ~keep
///
/// A method can land at `binding_excluded == true` for two structurally different reasons
/// that this IR flag does not otherwise distinguish -- see [`adapter_binds_method_for_language`]
/// for why an adapter-handled method must not be treated as excluded here. ~keep
pub(super) fn function_binding_excluded_for_language(
    fixture: &Fixture,
    language: &str,
    generator_language_name: &str,
    context: &SnippetRenderContext<'_>,
) -> bool {
    let Some(DocumentationLanguage::Binding(lang)) = parse_language(generator_language_name) else {
        return false;
    };
    if lang == Language::Rust {
        return false;
    }
    let docs_fixture = fixture.docs_call_fixture();
    let call = context.e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let Some(function_name) = call.core_lookup_name(language) else {
        return false;
    };
    if let Some(function) = context.functions.iter().find(|function| function.name == function_name) {
        return function.binding_excluded;
    }
    let mut methods = context
        .type_defs
        .iter()
        .flat_map(|type_def| {
            type_def
                .methods
                .iter()
                .map(move |method| (type_def.name.as_str(), method))
        })
        .filter(|(_, method)| method.name == function_name);
    let Some((first_type, first)) = methods.next() else {
        return false;
    };
    if !methods.all(|(_, other)| other.binding_excluded == first.binding_excluded) {
        return false;
    }
    if !first.binding_excluded {
        return false;
    }
    !adapter_binds_method_for_language(context.crate_config, first_type, function_name.as_ref(), lang)
}

/// Whether `[[crates.adapters]]` gives `type_name.method_name` a real binding surface in
/// `lang`, even though `MethodDef::binding_excluded` is `true` for it.
///
/// `mark_adapter_handled_methods` (`src/cli/pipeline/extract/services.rs`) sets
/// `binding_excluded = true` on every method matched by an adapter's `owner_type` +
/// `core_path`, with no per-language distinction at all -- it exists only to stop the
/// *generic* method codegen path from double-emitting a method a backend's adapter
/// machinery (async-method, streaming, callback-bridge) already handles through its own
/// specialised template. It says nothing about whether that language actually binds the
/// method: every backend that consumes `[[crates.adapters]]` (pyo3, napi, magnus, dart,
/// swift, kotlin, java, php, wasm -- see the `skip_languages` call sites in each backend's
/// `gen_bindings/mod.rs`) still emits the method for every language *except* the ones the
/// adapter's own `skip_languages` names; languages outside that pattern's backend set fall
/// through to the ordinary per-method codegen loop, which does not filter on
/// `binding_excluded` at the method level either (only `TypeDef::binding_excluded` gates a
/// whole type). So an adapter-handled method has a binding surface in `lang` unless this
/// specific adapter entry explicitly names `lang` in `skip_languages` -- re-deriving that
/// per-language answer from the same config `mark_adapter_handled_methods` read, rather
/// than trusting the language-blind flag it wrote, is what keeps this in sync with what
/// every backend's `skip_languages` filter already decides for real codegen. ~keep
fn adapter_binds_method_for_language(
    crate_config: &ResolvedCrateConfig,
    type_name: &str,
    method_name: &str,
    lang: Language,
) -> bool {
    let lang_name = lang.to_string();
    crate_config.adapters.iter().any(|adapter| {
        crate::core::config::extras::adapter_covers_method(adapter, type_name, method_name)
            && !adapter.skip_languages.contains(&lang_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{AdapterConfig, AdapterPattern};

    fn streaming_adapter(owner_type: &str, core_path: &str, skip_languages: Vec<String>) -> AdapterConfig {
        AdapterConfig {
            name: "chat_stream".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: core_path.to_string(),
            params: Vec::new(),
            returns: None,
            error_type: None,
            owner_type: Some(owner_type.to_string()),
            item_type: Some("ChatCompletionChunk".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages,
        }
    }

    /// Regression for the bug this issue found: the pre-fix `adapter.core_path == method_name`
    /// exact-string comparison never matched a fully-qualified `core_path`, the shape
    /// [`crate::core::config::AdapterConfig::core_path`] documents as the expected one
    /// (`sample_stream_rs::DefaultClient::chat_stream`). A method that was in fact adapter-handled was
    /// then treated as a genuine exclusion by
    /// [`function_binding_excluded_for_language`], dropping it from the snippet coverage
    /// ledger's `expected` set even though every backend still binds it.
    #[test]
    fn adapter_binds_method_for_language_matches_a_fully_qualified_core_path() {
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter(
                "DefaultClient",
                "sample_stream_rs::DefaultClient::chat_stream",
                Vec::new(),
            )],
            ..ResolvedCrateConfig::default()
        };

        assert!(adapter_binds_method_for_language(
            &config,
            "DefaultClient",
            "chat_stream",
            Language::Python
        ));
    }

    /// The bare `core_path` shape every existing fixture uses must keep matching exactly as
    /// before.
    #[test]
    fn adapter_binds_method_for_language_matches_a_bare_core_path() {
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter("DefaultClient", "chat_stream", Vec::new())],
            ..ResolvedCrateConfig::default()
        };

        assert!(adapter_binds_method_for_language(
            &config,
            "DefaultClient",
            "chat_stream",
            Language::Python
        ));
    }

    /// A qualified `core_path` naming the right method but a different owner type must not
    /// match.
    #[test]
    fn adapter_binds_method_for_language_does_not_match_a_different_owner() {
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter(
                "DefaultClient",
                "sample_stream_rs::DefaultClient::chat_stream",
                Vec::new(),
            )],
            ..ResolvedCrateConfig::default()
        };

        assert!(!adapter_binds_method_for_language(
            &config,
            "OtherClient",
            "chat_stream",
            Language::Python
        ));
    }

    /// A language named in the adapter's own `skip_languages` still gets no binding surface
    /// from the adapter, qualified `core_path` or not.
    #[test]
    fn adapter_binds_method_for_language_respects_skip_languages() {
        let config = ResolvedCrateConfig {
            adapters: vec![streaming_adapter(
                "DefaultClient",
                "sample_stream_rs::DefaultClient::chat_stream",
                vec!["python".to_string()],
            )],
            ..ResolvedCrateConfig::default()
        };

        assert!(!adapter_binds_method_for_language(
            &config,
            "DefaultClient",
            "chat_stream",
            Language::Python
        ));
        assert!(adapter_binds_method_for_language(
            &config,
            "DefaultClient",
            "chat_stream",
            Language::Node
        ));
    }
}
