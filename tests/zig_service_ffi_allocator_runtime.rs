use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef, ServiceDef, TypeRef,
};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn config() -> ResolvedCrateConfig {
    let parsed: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"
"#,
    )
    .expect("parse fixture config");
    parsed.resolve().expect("resolve fixture config").remove(0)
}

fn surface() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "test::TestService".to_owned(),
            constructor: MethodDef {
                name: "new".to_owned(),
                is_static: true,
                return_type: TypeRef::Unit,
                ..MethodDef::default()
            },
            configurators: vec![],
            registrations: vec![route_registration()],
            entrypoints: vec![],
            doc: "Neutral Zig service ABI fixture.".to_owned(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract()],
        ..ApiSurface::default()
    }
}

fn route_registration() -> RegistrationDef {
    RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![ParamDef {
            name: "path".to_owned(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        ..RegistrationDef::default()
    }
}

fn handler_contract() -> HandlerContractDef {
    HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "test::RequestHandler".to_owned(),
        dispatch: MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("Request".to_owned()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("Response".to_owned()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        },
        optional_methods: vec![],
        wire_request_type: Some("Request".to_owned()),
        wire_response_type: Some("Response".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Neutral handler contract.".to_owned(),
    }
}

fn write_fixture(directory: &Path, service_source: &str, header_name: &str) {
    fs::write(directory.join("test_service.zig"), service_source).expect("write generated Zig service");
    fs::write(directory.join(header_name), C_HEADER).expect("write C header");
    fs::write(directory.join("service_fixture.c"), C_SOURCE).expect("write C source");
    fs::write(directory.join("allocator_test.zig"), ZIG_HARNESS).expect("write Zig harness");
    fs::write(directory.join("build.zig"), ZIG_BUILD).expect("write Zig build script");
}

fn output_or_panic(action: &str, output: Output) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const C_HEADER: &str = r#"#include <stdint.h>
typedef char *(*handler_callback_t)(void *, const char *);
typedef void (*handler_response_free_t)(char *);
uint64_t test_test_service_new(void);
void test_test_service_free(uint64_t owner);
int test_test_service_register_route(
    uint64_t owner,
    handler_callback_t callback,
    handler_response_free_t response_free,
    void *context,
    const char *path
);
"#;

const C_SOURCE: &str = r#"#include "test.h"
#include <string.h>
uint64_t test_test_service_new(void) { return 41; }
void test_test_service_free(uint64_t owner) { (void)owner; }
int test_test_service_register_route(
    uint64_t owner,
    handler_callback_t callback,
    handler_response_free_t response_free,
    void *context,
    const char *path
) {
    char *response = callback(context, "{}");
    int valid = response != 0 && strcmp(response, "{}") == 0;
    response_free(response);
    return owner == 41 && valid && strcmp(path, "/ok") == 0 ? 17 : -1;
}
"#;

const ZIG_HARNESS: &str = r#"const std = @import("std");
const service = @import("test_service.zig");

fn handler(context: *anyopaque, request: [*:0]const u8) callconv(.c) [*:0]u8 {
    _ = context;
    _ = request;
    return std.fmt.allocPrintSentinel(std.heap.c_allocator, "{s}", .{"{}"}, 0) catch unreachable;
}

fn freeResponse(response: [*:0]u8) callconv(.c) void {
    std.heap.c_allocator.free(std.mem.span(response));
}

test "service registration preserves allocator pairing" {
    var instance = service.TestService.init();
    defer instance.deinit();
    const context: *anyopaque = @ptrFromInt(1);
    const status = instance.route(handler, freeResponse, context, "/ok");
    try std.testing.expectEqual(@as(c_int, 17), status);
}
"#;

const ZIG_BUILD: &str = r#"const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const translate_c = b.addTranslateC(.{
        .root_source_file = b.path("test.h"),
        .target = target,
        .optimize = optimize,
    });
    translate_c.addIncludePath(b.path("."));

    const test_module = b.createModule(.{
        .root_source_file = b.path("allocator_test.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    test_module.addImport("c", translate_c.createModule());
    test_module.addCSourceFile(.{
        .file = b.path("service_fixture.c"),
        .flags = &.{},
    });
    test_module.addIncludePath(b.path("."));

    const tests = b.addTest(.{ .root_module = test_module });
    const run_tests = b.addRunArtifact(tests);
    const test_step = b.step("test", "Run allocator ABI test");
    test_step.dependOn(&run_tests.step);
}
"#;

#[test]
fn generated_zig_service_matches_allocator_callback_abi_at_runtime() {
    if which::which("zig").is_err() || which::which("cc").is_err() {
        return;
    }
    let config = config();
    let files = ZigBackend
        .generate_service_api(&surface(), &config)
        .expect("generate Zig service API");
    let service_source = &files.first().expect("generated Zig service file").content;
    assert!(service_source.contains("response_free"));
    assert!(service_source.contains("owner: u64"));
    let directory = tempfile::tempdir().expect("create fixture directory");
    write_fixture(directory.path(), service_source, &config.ffi_header_name());
    let output = Command::new("zig")
        .args(["build", "test", "--summary", "none"])
        .current_dir(directory.path())
        .output()
        .expect("run Zig service allocator harness");
    output_or_panic("compile and run generated Zig service", output);
}
