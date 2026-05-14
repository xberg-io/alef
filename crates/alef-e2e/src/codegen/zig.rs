//! Zig e2e test generator using std.testing.
//!
//! Generates `packages/zig/src/<crate>_test.zig` files from JSON fixtures,
//! driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_zig, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::ResolvedCrateConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::toolchain;
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;
use super::client;
use super::streaming_assertions::{StreamingFieldResolver, is_streaming_virtual_field};

/// Zig e2e code generator.
pub struct ZigE2eCodegen;

impl E2eCodegen for ZigE2eCodegen {
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

        // Resolve call config with overrides.
        let call = &e2e_config.call;
        let overrides = call.overrides.get(lang);
        let _module_path = overrides
            .and_then(|o| o.module.as_ref())
            .cloned()
            .unwrap_or_else(|| call.module.clone());
        let function_name = overrides
            .and_then(|o| o.function.as_ref())
            .cloned()
            .unwrap_or_else(|| call.function.clone());
        let result_var = &call.result_var;

        // Resolve package config.
        let zig_pkg = e2e_config.resolve_package("zig");
        let pkg_path = zig_pkg
            .as_ref()
            .and_then(|p| p.path.as_ref())
            .cloned()
            .unwrap_or_else(|| "../../packages/zig".to_string());
        let pkg_name = zig_pkg
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_else(|| config.name.to_snake_case());

        // Generate build.zig.zon (Zig package manifest).
        files.push(GeneratedFile {
            path: output_base.join("build.zig.zon"),
            content: render_build_zig_zon(&pkg_name, &pkg_path, e2e_config.dep_mode),
            generated_header: false,
        });

        // Get the module name for imports.
        let module_name = config.zig_module_name();

        // Generate build.zig - collect test file names first.
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
            &e2e_config.fields_method_calls,
        );

        // Generate test files per category and collect their names.
        let mut test_filenames: Vec<String> = Vec::new();
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| super::should_include_fixture(f, lang, e2e_config))
                .collect();

            if active.is_empty() {
                continue;
            }

            let filename = format!("{}_test.zig", sanitize_filename(&group.category));
            test_filenames.push(filename.clone());
            let content = render_test_file(
                &group.category,
                &active,
                e2e_config,
                &function_name,
                result_var,
                &e2e_config.call.args,
                &field_resolver,
                &e2e_config.fields_enum,
                &module_name,
            );
            files.push(GeneratedFile {
                path: output_base.join("src").join(filename),
                content,
                generated_header: true,
            });
        }

        // Generate build.zig with collected test files.
        files.insert(
            files
                .iter()
                .position(|f| f.path.file_name().is_some_and(|n| n == "build.zig.zon"))
                .unwrap_or(1),
            GeneratedFile {
                path: output_base.join("build.zig"),
                content: render_build_zig(
                    &test_filenames,
                    &pkg_name,
                    &module_name,
                    &config.ffi_lib_name(),
                    &config.ffi_crate_path(),
                    &e2e_config.test_documents_relative_from(0),
                ),
                generated_header: false,
            },
        );

        Ok(files)
    }

    fn language_name(&self) -> &'static str {
        "zig"
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_build_zig_zon(pkg_name: &str, pkg_path: &str, dep_mode: crate::config::DependencyMode) -> String {
    let dep_block = match dep_mode {
        crate::config::DependencyMode::Registry => {
            // For registry mode, use a dummy hash (in real Zig, hash must be computed).
            format!(
                r#".{{
            .url = "https://registry.example.com/{pkg_name}/v0.1.0.tar.gz",
            .hash = "0000000000000000000000000000000000000000000000000000000000000000",
        }}"#
            )
        }
        crate::config::DependencyMode::Local => {
            format!(r#".{{ .path = "{pkg_path}" }}"#)
        }
    };

    let min_zig = toolchain::MIN_ZIG_VERSION;
    // Zig 0.16+ requires a fingerprint of the form (crc32_ieee(name) << 32) | id.
    let name_bytes: &[u8] = b"e2e_zig";
    let mut crc: u32 = 0xffff_ffff;
    for byte in name_bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    let name_crc: u32 = !crc;
    let mut id: u32 = 0x811c_9dc5;
    for byte in name_bytes {
        id ^= *byte as u32;
        id = id.wrapping_mul(0x0100_0193);
    }
    if id == 0 || id == 0xffff_ffff {
        id = 0x1;
    }
    let fingerprint: u64 = ((name_crc as u64) << 32) | (id as u64);
    format!(
        r#".{{
    .name = .e2e_zig,
    .version = "0.1.0",
    .fingerprint = 0x{fingerprint:016x},
    .minimum_zig_version = "{min_zig}",
    .dependencies = .{{
        .{pkg_name} = {dep_block},
    }},
    .paths = .{{
        "build.zig",
        "build.zig.zon",
        "src",
    }},
}}
"#
    )
}

fn render_build_zig(
    test_filenames: &[String],
    pkg_name: &str,
    module_name: &str,
    ffi_lib_name: &str,
    ffi_crate_path: &str,
    test_documents_path: &str,
) -> String {
    if test_filenames.is_empty() {
        return r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const test_step = b.step("test", "Run tests");
}
"#
        .to_string();
    }

    // The Zig build script wires up three names that all derive from the
    // crate config:
    //   * `ffi_lib_name`     — the dynamic library to link (e.g. `mylib_ffi`).
    //   * `pkg_name`         — the Zig package directory and source file stem
    //                          under `packages/zig/src/<pkg_name>.zig`.
    //   * `module_name`      — the Zig `@import("...")` identifier other test
    //                          files use to import the binding module.
    // Callers pass these in resolved form so this function never embeds a
    // downstream crate's name.
    let mut content = String::from("const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n");
    content.push_str("    const target = b.standardTargetOptions(.{});\n");
    content.push_str("    const optimize = b.standardOptimizeOption(.{});\n");
    content.push_str("    const test_step = b.step(\"test\", \"Run tests\");\n");
    let _ = writeln!(
        content,
        "    const ffi_path = b.option([]const u8, \"ffi_path\", \"Path to directory containing lib{ffi_lib_name}\") orelse \"../../target/debug\";"
    );
    let _ = writeln!(
        content,
        "    const ffi_include = b.option([]const u8, \"ffi_include_path\", \"Path to directory containing FFI header\") orelse \"{ffi_crate_path}/include\";"
    );
    let _ = writeln!(content);
    let _ = writeln!(
        content,
        "    const {module_name}_module = b.addModule(\"{module_name}\", .{{"
    );
    let _ = writeln!(
        content,
        "        .root_source_file = b.path(\"../../packages/zig/src/{pkg_name}.zig\"),"
    );
    content.push_str("        .target = target,\n");
    content.push_str("        .optimize = optimize,\n");
    // Zig 0.16 requires explicit libc linking for any module that transitively
    // references stdlib C bindings (e.g. `c.getenv` via std.posix). The shared
    // binding module pulls in the FFI header, so libc is always required.
    content.push_str("        .link_libc = true,\n");
    content.push_str("    });\n");
    let _ = writeln!(
        content,
        "    {module_name}_module.addLibraryPath(.{{ .cwd_relative = ffi_path }});"
    );
    let _ = writeln!(
        content,
        "    {module_name}_module.addIncludePath(.{{ .cwd_relative = ffi_include }});"
    );
    let _ = writeln!(
        content,
        "    {module_name}_module.linkSystemLibrary(\"{ffi_lib_name}\", .{{}});"
    );
    let _ = writeln!(content);

    for filename in test_filenames {
        // Convert filename like "basic_test.zig" to a test name
        let test_name = filename.trim_end_matches("_test.zig");
        content.push_str(&format!("    const {test_name}_module = b.createModule(.{{\n"));
        content.push_str(&format!("        .root_source_file = b.path(\"src/{filename}\"),\n"));
        content.push_str("        .target = target,\n");
        content.push_str("        .optimize = optimize,\n");
        // Each test module also needs libc linking because it imports the binding
        // module (which references C stdlib symbols) and may directly call helpers
        // like `std.c.getenv` for env-var-driven mock-server URLs.
        content.push_str("        .link_libc = true,\n");
        content.push_str("    });\n");
        content.push_str(&format!(
            "    {test_name}_module.addImport(\"{module_name}\", {module_name}_module);\n"
        ));
        content.push_str(&format!("    const {test_name}_tests = b.addTest(.{{\n"));
        content.push_str(&format!("        .root_module = {test_name}_module,\n"));
        content.push_str("    });\n");
        content.push_str(&format!(
            "    const {test_name}_run = b.addRunArtifact({test_name}_tests);\n"
        ));
        content.push_str(&format!(
            "    {test_name}_run.setCwd(b.path(\"{test_documents_path}\"));\n"
        ));
        content.push_str(&format!("    test_step.dependOn(&{test_name}_run.step);\n\n"));
    }

    content.push_str("}\n");
    content
}

// ---------------------------------------------------------------------------
// HTTP server test rendering — shared-driver integration
// ---------------------------------------------------------------------------

/// Renderer that emits Zig `test "..." { ... }` blocks targeting a mock server
/// via `std.http.Client`. Satisfies [`client::TestClientRenderer`] so the shared
/// [`client::http_call::render_http_test`] driver drives the call sequence.
struct ZigTestClientRenderer;

impl client::TestClientRenderer for ZigTestClientRenderer {
    fn language_name(&self) -> &'static str {
        "zig"
    }

    fn render_test_open(&self, out: &mut String, fn_name: &str, description: &str, skip_reason: Option<&str>) {
        if let Some(reason) = skip_reason {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
            let _ = writeln!(out, "    // skipped: {reason}");
            let _ = writeln!(out, "    return error.SkipZigTest;");
        } else {
            let _ = writeln!(out, "test \"{fn_name}\" {{");
            let _ = writeln!(out, "    // {description}");
        }
    }

    fn render_test_close(&self, out: &mut String) {
        let _ = writeln!(out, "}}");
    }

    fn render_call(&self, out: &mut String, ctx: &client::CallCtx<'_>) {
        let method = ctx.method.to_uppercase();
        let fixture_id = ctx.path.trim_start_matches("/fixtures/");

        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");

        let _ = writeln!(out, "    var url_buf: [512]u8 = undefined;");
        let _ = writeln!(
            out,
            "    const url = try std.fmt.bufPrint(&url_buf, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
        );

        // Headers
        if !ctx.headers.is_empty() {
            let mut header_pairs: Vec<(&String, &String)> = ctx.headers.iter().collect();
            header_pairs.sort_by_key(|(k, _)| k.as_str());
            let _ = writeln!(out, "    const headers = [_]std.http.Header{{");
            for (k, v) in &header_pairs {
                let ek = escape_zig(k);
                let ev = escape_zig(v);
                let _ = writeln!(out, "        .{{ .name = \"{ek}\", .value = \"{ev}\" }},");
            }
            let _ = writeln!(out, "    }};");
        }

        // Body
        if let Some(body) = ctx.body {
            let json_str = serde_json::to_string(body).unwrap_or_default();
            let escaped = escape_zig(&json_str);
            let _ = writeln!(out, "    const body_bytes: []const u8 = \"{escaped}\";");
        }

        let headers_arg = if ctx.headers.is_empty() { "&.{}" } else { "&headers" };
        let has_body = ctx.body.is_some();

        let _ = writeln!(
            out,
            "    var http_client = std.http.Client{{ .allocator = allocator }};"
        );
        let _ = writeln!(out, "    defer http_client.deinit();");
        let _ = writeln!(out, "    var response_body = std.ArrayList(u8).init(allocator);");
        let _ = writeln!(out, "    defer response_body.deinit();");

        let method_zig = match method.as_str() {
            "GET" => ".GET",
            "POST" => ".POST",
            "PUT" => ".PUT",
            "DELETE" => ".DELETE",
            "PATCH" => ".PATCH",
            "HEAD" => ".HEAD",
            "OPTIONS" => ".OPTIONS",
            _ => ".GET",
        };

        let payload_field = if has_body { ", .payload = body_bytes" } else { "" };
        let _ = writeln!(
            out,
            "    const {rv} = try http_client.fetch(.{{ .location = .{{ .url = url }}, .method = {method_zig}, .extra_headers = {headers_arg}{payload_field}, .response_storage = .{{ .dynamic = &response_body }} }});",
            rv = ctx.response_var,
        );
    }

    fn render_assert_status(&self, out: &mut String, response_var: &str, status: u16) {
        let _ = writeln!(
            out,
            "    try testing.expectEqual(@as(u10, {status}), @intFromEnum({response_var}.status));"
        );
    }

    fn render_assert_header(&self, out: &mut String, _response_var: &str, name: &str, expected: &str) {
        let ename = escape_zig(&name.to_lowercase());
        match expected {
            "<<present>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is present (header inspection not yet implemented)"
                );
            }
            "<<absent>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' is absent (header inspection not yet implemented)"
                );
            }
            "<<uuid>>" => {
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' matches UUID pattern (header inspection not yet implemented)"
                );
            }
            exact => {
                let evalue = escape_zig(exact);
                let _ = writeln!(
                    out,
                    "    // assert header '{ename}' == \"{evalue}\" (header inspection not yet implemented)"
                );
            }
        }
    }

    fn render_assert_json_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        let json_str = serde_json::to_string(expected).unwrap_or_default();
        let escaped = escape_zig(&json_str);
        let _ = writeln!(
            out,
            "    try testing.expectEqualStrings(\"{escaped}\", response_body.items);"
        );
    }

    fn render_assert_partial_body(&self, out: &mut String, _response_var: &str, expected: &serde_json::Value) {
        if let Some(obj) = expected.as_object() {
            for (key, val) in obj {
                let ekey = escape_zig(key);
                let eval = escape_zig(&serde_json::to_string(val).unwrap_or_default());
                let _ = writeln!(
                    out,
                    "    // assert body contains field \"{ekey}\" = \"{eval}\" (partial JSON not yet implemented)"
                );
            }
        }
    }

    fn render_assert_validation_errors(
        &self,
        out: &mut String,
        _response_var: &str,
        errors: &[crate::fixture::ValidationErrorExpectation],
    ) {
        for ve in errors {
            let loc = ve.loc.join(".");
            let escaped_loc = escape_zig(&loc);
            let escaped_msg = escape_zig(&ve.msg);
            let _ = writeln!(
                out,
                "    // assert validation error at \"{escaped_loc}\": \"{escaped_msg}\" (not yet implemented)"
            );
        }
    }
}

/// Render a Zig `test "..." { ... }` block for an HTTP server fixture.
///
/// Delegates to the shared [`client::http_call::render_http_test`] driver via
/// [`ZigTestClientRenderer`].
fn render_http_test_case(out: &mut String, fixture: &Fixture) {
    client::http_call::render_http_test(out, &ZigTestClientRenderer, fixture);
}

// ---------------------------------------------------------------------------
// Function-call test rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    e2e_config: &E2eConfig,
    function_name: &str,
    result_var: &str,
    args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    module_name: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    let _ = writeln!(out, "const std = @import(\"std\");");
    let _ = writeln!(out, "const testing = std.testing;");
    let _ = writeln!(out, "const {module_name} = @import(\"{module_name}\");");
    let _ = writeln!(out);

    let _ = writeln!(out, "// E2e tests for category: {category}");
    let _ = writeln!(out);

    for fixture in fixtures {
        if fixture.http.is_some() {
            render_http_test_case(&mut out, fixture);
        } else {
            render_test_fn(
                &mut out,
                fixture,
                e2e_config,
                function_name,
                result_var,
                args,
                field_resolver,
                enum_fields,
                module_name,
            );
        }
        let _ = writeln!(out);
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn render_test_fn(
    out: &mut String,
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    _function_name: &str,
    _result_var: &str,
    _args: &[crate::config::ArgMapping],
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    module_name: &str,
) {
    // Resolve per-fixture call config.
    let call_config = e2e_config.resolve_call_for_fixture(fixture.call.as_deref(), &fixture.input);
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let args = &call_config.args;
    // Client factory: when set, the test instantiates a client object via
    // `module.factory_fn(...)` and calls methods on the instance rather than
    // calling top-level package functions directly.
    // Mirrors the go codegen pattern (go.rs:981-1028 / CallOverride.client_factory).
    let client_factory = call_overrides.and_then(|o| o.client_factory.as_deref()).or_else(|| {
        e2e_config
            .call
            .overrides
            .get(lang)
            .and_then(|o| o.client_factory.as_deref())
    });

    // When `result_is_json_struct = true`, the Zig function returns `[]u8` JSON.
    // The test parses it with `std.json.parseFromSlice(std.json.Value, ...)` and
    // traverses the dynamic JSON object for field assertions.
    //
    // Client-factory methods on opaque handles always return JSON `[]u8` because
    // the zig backend serializes struct results via the FFI's `*_to_json` helper
    // (see alef-backend-zig/src/gen_bindings/opaque_handles.rs). Force the flag
    // on whenever a client_factory is in play so the test path parses the JSON
    // result rather than attempting direct field access on `[]u8`.
    //
    // Exception: when the call returns raw bytes (e.g. speech/file_content use the
    // FFI byte-buffer out-pointer shape and return `[]u8` audio/file bytes rather
    // than a serialised struct). Detect this by checking the call-level flag first
    // and then falling back to any per-language override that declares `result_is_bytes`.
    // The zig and C bindings share the same byte-buffer convention, so a C override
    // of `result_is_bytes = true` is a reliable proxy when no zig override exists.
    let call_result_is_bytes = call_config.result_is_bytes || call_config.overrides.values().any(|o| o.result_is_bytes);
    let result_is_json_struct =
        !call_result_is_bytes && (call_overrides.is_some_and(|o| o.result_is_json_struct) || client_factory.is_some());

    // Whether the bare wrapper return type is `?T` (Optional). The zig backend
    // emits `?[]u8` for nullable JSON results and `?<Primitive>` for nullable
    // primitives, so assertions on the bare result must use null-checks rather
    // than `.len`.
    let result_is_option = call_overrides.is_some_and(|o| o.result_is_option) || call_config.result_is_option;

    let test_name = fixture.id.to_snake_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str, setup_needs_gpa) = build_args_and_setup(&fixture.input, args, &fixture.id, module_name);
    // Append per-call zig extra_args (e.g. `["null"]` for the trailing
    // optional `query` parameter on `list_files` / `list_batches`). Mirrors
    // the same mechanism used by go/python/swift codegen — zig's method
    // signatures require every optional positional argument to be supplied
    // explicitly, so the e2e config carries a per-language extras list.
    let extra_args: Vec<String> = call_overrides.map(|o| o.extra_args.clone()).unwrap_or_default();
    let args_str = if extra_args.is_empty() {
        args_str
    } else if args_str.is_empty() {
        extra_args.join(", ")
    } else {
        format!("{args_str}, {}", extra_args.join(", "))
    };

    // Pre-compute whether any assertion will emit code that references `result` /
    // `allocator`. Used to decide whether to emit the GPA allocator binding.
    let any_happy_emits_code = fixture
        .assertions
        .iter()
        .any(|a| assertion_emits_code(a, field_resolver));
    let any_non_error_emits_code = fixture
        .assertions
        .iter()
        .filter(|a| a.assertion_type != "error")
        .any(|a| assertion_emits_code(a, field_resolver));

    // Pre-compute streaming-virtual path conditions.
    let has_streaming_virtual_assertions = fixture.assertions.iter().any(|a| {
        a.field
            .as_ref()
            .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
    });
    let is_stream_fn = function_name.contains("stream");
    let uses_streaming_virtual_path =
        result_is_json_struct && has_streaming_virtual_assertions && is_stream_fn && client_factory.is_some();
    // Whether the streaming-virtual path also parses JSON (for non-streaming assertions).
    let streaming_path_has_non_streaming = uses_streaming_virtual_path
        && fixture.assertions.iter().any(|a| {
            !a.field
                .as_ref()
                .is_some_and(|f| !f.is_empty() && is_streaming_virtual_field(f))
                && !matches!(a.assertion_type.as_str(), "not_error" | "error")
                && a.field
                    .as_ref()
                    .is_some_and(|f| !f.is_empty() && field_resolver.is_valid_for_result(f))
        });

    let _ = writeln!(out, "test \"{test_name}\" {{");
    let _ = writeln!(out, "    // {description}");

    // Emit GPA allocator only when it will actually be used: setup lines that
    // need GPA allocation (mock_url), or a JSON-struct result path where the test
    // will call `std.json.parseFromSlice`. The binding is not needed for
    // error-only paths or tests with no field assertions.
    // Note: `bytes` arg setup uses c_allocator directly and does NOT require GPA.
    // For the streaming-virtual path, `allocator` is only needed if there are also
    // non-streaming assertions that require JSON parsing via parseFromSlice.
    let needs_gpa = setup_needs_gpa
        || streaming_path_has_non_streaming
        || (!uses_streaming_virtual_path && result_is_json_struct && !expects_error && any_happy_emits_code)
        || (!uses_streaming_virtual_path && result_is_json_struct && expects_error && any_non_error_emits_code);
    if needs_gpa {
        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");
        let _ = writeln!(out);
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    // Client factory: when configured, instantiate a client object via the named
    // constructor function and call the method on the instance.
    // The client is pointed at MOCK_SERVER_URL/fixtures/<id> (mirrors go.rs:981-1028).
    // When not configured, fall back to calling the top-level package function directly.
    let call_prefix = if let Some(factory) = client_factory {
        let fixture_id = &fixture.id;
        let _ = writeln!(
            out,
            "    const _mock_url = try std.fmt.allocPrintSentinel(std.heap.c_allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}}, 0);"
        );
        let _ = writeln!(out, "    defer std.heap.c_allocator.free(_mock_url);");
        let _ = writeln!(
            out,
            "    var _client = try {module_name}.{factory}(\"test-key\", _mock_url, null, null, null);"
        );
        let _ = writeln!(out, "    defer _client.free();");
        "_client".to_string()
    } else {
        module_name.to_string()
    };

    if expects_error {
        // Error-path test: use error union syntax `!T` and try-catch.
        // Async functions execute via tokio::runtime::block_on in the FFI shim,
        // so the call site is synchronous from Zig's perspective.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        } else {
            let _ = writeln!(
                out,
                "    const result = {call_prefix}.{function_name}({args_str}) catch {{"
            );
        }
        let _ = writeln!(out, "        try testing.expect(true); // Error occurred as expected");
        let _ = writeln!(out, "        return;");
        let _ = writeln!(out, "    }};");
        // Whether any non-error assertion will emit code that references `result`.
        // If not, we must explicitly discard `result` to satisfy Zig's
        // strict-unused-locals rule.
        let any_emits_code = fixture
            .assertions
            .iter()
            .filter(|a| a.assertion_type != "error")
            .any(|a| assertion_emits_code(a, field_resolver));
        if result_is_json_struct && any_emits_code {
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let _ = writeln!(
                out,
                "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
            );
            let _ = writeln!(out, "    defer _parsed.deinit();");
            let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_json_assertion(out, assertion, result_var, field_resolver);
                }
            }
        } else if result_is_json_struct {
            let _ = writeln!(out, "    _ = _result_json;");
        } else if any_emits_code {
            let _ = writeln!(out, "    // Perform success assertions if any");
            for assertion in &fixture.assertions {
                if assertion.assertion_type != "error" {
                    render_assertion(
                        out,
                        assertion,
                        result_var,
                        field_resolver,
                        enum_fields,
                        result_is_option,
                    );
                }
            }
        } else {
            let _ = writeln!(out, "    _ = result;");
        }
    } else if fixture.assertions.is_empty() {
        // No assertions: emit a call to verify compilation.
        if result_is_json_struct {
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
        } else {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        }
    } else {
        // Happy path: call and assert. Detect whether any assertion actually
        // emits code that references `result` (some — like `not_error` — emit
        // nothing) so we don't leave an unused local, which Zig 0.16 rejects.
        let any_emits_code = fixture
            .assertions
            .iter()
            .any(|a| assertion_emits_code(a, field_resolver));
        if call_result_is_bytes && client_factory.is_some() {
            // Bytes path: the function returns raw `[]u8` (audio/file bytes), not
            // a JSON struct. Call, defer-free, then check len for not_empty/is_empty.
            let _ = writeln!(
                out,
                "    const _result_json = try {call_prefix}.{function_name}({args_str});"
            );
            let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
            let has_bytes_assertions = fixture
                .assertions
                .iter()
                .any(|a| matches!(a.assertion_type.as_str(), "not_empty" | "is_empty"));
            if has_bytes_assertions {
                for assertion in &fixture.assertions {
                    match assertion.assertion_type.as_str() {
                        "not_empty" => {
                            let _ = writeln!(out, "    try testing.expect(_result_json.len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, 0), _result_json.len);");
                        }
                        "not_error" | "error" => {}
                        _ => {
                            let atype = &assertion.assertion_type;
                            let _ = writeln!(
                                out,
                                "    // bytes result: assertion '{atype}' not implemented for zig bytes"
                            );
                        }
                    }
                }
            }
        } else if result_is_json_struct {
            // When streaming-virtual field assertions are present (pre-computed above),
            // emit raw FFI code to collect all chunks instead of calling
            // `chat_stream` (which only returns the last chunk's JSON).
            if uses_streaming_virtual_path {
                // Streaming-virtual path: inline FFI collect.
                // Build a sentinel-terminated request string.
                let _ = writeln!(
                    out,
                    "    const _req_z = try std.heap.c_allocator.dupeZ(u8, {args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_req_z);");
                let _ = writeln!(
                    out,
                    "    const _req_handle = {module_name}.c.literllm_chat_completion_request_from_json(_req_z.ptr);"
                );
                let _ = writeln!(
                    out,
                    "    defer {module_name}.c.literllm_chat_completion_request_free(_req_handle);"
                );
                let _ = writeln!(
                    out,
                    "    const _stream_handle = {module_name}.c.literllm_default_client_chat_stream_start(@as(*{module_name}.c.LITERLLMDefaultClient, @ptrCast(_client._handle)), _req_handle);"
                );
                let _ = writeln!(out, "    if (_stream_handle == null) return error.StreamStartFailed;");
                let _ = writeln!(
                    out,
                    "    defer {module_name}.c.literllm_default_client_chat_stream_free(_stream_handle);"
                );
                // Emit the collect snippet (already has 4-space indentation baked in).
                if let Some(snip) = StreamingFieldResolver::collect_snippet("zig", "_stream_handle", "chunks") {
                    out.push_str("    ");
                    out.push_str(&snip);
                    out.push('\n');
                }
                // For non-streaming assertions (e.g. usage), we also need _result_json.
                // Re-serialize the last chunk in `chunks` to get the JSON.
                if streaming_path_has_non_streaming {
                    let _ = writeln!(
                        out,
                        "    const _result_json = if (chunks.items.len > 0) chunks.items[chunks.items.len - 1] else &[_]u8{{}};"
                    );
                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                }
                for assertion in &fixture.assertions {
                    render_json_assertion(out, assertion, result_var, field_resolver);
                }
            } else {
                // JSON struct path: parse result JSON and access fields dynamically.
                let _ = writeln!(
                    out,
                    "    const _result_json = try {call_prefix}.{function_name}({args_str});"
                );
                let _ = writeln!(out, "    defer std.heap.c_allocator.free(_result_json);");
                if any_emits_code {
                    let _ = writeln!(
                        out,
                        "    var _parsed = try std.json.parseFromSlice(std.json.Value, allocator, _result_json, .{{}});"
                    );
                    let _ = writeln!(out, "    defer _parsed.deinit();");
                    let _ = writeln!(out, "    const {result_var} = &_parsed.value;");
                    for assertion in &fixture.assertions {
                        render_json_assertion(out, assertion, result_var, field_resolver);
                    }
                }
            }
        } else if any_emits_code {
            let _ = writeln!(
                out,
                "    const {result_var} = try {call_prefix}.{function_name}({args_str});"
            );
            for assertion in &fixture.assertions {
                render_assertion(
                    out,
                    assertion,
                    result_var,
                    field_resolver,
                    enum_fields,
                    result_is_option,
                );
            }
        } else {
            let _ = writeln!(out, "    _ = try {call_prefix}.{function_name}({args_str});");
        }
    }

    let _ = writeln!(out, "}}");
}

// ---------------------------------------------------------------------------
// JSON-struct assertion rendering (for result_is_json_struct = true)
// ---------------------------------------------------------------------------

/// Convert a dot-separated field path into a chain of `std.json.Value` lookups.
///
/// Each segment uses `.object.get("key").?` to traverse the JSON object tree.
/// The final segment stops before the leaf-type accessor so callers can append
/// the appropriate accessor (`.string`, `.integer`, `.array.items`, etc.).
///
/// Returns `(base_expr, last_key)` where `base_expr` already includes all
/// intermediate `.object.get("…").?` dereferences up to (but not including)
/// the leaf, and `last_key` is the last path segment.
/// Variant names of `FormatMetadata` (snake_case, from `#[serde(rename_all = "snake_case")]`).
/// These appear as typed accessors in fixture paths (e.g. `format.excel.sheet_count`)
/// but are NOT JSON keys — `FormatMetadata` is internally tagged so variant fields are
/// flattened directly into the `format` object alongside the `format_type` discriminant.
const FORMAT_METADATA_VARIANTS: &[&str] = &[
    "pdf",
    "docx",
    "excel",
    "email",
    "pptx",
    "archive",
    "image",
    "xml",
    "text",
    "html",
    "ocr",
    "csv",
    "bibtex",
    "citation",
    "fiction_book",
    "dbf",
    "jats",
    "epub",
    "pst",
    "code",
];

fn json_path_expr(result_var: &str, field_path: &str) -> String {
    let segments: Vec<&str> = field_path.split('.').collect();
    let mut expr = result_var.to_string();
    let mut prev_seg: Option<&str> = None;
    for seg in &segments {
        // Skip variant-name accessor segments that follow a `format` key.
        // FormatMetadata is an internally-tagged enum (`#[serde(tag = "format_type")]`),
        // so variant fields are flattened directly into the format object — there is no
        // intermediate JSON key for the variant name.
        if prev_seg == Some("format") && FORMAT_METADATA_VARIANTS.contains(seg) {
            prev_seg = Some(seg);
            continue;
        }
        // Handle array accessor notation:
        //   "links[]"     → access the array, then first element.
        //   "results[0]"  → access the array, then specific index N.
        if let Some(key) = seg.strip_suffix("[]") {
            expr = format!("{expr}.object.get(\"{key}\").?.array.items[0]");
        } else if let Some(bracket_pos) = seg.find('[') {
            if let Some(end_pos) = seg.find(']') {
                if end_pos > bracket_pos + 1 && end_pos == seg.len() - 1 {
                    let key = &seg[..bracket_pos];
                    let idx = &seg[bracket_pos + 1..end_pos];
                    if idx.chars().all(|c| c.is_ascii_digit()) {
                        expr = format!("{expr}.object.get(\"{key}\").?.array.items[{idx}]");
                        prev_seg = Some(seg);
                        continue;
                    }
                }
            }
            expr = format!("{expr}.object.get(\"{seg}\").?");
        } else {
            expr = format!("{expr}.object.get(\"{seg}\").?");
        }
        prev_seg = Some(seg);
    }
    expr
}

/// Render a single assertion for a JSON-struct result (result_is_json_struct = true).
///
/// The `result_var` variable is `*std.json.Value` (pointer to the parsed root object).
/// Field paths are traversed via `.object.get("key").?` chains.
fn render_json_assertion(out: &mut String, assertion: &Assertion, result_var: &str, field_resolver: &FieldResolver) {
    // Intercept streaming-virtual fields before the result-type validity check.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming_virtual_field(f) {
            if let Some(expr) = StreamingFieldResolver::accessor(f, "zig", "chunks") {
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(out, "    try testing.expect({expr}.len >= {n});");
                        }
                    }
                    "count_equals" => {
                        if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                            let _ = writeln!(out, "    try testing.expectEqual(@as(usize, {n}), {expr}.len);");
                        }
                    }
                    "equals" => {
                        if let Some(serde_json::Value::String(s)) = &assertion.value {
                            let escaped = escape_zig(s);
                            let _ = writeln!(out, "    try testing.expectEqualStrings(\"{escaped}\", {expr});");
                        } else if let Some(v) = &assertion.value {
                            let zig_val = json_to_zig(v);
                            let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {expr});");
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(out, "    try testing.expect({expr}.len > 0);");
                    }
                    "is_true" => {
                        let _ = writeln!(out, "    try testing.expect({expr});");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "    try testing.expect(!{expr});");
                    }
                    _ => {
                        let atype = &assertion.assertion_type;
                        let _ = writeln!(
                            out,
                            "    // streaming virtual field '{f}' assertion '{atype}' not implemented for zig"
                        );
                    }
                }
            }
            return;
        }
    }

    // Synthetic `embeddings` field on a JSON-array result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` → JSON `[[...],[...]]`). The field name is a
    // convention from the fixture schema — the JSON value IS the embeddings
    // array. Apply the assertion against `result.array.items` directly. The
    // synthetic path is only used when no explicit result_fields configure
    // `embeddings` as a real struct field.
    if let Some(f) = &assertion.field {
        if f == "embeddings" && !field_resolver.has_explicit_field("embeddings") {
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len >= {n});");
                    }
                    return;
                }
                "count_equals" => {
                    if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                        let _ = writeln!(
                            out,
                            "    try testing.expectEqual(@as(usize, {n}), {result_var}.array.items.len);"
                        );
                    }
                    return;
                }
                "not_empty" => {
                    let _ = writeln!(out, "    try testing.expect({result_var}.array.items.len > 0);");
                    return;
                }
                "is_empty" => {
                    let _ = writeln!(
                        out,
                        "    try testing.expectEqual(@as(usize, 0), {result_var}.array.items.len);"
                    );
                    return;
                }
                _ => {}
            }
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
            return;
        }
    }
    // error/not_error are handled at the call level, not assertion level.
    if matches!(assertion.assertion_type.as_str(), "not_error" | "error") {
        return;
    }

    let raw_field_path = assertion.field.as_deref().unwrap_or("").trim();
    let field_path = if raw_field_path.is_empty() {
        raw_field_path.to_string()
    } else {
        field_resolver.resolve(raw_field_path).to_string()
    };
    let field_path = field_path.trim();

    // "{array_field}.length" → strip suffix; use .array.items.len in the template.
    let (field_path_for_expr, is_length_access) = if let Some(parent) = field_path.strip_suffix(".length") {
        (parent, true)
    } else {
        (field_path, false)
    };

    let field_expr = if field_path_for_expr.is_empty() {
        result_var.to_string()
    } else {
        json_path_expr(result_var, field_path_for_expr)
    };

    // Compute context variables for the template.
    let zig_val = match &assertion.value {
        Some(serde_json::Value::String(s)) => format!("\"{}\"", escape_zig(s)),
        _ => String::new(),
    };
    let is_string_val = matches!(&assertion.value, Some(serde_json::Value::String(_)));
    let is_bool_val = matches!(&assertion.value, Some(serde_json::Value::Bool(_)));
    let bool_val = match &assertion.value {
        Some(serde_json::Value::Bool(b)) if *b => "true",
        _ => "false",
    };
    let is_null_val = matches!(&assertion.value, Some(serde_json::Value::Null));
    let n = assertion.value.as_ref().map(json_to_zig).unwrap_or_default();
    let has_n = assertion.value.as_ref().is_some_and(|v| v.is_number() || v.is_u64());
    // Distinguish float vs integer JSON values: `std.json.Value` exposes
    // `.integer` (i64) and `.float` (f64) as separate variants. Comparing
    // `.integer` against a literal with a fractional part (e.g. `0.9`) is a
    // Zig compile error, so the template must select the right tag.
    let is_float_val = matches!(&assertion.value, Some(serde_json::Value::Number(n)) if !n.is_i64() && !n.is_u64());
    let values_list: Vec<String> = assertion
        .values
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| {
            if let serde_json::Value::String(s) = v {
                Some(format!("\"{}\"", escape_zig(s)))
            } else {
                None
            }
        })
        .collect();

    let rendered = crate::template_env::render(
        "zig/json_assertion.jinja",
        minijinja::context! {
            assertion_type => assertion.assertion_type.as_str(),
            field_expr => field_expr,
            is_length_access => is_length_access,
            zig_val => zig_val,
            is_string_val => is_string_val,
            is_bool_val => is_bool_val,
            bool_val => bool_val,
            is_null_val => is_null_val,
            n => n,
            has_n => has_n,
            is_float_val => is_float_val,
            values_list => values_list,
        },
    );
    out.push_str(&rendered);
}

/// Predicate matching `render_assertion`: returns true when the assertion
/// would emit at least one statement that references the result variable.
fn assertion_emits_code(assertion: &Assertion, field_resolver: &FieldResolver) -> bool {
    if let Some(f) = &assertion.field {
        if !f.is_empty() && is_streaming_virtual_field(f) {
            // Streaming virtual fields always emit code — they are handled in a
            // dedicated collect path, not skipped.
        } else if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            return false;
        }
    }
    matches!(
        assertion.assertion_type.as_str(),
        "equals"
            | "contains"
            | "contains_all"
            | "not_contains"
            | "not_empty"
            | "is_empty"
            | "starts_with"
            | "ends_with"
            | "min_length"
            | "max_length"
            | "count_min"
            | "count_equals"
            | "is_true"
            | "is_false"
            | "greater_than"
            | "less_than"
            | "greater_than_or_equal"
            | "less_than_or_equal"
            | "contains_any"
    )
}

/// Build setup lines and the argument list for the function call.
///
/// Returns `(setup_lines, args_str, setup_needs_gpa)` where `setup_needs_gpa`
/// is `true` when at least one setup line requires the GPA `allocator` binding.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    fixture_id: &str,
    _module_name: &str,
) -> (Vec<String>, String, bool) {
    if args.is_empty() {
        return (Vec::new(), String::new(), false);
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut setup_needs_gpa = false;

    for arg in args {
        if arg.arg_type == "mock_url" {
            let name = arg.name.clone();
            let id_upper = fixture_id.to_uppercase();
            setup_lines.push(format!(
                "const {name} = if (std.c.getenv(\"MOCK_SERVER_{id_upper}\")) |_pf| try std.fmt.allocPrint(allocator, \"{{s}}\", .{{std.mem.span(_pf)}}) else try std.fmt.allocPrint(allocator, \"{{s}}/fixtures/{fixture_id}\", .{{if (std.c.getenv(\"MOCK_SERVER_URL\")) |v| std.mem.span(v) else \"http://localhost:8080\"}});"
            ));
            setup_lines.push(format!("defer allocator.free({name});"));
            parts.push(name);
            setup_needs_gpa = true;
            continue;
        }

        // Handle args (engine handle): serialize config to JSON string literal, or null.
        // The Zig binding accepts ?[]const u8 for engine params (creates handle internally).
        if arg.arg_type == "handle" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "null".to_string(),
                Some(v) => format!("\"{}\"", escape_zig(&serde_json::to_string(v).unwrap_or_default())),
            };
            parts.push(json_str);
            continue;
        }

        // The Zig wrapper accepts struct parameters (e.g. `ExtractionConfig`)
        // as JSON `[]const u8`, converting them to opaque FFI handles via the
        // `<prefix>_<snake>_from_json` helper at the binding layer. Emit the
        // fixture's configuration value as a JSON string literal, falling back
        // to `"{}"` when the fixture omits a config so callers exercise the
        // default path.
        if arg.name == "config" && arg.arg_type == "json_object" {
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let json_str = match input.get(field) {
                Some(serde_json::Value::Null) | None => "{}".to_string(),
                Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
            };
            parts.push(format!("\"{}\"", escape_zig(&json_str)));
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        // When `field` is empty or refers to `input` itself (no dotted subfield),
        // the entire fixture `input` value is the payload — most commonly for
        // `json_object` request bodies (chat/embed/etc.). Without this guard
        // `input.get("input")` returns `None` and we fall through to `"{}"`,
        // which the FFI rejects as a deserialization error.
        let val = if field.is_empty() || field == "input" {
            Some(input)
        } else {
            input.get(field)
        };
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                // Zig functions don't have default arguments, so we must
                // pass `null` explicitly for every optional parameter.
                parts.push("null".to_string());
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    "json_object" => "\"{}\"".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                // For `json_object` arguments other than `config` (handled
                // above) the Zig binding accepts a JSON `[]const u8`, so we
                // serialize the entire fixture value as a single JSON string
                // literal rather than rendering it as a Zig array/struct.
                if arg.arg_type == "json_object" {
                    let json_str = serde_json::to_string(v).unwrap_or_default();
                    parts.push(format!("\"{}\"", escape_zig(&json_str)));
                } else if arg.arg_type == "bytes" {
                    // `bytes` args are file paths in fixtures — read the file into a
                    // local buffer. The cwd is set to test_documents/ at runtime.
                    // Zig 0.16 uses std.Io.Dir.cwd() (not std.fs.cwd()) and requires
                    // an `io` instance from std.testing.io in test context.
                    if let serde_json::Value::String(path) = v {
                        let var_name = format!("{}_bytes", arg.name);
                        let epath = escape_zig(path);
                        setup_lines.push(format!(
                            "const {var_name} = try std.Io.Dir.cwd().readFileAlloc(std.testing.io, \"{epath}\", std.heap.c_allocator, .unlimited);"
                        ));
                        setup_lines.push(format!("defer std.heap.c_allocator.free({var_name});"));
                        parts.push(var_name);
                    } else {
                        parts.push(json_to_zig(v));
                    }
                } else {
                    parts.push(json_to_zig(v));
                }
            }
        }
    }

    (setup_lines, parts.join(", "), setup_needs_gpa)
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    result_is_option: bool,
) {
    // Bare-result assertions on `?T` (Optional) translate to null-checks instead
    // of `.len`. Mirrors the same behaviour in kotlin.rs (bare_result_is_option).
    let bare_result_is_option = result_is_option && assertion.field.as_deref().filter(|f| !f.is_empty()).is_none();
    if bare_result_is_option {
        match assertion.assertion_type.as_str() {
            "is_empty" => {
                let _ = writeln!(out, "    try testing.expect({result_var} == null);");
                return;
            }
            "not_empty" | "not_error" => {
                let _ = writeln!(out, "    try testing.expect({result_var} != null);");
                return;
            }
            _ => {}
        }
    }
    // Synthetic-field 'embeddings' on a JSON-bytes result (e.g. embed_texts
    // returns `Vec<Vec<f32>>` serialised as JSON). Parse the JSON array and
    // apply count_min/count_equals/not_empty/is_empty against the element count.
    if let Some(f) = &assertion.field {
        if f == "embeddings" && !field_resolver.is_valid_for_result(f) {
            match assertion.assertion_type.as_str() {
                "count_min" | "count_equals" | "not_empty" | "is_empty" => {
                    let _ = writeln!(out, "    {{");
                    let _ = writeln!(
                        out,
                        "        var _eparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {result_var}, .{{}});"
                    );
                    let _ = writeln!(out, "        defer _eparse.deinit();");
                    let _ = writeln!(out, "        const _embeddings_len = _eparse.value.array.items.len;");
                    match assertion.assertion_type.as_str() {
                        "count_min" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(out, "        try testing.expect(_embeddings_len >= {n});");
                            }
                        }
                        "count_equals" => {
                            if let Some(n) = assertion.value.as_ref().and_then(|v| v.as_u64()) {
                                let _ = writeln!(
                                    out,
                                    "        try testing.expectEqual(@as(usize, {n}), _embeddings_len);"
                                );
                            }
                        }
                        "not_empty" => {
                            let _ = writeln!(out, "        try testing.expect(_embeddings_len > 0);");
                        }
                        "is_empty" => {
                            let _ = writeln!(out, "        try testing.expectEqual(@as(usize, 0), _embeddings_len);");
                        }
                        _ => {}
                    }
                    let _ = writeln!(out, "    }}");
                    return;
                }
                _ => {}
            }
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field {
        if !f.is_empty() && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(out, "    // skipped: field '{{f}}' not available on result type");
            return;
        }
    }

    // Determine if this field is an enum type.
    let _field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|f| enum_fields.contains(f) || enum_fields.contains(field_resolver.resolve(f)));

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "zig", result_var),
        _ => result_var.to_string(),
    };

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(out, "    try testing.expectEqual({zig_val}, {field_expr});");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let zig_val = json_to_zig(val);
                    let _ = writeln!(
                        out,
                        "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) != null);"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.indexOf(u8, {field_expr}, {zig_val}) == null);"
                );
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len > 0);");
        }
        "is_empty" => {
            let _ = writeln!(out, "    try testing.expect({field_expr}.len == 0);");
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.startsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let zig_val = json_to_zig(expected);
                let _ = writeln!(
                    out,
                    "    try testing.expect(std.mem.endsWith(u8, {field_expr}, {zig_val}));"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len <= {n});");
                }
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    let _ = writeln!(out, "    try testing.expect({field_expr}.len >= {n});");
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value {
                if let Some(n) = val.as_u64() {
                    // When there is no field (field_expr == result_var), the result
                    // is `[]u8` JSON (e.g. batch functions). Parse the JSON array
                    // and count its elements; `.len` would give byte count, not item count.
                    let has_field = assertion.field.as_deref().is_some_and(|f| !f.is_empty());
                    if has_field {
                        let _ = writeln!(out, "    try testing.expectEqual({n}, {field_expr}.len);");
                    } else {
                        let _ = writeln!(out, "    {{");
                        let _ = writeln!(
                            out,
                            "        var _cparse = try std.json.parseFromSlice(std.json.Value, std.heap.c_allocator, {field_expr}, .{{}});"
                        );
                        let _ = writeln!(out, "        defer _cparse.deinit();");
                        let _ = writeln!(
                            out,
                            "        try testing.expectEqual({n}, _cparse.value.array.items.len);"
                        );
                        let _ = writeln!(out, "    }}");
                    }
                }
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    try testing.expect({field_expr});");
        }
        "is_false" => {
            let _ = writeln!(out, "    try testing.expect(!{field_expr});");
        }
        "not_error" => {
            // Already handled by the call succeeding.
        }
        "error" => {
            // Handled at the test function level.
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} > {zig_val});");
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} < {zig_val});");
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} >= {zig_val});");
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let zig_val = json_to_zig(val);
                let _ = writeln!(out, "    try testing.expect({field_expr} <= {zig_val});");
            }
        }
        "contains_any" => {
            // At least ONE of the values must be found in the field (OR logic).
            if let Some(values) = &assertion.values {
                let string_values: Vec<String> = values
                    .iter()
                    .filter_map(|v| {
                        if let serde_json::Value::String(s) = v {
                            Some(format!(
                                "std.mem.indexOf(u8, {field_expr}, \"{}\") != null",
                                escape_zig(s)
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();
                if !string_values.is_empty() {
                    let condition = string_values.join(" or\n        ");
                    let _ = writeln!(out, "    try testing.expect(\n        {condition}\n    );");
                }
            }
        }
        "matches_regex" => {
            let _ = writeln!(out, "    // regex match not yet implemented for Zig");
        }
        "method_result" => {
            let _ = writeln!(out, "    // method_result assertions not yet implemented for Zig");
        }
        other => {
            panic!("Zig e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Convert a `serde_json::Value` to a Zig literal string.
fn json_to_zig(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", escape_zig(s)),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_zig).collect();
            format!("&.{{{}}}", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            format!("\"{}\"", escape_zig(&json_str))
        }
    }
}
