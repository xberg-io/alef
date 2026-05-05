//! R e2e test generator using testthat.

use crate::config::E2eConfig;
use crate::escape::{escape_r, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use anyhow::Result;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// R e2e code generator.
pub struct RCodegen;

impl E2eCodegen for RCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        _config: &ResolvedCrateConfig,
    ) -> Result<Vec<GeneratedFile>> {
        let lang = self.language_name();
        let output_base = PathBuf::from(e2e_config.effective_output()).join(lang);

        let mut files = Vec::new();

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let _function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_is_simple = call.result_is_simple || overrides.is_some_and(|o| o.result_is_simple);
        let _result_var = &call.result_var;

        // Resolve package config.
        let r_pkg = e2e_config.resolve_package("r");
        let pkg_name = r_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| module_path.clone());
        let pkg_path = r_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/r".to_string());
        let pkg_version = r_pkg
            .as_ref()
            .and_then(|p| p.version.as_ref())
            .cloned()
            .unwrap_or_else(|| "0.1.0".to_string());

        // Generate DESCRIPTION file.
        files.push(GeneratedFile {
            path: output_base.join("DESCRIPTION"),
            content: render_description(&pkg_name, &pkg_version, e2e_config.dep_mode),
            generated_header: false,
        });

        // Generate test runner script.
        files.push(GeneratedFile {
            path: output_base.join("run_tests.R"),
            content: render_test_runner(&pkg_path, e2e_config.dep_mode),
            generated_header: true,
        });

        // setup-fixtures.R — testthat sources `setup-*.R` files in the tests
        // directory once before any tests run, with the working directory set
        // to the tests/ folder. We use this hook to chdir into the repo's
        // shared `test_documents/` directory so that fixture paths like
        // `pdf/fake_memo.pdf` resolve at extraction time.
        files.push(GeneratedFile {
            path: output_base.join("tests").join("setup-fixtures.R"),
            content: render_setup_fixtures(),
            generated_header: true,
        });

        // Generate test files per category.
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("test_{}.R", sanitize_filename(&group.category));
            let field_resolver = FieldResolver::new(
                &e2e_config.fields,
                &e2e_config.fields_optional,
                &e2e_config.result_fields,
                &e2e_config.fields_array,
            );
            let content = render_test_file(&group.category, &active, &field_resolver, result_is_simple, e2e_config);
            files.push(GeneratedFile {
                path: output_base.join("tests").join(filename),
                content,
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "r"
    }
}

fn render_description(pkg_name: &str, pkg_version: &str, dep_mode: crate::config::DependencyMode) -> String {
    let dep_line = match dep_mode {
        crate::config::DependencyMode::Registry => {
            format!("Imports: {pkg_name} ({pkg_version})\n")
        }
        crate::config::DependencyMode::Local => String::new(),
    };
    format!(
        r#"Package: e2e.r
Title: E2E Tests for {pkg_name}
Version: 0.1.0
Description: End-to-end test suite.
{dep_line}Suggests: testthat (>= 3.0.0)
Config/testthat/edition: 3
"#
    )
}

fn render_setup_fixtures() -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# Resolve fixture paths against the repo's `test_documents/` directory."
    );
    let _ = writeln!(
        out,
        "# testthat sources setup-*.R with the working directory at tests/,"
    );
    let _ = writeln!(
        out,
        "# so test_documents lives three directories up: tests/ -> e2e/r/ -> e2e/ -> repo root."
    );
    let _ = writeln!(
        out,
        "# Each `test_that()` block has its working directory reset back to tests/, so"
    );
    let _ = writeln!(
        out,
        "# fixture lookups must be performed via this helper rather than relying on `setwd`."
    );
    let _ = writeln!(
        out,
        ".kreuzberg_test_documents <- normalizePath(\"../../../test_documents\", mustWork = FALSE)"
    );
    let _ = writeln!(out, ".resolve_fixture <- function(path) {{");
    let _ = writeln!(out, "  if (dir.exists(.kreuzberg_test_documents)) {{");
    let _ = writeln!(out, "    file.path(.kreuzberg_test_documents, path)");
    let _ = writeln!(out, "  }} else {{");
    let _ = writeln!(out, "    path");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    out
}

fn render_test_runner(pkg_path: &str, dep_mode: crate::config::DependencyMode) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "library(testthat)");
    match dep_mode {
        crate::config::DependencyMode::Registry => {
            // In registry mode, require the installed CRAN package directly.
            let _ = writeln!(out, "# Package loaded via library() from CRAN install.");
        }
        crate::config::DependencyMode::Local => {
            // Use devtools::load_all() to load the local R package without requiring
            // a full install, matching the e2e test runner convention.
            let _ = writeln!(out, "devtools::load_all(\"{pkg_path}\")");
        }
    }
    let _ = writeln!(out);
    // Surface every failure rather than aborting at the default max_fails=10 —
    // partial pass counts are essential for triage during e2e bring-up.
    let _ = writeln!(out, "testthat::set_max_fails(Inf)");
    // Resolve the tests/ directory relative to this script. testthat reads
    // setup-*.R from there before each file runs, where path resolution
    // against test_documents/ is handled by the `.resolve_fixture` helper.
    let _ = writeln!(
        out,
        ".script_dir <- tryCatch(dirname(normalizePath(sys.frame(1)$ofile)), error = function(e) getwd())"
    );
    let _ = writeln!(out, "test_dir(file.path(.script_dir, \"tests\"))");
    out
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    e2e_config: &E2eConfig,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(&mut out, fixture, e2e_config, field_resolver, result_is_simple);
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Clean up trailing newlines.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn render_test_case(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
) {
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let function_name = &call_config.function;
    let result_var = &call_config.result_var;

    let test_name = sanitize_ident(&fixture.id);
    let description = fixture.description.replace('"', "\\\"");

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    // Allow per-call R overrides to remap fixture argument names. Many calls
    // (e.g. `extract_bytes`, `batch_extract_files`) use language-neutral
    // fixture field names (`data`, `paths`) that the R extendr binding
    // exposes under different identifiers (`content`, `items`).
    let r_override = call_config.overrides.get("r");
    let arg_name_map = r_override.map(|o| &o.arg_name_map);
    let args_str = build_args_string(&fixture.input, &call_config.args, arg_name_map);

    // Build visitor setup and args if present
    let mut setup_lines = Vec::new();
    let final_args = if let Some(visitor_spec) = &fixture.visitor {
        build_r_visitor(&mut setup_lines, visitor_spec);
        if args_str.is_empty() {
            "options = list(visitor = visitor)".to_string()
        } else {
            format!("{args_str}, options = list(visitor = visitor)")
        }
    } else {
        args_str
    };

    if expects_error {
        let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
        for line in &setup_lines {
            let _ = writeln!(out, "  {line}");
        }
        let _ = writeln!(out, "  expect_error({function_name}({final_args}))");
        let _ = writeln!(out, "}})");
        return;
    }

    let _ = writeln!(out, "test_that(\"{test_name}: {description}\", {{");
    for line in &setup_lines {
        let _ = writeln!(out, "  {line}");
    }
    // The extendr extraction wrappers return JSON strings carrying the
    // serialized core result; parse into an R list so tests can use `$`
    // accessors. `result_is_simple` calls (e.g. `convert_html_to_markdown`)
    // already return scalar values and must be passed through verbatim.
    if result_is_simple {
        let _ = writeln!(out, "  {result_var} <- {function_name}({final_args})");
    } else {
        let _ = writeln!(
            out,
            "  {result_var} <- jsonlite::fromJSON({function_name}({final_args}), simplifyVector = FALSE)"
        );
    }

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver, result_is_simple, e2e_config);
    }

    let _ = writeln!(out, "}})");
}

fn build_args_string(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    arg_name_map: Option<&std::collections::HashMap<String, String>>,
) -> String {
    if args.is_empty() {
        // No declared args means the wrapper takes zero parameters; emitting
        // `list()` here would trigger an `unused argument (list())` error in R.
        // Likewise, fall through to nothing if the fixture's input is empty.
        if matches!(input, serde_json::Value::Null) || input.as_object().is_some_and(|m| m.is_empty()) {
            return String::new();
        }
        return json_to_r(input, true);
    }

    let parts: Vec<String> = args
        .iter()
        .filter_map(|arg| {
            // Apply per-language argument renames before emitting the call.
            let arg_name: &str = arg_name_map
                .and_then(|m| m.get(&arg.name).map(String::as_str))
                .unwrap_or(&arg.name);

            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            // R extendr-generated wrappers do not preserve Option<T> defaults from
            // the Rust signature — every parameter is positional and required at
            // the R level. To keep generated calls valid we must pass a placeholder
            // (`NULL` for `Option<T>`, `ExtractionConfig$default()` for typed
            // configs) whenever the fixture omits an optional value.
            let val = match val {
                Some(v) if !(v.is_null() && arg.optional) => v,
                _ => {
                    if !arg.optional {
                        return None;
                    }
                    if arg.arg_type == "json_object" {
                        let r_value = r_default_for_config_arg(arg_name);
                        return Some(format!("{arg_name} = {r_value}"));
                    }
                    return Some(format!("{arg_name} = NULL"));
                }
            };
            // The extendr bindings expect owned PORs (ExternalPtr) for typed
            // config arguments — passing an R `list()` raises
            // `Expected ExternalPtr got List`. The fixtures don't carry the
            // option fields needed to round-trip through ExtractionConfig$new,
            // so emit `ExtractionConfig$default()` whenever a `json_object` arg
            // resolves to an empty / object-shaped JSON value. Non-empty objects
            // also fall back to `default()` because the R API surface only
            // exposes a `default()` constructor for ExtractionConfig at present.
            if arg.arg_type == "json_object" && (val.is_null() || val.is_object()) {
                let r_value = r_default_for_config_arg(arg_name);
                return Some(format!("{arg_name} = {r_value}"));
            }
            // `bytes` arg type: convert string fixture values into runtime
            // `readBin(...)` calls so the wrapper receives raw bytes instead
            // of an R character vector. This mirrors the Python emit_bytes_arg
            // helper and is what the extendr binding for Vec<u8> expects.
            if arg.arg_type == "bytes" {
                if let Some(raw) = val.as_str() {
                    let r_value = render_bytes_value(raw);
                    return Some(format!("{arg_name} = {r_value}"));
                }
            }
            // `file_path` arg type: fixtures encode relative paths that resolve
            // against the repo's `test_documents/` directory. Using a runtime
            // helper that anchors paths to that directory avoids fragility from
            // testthat resetting the working directory between files.
            if arg.arg_type == "file_path" {
                if let Some(raw) = val.as_str() {
                    if !raw.starts_with('/') && !raw.is_empty() {
                        let escaped = escape_r(raw);
                        return Some(format!("{arg_name} = .resolve_fixture(\"{escaped}\")"));
                    }
                }
            }
            Some(format!("{arg_name} = {}", json_to_r(val, true)))
        })
        .collect();

    parts.join(", ")
}

/// Render a `bytes` fixture value as the R expression that produces a raw
/// vector at test time. Mirrors python's `emit_bytes_arg` classifier so we can
/// support both file-path style fixtures (`"pdf/fake_memo.pdf"`) and inline
/// text payloads (`"<html>..."`). The resulting expression is dropped directly
/// into the call site, e.g. `content = readBin(.resolve_fixture("pdf/fake_memo.pdf"), ...)`.
fn render_bytes_value(raw: &str) -> String {
    if raw.starts_with('<') || raw.starts_with('{') || raw.starts_with('[') || raw.contains(' ') {
        // Inline text payload — encode to raw via charToRaw.
        let escaped = escape_r(raw);
        return format!("charToRaw(\"{escaped}\")");
    }
    let first = raw.chars().next().unwrap_or('\0');
    if first.is_ascii_alphanumeric() || first == '_' {
        if let Some(slash) = raw.find('/') {
            if slash > 0 {
                let after = &raw[slash + 1..];
                if after.contains('.') && !after.is_empty() {
                    let escaped = escape_r(raw);
                    return format!(
                        "readBin(.resolve_fixture(\"{escaped}\"), what = \"raw\", n = file.info(.resolve_fixture(\"{escaped}\"))$size)"
                    );
                }
            }
        }
    }
    // Default to inline text encoding — matches Python's InlineText branch.
    let escaped = escape_r(raw);
    format!("charToRaw(\"{escaped}\")")
}

/// Map the extractor argument name onto its R `*Config$default()` constructor.
/// Falls back to `list()` for unknown names — the extendr binding will error
/// with a clear message, which is preferable to silently passing a wrong type.
fn r_default_for_config_arg(arg_name: &str) -> String {
    match arg_name {
        "config" => "ExtractionConfig$default()".to_string(),
        "options" | "html_output" => "HtmlOutputConfig$default()".to_string(),
        "chunking" => "ChunkingConfig$default()".to_string(),
        "ocr" => "OcrConfig$default()".to_string(),
        "image" | "images" => "ImageExtractionConfig$default()".to_string(),
        "language_detection" => "LanguageDetectionConfig$default()".to_string(),
        _ => "list()".to_string(),
    }
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    _e2e_config: &E2eConfig,
) {
    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct attribute accesses on the result.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content" => {
                let pred = format!("all(sapply({result_var}$chunks %||% list(), function(c) nchar(c$content) > 0))");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({pred})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_false({pred})");
                    }
                    _ => {
                        let _ = writeln!(out, "  # skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let pred = format!(
                    "all(sapply({result_var}$chunks %||% list(), function(c) !is.null(c$embedding) && length(c$embedding) > 0))"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({pred})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_false({pred})");
                    }
                    _ => {
                        let _ = writeln!(out, "  # skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // embed_texts returns list of numeric vectors in R — no wrapper object.
            // result_var is the embedding matrix; use it directly.
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_equal(length({result_var}), {r_val})");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_gte(length({result_var}), {r_val})");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "  expect_gt(length({result_var}), 0)");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "  expect_equal(length({result_var}), 0)");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "  # skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!("(if (length({result_var}) == 0) 0L else length({result_var}[[1]]))");
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_equal({expr}, {r_val})");
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_gt({expr}, {r_val})");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "  # skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("all(sapply({result_var}, function(e) length(e) > 0))")
                    }
                    "embeddings_finite" => {
                        format!("all(sapply({result_var}, function(e) all(is.finite(e))))")
                    }
                    "embeddings_non_zero" => {
                        format!("all(sapply({result_var}, function(e) any(e != 0.0)))")
                    }
                    "embeddings_normalized" => {
                        format!("all(sapply({result_var}, function(e) abs(sum(e * e) - 1.0) < 1e-3))")
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({pred})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_false({pred})");
                    }
                    _ => {
                        let _ = writeln!(out, "  # skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            // ---- keywords / keywords_count ----
            // R ExtractionResult does not expose extracted_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(out, "  # skipped: field '{f}' not available on R ExtractionResult");
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "  # skipped: field '{f}' not available on result type");
            return;
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields
    // (e.g., metadata, document, structure) since the binding returns a plain value.
    if result_is_simple {
        if let Some(f) = &assertion.field {
            let f_lower = f.to_lowercase();
            if !f.is_empty()
                && f_lower != "content"
                && (f_lower.starts_with("metadata")
                    || f_lower.starts_with("document")
                    || f_lower.starts_with("structure"))
            {
                let _ = writeln!(
                    out,
                    "  # skipped: result_is_simple for field '{f}' not available on result type"
                );
                return;
            }
        }
    }

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "r", result_var),
            _ => result_var.to_string(),
        }
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_equal(trimws({field_expr}), {r_val})");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_true(grepl({r_val}, {field_expr}, fixed = TRUE))");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let r_val = json_to_r(val, false);
                    let _ = writeln!(out, "  expect_true(grepl({r_val}, {field_expr}, fixed = TRUE))");
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_false(grepl({r_val}, {field_expr}, fixed = TRUE))");
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "  expect_true(if (is.character({field_expr})) nchar({field_expr}) > 0 else length({field_expr}) > 0)"
            );
        }
        "is_empty" => {
            let _ = writeln!(out, "  expect_equal({field_expr}, \"\")");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(|v| json_to_r(v, false)).collect();
                let vec_str = items.join(", ");
                let _ = writeln!(
                    out,
                    "  expect_true(any(sapply(c({vec_str}), function(v) grepl(v, {field_expr}, fixed = TRUE))))"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let r_val = json_to_r(val, false);
                let _ = writeln!(out, "  expect_true({field_expr} > {r_val})");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let r_val = json_to_r(val, false);
                let _ = writeln!(out, "  expect_true({field_expr} < {r_val})");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let r_val = json_to_r(val, false);
                let _ = writeln!(out, "  expect_true({field_expr} >= {r_val})");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let r_val = json_to_r(val, false);
                let _ = writeln!(out, "  expect_true({field_expr} <= {r_val})");
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_true(startsWith({field_expr}, {r_val}))");
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_true(endsWith({field_expr}, {r_val}))");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "  expect_true(nchar({field_expr}) >= {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "  expect_true(nchar({field_expr}) <= {n})");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "  expect_true(length({field_expr}) >= {n})");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "  expect_equal(length({field_expr}), {n})");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "  expect_true({field_expr})");
        }
        "is_false" => {
            let _ = writeln!(out, "  expect_false({field_expr})");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let call_expr = build_r_method_call(result_var, method_name, assertion.args.as_ref());
                let check = assertion.check.as_deref().unwrap_or("is_true");
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out, "  expect_true({call_expr})");
                                } else {
                                    let _ = writeln!(out, "  expect_false({call_expr})");
                                }
                            } else {
                                let r_val = json_to_r(val, false);
                                let _ = writeln!(out, "  expect_equal({call_expr}, {r_val})");
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({call_expr})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_false({call_expr})");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_true({call_expr} >= {r_val})");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(out, "  expect_true(length({call_expr}) >= {n})");
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out, "  expect_error({call_expr})");
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_true(grepl({r_val}, {call_expr}, fixed = TRUE))");
                        }
                    }
                    other_check => {
                        panic!("R e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("R e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let r_val = json_to_r(expected, false);
                let _ = writeln!(out, "  expect_true(grepl({r_val}, {field_expr}))");
            }
        }
        "not_error" => {
            // Already handled — the call would stop on error.
        }
        "error" => {
            // Handled at the test level.
        }
        other => {
            panic!("R e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to an R literal string.
///
/// # Arguments
///
/// * `value` - The JSON value to convert
/// * `lowercase_enum_values` - If true, lowercase strings starting with uppercase letter (for enum values).
///   If false, preserve original case (for assertion expected values).
fn json_to_r(value: &serde_json::Value, lowercase_enum_values: bool) -> String {
    match value {
        serde_json::Value::String(s) => {
            // Lowercase enum values (strings starting with uppercase letter) only if requested
            let normalized = if lowercase_enum_values && s.chars().next().is_some_and(|c| c.is_uppercase()) {
                s.to_lowercase()
            } else {
                s.clone()
            };
            format!("\"{}\"", escape_r(&normalized))
        }
        serde_json::Value::Bool(true) => "TRUE".to_string(),
        serde_json::Value::Bool(false) => "FALSE".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(|v| json_to_r(v, lowercase_enum_values)).collect();
            format!("c({})", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("\"{}\" = {}", escape_r(k), json_to_r(v, lowercase_enum_values)))
                .collect();
            format!("list({})", entries.join(", "))
        }
    }
}

/// Build an R visitor list and add setup line.
fn build_r_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::fixture::VisitorSpec) {
    use std::fmt::Write as FmtWrite;
    // Collect each callback as a separate string, then join with ",\n" to avoid
    // trailing commas — R's list() does not accept a trailing comma.
    let methods: Vec<String> = visitor_spec
        .callbacks
        .iter()
        .map(|(method_name, action)| {
            let mut buf = String::new();
            emit_r_visitor_method(&mut buf, method_name, action);
            // strip the trailing ",\n" added by emit_r_visitor_method
            buf.trim_end_matches(['\n', ',']).to_string()
        })
        .collect();
    let mut visitor_obj = String::new();
    let _ = writeln!(visitor_obj, "list(");
    let _ = write!(visitor_obj, "{}", methods.join(",\n"));
    let _ = writeln!(visitor_obj);
    let _ = writeln!(visitor_obj, "  )");

    setup_lines.push(format!("visitor <- {visitor_obj}"));
}

/// Build an R call expression for a `method_result` assertion.
/// Maps method names to the appropriate R function or method calls.
fn build_r_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}$root_child_count()"),
        "root_node_type" => format!("{result_var}$root_node_type()"),
        "named_children_count" => format!("{result_var}$named_children_count()"),
        "has_error_nodes" => format!("tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("tree_error_count({result_var})"),
        "tree_to_sexp" => format!("tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("find_nodes_by_type({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| {
                                let r_val = json_to_r(v, false);
                                format!("{k} = {r_val}")
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("{result_var}${method_name}({arg_str})")
            } else {
                format!("{result_var}${method_name}()")
            }
        }
    }
}

/// Emit an R visitor method for a callback action.
fn emit_r_visitor_method(out: &mut String, method_name: &str, action: &CallbackAction) {
    use std::fmt::Write as FmtWrite;

    // R uses visit_ prefix (matches binding signature)
    let params = match method_name {
        "visit_link" => "ctx, href, text, title",
        "visit_image" => "ctx, src, alt, title",
        "visit_heading" => "ctx, level, text, id",
        "visit_code_block" => "ctx, lang, code",
        "visit_code_inline"
        | "visit_strong"
        | "visit_emphasis"
        | "visit_strikethrough"
        | "visit_underline"
        | "visit_subscript"
        | "visit_superscript"
        | "visit_mark"
        | "visit_button"
        | "visit_summary"
        | "visit_figcaption"
        | "visit_definition_term"
        | "visit_definition_description" => "ctx, text",
        "visit_text" => "ctx, text",
        "visit_list_item" => "ctx, ordered, marker, text",
        "visit_blockquote" => "ctx, content, depth",
        "visit_table_row" => "ctx, cells, is_header",
        "visit_custom_element" => "ctx, tag_name, html",
        "visit_form" => "ctx, action_url, method",
        "visit_input" => "ctx, input_type, name, value",
        "visit_audio" | "visit_video" | "visit_iframe" => "ctx, src",
        "visit_details" => "ctx, is_open",
        "visit_element_end" | "visit_table_end" | "visit_definition_list_end" | "visit_figure_end" => "ctx, output",
        "visit_list_start" => "ctx, ordered",
        "visit_list_end" => "ctx, ordered, output",
        _ => "ctx",
    };

    let _ = writeln!(out, "    {method_name} = function({params}) {{");
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "      \"skip\"");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "      \"continue\"");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "      \"preserve_html\"");
        }
        CallbackAction::Custom { output } => {
            let escaped = escape_r(output);
            let _ = writeln!(out, "      list(custom = \"{escaped}\")");
        }
        CallbackAction::CustomTemplate { template } => {
            let escaped = escape_r(template);
            let _ = writeln!(out, "      list(custom = \"{escaped}\")");
        }
    }
    let _ = writeln!(out, "    }},");
}
