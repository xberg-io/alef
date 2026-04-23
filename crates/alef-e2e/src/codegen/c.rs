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
use alef_core::config::AlefConfig;
use anyhow::Result;
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// C e2e code generator.
pub struct CCodegen;

impl E2eCodegen for CCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
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
            .or_else(|| alef_config.ffi.as_ref().and_then(|ffi| ffi.prefix.as_ref()).cloned())
            .unwrap_or_default();
        let header = overrides
            .and_then(|o| o.header.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("{}.h", call.module));

        // Resolve package config.
        let c_pkg = e2e_config.resolve_package("c");
        let lib_name = c_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());

        // Filter active groups (with non-skipped fixtures).
        let active_groups: Vec<(&FixtureGroup, Vec<&Fixture>)> = groups
            .iter()
            .filter_map(|group| {
                let active: Vec<&Fixture> = group
                    .fixtures
                    .iter()
                    .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
                    .filter(|f| f.visitor.is_none())
                    .collect();
                if active.is_empty() { None } else { Some((group, active)) }
            })
            .collect();

        // Resolve FFI crate path for local repo builds.
        // Default to `../../crates/{name}-ffi` derived from the crate name so that
        // projects like `liter-llm` resolve to `../../crates/liter-llm-ffi/include/`
        // rather than the generic (incorrect) `../../crates/ffi`.
        let ffi_crate_path = c_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("../../crates/{}-ffi", alef_config.crate_config.name));

        // Generate Makefile.
        let category_names: Vec<String> = active_groups
            .iter()
            .map(|(g, _)| sanitize_filename(&g.category))
            .collect();
        files.push(GeneratedFile {
            path: output_base.join("Makefile"),
            content: render_makefile(&category_names, &header, &ffi_crate_path, &lib_name),
            generated_header: true,
        });

        // Generate download_ffi.sh for downloading prebuilt FFI from GitHub releases.
        let github_repo = alef_config.github_repo();
        let version = alef_config.resolved_version().unwrap_or_else(|| "0.0.0".to_string());
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
    client_factory: Option<String>,
    args: Vec<crate::config::ArgMapping>,
}

fn resolve_call_info(call: &CallConfig, lang: &str) -> ResolvedCallInfo {
    let overrides = call.overrides.get(lang);
    let function_name = overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call.function.clone());
    let result_type_name = overrides
        .and_then(|o| o.result_type.as_ref())
        .cloned()
        .unwrap_or_else(|| function_name.to_pascal_case());
    let client_factory = overrides.and_then(|o| o.client_factory.as_ref()).cloned();
    ResolvedCallInfo {
        function_name,
        result_type_name,
        client_factory,
        args: call.args.clone(),
    }
}

/// Resolve call info for a fixture, with fallback to default call's client_factory.
///
/// Named call configs (e.g. `[e2e.calls.embed]`) may not repeat the `client_factory`
/// setting. We fall back to the default `[e2e.call]` override's client_factory so that
/// all methods on the same client use the same pattern.
fn resolve_fixture_call_info(fixture: &Fixture, e2e_config: &E2eConfig, lang: &str) -> ResolvedCallInfo {
    let call = e2e_config.resolve_call(fixture.call.as_deref());
    let mut info = resolve_call_info(call, lang);

    // Fallback: if the named call has no client_factory override, inherit from the
    // default call config so all calls use the same client pattern.
    if info.client_factory.is_none() {
        let default_overrides = e2e_config.call.overrides.get(lang);
        if let Some(factory) = default_overrides.and_then(|o| o.client_factory.as_ref()) {
            info.client_factory = Some(factory.clone());
        }
    }

    info
}

fn render_makefile(categories: &[String], header_name: &str, ffi_crate_path: &str, lib_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# This file is auto-generated by alef. DO NOT EDIT.");
    let _ = writeln!(out, "CC = gcc");
    let _ = writeln!(out, "FFI_DIR = ffi");
    let _ = writeln!(out);

    // 3-path fallback: ffi/ (download script) -> local repo build -> pkg-config.
    let _ = writeln!(out, "ifneq ($(wildcard $(FFI_DIR)/include/{header_name}),)");
    let _ = writeln!(out, "    CFLAGS = -Wall -Wextra -I. -I$(FFI_DIR)/include");
    let _ = writeln!(
        out,
        "    LDFLAGS = -L$(FFI_DIR)/lib -l{lib_name} -Wl,-rpath,$(FFI_DIR)/lib"
    );
    let _ = writeln!(out, "else ifneq ($(wildcard {ffi_crate_path}/include/{header_name}),)");
    let _ = writeln!(out, "    CFLAGS = -Wall -Wextra -I. -I{ffi_crate_path}/include");
    let _ = writeln!(
        out,
        "    LDFLAGS = -L../../target/release -l{lib_name} -Wl,-rpath,../../target/release"
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
    let _ = writeln!(out, "test: $(TARGET)");
    let _ = writeln!(out, "\t./$(TARGET)");
    let _ = writeln!(out);
    let _ = writeln!(out, "clean:");
    let _ = writeln!(out, "\trm -f $(TARGET)");
    out
}

fn render_download_script(github_repo: &str, version: &str, ffi_pkg_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#!/usr/bin/env bash");
    let _ = writeln!(out, "# This file is auto-generated by alef. DO NOT EDIT.");
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
    let _ = writeln!(out, "    x86_64|amd64) ARCH=\"x86_64\" ;;");
    let _ = writeln!(out, "    arm64|aarch64) ARCH=\"aarch64\" ;;");
    let _ = writeln!(out, "    *) echo \"Unsupported architecture: $ARCH\" >&2; exit 1 ;;");
    let _ = writeln!(out, "esac");
    let _ = writeln!(out);
    let _ = writeln!(out, "case \"$OS\" in");
    let _ = writeln!(out, "    linux)  TRIPLE=\"${{ARCH}}-unknown-linux-gnu\" ;;");
    let _ = writeln!(out, "    darwin) TRIPLE=\"${{ARCH}}-apple-darwin\" ;;");
    let _ = writeln!(out, "    *)      echo \"Unsupported OS: $OS\" >&2; exit 1 ;;");
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
    let _ = writeln!(out, "/* This file is auto-generated by alef. DO NOT EDIT. */");
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
    let _ = writeln!(out, "    if (*found != '\"') return NULL; /* not a string value */");
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
    let _ = writeln!(out, "/* This file is auto-generated by alef. DO NOT EDIT. */");
    let _ = writeln!(out, "#include <stdio.h>");
    let _ = writeln!(out, "#include \"test_runner.h\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "int main(void) {{");
    let _ = writeln!(out, "    int passed = 0;");
    let _ = writeln!(out, "    int failed = 0;");
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

    let _ = writeln!(
        out,
        "    printf(\"\\nResults: %d passed, %d failed\\n\", passed, failed);"
    );
    let _ = writeln!(out, "    return failed > 0 ? 1 : 0;");
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
    let _ = writeln!(out, "/* This file is auto-generated by alef. DO NOT EDIT. */");
    let _ = writeln!(out, "/* E2e tests for category: {category} */");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <assert.h>");
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
        render_test_function(
            &mut out,
            fixture,
            prefix,
            &call_info.function_name,
            result_var,
            &call_info.args,
            field_resolver,
            &e2e_config.fields_c_types,
            &call_info.result_type_name,
            call_info.client_factory.as_deref(),
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
    result_type_name: &str,
    client_factory: Option<&str>,
) {
    let fn_name = sanitize_ident(&fixture.id);
    let description = &fixture.description;

    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let _ = writeln!(out, "void test_{fn_name}(void) {{");
    let _ = writeln!(out, "    /* {description} */");

    let prefix_upper = prefix.to_uppercase();

    // Client pattern: used when client_factory is configured (e.g. liter-llm).
    // Builds typed request handles from json_object args, creates a client via the
    // factory function, calls {prefix}_default_client_{function_name}(client, req),
    // then frees result, request handles, and client.
    if let Some(factory) = client_factory {
        let mut request_handle_vars: Vec<(String, String)> = Vec::new(); // (arg_name, var_name)

        for arg in args {
            if arg.arg_type == "json_object" {
                // Derive request type from result type: strip "Response", append "Request".
                // E.g. "ChatCompletionResponse" -> "ChatCompletionRequest".
                let request_type_pascal = if let Some(stripped) = result_type_name.strip_suffix("Response") {
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
                        let normalized = super::normalize_json_keys_to_snake_case(val);
                        let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                        let escaped = escape_c(&json_str);
                        let _ = writeln!(
                            out,
                            "    {prefix_upper}{request_type_pascal}* {var_name} = \
                             {prefix}_{request_type_snake}_from_json(\"{escaped}\");"
                        );
                        let _ = writeln!(out, "    assert({var_name} != NULL && \"failed to build request\");");
                        request_handle_vars.push((arg.name.clone(), var_name));
                    }
                }
            }
        }

        let _ = writeln!(
            out,
            "    {prefix_upper}DefaultClient* client = {prefix}_{factory}(\"test-key\", NULL, 0, 0, NULL);"
        );
        let _ = writeln!(out, "    assert(client != NULL && \"failed to create client\");");

        let method_args = if request_handle_vars.is_empty() {
            String::new()
        } else {
            let handles: Vec<&str> = request_handle_vars.iter().map(|(_, v)| v.as_str()).collect();
            format!(", {}", handles.join(", "))
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

        for assertion in &fixture.assertions {
            if let Some(f) = &assertion.field {
                if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                    let resolved = field_resolver.resolve(f);
                    let local_var = f.replace(['.', '['], "_").replace(']', "");
                    let has_map_access = resolved.contains('[');
                    if resolved.contains('.') {
                        emit_nested_accessor(
                            out,
                            prefix,
                            resolved,
                            &local_var,
                            result_var,
                            fields_c_types,
                            &mut intermediate_handles,
                            result_type_name,
                        );
                    } else {
                        let result_type_snake = result_type_name.to_snake_case();
                        let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                        let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                    }
                    accessed_fields.push((f.clone(), local_var, has_map_access));
                }
            }
        }

        for assertion in &fixture.assertions {
            render_assertion(out, assertion, result_var, field_resolver, &accessed_fields);
        }

        for (_f, local_var, from_json) in &accessed_fields {
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
        for (_, var_name) in &request_handle_vars {
            let req_snake = var_name.strip_suffix("_handle").unwrap_or(var_name);
            let _ = writeln!(out, "    {prefix}_{req_snake}_free({var_name});");
        }
        let _ = writeln!(out, "    {prefix}_default_client_free(client);");
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
                    let normalized = super::normalize_json_keys_to_snake_case(val);
                    let json_str = serde_json::to_string(&normalized).unwrap_or_default();
                    let escaped = escape_c(&json_str);
                    let upper = prefix.to_uppercase();
                    let _ = writeln!(
                        out,
                        "    {upper}ConversionOptions* options_handle = {prefix}_conversion_options_from_json(\"{escaped}\");"
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
            let _ = writeln!(out, "    {prefix}_conversion_options_free(options_handle);");
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

    for assertion in &fixture.assertions {
        if let Some(f) = &assertion.field {
            if !f.is_empty() && !accessed_fields.iter().any(|(k, _, _)| k == f) {
                let resolved = field_resolver.resolve(f);
                let local_var = f.replace(['.', '['], "_").replace(']', "");
                let has_map_access = resolved.contains('[');

                if resolved.contains('.') {
                    emit_nested_accessor(
                        out,
                        prefix,
                        resolved,
                        &local_var,
                        result_var,
                        fields_c_types,
                        &mut intermediate_handles,
                        result_type_name,
                    );
                } else {
                    let result_type_snake = result_type_name.to_snake_case();
                    let accessor_fn = format!("{prefix}_{result_type_snake}_{resolved}");
                    let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({result_var});");
                }
                accessed_fields.push((f.clone(), local_var.clone(), has_map_access));
            }
        }
    }

    for assertion in &fixture.assertions {
        render_assertion(out, assertion, result_var, field_resolver, &accessed_fields);
    }

    // Free extracted leaf strings.
    for (_f, local_var, from_json) in &accessed_fields {
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
        let _ = writeln!(out, "    {prefix}_conversion_options_free(options_handle);");
    }
    let result_type_snake = result_type_name.to_snake_case();
    let _ = writeln!(out, "    {prefix}_{result_type_snake}_free({result_var});");
    let _ = writeln!(out, "}}");
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
    intermediate_handles: &mut Vec<(String, String)>,
    result_type_name: &str,
) {
    let segments: Vec<&str> = resolved.split('.').collect();
    let prefix_upper = prefix.to_uppercase();

    // Walk the path, starting from the root result type.
    let mut current_snake_type = result_type_name.to_snake_case();
    let mut current_handle = result_var.to_string();

    for (i, segment) in segments.iter().enumerate() {
        let is_leaf = i + 1 == segments.len();

        // Check for map access: "field[key]"
        if let Some(bracket_pos) = segment.find('[') {
            let field_name = &segment[..bracket_pos];
            let key = segment[bracket_pos + 1..].trim_end_matches(']');
            let field_snake = field_name.to_snake_case();
            let accessor_fn = format!("{prefix}_{current_snake_type}_{field_snake}");

            // The map accessor returns a char* (JSON object string).
            // Use alef_json_get_string to extract the key value.
            let json_var = format!("{field_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                let _ = writeln!(out, "    assert({json_var} != NULL);");
                // Track for freeing — use prefix_free_string since it's a char*.
                intermediate_handles.push((json_var.clone(), "free_string".to_string()));
            }
            // Extract the key from the JSON map.
            let _ = writeln!(
                out,
                "    char* {local_var} = alef_json_get_string({json_var}, \"{key}\");"
            );
            return; // Map access is always the leaf.
        }

        let seg_snake = segment.to_snake_case();
        let accessor_fn = format!("{prefix}_{current_snake_type}_{seg_snake}");

        if is_leaf {
            // Leaf field returns char* — assign to the local variable.
            let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({current_handle});");
        } else {
            // Intermediate field returns an opaque handle.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            let return_type_pascal = match fields_c_types.get(&lookup_key) {
                Some(t) => t.clone(),
                None => {
                    // Fallback: derive PascalCase from the segment name itself.
                    segment.to_pascal_case()
                }
            };
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

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    _field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
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

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                if expected.is_string() {
                    // Use str_trim_eq for string comparisons to handle trailing whitespace.
                    let _ = writeln!(
                        out,
                        "    assert(str_trim_eq({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
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
                    "    assert(strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "    assert(strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(strstr({field_expr}, {c_val}) == NULL && \"expected NOT to contain substring\");"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(
                out,
                "    assert(strlen({field_expr}) > 0 && \"expected non-empty value\");"
            );
        }
        "is_empty" => {
            let _ = writeln!(
                out,
                "    assert(strlen({field_expr}) == 0 && \"expected empty value\");"
            );
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
                let _ = writeln!(out, "    assert({field_expr} > {c_val} && \"expected greater than\");");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                let _ = writeln!(out, "    assert({field_expr} < {c_val} && \"expected less than\");");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} >= {c_val} && \"expected greater than or equal\");"
                );
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} <= {c_val} && \"expected less than or equal\");"
                );
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
                    method_name,
                    assertion.args.as_ref(),
                    assertion.check.as_deref().unwrap_or("is_true"),
                    assertion.value.as_ref(),
                );
            } else {
                panic!("C e2e generator: method_result assertion missing 'method' field");
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
/// Emits a scoped block that calls the appropriate FFI function on `result_var`,
/// performs the requested check, and frees any heap-allocated return values.
fn render_method_result_assertion(
    out: &mut String,
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    let call_expr = build_c_method_call(result_var, method_name, args);

    match method_name {
        // Integer-returning methods: no heap allocation, inline assert.
        "has_error_nodes" | "error_count" | "tree_error_count" => match check {
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
            other_check => {
                panic!("C e2e generator: unsupported method_result check type: {other_check}");
            }
        },

        // root_child_count / named_children_count: allocate NodeInfo, get count, free NodeInfo.
        "root_child_count" | "named_children_count" => {
            let _ = writeln!(out, "    {{");
            let _ = writeln!(
                out,
                "        TS_PACKNodeInfo* _node_info = ts_pack_root_node_info({result_var});"
            );
            let _ = writeln!(
                out,
                "        assert(_node_info != NULL && \"root_node_info returned NULL\");"
            );
            let _ = writeln!(
                out,
                "        size_t _count = ts_pack_node_info_named_child_count(_node_info);"
            );
            let _ = writeln!(out, "        ts_pack_node_info_free(_node_info);");
            match check {
                "equals" => {
                    if let Some(val) = value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "        assert(_count == (size_t){c_val} && \"method_result equals assertion failed\");"
                        );
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = value {
                        let n = val.as_u64().unwrap_or(0);
                        let _ = writeln!(
                            out,
                            "        assert(_count >= {n} && \"method_result >= {n} assertion failed\");"
                        );
                    }
                }
                "is_true" => {
                    let _ = writeln!(
                        out,
                        "        assert(_count > 0 && \"method_result is_true assertion failed\");"
                    );
                }
                other_check => {
                    panic!("C e2e generator: unsupported method_result check type: {other_check}");
                }
            }
            let _ = writeln!(out, "    }}");
        }

        // String-returning methods: heap-allocated char*, must free after assert.
        "tree_to_sexp" => {
            let _ = writeln!(out, "    {{");
            let _ = writeln!(out, "        char* _method_result = {call_expr};");
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
                other_check => {
                    panic!("C e2e generator: unsupported method_result check type: {other_check}");
                }
            }
            let _ = writeln!(out, "        ts_pack_free_string(_method_result);");
            let _ = writeln!(out, "    }}");
        }

        // contains_node_type returns int32_t (boolean).
        "contains_node_type" => match check {
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
            other_check => {
                panic!("C e2e generator: unsupported method_result check type: {other_check}");
            }
        },

        // find_nodes_by_type returns char* JSON array; count_min checks array length.
        "find_nodes_by_type" => {
            let _ = writeln!(out, "    {{");
            let _ = writeln!(out, "        char* _method_result = {call_expr};");
            let _ = writeln!(
                out,
                "        assert(_method_result != NULL && \"method_result returned NULL\");"
            );
            match check {
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
                "is_true" => {
                    let _ = writeln!(
                        out,
                        "        assert(alef_json_array_count(_method_result) > 0 && \"method_result is_true assertion failed\");"
                    );
                }
                "is_false" => {
                    let _ = writeln!(
                        out,
                        "        assert(alef_json_array_count(_method_result) == 0 && \"method_result is_false assertion failed\");"
                    );
                }
                "equals" => {
                    if let Some(val) = value {
                        let n = val.as_u64().unwrap_or(0);
                        let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                        let _ = writeln!(
                            out,
                            "        assert(_elem_count == {n} && \"method_result equals assertion failed\");"
                        );
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = value {
                        let n = val.as_u64().unwrap_or(0);
                        let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                        let _ = writeln!(
                            out,
                            "        assert(_elem_count >= {n} && \"method_result greater_than_or_equal assertion failed\");"
                        );
                    }
                }
                "contains" => {
                    if let Some(val) = value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                        );
                    }
                }
                other_check => {
                    panic!("C e2e generator: unsupported method_result check type: {other_check}");
                }
            }
            let _ = writeln!(out, "        ts_pack_free_string(_method_result);");
            let _ = writeln!(out, "    }}");
        }

        // run_query returns char* JSON array.
        "run_query" => {
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
                "is_true" => {
                    let _ = writeln!(
                        out,
                        "        assert(alef_json_array_count(_method_result) > 0 && \"method_result is_true assertion failed\");"
                    );
                }
                "contains" => {
                    if let Some(val) = value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                        );
                    }
                }
                other_check => {
                    panic!("C e2e generator: unsupported method_result check type: {other_check}");
                }
            }
            let _ = writeln!(out, "        ts_pack_free_string(_method_result);");
            let _ = writeln!(out, "    }}");
        }

        // root_node_type: get NodeInfo, serialize to JSON, extract "kind" field.
        "root_node_type" => {
            let _ = writeln!(out, "    {{");
            let _ = writeln!(
                out,
                "        TS_PACKNodeInfo* _node_info = ts_pack_root_node_info({result_var});"
            );
            let _ = writeln!(
                out,
                "        assert(_node_info != NULL && \"root_node_info returned NULL\");"
            );
            let _ = writeln!(out, "        char* _node_json = ts_pack_node_info_to_json(_node_info);");
            let _ = writeln!(
                out,
                "        assert(_node_json != NULL && \"node_info_to_json returned NULL\");"
            );
            let _ = writeln!(out, "        char* _kind = alef_json_get_string(_node_json, \"kind\");");
            let _ = writeln!(
                out,
                "        assert(_kind != NULL && \"kind field not found in NodeInfo JSON\");"
            );
            match check {
                "equals" => {
                    if let Some(val) = value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "        assert(strcmp(_kind, {c_val}) == 0 && \"method_result equals assertion failed\");"
                        );
                    }
                }
                "contains" => {
                    if let Some(val) = value {
                        let c_val = json_to_c(val);
                        let _ = writeln!(
                            out,
                            "        assert(strstr(_kind, {c_val}) != NULL && \"method_result contains assertion failed\");"
                        );
                    }
                }
                "is_true" => {
                    let _ = writeln!(
                        out,
                        "        assert(_kind != NULL && strlen(_kind) > 0 && \"method_result is_true assertion failed\");"
                    );
                }
                other_check => {
                    panic!("C e2e generator: unsupported method_result check type: {other_check}");
                }
            }
            let _ = writeln!(out, "        free(_kind);");
            let _ = writeln!(out, "        ts_pack_free_string(_node_json);");
            let _ = writeln!(out, "        ts_pack_node_info_free(_node_info);");
            let _ = writeln!(out, "    }}");
        }

        other_method => {
            panic!("C e2e generator: unsupported method_result method: {other_method}");
        }
    }
}

/// Build a C call expression for a `method_result` assertion on a tree-sitter Tree.
///
/// Maps well-known method names to the appropriate C FFI function calls.
/// For integer-returning methods, returns the full expression.
/// For pointer-returning methods, returns the expression (callers allocate a block).
fn build_c_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => {
            // tree-sitter native: get root node child count via NodeInfo named_child_count.
            format!("ts_pack_node_info_named_child_count(ts_pack_root_node_info({result_var}))")
        }
        "has_error_nodes" => format!("ts_pack_tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("ts_pack_tree_error_count({result_var})"),
        "tree_to_sexp" => format!("ts_pack_tree_to_sexp({result_var})"),
        "named_children_count" => {
            format!("ts_pack_node_info_named_child_count(ts_pack_root_node_info({result_var}))")
        }
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("ts_pack_tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("ts_pack_find_nodes_by_type({result_var}, \"{node_type}\")")
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
            // source and source_len are passed as NULL/0 since fixtures don't provide raw bytes here.
            format!("ts_pack_run_query({result_var}, \"{language}\", \"{query_source}\", NULL, 0)")
        }
        // root_node_type is handled separately in render_method_result_assertion.
        "root_node_type" => String::new(),
        other_method => format!("ts_pack_{other_method}({result_var})"),
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
