use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};

const HEADER: &str = r#"
#include <stdint.h>
#include <stddef.h>
void sample_set_mode(int mode);
int32_t sample_last_error_code(void);
const char *sample_last_error_context(void);
void sample_free_string(char *value);
uint64_t sample_client_new(void);
uint64_t sample_client_open(void);
uint64_t sample_client_plain(void);
void sample_client_free(uint64_t handle);
char *sample_list(const char *query);
size_t sample_list_len(const char *query);
"#;

// ~keep The native fixture owns this small C module. Rebuilding Zig 0.17's C translator here
// would repeat the separate cold-translation regressions instead of testing wrapper errors.
const C_DECLARATIONS: &str = r#"
pub extern fn sample_set_mode(mode: c_int) void;
pub extern fn sample_last_error_code() i32;
pub extern fn sample_last_error_context() [*c]const u8;
pub extern fn sample_free_string(value: [*c]u8) void;
pub extern fn sample_client_new() u64;
pub extern fn sample_client_open() u64;
pub extern fn sample_client_plain() u64;
pub extern fn sample_client_free(handle: u64) void;
pub extern fn sample_list(query: [*c]const u8) [*c]u8;
pub extern fn sample_list_len(query: [*c]const u8) usize;
"#;

const NATIVE: &str = r#"
#include "sample.h"
#include <stdlib.h>
#include <string.h>
static int mode;
static int32_t code;
static const char *message;
void sample_set_mode(int value) { mode = value; code = 0; message = NULL; }
int32_t sample_last_error_code(void) { return code; }
const char *sample_last_error_context(void) { return message; }
void sample_free_string(char *value) { free(value); code = 0; message = NULL; }
uint64_t sample_client_new(void) {
    if (mode) { code = mode == 1 ? 100 : mode == 3 ? 5 : 77; message = "client rejected"; return 0; }
    return 9;
}
uint64_t sample_client_open(void) { return sample_client_new(); }
uint64_t sample_client_plain(void) { return sample_client_new(); }
void sample_client_free(uint64_t handle) { (void)handle; code = 0; message = NULL; }
char *sample_list(const char *query) {
    if ((unsigned char)query[0] == 255) {
        code = 1; message = "invalid UTF-8 in query"; return NULL;
    }
    char *result = malloc(3);
    memcpy(result, "[]", 3);
    return result;
}
size_t sample_list_len(const char *query) { (void)query; code = 0; message = NULL; return 2; }
"#;

const CONSTRUCTOR_TESTS: &str = r#"
test "a constructor returns the native error and message" {
    c.sample_set_mode(1);
    try std.testing.expectError(error.Rejected, new_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "a static factory returns the native error and message" {
    c.sample_set_mode(1);
    try std.testing.expectError(error.Rejected, open_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "a configured constructor returns the native error and message" {
    c.sample_set_mode(1);
    try std.testing.expectError(error.Rejected, create_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "an unknown native code stays unknown" {
    c.sample_set_mode(2);
    try std.testing.expectError(error.UnknownFfiError, new_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "a successful constructor returns a usable handle" {
    c.sample_set_mode(0);
    var client = try new_client();
    defer client.free();
    try std.testing.expectEqual(@as(u64, 9), client._handle);
}
test "a cancelled native factory retains its typed cancellation and message" {
    c.sample_set_mode(3);
    try std.testing.expectError(error.Cancelled, new_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "an untyped static factory retains its native message" {
    c.sample_set_mode(2);
    try std.testing.expectError(error.UnknownFfiError, plain_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
test "an untyped static factory retains cancellation and its message" {
    c.sample_set_mode(3);
    try std.testing.expectError(error.Cancelled, plain_client());
    try std.testing.expectEqualStrings("client rejected", _last_error().?);
}
"#;

const LIST_TESTS: &str = r#"
test "an invalid argument returns an error before the length call clears its message" {
    c.sample_set_mode(0);
    try std.testing.expectError(error.UnknownFfiError, list("\xff"));
    try std.testing.expectEqualStrings("invalid UTF-8 in query", _last_error().?);
}
test "a successful empty list remains readable" {
    c.sample_set_mode(0);
    const result = try list("valid");
    defer std.heap.c_allocator.free(result);
    try std.testing.expectEqualStrings("[]", result);
}
"#;

fn request_error() -> ErrorDef {
    ErrorDef {
        name: "RequestError".into(),
        rust_path: "sample::RequestError".into(),
        variants: vec![ErrorVariant {
            name: "Rejected".into(),
            error_code: Some(100),
            is_unit: true,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn render(api: &ApiSurface, configured: bool) -> String {
    let config = if configured {
        r#"
[workspace]
languages = ["zig"]
[workspace.client_constructors.Client]
body = "Client::new()"
error_type = "RequestError"
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
"#
    } else {
        r#"
[workspace]
languages = ["zig"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
"#
    };
    let config: NewAlefConfig = toml::from_str(config).expect("valid fixture config");
    let config = config.resolve().expect("resolved config").remove(0);
    ZigBackend.generate_bindings(api, &config).expect("generated binding")[0]
        .content
        .clone()
}

fn run_zig(source: &str, tests: &str, count: usize) {
    let directory = tempfile::tempdir().expect("test directory");
    std::fs::write(directory.path().join("sample.h"), HEADER).expect("write C header");
    std::fs::write(directory.path().join("native.c"), NATIVE).expect("write native fixture");
    std::fs::write(directory.path().join("binding.zig"), format!("{source}\n{tests}")).expect("write binding");
    std::fs::write(directory.path().join("c.zig"), C_DECLARATIONS).expect("write native declarations");
    let output = std::process::Command::new("zig")
        .args([
            "test",
            "-lc",
            "native.c",
            "--dep",
            "c",
            "-Mroot=binding.zig",
            "-Mc=c.zig",
        ])
        .current_dir(directory.path())
        .output()
        .expect("Zig must be installed to run native binding regressions");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "generated binding failed:\n{report}\n{source}");
    assert!(report.contains(&format!("All {count} tests passed")), "{report}");
}

#[test]
fn should_preserve_native_constructor_and_static_errors() {
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Client".into(),
            is_opaque: true,
            methods: ["new", "open", "plain"]
                .into_iter()
                .map(|name| MethodDef {
                    name: name.into(),
                    is_static: true,
                    return_type: TypeRef::Named("Client".into()),
                    error_type: (name != "plain").then(|| "RequestError".into()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
        errors: vec![request_error()],
        ..Default::default()
    };
    run_zig(&render(&api, true), CONSTRUCTOR_TESTS, 8);
}

#[test]
fn should_guard_infallible_collection_results_before_slicing() {
    let api = ApiSurface {
        functions: vec![FunctionDef {
            name: "list".into(),
            params: vec![ParamDef {
                name: "query".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            return_type: TypeRef::Vec(Box::new(TypeRef::String)),
            ..Default::default()
        }],
        ..Default::default()
    };
    run_zig(&render(&api, false), LIST_TESTS, 2);
}
