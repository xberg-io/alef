//! A cancelled or expired `context.Context` must abort the in-flight native call, not merely stop
//! the Go side waiting for it. Go cannot interrupt a blocking cgo call, so the generated
//! `WithContext` methods hand the native call a cancel token and trip it from a watcher goroutine.
//!
//! String assertions on the generated source prove the plumbing is *emitted*; they cannot prove a
//! blocked call is actually released. This test drives the real generated Go package against a C
//! stub whose native calls block until the token they were given is tripped, so a call that is not
//! wired to its token hangs for the stub's full duration and fails the elapsed-time assertions.

#![allow(clippy::print_stderr)]

use alef::backends::go::GoBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::config::{AdapterConfig, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, ErrorDef, ErrorVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
    TypeDef, TypeRef,
};

/// The native stub. `fetch_cancellable` and the stream's second `_next` poll their token every few
/// milliseconds and give up with the reserved `Cancelled` code (5) once it is tripped; the plain
/// `fetch` never observes a token. Counters let the Go test prove every token was freed.
const NATIVE_HEADER: &str = r#"
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef uint64_t TESTEngine;
typedef uint64_t TESTAlefHandle;

static _Thread_local int32_t test_code = 0;
static _Thread_local const char *test_context = "native error";
static atomic_int test_engine_busy = 0;
static atomic_int test_created = 0;
static atomic_int test_freed = 0;
static atomic_int test_tripped = 0;
static atomic_int test_token_state[64];

static inline int32_t test_last_error_code(void) { return test_code; }
static inline const char *test_last_error_context(void) { return test_code ? test_context : NULL; }
static inline const char *test_last_error_variant(void) { return NULL; }

static inline TESTAlefHandle test_cancel_token_new(void) {
    int id = atomic_fetch_add(&test_created, 1) + 1;
    atomic_store(&test_token_state[id], 0);
    return (TESTAlefHandle)id;
}
static inline int32_t test_cancel_token_cancel(TESTAlefHandle token) {
    atomic_store(&test_token_state[token], 1);
    atomic_fetch_add(&test_tripped, 1);
    return 0;
}
static inline void test_cancel_token_free(TESTAlefHandle token) { atomic_fetch_add(&test_freed, 1); }

static inline int test_wait(uint32_t millis, TESTAlefHandle token) {
    for (uint32_t waited = 0; waited < millis; waited += 5) {
        if (token != 0 && atomic_load(&test_token_state[token])) return 1;
        usleep(5000);
    }
    return 0;
}

static inline char *test_dup(const char *text) {
    size_t length = strlen(text) + 1;
    char *copy = (char *)malloc(length);
    if (copy) memcpy(copy, text, length);
    return copy;
}

static inline void test_engine_free(TESTEngine h) {}
static inline void test_free_string(char *s) { free(s); }

static inline char *test_engine_fetch(TESTEngine self, uint32_t millis) {
    test_code = 0;
    test_wait(millis, 0);
    return test_dup("done");
}
/* Queues behind a running call on the same engine the way the real export does: it polls the
 * engine's lock and gives up with Cancelled as soon as its own token trips. */
static inline char *test_engine_fetch_cancellable(TESTEngine self, uint32_t millis, TESTAlefHandle token) {
    test_code = 0;
    while (atomic_exchange(&test_engine_busy, 1)) {
        if (token != 0 && atomic_load(&test_token_state[token])) {
            test_code = 5;
            return NULL;
        }
        usleep(1000);
    }
    int cancelled = test_wait(millis, token);
    atomic_store(&test_engine_busy, 0);
    if (cancelled) {
        test_code = 5;
        return NULL;
    }
    return test_dup("done");
}

/* Converting a request body fails with the FFI's code 2 and the serde message, as a real
 * `_from_json` does for a value the target type does not accept. */
static inline TESTAlefHandle test_submit_request_from_json(const char *json) {
    test_code = 0;
    if (strstr(json, "bogus")) {
        test_code = 2;
        test_context = "unknown variant `bogus`, expected `fast` or `slow`";
        return 0;
    }
    return 7;
}
static inline void test_submit_request_free(TESTAlefHandle request) {}
static inline char *test_engine_submit(TESTEngine self, TESTAlefHandle request) {
    test_code = 0;
    return test_dup("submitted");
}

/* Stream handles are 1000 + the token id they were started with; the first `_next` yields a chunk
 * at once, every later one blocks until the token trips. */
static atomic_int test_stream_reads = 0;
static inline TESTAlefHandle test_engine_crawl_stream_start_cancellable(TESTEngine self, TESTAlefHandle token) {
    test_code = 0;
    atomic_store(&test_stream_reads, 0);
    return 1000 + token;
}
static inline TESTAlefHandle test_engine_crawl_stream_next(TESTAlefHandle handle) {
    test_code = 0;
    if (atomic_fetch_add(&test_stream_reads, 1) == 0) return 1;
    if (test_wait(30000, handle - 1000)) test_code = 5;
    return 0;
}
static inline void test_engine_crawl_stream_free(TESTAlefHandle handle) {}

static inline char *test_crawl_event_to_json(TESTAlefHandle chunk) { return test_dup("{\"message\":\"x\"}"); }
static inline void test_crawl_event_free(TESTAlefHandle chunk) {}

static inline char *test_token_stats(void) {
    char text[96];
    snprintf(text, sizeof(text), "%d %d %d", atomic_load(&test_created), atomic_load(&test_freed), atomic_load(&test_tripped));
    return test_dup(text);
}
"#;

const CONTEXT_TEST_GO: &str = r#"package ctxtest

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"
)

func stats(t *testing.T) (created, freed, tripped int) {
	t.Helper()
	if _, err := fmt.Sscanf(TokenStats(), "%d %d %d", &created, &freed, &tripped); err != nil {
		t.Fatalf("unreadable token stats: %v", err)
	}
	return created, freed, tripped
}

func TestDeadlineAbortsTheBlockedNativeCall(t *testing.T) {
	engine := &Engine{ptr: 1}
	createdBefore, freedBefore, trippedBefore := stats(t)

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	started := time.Now()
	got, err := engine.FetchWithContext(ctx, 5000)
	elapsed := time.Since(started)

	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("expected context.DeadlineExceeded, got %v (value %q)", err, got)
	}
	if elapsed > time.Second {
		t.Fatalf("the native call was not aborted: returned after %v, the stub blocks for 5s", elapsed)
	}
	if got != "" {
		t.Fatalf("a cancelled call must return the zero value, got %q", got)
	}
	created, freed, tripped := stats(t)
	if created-createdBefore != 1 || freed-freedBefore != 1 || tripped-trippedBefore != 1 {
		t.Fatalf("one token must be created, tripped and freed; created=%d freed=%d tripped=%d",
			created-createdBefore, freed-freedBefore, tripped-trippedBefore)
	}
}

func TestCancelFromAnotherGoroutineReportsContextCanceled(t *testing.T) {
	engine := &Engine{ptr: 1}
	ctx, cancel := context.WithCancel(context.Background())
	time.AfterFunc(50*time.Millisecond, cancel)

	started := time.Now()
	_, err := engine.FetchWithContext(ctx, 5000)
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", err)
	}
	if elapsed := time.Since(started); elapsed > time.Second {
		t.Fatalf("returned after %v", elapsed)
	}
}

func TestAlreadyDoneContextNeverStartsTheNativeCall(t *testing.T) {
	engine := &Engine{ptr: 1}
	createdBefore, _, _ := stats(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	if _, err := engine.FetchWithContext(ctx, 5000); !errors.Is(err, context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", err)
	}
	if created, _, _ := stats(t); created != createdBefore {
		t.Fatalf("a context that is already done must not allocate a token (%d -> %d)", createdBefore, created)
	}
}

func TestPlainMethodKeepsItsSignatureAndStillWorks(t *testing.T) {
	engine := &Engine{ptr: 1}
	got, err := engine.Fetch(10)
	if err != nil || got != "done" {
		t.Fatalf("Fetch(10) = %q, %v", got, err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	got, err = engine.FetchWithContext(ctx, 10)
	if err != nil || got != "done" {
		t.Fatalf("FetchWithContext within its deadline = %q, %v", got, err)
	}
}

func TestQueuedCallBehindARunningOneIsCancelledPromptly(t *testing.T) {
	engine := &Engine{ptr: 1}
	runningCtx, stopRunning := context.WithCancel(context.Background())
	defer stopRunning()
	runningDone := make(chan error, 1)
	go func() {
		_, err := engine.FetchWithContext(runningCtx, 5000)
		runningDone <- err
	}()
	// The first call now holds the engine for 5s unless it is cancelled.
	time.Sleep(150 * time.Millisecond)

	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	started := time.Now()
	_, err := engine.FetchWithContext(ctx, 10)
	elapsed := time.Since(started)

	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("expected context.DeadlineExceeded from the queued call, got %v", err)
	}
	if elapsed > time.Second {
		t.Fatalf("the queued call was not cancelled while waiting: returned after %v", elapsed)
	}
	select {
	case err := <-runningDone:
		t.Fatalf("cancelling the queued call must not disturb the running one, but it returned %v", err)
	default:
	}

	stopRunning()
	select {
	case err := <-runningDone:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("expected the running call to report context.Canceled, got %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("the running call did not return after its own cancellation")
	}
	got, err := engine.FetchWithContext(context.Background(), 10)
	if err != nil || got != "done" {
		t.Fatalf("the engine must be usable once both calls ended, got %q, %v", got, err)
	}
}

func TestRequestConversionFailureIsTheTypedNativeError(t *testing.T) {
	engine := &Engine{ptr: 1}
	_, err := engine.Submit(SubmitRequest{Kind: "bogus"})
	if err == nil {
		t.Fatal("expected the conversion failure to be returned")
	}
	var native *Error
	if !errors.As(err, &native) {
		t.Fatalf("expected a *Error reachable through errors.As, got %T: %v", err, err)
	}
	if native.Code != "2" {
		t.Fatalf("the native error code must be preserved, got %q", native.Code)
	}
	if !strings.Contains(err.Error(), "failed to create submit_request") ||
		!strings.Contains(err.Error(), "unknown variant `bogus`") {
		t.Fatalf("the message must name the step and keep the native detail, got %q", err.Error())
	}

	got, err := engine.Submit(SubmitRequest{Kind: "fast"})
	if err != nil || got != "submitted" {
		t.Fatalf("a valid request must still convert and call through, got %q, %v", got, err)
	}
}

func TestStreamCancelAbortsTheBlockedNativeRead(t *testing.T) {
	engine := &Engine{ptr: 1}
	createdBefore, freedBefore, _ := stats(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	stream, err := engine.CrawlStreamWithContext(ctx)
	if err != nil {
		t.Fatalf("start failed: %v", err)
	}
	chunk, ok := <-stream.Chan()
	if !ok || chunk.Message != "x" {
		t.Fatalf("expected the first chunk, got %+v ok=%v", chunk, ok)
	}

	// The goroutine is now parked inside a native read that blocks for 30s unless cancelled.
	cancel()
	closed := make(chan struct{})
	go func() {
		for range stream.Chan() {
		}
		close(closed)
	}()
	select {
	case <-closed:
	case <-time.After(2 * time.Second):
		t.Fatal("cancelling ctx did not release the goroutine blocked in the native stream read")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected stream.Err() to be context.Canceled, got %v", stream.Err())
	}

	deadline := time.Now().Add(2 * time.Second)
	for {
		created, freed, _ := stats(t)
		if created-createdBefore == 1 && freed-freedBefore == 1 {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("the stream's token was not freed: created=%d freed=%d", created-createdBefore, freed-freedBefore)
		}
		time.Sleep(5 * time.Millisecond)
	}
}
"#;

fn native_library_dir() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(".lib/macos-arm64"),
        ("macos", "x86_64") => Some(".lib/macos-x86_64"),
        ("linux", "x86_64") => Some(".lib/linux-x86_64"),
        ("linux", "aarch64") => Some(".lib/linux-aarch64"),
        _ => None,
    }
}

/// The header's functions are all `static inline`, so cgo compiles them into the binding's own
/// translation unit; the linked archive only has to exist to satisfy `-ltest_ffi`.
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
package_name = "ctxtest"
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
                methods: vec![
                    MethodDef {
                        name: "fetch".to_string(),
                        params: vec![ParamDef {
                            name: "millis".to_string(),
                            ty: TypeRef::Primitive(PrimitiveType::U32),
                            ..ParamDef::default()
                        }],
                        return_type: TypeRef::String,
                        is_async: true,
                        error_type: Some("CtxtestError".to_string()),
                        receiver: Some(ReceiverKind::Ref),
                        ..MethodDef::default()
                    },
                    MethodDef {
                        name: "submit".to_string(),
                        params: vec![ParamDef {
                            name: "request".to_string(),
                            ty: TypeRef::Named("SubmitRequest".to_string()),
                            ..ParamDef::default()
                        }],
                        return_type: TypeRef::String,
                        error_type: Some("CtxtestError".to_string()),
                        receiver: Some(ReceiverKind::Ref),
                        ..MethodDef::default()
                    },
                    MethodDef {
                        name: "crawl_stream".to_string(),
                        return_type: TypeRef::Unit,
                        is_async: true,
                        receiver: Some(ReceiverKind::Ref),
                        ..MethodDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "SubmitRequest".to_string(),
                rust_path: "test_lib::SubmitRequest".to_string(),
                has_serde: true,
                fields: vec![FieldDef {
                    name: "kind".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
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
        functions: vec![FunctionDef {
            name: "token_stats".to_string(),
            rust_path: "test_lib::token_stats".to_string(),
            return_type: TypeRef::String,
            ..FunctionDef::default()
        }],
        errors: vec![ErrorDef {
            name: "CtxtestError".to_string(),
            rust_path: "test_lib::CtxtestError".to_string(),
            variants: vec![ErrorVariant {
                name: "Rejected".to_string(),
                ..ErrorVariant::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..ApiSurface::default()
    }
}

fn generated_binding() -> String {
    let files = GoBackend
        .generate_bindings(&api(), &config())
        .expect("Go bindings generate");
    files
        .iter()
        .find(|file| file.path.ends_with("binding.go"))
        .expect("binding.go is generated")
        .content
        .clone()
}

fn run_go_tests(binding: &str, test_source: &str) -> std::process::Output {
    let library_dir = native_library_dir().expect("generated Go binding compile fixture supports this target");
    let go = which::which("go").expect("Go is required for this fixture");
    let directory = tempfile::tempdir().expect("temporary Go package directory");
    std::fs::create_dir_all(directory.path().join("include")).expect("create include directory");
    std::fs::write(directory.path().join("include/test.h"), NATIVE_HEADER).expect("write native header");
    std::fs::write(directory.path().join("binding.go"), binding).expect("write generated binding");
    std::fs::write(directory.path().join("context_test.go"), test_source).expect("write context test");
    assert!(
        write_empty_native_archive(directory.path(), library_dir),
        "failed to build the empty native archive"
    );
    std::process::Command::new(go)
        .args(["test", "./...", "-count=1", "-v"])
        .env("GO111MODULE", "off")
        .env("CGO_ENABLED", "1")
        .current_dir(directory.path())
        .output()
        .expect("run go test")
}

fn toolchain_available(test: &str) -> bool {
    if native_library_dir().is_none() || which::which("go").is_err() {
        eprintln!("skipping {test}: Go toolchain or a supported native target is unavailable on this host");
        return false;
    }
    true
}

#[test]
fn context_cancellation_aborts_blocked_native_calls_and_streams() {
    if !toolchain_available("context_cancellation_aborts_blocked_native_calls_and_streams") {
        return;
    }
    let output = run_go_tests(&generated_binding(), CONTEXT_TEST_GO);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "generated context cancellation tests failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for test in [
        "TestDeadlineAbortsTheBlockedNativeCall",
        "TestCancelFromAnotherGoroutineReportsContextCanceled",
        "TestAlreadyDoneContextNeverStartsTheNativeCall",
        "TestPlainMethodKeepsItsSignatureAndStillWorks",
        "TestQueuedCallBehindARunningOneIsCancelledPromptly",
        "TestRequestConversionFailureIsTheTypedNativeError",
        "TestStreamCancelAbortsTheBlockedNativeRead",
    ] {
        assert!(
            stdout.contains(&format!("--- PASS: {test}")),
            "{test} did not run and pass; a zero-test pass proves nothing:\n{stdout}"
        );
    }
}
