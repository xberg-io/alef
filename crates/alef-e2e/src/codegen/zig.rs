//! Zig e2e test generator using std.testing.
//!
//! Generates `packages/zig/src/<crate>_test.zig` files from JSON fixtures,
//! driven entirely by `E2eConfig` and `CallConfig`.

use crate::config::E2eConfig;
use crate::escape::{escape_zig, sanitize_filename};
use crate::field_access::FieldResolver;
use crate::fixture::{Assertion, Fixture, FixtureGroup};
use alef_core::backend::GeneratedFile;
use alef_core::config::AlefConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::template_versions::toolchain;
use anyhow::Result;
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use super::E2eCodegen;

/// Zig e2e code generator.
pub struct ZigE2eCodegen;

impl E2eCodegen for ZigE2eCodegen {
    fn generate(
        &self,
        groups: &[FixtureGroup],
        e2e_config: &E2eConfig,
        alef_config: &AlefConfig,
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
            .unwrap_or_else(|| alef_config.crate_config.name.to_snake_case());

        // Generate build.zig.zon (Zig package manifest).
        files.push(GeneratedFile {
            path: output_base.join("build.zig.zon"),
            content: render_build_zig_zon(&pkg_name, &pkg_path, e2e_config.dep_mode),
            generated_header: false,
        });

        // Get the module name for imports.
        let module_name = alef_config.zig_module_name();

        // Generate build.zig - collect test file names first.
        let field_resolver = FieldResolver::new(
            &e2e_config.fields,
            &e2e_config.fields_optional,
            &e2e_config.result_fields,
            &e2e_config.fields_array,
        );

        // Generate test files per category and collect their names.
        let mut test_filenames: Vec<String> = Vec::new();
        for group in groups {
            let active: Vec<&Fixture> = group
                .fixtures
                .iter()
                .filter(|f| f.skip.as_ref().is_none_or(|s| !s.should_skip(lang)))
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
                content: render_build_zig(&test_filenames),
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

fn render_build_zig(test_filenames: &[String]) -> String {
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

    let mut content = String::from("const std = @import(\"std\");\n\npub fn build(b: *std.Build) void {\n");
    content.push_str("    const target = b.standardTargetOptions(.{});\n");
    content.push_str("    const optimize = b.standardOptimizeOption(.{});\n");
    content.push_str("    const test_step = b.step(\"test\", \"Run tests\");\n\n");

    for filename in test_filenames {
        // Convert filename like "basic_test.zig" to a test name
        let test_name = filename.trim_end_matches("_test.zig");
        content.push_str(&format!("    const {test_name}_module = b.createModule(.{{\n"));
        content.push_str(&format!("        .root_source_file = b.path(\"src/{filename}\"),\n"));
        content.push_str("        .target = target,\n");
        content.push_str("        .optimize = optimize,\n");
        content.push_str("    });\n");
        content.push_str(&format!("    const {test_name}_tests = b.addTest(.{{\n"));
        content.push_str(&format!("        .root_module = {test_name}_module,\n"));
        content.push_str("    });\n");
        content.push_str(&format!(
            "    const {test_name}_run = b.addRunArtifact({test_name}_tests);\n"
        ));
        content.push_str(&format!("    test_step.dependOn(&{test_name}_run.step);\n\n"));
    }

    content.push_str("}\n");
    content
}

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
    let call_config = e2e_config.resolve_call(fixture.call.as_deref());
    let lang = "zig";
    let call_overrides = call_config.overrides.get(lang);
    let function_name = call_overrides
        .and_then(|o| o.function.as_ref())
        .cloned()
        .unwrap_or_else(|| call_config.function.clone());
    let result_var = &call_config.result_var;
    let args = &call_config.args;

    let test_name = fixture.id.to_snake_case();
    let description = &fixture.description;
    let expects_error = fixture.assertions.iter().any(|a| a.assertion_type == "error");

    let (setup_lines, args_str) = build_args_and_setup(&fixture.input, args, &fixture.id);

    let _ = writeln!(out, "test \"{test_name}\" {{");
    let _ = writeln!(out, "    // {description}");

    // Only emit allocator setup when setup lines actually need it (avoids unused-variable errors).
    let needs_alloc = !setup_lines.is_empty();
    if needs_alloc {
        let _ = writeln!(out, "    var gpa: std.heap.DebugAllocator(.{{}}) = .init;");
        let _ = writeln!(out, "    defer _ = gpa.deinit();");
        let _ = writeln!(out, "    const allocator = gpa.allocator();");
        let _ = writeln!(out);
    }

    for line in &setup_lines {
        let _ = writeln!(out, "    {line}");
    }

    if expects_error {
        // Stub: error-path tests are not yet callable without a real FFI handle_request.
        let _ = writeln!(
            out,
            "    // TODO: call {module_name}.{function_name}({args_str}) and assert error"
        );
        let _ = writeln!(out, "    _ = testing;");
        let _ = writeln!(out, "}}");
        return;
    }

    if fixture.assertions.is_empty() {
        // No assertions: emit a compilation-only stub so the test passes trivially.
        let _ = writeln!(out, "    // TODO: call {module_name}.{function_name}({args_str})");
        let _ = writeln!(out, "    _ = testing;");
    } else {
        let _ = writeln!(
            out,
            "    const {result_var} = {module_name}.{function_name}({args_str});"
        );
        for assertion in &fixture.assertions {
            render_assertion(out, assertion, result_var, field_resolver, enum_fields);
        }
    }

    let _ = writeln!(out, "}}");
}

/// Build setup lines and the argument list for the function call.
fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::config::ArgMapping],
    fixture_id: &str,
) -> (Vec<String>, String) {
    if args.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        if arg.arg_type == "mock_url" {
            setup_lines.push(format!(
                "var {} = try allocator.alloc(u8, std.fmt.bufPrint(undefined, \"{{s}}/fixtures/{fixture_id}\", .{{std.posix.getenv(\"MOCK_SERVER_URL\") orelse \"http://localhost:8080\"}}) catch 0)",
                arg.name,
            ));
            parts.push(arg.name.clone());
            continue;
        }

        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);
        match val {
            None | Some(serde_json::Value::Null) if arg.optional => {
                continue;
            }
            None | Some(serde_json::Value::Null) => {
                let default_val = match arg.arg_type.as_str() {
                    "string" => "\"\"".to_string(),
                    "int" | "integer" => "0".to_string(),
                    "float" | "number" => "0.0".to_string(),
                    "bool" | "boolean" => "false".to_string(),
                    _ => "null".to_string(),
                };
                parts.push(default_val);
            }
            Some(v) => {
                parts.push(json_to_zig(v));
            }
        }
    }

    (setup_lines, parts.join(", "))
}

fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
) {
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
                    let _ = writeln!(out, "    try testing.expectEqual({n}, {field_expr}.len);");
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
