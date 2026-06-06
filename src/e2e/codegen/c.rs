//! C e2e test generator using assert.h and a Makefile.
//!
//! Generates `e2e/c/Makefile`, per-category `test_{category}.c` files,
//! a `main.c` test runner, a `test_runner.h` header, and a
//! `download_ffi.sh` script for downloading prebuilt FFI libraries from
//! GitHub releases.

use crate::core::backend::GeneratedFile;
use crate::core::config::AdapterPattern;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::escape::{escape_c, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// C e2e code generator.
pub struct CCodegen;

/// Returns true when `t` is a primitive C scalar type (uint64_t, int32_t, double,
/// etc.) that should be emitted as a typed local variable rather than a heap
/// `char*` accessor result.
fn is_primitive_c_type(t: &str) -> bool {
    matches!(
        t,
        "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uintptr_t"
            | "intptr_t"
            | "size_t"
            | "ssize_t"
            | "double"
            | "float"
            | "bool"
            | "int"
    )
}

/// Returns `true` when `fields_c_types["{parent}.{field}"]` is the magic
/// sentinel `"skip"` — the C codegen should omit any assertion that touches
/// this field rather than emitting a call to a non-existent FFI function.
fn is_skipped_c_field(fields_c_types: &HashMap<String, String>, parent_snake: &str, field_snake: &str) -> bool {
    let key = format!("{parent_snake}.{field_snake}");
    fields_c_types.get(&key).is_some_and(|t| t == "skip")
}

/// Infer the opaque-handle PascalCase return type for a bare-field accessor.
///
/// Returns `Some(pascal_type)` when the accessor `{prefix}_{parent}_{field}`
/// returns a pointer to an opaque struct (e.g. `SAMPLELLMUsage*`) rather than
/// a `char*` or primitive scalar.
///
/// Detection strategy:
/// 1. Direct lookup `fields_c_types["{parent}.{field}"]` — if present and
///    NOT a primitive AND NOT `char*`, treat as an opaque handle of that
///    PascalCase type.
/// 2. Inferred lookup — when ANY key in `fields_c_types` starts with
///    `"{field}."` (the snake_case of `field` as a parent type), the field
///    must be a struct whose nested fields are mapped. Default the struct
///    type to `field.to_pascal_case()`. This mirrors the fallback used by
///    `emit_nested_accessor` for intermediate segments.
///
/// Returns `None` when the field looks like a `char*` string accessor.
fn infer_opaque_handle_type(
    fields_c_types: &HashMap<String, String>,
    parent_snake_type: &str,
    field_snake: &str,
) -> Option<String> {
    let lookup_key = format!("{parent_snake_type}.{field_snake}");
    if let Some(t) = fields_c_types.get(&lookup_key) {
        if !is_primitive_c_type(t) && t != "char*" {
            return Some(t.clone());
        }
        // Primitive or explicit char* — caller handles those paths.
        return None;
    }
    // Inferred: nested keys exist with `field_snake` as the parent type prefix.
    let nested_prefix = format!("{field_snake}.");
    if fields_c_types.keys().any(|k| k.starts_with(&nested_prefix)) {
        return Some(field_snake.to_pascal_case());
    }
    None
}

/// Try to emit an enum-aware field accessor: when `raw_field`/`resolved_field`
/// is registered in `fields_enum` AND `fields_c_types[parent.field]` resolves
/// to a non-primitive PascalCase type name, treat the accessor return as an
/// opaque enum pointer and convert it to `char*` via the FFI's
/// `{prefix}_{enum_snake}_to_string` accessor.
///
/// Without this, the C codegen would default-declare the accessor result as
/// `char* status = {prefix}_batch_object_status(result);` and string-compare
/// it — but the FFI returns `SAMPLELLMBatchStatus*` (an opaque enum struct
/// pointer), not a C string. The mismatch causes immediate `Abort trap: 6` /
/// `strcmp(NULL,...)` failures in every assertion that targets an enum field.
///
/// Returns `true` when an accessor was emitted (caller must NOT emit the
/// default `char*` declaration). When emitted, the opaque-enum handle is
/// pushed to `intermediate_handles` so the existing cleanup loop frees it via
/// `{prefix}_{enum_snake}_free(...)` after the test body runs.
#[allow(clippy::too_many_arguments)]
fn try_emit_enum_accessor(
    out: &mut String,
    prefix: &str,
    prefix_upper: &str,
    raw_field: &str,
    resolved_field: &str,
    parent_snake_type: &str,
    accessor_fn: &str,
    parent_handle: &str,
    local_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
) -> bool {
    if !(fields_enum.contains(raw_field) || fields_enum.contains(resolved_field)) {
        return false;
    }
    let lookup_key = format!("{parent_snake_type}.{resolved_field}");
    let Some(enum_pascal) = fields_c_types.get(&lookup_key) else {
        return false;
    };
    if is_primitive_c_type(enum_pascal) || enum_pascal == "char*" {
        return false;
    }
    let enum_snake = enum_pascal.to_snake_case();
    let handle_var = format!("{local_var}_handle");
    let _ = writeln!(
        out,
        "    {prefix_upper}{enum_pascal}* {handle_var} = {accessor_fn}({parent_handle});"
    );
    let _ = writeln!(out, "    assert({handle_var} != NULL);");
    let _ = writeln!(
        out,
        "    char* {local_var} = {prefix}_{enum_snake}_to_string({handle_var});"
    );
    intermediate_handles.push((handle_var, enum_snake));
    true
}

impl E2eCodegen for CCodegen {
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

        // Resolve default call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let result_var = &call.result_var;
        let prefix = overrides
            .and_then(|o| o.prefix.as_ref())
            .cloned()
            .or_else(|| config.ffi.as_ref().and_then(|ffi| ffi.prefix.as_ref()).cloned())
            .unwrap_or_default();
        let header = overrides
            .and_then(|o| o.header.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_header_name());

        // Resolve package config.
        let c_pkg = e2e_config.resolve_package("c");
        // lib_name is the actual Rust library name (for linking)
        let lib_name = config.ffi_lib_name();

        // ffi_pkg_name is the release artifact package name (for downloads, may differ from lib_name).
        // Falls back to "{base_package_name}-ffi" when not explicitly configured in the package registry.
        let ffi_pkg_name = c_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                // Derive from the base package name (e.g., "<lib>" → "<lib>-ffi")
                // rather than from the cargo crate name (which may differ when
                // the C ffi crate uses a non-default suffix).
                format!("{}-ffi", config.name)
            });

        // Filter active groups (with non-skipped fixtures).
        let active_groups: Vec<(&FixtureGroup, Vec<&Fixture>)> = groups
            .iter()
            .filter_map(|group| {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                    .filter(|f| f.visitor.is_none())
                    .collect();
                if active.is_empty() { None } else { Some((group, active)) }
            })
            .collect();

        // Collect active visitor fixtures (flattened across all groups).
        let visitor_fixtures: Vec<&Fixture> = groups
            .iter()
            .flat_map(|group| group.fixtures.iter())
            .filter(|f| super::should_include_fixture(f, lang, e2e_config))
            .filter(|f| f.visitor.is_some())
            .filter(|f| c_visitor_fixture_has_typed_call(f, e2e_config))
            .collect();

        // Resolve FFI crate path for local repo builds.
        // Default to `../../crates/{name}-ffi` derived from the crate name so that
        // projects with named FFI crates resolve to `../../crates/{name}-ffi/include/`
        // rather than the generic (incorrect) `../../crates/ffi`.
        // When `[crates.output] ffi` is set explicitly, derive the crate path from
        // that value so that renamed FFI crates (e.g. `parser-core-core-ffi`) resolve
        // correctly without any hardcoded special cases.
        let ffi_crate_path = c_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_crate_path());

        // Generate Makefile.
        let mut category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
        if !visitor_fixtures.is_empty() {
            category_names.push("visitor".to_string());
        }
        let needs_mock_server = active_groups
            .iter()
            .flat_map(|(_, fixtures)| fixtures.iter())
            .any(|f| f.needs_mock_server());
        files.push(GeneratedFile {
            path: output_base.join("Makefile"),
            content: render_makefile(&category_names, &header, &ffi_crate_path, &lib_name, needs_mock_server),
            generated_header: true,
        });

        // Generate download_ffi.sh for downloading prebuilt FFI from GitHub releases.
        let github_repo = config.github_repo();
        let version = config.resolved_version().unwrap_or_else(|| "0.0.0".to_string());
        files.push(GeneratedFile {
            path: output_base.join("download_ffi.sh"),
            content: render_download_script(&github_repo, &version, &ffi_pkg_name),
            generated_header: true,
        });

        // Generate test_runner.h.
        files.push(GeneratedFile {
            path: output_base.join("test_runner.h"),
            content: render_test_runner_header(&active_groups, &visitor_fixtures),
            generated_header: true,
        });

        // Generate main.c.
        files.push(GeneratedFile {
            path: output_base.join("main.c"),
            content: render_main_c(&active_groups, &visitor_fixtures),
            generated_header: true,
        });

        // Generate .gitignore so locally-built binaries and mock-server pipe
        // artifacts are never accidentally checked in. A committed macOS Mach-O
        // `run_tests` binary will fail Linux CI with `Exec format error`.
        files.push(GeneratedFile {
            path: output_base.join(".gitignore"),
            content: render_gitignore(),
            generated_header: false,
        });

        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &std::collections::HashSet::new(),
        );

        // Generate per-category test files.
        // Each fixture may reference a named call config (fixture.call), so we pass
        // e2e_config to render_test_file so it can resolve per-fixture call settings.
        for (group, active) in &active_groups {
            let filename = format!("test_{}.c", sanitize_filename(&group.category));
            let content = render_test_file(
                &group.category,
                active,
                &header,
                &prefix,
                result_var,
                e2e_config,
                lang,
                &field_resolver,
                config,
                type_defs,
            );
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
                generated_header: true,
            });
        }

        // Generate test_visitor.c if there are visitor fixtures.
        if !visitor_fixtures.is_empty() {
            files.push(GeneratedFile {
                path: output_base.join("test_visitor.c"),
                content: render_visitor_test_file(&visitor_fixtures, &header, &prefix, e2e_config, config),
                generated_header: true,
            });
        }

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "c"
    }
}

/// Resolve per-call-config C-specific settings for a given call config and lang.
struct ResolvedCallInfo {
    function_name: String,
    result_type_name: String,
    options_type_name: String,
    client_factory: Option<String>,
    args: Vec<crate::e2e::config::ArgMapping>,
    raw_c_result_type: Option<String>,
    c_free_fn: Option<String>,
    c_engine_factory: Option<String>,
    result_is_option: bool,
    /// When `true`, the FFI signature for this method follows the byte-buffer
    /// out-pointer pattern: `int32_t fn(this, req, uint8_t** out_ptr,
    /// uintptr_t* out_len, uintptr_t* out_cap)`. The C codegen emits out-param
    /// declarations, a status-code check, and `<prefix>_free_bytes` rather
    /// than treating the result as an opaque response handle.
    result_is_bytes: bool,
    streaming: Option<bool>,
    /// Per-language `extra_args` from call overrides — verbatim trailing
    /// arguments appended after the configured `args`. The C codegen passes
    /// `NULL` for absent optional pointers via this mechanism.
    extra_args: Vec<String>,
}

fn resolve_call_info(call: &CallConfig, lang: &str) -> ResolvedCallInfo {
    let overrides = call.overrides.get(lang);
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.clone());
    // Fall back to the *base* (non-C-overridden) function name when no explicit
    // result_type is set.  Using the C-overridden name (e.g. "htm_convert") would
    // produce a doubled-prefix type like `HTMHtmConvert*`; the base name
    // ("convert") yields the correct `HTMConvert*` shape.
    let result_type_name = overrides
        .and_then(|o| o.result_type.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.to_pascal_case());
    let options_type_name = overrides
        .and_then(|o| o.options_type.as_deref())
        .or(call.options_type.as_deref())
        .unwrap_or_default()
        .to_string();
    let client_factory = overrides.and_then(|o| o.client_factory.as_ref()).cloned();
    let raw_c_result_type = overrides.and_then(|o| o.raw_c_result_type.clone());
    let c_free_fn = overrides.and_then(|o| o.c_free_fn.clone());
    let c_engine_factory = overrides.and_then(|o| o.c_engine_factory.clone());
    let result_is_option = overrides
        .and_then(|o| if o.result_is_option { Some(true) } else { None })
        .unwrap_or(call.result_is_option);
    // result_is_bytes is read from either the call-level config (preferred —
    // the byte-buffer FFI shape is identical across languages that use the
    // same FFI crate) or the per-language override (back-compat with the
    // pattern used by Java / PHP / etc.).
    let result_is_bytes = call.result_is_bytes || overrides.is_some_and(|o| o.result_is_bytes);
    let extra_args = overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    ResolvedCallInfo {
        function_name,
        result_type_name,
        options_type_name,
        client_factory,
        args: call.args.clone(),
        raw_c_result_type,
        c_free_fn,
        c_engine_factory,
        result_is_option,
        result_is_bytes,
        streaming: call.streaming_enabled(),
        extra_args,
    }
}

/// Resolve call info for a fixture, with fallback to default call's client_factory.
///
/// Named call configs (e.g. `[e2e.calls.embed]`) may not repeat the `client_factory`
/// setting. We fall back to the default `[e2e.call]` override's client_factory so that
/// all methods on the same client use the same pattern.
fn resolve_fixture_call_info(fixture: &Fixture, e2e_config: &E2eConfig, lang: &str) -> ResolvedCallInfo {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let mut info = resolve_call_info(call, lang);

    let default_overrides = e2e_config.call.overrides.get(lang);

    // Fallback: if the named call has no client_factory override, inherit from the
    // default call config so all calls use the same client pattern.
    if info.client_factory.is_none() {
        if let Some(factory) = default_overrides.and_then(|o| o.client_factory.as_ref()) {
            info.client_factory = Some(factory.clone());
        }
    }

    // Fallback: if the named call has no c_engine_factory override, inherit from the
    // default call config so all calls use the same engine pattern.
    if info.c_engine_factory.is_none() {
        if let Some(factory) = default_overrides.and_then(|o| o.c_engine_factory.as_ref()) {
            info.c_engine_factory = Some(factory.clone());
        }
    }

    info
}

fn c_visitor_fixture_has_typed_call(fixture: &Fixture, e2e_config: &E2eConfig) -> bool {
    let call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    let info = resolve_call_info(call, "c");
    let has_function = call
        .overrides
        .get("c")
        .and_then(|override_config| override_config.function.as_deref())
        .is_some_and(|function| !function.is_empty());
    has_function && !info.options_type_name.is_empty()
}

mod project;
mod runner;
mod visitor;

use project::{render_download_script, render_gitignore, render_makefile};
use runner::{render_main_c, render_test_runner_header};
use visitor::render_visitor_test_file;

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    header: &str,
    prefix: &str,
    result_var: &str,
    e2e_config: &E2eConfig,
    lang: &str,
    field_resolver: &FieldResolver,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "/* E2e tests for category: {category} */");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <assert.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out, "#include \"{header}\"");
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        // Visitor fixtures are filtered out before render_test_file is called.
        // This guard is a safety net in case a fixture reaches here unexpectedly.
        if fixture.visitor.is_some() {
            panic!(
                "C e2e generator: visitor pattern not supported for fixture: {}",
                fixture.id
            );
        }

        let call_info = resolve_fixture_call_info(fixture, e2e_config, lang);

        // Effective enum fields for this fixture: merge global e2e_config.fields_enum
        // (HashSet) with the per-call C override's enum_fields (HashMap keys). This
        // mirrors Ruby/Java's pattern: global = always-enum-typed paths; per-call =
        // context-dependent paths (BatchObject.status is BatchStatus, but
        // ResponseObject.status is plain String).
        let mut effective_fields_enum = e2e_config.fields_enum.clone();
        let fixture_call = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        if let Some(co) = fixture_call.overrides.get(lang) {
            for k in co.enum_fields.keys() {
                effective_fields_enum.insert(k.clone());
            }
        }

        // Per-call field resolver: overrides the top-level resolver when this call
        // declares its own result_fields / fields / fields_optional / fields_array.
        // Without this, `pages.length` on a `crawl` call would skip because the
        // default `result_fields` (configured for the top-level `scrape` call)
        // does not contain `pages`.
        let per_call_field_resolver = FieldResolver::new(
            e2e_config.effective_fields(fixture_call),
            e2e_config.effective_fields_optional(fixture_call),
            e2e_config.effective_result_fields(fixture_call),
            e2e_config.effective_fields_array(fixture_call),
            &std::collections::HashSet::new(),
        );
        let _ = field_resolver; // top-level resolver retained for compat; per-call wins
        let field_resolver = &per_call_field_resolver;

        render_test_function(
            &mut out,
            fixture,
            prefix,
            &call_info.function_name,
            result_var,
            &call_info.args,
            field_resolver,
            &e2e_config.fields_c_types,
            &effective_fields_enum,
            &call_info.result_type_name,
            &call_info.options_type_name,
            call_info.client_factory.as_deref(),
            call_info.raw_c_result_type.as_deref(),
            call_info.c_free_fn.as_deref(),
            call_info.c_engine_factory.as_deref(),
            call_info.result_is_option,
            call_info.result_is_bytes,
            call_info.streaming,
            &call_info.extra_args,
            config,
            type_defs,
        );
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    field_resolver: &FieldResolver,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    result_type_name: &str,
    options_type_name: &str,
    client_factory: Option<&str>,
    raw_c_result_type: Option<&str>,
    c_free_fn: Option<&str>,
    c_engine_factory: Option<&str>,
    result_is_option: bool,
    result_is_bytes: bool,
    streaming: Option<bool>,
    extra_args: &[String],
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let _ = writeln!(out, "void test_{fn_name}(void) {{");
    let _ = writeln!(out, "    /* {description} */");

    // Smoke/live fixtures gated on a required env var (e.g. OPENAI_API_KEY).
    // When the var is missing, treat as a successful skip — mirrors Python's
    // `pytest.skip("OPENAI_API_KEY not set")` and Java's `Assumptions.assumeTrue(...)`
    // so CI runs without provider credentials don't fail every smoke test.
    //
    // When the fixture also has a mock_response/http block, we support an env+mock
    // fallback: if the API key is set, use the real API; otherwise fall back to the
    // mock server. This lets the same fixture exercise both paths.
    let has_mock = fixture.needs_mock_server();
    let api_key_var = fixture.env.as_ref().and_then(|e| e.api_key_var.as_deref());
    if let Some(env) = &fixture.env {
        if let Some(var) = &env.api_key_var {
            let fixture_id = &fixture.id;
            if has_mock {
                let _ = writeln!(out, "    const char* api_key = getenv(\"{var}\");");
                let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
                let _ = writeln!(out, "    char base_url_buf[512];");
                let _ = writeln!(out, "    int use_mock = !(api_key && api_key[0] != '\\0');");
                let _ = writeln!(out, "    if (!use_mock) {{");
                let _ = writeln!(
                    out,
                    "        fprintf(stderr, \"{fixture_id}: using real API ({var} is set)\\n\");"
                );
                let _ = writeln!(out, "    }} else {{");
                let _ = writeln!(
                    out,
                    "        fprintf(stderr, \"{fixture_id}: using mock server ({var} not set)\\n\");"
                );
                let _ = writeln!(
                    out,
                    "        snprintf(base_url_buf, sizeof(base_url_buf), \"%s/fixtures/{fixture_id}\", mock_base ? mock_base : \"\");"
                );
                let _ = writeln!(out, "        api_key = \"test-key\";");
                let _ = writeln!(out, "    }}");
            } else {
                let _ = writeln!(out, "    if (getenv(\"{var}\") == NULL) {{ return; }}");
            }
        }
    }

    let prefix_upper = prefix.to_uppercase();

    // Engine-factory pattern: used when c_engine_factory is configured.
    // Creates a config handle from JSON, builds an engine, calls {prefix}_{function}(engine, url),
    // frees result and engine.
    if let Some(config_type) = c_engine_factory {
        render_engine_factory_test_function(
            out,
            fixture,
            prefix,
            function_name,
            result_var,
            field_resolver,
            fields_c_types,
            fields_enum,
            result_type_name,
            config_type,
            expects_error,
            raw_c_result_type,
        );
        return;
    }

    // Streaming adapters use an FFI iterator handle instead of a single
    // response. Emit start/next/free loop and aggregate per-chunk data
    // into local vars (chunks_count, stream_content, stream_complete) so fixture
    // assertions on pseudo-fields resolve to those locals rather than to
    // non-existent accessor functions on a single chunk handle.
    if client_factory.is_some() && crate::e2e::codegen::streaming_assertions::resolve_is_streaming(fixture, streaming) {
        let Some(streaming) = resolve_c_streaming_adapter(config, function_name) else {
            render_c_diagnostic_skip(
                out,
                "streaming fixture requires matching [[crates.adapters]] metadata for C e2e codegen",
            );
            return;
        };
        render_streaming_test_function(
            out,
            fixture,
            prefix,
            result_var,
            args,
            client_factory.unwrap_or(""),
            &streaming,
            expects_error,
            api_key_var,
        );
        return;
    }

    // Byte-buffer pattern: methods like `speech` and `file_content` return raw
    // bytes via the out-pointer FFI shape:
    //   `int32_t fn(this, req, uint8_t** out_ptr, uintptr_t* out_len, uintptr_t* out_cap)`
    // rather than as an opaque `*Response` handle. The C codegen must declare
    // the out-params, check the int32_t status code, and free with
    // `<prefix>_free_bytes` rather than emitting non-existent
    // `<prefix>_<response>_audio` / `_content` accessors.
    if let Some(factory) = client_factory {
        if result_is_bytes {
            let Some(client_owner_type) = resolve_c_client_owner_type(config, type_defs, function_name) else {
                render_c_diagnostic_skip(
                    out,
                    "client_factory is configured but C e2e could not resolve the client owner type",
                );
                return;
            };
            render_bytes_test_function(
                out,
                fixture,
                prefix,
                function_name,
                result_var,
                args,
                options_type_name,
                result_type_name,
                factory,
                &client_owner_type,
                expects_error,
            );
            return;
        }
    }

    // Client pattern: used when client_factory is configured.
    // Builds typed request handles from json_object args, creates a client via the
    // factory function, calls {prefix}_default_client_{function_name}(client, req),
    // then frees result, request handles, and client.
    if let Some(factory) = client_factory {
        let Some(client_owner_type) = resolve_c_client_owner_type(config, type_defs, function_name) else {
            render_c_diagnostic_skip(
                out,
                "client_factory is configured but C e2e could not resolve the client owner type",
            );
            return;
        };
        let mut request_handle_vars: Vec<(String, String)> = Vec::new(); // (arg_name, var_name)
        // Inline argument expressions appended after request handles in the
        // method call (e.g. literal C strings for `string` args, `NULL` for
        // optional pointer args). Order matches the position in `args`.
        let mut inline_method_args: Vec<String> = Vec::new();

        for arg in args {
            if arg.arg_type == "json_object" {
                // Prefer options_type from the C override when set, since the result
                // type isn't always a clean strip-Response/append-Request transform
                // (e.g. transcribe -> Create**Transcription**Request, not TranscriptionRequest).
                // Fall back to deriving from result_type for backward-compat cases.
                let request_type_pascal = if !options_type_name.is_empty() {
                    options_type_name.to_string()
                } else if let Some(stripped) = result_type_name.strip_suffix("Response") {
                    format!("{}Request", stripped)
                } else {
                    format!("{result_type_name}Request")
                };
                let request_type_snake = request_type_pascal.to_snake_case();
                let var_name = format!("{request_type_snake}_handle");

                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let json_val = if field.is_empty() || field == "input" {
                    Some(&fixture.input)
                } else {
                    fixture.input.get(field)
                };

                if let Some(val) = json_val {
                    if !val.is_null() {
                        let normalized = super::transform_json_keys_for_language(val, "snake_case");
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let escaped = escape_c(&json_str);
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{request_type_pascal}* {var_name} = \
                             {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                        );
                        if expects_error {
                            // For error fixtures (e.g. invalid enum value rejected by
                            // serde), `_from_json` may legitimately return NULL — that
                            // counts as the expected failure. Mirror Java's pattern of
                            // wrapping setup + call inside `assertThrows(...)` so error
                            // fixtures pass at *any* failure step. The test returns
                            // before attempting to create a client, leaving no
                            // resources to free.
                            let _ = writeln!(out, "    if ({var_name} == NULL) {{ return; }}");
                        } else {
                            let _ = writeln!(out, "    assert({var_name} != NULL && \"failed to build request\");");
                        }
                        request_handle_vars.push((arg.name.clone(), var_name));
                    }
                }
            } else if arg.arg_type == "string" {
                // String arg: read fixture input, emit as a C string literal inline.
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field);
                match val {
                    Some(v) if v.is_string() => {
                        let s = v.as_str().unwrap_or_default();
                        let escaped = escape_c(s);
                        inline_method_args.push(format!("\"{escaped}\""));
                    }
                    Some(serde_json::Value::Null) | None if arg.optional => {
                        inline_method_args.push("NULL".to_string());
                    }
                    None => {
                        inline_method_args.push("\"\"".to_string());
                    }
                    Some(other) => {
                        let s = serde_json::to_string(other).unwrap_or_default();
                        let escaped = escape_c(&s);
                        inline_method_args.push(format!("\"{escaped}\""));
                    }
                }
            } else if arg.optional {
                // Optional non-string, non-json_object arg: pass NULL.
                inline_method_args.push("NULL".to_string());
            }
        }

        let fixture_id = &fixture.id;
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        if has_mock && api_key_var.is_some() {
            // api_key and base_url_buf are already declared in the env-fallback block above.
            // use_mock was captured before api_key was potentially reassigned to "test-key",
            // so it correctly reflects the original env state.
            let _ = writeln!(out, "    const char* _base_url_arg = use_mock ? base_url_buf : NULL;");
            let _ = writeln!(
                out,
                "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else if has_mock {
            let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
            let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
            let _ = writeln!(out, "    char base_url[1024];");
            let _ = writeln!(
                out,
                "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
            );
            let _ = writeln!(
                out,
                "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else {
            let _ = writeln!(
                out,
                "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        }
        let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

        let method_args = if request_handle_vars.is_empty() && inline_method_args.is_empty() && extra_args.is_empty() {
            String::new()
        } else {
            let handles: Vec<String> = request_handle_vars.iter().map(|(_, v)| v.clone()).collect();
            let parts: Vec<String> = handles
                .into_iter()
                .chain(inline_method_args.iter().cloned())
                .chain(extra_args.iter().cloned())
                .collect();
            format!(", {}", parts.join(", "))
        };

        let call_fn = format!("{prefix}_default_client_{function_name}");

        if expects_error {
            let _ = writeln!(
                out,
                "    {prefix_upper}{result_type_name}* {result_var} = {call_fn}(client{method_args});"
            );
            for (_, var_name) in &request_handle_vars {
                let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
                let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
            }
            let _ = writeln!(out, "    {prefix}_default_client_free(client);");
            let _ = writeln!(out, "    assert({result_var} == NULL && \"expected call to fail\");");
            let _ = writeln!(out, "}}");
            return;
        }

        let _ = writeln!(
            out,
            "    {prefix_upper}{result_type_name}* {result_var} = {call_fn}(client{method_args});"
        );
        let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");

        let mut intermediate_handles: Vec<(String, String)> = Vec::new();
        let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
        // Locals declared as primitive C scalars (uint64_t, double, bool, ...).
        // Locals not present here default to char* (heap-allocated accessor result).
        let mut primitive_locals: HashMap<String, String> = HashMap::new();
        // Locals declared as opaque struct handles (e.g. SAMPLELLMUsage*).
        // Keyed by local_var, value is the snake_case type name used for free().
        let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field {
                if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                    let resolved_raw = field_resolver.resolve(f);
                    // Strip virtual namespace prefixes (e.g. "interaction.action_results[0].x"
                    // → "action_results[0].x") matching the same logic as FieldResolver::accessor.
                    let resolved = if let Some(stripped) = field_resolver.namespace_stripped_path(resolved_raw) {
                        let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                        let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                        if field_resolver.is_valid_for_result(stripped_first) {
                            stripped
                        } else {
                            resolved_raw
                        }
                    } else {
                        resolved_raw
                    };
                    let local_var = f.replace(['.', '['], "_").replace(']', "");
                    let has_map_access = resolved.contains('[');
                    if resolved.contains('.') {
                        let leaf_primitive = emit_nested_accessor(
                            out,
                            prefix,
                            resolved,
                            &local_var,
                            result_var,
                            fields_c_types,
                            fields_enum,
                            &mut intermediate_handles,
                            result_type_name,
                            f,
                        );
                        if let Some(prim) = leaf_primitive {
                            primitive_locals.insert(local_var.clone(), prim);
                        }
                    } else {
                        let result_type_snake = result_type_name.to_snake_case();
                        let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                        let lookup_key = format!("{result_type_snake}.{resolved}");
                        if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                            // Field marked "skip" — record sentinel so render_assertion skips it.
                            primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                        } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                            let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                            primitive_locals.insert(local_var.clone(), t.clone());
                        } else if try_emit_enum_accessor(
                            out,
                            prefix,
                            &prefix_upper,
                            f,
                            resolved,
                            &result_type_snake,
                            &accessor_fn,
                            result_var,
                            &local_var,
                            fields_c_types,
                            fields_enum,
                            &mut intermediate_handles,
                        ) {
                            // accessor emitted with enum-to-string conversion
                        } else if let Some(handle_pascal) =
                            infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                        {
                            // Opaque struct handle: cannot be read as char*.
                            let _ = writeln!(
                                out,
                                "    {prefix_upper}{handle_pascal}* {local_var} = {accessor_fn}({result_var});"
                            );
                            opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                        } else {
                            let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                        }
                    }
                    accessed_fields.push((f.clone(), local_var, has_map_access));
                }
            }
        }

        for assertion in &fixture.assertions {
            render_assertion(
                out,
                assertion,
                result_var,
                prefix,
                field_resolver,
                &accessed_fields,
                &primitive_locals,
                &opaque_handle_locals,
            );
        }

        for (_f, local_var, from_json) in &accessed_fields {
            if primitive_locals.contains_key(local_var) {
                continue;
            }
            if let Some(snake_type) = opaque_handle_locals.get(local_var) {
                let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
                continue;
            }
            if *from_json {
                let _ = writeln!(out, "    free({local_var});");
            } else {
                let _ = writeln!(out, "    {prefix}_free_string({local_var});");
            }
        }
        for (handle_var, snake_type) in intermediate_handles.iter().rev() {
            if snake_type == "free_string" {
                let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
            } else if snake_type == "free" {
                // Intermediate JSON-key extraction (alef_json_get_string) — heap
                // char* allocated by malloc-class helper; freed via plain free().
                let _ = writeln!(out, "    free({handle_var});");
            } else {
                let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
            }
        }
        let result_type_snake = result_type_name.to_snake_case();
        let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
        let _ = writeln!(out, "}}");
        return;
    }

    // Raw C result type path: functions returning a primitive C type (char*, int32_t,
    // uintptr_t) rather than an opaque handle pointer.
    if let Some(raw_type) = raw_c_result_type {
        // Build argument string. Void-arg functions pass nothing.
        let args_str = if args.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = args
                .iter()
                .filter_map(|arg| {
                    let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                    let val = fixture.input.get(field);
                    match val {
                        None if arg.optional => Some("NULL".to_string()),
                        None => None,
                        Some(v) if v.is_null() && arg.optional => Some("NULL".to_string()),
                        Some(v) => Some(json_to_c(v)),
                    }
                })
                .collect();
            parts.join(", ")
        };

        // Declare result variable.
        let _ = writeln!(out, "    {raw_type} {result_var} = {function_name}({args_str});");

        // not_error assertion.
        let has_not_error = fixture.assertions.iter().any(|a| a.assertion_type == "not_error");
        if has_not_error {
            match raw_type {
                "char*" if !result_is_option => {
                    let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");
                }
                "int32_t" => {
                    let _ = writeln!(out, "    assert({result_var} >= 0 && \"expected call to succeed\");");
                }
                "uintptr_t" => {
                    let _ = writeln!(
                        out,
                        "    assert({prefix}_last_error_code() == 0 && \"expected call to succeed\");"
                    );
                }
                _ => {}
            }
        }

        // Other assertions.
        for assertion in &fixture.assertions {
            match assertion.assertion_type.as_str() {
                "not_error" | "error" => {} // handled above / not applicable
                "not_empty" => {
                    let _ = writeln!(
                        out,
                        "    assert({result_var} != NULL && strlen({result_var}) > 0 && \"expected non-empty value\");"
                    );
                }
                "is_empty" => {
                    if result_is_option && raw_type == "char*" {
                        let _ = writeln!(
                            out,
                            "    assert({result_var} == NULL && \"expected empty/null value\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert(strlen({result_var}) == 0 && \"expected empty value\");"
                        );
                    }
                }
                "count_min" => {
                    if let Some(val) = &assertion.value {
                        if let Some(n) = val.as_u64() {
                            match raw_type {
                                "char*" => {
                                    let _ = writeln!(out, "    {{");
                                    let _ = writeln!(
                                        out,
                                        "        assert({result_var} != NULL && \"expected non-null JSON array\");"
                                    );
                                    let _ =
                                        writeln!(out, "        int elem_count = alef_json_array_count({result_var});");
                                    let _ = writeln!(
                                        out,
                                        "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
                                    );
                                    let _ = writeln!(out, "    }}");
                                }
                                _ => {
                                    let _ = writeln!(
                                        out,
                                        "    assert((size_t){result_var} >= {n} && \"expected at least {n} elements\");"
                                    );
                                }
                            }
                        }
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert({result_var} >= {c_val} && \"expected greater than or equal\");"
                        );
                    }
                }
                "contains" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strstr({result_var}, {c_val}) != NULL && \"expected to contain substring\");"
                        );
                    }
                }
                "contains_all" => {
                    if let Some(values) = &assertion.values {
                        for val in values {
                            let c_val = json_to_c(val);
                            let _ = writeln!(
                                out,
                                "    assert(strstr({result_var}, {c_val}) != NULL && \"expected to contain substring\");"
                            );
                        }
                    }
                }
                "equals" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        if val.is_string() {
                            let _ = writeln!(
                                out,
                                "    assert({result_var} != NULL && str_trim_eq({result_var}, {c_val}) == 0 && \"equals assertion failed\");"
                            );
                        } else {
                            let _ = writeln!(
                                out,
                                "    assert({result_var} == {c_val} && \"equals assertion failed\");"
                            );
                        }
                    }
                }
                "not_contains" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strstr({result_var}, {c_val}) == NULL && \"expected NOT to contain substring\");"
                        );
                    }
                }
                "starts_with" => {
                    if let Some(val) = &assertion.value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "    assert(strncmp({result_var}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                        );
                    }
                }
                "is_true" => {
                    let _ = writeln!(out, "    assert({result_var});");
                }
                "is_false" => {
                    let _ = writeln!(out, "    assert(!{result_var});");
                }
                other => {
                    panic!("C e2e raw-result generator: unsupported assertion type: {other}");
                }
            }
        }

        // Free char* results.
        if raw_type == "char*" {
            let free_fn = c_free_fn
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{prefix}_free_string"));
            if result_is_option {
                let _ = writeln!(out, "    if ({result_var} != NULL) {{ {free_fn}({result_var}); }}");
            } else {
                let _ = writeln!(out, "    {free_fn}({result_var});");
            }
        }

        let _ = writeln!(out, "}}");
        return;
    }

    // Legacy (non-client) path: call the function directly.
    // Used for libraries that expose standalone FFI functions.

    // Use the function name directly — the override already includes the prefix
    // (e.g. "htm_convert"), so we must NOT prepend it again.
    let prefixed_fn = function_name.to_string();

    // For json_object args, emit a from_json call to construct the options handle.
    let mut has_options_handle = false;
    for arg in args {
        if arg.arg_type == "json_object" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            if let Some(val) = fixture.input.get(field) {
                if !val.is_null() {
                    // Fixture keys are camelCase; generated FFI from_json helpers
                    // deserialize into Rust types using serde's configured casing.
                    // Normalize keys before serializing.
                    let normalized = super::transform_json_keys_for_language(val, "snake_case");
                    let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                    let escaped = escape_c(&json_str);
                    let upper = prefix.to_uppercase();
                    let options_type_pascal = options_type_name;
                    let options_type_snake = options_type_name.to_snake_case();
                    let _ = writeln!(
                        out,
                        "    {upper}{options_type_pascal}* options_handle = {prefix}_{options_type_snake}_from_json(\"{escaped}\");"
                    );
                    has_options_handle = true;
                }
            }
        }
    }

    let args_str = build_args_string_c(&fixture.input, args, has_options_handle, config, type_defs, fixture);

    if expects_error {
        let _ = writeln!(
            out,
            "    {prefix_upper}{result_type_name}* {result_var} = {prefixed_fn}({args_str});"
        );
        if has_options_handle {
            let options_type_snake = options_type_name.to_snake_case();
            let _ = writeln!(out, "    {prefix}_{options_type_snake}_free(options_handle);");
        }
        let _ = writeln!(out, "    assert({result_var} == NULL && \"expected call to fail\");");
        let _ = writeln!(out, "}}");
        return;
    }

    // The FFI returns an opaque handle; extract the content string from it.
    let _ = writeln!(
        out,
        "    {prefix_upper}{result_type_name}* {result_var} = {prefixed_fn}({args_str});"
    );
    let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");

    // Collect fields accessed by assertions so we can emit accessor calls.
    // C FFI uses the opaque handle pattern: {prefix}_conversion_result_{field}(handle).
    // For nested paths we generate chained FFI accessor calls using the type
    // chain from `fields_c_types`.
    // Each entry: (fixture_field, local_var, from_json_extract).
    // `from_json_extract` is true when the variable was extracted from a JSON
    // map via alef_json_get_string and needs free() instead of {prefix}_free_string().
    let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
    // Track intermediate handles emitted so we can free them and avoid duplicates.
    // Each entry: (handle_var_name, snake_type_name) — freed in reverse order.
    let mut intermediate_handles: Vec<(String, String)> = Vec::new();
    // Locals declared as primitive C scalars (uint64_t, double, bool, ...).
    let mut primitive_locals: HashMap<String, String> = HashMap::new();
    // Locals declared as opaque struct handles (e.g. SAMPLELLMUsage*).
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                let resolved_raw = field_resolver.resolve(f);
                // Strip virtual namespace prefixes (e.g. "interaction.action_results[0].x"
                // → "action_results[0].x") matching the same logic as FieldResolver::accessor.
                let resolved = if let Some(stripped) = field_resolver.namespace_stripped_path(resolved_raw) {
                    let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                    let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                    if field_resolver.is_valid_for_result(stripped_first) {
                        stripped
                    } else {
                        resolved_raw
                    }
                } else {
                    resolved_raw
                };
                let local_var = f.replace(['.', '['], "_").replace(']', "");
                let has_map_access = resolved.contains('[');

                if resolved.contains('.') {
                    let leaf_result = emit_nested_accessor(
                        out,
                        prefix,
                        resolved,
                        &local_var,
                        result_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                        result_type_name,
                        f,
                    );
                    if let Some(returned_type) = leaf_result {
                        // Could be a primitive type (primitive_locals) or opaque handle type
                        if is_primitive_c_type(&returned_type) {
                            primitive_locals.insert(local_var.clone(), returned_type);
                        } else {
                            // Opaque handle returned — register for cleanup
                            opaque_handle_locals.insert(local_var.clone(), returned_type);
                        }
                    }
                } else {
                    let result_type_snake = result_type_name.to_snake_case();
                    let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                    let lookup_key = format!("{result_type_snake}.{resolved}");
                    if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                        // Field marked "skip" — record sentinel so render_assertion skips it.
                        primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                    } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                        let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                        primitive_locals.insert(local_var.clone(), t.clone());
                    } else if try_emit_enum_accessor(
                        out,
                        prefix,
                        &prefix_upper,
                        f,
                        resolved,
                        &result_type_snake,
                        &accessor_fn,
                        result_var,
                        &local_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                    ) {
                        // accessor emitted with enum-to-string conversion
                    } else if let Some(handle_pascal) =
                        infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                    {
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{handle_pascal}* {local_var} = {accessor_fn}({result_var});"
                        );
                        opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                    } else {
                        let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                    }
                }
                accessed_fields.push((f.clone(), local_var.clone(), has_map_access));
            }
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            prefix,
            field_resolver,
            &accessed_fields,
            &primitive_locals,
            &opaque_handle_locals,
        );
    }

    // Free extracted leaf strings.
    for (_f, local_var, from_json) in &accessed_fields {
        if primitive_locals.contains_key(local_var) {
            continue;
        }
        if let Some(snake_type) = opaque_handle_locals.get(local_var) {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
            continue;
        }
        if *from_json {
            let _ = writeln!(out, "    free({local_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_free_string({local_var});");
        }
    }
    // Free intermediate handles in reverse order.
    for (handle_var, snake_type) in intermediate_handles.iter().rev() {
        if snake_type == "free_string" {
            // free_string handles are freed with the free_string function directly.
            let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
        } else if snake_type == "free" {
            // Intermediate JSON-key extraction (e.g. alef_json_array_get_index) — freed via plain free().
            let _ = writeln!(out, "    free({handle_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
        }
    }
    if has_options_handle {
        let options_type_snake = options_type_name.to_snake_case();
        let _ = writeln!(out, "    {prefix}_{options_type_snake}_free(options_handle);");
    }
    let result_type_snake = result_type_name.to_snake_case();
    let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
    let _ = writeln!(out, "}}");
}

/// Emit a test function using the engine-factory pattern:
///   `{prefix}_crawl_config_from_json(json)` → `{prefix}_create_engine(config)` →
///   `{prefix}_{function}(engine, url)` → assertions → free chain.
///
/// When all fixture assertions are skipped (fields not present on result type,
/// or only "error" assertions that C cannot replicate via a simple URL scrape),
/// the null-check is a soft guard (`if (result != NULL)`) so the test does not
/// abort when the mock server has no matching route.
#[allow(clippy::too_many_arguments)]
fn render_engine_factory_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    result_type_name: &str,
    config_type: &str,
    expects_error: bool,
    raw_c_result_type: Option<&str>,
) {
    let prefix_upper = prefix.to_uppercase();
    let config_snake = config_type.to_snake_case();

    // Build config JSON from fixture input (snake_case keys).
    let config_val = fixture.input.get("config");
    let config_json = match config_val {
        Some(v) if !v.is_null() => {
            let normalized = super::transform_json_keys_for_language(v, "snake_case");
            serde_json::to_string(&normalized).unwrap_or_else(|_| "{}".to_string())
        }
        _ => "{}".to_string(),
    };
    let config_escaped = escape_c(&config_json);
    let fixture_id = &fixture.id;

    // An assertion is "active" when it has a field that is valid for the result type.
    // Error-only assertions are NOT treated as active for the engine factory pattern
    // because C's kcrawl_scrape() doesn't replicate batch/validation error semantics.
    let has_active_assertions = fixture.assertions.iter().any(|a| {
        if let Some(f) = &a.field {
            !f.is_empty() && field_resolver.is_valid_for_result(f)
        } else {
            false
        }
    });

    // --- engine setup ---
    let _ = writeln!(
        out,
        "    {prefix_upper}{config_type}* config_handle = \
         {prefix}_{config_snake}_from_json(\"{config_escaped}\");"
    );
    if expects_error {
        // Config parsing may legitimately fail for error fixtures (e.g. invalid config
        // rejected by the FFI layer). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (config_handle == NULL) {{ return; }}");
    } else {
        let _ = writeln!(out, "    assert(config_handle != NULL && \"failed to parse config\");");
    }
    let _ = writeln!(
        out,
        "    {prefix_upper}CrawlEngineHandle* engine = {prefix}_create_engine(config_handle);"
    );
    let _ = writeln!(out, "    {prefix}_{config_snake}_free(config_handle);");
    if expects_error {
        // Engine creation may legitimately fail for error fixtures (e.g. invalid config
        // rejected at engine-creation time). Return early — that counts as the expected failure.
        let _ = writeln!(out, "    if (engine == NULL) {{ return; }}");
    } else {
        let _ = writeln!(out, "    assert(engine != NULL && \"failed to create engine\");");
    }

    // --- URL construction: prefer per-fixture MOCK_SERVER_<UPPER_ID> (for fixtures
    // that need host-root routes like /robots.txt or /sitemap.xml), fall back to
    // MOCK_SERVER_URL/fixtures/<id> for the common case. ---
    let fixture_env_key = format!("MOCK_SERVER_{}", fixture_id.to_uppercase());
    let _ = writeln!(out, "    const char* mock_per_fixture = getenv(\"{fixture_env_key}\");");
    let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
    let _ = writeln!(out, "    char url[2048];");
    let _ = writeln!(out, "    if (mock_per_fixture && mock_per_fixture[0] != '\\0') {{");
    let _ = writeln!(out, "        snprintf(url, sizeof(url), \"%s\", mock_per_fixture);");
    let _ = writeln!(out, "    }} else {{");
    let _ = writeln!(
        out,
        "        assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");"
    );
    let _ = writeln!(
        out,
        "        snprintf(url, sizeof(url), \"%s/fixtures/{fixture_id}\", mock_base);"
    );
    let _ = writeln!(out, "    }}");

    // --- actions argument (interact and similar 3-arg engine-factory calls) ---
    // When the fixture input contains an "actions" key (interaction fixtures), the FFI
    // function signature is `{prefix}_{fn}(engine, url, actions_json)`.  Serialize the
    // actions value to a JSON string and emit a local `const char*` that is appended as
    // the third positional argument.
    let actions_arg = fixture.input.get("actions").and_then(|v| {
        if v.is_null() {
            None
        } else {
            let normalized = super::transform_json_keys_for_language(v, "snake_case");
            let json = serde_json::to_string(&normalized).ok()?;
            let escaped = escape_c(&json);
            Some(escaped)
        }
    });
    if let Some(ref escaped_actions) = actions_arg {
        let _ = writeln!(out, "    const char* actions_json = \"{escaped_actions}\";");
    }

    // --- call ---
    // Determine the trailing extra arguments beyond (engine, url).
    let extra_call_args = if actions_arg.is_some() {
        ", actions_json".to_string()
    } else {
        String::new()
    };

    // When the function returns a raw C type that is NOT an opaque struct pointer, emit a
    // plain variable declaration.
    //   • "char*" — JSON-returning helpers (batch_scrape historic config); use char* type
    //     and free with {prefix}_free_string.
    //   • Any other non-empty value — treat as an opaque PascalCase type name, emit
    //     {PREFIX}{Type}* and free with {prefix}_{type_snake}_free.  Callers set this when
    //     the function returns a named result struct (e.g. "BatchCrawlResults") that has no
    //     structured field accessors to assert on.
    if let Some(raw_type) = raw_c_result_type {
        if raw_type == "char*" {
            let _ = writeln!(
                out,
                "    char* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
            );
            let _ = writeln!(out, "    if ({result_var} != NULL) {prefix}_free_string({result_var});");
            let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
            let _ = writeln!(out, "}}");
            return;
        } else {
            // Opaque struct return: emit the typed pointer, a soft null-guard, and the
            // matching free function derived from the snake_case type name.
            let raw_snake = raw_type.to_snake_case();
            let _ = writeln!(
                out,
                "    {prefix_upper}{raw_type}* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
            );
            let _ = writeln!(
                out,
                "    if ({result_var} != NULL) {prefix}_{raw_snake}_free({result_var});"
            );
            let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
            let _ = writeln!(out, "}}");
            return;
        }
    }

    let _ = writeln!(
        out,
        "    {prefix_upper}{result_type_name}* {result_var} = {prefix}_{function_name}(engine, url{extra_call_args});"
    );

    // When no assertions can be verified (all skipped or error-only), use a soft
    // null-guard so the test is a no-op rather than aborting on a NULL result.
    if !has_active_assertions {
        let result_type_snake = result_type_name.to_snake_case();
        let _ = writeln!(
            out,
            "    if ({result_var} != NULL) {prefix}_{result_type_snake}_free({result_var});"
        );
        let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(out, "    assert({result_var} != NULL && \"expected call to succeed\");");

    // --- field assertions ---
    let mut intermediate_handles: Vec<(String, String)> = Vec::new();
    let mut accessed_fields: Vec<(String, String, bool)> = Vec::new();
    let mut primitive_locals: HashMap<String, String> = HashMap::new();
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && field_resolver.is_valid_for_result(f) && !accessed_fields.iter().any(|(k, _, _)| k == f)
            {
                let resolved_raw = field_resolver.resolve(f);
                // Strip virtual namespace prefixes (e.g. "interaction.action_results[0].x"
                // → "action_results[0].x") matching the same logic as FieldResolver::accessor.
                let resolved = if let Some(stripped) = field_resolver.namespace_stripped_path(resolved_raw) {
                    let stripped_first = stripped.split('.').next().unwrap_or(stripped);
                    let stripped_first = stripped_first.split('[').next().unwrap_or(stripped_first);
                    if field_resolver.is_valid_for_result(stripped_first) {
                        stripped
                    } else {
                        resolved_raw
                    }
                } else {
                    resolved_raw
                };
                let local_var = f.replace(['.', '['], "_").replace(']', "");
                let has_map_access = resolved.contains('[');
                if resolved.contains('.') {
                    let leaf_result = emit_nested_accessor(
                        out,
                        prefix,
                        resolved,
                        &local_var,
                        result_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                        result_type_name,
                        f,
                    );
                    if let Some(returned_type) = leaf_result {
                        // Could be a primitive type (primitive_locals) or opaque handle type
                        if is_primitive_c_type(&returned_type) {
                            primitive_locals.insert(local_var.clone(), returned_type);
                        } else {
                            // Opaque handle returned — register for cleanup
                            opaque_handle_locals.insert(local_var.clone(), returned_type);
                        }
                    }
                } else {
                    let result_type_snake = result_type_name.to_snake_case();
                    let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                    let lookup_key = format!("{result_type_snake}.{resolved}");
                    if is_skipped_c_field(fields_c_types, &result_type_snake, resolved) {
                        // Field marked "skip" — record sentinel so render_assertion skips it.
                        primitive_locals.insert(local_var.clone(), "__skip__".to_string());
                    } else if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                        let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({result_var});");
                        primitive_locals.insert(local_var.clone(), t.clone());
                    } else if try_emit_enum_accessor(
                        out,
                        prefix,
                        &prefix_upper,
                        f,
                        resolved,
                        &result_type_snake,
                        &accessor_fn,
                        result_var,
                        &local_var,
                        fields_c_types,
                        fields_enum,
                        &mut intermediate_handles,
                    ) {
                        // accessor emitted with enum-to-string conversion
                    } else if let Some(handle_pascal) =
                        infer_opaque_handle_type(fields_c_types, &result_type_snake, resolved)
                    {
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{handle_pascal}* {local_var} = {accessor_fn}({result_var});"
                        );
                        opaque_handle_locals.insert(local_var.clone(), handle_pascal.to_snake_case());
                    } else {
                        let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                    }
                }
                accessed_fields.push((f.clone(), local_var, has_map_access));
            }
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(
            out,
            assertion,
            result_var,
            prefix,
            field_resolver,
            &accessed_fields,
            &primitive_locals,
            &opaque_handle_locals,
        );
    }

    // --- free locals ---
    for (_f, local_var, from_json) in &accessed_fields {
        if primitive_locals.contains_key(local_var) {
            continue;
        }
        if let Some(snake_type) = opaque_handle_locals.get(local_var) {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({local_var});");
            continue;
        }
        if *from_json {
            let _ = writeln!(out, "    free({local_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_free_string({local_var});");
        }
    }
    for (handle_var, snake_type) in intermediate_handles.iter().rev() {
        if snake_type == "free_string" {
            let _ = writeln!(out, "    {prefix}_free_string({handle_var});");
        } else if snake_type == "free" {
            // Intermediate JSON-key extraction (e.g. alef_json_array_get_index) — freed via plain free().
            let _ = writeln!(out, "    free({handle_var});");
        } else {
            let _ = writeln!(out, "    {prefix}_{snake_type}_free({handle_var});");
        }
    }

    let result_type_snake = result_type_name.to_snake_case();
    let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
    let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
    let _ = writeln!(out, "}}");
}

/// Emit a byte-buffer test function for FFI methods returning raw bytes via
/// the out-pointer pattern (e.g. `speech`, `file_content`).
///
/// FFI signature shape:
/// ```c
/// int32_t {prefix}_default_client_{fn}(
///     const Client *this_,
///     const Request *req,                /* present when args is non-empty */
///     uint8_t **out_ptr,
///     uintptr_t *out_len,
///     uintptr_t *out_cap);
/// ```
///
/// Emits:
/// - request handle build (same as the standard client pattern)
/// - `uint8_t *out_ptr = NULL; uintptr_t out_len = 0, out_cap = 0;`
/// - call with `&out_ptr, &out_len, &out_cap`
/// - status assertion: `status == 0` on success, `status != 0` on expected error
/// - per-assertion: `not_empty` / `not_null` collapse to `out_len > 0` because
///   the pseudo "audio" / "content" field is the byte buffer itself
/// - `{prefix}_free_bytes(out_ptr, out_len, out_cap)` after assertions
#[allow(clippy::too_many_arguments)]
fn render_bytes_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    function_name: &str,
    _result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    options_type_name: &str,
    result_type_name: &str,
    factory: &str,
    client_owner_type: &str,
    expects_error: bool,
) {
    let prefix_upper = prefix.to_uppercase();
    let mut request_handle_vars: Vec<(String, String)> = Vec::new();
    let mut string_arg_exprs: Vec<String> = Vec::new();

    for arg in args {
        match arg.arg_type.as_str() {
            "json_object" => {
                let request_type_pascal = if !options_type_name.is_empty() {
                    options_type_name.to_string()
                } else if let Some(stripped) = result_type_name.strip_suffix("Response") {
                    format!("{}Request", stripped)
                } else {
                    format!("{result_type_name}Request")
                };
                let request_type_snake = request_type_pascal.to_snake_case();
                let var_name = format!("{request_type_snake}_handle");

                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let json_val = if field.is_empty() || field == "input" {
                    Some(&fixture.input)
                } else {
                    fixture.input.get(field)
                };

                if let Some(val) = json_val {
                    if !val.is_null() {
                        let normalized = super::transform_json_keys_for_language(val, "snake_case");
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let escaped = escape_c(&json_str);
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{request_type_pascal}* {var_name} = \
                             {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                        );
                        if expects_error {
                            // For error fixtures (e.g. invalid enum value rejected by
                            // serde), `_from_json` may legitimately return NULL — that
                            // counts as the expected failure. Mirror Java's pattern of
                            // wrapping setup + call inside `assertThrows(...)` so error
                            // fixtures pass at *any* failure step. The test returns
                            // before attempting to create a client, leaving no
                            // resources to free.
                            let _ = writeln!(out, "    if ({var_name} == NULL) {{ return; }}");
                        } else {
                            let _ = writeln!(out, "    assert({var_name} != NULL && \"failed to build request\");");
                        }
                        request_handle_vars.push((arg.name.clone(), var_name));
                    }
                }
            }
            "string" => {
                // Pass string args (e.g. file_id for file_content) directly as
                // C string literals.
                let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
                let val = fixture.input.get(field);
                let expr = match val {
                    Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_c(s)),
                    Some(serde_json::Value::Null) | None if arg.optional => "NULL".to_string(),
                    Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "NULL".to_string()),
                    None => "NULL".to_string(),
                };
                string_arg_exprs.push(expr);
            }
            _ => {
                // Other arg types are not currently exercised by byte-buffer
                // methods; pass NULL so the call shape compiles.
                string_arg_exprs.push("NULL".to_string());
            }
        }
    }

    let fixture_id = &fixture.id;
    if fixture.needs_mock_server() {
        let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
        let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
        let _ = writeln!(out, "    char base_url[1024];");
        let _ = writeln!(
            out,
            "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
        );
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        let _ = writeln!(
            out,
            "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}{client_owner_type}* client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

    // Out-params for the byte buffer.
    let _ = writeln!(out, "    uint8_t* out_ptr = NULL;");
    let _ = writeln!(out, "    uintptr_t out_len = 0;");
    let _ = writeln!(out, "    uintptr_t out_cap = 0;");

    // Build the comma-separated argument list: handles, then string args.
    let mut method_args: Vec<String> = Vec::new();
    for (_, v) in &request_handle_vars {
        method_args.push(v.clone());
    }
    method_args.extend(string_arg_exprs.iter().cloned());
    let extra_args = if method_args.is_empty() {
        String::new()
    } else {
        format!(", {}", method_args.join(", "))
    };

    let call_fn = format!("{prefix}_default_client_{function_name}");
    let _ = writeln!(
        out,
        "    int32_t status = {call_fn}(client{extra_args}, &out_ptr, &out_len, &out_cap);"
    );

    if expects_error {
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
        let _ = writeln!(out, "    assert(status != 0 && \"expected call to fail\");");
        // free_bytes accepts a NULL ptr (no-op), so it is safe regardless of
        // whether the failed call wrote out_ptr.
        let _ = writeln!(out, "    {prefix}_free_bytes(out_ptr, out_len, out_cap);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(out, "    assert(status == 0 && \"expected call to succeed\");");

    // Render assertions. For byte-buffer methods, the only meaningful per-field
    // assertions are presence/length checks on the buffer itself. Field names
    // (e.g. "audio", "content") are pseudo-fields — collapse them all to
    // `out_len > 0`.
    let mut emitted_len_check = false;
    for assertion in &fixture.assertions {
        match assertion.assertion_type.as_str() {
            "not_error" => {
                // Already covered by the status == 0 assertion above.
            }
            "not_empty" | "not_null" => {
                if !emitted_len_check {
                    let _ = writeln!(out, "    assert(out_len > 0 && \"expected non-empty value\");");
                    emitted_len_check = true;
                }
            }
            _ => {
                // Other assertion shapes (equals, contains, ...) don't apply to
                // raw bytes; emit a comment so the test stays readable but does
                // not emit broken accessor calls.
                let _ = writeln!(
                    out,
                    "    /* skipped: assertion '{}' not meaningful on raw byte buffer */",
                    assertion.assertion_type
                );
            }
        }
    }

    let _ = writeln!(out, "    {prefix}_free_bytes(out_ptr, out_len, out_cap);");
    for (_, var_name) in &request_handle_vars {
        let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
        let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
    }
    let _ = writeln!(out, "    {prefix}_default_client_free(client);");
    let _ = writeln!(out, "}}");
}

struct CStreamingAdapterMetadata {
    owner_type: String,
    item_type: String,
    request_type: String,
    adapter_name: String,
}

fn resolve_c_streaming_adapter(config: &ResolvedCrateConfig, function_name: &str) -> Option<CStreamingAdapterMetadata> {
    config
        .adapters
        .iter()
        .find(|adapter| matches!(adapter.pattern, AdapterPattern::Streaming) && adapter.name == function_name)
        .and_then(|adapter| {
            Some(CStreamingAdapterMetadata {
                owner_type: adapter.owner_type.clone()?,
                item_type: adapter.item_type.clone()?,
                request_type: adapter
                    .request_type
                    .as_deref()
                    .and_then(|path| path.rsplit("::").next())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)?,
                adapter_name: adapter.name.clone(),
            })
        })
}

fn resolve_c_client_owner_type(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    function_name: &str,
) -> Option<String> {
    config
        .adapters
        .iter()
        .find(|adapter| {
            matches!(adapter.pattern, AdapterPattern::Streaming | AdapterPattern::AsyncMethod)
                && adapter.name == function_name
        })
        .and_then(|adapter| adapter.owner_type.clone())
        .or_else(|| {
            type_defs.iter().find_map(|type_def| {
                type_def
                    .methods
                    .iter()
                    .any(|method| method.name == function_name)
                    .then(|| type_def.name.clone())
            })
        })
        .or_else(|| {
            let opaque_types: Vec<&crate::core::ir::TypeDef> =
                type_defs.iter().filter(|type_def| type_def.is_opaque).collect();
            (opaque_types.len() == 1).then(|| opaque_types[0].name.clone())
        })
}

fn render_c_diagnostic_skip(out: &mut String, reason: &str) {
    let escaped = escape_c(reason);
    let _ = writeln!(out, "    fprintf(stderr, \"skipped: {escaped}\\n\");");
    let _ = writeln!(out, "}}");
}

/// Emit a streaming-adapter test function that drives the FFI iterator handle.
///
/// Calls the adapter-derived `{prefix}_{owner}_{method}_start` function to
/// obtain an opaque handle, loops over the corresponding `_next` function until
/// it returns null,
/// and aggregates per-chunk data into local variables (`chunks_count`,
/// `stream_content`, `stream_complete`, `last_choices_json`, ...). Fixture
/// assertions on streaming pseudo-fields (`chunks`, `stream_content`,
/// `stream_complete`, `no_chunks_after_done`, `finish_reason`, `tool_calls`,
/// `tool_calls[0].function.name`, `usage.total_tokens`) are translated to
/// assertions on these locals. Chat-specific field extraction remains best
/// effort and unsupported fields are skipped by `emit_chat_stream_assertion`.
#[allow(clippy::too_many_arguments)]
fn render_streaming_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    result_var: &str,
    args: &[crate::e2e::config::ArgMapping],
    client_factory: &str,
    streaming: &CStreamingAdapterMetadata,
    expects_error: bool,
    api_key_var: Option<&str>,
) {
    let prefix_upper = prefix.to_uppercase();
    let owner_snake = streaming.owner_type.to_snake_case();
    let request_type_pascal = &streaming.request_type;
    let request_type_snake = request_type_pascal.to_snake_case();
    let item_type_pascal = &streaming.item_type;
    let item_type_snake = item_type_pascal.to_snake_case();
    let adapter_name = &streaming.adapter_name;
    let stream_start = format!("{prefix}_{owner_snake}_{adapter_name}_start");
    let stream_next = format!("{prefix}_{owner_snake}_{adapter_name}_next");
    let stream_free = format!("{prefix}_{owner_snake}_{adapter_name}_free");
    let owner_type = &streaming.owner_type;

    let mut request_var: Option<String> = None;
    for arg in args {
        if arg.arg_type == "json_object" {
            let var_name = format!("{request_type_snake}_handle");

            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_val = if field.is_empty() || field == "input" {
                Some(&fixture.input)
            } else {
                fixture.input.get(field)
            };

            if let Some(val) = json_val {
                if !val.is_null() {
                    let normalized = super::transform_json_keys_for_language(val, "snake_case");
                    let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                    let escaped = escape_c(&json_str);
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}{request_type_pascal}* {var_name} = \
                         {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                    );
                    let _ = writeln!(out, "    assert({var_name} != NULL && \"failed to build request\");");
                    request_var = Some(var_name);
                    break;
                }
            }
        }
    }

    let req_handle = request_var.clone().unwrap_or_else(|| "NULL".to_string());
    let req_snake = request_var
        .as_ref()
        .and_then(|v| v.strip_suffix("_handle"))
        .unwrap_or(request_type_snake.as_str())
        .to_string();

    let fixture_id = &fixture.id;
    let has_mock = fixture.needs_mock_server();
    if has_mock && api_key_var.is_some() {
        // `api_key` and `base_url_buf` are already declared by the env-fallback
        // block above (the smoke+mock path). Reuse them — don't redeclare
        // `mock_base`/`base_url`, which would be a C compile error.
        // use_mock was captured before api_key was potentially reassigned to "test-key",
        // so it correctly reflects the original env state.
        let _ = writeln!(out, "    const char* _base_url_arg = use_mock ? base_url_buf : NULL;");
        let _ = writeln!(
            out,
            "    {prefix_upper}{owner_type}* client = {prefix}_{client_factory}(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else if has_mock {
        let _ = writeln!(out, "    const char* mock_base = getenv(\"MOCK_SERVER_URL\");");
        let _ = writeln!(out, "    assert(mock_base != NULL && \"MOCK_SERVER_URL must be set\");");
        let _ = writeln!(out, "    char base_url[1024];");
        let _ = writeln!(
            out,
            "    snprintf(base_url, sizeof(base_url), \"%s/fixtures/{fixture_id}\", mock_base);"
        );
        // Pass UINT64_MAX/UINT32_MAX (≡ -1ULL/-1U) as the FFI's None sentinel for
        // optional numeric primitives — passing literal 0 makes the binding see
        // Some(0), which Rust core treats as `Duration::from_secs(0)` (immediate
        // request deadline) and breaks every HTTP fixture.
        let _ = writeln!(
            out,
            "    {prefix_upper}{owner_type}* client = {prefix}_{client_factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}{owner_type}* client = {prefix}_{client_factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

    // The streaming opaque handle is a Rust type named `{Prefix}{Owner}{Method}StreamHandle`;
    // cbindgen additionally prepends the configured uppercase type-name `prefix` (e.g. `SAMPLELLM`),
    // exactly as it does for ordinary opaque handle types like `{prefix_upper}{owner_type}`.
    let pascal_prefix = prefix.to_pascal_case();
    let pascal_owner = streaming.owner_type.to_pascal_case();
    let pascal_name = streaming.adapter_name.to_pascal_case();
    let _ = writeln!(
        out,
        "    {prefix_upper}{pascal_prefix}{pascal_owner}{pascal_name}StreamHandle* stream_handle = \
         {stream_start}(client, {req_handle});"
    );

    if expects_error {
        let _ = writeln!(
            out,
            "    assert(stream_handle == NULL && \"expected stream-start to fail\");"
        );
        if request_var.is_some() {
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
        }
        let _ = writeln!(out, "    {prefix}_{owner_snake}_free(client);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(
        out,
        "    assert(stream_handle != NULL && \"expected stream-start to succeed\");"
    );

    let _ = writeln!(out, "    size_t chunks_count = 0;");
    let _ = writeln!(out, "    char* stream_content = (char*)malloc(1);");
    let _ = writeln!(out, "    assert(stream_content != NULL);");
    let _ = writeln!(out, "    stream_content[0] = '\\0';");
    let _ = writeln!(out, "    size_t stream_content_len = 0;");
    let _ = writeln!(out, "    int stream_complete = 0;");
    let _ = writeln!(out, "    int no_chunks_after_done = 1;");
    let _ = writeln!(out);

    let _ = writeln!(out, "    while (1) {{");
    let _ = writeln!(
        out,
        "        {prefix_upper}{item_type_pascal}* {result_var} = {stream_next}(stream_handle);"
    );
    let _ = writeln!(out, "        if ({result_var} == NULL) {{");
    let _ = writeln!(
        out,
        "            if ({prefix}_last_error_code() == 0) {{ stream_complete = 1; }}"
    );
    let _ = writeln!(out, "            break;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        chunks_count++;");
    let _ = writeln!(out, "        {prefix}_{item_type_snake}_free({result_var});");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    {stream_free}(stream_handle);");
    let _ = writeln!(out);

    for assertion in &fixture.assertions {
        emit_chat_stream_assertion(out, assertion);
    }

    let _ = writeln!(out, "    free(stream_content);");
    if request_var.is_some() {
        let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
    }
    let _ = writeln!(out, "    {prefix}_{owner_snake}_free(client);");
    let _ = writeln!(
        out,
        "    /* suppress unused */ (void)no_chunks_after_done; \
         (void)stream_complete; (void)chunks_count; (void)stream_content_len;"
    );
    let _ = writeln!(out, "}}");
}

/// Emit a single fixture assertion for a streaming test, mapping fixture
/// pseudo-field references (`chunks`, `stream_content`, `stream_complete`, ...)
/// to the local aggregator variables built by [`render_streaming_test_function`].
fn emit_chat_stream_assertion(out: &mut String, assertion: &Assertion) {
    let field = assertion.field.as_deref().unwrap_or("");

    enum Kind {
        IntCount,
        Bool,
        Unsupported,
    }

    let (expr, kind) = match field {
        "chunks" => ("chunks_count", Kind::IntCount),
        "stream_complete" => ("stream_complete", Kind::Bool),
        "no_chunks_after_done" => ("no_chunks_after_done", Kind::Bool),
        "stream_content" | "finish_reason" | "tool_calls" | "tool_calls[0].function.name" | "usage.total_tokens" => {
            ("", Kind::Unsupported)
        }
        _ => ("", Kind::Unsupported),
    };

    let atype = assertion.assertion_type.as_str();
    if atype == "not_error" || atype == "error" {
        return;
    }

    if matches!(kind, Kind::Unsupported) {
        let _ = writeln!(
            out,
            "    /* skipped: streaming assertion on unsupported field '{field}' */"
        );
        return;
    }

    match (atype, &kind) {
        ("count_min", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} >= {n} && \"expected at least {n} chunks\");");
            }
        }
        ("is_true", Kind::Bool) => {
            let _ = writeln!(out, "    assert({expr} && \"expected {field} to be true\");");
        }
        ("is_false", Kind::Bool) => {
            let _ = writeln!(out, "    assert(!{expr} && \"expected {field} to be false\");");
        }
        ("greater_than_or_equal", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} >= {n} && \"expected {expr} >= {n}\");");
            }
        }
        ("equals", Kind::IntCount) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} == {n} && \"equals assertion failed\");");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "    /* skipped: streaming assertion '{atype}' on field '{field}' not supported */"
            );
        }
    }
}

/// Emit chained FFI accessor calls for a nested resolved field path.
///
/// For a path like `metadata.document.title`, this generates:
/// ```c
/// HTMHtmlMetadata* metadata_handle = htm_conversion_result_metadata(result);
/// assert(metadata_handle != NULL);
/// HTMDocumentMetadata* doc_handle = htm_html_metadata_document(metadata_handle);
/// assert(doc_handle != NULL);
/// char* metadata_title = htm_document_metadata_title(doc_handle);
/// ```
///
/// The type chain is looked up from `fields_c_types` which maps
/// `"{parent_snake_type}.{field}"` -> `"PascalCaseType"`.
#[allow(clippy::too_many_arguments)]
fn emit_nested_accessor(
    out: &mut String,
    prefix: &str,
    resolved: &str,
    local_var: &str,
    result_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
    result_type_name: &str,
    raw_field: &str,
) -> Option<String> {
    let segments: Vec<&str> = resolved.split('.').collect();
    let prefix_upper = prefix.to_uppercase();

    // Walk the path, starting from the root result type.
    let mut current_snake_type = result_type_name.to_snake_case();
    let mut current_handle = result_var.to_string();
    // Set to true when we've traversed a `[]` array element accessor and subsequent
    // fields must be extracted via alef_json_get_string rather than FFI function calls.
    let mut json_extract_mode = false;

    for (i, segment) in segments.iter().enumerate() {
        let is_leaf = i + 1 == segments.len();

        // In JSON extraction mode, the current_handle is a JSON string and all
        // segments name keys to extract via alef_json_get_string (for primitive
        // leaves) or alef_json_get_object (for intermediate object hops).
        if json_extract_mode {
            // Decompose `field` or `field[N]`/`field[]`. Numeric indexing must
            // extract the Nth element so later key lookups don't ambiguously
            // pick the first occurrence (matters for fixtures with multiple
            // array elements like `data[0]`/`data[1]`).
            let (bare_segment, bracket_key): (&str, Option<&str>) = match segment.find('[') {
                Some(pos) => (&segment[..pos], Some(segment[pos + 1..].trim_end_matches(']'))),
                None => (segment, None),
            };
            let seg_snake = bare_segment.to_snake_case();
            if is_leaf {
                let _ = writeln!(
                    out,
                    "    char* {local_var} = alef_json_get_string({current_handle}, \"{seg_snake}\");"
                );
                return None; // JSON key leaf — char*.
            }
            // Intermediate JSON key — must be an object/array value. Use the
            // object extractor so the substring includes braces/brackets and
            // later primitive lookups against it find their keys
            // (alef_json_get_string would return NULL on non-string values).
            let json_var = format!("{seg_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(
                    out,
                    "    char* {json_var} = alef_json_get_object({current_handle}, \"{seg_snake}\");"
                );
                intermediate_handles.push((json_var.clone(), "free".to_string()));
            }
            // If the segment also includes a numeric index `[N]`, drill into
            // the Nth element of the extracted array; otherwise stay on the
            // object/array substring.
            if let Some(key) = bracket_key {
                if let Ok(idx) = key.parse::<usize>() {
                    let elem_var = format!("{seg_snake}_{idx}_json");
                    if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                        let _ = writeln!(
                            out,
                            "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                        );
                        intermediate_handles.push((elem_var.clone(), "free".to_string()));
                    }
                    current_handle = elem_var;
                    continue;
                }
            }
            current_handle = json_var;
            continue;
        }

        // Check for map access: "field[key]" or array element access: "field[]"
        if let Some(bracket_pos) = segment.find('[') {
            let field_name = &segment[..bracket_pos];
            let key = segment[bracket_pos + 1..].trim_end_matches(']');
            let field_snake = field_name.to_snake_case();
            let accessor_fn = format!("{prefix}_{current_snake_type}_{field_snake}");

            // The accessor returns a char* (JSON object/array string).
            let json_var = format!("{field_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                let _ = writeln!(out, "    assert({json_var} != NULL);");
                // Track for freeing — use prefix_free_string since it's a char*.
                intermediate_handles.push((json_var.clone(), "free_string".to_string()));
            }

            // Empty key `[]`: array-element substring access (any element matches).
            // Numeric key `[N]` (e.g. `choices[0]`, `data[1]`): extract the exact
            // Nth top-level element so subsequent key lookups don't ambiguously
            // pick the first occurrence — required for fixtures whose results
            // contain multiple array elements (e.g. `data[0].index`/`data[1].index`).
            if key.is_empty() {
                if !is_leaf {
                    current_handle = json_var;
                    json_extract_mode = true;
                    continue;
                }
                return None;
            }
            if let Ok(idx) = key.parse::<usize>() {
                let elem_var = format!("{field_snake}_{idx}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                    let _ = writeln!(
                        out,
                        "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                    );
                    intermediate_handles.push((elem_var.clone(), "free".to_string()));
                }
                if !is_leaf {
                    current_handle = elem_var;
                    json_extract_mode = true;
                    continue;
                }
                // Trailing `[N]` — caller asserts on the element JSON.
                return None;
            }

            // Named map key access: extract the key value from the JSON object.
            let _ = writeln!(
                out,
                "    char* {local_var} = alef_json_get_string({json_var}, \"{key}\");"
            );
            return None; // Map access leaf — char*.
        }

        let seg_snake = segment.to_snake_case();
        let accessor_fn = format!("{prefix}_{current_snake_type}_{seg_snake}");

        // Skip any assertion that touches a field marked "skip" in fields_c_types.
        if is_skipped_c_field(fields_c_types, &current_snake_type, &seg_snake) {
            return Some("__skip__".to_string()); // Sentinel: no accessor emitted, assertion skipped later.
        }

        if is_leaf {
            // Leaf may be a primitive scalar (uint64_t, double, ...) when
            // configured in `fields_c_types`. Otherwise default to char*.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({current_handle});");
                return Some(t.clone());
            }
            // Opaque struct leaf: when fields_c_types maps "{parent}.{field}" to a
            // PascalCase type name (not a primitive, not "char*", not "skip"), the
            // accessor returns a struct pointer rather than a string. Emit the typed
            // handle declaration and register it for freeing.
            if let Some(opaque_type) = fields_c_types.get(&lookup_key).filter(|t| {
                *t != "char*"
                    && *t != "skip"
                    && !is_primitive_c_type(t)
                    && t.chars().next().is_some_and(|c| c.is_uppercase())
            }) {
                let handle_var = format!("{seg_snake}_handle");
                let opaque_snake = opaque_type.to_snake_case();
                if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}{opaque_type}* {handle_var} = {accessor_fn}({current_handle});"
                    );
                    intermediate_handles.push((handle_var.clone(), opaque_snake.clone()));
                }
                // Treat the handle itself as the local_var for later assertions.
                // Map local_var → handle_var so render_assertion uses the handle name.
                if local_var != handle_var {
                    let _ = writeln!(out, "    {prefix_upper}{opaque_type}* {local_var} = {handle_var};");
                }
                return Some(opaque_snake); // return type name so caller can register opaque handle cleanup
            }
            // Enum leaf: opaque enum pointer that needs `_to_string` conversion.
            if try_emit_enum_accessor(
                out,
                prefix,
                &prefix_upper,
                raw_field,
                &seg_snake,
                &current_snake_type,
                &accessor_fn,
                &current_handle,
                local_var,
                fields_c_types,
                fields_enum,
                intermediate_handles,
            ) {
                return None;
            }
            let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({current_handle});");
        } else {
            // Intermediate field — check if it's a char* (JSON string/array) or an opaque handle.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            let return_type_pascal = match fields_c_types.get(&lookup_key) {
                Some(t) => t.clone(),
                None => {
                    // Fallback: derive PascalCase from the segment name itself.
                    segment.to_pascal_case()
                }
            };

            // Special case: intermediate char* fields (e.g. links, assets) are JSON
            // strings/arrays, not opaque handles. For a `.length` suffix, emit alef_json_array_count.
            if return_type_pascal == "char*" {
                let json_var = format!("{seg_snake}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                    let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                    intermediate_handles.push((json_var.clone(), "free_string".to_string()));
                }
                // If the next (and final) segment is "length", emit the count accessor.
                if i + 2 == segments.len() && segments[i + 1] == "length" {
                    let _ = writeln!(out, "    int {local_var} = alef_json_array_count({json_var});");
                    return Some("int".to_string());
                }
                current_snake_type = seg_snake.clone();
                current_handle = json_var;
                continue;
            }

            let return_snake = return_type_pascal.to_snake_case();
            let handle_var = format!("{seg_snake}_handle");

            // Only emit the handle if we haven't already (multiple fields may
            // share the same intermediate path prefix).
            if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                let _ = writeln!(
                    out,
                    "    {prefix_upper}{return_type_pascal}* {handle_var} = \
                     {accessor_fn}({current_handle});"
                );
                let _ = writeln!(out, "    assert({handle_var} != NULL);");
                intermediate_handles.push((handle_var.clone(), return_snake.clone()));
            }

            current_snake_type = return_snake;
            current_handle = handle_var;
        }
    }
    None
}

/// Build the C argument string for the function call.
/// When `has_options_handle` is true, json_object args are replaced with
/// the `options_handle` pointer (which was constructed via `from_json`).
fn build_args_string_c(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    has_options_handle: bool,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
) -> String {
    if args.is_empty() {
        return json_to_c(input);
    }

    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        // Handle test_backend args: emit the stub and use it.
        if arg.arg_type == "test_backend" {
            if let Some(trait_name) = &arg.trait_name {
                if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                    let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                        .iter()
                        .find(|t| t.name == *trait_name)
                        .map(|t| t.methods.iter().collect())
                        .unwrap_or_default();
                    if let Some(super_trait) = &trait_bridge.super_trait {
                        if let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait) {
                            for method in &super_type.methods {
                                if !methods.iter().any(|m| m.name == method.name) {
                                    methods.push(method);
                                }
                            }
                        }
                    }
                    let emission = super::emit_test_backend("c", trait_bridge, &methods, fixture);
                    parts.push(emission.arg_expr);
                    continue;
                }
            }
            // Unimplemented trait fallback
            parts.push("NULL".to_string());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            // Field missing entirely and optional → pass NULL.
            None if arg.optional => parts.push("NULL".to_string()),
            // Field missing and required → skip (caller error, but don't crash).
            None => {}
            // Explicit null on optional arg → pass NULL.
            Some(v) if v.is_null() && arg.optional => parts.push("NULL".to_string()),
            Some(v) => {
                // For json_object args, use the options_handle pointer
                // instead of the raw JSON string.
                if arg.arg_type == "json_object" && has_options_handle && !v.is_null() {
                    parts.push("options_handle".to_string())
                } else {
                    parts.push(json_to_c(v))
                }
            }
        }
    }

    parts.join(", ")
}

#[allow(clippy::too_many_arguments)]
fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    _field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
    primitive_locals: &HashMap<String, String>,
    opaque_handle_locals: &HashMap<String, String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !_field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => {
            // Use the local variable extracted from the opaque handle.
            accessed_fields
                .iter()
                .find(|(k, _, _)| k == f)
                .map(|(_, local, _)| local.clone())
                .unwrap_or_else(|| result_var.to_string())
        }
        _ => result_var.to_string(),
    };

    // If the field was marked with the "__skip__" sentinel (fields_c_types = "skip"),
    // the accessor was never emitted — skip the assertion silently.
    if primitive_locals.get(&field_expr).is_some_and(|t| t == "__skip__") {
        let _ = writeln!(out, "    // skipped: field '{field_expr}' not available in C FFI");
        return;
    }

    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    // Opaque-handle fields (e.g. `usage` → SAMPLELLMUsage*) cannot be treated
    // as C strings — `strlen` / `strcmp` on a struct pointer is undefined
    // behavior (SIGABRT in practice). `not_empty` / `is_empty` collapse to
    // NULL checks; other string assertions are skipped for these fields.
    let field_is_opaque_handle = opaque_handle_locals.contains_key(&field_expr);
    // Map-access fields are extracted via `alef_json_get_string` and end up
    // as char*. When the assertion expects a numeric or boolean value, we
    // emit a parsed/literal comparison rather than `strcmp`.
    let field_is_map_access = if let Some(f) = &assertion.field {
        accessed_fields.iter().any(|(k, _, m)| k == f && *m)
    } else {
        false
    };

    // Check if the assertion field is optional — used to emit conditional assertions
    // for optional numeric fields (returns 0 when None, so 0 == "not set").
    // Check both the raw field name and its resolved alias.
    let assertion_field_is_optional = assertion
        .field
        .as_deref()
        .map(|f| {
            if f.is_empty() {
                return false;
            }
            if _field_resolver.is_optional(f) {
                return true;
            }
            // Also check the resolved alias (e.g. "robots.crawl_delay" → "crawl_delay").
            let resolved = _field_resolver.resolve(f);
            _field_resolver.is_optional(resolved)
        })
        .unwrap_or(false);

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                if field_is_primitive {
                    let cmp_val = if field_primitive_type.as_deref() == Some("bool") {
                        match expected.as_bool() {
                            Some(true) => "1".to_string(),
                            Some(false) => "0".to_string(),
                            None => c_val,
                        }
                    } else {
                        c_val
                    };
                    // For optional numeric fields, treat 0 as "not set" and allow it.
                    // This mirrors Go's nil-pointer check for optional fields.
                    let is_numeric = field_primitive_type.as_deref().map(|t| t != "bool").unwrap_or(false);
                    if assertion_field_is_optional && is_numeric {
                        let _ = writeln!(
                            out,
                            "    assert(({field_expr} == 0 || {field_expr} == {cmp_val}) && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} == {cmp_val} && \"equals assertion failed\");"
                        );
                    }
                } else if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    assert(str_trim_eq({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_boolean() {
                    let lit = match expected.as_bool() {
                        Some(true) => "\"true\"",
                        _ => "\"false\"",
                    };
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strcmp({field_expr}, {lit}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_number() {
                    if expected.is_f64() {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atof({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atoll({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    }
                } else {
                    let _ = writeln!(
                        out,
                        "    assert(strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(({field_expr} == NULL || strstr({field_expr}, {c_val}) == NULL) && \"expected NOT to contain substring\");"
                );
            }
        }
        "not_empty" => {
            if field_is_opaque_handle {
                // Opaque struct handle: `strlen` on a struct pointer is UB.
                // Weaken to a non-null check — strictly weaker than the
                // original intent but won't false-trigger SIGABRT.
                let _ = writeln!(out, "    assert({field_expr} != NULL && \"expected non-null handle\");");
            } else {
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strlen({field_expr}) > 0 && \"expected non-empty value\");"
                );
            }
        }
        "is_empty" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} == NULL && \"expected null handle\");");
            } else if assertion_field_is_optional || !field_is_primitive {
                // Optional string fields may return NULL — treat NULL as empty.
                let _ = writeln!(
                    out,
                    "    assert(({field_expr} == NULL || strlen({field_expr}) == 0) && \"expected empty value\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) == 0 && \"expected empty value\");"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        int found = 0;");
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        if (strstr({field_expr}, {c_val}) != NULL) {{ found = 1; }}"
                    );
                }
                let _ = writeln!(
                    out,
                    "        assert(found && \"expected to contain at least one of the specified values\");"
                );
                let _ = writeln!(out, "    }}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) > {c_val} && \"expected greater than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} > {c_val} && \"expected greater than\");");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) < {c_val} && \"expected less than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} < {c_val} && \"expected less than\");");
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) >= {c_val} && \"expected greater than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} >= {c_val} && \"expected greater than or equal\");"
                    );
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) <= {c_val} && \"expected less than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} <= {c_val} && \"expected less than or equal\");"
                    );
                }
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(strncmp({field_expr}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    assert(strlen({field_expr}) >= strlen({c_val}) && ");
                let _ = writeln!(
                    out,
                    "           strcmp({field_expr} + strlen({field_expr}) - strlen({c_val}), {c_val}) == 0 && \"expected to end with\");"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert(strlen({field_expr}) >= {n} && \"expected minimum length\");"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(
                        out,
                        "    assert(strlen({field_expr}) <= {n} && \"expected maximum length\");"
                    );
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(out, "        /* count_min: count top-level JSON array elements */");
                    let _ = writeln!(
                        out,
                        "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                    );
                    let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                    let _ = writeln!(
                        out,
                        "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
                    );
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(out, "        /* count_equals: count elements in array */");
                    let _ = writeln!(
                        out,
                        "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                    );
                    let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                    let _ = writeln!(out, "        assert(elem_count == {n} && \"expected {n} elements\");");
                    let _ = writeln!(out, "    }}");
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert({field_expr});");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert(!{field_expr});");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                render_method_result_assertion(
                    out,
                    result_var,
                    ffi_prefix,
                    method_name,
                    assertion.args.as_ref(),
                    assertion.return_type.as_deref(),
                    assertion.check.as_deref().unwrap_or("is_true"),
                    assertion.value.as_ref(),
                );
            } else {
                panic!("C e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        regex_t _re;");
                let _ = writeln!(
                    out,
                    "        assert(regcomp(&_re, {c_val}, REG_EXTENDED) == 0 && \"regex compile failed\");"
                );
                let _ = writeln!(
                    out,
                    "        assert(regexec(&_re, {field_expr}, 0, NULL, 0) == 0 && \"expected value to match regex\");"
                );
                let _ = writeln!(out, "        regfree(&_re);");
                let _ = writeln!(out, "    }}");
            }
        }
        "not_error" => {
            // Already handled — the NULL check above covers this.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("C e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Render a `method_result` assertion in C.
///
/// Dispatches generically using `{ffi_prefix}_{method_name}` for the FFI call.
/// The `return_type` fixture field controls how the return value is handled:
/// - `"string"` — the method returns a heap-allocated `char*`; the generator
///   emits a scoped block that asserts, then calls `free()`.
/// - absent/other — treated as a primitive integer (or pointer-as-bool); the
///   assertion is emitted inline without any heap management.
#[allow(clippy::too_many_arguments)]
fn render_method_result_assertion(
    out: &mut String,
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    return_type: Option<&str>,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    let call_expr = build_c_method_call(result_var, ffi_prefix, method_name, args);

    if return_type == Some("string") {
        // Heap-allocated char* return: emit a scoped block, assert, then free.
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        char* _method_result = {call_expr};");
        if check == "is_error" {
            let _ = writeln!(
                out,
                "        assert(_method_result == NULL && \"expected method to return error\");"
            );
            let _ = writeln!(out, "    }}");
            return;
        }
        let _ = writeln!(
            out,
            "        assert(_method_result != NULL && \"method_result returned NULL\");"
        );
        match check {
            "contains" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                    );
                }
            }
            "equals" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(str_trim_eq(_method_result, {c_val}) == 0 && \"method_result equals assertion failed\");"
                    );
                }
            }
            "is_true" => {
                let _ = writeln!(
                    out,
                    "        assert(_method_result != NULL && strlen(_method_result) > 0 && \"method_result is_true assertion failed\");"
                );
            }
            "count_min" => {
                if let Some(val) = value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                    let _ = writeln!(
                        out,
                        "        assert(_elem_count >= {n} && \"method_result count_min assertion failed\");"
                    );
                }
            }
            other_check => {
                panic!("C e2e generator: unsupported method_result check type for string return: {other_check}");
            }
        }
        let _ = writeln!(out, "        free(_method_result);");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Primitive (integer / pointer-as-bool) return: inline assert, no heap management.
    match check {
        "equals" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} == {c_val} && \"method_result equals assertion failed\");"
                );
            }
        }
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert({call_expr} && \"method_result is_true assertion failed\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert(!{call_expr} && \"method_result is_false assertion failed\");"
            );
        }
        "greater_than_or_equal" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result >= {n} assertion failed\");"
                );
            }
        }
        "count_min" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result count_min assertion failed\");"
                );
            }
        }
        other_check => {
            panic!("C e2e generator: unsupported method_result check type: {other_check}");
        }
    }
}

/// Build a C call expression for a `method_result` assertion.
///
/// Uses generic dispatch: `{ffi_prefix}_{method_name}(result_var, args...)`.
/// Args from the fixture JSON object are emitted as positional C arguments in
/// insertion order, using best-effort type conversion (strings → C string literals,
/// numbers and booleans → verbatim literals).
fn build_c_method_call(
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
                        serde_json::Value::Bool(true) => "1".to_string(),
                        serde_json::Value::Bool(false) => "0".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "NULL".to_string(),
                        other => format!("\"{}\"", escape_c(&other.to_string())),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("{ffi_prefix}_{method_name}({result_var})")
    } else {
        format!("{ffi_prefix}_{method_name}({result_var}, {extra_args})")
    }
}

/// Convert a `serde_json::Value` to a C literal string.
fn json_to_c(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
        serde_json::Value::Bool(true) => "1".to_string(),
        serde_json::Value::Bool(false) => "0".to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "NULL".to_string(),
        other => format!("\"{}\"", escape_c(&other.to_string())),
    }
}

/// Emit a test backend stub.
pub fn emit_test_backend(
    _trait_bridge: &crate::core::config::TraitBridgeConfig,
    _methods: &[&crate::core::ir::MethodDef],
    _fixture: &crate::e2e::fixture::Fixture,
) -> super::TestBackendEmission {
    super::TestBackendEmission::unimplemented("c")
}
