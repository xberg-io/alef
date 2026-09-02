//! Error assertion rendering for generated Python tests.

use std::fmt::Write as FmtWrite;

use crate::core::ir::ErrorDef;
use crate::e2e::codegen::error_field_reachability::resolvable_equals_error_field;
use crate::e2e::escape::escape_python;
use crate::e2e::fixture::{Assertion, Fixture};

use super::super::json::json_to_python_literal;

/// A fixture's `equals` assertion against `error.<field>` that resolves to one of the crate's
/// whitelisted error introspection methods (`ErrorDef.methods`), paired with the declared error
/// type that answers it.
struct ResolvedErrorField<'a> {
    sub_field: &'a str,
    expected: &'a serde_json::Value,
    error_def: &'a ErrorDef,
}

fn resolved_error_fields<'a>(fixture: &'a Fixture, errors: &'a [ErrorDef]) -> Vec<ResolvedErrorField<'a>> {
    fixture
        .assertions
        .iter()
        .filter_map(|assertion: &'a Assertion| {
            let (error_def, _method) = resolvable_equals_error_field(assertion, errors)?;
            let sub_field = assertion.field.as_deref()?.strip_prefix("error.")?;
            let expected = assertion.value.as_ref()?;
            Some(ResolvedErrorField {
                sub_field,
                expected,
                error_def,
            })
        })
        .collect()
}

pub(super) fn emit_error_assertion(
    out: &mut String,
    fixture: &Fixture,
    arg_bindings_str: &str,
    call_expr: &str,
    is_streaming_error_call: bool,
    errors: &[ErrorDef],
    native_module: &str,
) {
    // ~keep Routed through the shared `declared_error_value` (see its own doc comment) rather
    // than a local `.find(|a| a.assertion_type == "error")`: a fixture commonly declares two
    // `"error"` assertions — a bare one, then one carrying the message/type-name value — and
    // only the shared helper looks past the first to find the one that actually has a value.
    let declared_value = crate::e2e::codegen::declared_error_value(fixture);
    let has_message = declared_value.is_some();
    // ~keep Reuses the same seam the docs-snippet renderer already consults
    // (`python/snippet.rs`) instead of re-deriving "does this fixture name a real variant" —
    // see the `two-generators-disagree` skill. `pyo3::create_exception!` gives every
    // `ErrorVariant` its own exception class unconditionally
    // (`declared_error_variant::substantiates_variant_identity`'s `"python" => true` arm), so
    // when the declared value names a real variant, `pytest.raises(<TheVariantError>)` is a
    // strictly stronger, type-discriminating check than the message-or-class-name substring
    // match below — it fails if the wrong error type is raised for any reason. The substring
    // fallback still renders for message-style values (config-validation fixtures whose
    // declared value is a message substring, not a variant name), which no per-variant class
    // exists for.
    let typed_branch = crate::e2e::codegen::snippet_error_branch::for_fixture("python", fixture, errors);

    // Real, per-field assertions this backend CAN render: an `equals` against `error.<field>`
    // where `<field>` is one of the crate's whitelisted error introspection methods. See
    // `error_field_reachability` for why this is reachable here (pyo3's own error converter
    // calls the live Rust error value's method and threads the result through the raised
    // exception's `args`) when it is NOT reachable for most other backends. Collected before
    // deciding whether to bind `exc_info` — a fixture may declare no message value at all and
    // still need it for one of these, and a fixture that ALSO has a typed variant branch still
    // needs `exc_info` bound alongside the narrower `pytest.raises({Variant}Error)`.
    let resolved_fields = resolved_error_fields(fixture, errors);
    let needs_exc_info = has_message || !resolved_fields.is_empty();

    render_unrenderable_error_path_assertions(out, fixture, errors);

    // Re-indent arg_bindings by an extra 4 spaces so they land inside the `with`
    // block. arg_bindings already begin with 4 spaces (function-body level);
    // prepending 4 more puts them at the with-body level (8 spaces).
    let indented_bindings: String = arg_bindings_str
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("    {l}\n"))
        .collect();

    if needs_exc_info {
        if let Some(branch) = &typed_branch {
            // The fixture names a real `ErrorVariant` and pyo3 generates a dedicated exception
            // class for it — catch that class directly. No `# noqa: B017` needed: B017 warns
            // specifically about the broad `pytest.raises(Exception)`, and a named class is
            // exactly the narrower catch the lint wants. Unlike the substring fallback below,
            // this fails the test when the wrong error type is raised, even if its message or
            // class name happens to contain the same substring. `exc_info` is bound alongside
            // the narrowed class exactly when a resolved field assertion below needs it — the
            // typed catch and the field assertion are independent claims about the same raised
            // exception and must compose, not compete.
            let exc_info_suffix = if resolved_fields.is_empty() { "" } else { " as exc_info" };
            let _ = writeln!(out, "    with pytest.raises({}){exc_info_suffix}:", branch.host_type);
        } else {
            // No `# noqa: B017` here, unlike the bare `pytest.raises(Exception):` below: B017
            // does not fire once the exception is BOUND, because binding it is the evidence that
            // the test goes on to inspect what was raised. Emitting the directive anyway made
            // ruff report RUF100 (unused `noqa`) against alef's own generated output -- caught by
            // `lint_clean_python_tests`, and confirmed by deleting the directive and watching
            // B017 stay silent, not by reading the rule's documentation. ~keep
            let _ = writeln!(out, "    with pytest.raises(Exception) as exc_info:");
        }
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            // The streaming iterator returns synchronously (chat_stream returns the
            // iterator without await); errors only appear when iterating via
            // __anext__. Strip the `await ` prefix the async-call codegen would
            // attach, then drain the iterator inside the raises block so the
            // exception propagates before the with-block exits.
            let sync_call_expr = call_expr.strip_prefix("await ").unwrap_or(call_expr);
            let _ = writeln!(out, "        _iterator = {sync_call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
        if typed_branch.is_none()
            && let Some(msg) = declared_value
        {
            let escaped = escape_python(msg);
            // Match against EITHER the rendered exception message OR the
            // exception class name. Different crates use different
            // fixture-shape conventions:
            //   * config-validation fixtures may use field names that are substrings
            //     of the user-facing error message, never of a class name.
            //   * API-error fixtures may use class-name prefixes such as
            //     `Authentication`, `BadRequest`, or `ContentPolicy`.
            //     `BadRequestError`, `ContentPolicyError`), not message text.
            // The disjunction lets a single codegen path satisfy both. Only reached when no
            // typed class exists for the declared value (see `typed_branch` above).
            let _ = writeln!(
                out,
                "    assert \"{escaped}\" in str(exc_info.value) or \"{escaped}\" in type(exc_info.value).__name__"
            );
        }
        emit_resolved_error_field_assertions(out, &resolved_fields, native_module);
    } else {
        let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
        out.push_str(&indented_bindings);
        if is_streaming_error_call {
            let _ = writeln!(out, "        _iterator = {call_expr}");
            let _ = writeln!(out, "        async for _ in _iterator:");
            let _ = writeln!(out, "            pass");
        } else {
            let _ = writeln!(out, "        {call_expr}");
        }
    }
}

/// Renders one `assert {module}.{info_fn}(exc_info.value).{field} == {expected}` line per
/// resolved field, importing the info-function's module once up front. `{info_fn}` is the exact
/// free function `src/codegen/error_gen/pyo3.rs`'s `gen_pyo3_error_methods_impl` registers on the
/// native module (`m.add_function(wrap_pyfunction!({info_fn}, m)?)?;` in
/// `pyo3::gen_bindings::methods::gen_module_init`) — asked for by name via
/// `pyo3_error_info_fn_name` rather than re-derived, so this can never spell a function the
/// native module does not actually export. That guarantee covers the NAME only, not the
/// MODULE: `native_module` here is whatever the caller passed (see `test_function.rs`'s
/// `from_json_module` resolution), which for a body-less call falls back to the public facade
/// package, not the native extension module. `gen_init_py` (`gen_bindings/errors.rs`) now
/// re-exports every `pyo3_error_info_fn_name`/`pyo3_error_info_struct_name` pair from the
/// facade too, so both resolutions work — but if that re-export ever regresses, this function
/// would silently render an assertion against a name the target module does not carry.
fn emit_resolved_error_field_assertions(
    out: &mut String,
    resolved_fields: &[ResolvedErrorField<'_>],
    native_module: &str,
) {
    if resolved_fields.is_empty() {
        return;
    }
    let _ = writeln!(out, "    import {native_module}  # noqa: PLC0415");
    for field in resolved_fields {
        let info_fn = crate::codegen::error_gen::pyo3_error_info_fn_name(field.error_def);
        let expected = json_to_python_literal(field.expected);
        let _ = writeln!(
            out,
            "    assert {native_module}.{info_fn}(exc_info.value).{} == {expected}",
            field.sub_field
        );
    }
}

/// Every fixture assertion beyond the one `"error"`-type check [`emit_error_assertion`] renders
/// (a message-or-class-name match inside the `pytest.raises` block) used to be silently dropped: a
/// second `"error"` assertion, an `equals` against an `error.<field>` path, or any other assertion
/// type on an error-path fixture rendered nothing at all — not even a skip comment — because this
/// function returns before the fixture's other assertions are ever visited. The wording, the
/// ledger recording and the reason no non-`rust` backend can resolve `error.<field>` now all live
/// in [`crate::e2e::codegen::error_path_assertions`], shared with every other backend's error
/// block; this stays as the python-shaped entry point (comment token `#`, four-space indent). ~keep
fn render_unrenderable_error_path_assertions(out: &mut String, fixture: &Fixture, errors: &[ErrorDef]) {
    crate::e2e::codegen::error_path_assertions::emit_with_errors(out, fixture, "    # ", "python", errors);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_with_error(value: Option<serde_json::Value>) -> Fixture {
        Fixture {
            docs: None,
            requirements: Vec::new(),
            id: "streaming_error".to_string(),
            description: "streaming error".to_string(),
            input: serde_json::Value::Null,
            http: None,
            asyncapi: None,
            websocket: None,
            preserve_input_urls: false,
            assertions: vec![crate::e2e::fixture::Assertion {
                skip: None,
                assertion_type: "error".to_string(),
                field: None,
                value,
                values: None,
                method: None,
                check: None,
                args: None,
                return_type: None,
            }],
            call: None,
            skip: None,
            env: None,
            setup: Vec::new(),
            visitor: None,
            args: Vec::new(),
            assertion_recipes: Vec::new(),
            mock_response: None,
            source: String::new(),
            category: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn streaming_error_assertion_drains_iterator_inside_raises() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let mut out = String::new();

        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "await client.chat_stream(payload)",
            true,
            &[],
            "native",
        );

        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(out.contains("        payload = {}"), "got: {out}");
        assert!(
            out.contains("        _iterator = client.chat_stream(payload)"),
            "got: {out}"
        );
        assert!(out.contains("        async for _ in _iterator:"), "got: {out}");
        assert!(out.contains("BadRequest"), "got: {out}");
    }

    #[test]
    fn plain_error_assertion_emits_call_inside_raises() {
        let fixture = fixture_with_error(None);
        let mut out = String::new();

        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &[],
            "native",
        );

        assert!(out.contains("with pytest.raises(Exception):"), "got: {out}");
        assert!(out.contains("        payload = {}"), "got: {out}");
        assert!(out.contains("        client.create(payload)"), "got: {out}");
        assert!(!out.contains("async for _ in _iterator"), "got: {out}");
    }

    fn assertion(
        assertion_type: &str,
        field: Option<&str>,
        value: Option<serde_json::Value>,
    ) -> crate::e2e::fixture::Assertion {
        crate::e2e::fixture::Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|f| f.to_string()),
            value,
            ..crate::e2e::fixture::Assertion::default()
        }
    }

    fn fixture_with_assertions(assertions: Vec<crate::e2e::fixture::Assertion>) -> Fixture {
        Fixture {
            assertions,
            ..fixture_with_error(None)
        }
    }

    /// Drives the real emission path (not a hand-built `SkipRecord`): a fixture with an `equals`
    /// assertion against `error.status_code` alongside the primary `error` check. Before this
    /// change, `emit_error_assertion` rendered only the primary check and the second assertion
    /// left no trace in the output at all — the gate had nothing to scan. This proves three
    /// things in sequence: the primary check still actually runs, the second assertion is now
    /// named in a skip comment instead of vanishing, and the widened gate recognises exactly that
    /// comment as an assertion-type skip.
    #[test]
    fn equals_on_error_field_is_now_visible_and_counted_by_the_gate() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, Some(serde_json::Value::String("BadRequest".to_string()))),
            assertion("equals", Some("error.status_code"), Some(serde_json::Value::from(429))),
        ]);
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &[],
            "native",
        );

        // The fixture's only assertion this backend can actually run must still run.
        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(
            out.contains(
                "assert \"BadRequest\" in str(exc_info.value) or \"BadRequest\" in type(exc_info.value).__name__"
            ),
            "the primary error assertion must still render: got: {out}"
        );

        // The second assertion must now be named, not silently dropped.
        assert!(
            out.contains(
                "# skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "got: {out}"
        );

        // And the widened gate must recognise exactly that line.
        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].field, "equals");
        assert_eq!(
            records[0].verdict,
            crate::e2e::codegen::SkipVerdict::AwaitingGeneratorSupport
        );
        assert_eq!(records[0].origin, crate::e2e::codegen::SkipOrigin::AssertionType);
    }

    /// Negative control: the fixture's one assertion IS rendered (the primary error check), so the
    /// assertion-type gate must find nothing to count. Without this, a gate that fires on every
    /// line would be exactly as uninformative as the gate that fired on none before this change.
    #[test]
    fn a_rendered_error_assertion_does_not_trip_the_assertion_type_gate() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &[],
            "native",
        );
        assert!(
            out.contains("assert \"BadRequest\" in str(exc_info.value)"),
            "the fixture's only assertion must actually render before we assert nothing was \
             flagged: got: {out}"
        );

        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        assert!(
            crate::e2e::codegen::take_skip_records().is_empty(),
            "a rendered assertion must not be recognised as an assertion-type skip"
        );
    }

    /// The exact shape observed live in `crawlberg`'s `validation_ssrf_*` fixtures: a bare
    /// `{"type": "error"}` assertion FOLLOWED BY `{"type": "error", "value": "..."}`. Before the
    /// fix, `emit_error_assertion` found only the first (bare) `"error"` assertion, so
    /// `has_message` was always false for this shape and the generated test dropped the message
    /// check entirely — `with pytest.raises(Exception):` with no `assert "..." in ...` line, so
    /// `assert result.is_err()` could not tell an SSRF refusal from an unrelated failure. This
    /// must render the message check exactly as it would if the fixture had declared the value on
    /// its only `"error"` assertion.
    #[test]
    fn a_bare_check_followed_by_a_valued_one_still_renders_the_message_check() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, None),
            assertion(
                "error",
                None,
                Some(serde_json::Value::String("ssrf_policy_violation".to_string())),
            ),
        ]);
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    url = \"http://127.0.0.1:9/\"\n",
            "scrape(engine, url)",
            false,
            &[],
            "native",
        );

        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(
            out.contains(
                "assert \"ssrf_policy_violation\" in str(exc_info.value) or \"ssrf_policy_violation\" in \
                 type(exc_info.value).__name__"
            ),
            "the declared value on the second `error` assertion must still render a message \
             check: got: {out}"
        );
    }

    fn error_def_with_variant(error_name: &str, variant_name: &str) -> crate::core::ir::ErrorDef {
        crate::core::ir::ErrorDef {
            name: error_name.to_string(),
            rust_path: format!("lib::{error_name}"),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant {
                name: variant_name.to_string(),
                is_unit: true,
                ..crate::core::ir::ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// The structural half of the xberg #1525 fix: when a fixture's declared `error` value
    /// names a real `ErrorVariant`, `pyo3::create_exception!` gives it a dedicated exception
    /// class unconditionally (`declared_error_variant::substantiates_variant_identity`'s
    /// `"python" => true` arm), so the generated assertion must catch THAT class rather than
    /// the broad `Exception`, and the substring proxy the class-scoped catch supersedes must
    /// not also render.
    #[test]
    fn a_declared_variant_renders_a_class_scoped_raises_instead_of_the_substring_proxy() {
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let errors = vec![error_def_with_variant("ApiError", "BadRequest")];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
            "native",
        );

        assert!(out.contains("with pytest.raises(BadRequestError):"), "got: {out}");
        assert!(!out.contains("pytest.raises(Exception)"), "got: {out}");
        assert!(
            !out.contains("in str(exc_info.value) or"),
            "the class-scoped catch makes the substring proxy redundant: got: {out}"
        );
    }

    fn error_def_with_status_code() -> crate::core::ir::ErrorDef {
        crate::core::ir::ErrorDef {
            name: "SampleError".to_string(),
            rust_path: "sample_llm::SampleError".to_string(),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant::default()],
            doc: String::new(),
            methods: vec![status_code_method()],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn status_code_method() -> crate::core::ir::MethodDef {
        crate::core::ir::MethodDef {
            name: "status_code".to_string(),
            params: Vec::new(),
            return_type: crate::core::ir::TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// Same shape as `error_def_with_variant`, but ALSO whitelists `status_code` — the fixture
    /// this backs declares both a real variant (drives the typed `pytest.raises` branch) and a
    /// resolvable `error.status_code` field (drives the field assertion), so the two independently
    /// -correct changes have to compose in one `ErrorDef`.
    fn error_def_with_variant_and_status_code(error_name: &str, variant_name: &str) -> crate::core::ir::ErrorDef {
        crate::core::ir::ErrorDef {
            name: error_name.to_string(),
            rust_path: format!("lib::{error_name}"),
            original_rust_path: String::new(),
            variants: vec![crate::core::ir::ErrorVariant {
                name: variant_name.to_string(),
                is_unit: true,
                ..crate::core::ir::ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![status_code_method()],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// The positive case task #1524 exists to prove: a fixture whose declared `error` value is
    /// message-style (so no typed `pytest.raises` branch applies) alongside an `equals` against a
    /// whitelisted `error.<field>`. The generator must now emit a REAL assertion against the
    /// crate's own `{Error}Info` companion pyclass — the one `src/codegen/error_gen/pyo3.rs`
    /// already registers on the native module and populates from the live Rust error value at
    /// conversion time — instead of a skip comment. Asserts exact rendered bytes, not `contains`,
    /// so a change to either the accessor shape or the literal rendering is caught here. ~keep
    #[test]
    fn equals_on_a_whitelisted_error_field_renders_a_real_assertion_instead_of_a_skip() {
        let fixture = fixture_with_assertions(vec![
            assertion(
                "error",
                None,
                Some(serde_json::Value::String("rate limit exceeded".to_string())),
            ),
            assertion("equals", Some("error.status_code"), Some(serde_json::Value::from(429))),
        ]);
        let errors = vec![error_def_with_status_code()];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
            "sample_llm._native",
        );

        assert_eq!(
            out,
            "    with pytest.raises(Exception) as exc_info:\n        payload = {}\n        \
             client.create(payload)\n    assert \"rate limit exceeded\" in str(exc_info.value) or \"rate limit \
             exceeded\" in type(exc_info.value).__name__\n    import sample_llm._native  # noqa: PLC0415\n    \
             assert sample_llm._native.sample_error_info(exc_info.value).status_code == 429\n"
        );

        // No skip marker for this assertion, and the strict gate agrees there is nothing left
        // to count for this fixture.
        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    /// A resolvable field with NO declared message value still needs `exc_info` bound — the
    /// `has_message` flag alone used to decide that, so a bare `{"type": "error"}` (no value)
    /// paired with a resolvable `error.status_code` would have rendered `exc_info` unbound and
    /// crashed the emitted assertion at generation time with a Python `NameError` at runtime, not
    /// a compile-time signal. Pins the `with ... as exc_info:` form even though no message
    /// assertion is present, and confirms no typed branch applies (there is no declared value to
    /// name a variant from).
    #[test]
    fn a_resolvable_field_forces_exc_info_binding_even_without_a_message() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, None),
            assertion("equals", Some("error.status_code"), Some(serde_json::Value::from(429))),
        ]);
        let errors = vec![error_def_with_status_code()];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
            "sample_llm._native",
        );

        assert!(out.contains("with pytest.raises(Exception) as exc_info"), "got: {out}");
        assert!(
            out.contains("assert sample_llm._native.sample_error_info(exc_info.value).status_code == 429"),
            "got: {out}"
        );
    }

    /// THE case that matters most in this change: the typed-variant branch (merged separately,
    /// xberg #1525) and the resolved-error-field branch (this change, xberg #1524) both apply to
    /// the same fixture and must compose in one `pytest.raises` block, not compete — the fixture
    /// declares a real `ErrorVariant` (narrows the catch to `BadRequestError`) AND a resolvable
    /// `error.status_code` (adds a field assertion), and both claims about the same raised
    /// exception must render together. Before this composition was wired, the typed branch bound
    /// no `exc_info` at all, so the field assertion below would have referenced an undefined name.
    /// Exact-bytes assertion so a regression in either half, or in how they combine, is caught
    /// here rather than by two tests that each pass in isolation. ~keep
    #[test]
    fn a_declared_variant_and_a_resolvable_error_field_compose() {
        let fixture = fixture_with_assertions(vec![
            assertion("error", None, Some(serde_json::Value::String("BadRequest".to_string()))),
            assertion("equals", Some("error.status_code"), Some(serde_json::Value::from(429))),
        ]);
        let errors = vec![error_def_with_variant_and_status_code("ApiError", "BadRequest")];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
            "sample_llm._native",
        );

        assert_eq!(
            out,
            "    with pytest.raises(BadRequestError) as exc_info:\n        payload = {}\n        \
             client.create(payload)\n    import sample_llm._native  # noqa: PLC0415\n    assert \
             sample_llm._native.api_error_info(exc_info.value).status_code == 429\n"
        );
        // The typed catch supersedes the substring proxy exactly as it does without a resolved
        // field present — composing with the field assertion must not resurrect it.
        assert!(!out.contains("in str(exc_info.value) or"), "got: {out}");
        assert!(!out.contains("pytest.raises(Exception)"), "got: {out}");

        let _ = crate::e2e::codegen::take_skip_records();
        crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out, "python", &fixture.id);
        assert!(crate::e2e::codegen::take_skip_records().is_empty());
    }

    fn python3_available() -> bool {
        which::which("python3").is_ok()
    }

    /// A minimal `pytest.raises` stand-in carrying the ONE behaviour this test cares about:
    /// like real `pytest.raises`, it does NOT suppress an exception whose type is not a
    /// subclass of the expected one — it propagates, failing the enclosing test. There is no
    /// `pytest` package dependency available to a Rust unit test, so this mirrors just that
    /// discriminating behaviour rather than pulling one in. ~keep
    const PYTEST_RAISES_STUB: &str = "\
class _Raises:
    def __init__(self, expected):
        self.expected = expected

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, tb):
        if exc_type is None:
            raise AssertionError(f\"DID NOT RAISE {self.expected}\")
        return issubclass(exc_type, self.expected)


def raises(expected, *args, **kwargs):
    return _Raises(expected)
";

    /// Runs `raises_block` (the exact text [`emit_error_assertion`] renders for the `with
    /// pytest.raises(...)` block) as a real Python 3 process under [`PYTEST_RAISES_STUB`],
    /// with `BadRequestError`/`UnrelatedError` classes defined and a `client.create(...)` that
    /// raises `raising_class`. Returns whether the script ran to completion with no uncaught
    /// exception — i.e. whether the generated assertion would have passed.
    fn generated_assertion_passes_when_call_raises(raises_block: &str, raising_class: &str) -> bool {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("pytest.py"), PYTEST_RAISES_STUB).expect("write pytest stub");
        let script = format!(
            "import pytest\n\n\
             class BadRequestError(Exception):\n    pass\n\n\
             class UnrelatedError(Exception):\n    pass\n\n\
             class _Client:\n    def create(self, payload):\n        raise {raising_class}(\"BadRequest-shaped input rejected\")\n\n\
             client = _Client()\n\n\
             def test_case():\n{raises_block}\n\
             test_case()\n"
        );
        std::fs::write(dir.path().join("script.py"), script).expect("write script");
        let status = std::process::Command::new("python3")
            .arg("script.py")
            .current_dir(dir.path())
            .status()
            .expect("run python3");
        status.success()
    }

    /// The runtime half of the xberg #1525 fix, and the one property the replaced substring
    /// proxy provably lacked: it passed for ANY exception whose message or class name merely
    /// *contained* the declared variant name — `"BadRequest" in str(exc_info.value) or ...` —
    /// including an unrelated error. `UnrelatedError("BadRequest-shaped input rejected")` is
    /// exactly that shape: its message contains the substring, its class does not carry the
    /// name. Under the class-scoped `pytest.raises(BadRequestError)` this now renders, that
    /// call must FAIL the generated assertion — proving the discrimination the substring check
    /// could never provide — while the real `BadRequestError` must still pass it.
    #[test]
    fn wrong_error_type_fails_the_generated_assertion_even_when_its_message_matches() {
        if !python3_available() {
            return;
        }
        let fixture = fixture_with_error(Some(serde_json::Value::String("BadRequest".to_string())));
        let errors = vec![error_def_with_variant("ApiError", "BadRequest")];
        let mut out = String::new();
        emit_error_assertion(
            &mut out,
            &fixture,
            "    payload = {}\n",
            "client.create(payload)",
            false,
            &errors,
            "native",
        );
        let raises_block = out.trim_start_matches('\n');

        assert!(
            generated_assertion_passes_when_call_raises(raises_block, "BadRequestError"),
            "the correct error type must satisfy the generated assertion"
        );
        assert!(
            !generated_assertion_passes_when_call_raises(raises_block, "UnrelatedError"),
            "an unrelated error type whose MESSAGE merely contains the variant name must fail \
             the generated assertion, not pass it — this is exactly what the substring proxy \
             could not do"
        );
    }
}
