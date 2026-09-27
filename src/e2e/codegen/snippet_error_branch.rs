//! The one shared decision point for whether a documentation snippet may catch a *specific*
//! error variant rather than the crate's flat error type.
//!
//! ~keep A generated error snippet renders `try { .. } catch (Error error) { print(type(error)) }`
//! — one generic branch that teaches "catch everything and print the class name". The
//! hand-written error-handling guides it was meant to replace teach the opposite: one branch per
//! failure mode, because that is the decision a reader actually has to make (rotate the key vs.
//! back off vs. trim the prompt). The information needed to close that gap is already in the IR
//! and already generated into the bindings — `pyo3::create_exception!` emits a distinct exception
//! class per `ErrorVariant`, and the fixture names the variant it provokes in its `error`
//! assertion — but no snippet renderer ever joined the two.
//!
//! ~keep This module deliberately reuses [`super::declared_error_variant`]'s verdict rather than
//! deciding again. That module already answers, per backend, "can this binding tell one variant
//! of an error type from another?" — with a per-backend audit recorded next to the answer. A
//! snippet that catches `AuthenticationError` is exactly the same claim as an e2e assertion that
//! the thrown type is `AuthenticationError`: if one is unsubstantiable the other is too. Two
//! functions answering that question independently is this codebase's recurring defect shape, so
//! there is only one, and [`tests::branch_and_classify_never_disagree_across_backends`] pins that
//! they cannot drift apart.
//!
//! Degradation is the default: [`for_fixture`] returns `None` for every fixture that does not
//! name a real variant and for every backend whose binding cannot differentiate one, and every
//! caller renders its pre-existing generic branch unchanged in that case.

use crate::core::ir::{ErrorDef, ErrorVariant};
use crate::e2e::fixture::Fixture;

/// A single typed catch branch a snippet can render ahead of its generic catch-all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnippetErrorBranch {
    /// The IR variant name, verbatim (e.g. `Authentication`).
    pub variant: String,
    /// The host-language type the generated binding actually exposes for that variant
    /// (e.g. Python `AuthenticationError`, Go `ErrAuthentication`).
    pub host_type: String,
}

/// The typed catch branch `fixture` licenses in `lang`, if any.
///
/// `errors` is `ApiSurface::errors`, the crate's full error-type registry. Returns `None` — the
/// generic-catch-all fallback — when the fixture declares no `error` value, when the declared
/// value is a message substring rather than a variant name, or when `lang`'s generated binding
/// cannot distinguish the named variant from any other variant of the same type.
pub(crate) fn for_fixture(lang: &str, fixture: &Fixture, errors: &[ErrorDef]) -> Option<SnippetErrorBranch> {
    let (error, variant) = super::declared_error_variant::declared_variant(fixture, errors)?;
    if !super::declared_error_variant::substantiates_variant_identity(lang, variant) {
        return None;
    }
    Some(SnippetErrorBranch {
        variant: variant.name.clone(),
        host_type: host_error_type(lang, &error.name, variant, errors)?,
    })
}

/// The host-language type name a generated binding gives `variant`, per backend.
///
/// ~keep Every arm delegates to the naming helper the *binding* generator itself calls, so a
/// snippet can never name a type the binding does not define. This table cannot widen the
/// verdict — [`for_fixture`] consults
/// [`super::declared_error_variant::substantiates_variant_identity`] first and returns before
/// reaching here — so the only way the two can disagree is a language becoming substantiable
/// with no name registered for it. That is the direction that actually happens (a backend gains
/// per-variant identity, or a new backend is added), and
/// [`tests::branch_and_classify_never_disagree_across_backends`] fails on it.
fn host_error_type(lang: &str, error_name: &str, variant: &ErrorVariant, errors: &[ErrorDef]) -> Option<String> {
    match lang {
        // ~keep `backends/pyo3/gen_bindings/errors.rs` re-exports one `create_exception!` class
        // per variant from the package root.
        "python" => Some(crate::codegen::error_gen::python_exception_name(
            &variant.name,
            error_name,
        )),
        // ~keep `backends/go/gen_bindings/types/helpers.rs::gen_last_error_helper` maps the FFI
        // taxonomy code to this sentinel.
        "go" => Some(crate::codegen::error_gen::go_error_sentinel_name(
            errors,
            error_name,
            &variant.name,
        )),
        // ~keep `backends/java/gen_bindings/marshal.rs::emit_error_helper`.
        "java" => Some(format!("{}Exception", variant.name)),
        // ~keep `backends/zig/gen_bindings/helpers.rs` dispatches to this error-set member.
        "zig" => Some(format!(
            "error.{}",
            crate::codegen::naming::cased_public_type_name(
                crate::core::config::Language::Zig,
                &variant.name,
            )
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{SnippetErrorBranch, for_fixture};
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify};
    use crate::e2e::fixture::{Assertion, Fixture};

    /// Every language string a `classify` call site threads through, plus `python`. Mirrors
    /// `declared_error_variant::tests::every_classify_backend_finds_a_value_declared_after_a_bare_check`
    /// so a backend added to one sweep is visible as missing from the other.
    const BACKENDS: &[&str] = &[
        "python", "go", "csharp", "java", "zig", "dart", "ruby", "c", "php", "swift", "gleam", "elixir", "r", "node",
    ];

    fn fixture_naming(variant: &str) -> Fixture {
        Fixture {
            id: "auth_401".to_string(),
            assertions: vec![
                Assertion {
                    assertion_type: "error".to_string(),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "error".to_string(),
                    value: Some(serde_json::Value::String(variant.to_string())),
                    ..Assertion::default()
                },
            ],
            ..Fixture::default()
        }
    }

    fn variant(name: &str, code: Option<u32>) -> ErrorVariant {
        ErrorVariant {
            name: name.to_string(),
            error_code: code,
            is_unit: true,
            ..ErrorVariant::default()
        }
    }

    fn errors_with(variants: Vec<ErrorVariant>) -> Vec<ErrorDef> {
        vec![ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants,
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    /// The cross-backend guard. For a fixture naming a real, ABI-coded variant, every backend's
    /// snippet branch must exist for exactly the backends whose e2e assertion path calls the
    /// declared value substantiable — the snippet and the assertion make the same claim about the
    /// same variant, so they must never answer differently. A future backend that gains (or
    /// loses) per-variant identity flips both sides at once because both read the same verdict
    /// function; a per-backend reimplementation of either side fails here.
    #[test]
    fn branch_and_classify_never_disagree_across_backends() {
        let fixture = fixture_naming("Authentication");
        let errors = errors_with(vec![variant("Authentication", Some(100))]);
        for lang in BACKENDS {
            let branch = for_fixture(lang, &fixture, &errors);
            let substantiable = classify(lang, &fixture, &errors) == DeclaredErrorAssertion::Assert("Authentication");
            assert_eq!(
                branch.is_some(),
                substantiable,
                "lang={lang}: snippet branch {branch:?} disagrees with the e2e assertion verdict"
            );
        }
    }

    /// An uncoded variant is substantiable only where identity does not ride on the FFI taxonomy
    /// (Python). The same sweep, with the one input that separates the two groups.
    #[test]
    fn an_uncoded_variant_branches_only_where_identity_is_not_abi_derived() {
        let fixture = fixture_naming("Authentication");
        let errors = errors_with(vec![variant("Authentication", None)]);
        for lang in BACKENDS {
            let branch = for_fixture(lang, &fixture, &errors);
            assert_eq!(
                branch.is_some(),
                *lang == "python",
                "lang={lang}: an uncoded variant yielded {branch:?}"
            );
        }
    }

    #[test]
    fn python_names_the_generated_exception_class() {
        let fixture = fixture_naming("Authentication");
        let errors = errors_with(vec![variant("Authentication", None)]);
        assert_eq!(
            for_fixture("python", &fixture, &errors),
            Some(SnippetErrorBranch {
                variant: "Authentication".to_string(),
                host_type: "AuthenticationError".to_string(),
            })
        );
    }

    /// The deliberate fallback: a message-style value names no variant, so every backend keeps
    /// its generic catch-all. This is the majority of fixtures and must not change.
    #[test]
    fn a_message_style_value_never_branches() {
        let fixture = fixture_naming("size must be positive");
        let errors = errors_with(vec![variant("Authentication", Some(100))]);
        for lang in BACKENDS {
            assert_eq!(for_fixture(lang, &fixture, &errors), None, "lang={lang}");
        }
    }

    /// No IR error registry threaded through (how most snippet call sites still run) means no
    /// variant can be recognised, so output stays byte-identical to before this module existed.
    #[test]
    fn no_error_registry_never_branches() {
        let fixture = fixture_naming("Authentication");
        for lang in BACKENDS {
            assert_eq!(for_fixture(lang, &fixture, &[]), None, "lang={lang}");
        }
    }
}
