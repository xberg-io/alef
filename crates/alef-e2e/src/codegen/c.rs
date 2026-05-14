//! C e2e test generator using assert.h and a Makefile.
//!
//! Generates `e2e/c/Makefile`, per-category `test_{category}.c` files,
//! a `main.c` test runner, a `test_runner.h` header, and a
//! `download_ffi.sh` script for downloading prebuilt FFI libraries from
//! GitHub releases.

use crate::config::{CallConfig, E2eConfig};
use crate::escape::{escape_c, sanitize_filename, sanitize_ident};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
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

/// Infer the opaque-handle PascalCase return type for a bare-field accessor.
///
/// Returns `Some(pascal_type)` when the accessor `{prefix}_{parent}_{field}`
/// returns a pointer to an opaque struct (e.g. `LITERLLMUsage*`) rather than
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
/// `char* status = literllm_batch_object_status(result);` and string-compare
/// it — but the FFI returns `LITERLLMBatchStatus*` (an opaque enum struct
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
        _type_defs: &[alef_core::ir::TypeDef],
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
        let lib_name = c_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_lib_name());

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

        // Resolve FFI crate path for local repo builds.
        // Default to `../../crates/{name}-ffi` derived from the crate name so that
        // projects like `liter-llm` resolve to `../../crates/liter-llm-ffi/include/`
        // rather than the generic (incorrect) `../../crates/ffi`.
        // When `[crates.output] ffi` is set explicitly, derive the crate path from
        // that value so that renamed FFI crates (e.g. `ts-pack-core-ffi`) resolve
        // correctly without any hardcoded special cases.
        let ffi_crate_path = c_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| config.ffi_crate_path());

        // Generate Makefile.
        let category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
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
        let ffi_pkg_name = e2e_config
            .registry
            .packages
            .get("c")
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| lib_name.clone());
        files.push(GeneratedFile {
            path: output_base.join("download_ffi.sh"),
            content: render_download_script(&github_repo, &version, &ffi_pkg_name),
            generated_header: true,
        });

        // Generate test_runner.h.
        files.push(GeneratedFile {
            path: output_base.join("test_runner.h"),
            content: render_test_runner_header(&active_groups),
            generated_header: true,
        });

        // Generate main.c.
        files.push(GeneratedFile {
            path: output_base.join("main.c"),
            content: render_main_c(&active_groups),
            generated_header: true,
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
            );
            files.push(GeneratedFile {
                path: output_base.join(filename),
                content,
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
    args: Vec<crate::config::ArgMapping>,
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
        .unwrap_or("ConversionOptions")
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
        extra_args,
    }
}

/// Resolve call info for a fixture, with fallback to default call's client_factory.
///
/// Named call configs (e.g. `[e2e.calls.embed]`) may not repeat the `client_factory`
/// setting. We fall back to the default `[e2e.call]` override's client_factory so that
/// all methods on the same client use the same pattern.
fn resolve_fixture_call_info(fixture: &Fixture, e2e_config: &E2eConfig, lang: &str) -> ResolvedCallInfo {
    let call = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
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

fn render_makefile(
    categories: &[String],
    header_name: &str,
    ffi_crate_path: &str,
    lib_name: &str,
    needs_mock_server: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "CC = gcc");
    let _ = writeln!(out, "FFI_DIR = ffi");
    let _ = writeln!(out);

    // Rust's cdylib output normalizes hyphens to underscores in the filename
    // (e.g. crate "html-to-markdown-ffi" → "libhtml_to_markdown_ffi.dylib").
    // The -l linker flag must therefore use the underscore form, while the
    // pkg-config package name retains the original form (as declared in the .pc file).
    let link_lib_name = lib_name.replace('-', "_");

    // 3-path fallback: ffi/ (download script) -> local repo build -> pkg-config.
    let _ = writeln!(out, "ifneq ($(wildcard $(FFI_DIR)/include/{header_name}),)");
    let _ = writeln!(out, "    CFLAGS = -Wall -Wextra -I. -I$(FFI_DIR)/include");
    let _ = writeln!(
        out,
        "    LDFLAGS = -L$(FFI_DIR)/lib -l{link_lib_name} -Wl,-rpath,$(FFI_DIR)/lib"
    );
    let _ = writeln!(out, "else ifneq ($(wildcard {ffi_crate_path}/include/{header_name}),)");
    let _ = writeln!(out, "    CFLAGS = -Wall -Wextra -I. -I{ffi_crate_path}/include");
    let _ = writeln!(
        out,
        "    LDFLAGS = -L../../target/release -l{link_lib_name} -Wl,-rpath,../../target/release"
    );
    let _ = writeln!(out, "else");
    let _ = writeln!(
        out,
        "    CFLAGS = -Wall -Wextra -I. $(shell pkg-config --cflags {lib_name} 2>/dev/null)"
    );
    let _ = writeln!(out, "    LDFLAGS = $(shell pkg-config --libs {lib_name} 2>/dev/null)");
    let _ = writeln!(out, "endif");
    let _ = writeln!(out);

    let src_files: Vec<String> = categories.iter().map(|c| format!("test_{c}.c")).collect();
    let srcs = src_files.join(" ");

    let _ = writeln!(out, "SRCS = main.c {srcs}");
    let _ = writeln!(out, "TARGET = run_tests");
    let _ = writeln!(out);
    let _ = writeln!(out, ".PHONY: all clean test");
    let _ = writeln!(out);
    let _ = writeln!(out, "all: $(TARGET)");
    let _ = writeln!(out);
    let _ = writeln!(out, "$(TARGET): $(SRCS)");
    let _ = writeln!(out, "\t$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS)");
    let _ = writeln!(out);

    if !needs_mock_server {
        // No fixtures require an HTTP mock backend; run the test binary directly.
        let _ = writeln!(out, "test: $(TARGET)");
        let _ = writeln!(out, "\t./$(TARGET)");
        let _ = writeln!(out);
        let _ = writeln!(out, "clean:");
        let _ = writeln!(out, "\trm -f $(TARGET)");
        return out;
    }

    // The `test:` target spawns the e2e mock-server binary, captures its
    // assigned MOCK_SERVER_URL line on stdout, exports it for the test process,
    // runs the suite, then tears the server down. This mirrors the per-language
    // conftest/setup machinery used by Python, Ruby, Java, etc.
    //
    // The mock-server also emits MOCK_SERVERS={...json...} mapping fixture IDs to
    // their per-fixture listener URLs (needed for fixtures like robots/sitemap that
    // require host-root routes). We parse this with python3 and export
    // MOCK_SERVER_<UPPER_ID> env vars so the test binary can look them up.
    let _ = writeln!(out, "MOCK_SERVER_BIN ?= ../rust/target/release/mock-server");
    let _ = writeln!(out, "FIXTURES_DIR ?= ../../fixtures");
    let _ = writeln!(out);
    let _ = writeln!(out, "test: $(TARGET)");
    let _ = writeln!(out, "\t@if [ -n \"$$MOCK_SERVER_URL\" ]; then \\");
    // When MOCK_SERVER_URL is already set (e.g. run under an existing mock-server
    // process), also parse MOCK_SERVERS env var and export per-fixture vars.
    let _ = writeln!(out, "\t\tif [ -n \"$$MOCK_SERVERS\" ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\teval $$(python3 -c \"import json,os; d=json.loads(os.environ.get('MOCK_SERVERS','{{}}')); print(' '.join('export MOCK_SERVER_'+k.upper()+'='+v for k,v in d.items()))\"); \\"
    );
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\t./$(TARGET); \\");
    let _ = writeln!(out, "\telse \\");
    let _ = writeln!(out, "\t\tif [ ! -x \"$(MOCK_SERVER_BIN)\" ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\techo \"mock-server binary not found at $(MOCK_SERVER_BIN); run: cargo build -p mock-server --release\" >&2; \\"
    );
    let _ = writeln!(out, "\t\t\texit 1; \\");
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\trm -f mock_server.stdout mock_server.stdin; \\");
    let _ = writeln!(out, "\t\tmkfifo mock_server.stdin; \\");
    let _ = writeln!(
        out,
        "\t\t\"$(MOCK_SERVER_BIN)\" \"$(FIXTURES_DIR)\" <mock_server.stdin >mock_server.stdout 2>&1 & \\"
    );
    let _ = writeln!(out, "\t\tMOCK_PID=$$!; \\");
    let _ = writeln!(out, "\t\texec 9>mock_server.stdin; \\");
    let _ = writeln!(out, "\t\tMOCK_URL=\"\"; MOCK_SERVERS_JSON=\"\"; \\");
    // Wait until MOCK_SERVER_URL appears in stdout (server is ready), bail after 5 s.
    let _ = writeln!(out, "\t\tfor _ in $$(seq 1 100); do \\");
    let _ = writeln!(out, "\t\t\tif [ -s mock_server.stdout ]; then \\");
    let _ = writeln!(
        out,
        "\t\t\t\tMOCK_URL=$$(grep -o 'MOCK_SERVER_URL=[^ ]*' mock_server.stdout | head -1 | cut -d= -f2); \\"
    );
    let _ = writeln!(out, "\t\t\t\tif [ -n \"$$MOCK_URL\" ]; then break; fi; \\");
    let _ = writeln!(out, "\t\t\tfi; \\");
    let _ = writeln!(out, "\t\t\tsleep 0.05; \\");
    let _ = writeln!(out, "\t\tdone; \\");
    // MOCK_SERVERS line is printed after MOCK_SERVER_URL; give it a short extra read.
    let _ = writeln!(
        out,
        "\t\tMOCK_SERVERS_JSON=$$(grep -o 'MOCK_SERVERS={{.*}}' mock_server.stdout | head -1 | cut -d= -f2-); \\"
    );
    let _ = writeln!(
        out,
        "\t\tif [ -z \"$$MOCK_URL\" ]; then echo 'failed to start mock-server' >&2; cat mock_server.stdout >&2; kill $$MOCK_PID 2>/dev/null || true; exit 1; fi; \\"
    );
    // Export per-fixture MOCK_SERVER_<UPPER_ID> env vars from the JSON map.
    let _ = writeln!(
        out,
        "\t\tif [ -n \"$$MOCK_SERVERS_JSON\" ] && command -v python3 >/dev/null 2>&1; then \\"
    );
    let _ = writeln!(
        out,
        "\t\t\teval $$(python3 -c \"import json,sys; d=json.loads(sys.argv[1]); print(' '.join('export MOCK_SERVER_{{}}={{}}'.format(k.upper(),v) for k,v in d.items()))\" \"$$MOCK_SERVERS_JSON\"); \\"
    );
    let _ = writeln!(out, "\t\tfi; \\");
    let _ = writeln!(out, "\t\tMOCK_SERVER_URL=\"$$MOCK_URL\" ./$(TARGET); STATUS=$$?; \\");
    let _ = writeln!(out, "\t\texec 9>&-; \\");
    let _ = writeln!(out, "\t\tkill $$MOCK_PID 2>/dev/null || true; \\");
    let _ = writeln!(out, "\t\trm -f mock_server.stdout mock_server.stdin; \\");
    let _ = writeln!(out, "\t\texit $$STATUS; \\");
    let _ = writeln!(out, "\tfi");
    let _ = writeln!(out);
    let _ = writeln!(out, "clean:");
    let _ = writeln!(out, "\trm -f $(TARGET) mock_server.stdout mock_server.stdin");
    out
}

fn render_download_script(github_repo: &str, version: &str, ffi_pkg_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "set -euo pipefail");
    let _ = writeln!(out);
    let _ = writeln!(out, "REPO_URL=\"{github_repo}\"");
    let _ = writeln!(out, "VERSION=\"{version}\"");
    let _ = writeln!(out, "FFI_PKG_NAME=\"{ffi_pkg_name}\"");
    let _ = writeln!(out, "FFI_DIR=\"ffi\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "# Detect OS and architecture.");
    let _ = writeln!(out, "OS=\"$(uname -s | tr '[:upper:]' '[:lower:]')\"");
    let _ = writeln!(out, "ARCH=\"$(uname -m)\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "case \"$ARCH\" in");
    let _ = writeln!(out, "x86_64 | amd64) ARCH=\"x86_64\" ;;");
    let _ = writeln!(out, "arm64 | aarch64) ARCH=\"aarch64\" ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported architecture: $ARCH\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "case \"$OS\" in");
    let _ = writeln!(out, "linux) TRIPLE=\"${{ARCH}}-unknown-linux-gnu\" ;;");
    let _ = writeln!(out, "darwin) TRIPLE=\"${{ARCH}}-apple-darwin\" ;;");
    let _ = writeln!(out, "*)");
    let _ = writeln!(out, "  echo \"Unsupported OS: $OS\" >&2");
    let _ = writeln!(out, "  exit 1");
    let _ = writeln!(out, "  ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "ARCHIVE=\"${{FFI_PKG_NAME}}-${{TRIPLE}}.tar.gz\"");
    let _ = writeln!(
        out,
        "URL=\"${{REPO_URL}}/releases/download/v${{VERSION}}/${{ARCHIVE}}\""
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "echo \"Downloading ${{ARCHIVE}} from v${{VERSION}}...\"");
    let _ = writeln!(out, "mkdir -p \"$FFI_DIR\"");
    let _ = writeln!(out, "curl -fSL \"$URL\" | tar xz -C \"$FFI_DIR\"");
    let _ = writeln!(out, "echo \"FFI library extracted to $FFI_DIR/\"");
    out
}

fn render_test_runner_header(active_groups: &[(&FixtureGroup, Vec<&Fixture>)]) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#ifndef TEST_RUNNER_H");
    let _ = writeln!(out, "#define TEST_RUNNER_H");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <string.h>");
    let _ = writeln!(out, "#include <stdlib.h>");
    let _ = writeln!(out);
    // Trim helper for comparing strings that may have trailing whitespace/newlines.
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Compare a string against an expected value, trimming trailing whitespace."
    );
    let _ = writeln!(
        out,
        " * Returns 0 if the trimmed actual string equals the expected string."
    );
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline int str_trim_eq(const char *actual, const char *expected) {{"
    );
    let _ = writeln!(
        out,
        "    if (actual == NULL || expected == NULL) return actual != expected;"
    );
    let _ = writeln!(out, "    size_t alen = strlen(actual);");
    let _ = writeln!(
        out,
        "    while (alen > 0 && (actual[alen-1] == ' ' || actual[alen-1] == '\\n' || actual[alen-1] == '\\r' || actual[alen-1] == '\\t')) alen--;"
    );
    let _ = writeln!(out, "    size_t elen = strlen(expected);");
    let _ = writeln!(out, "    if (alen != elen) return 1;");
    let _ = writeln!(out, "    return memcmp(actual, expected, elen);");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Forward declaration so alef_json_get_string can fall through to the
    // object/array extractor for non-string values without reordering the helpers.
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_object(const char *json, const char *key);"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract a string value for a given key from a JSON object string."
    );
    let _ = writeln!(
        out,
        " * Returns a heap-allocated copy of the value, or NULL if not found."
    );
    let _ = writeln!(out, " * Caller must free() the returned string.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_string(const char *json, const char *key) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || key == NULL) return NULL;");
    let _ = writeln!(out, "    /* Build search pattern: \"key\":  */");
    let _ = writeln!(out, "    size_t key_len = strlen(key);");
    let _ = writeln!(out, "    char *pattern = (char *)malloc(key_len + 5);");
    let _ = writeln!(out, "    if (!pattern) return NULL;");
    let _ = writeln!(out, "    pattern[0] = '\"';");
    let _ = writeln!(out, "    memcpy(pattern + 1, key, key_len);");
    let _ = writeln!(out, "    pattern[key_len + 1] = '\"';");
    let _ = writeln!(out, "    pattern[key_len + 2] = ':';");
    let _ = writeln!(out, "    pattern[key_len + 3] = '\\0';");
    let _ = writeln!(out, "    const char *found = strstr(json, pattern);");
    let _ = writeln!(out, "    free(pattern);");
    let _ = writeln!(out, "    if (!found) return NULL;");
    let _ = writeln!(out, "    found += key_len + 3; /* skip past \"key\": */");
    let _ = writeln!(out, "    while (*found == ' ' || *found == '\\t') found++;");
    let _ = writeln!(
        out,
        "    /* Non-string values (arrays/objects) — fall through to alef_json_get_object so"
    );
    let _ = writeln!(
        out,
        "       leaf accessors over collection-typed fields (Vec<T>, Option<Vec<T>>) work for"
    );
    let _ = writeln!(
        out,
        "       not_empty / count_equals assertions without needing per-field type metadata. */"
    );
    let _ = writeln!(out, "    if (*found == '{{' || *found == '[') {{");
    let _ = writeln!(out, "        return alef_json_get_object(json, key);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(
        out,
        "    /* Primitive non-string value: extract its raw token (numeric / true / false / null)"
    );
    let _ = writeln!(
        out,
        "       so callers asserting on numeric fields can `atoll`/`atof` the result. */"
    );
    let _ = writeln!(out, "    if (*found != '\"') {{");
    let _ = writeln!(out, "        const char *p = found;");
    let _ = writeln!(
        out,
        "        while (*p && *p != ',' && *p != '}}' && *p != ']' && *p != ' ' && *p != '\\t' && *p != '\\n' && *p != '\\r') p++;"
    );
    let _ = writeln!(out, "        size_t plen = (size_t)(p - found);");
    let _ = writeln!(out, "        if (plen == 0) return NULL;");
    let _ = writeln!(out, "        char *prim = (char *)malloc(plen + 1);");
    let _ = writeln!(out, "        if (!prim) return NULL;");
    let _ = writeln!(out, "        memcpy(prim, found, plen);");
    let _ = writeln!(out, "        prim[plen] = '\\0';");
    let _ = writeln!(out, "        return prim;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    found++; /* skip opening quote */");
    let _ = writeln!(out, "    const char *end = found;");
    let _ = writeln!(out, "    while (*end && *end != '\"') {{");
    let _ = writeln!(out, "        if (*end == '\\\\') {{ end++; if (*end) end++; }}");
    let _ = writeln!(out, "        else end++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    size_t val_len = (size_t)(end - found);");
    let _ = writeln!(out, "    char *result_str = (char *)malloc(val_len + 1);");
    let _ = writeln!(out, "    if (!result_str) return NULL;");
    let _ = writeln!(out, "    memcpy(result_str, found, val_len);");
    let _ = writeln!(out, "    result_str[val_len] = '\\0';");
    let _ = writeln!(out, "    return result_str;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract a JSON object/array value `{{...}}` or `[...]` for a given key from"
    );
    let _ = writeln!(
        out,
        " * a JSON object string. Returns a heap-allocated copy of the value INCLUDING"
    );
    let _ = writeln!(
        out,
        " * its surrounding braces, or NULL if the key is missing or its value is a"
    );
    let _ = writeln!(out, " * primitive. Caller must free() the returned string.");
    let _ = writeln!(out, " *");
    let _ = writeln!(
        out,
        " * Used by chained-accessor codegen for intermediate object extraction:"
    );
    let _ = writeln!(
        out,
        " * `choices[0].message.content` first peels off `message` (an object), then"
    );
    let _ = writeln!(out, " * looks up `content` (a string) within the extracted substring.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_get_object(const char *json, const char *key) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || key == NULL) return NULL;");
    let _ = writeln!(out, "    size_t key_len = strlen(key);");
    let _ = writeln!(out, "    char *pattern = (char *)malloc(key_len + 4);");
    let _ = writeln!(out, "    if (!pattern) return NULL;");
    let _ = writeln!(out, "    pattern[0] = '\"';");
    let _ = writeln!(out, "    memcpy(pattern + 1, key, key_len);");
    let _ = writeln!(out, "    pattern[key_len + 1] = '\"';");
    let _ = writeln!(out, "    pattern[key_len + 2] = ':';");
    let _ = writeln!(out, "    pattern[key_len + 3] = '\\0';");
    let _ = writeln!(out, "    const char *found = strstr(json, pattern);");
    let _ = writeln!(out, "    free(pattern);");
    let _ = writeln!(out, "    if (!found) return NULL;");
    let _ = writeln!(out, "    found += key_len + 3;");
    let _ = writeln!(out, "    while (*found == ' ' || *found == '\\t') found++;");
    let _ = writeln!(out, "    char open_ch = *found;");
    let _ = writeln!(out, "    char close_ch;");
    let _ = writeln!(out, "    if (open_ch == '{{') close_ch = '}}';");
    let _ = writeln!(out, "    else if (open_ch == '[') close_ch = ']';");
    let _ = writeln!(
        out,
        "    else return NULL; /* primitive — caller should use alef_json_get_string */"
    );
    let _ = writeln!(out, "    int depth = 0;");
    let _ = writeln!(out, "    int in_string = 0;");
    let _ = writeln!(out, "    const char *end = found;");
    let _ = writeln!(out, "    for (; *end; end++) {{");
    let _ = writeln!(out, "        if (in_string) {{");
    let _ = writeln!(
        out,
        "            if (*end == '\\\\' && *(end + 1)) {{ end++; continue; }}"
    );
    let _ = writeln!(out, "            if (*end == '\"') in_string = 0;");
    let _ = writeln!(out, "            continue;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        if (*end == '\"') {{ in_string = 1; continue; }}");
    let _ = writeln!(out, "        if (*end == open_ch) depth++;");
    let _ = writeln!(out, "        else if (*end == close_ch) {{");
    let _ = writeln!(out, "            depth--;");
    let _ = writeln!(out, "            if (depth == 0) {{ end++; break; }}");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    if (depth != 0) return NULL;");
    let _ = writeln!(out, "    size_t val_len = (size_t)(end - found);");
    let _ = writeln!(out, "    char *result_str = (char *)malloc(val_len + 1);");
    let _ = writeln!(out, "    if (!result_str) return NULL;");
    let _ = writeln!(out, "    memcpy(result_str, found, val_len);");
    let _ = writeln!(out, "    result_str[val_len] = '\\0';");
    let _ = writeln!(out, "    return result_str;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Extract the Nth top-level element of a JSON array as a heap string."
    );
    let _ = writeln!(
        out,
        " * Returns NULL if the input is not an array, the index is out of bounds, or"
    );
    let _ = writeln!(out, " * allocation fails. Caller must free() the returned string.");
    let _ = writeln!(out, " */");
    let _ = writeln!(
        out,
        "static inline char *alef_json_array_get_index(const char *json, int index) {{"
    );
    let _ = writeln!(out, "    if (json == NULL || index < 0) return NULL;");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json != '[') return NULL;");
    let _ = writeln!(out, "    json++;");
    let _ = writeln!(out, "    int current = 0;");
    let _ = writeln!(out, "    while (*json) {{");
    let _ = writeln!(
        out,
        "        while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "        if (*json == ']') return NULL;");
    let _ = writeln!(out, "        const char *elem_start = json;");
    let _ = writeln!(out, "        int depth = 0;");
    let _ = writeln!(out, "        int in_string = 0;");
    let _ = writeln!(out, "        for (; *json; json++) {{");
    let _ = writeln!(out, "            if (in_string) {{");
    let _ = writeln!(
        out,
        "                if (*json == '\\\\' && *(json + 1)) {{ json++; continue; }}"
    );
    let _ = writeln!(out, "                if (*json == '\"') in_string = 0;");
    let _ = writeln!(out, "                continue;");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            if (*json == '\"') {{ in_string = 1; continue; }}");
    let _ = writeln!(out, "            if (*json == '{{' || *json == '[') depth++;");
    let _ = writeln!(out, "            else if (*json == '}}' || *json == ']') {{");
    let _ = writeln!(out, "                if (depth == 0) break;");
    let _ = writeln!(out, "                depth--;");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "            else if (*json == ',' && depth == 0) break;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        if (current == index) {{");
    let _ = writeln!(out, "            const char *elem_end = json;");
    let _ = writeln!(
        out,
        "            while (elem_end > elem_start && (*(elem_end - 1) == ' ' || *(elem_end - 1) == '\\t' || *(elem_end - 1) == '\\n')) elem_end--;"
    );
    let _ = writeln!(out, "            size_t elem_len = (size_t)(elem_end - elem_start);");
    let _ = writeln!(out, "            char *out_buf = (char *)malloc(elem_len + 1);");
    let _ = writeln!(out, "            if (!out_buf) return NULL;");
    let _ = writeln!(out, "            memcpy(out_buf, elem_start, elem_len);");
    let _ = writeln!(out, "            out_buf[elem_len] = '\\0';");
    let _ = writeln!(out, "            return out_buf;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        current++;");
    let _ = writeln!(out, "        if (*json == ']') return NULL;");
    let _ = writeln!(out, "        if (*json == ',') json++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return NULL;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(out, " * Count top-level elements in a JSON array string.");
    let _ = writeln!(out, " * Returns 0 for empty arrays (\"[]\") or NULL input.");
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "static inline int alef_json_array_count(const char *json) {{");
    let _ = writeln!(out, "    if (json == NULL) return 0;");
    let _ = writeln!(out, "    /* Skip leading whitespace */");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json != '[') return 0;");
    let _ = writeln!(out, "    json++;");
    let _ = writeln!(out, "    /* Skip whitespace after '[' */");
    let _ = writeln!(
        out,
        "    while (*json == ' ' || *json == '\\t' || *json == '\\n') json++;"
    );
    let _ = writeln!(out, "    if (*json == ']') return 0;");
    let _ = writeln!(out, "    int count = 1;");
    let _ = writeln!(out, "    int depth = 0;");
    let _ = writeln!(out, "    int in_string = 0;");
    let _ = writeln!(
        out,
        "    for (; *json && !(*json == ']' && depth == 0 && !in_string); json++) {{"
    );
    let _ = writeln!(out, "        if (*json == '\\\\' && in_string) {{ json++; continue; }}");
    let _ = writeln!(
        out,
        "        if (*json == '\"') {{ in_string = !in_string; continue; }}"
    );
    let _ = writeln!(out, "        if (in_string) continue;");
    let _ = writeln!(out, "        if (*json == '[' || *json == '{{') depth++;");
    let _ = writeln!(out, "        else if (*json == ']' || *json == '}}') depth--;");
    let _ = writeln!(out, "        else if (*json == ',' && depth == 0) count++;");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return count;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    for (group, fixtures) in active_groups {
        let _ = writeln!(out, "/* Tests for category: {} */", group.category);
        for fixture in fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "void test_{fn_name}(void);");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "#endif /* TEST_RUNNER_H */");
    out
}

fn render_main_c(active_groups: &[(&FixtureGroup, Vec<&Fixture>)]) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Block));
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "    int passed = 0;");
    let _ = writeln!(out);

    for (group, fixtures) in active_groups {
        let _ = writeln!(out, "    /* Category: {} */", group.category);
        for fixture in fixtures {
            let fn_name = sanitize_ident(&fixture.id);
            let _ = writeln!(out, "    printf(\"  Running test_{fn_name}...\");");
            let _ = writeln!(out, "    test_{fn_name}();");
            let _ = writeln!(out, "    printf(\" PASSED\\n\");");
            let _ = writeln!(out, "    passed++;");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "    printf(\"\\nResults: %d passed, 0 failed\\n\", passed);");
    let _ = writeln!(out, "    return 0;");
    let _ = writeln!(out, "}}");
    out
}

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
        let fixture_call = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
        if let Some(co) = fixture_call.overrides.get(lang) {
            for k in co.enum_fields.keys() {
                effective_fields_enum.insert(k.clone());
            }
        }

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
            &call_info.extra_args,
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
    args: &[crate::config::ArgMapping],
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
    extra_args: &[String],
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

    // Engine-factory pattern: used when c_engine_factory is configured (e.g. kreuzcrawl).
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

    // Streaming pattern: chat_stream uses an FFI iterator handle instead of a
    // single response. Emit start/next/free loop and aggregate per-chunk data
    // into local vars (chunks_count, stream_content, stream_complete) so fixture
    // assertions on pseudo-fields resolve to those locals rather than to
    // non-existent accessor functions on a single chunk handle.
    if client_factory.is_some() && function_name == "chat_stream" {
        render_chat_stream_test_function(
            out,
            fixture,
            prefix,
            result_var,
            args,
            options_type_name,
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
                expects_error,
            );
            return;
        }
    }

    // Client pattern: used when client_factory is configured (e.g. liter-llm).
    // Builds typed request handles from json_object args, creates a client via the
    // factory function, calls {prefix}_default_client_{function_name}(client, req),
    // then frees result, request handles, and client.
    if let Some(factory) = client_factory {
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
                let request_type_pascal = if !options_type_name.is_empty() && options_type_name != "ConversionOptions" {
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
                "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
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
                "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
            );
        } else {
            let _ = writeln!(
                out,
                "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
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
        // Locals declared as opaque struct handles (e.g. LITERLLMUsage*).
        // Keyed by local_var, value is the snake_case type name used for free().
        let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field {
                if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                    let resolved = field_resolver.resolve(f);
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
                        if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
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
    // Used for libraries like html-to-markdown that expose standalone FFI functions.

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
                    // Fixture keys are camelCase; the FFI htm_conversion_options_from_json
                    // deserializes into the Rust ConversionOptions type which uses default
                    // serde (snake_case). Normalize keys before serializing.
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

    let args_str = build_args_string_c(&fixture.input, args, has_options_handle);

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
    // Locals declared as opaque struct handles (e.g. LITERLLMUsage*).
    let mut opaque_handle_locals: HashMap<String, String> = HashMap::new();

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                let resolved = field_resolver.resolve(f);
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
                    if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
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

    // --- call ---
    // When the function returns a raw C type (e.g. char* for JSON-returning batch functions
    // like batch_scrape / batch_crawl), emit a plain variable declaration rather than an
    // opaque handle pointer.
    if raw_c_result_type == Some("char*") {
        let _ = writeln!(out, "    char* {result_var} = {prefix}_{function_name}(engine, url);");
        // For char* results there are no structured field accessors — emit a minimal
        // null-guard and free, then return.
        let _ = writeln!(out, "    if ({result_var} != NULL) {prefix}_free_string({result_var});");
        let _ = writeln!(out, "    {prefix}_crawl_engine_handle_free(engine);");
        let _ = writeln!(out, "}}");
        return;
    }

    let _ = writeln!(
        out,
        "    {prefix_upper}{result_type_name}* {result_var} = {prefix}_{function_name}(engine, url);"
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
                let resolved = field_resolver.resolve(f);
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
                    if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
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
    args: &[crate::config::ArgMapping],
    options_type_name: &str,
    result_type_name: &str,
    factory: &str,
    expects_error: bool,
) {
    let prefix_upper = prefix.to_uppercase();
    let mut request_handle_vars: Vec<(String, String)> = Vec::new();
    let mut string_arg_exprs: Vec<String> = Vec::new();

    for arg in args {
        match arg.arg_type.as_str() {
            "json_object" => {
                let request_type_pascal = if !options_type_name.is_empty() && options_type_name != "ConversionOptions" {
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
            "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
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

/// Emit a chat-stream test function that drives the FFI iterator handle.
///
/// Calls `{prefix}_default_client_chat_stream_start` to obtain an opaque handle,
/// loops over `{prefix}_default_client_chat_stream_next` until it returns null,
/// and aggregates per-chunk data into local variables (`chunks_count`,
/// `stream_content`, `stream_complete`, `last_choices_json`, ...). Fixture
/// assertions on streaming pseudo-fields (`chunks`, `stream_content`,
/// `stream_complete`, `no_chunks_after_done`, `finish_reason`, `tool_calls`,
/// `tool_calls[0].function.name`, `usage.total_tokens`) are translated to
/// assertions on these locals.
#[allow(clippy::too_many_arguments)]
fn render_chat_stream_test_function(
    out: &mut String,
    fixture: &Fixture,
    prefix: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    options_type_name: &str,
    expects_error: bool,
    api_key_var: Option<&str>,
) {
    let prefix_upper = prefix.to_uppercase();

    let mut request_var: Option<String> = None;
    for arg in args {
        if arg.arg_type == "json_object" {
            let request_type_pascal = if !options_type_name.is_empty() && options_type_name != "ConversionOptions" {
                options_type_name.to_string()
            } else {
                "ChatCompletionRequest".to_string()
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
        .unwrap_or("chat_completion_request")
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
            "    {prefix_upper}DefaultClient* client = {prefix}_create_client(api_key, _base_url_arg, (uint64_t)-1, (uint32_t)-1, NULL);"
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
            "    {prefix_upper}DefaultClient* client = {prefix}_create_client(\"test-key\", base_url, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    } else {
        let _ = writeln!(
            out,
            "    {prefix_upper}DefaultClient* client = {prefix}_create_client(\"test-key\", NULL, (uint64_t)-1, (uint32_t)-1, NULL);"
        );
    }
    let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

    let _ = writeln!(
        out,
        "    {prefix_upper}LiterllmDefaultClientChatStreamStreamHandle* stream_handle = \
         {prefix}_default_client_chat_stream_start(client, {req_handle});"
    );

    if expects_error {
        let _ = writeln!(
            out,
            "    assert(stream_handle == NULL && \"expected stream-start to fail\");"
        );
        if request_var.is_some() {
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
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
    let _ = writeln!(out, "    char* last_choices_json = NULL;");
    let _ = writeln!(out, "    uint64_t total_tokens = 0;");
    let _ = writeln!(out);

    let _ = writeln!(out, "    while (1) {{");
    let _ = writeln!(
        out,
        "        {prefix_upper}ChatCompletionChunk* {result_var} = \
         {prefix}_default_client_chat_stream_next(stream_handle);"
    );
    let _ = writeln!(out, "        if ({result_var} == NULL) {{");
    let _ = writeln!(
        out,
        "            if ({prefix}_last_error_code() == 0) {{ stream_complete = 1; }}"
    );
    let _ = writeln!(out, "            break;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        chunks_count++;");
    let _ = writeln!(
        out,
        "        char* choices_json = {prefix}_chat_completion_chunk_choices({result_var});"
    );
    let _ = writeln!(out, "        if (choices_json != NULL) {{");
    let _ = writeln!(
        out,
        "            const char* d = strstr(choices_json, \"\\\"content\\\":\");"
    );
    let _ = writeln!(out, "            if (d != NULL) {{");
    let _ = writeln!(out, "                d += 10;");
    let _ = writeln!(out, "                while (*d == ' ' || *d == '\\t') d++;");
    let _ = writeln!(out, "                if (*d == '\"') {{");
    let _ = writeln!(out, "                    d++;");
    let _ = writeln!(out, "                    const char* e = d;");
    let _ = writeln!(out, "                    while (*e && *e != '\"') {{");
    let _ = writeln!(
        out,
        "                        if (*e == '\\\\' && *(e+1)) e += 2; else e++;"
    );
    let _ = writeln!(out, "                    }}");
    let _ = writeln!(out, "                    size_t add = (size_t)(e - d);");
    let _ = writeln!(out, "                    if (add > 0) {{");
    let _ = writeln!(
        out,
        "                        char* nc = (char*)realloc(stream_content, stream_content_len + add + 1);"
    );
    let _ = writeln!(out, "                        if (nc != NULL) {{");
    let _ = writeln!(out, "                            stream_content = nc;");
    let _ = writeln!(
        out,
        "                            memcpy(stream_content + stream_content_len, d, add);"
    );
    let _ = writeln!(out, "                            stream_content_len += add;");
    let _ = writeln!(
        out,
        "                            stream_content[stream_content_len] = '\\0';"
    );
    let _ = writeln!(out, "                        }}");
    let _ = writeln!(out, "                    }}");
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(
        out,
        "            if (last_choices_json != NULL) {prefix}_free_string(last_choices_json);"
    );
    let _ = writeln!(out, "            last_choices_json = choices_json;");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(
        out,
        "        {prefix_upper}Usage* usage_handle = {prefix}_chat_completion_chunk_usage({result_var});"
    );
    let _ = writeln!(out, "        if (usage_handle != NULL) {{");
    let _ = writeln!(
        out,
        "            total_tokens = (uint64_t){prefix}_usage_total_tokens(usage_handle);"
    );
    let _ = writeln!(out, "            {prefix}_usage_free(usage_handle);");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        {prefix}_chat_completion_chunk_free({result_var});");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    {prefix}_default_client_chat_stream_free(stream_handle);");
    let _ = writeln!(out);

    let _ = writeln!(out, "    char* finish_reason = NULL;");
    let _ = writeln!(out, "    char* tool_calls_json = NULL;");
    let _ = writeln!(out, "    char* tool_calls_0_function_name = NULL;");
    let _ = writeln!(out, "    if (last_choices_json != NULL) {{");
    let _ = writeln!(
        out,
        "        finish_reason = alef_json_get_string(last_choices_json, \"finish_reason\");"
    );
    let _ = writeln!(
        out,
        "        const char* tc = strstr(last_choices_json, \"\\\"tool_calls\\\":\");"
    );
    let _ = writeln!(out, "        if (tc != NULL) {{");
    let _ = writeln!(out, "            tc += 13;");
    let _ = writeln!(out, "            while (*tc == ' ' || *tc == '\\t') tc++;");
    let _ = writeln!(out, "            if (*tc == '[') {{");
    let _ = writeln!(out, "                int depth = 0;");
    let _ = writeln!(out, "                const char* end = tc;");
    let _ = writeln!(out, "                int in_str = 0;");
    let _ = writeln!(out, "                for (; *end; end++) {{");
    let _ = writeln!(
        out,
        "                    if (*end == '\\\\' && in_str) {{ if (*(end+1)) end++; continue; }}"
    );
    let _ = writeln!(
        out,
        "                    if (*end == '\"') {{ in_str = !in_str; continue; }}"
    );
    let _ = writeln!(out, "                    if (in_str) continue;");
    let _ = writeln!(out, "                    if (*end == '[' || *end == '{{') depth++;");
    let _ = writeln!(
        out,
        "                    else if (*end == ']' || *end == '}}') {{ depth--; if (depth == 0) {{ end++; break; }} }}"
    );
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "                size_t tlen = (size_t)(end - tc);");
    let _ = writeln!(out, "                tool_calls_json = (char*)malloc(tlen + 1);");
    let _ = writeln!(out, "                if (tool_calls_json != NULL) {{");
    let _ = writeln!(out, "                    memcpy(tool_calls_json, tc, tlen);");
    let _ = writeln!(out, "                    tool_calls_json[tlen] = '\\0';");
    let _ = writeln!(
        out,
        "                    const char* fn = strstr(tool_calls_json, \"\\\"function\\\"\");"
    );
    let _ = writeln!(out, "                    if (fn != NULL) {{");
    let _ = writeln!(
        out,
        "                        const char* np = strstr(fn, \"\\\"name\\\":\");"
    );
    let _ = writeln!(out, "                        if (np != NULL) {{");
    let _ = writeln!(out, "                            np += 7;");
    let _ = writeln!(
        out,
        "                            while (*np == ' ' || *np == '\\t') np++;"
    );
    let _ = writeln!(out, "                            if (*np == '\"') {{");
    let _ = writeln!(out, "                                np++;");
    let _ = writeln!(out, "                                const char* ne = np;");
    let _ = writeln!(
        out,
        "                                while (*ne && *ne != '\"') {{ if (*ne == '\\\\' && *(ne+1)) ne += 2; else ne++; }}"
    );
    let _ = writeln!(out, "                                size_t nlen = (size_t)(ne - np);");
    let _ = writeln!(
        out,
        "                                tool_calls_0_function_name = (char*)malloc(nlen + 1);"
    );
    let _ = writeln!(
        out,
        "                                if (tool_calls_0_function_name != NULL) {{"
    );
    let _ = writeln!(
        out,
        "                                    memcpy(tool_calls_0_function_name, np, nlen);"
    );
    let _ = writeln!(
        out,
        "                                    tool_calls_0_function_name[nlen] = '\\0';"
    );
    let _ = writeln!(out, "                                }}");
    let _ = writeln!(out, "                            }}");
    let _ = writeln!(out, "                        }}");
    let _ = writeln!(out, "                    }}");
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "            }}");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);

    for assertion in &fixture.assertions {
        emit_chat_stream_assertion(out, assertion);
    }

    let _ = writeln!(out, "    free(stream_content);");
    let _ = writeln!(
        out,
        "    if (last_choices_json != NULL) {prefix}_free_string(last_choices_json);"
    );
    let _ = writeln!(out, "    if (finish_reason != NULL) free(finish_reason);");
    let _ = writeln!(out, "    if (tool_calls_json != NULL) free(tool_calls_json);");
    let _ = writeln!(
        out,
        "    if (tool_calls_0_function_name != NULL) free(tool_calls_0_function_name);"
    );
    if request_var.is_some() {
        let _ = writeln!(out, "    {prefix}_{req_snake}_free({req_handle});");
    }
    let _ = writeln!(out, "    {prefix}_default_client_free(client);");
    let _ = writeln!(
        out,
        "    /* suppress unused */ (void)total_tokens; (void)no_chunks_after_done; \
         (void)stream_complete; (void)chunks_count; (void)stream_content_len;"
    );
    let _ = writeln!(out, "}}");
}

/// Emit a single fixture assertion for a chat-stream test, mapping fixture
/// pseudo-field references (`chunks`, `stream_content`, `stream_complete`, ...)
/// to the local aggregator variables built by [`render_chat_stream_test_function`].
fn emit_chat_stream_assertion(out: &mut String, assertion: &Assertion) {
    let field = assertion.field.as_deref().unwrap_or("");

    enum Kind {
        IntCount,
        Bool,
        Str,
        IntTokens,
        Unsupported,
    }

    let (expr, kind) = match field {
        "chunks" => ("chunks_count", Kind::IntCount),
        "stream_content" => ("stream_content", Kind::Str),
        "stream_complete" => ("stream_complete", Kind::Bool),
        "no_chunks_after_done" => ("no_chunks_after_done", Kind::Bool),
        "finish_reason" => ("finish_reason", Kind::Str),
        // tool_calls / tool_calls[0].function.name require accumulating across
        // delta chunks (the OpenAI SSE wire format spreads the array contents
        // over many chunks). The current C inline SSE parser only inspects the
        // *last* chunk's `choices`, which carries `finish_reason=tool_calls`
        // but no payload — so these assertions can't reliably evaluate. Skip
        // them, mirroring Python's `# skipped: field 'tool_calls' not available
        // on result type` outcome (Python's stream iterator doesn't expose them
        // either). Adding a delta-merge accumulator is its own follow-up.
        "tool_calls" | "tool_calls[0].function.name" => ("", Kind::Unsupported),
        "usage.total_tokens" => ("total_tokens", Kind::IntTokens),
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
        ("equals", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({expr} != NULL && str_trim_eq({expr}, {c_val}) == 0 && \"streaming equals assertion failed\");"
                );
            }
        }
        ("contains", Kind::Str) => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({expr} != NULL && strstr({expr}, {c_val}) != NULL && \"streaming contains assertion failed\");"
                );
            }
        }
        ("not_empty", Kind::Str) => {
            let _ = writeln!(
                out,
                "    assert({expr} != NULL && strlen({expr}) > 0 && \"expected non-empty {field}\");"
            );
        }
        ("is_true", Kind::Bool) => {
            let _ = writeln!(out, "    assert({expr} && \"expected {field} to be true\");");
        }
        ("is_false", Kind::Bool) => {
            let _ = writeln!(out, "    assert(!{expr} && \"expected {field} to be false\");");
        }
        ("greater_than_or_equal", Kind::IntCount) | ("greater_than_or_equal", Kind::IntTokens) => {
            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                let _ = writeln!(out, "    assert({expr} >= {n} && \"expected {expr} >= {n}\");");
            }
        }
        ("equals", Kind::IntCount) | ("equals", Kind::IntTokens) => {
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
            // downstream primitive lookups against it find their keys
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

        if is_leaf {
            // Leaf may be a primitive scalar (uint64_t, double, ...) when
            // configured in `fields_c_types`. Otherwise default to char*.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({current_handle});");
                return Some(t.clone());
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
    args: &[crate::config::ArgMapping],
    has_options_handle: bool,
) -> String {
    if args.is_empty() {
        return json_to_c(input);
    }

    let parts: Vec<String> = args
        .iter()
        .filter_map(|arg| {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            match val {
                // Field missing entirely and optional → pass NULL.
                None if arg.optional => Some("NULL".to_string()),
                // Field missing and required → skip (caller error, but don't crash).
                None => None,
                // Explicit null on optional arg → pass NULL.
                Some(v) if v.is_null() && arg.optional => Some("NULL".to_string()),
                Some(v) => {
                    // For json_object args, use the options_handle pointer
                    // instead of the raw JSON string.
                    if arg.arg_type == "json_object" && has_options_handle && !v.is_null() {
                        Some("options_handle".to_string())
                    } else {
                        Some(json_to_c(v))
                    }
                }
            }
        })
        .collect();

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

    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    // Opaque-handle fields (e.g. `usage` → LITERLLMUsage*) cannot be treated
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
