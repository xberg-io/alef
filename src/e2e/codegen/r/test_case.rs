#[path = "test_case/client_options.rs"]
mod client_options;

//! R e2e individual test case rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::sanitize_ident;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::{args, assertions, not_error_assertion, visitor};

/// Render the R `expect_error(...)`/`tryCatch` block for an `error`-asserting test.
///
/// ~keep With no declared value this returns output byte-identical to the bare
/// `expect_error(fn(args))` call. testthat's `expect_error(regexp=, class=)`
/// combine with AND semantics, so they can't express the message-or-class-name
/// disjunction other backends use (see `declared_error_value`'s doc comment);
/// a manual `tryCatch` is required instead. A value naming a real `ErrorVariant` this
/// backend cannot substantiate renders the registered skip instead: the R backend has no
/// error-conversion code building a per-variant condition class at all — every failure
/// surfaces as extendr's generic `simpleError`, so `class(e)` never carries a variant name for
/// `grepl` to find.
fn render_r_error_check(
    function_name: &str,
    final_args: &str,
    fixture: &crate::e2e::fixture::Fixture,
    errors: &[crate::core::ir::ErrorDef],
) -> String {
    match classify("r", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => format!("  expect_error({function_name}({final_args}))"),
        DeclaredErrorAssertion::Assert(value) => {
            let pattern = crate::e2e::escape::r_regex_literal(value);
            format!(
                "  tryCatch({{\n    {function_name}({final_args})\n    fail(\"expected an error to be thrown\")\n  }}, error = function(e) {{\n    expect_true(grepl({pattern}, conditionMessage(e)) || grepl({pattern}, paste(class(e), collapse = \" \")))\n  }})"
            )
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let skip = skip_line("  ", "#", variant, &fixture.id, "r");
            format!("  expect_error({function_name}({final_args}))\n{skip}")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    default_result_is_simple: bool,
    default_result_is_r_list: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    errors: &[crate::core::ir::ErrorDef],
) {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let (ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields) = FieldResolver::ir_field_sets(type_defs);
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_ir_fields(ir_reachable_fields, ir_known_excluded_fields, ir_optional_fields);
    let field_resolver = &call_field_resolver;
    // Resolve `function` via the R override when present. The default
    // `call_config.function` can be empty (e.g. trait-bridge calls like
    // `clear_document_extractors` set `function = ""` at the top level and
    // expose the real binding name only through per-language overrides);
    // emitting it verbatim produces invalid `result <- ()` calls.
    let function_name = call_config
        .overrides
        .get("r")
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = call_config.effective_result_var();
    // Per-fixture call configs (e.g. `list_document_extractors`) may set
    // `result_is_simple = true` even when the default `[e2e.call]` does not.
    // Without this lookup the registry/detection wrappers (which return scalar
    // strings or character vectors directly) get wrapped in
    // `jsonlite::fromJSON(...)` and the parser fails on non-JSON output.
    let r_override = call_config.overrides.get("r");
    let result_is_simple = if fixture.call.is_some() {
        call_config.result_is_simple || r_override.is_some_and(|o| o.result_is_simple)
    } else {
        default_result_is_simple
    };
    // Per-fixture override: when the R binding already returns a native R list
    // (not a JSON string), suppress `jsonlite::fromJSON` wrapping while still
    // using field-path (`result$field`) accessors in assertions.
    let result_is_r_list = if fixture.call.is_some() {
        r_override.is_some_and(|o| o.result_is_r_list)
    } else {
        default_result_is_r_list
    };
    let result_is_option = call_config.result_is_option || r_override.is_some_and(|o| o.result_is_option);

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('"', "\\\"");

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Allow per-call R overrides to remap fixture argument names. Many calls
    // (e.g. `extract_bytes`, `batch_extract_files`) use language-neutral
    // fixture field names (`data`, `paths`) that the R extendr binding
    // exposes under different identifiers (`content`, `items`).
    let arg_name_map = r_override.map(|o| &o.arg_name_map);
    let recipe = crate::e2e::codegen::recipe::ResolvedE2eCallRecipe::resolve("r", fixture, call_config, type_defs);
    let options_type = recipe.compatible_options_type(&["r", "csharp", "java", "go", "php", "python"]);
    // Build visitor setup and args if present
    let mut setup_lines = Vec::new();
    let mut teardown_block = String::new();
    let args_str = args::build_args_string(
        &fixture.input,
        fixture.resolved_args(call_config),
        args::RArgsContext {
            arg_name_map,
            options_type,
            fixture,
            config,
            type_defs,
            setup_lines: &mut setup_lines,
            teardown_block: &mut teardown_block,
        },
    );

    // Per-call R extra_args: positional trailing arguments appended verbatim.
    // Used when the extendr wrapper has more parameters than the fixture
    // declares (e.g. `render_pdf_page_to_png(pdf_bytes, page_index, dpi,
    // password)` where `dpi`/`password` are optional in Rust but extendr
    // surfaces them as required R parameters with no defaults).
    let r_extra_args: Vec<String> = r_override.map(|o| o.extra_args.clone()).unwrap_or_default();
    let args_with_extra = if r_extra_args.is_empty() {
        args_str
    } else {
        let extra = r_extra_args.join(", ");
        if args_str.is_empty() {
            extra
        } else {
            format!("{args_str}, {extra}")
        }
    };
    let final_args = if let Some(visitor_spec) = &fixture.visitor {
        visitor::build_r_visitor(&mut setup_lines, visitor_spec);
        // R rejects duplicated named arguments ("matched by multiple actual arguments"), so
        // strip any existing `options = ...` arg before appending the visitor-options list.
        // Handles `options = NULL` (when no default) and `options = <OptionsType>$default()`
        // (when build_args_string emits a default placeholder for an optional options arg).
        let base = args::strip_options_arg(&args_with_extra);
        let visitor_opts = "options = list(visitor = visitor)";
        let trimmed = base.trim_matches([' ', ',']);
        if trimmed.is_empty() {
            visitor_opts.to_string()
        } else {
            format!("{trimmed}, {visitor_opts}")
        }
    } else {
        args_with_extra
    };

    let final_args = if let Some(setup) = client_options::render(fixture, e2e_config, call_config) {
        setup_lines.push(setup);
        if final_args.is_empty() { "client".to_string() } else { format!("client, {final_args}") }
    } else { final_args };

    if expects_error {
        let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
        for line in &setup_lines {
            let _ = writeln!(out, "  {line}");
        }
        let error_check = render_r_error_check(&function_name, &final_args, fixture, errors);
        let _ = writeln!(out, "{error_check}");
        let _ = writeln!(out, "}})");
        return;
    }

    let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
    for line in &setup_lines {
        let _ = writeln!(out, "  {line}");
    }
    // The extendr extraction wrappers return JSON strings carrying the
    // serialized core result; parse into an R list so tests can use `$`
    // accessors. `result_is_simple` calls
    // already return scalar values and must be passed through verbatim.
    // `result_is_r_list` signals the binding returns a native R list (Robj),
    // not a JSON string — skip `jsonlite::fromJSON` but keep `$` accessors.
    // `returns_void` calls (trait-bridge `clear_*` wrappers that return `()`
    // in Rust → `NULL` in R) must not bind a `result` variable: the previous
    // emission of `result <- {function_name}(...)` was already correct when
    // `function_name` resolved, but parsers flag a stray `result` for void
    // calls. Use `invisible(...)` to make the void contract explicit.
    // A `returns_void` call binds no `result`, so a fixture whose only assertion is `not_error`
    // has nothing to assert a value against. testthat's `expect_no_error(...)` IS a real,
    // failable expectation for exactly this shape, so wrap the call in it rather than emitting
    // a bare `invisible(...)` beside an `expect_true(TRUE)` that can never fail. ~keep
    let has_not_error_assertion = fixture
        .assertions
        .iter()
        .any(|assertion| assertion.assertion_type == "not_error");
    let void_not_error = call_config.returns_void && has_not_error_assertion;
    // A `result_is_simple`/`result_is_option` result may legitimately be R `NULL`/`NA` on a
    // *successful* call (`Result<Option<T>, E>::Ok(None)` is not an error), so `not_error`
    // cannot assert non-null on it the way the ordinary-shape case does -- see
    // `not_error_assertion`'s module doc. The real, failable check moves here instead: wrap
    // the call itself in `expect_no_error(...)`, exactly like the void case above, rather than
    // asserting on a value whose emptiness is sometimes the correct outcome. Still binds
    // `result_var` (unlike the void case), since other assertions on the same fixture may
    // still need it. ~keep
    let option_not_error = !call_config.returns_void
        && not_error_assertion::unsafe_for_null_check(result_is_simple, result_is_option)
        && has_not_error_assertion;
    if void_not_error {
        let _ = writeln!(out, "  expect_no_error({function_name}({final_args}))");
    } else if call_config.returns_void {
        let _ = writeln!(out, "  invisible({function_name}({final_args}))");
    } else if result_is_simple || result_is_r_list {
        if option_not_error {
            let _ = writeln!(out, "  {result_var} <- expect_no_error({function_name}({final_args}))");
        } else {
            let _ = writeln!(out, "  {result_var} <- {function_name}({final_args})");
        }
    } else if option_not_error {
        let _ = writeln!(
            out,
            "  {result_var} <- jsonlite::fromJSON(expect_no_error({function_name}({final_args})), simplifyVector = FALSE)"
        );
    } else {
        let _ = writeln!(
            out,
            "  {result_var} <- jsonlite::fromJSON({function_name}({final_args}), simplifyVector = FALSE)"
        );
    }

    let result_is_bytes = call_config.result_is_bytes || r_override.is_some_and(|o| o.result_is_bytes);
    // Resolve assert_enum_fields from the R-language override so the assertion renderer
    // can identify fields that require the `.alef_format_value` wrapper rather than
    // matching against the literal field path "metadata.format".
    static EMPTY_ASSERT_ENUM_FIELDS: std::sync::LazyLock<std::collections::HashMap<String, String>> =
        std::sync::LazyLock::new(std::collections::HashMap::new);
    let assert_enum_fields = r_override
        .map(|o| &o.assert_enum_fields)
        .unwrap_or(&EMPTY_ASSERT_ENUM_FIELDS);
    // `out` accumulates every fixture's rendered test case in this file (see the
    // caller in `test_file.rs`), so the strict-availability scan below must only
    // look at the text this fixture's own assertion loop appends — scanning the
    // whole buffer would misattribute an earlier fixture's skip comment to this
    // fixture's id. ~keep
    let assertions_start = out.len();
    for assertion in &fixture.assertions {
        let context = assertions::RAssertionContext {
            field_resolver,
            result_is_simple,
            result_is_bytes,
            assert_enum_fields,
            returns_void: call_config.returns_void,
            result_is_option,
        };
        assertions::render_assertion(out, assertion, result_var, &context);
    }
    apply_vacuous_assertion_fallback(
        out,
        assertions_start,
        !fixture.assertions.is_empty(),
        call_config.returns_void,
        result_var,
        option_not_error,
    );
    crate::e2e::codegen::fail_on_unavailable_field_markers(
        &out[assertions_start..],
        "r",
        &fixture.id,
        &fixture.assertions,
    );
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&out[assertions_start..], "r", &fixture.id);
    out.push_str(&crate::e2e::codegen::inert_example::coverage_warning(
        &out[assertions_start..],
        "r",
        &fixture.id,
    ));

    // Emit teardown for trait-bridge tests to clean up registered test backends.
    for line in teardown_block.lines() {
        let _ = writeln!(out, "{line}");
    }

    let _ = writeln!(out, "}})");
}

/// When a fixture declares at least one assertion but the rendered body has no
/// executable statement — every field assertion resolved to a "skipped: ..."
/// comment because the field is unavailable on the result type — inject a
/// real assertion instead of leaving the test vacuous. `not_error` already
/// renders a real `expect_true(!is.null(...))` on its own for ordinary result
/// shapes (see `not_error_assertion::render`), so this only fires on the
/// remaining gap: declared field assertions that all turned out unavailable.
/// Fixtures that declare NO assertions at all are left untouched — a
/// deliberate "just call it" smoke test, matching every other backend in this
/// defect class (mirrors typescript's `apply_vacuous_assertion_fallback`).
/// `returns_void` calls never bind `result_var` (see the `invisible(...)`
/// branch above), so this must never fire for them — referencing an unbound
/// variable would not compile. ~keep
///
/// `option_not_error` must also suppress this fallback: when it is `true`, the call site
/// already wrapped the call in `expect_no_error(...)` as `not_error`'s real check for a
/// `result_is_simple`/`result_is_option` shape, and that assertion renders nothing in the
/// per-assertion loop by design (see `not_error_assertion::render`). Without this guard the
/// fallback would see an all-blank rendered body and re-inject the exact unsafe
/// `expect_true(!is.null(result))` check `not_error_assertion` exists to avoid for that
/// shape, undoing the fix one call frame up. ~keep
fn apply_vacuous_assertion_fallback(
    out: &mut String,
    assertions_start: usize,
    has_declared_assertions: bool,
    returns_void: bool,
    result_var: &str,
    option_not_error: bool,
) {
    if returns_void || !has_declared_assertions || option_not_error {
        return;
    }
    let has_real_assertion = out[assertions_start..]
        .lines()
        .any(|line| !line.trim().is_empty() && !line.trim().starts_with('#'));
    if has_real_assertion {
        return;
    }
    let _ = writeln!(out, "  expect_true(!is.null({result_var}))");
}

#[cfg(test)]
mod render_r_error_check_tests {
    use super::render_r_error_check;
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_with_declared_error(value: &str) -> Fixture {
        Fixture {
            id: "declares_error".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    fn coded_variant(name: &str) -> Vec<ErrorDef> {
        vec![ErrorDef {
            exception_base: None,
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: name.to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    #[test]
    fn no_declared_value_is_byte_identical_to_bare_expect_error() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        assert_eq!(
            render_r_error_check("extract_bytes", "data", &fixture, &[]),
            "  expect_error(extract_bytes(data))"
        );
    }

    #[test]
    fn declared_value_adds_message_or_class_name_check() {
        let fixture = fixture_with_declared_error("BadRequest");
        let check = render_r_error_check("extract_bytes", "data", &fixture, &[]);
        assert_eq!(
            check,
            "  tryCatch({\n    extract_bytes(data)\n    fail(\"expected an error to be thrown\")\n  }, error = function(e) {\n    expect_true(grepl(\"BadRequest\", conditionMessage(e)) || grepl(\"BadRequest\", paste(class(e), collapse = \" \")))\n  })"
        );
    }

    #[test]
    fn declared_value_with_regex_metacharacters_is_escaped() {
        let fixture = fixture_with_declared_error("field.name[0]");
        let check = render_r_error_check("extract_bytes", "data", &fixture, &[]);
        assert!(
            check.contains("\"field\\\\.name\\\\[0\\\\]\""),
            "expected escaped regex literal, got: {check}"
        );
    }

    /// The defect this fix closes: a declared value naming a real `ErrorVariant` — the R
    /// backend has no error-conversion code at all, so `class(e)` never carries a variant's
    /// identity — must render the registered skip, not a `grepl` check that can never pass.
    #[test]
    fn known_variant_r_cannot_substantiate_is_skipped() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = coded_variant("Authentication");
        let check = render_r_error_check("extract_bytes", "data", &fixture, &errors);
        assert_eq!(
            check,
            "  expect_error(extract_bytes(data))\n  # skipped: declared error variant 'Authentication' not yet \
             preserved as a distinct identity by this backend's generator"
        );
    }
}

#[cfg(test)]
mod vacuous_assertion_fallback_tests {
    use super::render_test_case;
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    /// A fixture whose sole assertion targets a field absent from
    /// `result_fields` renders only a "skipped" comment for that assertion.
    /// `apply_vacuous_assertion_fallback` must inject a real
    /// `expect_true(!is.null(result))` so the generated `testthat` block is
    /// never vacuously passing.
    #[test]
    fn dropped_field_assertion_still_gets_a_real_fallback_assertion() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "process".to_string();
        e2e_config.call.result_var = "result".to_string();
        e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);

        let fixture = Fixture {
            id: "process_smoke".to_string(),
            description: "test".to_string(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                field: Some("nonexistent_field".to_string()),
                value: Some(serde_json::json!("x")),
                ..Assertion::default()
            }],
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[], &[]);

        assert!(
            out.contains("# skipped: field 'nonexistent_field' not available on result type"),
            "expected the dropped assertion's skip comment, got:\n{out}"
        );
        assert!(
            out.contains("expect_true(!is.null(result))"),
            "expected a real fallback assertion on the discarded result, got:\n{out}"
        );
    }

    /// Positive control for the same fix: a fixture with genuinely zero
    /// declared assertions is left untouched (deliberate "just call it"
    /// smoke test), matching every other backend in this defect class.
    #[test]
    fn zero_declared_assertions_are_left_untouched() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "process".to_string();
        e2e_config.call.result_var = "result".to_string();

        let fixture = Fixture {
            id: "process_smoke".to_string(),
            description: "test".to_string(),
            assertions: Vec::new(),
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[], &[]);

        assert!(
            !out.contains("expect_true(!is.null(result))"),
            "a fixture with zero declared assertions must stay vacuous, got:\n{out}"
        );
    }

    /// Regression test for the void `not_error` defect: before this fix, a `returns_void`
    /// fixture whose only assertion was `not_error` rendered `invisible(process())` followed by
    /// `expect_true(TRUE)` — an "assertion" that can never fail, which is the exact vacuous shape
    /// `inert_example` exists to catch. testthat's `expect_no_error(...)` is a real, failable
    /// expectation for this shape, so the call is wrapped in it instead.
    #[test]
    fn void_not_error_wraps_the_call_in_expect_no_error() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "prefetch_languages".to_string();
        e2e_config.call.result_var = "result".to_string();
        e2e_config.call.returns_void = true;

        let fixture = Fixture {
            id: "prefetch_languages".to_string(),
            description: "test".to_string(),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Assertion::default()
            }],
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[], &[]);

        assert!(
            out.contains("expect_no_error(prefetch_languages())"),
            "expected the void call wrapped in expect_no_error, got:\n{out}"
        );
        assert!(
            !out.contains("expect_true(TRUE)"),
            "must not emit an assertion that can never fail beside the real one, got:\n{out}"
        );
        assert!(
            !out.contains("invisible("),
            "the call must not also be emitted unasserted, got:\n{out}"
        );
    }

    /// Regression test for issue #122: before this fix, a non-void fixture whose only
    /// assertion was `not_error` rendered a bare `expect_true(TRUE)` -- a testthat
    /// expectation that can never fail, the same vacuous shape the void case above was
    /// already fixed for. The obvious replacement (`expect_true(!is.null(result))`) is
    /// itself unsafe for a `result_is_option` call: `Result<Option<T>, E>::Ok(None)` is a
    /// successful call whose result is legitimately R `NULL`, so a non-null check would
    /// reject correct behaviour. The real, failable check must move to the call site
    /// instead (`expect_no_error(...)` around the fallible call), the same pattern already
    /// used for `returns_void`.
    #[test]
    fn option_not_error_wraps_the_call_in_expect_no_error_instead_of_a_false_null_check() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "get_embedding_preset".to_string();
        e2e_config.call.result_var = "result".to_string();
        e2e_config.call.result_is_option = true;

        let fixture = Fixture {
            id: "get_embedding_preset_missing".to_string(),
            description: "test".to_string(),
            assertions: vec![Assertion {
                assertion_type: "not_error".to_string(),
                ..Assertion::default()
            }],
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[], &[]);

        assert!(
            out.contains(
                "result <- jsonlite::fromJSON(expect_no_error(get_embedding_preset()), simplifyVector = FALSE)"
            ),
            "expected the fallible call wrapped in expect_no_error while still binding result, got:\n{out}"
        );
        assert!(
            !out.contains("expect_true(TRUE)"),
            "must not emit an assertion that can never fail, got:\n{out}"
        );
        assert!(
            !out.contains("!is.null(result)"),
            "must not assert non-null on a result that may legitimately be NULL on success, got:\n{out}"
        );
    }

    /// A void fixture with no `not_error` assertion keeps its prior `invisible(...)` emission —
    /// wrapping every void call regardless of what it asserts would be a different, unrequested
    /// behavior change.
    #[test]
    fn void_call_without_not_error_keeps_invisible() {
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "prefetch_languages".to_string();
        e2e_config.call.result_var = "result".to_string();
        e2e_config.call.returns_void = true;

        let fixture = Fixture {
            id: "prefetch_languages".to_string(),
            description: "test".to_string(),
            assertions: Vec::new(),
            ..Fixture::default()
        };

        let config = ResolvedCrateConfig {
            name: "sample".into(),
            ..ResolvedCrateConfig::default()
        };

        let mut out = String::new();
        render_test_case(&mut out, &fixture, &e2e_config, false, false, &config, &[], &[]);

        assert!(
            out.contains("invisible(prefetch_languages())"),
            "expected the prior invisible(...) emission, got:\n{out}"
        );
        assert!(!out.contains("expect_no_error"), "got:\n{out}");
    }
}
