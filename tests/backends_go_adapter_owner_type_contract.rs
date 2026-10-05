//! Issue #447: `[[adapters]] owner_type` must be normalized through the same `go_type_name`
//! helper `methods.rs` uses for the receiver type, or a configured value containing an
//! initialism (`ApiClient`, `HttpEngine`, ...) names a Go identifier the receiver/iterator
//! declaration never declares. A real `go build` is the only check that actually catches a
//! mismatched identifier -- a string assertion on either emitter alone passes while the two
//! disagree, which is exactly how this bug survived #441. ~keep

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

const OWNER_TYPE_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>

typedef uint64_t TESTApiEngine;
typedef uint64_t TESTAlefHandle;

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }
static inline void test_api_engine_free(TESTApiEngine h) {}
static inline TESTAlefHandle test_api_engine_crawl_stream_start(TESTApiEngine self) { return 1; }
static inline TESTAlefHandle test_api_engine_crawl_stream_next(TESTAlefHandle handle) { return 0; }
static inline void test_api_engine_crawl_stream_free(TESTAlefHandle handle) {}
static inline char *test_crawl_event_to_json(TESTAlefHandle chunk) { return NULL; }
static inline void test_crawl_event_free(TESTAlefHandle chunk) {}
static inline void test_free_string(char *s) {}
"#;

fn owner_type_config() -> ResolvedCrateConfig {
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
    // `owner_type` is deliberately `ApiEngine`, not `Engine`: `Api` is a Go initialism
    // (`go_type_name` uppercases it to `API`), so this is the config shape that discriminates
    // between "normalized like the receiver type" and "used verbatim" (`CrawlEngine`/`Engine`
    // style values happen not to contain an initialism and pass either way).
    resolved.adapters = vec![AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "test_lib::ApiEngine::crawl_stream".to_string(),
        params: Vec::new(),
        returns: None,
        error_type: None,
        owner_type: Some("ApiEngine".to_string()),
        item_type: Some("CrawlEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("test_lib::CrawlRequest".to_string()),
        skip_languages: Vec::new(),
    }];
    resolved
}

/// The opaque `ApiEngine` receiver (matching `owner_type` above) with one streaming method, plus
/// the `CrawlEvent` item type -- the same minimal shape as the #441 streaming contract test, with
/// the receiver/owner name swapped for one that contains an initialism.
fn owner_type_api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "ApiEngine".to_string(),
                rust_path: "test_lib::ApiEngine".to_string(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "crawl_stream".to_string(),
                    return_type: TypeRef::Unit,
                    receiver: Some(ReceiverKind::Ref),
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "CrawlEvent".to_string(),
                rust_path: "test_lib::CrawlEvent".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "message".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
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

/// Real `go build` proof for #447: `methods.rs` declares the receiver and the
/// `<Recv><Method>Stream` iterator as `APIEngine`/`APIEngineCrawlStreamStream` (via
/// `go_type_name`), and the module-level adapter wrapper in `functions.rs` must reference the
/// exact same identifiers rather than the raw `ApiEngine` config string -- otherwise the package
/// references two undeclared types and `go build` fails.
#[test]
fn adapter_owner_type_with_an_initialism_agrees_with_the_receiver_type_and_compiles() {
    let files = GoBackend
        .generate_bindings(&owner_type_api(), &owner_type_config())
        .expect("Go bindings generate");
    let binding = files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated");

    assert!(
        binding.content.contains("type APIEngineCrawlStreamStream struct {"),
        "the iterator struct must be declared under its go_type_name-normalized name, got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains(
            "func (h *APIEngine) CrawlStreamWithContext(ctx context.Context) (*APIEngineCrawlStreamStream, error) {"
        ),
        "the method must take a leading ctx context.Context (issue #448) and return the \
         normalized iterator type, got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains(
            "func CrawlStreamWithContext(ctx context.Context, engine *APIEngine) (*APIEngineCrawlStreamStream, error) {"
        ),
        "the module-level adapter wrapper must reference the exact same normalized identifiers \
         the receiver/iterator declare, not the raw `owner_type` config string, got:\n{}",
        binding.content
    );
    assert!(
        !binding.content.contains("*ApiEngine"),
        "the raw (non-normalized) owner_type string must not leak into the generated package, got:\n{}",
        binding.content
    );

    assert_real_go_build_with_header(&binding.content, OWNER_TYPE_HEADER);
}
