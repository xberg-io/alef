//! R e2e test generator using testthat.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::core::version::to_r_version;
use crate::e2e::config::E2eConfig;
use crate::e2e::escape::{escape_r, r_template_to_paste0, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, CallbackAction, Fixture, FixtureGroup, TemplateReturnForm};
use anyhow::{Context as _, Result};
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
        config: &ResolvedCrateConfig,
        type_defs: &[crate::core::ir::TypeDef],
        _enums: &[crate::core::ir::EnumDef],
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
        let result_is_r_list = overrides.is_some_and(|o| o.result_is_r_list);
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
            .or_else(|| config.resolved_version())
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
            content: render_test_runner(&pkg_name, &pkg_path, e2e_config.dep_mode),
            generated_header: true,
        });

        // Generate install.R (registry mode only) — installs the published R package
        // from GitHub releases before running tests. The pinned version is baked in at
        // generate time so callers can run `Rscript install.R` with no args.
        if e2e_config.dep_mode == crate::e2e::config::DependencyMode::Registry {
            files.push(GeneratedFile {
                path: output_base.join("install.R"),
                content: render_install_r(
                    &pkg_name,
                    &pkg_version,
                    e2e_config
                        .registry
                        .github_repo
                        .as_deref()
                        .context("R registry mode requires `[crates.e2e.registry] github_repo`")?,
                ),
                generated_header: false,
            });
        }

        // setup-fixtures.R — testthat sources `setup-*.R` files in the tests
        // directory once before any tests run, with the working directory set
        // to the tests/ folder. We use this hook to chdir into the repo's
        // shared `test_documents/` directory so that fixture paths like
        // `pdf/fake_memo.pdf` resolve at extraction time.
        files.push(GeneratedFile {
            path: output_base.join("tests").join("setup-fixtures.R"),
            content: render_setup_fixtures(&e2e_config.test_documents_relative_from(1)),
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
            let content = render_test_file(
                &group.category,
                &active,
                result_is_simple,
                result_is_r_list,
                e2e_config,
                config,
                type_defs,
            );
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

fn render_description(pkg_name: &str, pkg_version: &str, dep_mode: crate::e2e::config::DependencyMode) -> String {
    let dep_line = match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            let r_version = to_r_version(pkg_version);
            format!("Imports: {pkg_name} ({r_version})\n")
        }
        crate::e2e::config::DependencyMode::Local => String::new(),
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

fn render_setup_fixtures(test_documents_path: &str) -> String {
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
        ".alef_test_documents <- normalizePath(\"{test_documents_path}\", mustWork = FALSE)"
    );
    let _ = writeln!(out, ".resolve_fixture <- function(path) {{");
    let _ = writeln!(out, "  if (dir.exists(.alef_test_documents)) {{");
    let _ = writeln!(out, "    file.path(.alef_test_documents, path)");
    let _ = writeln!(out, "  }} else {{");
    let _ = writeln!(out, "    path");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    // FormatMetadata is an internally-tagged enum (serde tag = "format_type")
    // so the JSON shape varies. `simplifyVector = FALSE` hands us a per-variant
    // list — keyed by the snake_case variant name (`image`, `excel`, ...) — that
    // points at the inner metadata struct, with all other variants set to NULL.
    // Collapse both shapes here so terminal `metadata$format` assertions see
    // the human-readable format string (e.g. "PNG") instead of the wrapper list.
    let _ = writeln!(
        out,
        ".alef_format_value <- function(x) {{
  if (is.list(x)) {{
    for (variant in names(x)) {{
      v <- x[[variant]]
      if (is.list(v) && !is.null(v[[\"format\"]]) && is.character(v[[\"format\"]])) {{
        return(v[[\"format\"]])
      }}
    }}
    if (!is.null(x[[\"format\"]]) && is.character(x[[\"format\"]])) {{
      return(x[[\"format\"]])
    }}
    if (!is.null(x[[\"format_type\"]])) {{
      return(x[[\"format_type\"]])
    }}
  }}
  x
}}"
    );
    out
}

fn render_test_runner(pkg_name: &str, pkg_path: &str, dep_mode: crate::e2e::config::DependencyMode) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "library(testthat)");
    match dep_mode {
        crate::e2e::config::DependencyMode::Registry => {
            // In registry mode, load the installed CRAN package. This must happen before
            // test_dir() runs so that all package functions are available to the tests.
            let _ = writeln!(out, "library({})", pkg_name);
        }
        crate::e2e::config::DependencyMode::Local => {
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

fn render_install_r(pkg_name: &str, pkg_version: &str, github_repo: &str) -> String {
    let github_repo = github_repo.trim_end_matches('/');
    let mut out = String::new();
    let _ = writeln!(out, "# alef-generated installer for registry-mode R test_app.");
    let _ = writeln!(out, "# Installs the configured R package from GitHub releases.");
    let _ = writeln!(out, "# Requires `R` on PATH.");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Version override: pass as commandArgs()[6] to test an");
    let _ = writeln!(out, "# arbitrary tag; defaults to the alef-pinned version from");
    let _ = writeln!(out, "# [crates.e2e.registry.packages.r].version.");
    let _ = writeln!(out, "args <- commandArgs(trailingOnly = TRUE)");
    let _ = writeln!(out, "VERSION <- if (length(args) > 0) args[1] else \"{pkg_version}\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Construct the GitHub release tarball URL.");
    let _ = writeln!(out, "url <- sprintf(");
    let _ = writeln!(out, "  \"{github_repo}/releases/download/v%s/{pkg_name}_%s.tar.gz\",");
    let _ = writeln!(out, "  VERSION,");
    let _ = writeln!(out, "  VERSION");
    let _ = writeln!(out, ")");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "# Install from the release tarball without requiring devtools or remotes."
    );
    let _ = writeln!(out, "tryCatch({{",);
    let _ = writeln!(
        out,
        "  install.packages(url, repos = NULL, type = \"source\", quiet = TRUE)"
    );
    let _ = writeln!(out, "  message(paste(\"Successfully installed {pkg_name}\", VERSION))");
    let _ = writeln!(out, "}}, error = function(e) {{");
    let _ = writeln!(out, "  message(paste(\"Error installing {pkg_name} from\", url))");
    let _ = writeln!(out, "  message(conditionMessage(e))");
    let _ = writeln!(out, "  quit(status = 1)");
    let _ = writeln!(out, "}})");
    out
}

fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    result_is_simple: bool,
    result_is_r_list: bool,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(
            &mut out,
            fixture,
            e2e_config,
            result_is_simple,
            result_is_r_list,
            config,
            type_defs,
        );
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
    default_result_is_simple: bool,
    default_result_is_r_list: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    let call_config = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let call_field_resolver = FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    );
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
    let result_var = &call_config.result_var;
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
    let args_str = build_args_string(
        &fixture.input,
        fixture.resolved_args(call_config),
        RArgsContext {
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
        build_r_visitor(&mut setup_lines, visitor_spec);
        // R rejects duplicated named arguments ("matched by multiple actual arguments"), so
        // strip any existing `options = ...` arg before appending the visitor-options list.
        // Handles `options = NULL` (when no default) and `options = <OptionsType>$default()`
        // (when build_args_string emits a default placeholder for an optional options arg).
        let base = strip_options_arg(&args_with_extra);
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
    // accessors. `result_is_simple` calls
    // already return scalar values and must be passed through verbatim.
    // `result_is_r_list` signals the binding returns a native R list (Robj),
    // not a JSON string — skip `jsonlite::fromJSON` but keep `$` accessors.
    // `returns_void` calls (trait-bridge `clear_*` wrappers that return `()`
    // in Rust → `NULL` in R) must not bind a `result` variable: the previous
    // emission of `result <- {function_name}(...)` was already correct when
    // `function_name` resolved, but parsers flag a stray `result` for void
    // calls. Use `invisible(...)` to make the void contract explicit.
    if call_config.returns_void {
        let _ = writeln!(out, "  invisible({function_name}({final_args}))");
    } else if result_is_simple || result_is_r_list {
        let _ = writeln!(out, "  {result_var} <- {function_name}({final_args})");
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
    for assertion in &fixture.assertions {
        let context = RAssertionContext {
            field_resolver,
            result_is_simple,
            result_is_bytes,
            assert_enum_fields,
        };
        render_assertion(out, assertion, result_var, &context);
    }

    // Emit teardown for trait-bridge tests to clean up registered test backends.
    for line in teardown_block.lines() {
        let _ = writeln!(out, "{line}");
    }

    let _ = writeln!(out, "}})");
}

/// Remove the named `options = …` argument (if any) from an R call-args string.
///
/// Walks the string while tracking paren/quote depth so a comma inside a nested
/// expression like `options = list(visitor = visitor)` isn't treated as the
/// arg terminator. Returns the rebuilt args string with the `options =` arg
/// dropped; callers append a fresh one.
fn strip_options_arg(args_str: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut paren_depth: i32 = 0;
    let mut in_single = false;
    let mut in_double = false;
    for c in args_str.chars() {
        if !in_single && !in_double {
            match c {
                '(' | '[' | '{' => paren_depth += 1,
                ')' | ']' | '}' => paren_depth -= 1,
                '\'' => in_single = true,
                '"' => in_double = true,
                ',' if paren_depth == 0 => {
                    parts.push(current.trim().to_string());
                    current.clear();
                    continue;
                }
                _ => {}
            }
        } else if in_single && c == '\'' {
            in_single = false;
        } else if in_double && c == '"' {
            in_double = false;
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
        .into_iter()
        .filter(|p| !p.starts_with("options ") && !p.starts_with("options="))
        .collect::<Vec<_>>()
        .join(", ")
}

struct RArgsContext<'a> {
    arg_name_map: Option<&'a std::collections::HashMap<String, String>>,
    options_type: Option<&'a str>,
    fixture: &'a Fixture,
    config: &'a ResolvedCrateConfig,
    type_defs: &'a [crate::core::ir::TypeDef],
    setup_lines: &'a mut Vec<String>,
    teardown_block: &'a mut String,
}

fn build_args_string(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    context: RArgsContext<'_>,
) -> String {
    let RArgsContext {
        arg_name_map,
        options_type,
        fixture,
        config,
        type_defs,
        setup_lines,
        teardown_block,
    } = context;
    if args.is_empty() {
        // No declared args means the wrapper takes zero parameters. Always
        // emit an empty arg list — fixtures may carry harness metadata under
        // `input` (e.g. `setup.lazy_init_required` for Go's eager-init shim)
        // that must not leak into the R call site as a positional `list(...)`.
        return String::new();
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
            // (`NULL` for `Option<T>`, `<OptionsType>$default()` for typed
            // configs) whenever the fixture omits an optional value.
            let val = match val {
                Some(v) if !(v.is_null() && arg.optional) => v,
                _ => {
                    if !arg.optional {
                        return None;
                    }
                    if arg.arg_type == "json_object" {
                        let r_value = r_default_for_config_arg(arg_name, options_type);
                        return Some(format!("{arg_name} = {r_value}"));
                    }
                    return Some(format!("{arg_name} = NULL"));
                }
            };
            // The extendr bindings expect owned PORs (ExternalPtr) for typed
            // config arguments — passing an R `list()` raises
            // `Expected ExternalPtr got List`. The fixtures don't carry the
            // option fields needed to round-trip through the configured type's constructor,
            // so emit `<OptionsType>$default()` whenever a `json_object` arg
            // resolves to an empty / object-shaped JSON value or NULL.
            if arg.arg_type == "json_object" && (val.is_null() || val.as_object().is_some_and(|m| m.is_empty())) {
                let r_value = r_default_for_config_arg(arg_name, options_type);
                return Some(format!("{arg_name} = {r_value}"));
            }
            // Non-empty json_object for typed config args (those whose default is a
            // `$default()` constructor): use `TypeName$from_json(jsonlite::toJSON(...))`
            // so the Rust function receives a proper ExternalPtr, not a list.
            // For `options`-style args (default = NULL) emit as a plain R list.
            if arg.arg_type == "json_object" && val.is_object() {
                let default_expr = r_default_for_config_arg(arg_name, options_type);
                if default_expr.ends_with("$default()") {
                    // Extract the type name from "TypeName$default()"
                    let type_name = default_expr.trim_end_matches("$default()");
                    // Use the `I(...)` (AsIs) wrapper for array-valued fields so
                    // `jsonlite::toJSON(..., auto_unbox = TRUE)` preserves them as
                    // JSON arrays. Without this, single-element vectors get
                    // unboxed to scalars (e.g. `c("foo")` → `"foo"`) and serde
                    // rejects them when deserializing `Vec<T>` fields.
                    let r_list = json_to_r_preserve_arrays(val, true);
                    let r_value = format!("{type_name}$from_json(jsonlite::toJSON({r_list}, auto_unbox = TRUE))");
                    return Some(format!("{arg_name} = {r_value}"));
                }
                let r_value = json_to_r(val, true);
                return Some(format!("{arg_name} = {r_value}"));
            }
            // `json_object` arrays are passed to extendr functions whose Rust
            // signature is `items: String` (JSON-serialized object items). The
            // wrapper has no R-list → JSON conversion, so we must serialize the
            // fixture value to a literal JSON string at test-emit time.
            //
            // Exception: when `element_type = "String"` the Rust signature is
            // `Vec<String>` (e.g. `embed_texts(texts: Vec<String>, ...)`), which
            // extendr binds as a native R character vector. Passing a JSON
            // literal there would land as a single-element character vector
            // containing the literal bytes `["a","b"]`, which is not what the
            // caller intended. Emit a plain `c("a","b")` literal instead.
            if arg.arg_type == "json_object" && val.is_array() {
                if arg.element_type.as_deref() == Some("String") {
                    // `c()` is `NULL` in R, which extendr rejects with
                    // `Expected Strings got Null` when the Rust signature is
                    // `Vec<String>`. Emit a typed empty char vector for the
                    // empty-input case so the binding sees `character(0)`.
                    let r_value = if val.as_array().is_some_and(|arr| arr.is_empty()) {
                        "character(0)".to_string()
                    } else {
                        json_to_r(val, false)
                    };
                    return Some(format!("{arg_name} = {r_value}"));
                }
                let json_literal = serde_json::to_string(val).unwrap_or_else(|_| "[]".to_string());
                let escaped = escape_r(&json_literal);
                return Some(format!("{arg_name} = \"{escaped}\""));
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
            // `test_backend` arg type: emit a test stub for trait implementations.
            if arg.arg_type == "test_backend" {
                if let Some(trait_name) = &arg.trait_name {
                    if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();
                        let emission = crate::e2e::codegen::emit_test_backend("r", trait_bridge, &methods, fixture);
                        // Emit the backend list definition before the call site.
                        if !emission.setup_block.is_empty() {
                            setup_lines.push(emission.setup_block.trim_end().to_string());
                        }
                        // Collect teardown for trait-bridge tests to clean up after assertions.
                        teardown_block.push_str(&emission.teardown_block);
                        return Some(format!("{arg_name} = {}", emission.arg_expr));
                    }
                }
                let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("r");
                return Some(format!("{arg_name} = NULL # {}", emission.arg_expr));
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
/// Falls back to `NULL` for unknown names so optional/default config slots stay
/// absent instead of passing a plain R list to an ExternalPtr-backed DTO.
///
/// When `options_type` is provided, emit the corresponding typed default.
/// Otherwise leave the optional slot unset instead of guessing a project type.
fn r_default_for_config_arg(arg_name: &str, options_type: Option<&str>) -> String {
    if let Some(type_name) = options_type {
        return format!("{type_name}$default()");
    }
    let _ = arg_name;
    "NULL".to_string()
}

struct RAssertionContext<'a> {
    field_resolver: &'a FieldResolver,
    result_is_simple: bool,
    result_is_bytes: bool,
    assert_enum_fields: &'a std::collections::HashMap<String, String>,
}

fn render_assertion(out: &mut String, assertion: &Assertion, result_var: &str, context: &RAssertionContext<'_>) {
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
            "chunks_have_heading_context" => {
                // prepend_heading_context adds heading text to chunk content, so verify chunks
                // exist and every chunk has non-empty content.
                let pred_true = format!(
                    "!is.null({result_var}$chunks) && length({result_var}$chunks) > 0 && all(sapply({result_var}$chunks, function(c) nchar(c$content) > 0))"
                );
                let pred_false = format!("is.null({result_var}$chunks) || length({result_var}$chunks) == 0");
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({pred_true})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_true({pred_false})");
                    }
                    _ => {
                        let _ = writeln!(out, "  # skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "first_chunk_starts_with_heading" => {
                // First chunk's content should start with a markdown heading marker (`#`)
                // when prepend_heading_context is enabled.
                let pred_true = format!(
                    "!is.null({result_var}$chunks) && length({result_var}$chunks) > 0 && startsWith(trimws({result_var}$chunks[[1]]$content), \"#\")"
                );
                let pred_false = format!(
                    "is.null({result_var}$chunks) || length({result_var}$chunks) == 0 || !startsWith(trimws({result_var}$chunks[[1]]$content), \"#\")"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "  expect_true({pred_true})");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "  expect_true({pred_false})");
                    }
                    _ => {
                        let _ = writeln!(out, "  # skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            // ---- EmbedResponse virtual fields ----
            // The extendr binding cannot return `Vec<Vec<f32>>` directly (extendr's
            // Robj conversion has no impl for nested numeric vectors), so the
            // wrapper serializes the result to a JSON string at the FFI boundary.
            // Parse it on demand here so length/index assertions operate on the
            // matrix structure rather than on the single string scalar.
            "embeddings" => {
                let parsed = format!(
                    "(if (is.character({result_var}) && length({result_var}) == 1) jsonlite::fromJSON({result_var}, simplifyVector = FALSE) else {result_var})"
                );
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_equal(length({parsed}), {r_val})");
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let r_val = json_to_r(val, false);
                            let _ = writeln!(out, "  expect_gte(length({parsed}), {r_val})");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "  expect_gt(length({parsed}), 0)");
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "  expect_equal(length({parsed}), 0)");
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
            // R ProcessingResult does not expose result_keywords; skip.
            "keywords" | "keywords_count" => {
                let _ = writeln!(out, "  # skipped: field '{f}' not available on R ProcessingResult");
                return;
            }
            _ => {}
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    // Exception: for result_is_simple, "result" is valid because it refers to the
    // result variable directly (which holds the plain string/array value).
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !context.field_resolver.is_valid_for_result(f) {
            // Allow "result" field on simple-type returns
            if !(context.result_is_simple && f == "result") {
                let _ = writeln!(out, "  # skipped: field '{f}' not available on result type");
                return;
            }
        }
    }

    // When result_is_simple, skip assertions that reference non-content fields
    // (e.g., metadata, document, structure) since the binding returns a plain value.
    if context.result_is_simple {
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

    let field_expr = if context.result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => context.field_resolver.accessor(f, "r", result_var),
            _ => result_var.to_string(),
        }
    };

    // Fields declared in `assert_enum_fields` map to sealed/internally-tagged enum
    // types.  Under `simplifyVector = FALSE`, such fields deserialize as named lists
    // keyed by the active variant.  Wrap the accessor with `.alef_format_value`
    // (defined in setup-fixtures.R) so the assertion sees the display string rather
    // than the raw list structure.
    let field_expr = match &assertion.field {
        Some(f) if context.assert_enum_fields.contains_key(f.as_str()) => {
            format!(".alef_format_value({field_expr})")
        }
        _ => field_expr,
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
                    let _ = writeln!(out, "  expect_true(any(grepl({r_val}, {field_expr}, fixed = TRUE)))");
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
            // Multi-element character vectors (e.g. `list_embedding_presets`)
            // would otherwise evaluate `nchar(x) > 0` element-wise and fail
            // `expect_true`'s scalar-logical contract. Reduce with `any()` so
            // the predicate stays a single TRUE/FALSE regardless of length,
            // and treat zero-length vectors as empty.
            let _ = writeln!(
                out,
                "  expect_true(if (is.character({field_expr})) length({field_expr}) > 0 && any(nchar({field_expr}) > 0) else length({field_expr}) > 0)"
            );
        }
        "is_empty" => {
            // Rust `Option<String>::None` surfaces as `NA_character_` through
            // extendr, and `Vec<...>` empties as a zero-length vector. Treat
            // NULL, NA, "", and zero-length collections as "empty" so the same
            // assertion works for scalar Option returns (`get_embedding_preset`)
            // and collection returns alike.
            let _ = writeln!(
                out,
                "  expect_true(is.null({field_expr}) || length({field_expr}) == 0 || (length({field_expr}) == 1 && (is.na({field_expr}) || identical({field_expr}, \"\"))))"
            );
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
                    // Raw byte returns (`result_is_bytes`) come back as an R
                    // raw vector; `nchar()` element-wises and breaks the
                    // expect_true scalar contract. Use `length()` to compare
                    // the byte count instead.
                    let size_fn = if context.result_is_bytes { "length" } else { "nchar" };
                    let _ = writeln!(out, "  expect_true({size_fn}({field_expr}) >= {n})");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let size_fn = if context.result_is_bytes { "length" } else { "nchar" };
                    let _ = writeln!(out, "  expect_true({size_fn}({field_expr}) <= {n})");
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
            // The call itself stops the test on error; emit an explicit
            // `expect_true(TRUE)` so testthat doesn't report the test as
            // empty when this is the only assertion.
            let _ = writeln!(out, "  expect_true(TRUE)");
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
///
/// Convert a PascalCase string to snake_case.
/// e.g. "DoubleEqual" → "double_equal", "Backticks" → "backticks"
fn pascal_to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        for lc in ch.to_lowercase() {
            result.push(lc);
        }
    }
    result
}

/// Convert a JSON value to an R expression suitable for embedding inside a
/// `list(...)` that will be passed to `jsonlite::toJSON(..., auto_unbox = TRUE)`.
///
/// Differs from [`json_to_r`] in that any array-valued field is wrapped with
/// `I(...)` (jsonlite's `AsIs` marker) so it remains a JSON array after the
/// `auto_unbox` transform. Empty arrays become `I(list())` (→ `[]`) and
/// non-empty arrays become `I(c(...))` (→ `[..]`). Without this wrapping,
/// `Vec<String>` fields like `exclude_selectors` get unboxed to scalars and
/// serde deserialization on the Rust side fails with
/// `invalid type: string "foo", expected a sequence`.
fn json_to_r_preserve_arrays(value: &serde_json::Value, lowercase_enum_values: bool) -> String {
    match value {
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                "I(list())".to_string()
            } else {
                let items: Vec<String> = arr.iter().map(|v| json_to_r(v, lowercase_enum_values)).collect();
                format!("I(c({}))", items.join(", "))
            }
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "\"{}\" = {}",
                        escape_r(k),
                        json_to_r_preserve_arrays(v, lowercase_enum_values)
                    )
                })
                .collect();
            format!("list({})", entries.join(", "))
        }
        _ => json_to_r(value, lowercase_enum_values),
    }
}

/// * `lowercase_enum_values` - If true, convert PascalCase strings to snake_case (for enum values).
///   If false, preserve original case (for assertion expected values).
fn json_to_r(value: &serde_json::Value, lowercase_enum_values: bool) -> String {
    match value {
        serde_json::Value::String(s) => {
            // Convert PascalCase enum values to snake_case only if requested.
            // e.g. "Backticks" → "backticks", "DoubleEqual" → "double_equal"
            let normalized = if lowercase_enum_values && s.chars().next().is_some_and(|c| c.is_uppercase()) {
                pascal_to_snake_case(s)
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
fn build_r_visitor(setup_lines: &mut Vec<String>, visitor_spec: &crate::e2e::fixture::VisitorSpec) {
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
        "visit_details" => "ctx, open",
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
        CallbackAction::CustomTemplate { template, return_form } => {
            let r_expr = r_template_to_paste0(template);
            match return_form {
                TemplateReturnForm::BareString => {
                    let _ = writeln!(out, "      {r_expr}");
                }
                TemplateReturnForm::Dict => {
                    let _ = writeln!(out, "      list(custom = {r_expr})");
                }
            }
        }
    }
    let _ = writeln!(out, "    }},");
}

/// Emit an R test backend stub.
///
/// Generates an R named-list object that satisfies the Rust extendr bridge
/// validation for the given trait.  The list contains one entry per required
/// method (those without `has_default_impl`) as anonymous R functions, plus a
/// `name` string entry for the Plugin super-trait when
/// `trait_bridge.super_trait.is_some()`.
///
/// Rules:
/// - Variable name: `r_backend_{sanitized_fixture_id}`.
/// - `name` key is a plain string (`"test"`), not a function — the Rust bridge
///   reads it as `r_obj.dollar("name")` expecting a character vector.
/// - Each required method key is the Rust snake_case method name.
/// - Return defaults come from `RDefaults`.
/// - The registration call uses `{register_fn}(r_backend_{id})`.
pub fn emit_test_backend(
    trait_bridge: &crate::core::config::TraitBridgeConfig,
    methods: &[&crate::core::ir::MethodDef],
    fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    use crate::codegen::defaults::language_defaults;
    use crate::e2e::escape::escape_r;

    let defaults = language_defaults("r");
    let backend_name = extract_backend_name_from_input(&fixture.input, &fixture.id);
    let var_name = format!("r_backend_{}", sanitize_ident(&fixture.id));

    let mut setup = String::new();

    let _ = writeln!(setup, "  {var_name} <- list(");

    // Collect required methods (those without default implementations).
    let required: Vec<_> = methods.iter().filter(|m| !m.has_default_impl).collect();

    // Plugin super-trait: emit `name`, `initialize`, and `shutdown` entries.
    // The R extendr trait bridge unconditionally calls `initialize` and
    // `shutdown` on every registered plugin (mirroring the python/ruby
    // bridges), so the R `list` stub must define them or registration
    // fails with `Plugin '<name>' missing method 'initialize'`.
    let super_trait_entries: Vec<String> = if trait_bridge.super_trait.is_some() {
        let escaped_name = escape_r(&backend_name);
        vec![
            format!("    name = \"{escaped_name}\""),
            "    initialize = function() invisible(NULL)".to_string(),
            "    shutdown = function() invisible(NULL)".to_string(),
        ]
    } else {
        vec![]
    };

    let total_entries = super_trait_entries.len() + required.len();
    let mut emitted = 0usize;

    for entry in &super_trait_entries {
        emitted += 1;
        let trailing = if emitted < total_entries { "," } else { "" };
        let _ = writeln!(setup, "{entry}{trailing}");
    }

    for method in required.iter() {
        let method_name = &method.name;

        // Try to extract method return value from fixture input, fall back to default.
        let method_val = if let Some(backend_obj) = fixture.input.get("backend") {
            if let Some(val) = backend_obj.get(method_name) {
                match val {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => format!("\"{}\"", escape_r(s)),
                    serde_json::Value::Bool(b) => {
                        if *b {
                            "TRUE".to_string()
                        } else {
                            "FALSE".to_string()
                        }
                    }
                    serde_json::Value::Array(_) => "c()".to_string(), // empty vector fallback
                    serde_json::Value::Null | serde_json::Value::Object(_) => {
                        defaults.emit_default(&method.return_type)
                    }
                }
            } else {
                defaults.emit_default(&method.return_type)
            }
        } else {
            defaults.emit_default(&method.return_type)
        };

        // Build parameter list: skip `&self` (no receiver in R).
        let params: Vec<&str> = method.params.iter().map(|p| p.name.as_str()).collect();
        let param_list = params.join(", ");

        emitted += 1;
        let trailing = if emitted < total_entries { "," } else { "" };
        let _ = writeln!(
            setup,
            "    {method_name} = function({param_list}) {method_val}{trailing}"
        );
    }

    let _ = writeln!(setup, "  )");

    // R test runner (testthat) runs each test in the same process, so registering a
    // test backend leaks into later tests. Emit `unregister_<trait>("backend_name")`
    // after the call+assertions to drain the test backend from the global registry.
    let teardown_block = trait_bridge
        .unregister_fn
        .as_deref()
        .map(|unregister_fn| {
            let escaped = escape_r(&backend_name);
            format!("  {unregister_fn}(\"{escaped}\")\n")
        })
        .unwrap_or_default();

    // The arg_expr is just the variable name — the outer call (the fixture's
    // configured function) supplies the registration wrapper.  The setup_block
    // containing the list definition must be emitted before the call site.
    super::TestBackendEmission {
        setup_block: setup,
        arg_expr: var_name,
        type_imports: Vec::new(),
        teardown_block,
    }
}

/// Extract a backend name string from the fixture input JSON.
///
/// Searches the top-level input object for the first string value at any depth
/// under keys commonly used for names (`name`, or the first string field found).
/// Falls back to the fixture id when no string is found.
fn extract_backend_name_from_input(input: &serde_json::Value, fallback: &str) -> String {
    // Walk the top-level object, then one level deeper, looking for "name".
    if let Some(obj) = input.as_object() {
        // Direct "name" key.
        if let Some(s) = obj.get("name").and_then(|v| v.as_str()) {
            return s.to_string();
        }
        for v in obj.values() {
            if let Some(inner) = v.as_object() {
                if let Some(s) = inner.get("name").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
        for v in obj.values() {
            if let Some(s) = v.as_str() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    /// Verify `emit_test_backend` is generic: output must not contain any
    /// hardcoded domain trait or method names — only names derived from the
    /// synthetic `TestTrait` / `do_work` inputs.
    #[test]
    fn test_emit_test_backend_is_generic_no_domain_names() {
        use crate::core::config::TraitBridgeConfig;
        use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};
        use crate::e2e::fixture::Fixture;

        let method = MethodDef {
            name: "do_work".to_string(),
            params: vec![ParamDef {
                name: "payload".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: crate::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let bridge = TraitBridgeConfig {
            trait_name: "TestTrait".to_string(),
            super_trait: Some("Plugin".to_string()),
            register_fn: Some("register_test_trait".to_string()),
            ..Default::default()
        };

        let fixture = Fixture {
            id: "my_fixture".to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            call: None,
            input: serde_json::Value::Null,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        };

        let methods = vec![&method];
        let emission = super::emit_test_backend(&bridge, &methods, &fixture);

        // The setup_block must contain the R method key name.
        assert!(
            emission.setup_block.contains("do_work"),
            "setup_block should contain the method 'do_work', got:\n{}",
            emission.setup_block
        );
        // The arg_expr is just the variable name — the outer fixture call handles registration.
        assert!(
            emission.arg_expr.contains("r_backend_"),
            "arg_expr should be the variable name (r_backend_*), got:\n{}",
            emission.arg_expr
        );
        // The super-trait name entry must be present and derived from the fixture id
        // (not a hardcoded backend name).
        assert!(
            emission.setup_block.contains("name = \"my_fixture\""),
            "setup_block should contain fixture-derived name = \"my_fixture\" for super-trait, got:\n{}",
            emission.setup_block
        );
        // The R extendr trait bridge unconditionally calls `initialize` and
        // `shutdown` on every registered plugin, so the stub must emit them
        // alongside `name` when `super_trait` is set.
        assert!(
            emission.setup_block.contains("initialize = function()"),
            "setup_block should contain initialize = function() for super-trait, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("shutdown = function()"),
            "setup_block should contain shutdown = function() for super-trait, got:\n{}",
            emission.setup_block
        );

        // Must not contain any hardcoded domain-specific names.
        for name in &[
            "ImageBackend",
            "RecordProvider",
            "process_image",
            "extract_bytes",
            "sample_lib",
        ] {
            assert!(
                !emission.setup_block.contains(name),
                "setup_block must not contain domain name '{name}', got:\n{}",
                emission.setup_block
            );
        }
    }
}

#[cfg(test)]
mod description_tests {
    use super::render_description;
    use crate::e2e::config::DependencyMode;

    #[test]
    fn render_description_registry_release_uses_plain_version() {
        let out = render_description("mypkg", "1.2.3", DependencyMode::Registry);
        assert!(out.contains("Imports: mypkg (1.2.3)"), "got: {out}");
    }

    #[test]
    fn render_description_registry_prerelease_uses_r_version_form() {
        // 3.6.0-rc.1 → 3.6.0.9001 (CRAN-compatible dev-pin form)
        let out = render_description("mypkg", "3.6.0-rc.1", DependencyMode::Registry);
        assert!(
            out.contains("Imports: mypkg (3.6.0.9001)"),
            "pre-release must use CRAN dev-pin form, got: {out}"
        );
        assert!(
            !out.contains("3.6.0-rc.1"),
            "raw semver dash form must not appear in DESCRIPTION, got: {out}"
        );
    }

    #[test]
    fn render_description_local_omits_imports_line() {
        let out = render_description("mypkg", "3.6.0-rc.1", DependencyMode::Local);
        assert!(
            !out.contains("Imports:"),
            "local mode must not emit Imports line, got: {out}"
        );
    }
}
