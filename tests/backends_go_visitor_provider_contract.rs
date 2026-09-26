use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

const PROVIDER_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }
"#;

fn options_field_config() -> ResolvedCrateConfig {
    let source = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "example.invalid/test-lib"
"#;
    let config: NewAlefConfig = toml::from_str(source).expect("test config parses");
    let mut resolved = config.resolve().expect("test config resolves").remove(0);
    resolved.trait_bridges = vec![TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("Options".to_string()),
        options_field: Some("visitor".to_string()),
        context_type: Some("NodeContext".to_string()),
        result_type: Some("VisitorChoice".to_string()),
        ..TraitBridgeConfig::default()
    }];
    resolved
}

fn visitor_owned_field_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            envelope_type(),
            node_context_type(),
            TypeDef {
                name: "Options".into(),
                has_serde: true,
                ..Default::default()
            },
        ],
        enums: vec![EnumDef {
            name: "VisitorChoice".into(),
            variants: vec![EnumVariant {
                name: "Continue".into(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..ApiSurface::default()
    }
}

fn envelope_type() -> TypeDef {
    TypeDef {
        name: "Envelope".to_string(),
        rust_path: "test_lib::Envelope".to_string(),
        fields: vec![
            optional_named_field("context", "NodeContext"),
            optional_named_field("choice", "VisitorChoice"),
            FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
        ],
        has_serde: true,
        ..TypeDef::default()
    }
}

fn node_context_type() -> TypeDef {
    TypeDef {
        name: "NodeContext".to_string(),
        rust_path: "test_lib::NodeContext".to_string(),
        methods: vec![MethodDef {
            name: "to_json".to_string(),
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..MethodDef::default()
        }],
        has_serde: true,
        has_lifetime_params: true,
        ..TypeDef::default()
    }
}

fn optional_named_field(name: &str, target: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named(target.into()))),
        optional: true,
        ..Default::default()
    }
}

fn native_library_dir() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(".lib/macos-arm64"),
        ("macos", "x86_64") => Some(".lib/macos-x86_64"),
        ("linux", "x86_64") => Some(".lib/linux-x86_64"),
        ("linux", "aarch64") => Some(".lib/linux-aarch64"),
        ("windows", "x86_64") => Some(".lib/windows-x86_64"),
        _ => None,
    }
}

fn write_empty_native_archive(directory: &std::path::Path, library_dir: &str) -> bool {
    let compiler = which::which("cc").or_else(|_| which::which("gcc"));
    let (Ok(compiler), Ok(archiver)) = (compiler, which::which("ar")) else {
        return false;
    };
    let object = directory.join("empty.o");
    std::fs::write(directory.join("empty.c"), "void test_link_anchor(void) {}\n").expect("write empty C source");
    let compiled = std::process::Command::new(compiler)
        .args(["-c", "empty.c", "-o", "empty.o"])
        .current_dir(directory)
        .status()
        .expect("run C compiler");
    assert!(compiled.success(), "empty native object compiles");
    let library_dir = directory.join(library_dir);
    std::fs::create_dir_all(&library_dir).expect("create native library directory");
    std::process::Command::new(archiver)
        .arg("rcs")
        .arg(library_dir.join("libtest_ffi.a"))
        .arg(object)
        .status()
        .expect("run native archiver")
        .success()
}

fn assert_real_go_build(binding: &str) {
    assert_real_go_build_with_header(binding, PROVIDER_HEADER);
}

fn assert_real_go_build_with_header(binding: &str, header: &str) {
    let library_dir = native_library_dir().expect("generated Go binding compile fixture supports this target");
    let go = which::which("go").expect("Go is required for generated binding compile fixtures");
    let directory = tempfile::tempdir().expect("temporary Go package directory");
    std::fs::create_dir_all(directory.path().join("include")).expect("create include directory");
    std::fs::write(directory.path().join("include/test.h"), header).expect("write provider header");
    std::fs::write(directory.path().join("binding.go"), binding).expect("write generated binding");
    assert!(write_empty_native_archive(directory.path(), library_dir));
    let output = std::process::Command::new(go)
        .args(["build", "./..."])
        .env("GO111MODULE", "off")
        .env("CGO_ENABLED", "1")
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler");
    assert!(
        output.status.success(),
        "generated Go package failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn options_field_associated_types_do_not_call_unprovided_ffi_symbols() {
    let files = GoBackend
        .generate_bindings(&visitor_owned_field_api(), &options_field_config())
        .expect("Go bindings generate");
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated");

    assert!(
        !binding.content.contains("C.test_node_context_"),
        "options-field associated types are provided by visitor.go and must not bind unexported FFI symbols:\n{}",
        binding.content
    );
    assert_eq!(
        binding.content.matches("*json.RawMessage").count(),
        4,
        "{}",
        binding.content
    );
    assert!(!binding.content.contains("type NodeContext struct"));
    assert!(!binding.content.contains("type VisitorChoice "));
    assert_real_go_build(&binding.content);
}

// -- issue #439: LockOSThread around the FFI call + lastError() read + result conversion -- ~keep

const LOCK_OS_THREAD_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>

typedef uint64_t TESTCounter;
typedef uint64_t TESTAlefHandle;

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }
static inline TESTAlefHandle test_placeholder_from_json(const char *json) { return 1; }
static inline void test_placeholder_free(TESTAlefHandle h) {}
static inline uint32_t test_get_count(TESTAlefHandle options) { return 7; }
static inline void test_counter_free(TESTCounter h) {}
static inline uint32_t test_counter_value(TESTCounter h) { return 9; }
"#;

fn lock_os_thread_config() -> ResolvedCrateConfig {
    let source = r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.go]
module = "example.invalid/test-lib"
"#;
    let config: NewAlefConfig = toml::from_str(source).expect("test config parses");
    config.resolve().expect("test config resolves").remove(0)
}

/// A fallible free function and a fallible method on an opaque type, both plain primitive
/// returns -- the minimal shape that exercises `gen_function_wrapper`'s and
/// `gen_method_wrapper`'s `C.<fn>(...)` call followed by the separate `lastError()` cgo call.
fn lock_os_thread_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Counter".to_string(),
                rust_path: "test_lib::Counter".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "value".to_string(),
                    return_type: TypeRef::Primitive(PrimitiveType::U32),
                    error_type: Some("TestError".to_string()),
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
            // A plain DTO whose own marshal/unmarshal code is what makes `encoding/json`
            // legitimately needed here -- `Counter.value()` and `get_count()`'s own bodies are
            // both primitive in and out, so this exercises the case where the import must stay
            // because a *different* part of the package (the DTO's marshal/unmarshal code)
            // literally calls `json.`, not because the package merely has sync functions or
            // non-static methods (see #440 for the case where that used to be enough on its own).
            TypeDef {
                name: "Placeholder".to_string(),
                rust_path: "test_lib::Placeholder".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "label".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..TypeDef::default()
            },
        ],
        functions: vec![FunctionDef {
            name: "get_count".to_string(),
            rust_path: "test_lib::get_count".to_string(),
            params: vec![alef::core::ir::ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("Placeholder".to_string()),
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
                core_wrapper: alef::core::ir::CoreWrapper::None,
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            error_type: Some("TestError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

/// Real `go build` proof for #439: the generated free function and method wrappers both pin
/// the goroutine to one OS thread across the FFI call and the `lastError()` read, and the
/// package still compiles -- in particular, the `runtime` import `body_uses_qualified_name`
/// adds is neither missing (the lock lines would be undeclared identifiers) nor unused/duplicated
/// (either of which is a Go compile error).
#[test]
fn lock_os_thread_wrappers_compile_and_lock_around_the_ffi_call() {
    let files = GoBackend
        .generate_bindings(&lock_os_thread_api(), &lock_os_thread_config())
        .expect("Go bindings generate");
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated");

    assert_eq!(
        binding.content.matches("\"runtime\"").count(),
        1,
        "the runtime import must be added exactly once, got:\n{}",
        binding.content
    );

    let free_fn_body = binding
        .content
        .split("func GetCount(")
        .nth(1)
        .expect("GetCount wrapper must be generated");
    let lock_pos = free_fn_body
        .find("runtime.LockOSThread()")
        .expect("free function must lock");
    let call_pos = free_fn_body
        .find("C.test_get_count(")
        .expect("free function must call the FFI symbol");
    let last_error_pos = free_fn_body
        .find("if err := lastError()")
        .expect("free function must read lastError()");
    assert!(
        lock_pos < call_pos && call_pos < last_error_pos,
        "lock must precede the FFI call, which must precede the lastError() read, got:\n{free_fn_body}"
    );
    assert!(
        free_fn_body.contains("defer runtime.UnlockOSThread()"),
        "free function must defer the unlock so it covers every return path, got:\n{free_fn_body}"
    );

    let method_body = binding
        .content
        .split("func (h *Counter) Value(")
        .nth(1)
        .expect("Value method wrapper must be generated");
    assert!(
        method_body.contains("runtime.LockOSThread()") && method_body.contains("defer runtime.UnlockOSThread()"),
        "method wrapper must also lock around its FFI call and lastError() read, got:\n{method_body}"
    );

    assert_real_go_build_with_header(&binding.content, LOCK_OS_THREAD_HEADER);
}

// -- issue #440: encoding/json is only needed when a generated body literally calls it -- ~keep

const PRIMITIVE_ONLY_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>

typedef uint64_t TESTCounter;

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }
static inline uint32_t test_add(uint32_t left, uint32_t right) { return left + right; }
static inline void test_counter_free(TESTCounter h) {}
static inline uint32_t test_counter_value(TESTCounter h) { return 9; }
"#;

/// A sync free function and a non-static method, both entirely primitive params and returns,
/// with no DTO anywhere in the surface -- the minimal shape that triggers #440:
/// `has_sync_functions`/`has_non_static_methods` were true while nothing in the generated body
/// ever referenced `json.`.
fn primitive_only_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "Counter".to_string(),
            rust_path: "test_lib::Counter".to_string(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "value".to_string(),
                return_type: TypeRef::Primitive(PrimitiveType::U32),
                receiver: Some(ReceiverKind::Ref),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        functions: vec![FunctionDef {
            name: "add".to_string(),
            rust_path: "test_lib::add".to_string(),
            params: vec![
                alef::core::ir::ParamDef {
                    name: "left".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
                alef::core::ir::ParamDef {
                    name: "right".to_string(),
                    ty: TypeRef::Primitive(PrimitiveType::U32),
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
                    core_wrapper: alef::core::ir::CoreWrapper::None,
                },
            ],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

/// Real `go build` proof for #440: a package whose only sync function and only non-static
/// method are both fully primitive must not import `encoding/json` -- an unused import is a
/// Go compile error. `has_sync_functions`/`has_non_static_methods` used to add the import
/// unconditionally regardless of whether anything in the generated body actually called `json.`.
#[test]
fn primitive_only_package_does_not_import_unused_encoding_json() {
    let files = GoBackend
        .generate_bindings(&primitive_only_api(), &lock_os_thread_config())
        .expect("Go bindings generate");
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated");

    assert!(
        !binding.content.contains("\"encoding/json\""),
        "a primitive-only package must not import encoding/json, got:\n{}",
        binding.content
    );

    assert_real_go_build_with_header(&binding.content, PRIMITIVE_ONLY_HEADER);
}
