//! Config resolution helpers and bytes classification for Python e2e tests.

use std::collections::{HashMap, HashSet};

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

// ---------------------------------------------------------------------------
// Config resolution
// ---------------------------------------------------------------------------

pub(super) fn resolve_function_name(e2e_config: &E2eConfig) -> String {
    resolve_function_name_for_call(&e2e_config.call)
}

pub(super) fn resolve_function_name_for_call(call_config: &crate::e2e::config::CallConfig) -> String {
    call_config
        .overrides
        .get("python")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| call_config.function.clone())
}

pub(super) fn resolve_module(e2e_config: &E2eConfig) -> String {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.module.clone())
        .unwrap_or_else(|| e2e_config.call.module.replace('-', "_"))
}

pub(super) fn resolve_options_type(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_type.clone())
}

/// Resolve the client factory function name from the Python override config.
///
/// When set, the generated test creates a client instance via `factory("test-key", base_url)`
/// and dispatches API calls as methods on the client rather than top-level functions.
pub(super) fn resolve_client_factory(e2e_config: &E2eConfig) -> Option<String> {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.client_factory.clone())
}

/// Resolve how json_object args are passed: "kwargs" (default), "dict", or "json".
pub(super) fn resolve_options_via(e2e_config: &E2eConfig) -> &str {
    e2e_config
        .call
        .overrides
        .get("python")
        .and_then(|o| o.options_via.as_deref())
        .unwrap_or("kwargs")
}

/// Compute the exact type-name set the pyo3 backend injects a `from_json()` staticmethod for.
///
/// pyo3's gate (`src/backends/pyo3/gen_bindings/mod.rs`) is the conjunction
/// `has_serde && core_to_binding_convertible_types(api, &[]).contains(&typ.name)` — `has_serde`
/// alone is necessary but not sufficient, since a serde-derived type can still fail the
/// transitive core→binding convertibility fixpoint (e.g. a field whose type has no matching
/// binding conversion). Calling the real `core_to_binding_convertible_types` here — rather than
/// re-deriving convertibility from `type_defs`/`enums` by hand — keeps this in lockstep with
/// pyo3 even if that algorithm changes; it reads only `surface.types`/`surface.enums`, so the
/// synthetic surface built from the same two IR slices already threaded through e2e codegen is
/// faithful to the real one. ~keep
pub(super) fn core_to_binding_convertible_types(
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> ahash::AHashSet<String> {
    let surface = crate::core::ir::ApiSurface {
        types: type_defs.to_vec(),
        enums: enums.to_vec(),
        ..Default::default()
    };
    crate::codegen::conversions::core_to_binding_convertible_types(&surface, &[])
}

/// Mirrors pyo3's own gate for injecting a `from_json()` staticmethod into a type's generated
/// Rust `impl` block — the shared [`crate::codegen::conversions::pyo3_from_json_eligible`]
/// predicate, requiring per-type serde derives, crate-level serde availability, and
/// core<->binding convertibility (see `src/backends/pyo3/gen_bindings/types.rs`). As of
/// `093c42f31`, `type_has_from_json` is the single predicate shared by both the raw-text
/// `#[pymethods]` injection and the `.pyi` stub generator, so passing this gate also means the
/// shipped stub declares the method — verified against a consumer's `CreateImageRequest`, where
/// `_internal_bindings.pyi` carries `def from_json`.
///
/// This says only what the *native* `#[pyclass]` carries. It says nothing about which class a
/// type's public name actually resolves to — see [`options_wrapped_type_names`]. ~keep
pub(super) fn pyo3_would_inject_from_json(
    name: &str,
    type_defs: &[crate::core::ir::TypeDef],
    convertible_types: &ahash::AHashSet<String>,
    crate_has_serde: bool,
) -> bool {
    type_defs
        .iter()
        .find(|t| t.name == name)
        .is_some_and(|t| crate::codegen::conversions::pyo3_from_json_eligible(t, crate_has_serde, convertible_types))
}

/// Names of the types whose public Python spelling is `options.py`'s `@dataclass` mirror rather
/// than the native `#[pyclass]` — the exact union `__init__.py`/`api.py` import from `.options`
/// (`options_dataclass_type_names` ∪ `options_return_dataclass_names`,
/// `src/backends/pyo3/gen_bindings/types.rs`). `gen_options_py` never emits a method on any
/// dataclass it writes (see that function's doc comment): a type in this set exposes `from_json`
/// -- or any other native-only method -- through its public name only by accident of a caller
/// never checking, regardless of whether pyo3 injected `from_json` on the native class
/// underneath. Built from a synthetic `ApiSurface` over the same `type_defs`/`enums` slices
/// already threaded through e2e codegen, mirroring [`core_to_binding_convertible_types`] above,
/// so this reads the pyo3 backend's actual export decision instead of re-deriving a third,
/// independently-drifting copy of it. ~keep
pub(super) fn options_wrapped_type_names(
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    dto: &crate::core::config::DtoConfig,
    reexported_types: &[String],
) -> HashSet<String> {
    let surface = crate::core::ir::ApiSurface {
        types: type_defs.to_vec(),
        enums: enums.to_vec(),
        ..Default::default()
    };
    let mut names = crate::backends::pyo3::gen_bindings::options_dataclass_type_names(&surface, reexported_types);
    names.extend(crate::backends::pyo3::gen_bindings::options_return_dataclass_names(
        &surface,
        dto,
        reexported_types,
    ));
    names
}

/// Downgrade `options_via` from `"from_json"` to `"kwargs"` unless the target type's *public*
/// name actually resolves to a class carrying `from_json`. Two independent conditions must both
/// hold:
///
/// 1. The name is not in [`options_wrapped_type_names`] -- otherwise `__init__.py` exports it
///    from `.options`, a plain dataclass with no methods at all, and `from_json` is unreachable
///    through the public name no matter what the native class underneath carries.
/// 2. Failing that (or when there's no wrapper for the type at all), the native class itself must
///    pass pyo3's Rust-codegen gate ([`pyo3_would_inject_from_json`]) -- the gate that also
///    controls whether the `.pyi` stub declares the method, so passing it guarantees the emitted
///    call type-checks against the shipped stub.
///
/// Every generated DTO still exposes a plain kwargs constructor, so falling back there keeps the
/// emitted call valid for types that clear neither condition. ~keep
pub(super) fn effective_options_via_for_type<'a>(
    options_via: &'a str,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    convertible_types: &ahash::AHashSet<String>,
    crate_has_serde: bool,
    options_wrapped_types: &HashSet<String>,
) -> &'a str {
    if options_via != "from_json" {
        return options_via;
    }
    let Some(name) = options_type else {
        return "kwargs";
    };
    if options_wrapped_types.contains(name) {
        return "kwargs";
    }
    if pyo3_would_inject_from_json(name, type_defs, convertible_types, crate_has_serde) {
        options_via
    } else {
        "kwargs"
    }
}

/// Resolve enum field mappings from the Python override config.
pub(super) fn resolve_enum_fields(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.enum_fields)
        .unwrap_or(&EMPTY)
}

/// Resolve per-call result-field enum mappings from the Python override config.
///
/// Returns the `assert_enum_fields` map from the Python override block for
/// `call_config`, falling back to an empty map when no override is present.
pub(super) fn resolve_assert_enum_fields(call_config: &crate::e2e::config::CallConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    call_config
        .overrides
        .get("python")
        .map(|o| &o.assert_enum_fields)
        .unwrap_or(&EMPTY)
}

/// Resolve handle nested type mappings from the Python override config.
pub(super) fn resolve_handle_nested_types(e2e_config: &E2eConfig) -> &HashMap<String, String> {
    static EMPTY: std::sync::LazyLock<HashMap<String, String>> = std::sync::LazyLock::new(HashMap::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_nested_types)
        .unwrap_or(&EMPTY)
}

/// Resolve handle dict type set from the Python override config.
pub(super) fn resolve_handle_dict_types(e2e_config: &E2eConfig) -> &HashSet<String> {
    static EMPTY: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(HashSet::new);
    e2e_config
        .call
        .overrides
        .get("python")
        .map(|o| &o.handle_dict_types)
        .unwrap_or(&EMPTY)
}

pub(super) fn is_skipped(fixture: &Fixture, language: &str) -> bool {
    fixture.skip.as_ref().is_some_and(|s| s.should_skip(language))
}

// ---------------------------------------------------------------------------
// Bytes classification
// ---------------------------------------------------------------------------

/// How to represent a fixture `type = "bytes"` string value in generated Python.
pub(super) enum BytesKind {
    /// A relative file path like `"pdf/fake_memo.pdf"` — read with `Path(...).read_bytes()`.
    FilePath,
    /// Inline text content like `"<!DOCTYPE html>..."` — encode to `b"..."`.
    InlineText,
    /// A base64-encoded blob like `"/9j/4AAQ"` — decode with `base64.b64decode(...)`.
    Base64,
}

/// Classify a fixture string value that maps to a `bytes` argument.
pub(super) fn classify_bytes_value(s: &str) -> BytesKind {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return BytesKind::InlineText;
    }

    let first = s.chars().next().unwrap_or('\0');
    if (first.is_ascii_alphanumeric() || first == '_')
        && let Some(slash_pos) = s.find('/')
        && slash_pos > 0
    {
        let after_slash = &s[slash_pos + 1..];
        if after_slash.contains('.') && !after_slash.is_empty() {
            return BytesKind::FilePath;
        }
    }

    BytesKind::Base64
}

/// Returns the Python import name for a method_result method that uses a
/// module-level helper function (not a method on the result object).
pub(super) fn python_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("tree_has_error_nodes".to_string()),
        "error_count" | "tree_error_count" => Some("tree_error_count".to_string()),
        "tree_to_sexp" => Some("tree_to_sexp".to_string()),
        "contains_node_type" => Some("tree_contains_node_type".to_string()),
        "find_nodes_by_type" => Some("find_nodes_by_type".to_string()),
        "run_query" => Some("run_query".to_string()),
        _ => None,
    }
}

/// Strip one redundant enclosing `(...)` pair from `expr`, for use when `expr` is about to be
/// placed in call-argument position (e.g. the sole content between a call's own parens, as in
/// `len({expr})`).
///
/// A field accessor built from an `Optional` narrowing crossing (`render_python_with_optionals`)
/// already carries its own enclosing parens: `(result.markdown.content if result.markdown else
/// None)`. Those parens are load-bearing where the accessor is embedded in a larger expression
/// whose own grammar would otherwise bind past the ternary (`{expr}.startswith(x)`, `{expr} is
/// not None`, ...), but they are pure noise once wrapped in a call's own delimiters — a call's
/// parens already draw that boundary, so `len((EXPR))` and `len(EXPR)` parse identically and only
/// the first trips ruff `UP034`. Call-argument position is exactly the one context where
/// stripping is always safe regardless of what `expr` itself contains, so this is applied there
/// and nowhere else — the receiver of a method call (`{expr}.startswith(...)`) is NOT call-
/// argument position (it's the thing before the dot) and must keep its parens. ~keep
///
/// Only strips a pair that both wraps the ENTIRE string and matches (the leading `(` closes
/// exactly at the trailing `)`, not before) — `(a) + (b)` starts with `(` and ends with `)` but
/// is two separate groups, not one redundant enclosing pair, so it is returned unchanged.
pub(super) fn strip_redundant_call_arg_parens(expr: &str) -> &str {
    if !expr.starts_with('(') || !expr.ends_with(')') {
        return expr;
    }
    let mut depth: i32 = 0;
    for (index, ch) in expr.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    // The opening paren closed before the string's last character — the outer
                    // '(' / ')' are not one matching pair enclosing the whole expression.
                    return if index == expr.len() - 1 {
                        &expr[1..expr.len() - 1]
                    } else {
                        expr
                    };
                }
            }
            _ => {}
        }
    }
    expr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_redundant_call_arg_parens_strips_a_narrowing_ternarys_enclosing_parens() {
        assert_eq!(
            strip_redundant_call_arg_parens("(result.markdown.content if result.markdown else None)"),
            "result.markdown.content if result.markdown else None"
        );
    }

    #[test]
    fn strip_redundant_call_arg_parens_leaves_an_unparenthesized_expression_untouched() {
        assert_eq!(
            strip_redundant_call_arg_parens("result.status_code"),
            "result.status_code"
        );
    }

    #[test]
    fn strip_redundant_call_arg_parens_leaves_two_separate_groups_untouched() {
        assert_eq!(strip_redundant_call_arg_parens("(a) + (b)"), "(a) + (b)");
    }

    #[test]
    fn strip_redundant_call_arg_parens_leaves_a_bare_parenthesized_tuple_element_untouched() {
        // A single parenthesized call, not a redundant wrap: `(x)(y)` — the outer '(' closes
        // before the string's end, so this must not be mistaken for one enclosing pair.
        assert_eq!(strip_redundant_call_arg_parens("(x)(y)"), "(x)(y)");
    }

    #[test]
    fn classify_bytes_value_html_is_inline() {
        matches!(classify_bytes_value("<!DOCTYPE html>"), BytesKind::InlineText);
    }

    #[test]
    fn classify_bytes_value_pdf_path_is_file_path() {
        matches!(classify_bytes_value("pdf/fake_memo.pdf"), BytesKind::FilePath);
    }

    #[test]
    fn classify_bytes_value_base64_is_base64() {
        matches!(classify_bytes_value("/9j/4AAQSkZJRgABAQEASABIAAD"), BytesKind::Base64);
    }

    // --- pyo3_would_inject_from_json: the shared pyo3_from_json_eligible gate. All three
    // conditions — per-type has_serde, crate-level has_serde, and convertibility — are
    // independently necessary. ---

    #[test]
    fn pyo3_would_inject_from_json_true_when_all_three_conditions_hold() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        assert!(
            convertible.contains("WidgetRequest"),
            "test setup: a plain fieldless type must be convertible"
        );

        assert!(pyo3_would_inject_from_json(
            "WidgetRequest",
            &type_defs,
            &convertible,
            true
        ));
    }

    #[test]
    fn pyo3_would_inject_from_json_false_when_type_lacks_serde() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: false,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);

        assert!(!pyo3_would_inject_from_json(
            "WidgetRequest",
            &type_defs,
            &convertible,
            true
        ));
    }

    /// Mirrors the measured liter-llm defect: `has_serde` alone is not the pyo3 gate. A
    /// serde-derived type whose field references a type that never resolves fails the second
    /// half of pyo3's conjunction (`core_to_binding_convertible_types`), so pyo3 never injects
    /// `from_json()` for it even though `has_serde` is true.
    #[test]
    fn pyo3_would_inject_from_json_false_when_type_has_serde_but_is_not_convertible() {
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};

        let type_defs = vec![TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "extra".to_string(),
                ty: TypeRef::Named("UnresolvedExternalType".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        assert!(
            !convertible.contains("WidgetRequest"),
            "test setup: an unresolved field type must fail convertibility"
        );

        assert!(!pyo3_would_inject_from_json(
            "WidgetRequest",
            &type_defs,
            &convertible,
            true
        ));
    }

    /// The unified predicate's third independent condition: even a per-type-serde,
    /// convertible type must not get `from_json` when the binding crate itself lacks
    /// `serde`/`serde_json` — `serde_json::from_str` wouldn't compile there.
    #[test]
    fn pyo3_would_inject_from_json_false_when_crate_lacks_serde() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        assert!(
            convertible.contains("WidgetRequest"),
            "test setup: a plain fieldless type must be convertible"
        );

        assert!(!pyo3_would_inject_from_json(
            "WidgetRequest",
            &type_defs,
            &convertible,
            false
        ));
    }

    // --- effective_options_via_for_type: what the emitter actually does today ---

    /// As of `093c42f31`, the pyo3 `.pyi` stub generator declares `from_json` under the exact
    /// same predicate as pyo3's Rust-codegen gate (`type_has_from_json` in
    /// `src/backends/pyo3/gen_bindings/types.rs`), so a type that passes the gate keeps
    /// `options_via = "from_json"` instead of downgrading. This is the exact liter-llm
    /// `CreateImageRequest` case: has_serde and convertible are both true, and
    /// `_internal_bindings.pyi` now declares `def from_json` for it.
    #[test]
    fn effective_options_via_for_type_keeps_from_json_when_pyo3_would_inject_it() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        assert!(pyo3_would_inject_from_json(
            "WidgetRequest",
            &type_defs,
            &convertible,
            true
        ));

        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("WidgetRequest"),
                &type_defs,
                &convertible,
                true,
                &HashSet::new()
            ),
            "from_json"
        );
    }

    #[test]
    fn effective_options_via_for_type_downgrades_to_kwargs_when_type_lacks_serde() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: false,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);

        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("WidgetRequest"),
                &type_defs,
                &convertible,
                true,
                &HashSet::new()
            ),
            "kwargs"
        );
    }

    #[test]
    fn effective_options_via_for_type_downgrades_to_kwargs_when_crate_lacks_serde() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "WidgetRequest".to_string(),
            has_serde: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);

        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("WidgetRequest"),
                &type_defs,
                &convertible,
                false,
                &HashSet::new()
            ),
            "kwargs"
        );
    }

    #[test]
    fn effective_options_via_for_type_downgrades_to_kwargs_when_type_is_unknown() {
        let convertible = core_to_binding_convertible_types(&[], &[]);
        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("WidgetRequest"),
                &[],
                &convertible,
                true,
                &HashSet::new()
            ),
            "kwargs"
        );
    }

    #[test]
    fn effective_options_via_for_type_leaves_non_from_json_values_untouched() {
        let convertible = core_to_binding_convertible_types(&[], &[]);
        assert_eq!(
            effective_options_via_for_type("dict", None, &[], &convertible, true, &HashSet::new()),
            "dict"
        );
        assert_eq!(
            effective_options_via_for_type("kwargs", None, &[], &convertible, true, &HashSet::new()),
            "kwargs"
        );
    }

    /// The exact xberg `ExtractInput` defect: a type is both native-`from_json`-eligible (serde
    /// derive, crate has serde, convertible) AND options-wrapped (`has_default`, not a return
    /// type, not reexported) -- e.g. any config/input DTO with a hand-written constructor like
    /// `from_bytes`/`from_uri` alongside pyo3's mechanically-injected `from_json`. `__init__.py`
    /// exports the wrapped name from `.options`, a plain dataclass with zero methods, so
    /// `from_json` must downgrade to `kwargs` even though the native gate alone says "keep it".
    #[test]
    fn effective_options_via_for_type_downgrades_to_kwargs_when_type_is_options_wrapped() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "ExtractInput".to_string(),
            has_serde: true,
            has_default: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        assert!(
            pyo3_would_inject_from_json("ExtractInput", &type_defs, &convertible, true),
            "test setup: the native class must still pass pyo3's own gate"
        );

        let dto = crate::core::config::DtoConfig::default();
        let options_wrapped = options_wrapped_type_names(&type_defs, &[], &dto, &[]);
        assert!(
            options_wrapped.contains("ExtractInput"),
            "test setup: a has_default, non-return, non-reexported type must be options-wrapped"
        );

        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("ExtractInput"),
                &type_defs,
                &convertible,
                true,
                &options_wrapped
            ),
            "kwargs",
            "the public name resolves to options.py's method-less dataclass, not the native class"
        );
    }

    /// Symmetric case: `reexported_types` is the documented per-type escape hatch
    /// (`xberg-io/alef#134`) that keeps a type native despite otherwise qualifying for the
    /// `options.py` wrapper. A reexported type's public name IS the native class, so `from_json`
    /// must be kept when the native gate says so.
    #[test]
    fn effective_options_via_for_type_keeps_from_json_when_type_is_reexported() {
        let type_defs = vec![crate::core::ir::TypeDef {
            name: "ExtractionResult".to_string(),
            has_serde: true,
            has_default: true,
            ..Default::default()
        }];
        let convertible = core_to_binding_convertible_types(&type_defs, &[]);
        let dto = crate::core::config::DtoConfig::default();
        let reexported = vec!["ExtractionResult".to_string()];
        let options_wrapped = options_wrapped_type_names(&type_defs, &[], &dto, &reexported);
        assert!(
            !options_wrapped.contains("ExtractionResult"),
            "test setup: reexported_types must exclude the type from the options.py wrapper set"
        );

        assert_eq!(
            effective_options_via_for_type(
                "from_json",
                Some("ExtractionResult"),
                &type_defs,
                &convertible,
                true,
                &options_wrapped
            ),
            "from_json"
        );
    }

    #[test]
    fn python_method_helper_import_recognizes_has_error_nodes() {
        assert_eq!(
            python_method_helper_import("has_error_nodes"),
            Some("tree_has_error_nodes".to_string())
        );
    }

    #[test]
    fn python_method_helper_import_returns_none_for_plain_method() {
        assert!(python_method_helper_import("root_child_count").is_none());
    }
}
