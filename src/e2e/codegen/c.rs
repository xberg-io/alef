//! C e2e test generator using assert.h and a Makefile.
//!
//! Generates `e2e/c/Makefile`, per-category `test_{category}.c` files,
//! a `main.c` test runner, a `test_runner.h` header, and a
//! `download_ffi.sh` script for downloading prebuilt FFI libraries from
//! GitHub releases.

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::escape::{escape_c, sanitize_filename, sanitize_ident};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Fixture, FixtureGroup};
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

mod assertions;
mod call_patterns;
mod project;
mod runner;
mod streaming;
mod visitor;

use assertions::{build_args_string_c, emit_nested_accessor, render_assertion};
use call_patterns::{render_bytes_test_function, render_engine_factory_test_function};
use project::{render_download_script, render_gitignore, render_makefile};
use runner::{render_main_c, render_test_runner_header};
use streaming::{
    render_c_diagnostic_skip, render_streaming_test_function, resolve_c_client_owner_type, resolve_c_streaming_adapter,
};
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
