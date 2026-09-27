//! Issue #448: on an unbuffered channel, a consumer that `break`s out of
//! `for chunk := range stream.Chan()` early parks the forwarding goroutine forever on
//! `ch <- chunk`, since nothing is left to receive. That leaks both the goroutine and the
//! native stream handle -- the deferred `close(ch)`/`C.<fn_free>(handle)` never run. The fix
//! races the send against `<-ctx.Done()` so a cancelled context can unblock the goroutine even
//! with no receiver.
//!
//! A string assertion on the generated source (see
//! `src/backends/go/gen_bindings/methods/streaming_error_paths_tests.rs`) proves the `select`
//! is *emitted*; it cannot prove the goroutine actually *exits*. This test drives the real
//! generated Go package end to end with a working native stub (not the empty archive
//! `tests/backends_go_adapter_owner_type_contract.rs` links against) so the forwarding
//! goroutine really runs, really blocks, and really gets unblocked by cancellation --
//! `runtime.NumGoroutine()` before/after is the proof the issue asks for, not just that
//! `Close()`/a `select` is present in the source.

#![allow(clippy::print_stderr)]

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

/// A working (not empty-stub) native implementation: `_next` never naturally ends the stream
/// (always returns a live handle), so the ONLY way the Go test's stream ever stops is the
/// context cancellation under test -- an empty/short-lived native stream would let the old,
/// buggy bare-send code look correct by accident (the goroutine would exit on its own once the
/// native side ran out of items, whether or not the send raced ctx.Done()).
const STREAM_HEADER: &str = r#"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t TESTEngine;
typedef uint64_t TESTAlefHandle;

static inline int32_t test_last_error_code(void) { return 0; }
static inline const char *test_last_error_context(void) { return NULL; }

static inline void test_engine_free(TESTEngine h) {}

static inline TESTAlefHandle test_engine_crawl_stream_start(TESTEngine self) { return 1; }
/* Never returns 0 (end-of-stream): the fixture's stream has no natural end, so only ctx
 * cancellation can stop the Go-side forwarding goroutine. */
static inline TESTAlefHandle test_engine_crawl_stream_next(TESTAlefHandle handle) { return 1; }
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

/// The Go test file exercising the generated binding. Lives in the SAME package as `binding.go`
/// (see `render`) so it can construct an `Engine` directly (`ptr` is unexported) without a
/// generated constructor -- this fixture's `Engine` has none, by design, to keep the fixture IR
/// minimal.
const LEAK_TEST_GO: &str = r#"package streamtest

import (
	"context"
	"errors"
	"runtime"
	"testing"
	"time"
)

// TestStreamingCancelDoesNotLeakGoroutine reproduces issue #448: break out of the range loop
// early (the exact shape the issue names), then cancel. The forwarding goroutine must exit and
// runtime.NumGoroutine() must return to its pre-stream baseline -- not just "the select exists
// in the source", which a text-only check cannot tell apart from a select that never fires.
func TestStreamingCancelDoesNotLeakGoroutine(t *testing.T) {
	runtime.GC()
	baseline := runtime.NumGoroutine()

	engine := &Engine{ptr: 1}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	stream, err := engine.CrawlStream(ctx)
	if err != nil {
		t.Fatalf("start failed: %v", err)
	}

	consumed := 0
	for chunk := range stream.Chan() {
		if chunk.Message != "x" {
			t.Fatalf("unexpected chunk: %+v", chunk)
		}
		consumed++
		if consumed == 2 {
			// The bug: breaking here, with nobody left to receive, used to park the
			// forwarding goroutine on `ch <- chunk` forever.
			break
		}
	}
	if consumed != 2 {
		t.Fatalf("expected to consume 2 chunks before breaking, got %d", consumed)
	}

	cancel()

	// The goroutine-exit proof the issue asks for: poll NumGoroutine with a bounded retry
	// (never a fixed sleep -- the scheduler is not instantaneous) instead of asserting once
	// immediately after cancel.
	deadline := time.Now().Add(5 * time.Second)
	for {
		if runtime.NumGoroutine() <= baseline {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf(
				"goroutine leak: NumGoroutine() did not return to baseline %d within 5s, still %d",
				baseline, runtime.NumGoroutine(),
			)
		}
		runtime.Gosched()
	}

	// Secondary check, bounded so a regression here reports its OWN failure instead of hanging
	// the whole test: per Chan()'s documented discipline, draining to closure after cancelling
	// must still surface ctx.Err() through Err() -- cancellation unblocks the goroutine, it does
	// not publish err by itself, only the channel close does.
	drained := make(chan error, 1)
	go func() {
		for range stream.Chan() {
		}
		drained <- stream.Err()
	}()
	select {
	case err := <-drained:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("expected stream.Err() to be context.Canceled after cancelling, got %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatalf("stream.Chan() did not close within 5s of cancelling ctx")
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

/// Runs the generated package's own test suite (not just `go build`) with a real native stub, so
/// the assertions above actually execute. Returns the process `Output` for the caller to inspect
/// pass/fail -- unlike the build-only fixtures elsewhere, this test's whole point is to be run
/// twice (once against the fix, once against a deliberately reverted `select`), so the exact
/// failure text matters to the person doing that comparison.
fn run_generated_go_test(binding: &str) -> std::process::Output {
    let library_dir = native_library_dir().expect("generated Go binding compile fixture supports this target");
    let go = which::which("go").expect("Go is required for this fixture");
    let directory = tempfile::tempdir().expect("temporary Go package directory");
    std::fs::create_dir_all(directory.path().join("include")).expect("create include directory");
    std::fs::write(directory.path().join("include/test.h"), STREAM_HEADER).expect("write native header");
    std::fs::write(directory.path().join("binding.go"), binding).expect("write generated binding");
    std::fs::write(directory.path().join("leak_test.go"), LEAK_TEST_GO).expect("write leak test");
    assert!(
        write_empty_native_archive(directory.path(), library_dir),
        "failed to build the empty native archive"
    );
    std::process::Command::new(go)
        .args(["test", "./...", "-run", "TestStreamingCancelDoesNotLeakGoroutine", "-v"])
        .env("GO111MODULE", "off")
        .env("CGO_ENABLED", "1")
        .current_dir(directory.path())
        .output()
        .expect("run go test")
}

#[test]
fn streaming_cancel_after_early_break_does_not_leak_the_forwarding_goroutine() {
    let files = GoBackend
        .generate_bindings(&api(), &config())
        .expect("Go bindings generate");
    let binding = &files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated")
        .content;

    // Fails closed (skip, not a silent pass) when the toolchain is unavailable, matching
    // `tests/backends_go_adapter_owner_type_contract.rs` -- but the skip must be visible.
    if native_library_dir().is_none() || which::which("go").is_err() {
        eprintln!(
            "skipping streaming_cancel_after_early_break_does_not_leak_the_forwarding_goroutine: \
             Go toolchain or a supported native target is unavailable on this host"
        );
        return;
    }

    let output = run_generated_go_test(binding);
    assert!(
        output.status.success(),
        "generated streaming cancellation test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
