//! Issue #623: the plain Go streaming call returns the channel only, so a stream that ends in an
//! error closes the channel like a clean end. `<Method>WithErr` returns the stream object, so
//! the caller can read `Err()` after the range loop.
//!
//! The text assertions in `src/backends/go/gen_bindings/methods/streaming_error_paths_tests.rs`
//! prove the new call is emitted. This test builds the generated package against a native stub
//! whose stream yields two items and then fails, and runs it with `go test`: the plain call sees
//! two chunks and no error, and the new call sees the same chunks and the stream error. A
//! `WithErr` that drops the error, for example by returning `stream.Chan()`, fails this test.

#![allow(clippy::print_stderr)]

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// The native stub: the stream yields two items, then reports end-of-stream with a nonzero error
/// code, which is how the native layer reports a stream that failed after it opened.
const STREAM_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t TESTEngine;
typedef uint64_t TESTAlefHandle;

static int test_items_sent = 0;

static inline int32_t test_last_error_code(void) { return test_items_sent > 2 ? 500 : 0; }
static inline const char *test_last_error_context(void) { return test_items_sent > 2 ? "stream cut" : NULL; }

static inline void test_engine_free(TESTEngine h) {}

static inline TESTAlefHandle test_cancel_token_new(void) { return 7; }
static inline int32_t test_cancel_token_cancel(TESTAlefHandle token) { return 0; }
static inline void test_cancel_token_free(TESTAlefHandle token) {}

static inline TESTAlefHandle test_engine_crawl_stream_start_cancellable(TESTEngine self, TESTAlefHandle token) {
    test_items_sent = 0;
    return 1;
}
static inline TESTAlefHandle test_engine_crawl_stream_next(TESTAlefHandle handle) {
    test_items_sent += 1;
    return test_items_sent <= 2 ? 1 : 0;
}
static inline void test_engine_crawl_stream_free(TESTAlefHandle handle) {}

static inline char *test_crawl_event_to_json(TESTAlefHandle chunk) {
    static const char json[] = "{\"message\":\"x\"}";
    char *copy = (char *)malloc(sizeof(json));
    if (copy) {
        memcpy(copy, json, sizeof(json));
    }
    return copy;
}
static inline void test_crawl_event_free(TESTAlefHandle chunk) {}
static inline void test_free_string(char *s) { free(s); }
"#;

const ERROR_TEST_GO: &str = r#"package streamtest

import (
	"strings"
	"testing"
)

// The plain call returns the channel only: the cut is not visible to its caller.
func TestPlainStreamDropsTheStreamError(t *testing.T) {
	engine := &Engine{ptr: 1}
	ch, err := engine.CrawlStream()
	if err != nil {
		t.Fatalf("start failed: %v", err)
	}
	n := 0
	for range ch {
		n++
	}
	if n != 2 {
		t.Fatalf("expected 2 chunks, got %d", n)
	}
}

// The new call returns the stream object, and Err reports the cut after the range loop.
func TestStreamWithErrReportsTheStreamError(t *testing.T) {
	engine := &Engine{ptr: 1}
	stream, err := engine.CrawlStreamWithErr()
	if err != nil {
		t.Fatalf("start failed: %v", err)
	}
	n := 0
	for range stream.Chan() {
		n++
	}
	if n != 2 {
		t.Fatalf("expected 2 chunks, got %d", n)
	}
	if err := stream.Err(); err == nil || !strings.Contains(err.Error(), "stream cut") {
		t.Fatalf("expected the stream error, got %v", err)
	}
}
"#;

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

/// `STREAM_HEADER`'s functions are all `static inline`, so cgo compiles them directly into
/// `binding.go`'s own translation unit via the `#include "test.h"` in its cgo preamble -- they
/// never need to live in the linked static archive. The archive still has to exist and satisfy
/// the LDFLAGS `-ltest_ffi`, so this is the same empty-anchor archive
/// `tests/backends_go_adapter_owner_type_contract.rs` links against, not a rebuild of the real
/// native library.
fn write_empty_native_archive(directory: &std::path::Path, library_dir: &str) -> bool {
    let compiler = which::which("cc").or_else(|_| which::which("gcc"));
    let (Ok(compiler), Ok(archiver)) = (compiler, which::which("ar")) else {
        return false;
    };
    std::fs::write(directory.join("empty.c"), "void test_link_anchor(void) {}\n").expect("write empty C source");
    let compiled = std::process::Command::new(&compiler)
        .args(["-c", "empty.c", "-o", "empty.o"])
        .current_dir(directory)
        .status()
        .expect("run C compiler");
    if !compiled.success() {
        return false;
    }
    let library_dir_path = directory.join(library_dir);
    std::fs::create_dir_all(&library_dir_path).expect("create native library directory");
    std::process::Command::new(archiver)
        .arg("rcs")
        .arg(library_dir_path.join("libtest_ffi.a"))
        .arg(directory.join("empty.o"))
        .status()
        .expect("run native archiver")
        .success()
}

fn config() -> ResolvedCrateConfig {
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
package_name = "streamtest"
"#;
    let parsed: NewAlefConfig = toml::from_str(source).expect("test config parses");
    let mut resolved = parsed.resolve().expect("test config resolves").remove(0);
    resolved.adapters = vec![AdapterConfig {
        name: "crawl_stream".to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: "test_lib::Engine::crawl_stream".to_string(),
        params: Vec::new(),
        returns: None,
        error_type: None,
        owner_type: Some("Engine".to_string()),
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

fn api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "Engine".to_string(),
                rust_path: "test_lib::Engine".to_string(),
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

/// Builds the generated package with the native stub and runs its Go tests.
fn run_generated_go_test(binding: &str) -> std::process::Output {
    let library_dir = native_library_dir().expect("generated Go binding compile fixture supports this target");
    let go = which::which("go").expect("Go is required for this fixture");
    let directory = tempfile::tempdir().expect("temporary Go package directory");
    std::fs::create_dir_all(directory.path().join("include")).expect("create include directory");
    std::fs::write(directory.path().join("include/test.h"), STREAM_HEADER).expect("write native header");
    std::fs::write(directory.path().join("binding.go"), binding).expect("write generated binding");
    std::fs::write(directory.path().join("error_test.go"), ERROR_TEST_GO).expect("write error test");
    assert!(
        write_empty_native_archive(directory.path(), library_dir),
        "failed to build the empty native archive"
    );
    std::process::Command::new(go)
        .args(["test", "./...", "-v"])
        .env("GO111MODULE", "off")
        .env("CGO_ENABLED", "1")
        .current_dir(directory.path())
        .output()
        .expect("run go test")
}

#[test]
fn stream_with_err_returns_the_stream_error_that_the_plain_call_drops() {
    let files = GoBackend
        .generate_bindings(&api(), &config())
        .expect("Go bindings generate");
    let binding = &files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated")
        .content;

    // The gate job sets this to compare the generated Go public surface with the base tree.
    if let Some(path) = std::env::var_os("ALEF_GO_SURFACE_OUT") {
        std::fs::write(path, binding).expect("write the generated binding for the surface diff");
    }

    if native_library_dir().is_none() || which::which("go").is_err() {
        eprintln!(
            "skipping stream_with_err_returns_the_stream_error_that_the_plain_call_drops: \
             Go toolchain or a supported native target is unavailable on this host"
        );
        return;
    }

    let output = run_generated_go_test(binding);
    assert!(
        output.status.success(),
        "generated stream error test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
