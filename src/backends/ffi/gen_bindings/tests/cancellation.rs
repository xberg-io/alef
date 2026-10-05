//! Cancellation across the C ABI: every export that blocks on a future has a `_cancellable`
//! sibling taking a trailing cancel-token handle, and the token API aborts the blocked call.

use super::common::resolved_one;
use crate::backends::ffi::gen_bindings::helpers::gen_ffi_cancel_token;
use crate::backends::ffi::template_env;
use crate::core::backend::Backend;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

fn config(with_streaming_adapter: bool) -> crate::core::config::ResolvedCrateConfig {
    let base = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "ml"
"#;
    let adapter = r#"
[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "chat_stream"
owner_type = "DefaultClient"
item_type = "ChatChunk"
error_type = "MyError"
request_type = "my_lib::ChatRequest"

[[crates.adapters.params]]
name = "req"
type = "ChatRequest"
"#;
    resolved_one(&if with_streaming_adapter {
        format!("{base}{adapter}")
    } else {
        base.to_string()
    })
}

fn async_api(with_async: bool) -> ApiSurface {
    let method = |name: &str, is_async: bool| MethodDef {
        name: name.to_string(),
        params: vec![ParamDef {
            name: "count".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        is_async,
        error_type: Some("MyError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "DefaultClient".to_string(),
            rust_path: "my_lib::DefaultClient".to_string(),
            is_opaque: true,
            methods: vec![method("fetch", with_async), method("peek", false)],
            ..TypeDef::default()
        }],
        functions: vec![FunctionDef {
            name: "download".to_string(),
            rust_path: "my_lib::download".to_string(),
            return_type: TypeRef::Unit,
            is_async: with_async,
            error_type: Some("MyError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn generated_lib(api: &ApiSurface, with_streaming_adapter: bool) -> String {
    let files = crate::backends::ffi::FfiBackend
        .generate_bindings(api, &config(with_streaming_adapter))
        .unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();
    syn::parse_file(&lib.content).expect("generated FFI crate must parse as Rust");
    lib.content.clone()
}

fn signature_of<'a>(source: &'a str, symbol: &str) -> &'a str {
    let marker = format!("pub unsafe extern \"C\" fn {symbol}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("`{symbol}` must be exported, got:\n{source}"));
    let rest = &source[start..];
    &rest[..rest.find(") ->").expect("signature must end with a return type")]
}

#[test]
fn async_exports_gain_a_cancellable_sibling_with_a_trailing_token_and_the_primary_is_unchanged() {
    let source = generated_lib(&async_api(true), false);

    let primary = signature_of(&source, "ml_default_client_fetch");
    let cancellable = signature_of(&source, "ml_default_client_fetch_cancellable");
    assert!(!primary.contains("alef_cancel_token"), "{primary}");
    assert_eq!(
        cancellable
            .replace(",\n    alef_cancel_token: AlefHandle", "")
            .replace("_cancellable", ""),
        primary,
        "the sibling must be the primary's exact signature plus one trailing token"
    );
    assert!(
        cancellable.trim_end().ends_with("alef_cancel_token: AlefHandle"),
        "{cancellable}"
    );

    let free_primary = signature_of(&source, "ml_download");
    let free_cancellable = signature_of(&source, "ml_download_cancellable");
    assert!(!free_primary.contains("alef_cancel_token"), "{free_primary}");
    assert!(
        free_cancellable.contains("alef_cancel_token: AlefHandle"),
        "{free_cancellable}"
    );

    assert!(
        source.contains("alef_block_on_cancellable(alef_cancel_token, async { obj.fetch(count_rs).await })"),
        "the method sibling must race its future against the token:\n{source}"
    );
    assert!(
        source.contains("alef_block_on_cancellable(alef_cancel_token, async { my_lib::download().await })"),
        "the function sibling must race its future against the token:\n{source}"
    );
    assert!(
        source.contains("get_ffi_runtime().block_on(async { obj.fetch(count_rs).await })"),
        "the primary export keeps its uncancellable block_on:\n{source}"
    );

    for symbol in ["ml_cancel_token_new", "ml_cancel_token_cancel", "ml_cancel_token_free"] {
        signature_of(&source, symbol);
    }
    assert!(source.contains("Cancelled = 5"), "{source}");
}

#[test]
fn sync_exports_get_no_cancellable_sibling_and_a_crate_without_async_gets_no_token_api() {
    let source = generated_lib(&async_api(true), false);
    assert!(!source.contains("fn ml_default_client_peek_cancellable"), "{source}");

    let source = generated_lib(&async_api(false), false);
    assert!(!source.contains("_cancellable"), "{source}");
    assert!(!source.contains("cancel_token"), "{source}");
    assert!(!source.contains("AlefCancelState"), "{source}");
}

#[test]
fn streaming_adapter_gains_a_cancellable_start_that_the_handle_remembers() {
    let source = generated_lib(&async_api(false), true);

    let start = signature_of(&source, "ml_default_client_chat_stream_start_cancellable");
    assert!(start.contains("alef_cancel_token: AlefHandle"), "{start}");
    assert!(
        !signature_of(&source, "ml_default_client_chat_stream_start").contains("alef_cancel_token"),
        "the primary start keeps its signature"
    );
    assert!(
        source.contains("alef_block_on_in(&h.rt, h.cancel.as_ref(), stream.next())"),
        "the blocking read must observe the token stored on the handle:\n{source}"
    );
}

/// The harness stands a thread-parking executor in for tokio, so the rendered token module is
/// compiled and run exactly as generated while `rustc` needs no dependency graph. A future that
/// never completes can only be unblocked by the token's own wake-up, so a missed wake-up hangs
/// the harness until its watchdog kills it.
fn run_harness(name: &str, main_body: &str) {
    let last_error = template_env::render(
        "last_error.jinja",
        minijinja::context! {
            prefix => "smp",
            builtin_prefix => crate::codegen::naming::ffi_builtin_error_code_prefix("smp"),
            error_code_impls => Vec::<String>::new(),
            has_error_code_impls => false,
            taxonomy => Vec::<String>::new(),
            no_error_code => 0,
            conversion_error_code => 1,
            unknown_error_code => 2,
            panic_error_code => 3,
            invalid_handle_error_code => 4,
            cancelled_error_code => 5,
        },
    );
    let mut registry = template_env::render("handle_registry.rs.jinja", minijinja::context! {});
    let start = registry
        .find("struct SerializedHandle")
        .expect("serialized helper start");
    let resume = registry[start..]
        .find("fn with_handle")
        .map(|offset| start + offset)
        .expect("core registry helpers resume");
    registry.replace_range(start..resume, "");
    let token_module = gen_ffi_cancel_token("smp");

    let source = format!(
        r#"
use std::cell::RefCell;
use std::ffi::{{c_char, CString}};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{{AtomicUsize, Ordering}};
use std::sync::Arc;
use std::task::{{Context, Poll}};
use std::time::Duration;

mod tokio {{
    pub mod runtime {{
        pub struct Runtime;
        impl Runtime {{
            pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {{
                struct Unparker(std::thread::Thread);
                impl std::task::Wake for Unparker {{
                    fn wake(self: std::sync::Arc<Self>) {{ self.0.unpark(); }}
                }}
                let waker = std::task::Waker::from(std::sync::Arc::new(Unparker(std::thread::current())));
                let mut cx = std::task::Context::from_waker(&waker);
                let mut future = std::pin::pin!(future);
                loop {{
                    if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut cx) {{
                        return output;
                    }}
                    std::thread::park();
                }}
            }}
        }}
    }}
}}

fn get_ffi_runtime() -> &'static tokio::runtime::Runtime {{
    static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Runtime;
    &RUNTIME
}}

{last_error}
{registry}
{token_module}

struct Pending {{
    polls: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}}
impl Pending {{
    fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {{
        let polls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        (Self {{ polls: polls.clone(), dropped: dropped.clone() }}, polls, dropped)
    }}
}}
impl Future for Pending {{
    type Output = u32;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<u32> {{
        self.polls.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }}
}}
impl Drop for Pending {{
    fn drop(&mut self) {{ self.dropped.fetch_add(1, Ordering::SeqCst); }}
}}

fn error_code() -> i32 {{ unsafe {{ smp_last_error_code() }} }}

fn waiter_count(token: AlefHandle) -> usize {{
    with_handle::<AlefCancelToken, _>(token, |token| token.0.waiters.lock().unwrap().len()).unwrap()
}}

fn main() {{
    std::thread::spawn(|| {{
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("watchdog: a blocked call was never woken by the token");
        std::process::exit(101);
    }});
    {main_body}
}}
"#
    );

    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join(format!("{name}.rs"));
    let binary_path = directory.path().join(name);
    std::fs::write(&source_path, &source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        compile.status.success(),
        "{}\n---source---\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(
        run.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn tripping_a_token_aborts_a_pending_future_drops_it_and_reports_cancelled() {
    run_harness(
        "cancel-pending",
        r#"
    let token = unsafe { smp_cancel_token_new() };
    assert_ne!(token, 0);
    let (future, polls, dropped) = Pending::new();

    let worker = std::thread::spawn(move || {
        let outcome = alef_block_on_cancellable(token, future);
        (outcome, error_code())
    });
    while polls.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(dropped.load(Ordering::SeqCst), 0, "the future is still pending, so it is still alive");
    assert_eq!(unsafe { smp_cancel_token_cancel(token) }, 0);

    let (outcome, code) = worker.join().unwrap();
    assert_eq!(outcome, None, "cancellation yields no output");
    assert_eq!(code, 5, "the failure must be the reserved Cancelled code");
    assert_eq!(dropped.load(Ordering::SeqCst), 1, "the in-flight future must be dropped on cancel");
    assert_eq!(waiter_count(token), 0, "a finished call must not leave its waker registered");
    unsafe { smp_cancel_token_free(token) };
    "#,
    );
}

#[test]
fn a_null_token_never_cancels_and_a_cancelled_token_refuses_to_start() {
    run_harness(
        "cancel-null-and-precancelled",
        r#"
    assert_eq!(alef_block_on_cancellable(0, async { 7_u32 }), Some(7));
    assert_eq!(error_code(), 0, "an uncancelled call leaves the error slot clear");

    let token = unsafe { smp_cancel_token_new() };
    assert_eq!(unsafe { smp_cancel_token_cancel(token) }, 0);
    assert_eq!(unsafe { smp_cancel_token_cancel(token) }, 0, "cancelling twice is harmless");
    let (future, polls, dropped) = Pending::new();
    assert_eq!(alef_block_on_cancellable(token, future), None);
    assert_eq!(error_code(), 5);
    assert_eq!(polls.load(Ordering::SeqCst), 0, "an already-cancelled token must not start the work");
    assert_eq!(dropped.load(Ordering::SeqCst), 1);

    assert_eq!(alef_block_on_cancellable(token, async { 1_u32 }), None, "a cancelled token stays cancelled");

    let fresh = unsafe { smp_cancel_token_new() };
    assert_eq!(alef_block_on_cancellable(fresh, async { 9_u32 }), Some(9), "an untripped token lets the call finish");
    assert_eq!(waiter_count(fresh), 0);
    unsafe {
        smp_cancel_token_free(fresh);
        smp_cancel_token_free(token);
        smp_cancel_token_free(0);
    }
    "#,
    );
}

#[test]
fn one_token_cancels_every_call_sharing_it() {
    run_harness(
        "cancel-shared",
        r#"
    let token = unsafe { smp_cancel_token_new() };
    let mut workers = Vec::new();
    let mut ready = Vec::new();
    for _ in 0..4 {
        let (future, polls, dropped) = Pending::new();
        ready.push((polls, dropped));
        workers.push(std::thread::spawn(move || (alef_block_on_cancellable(token, future), error_code())));
    }
    for (polls, _) in &ready {
        while polls.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(waiter_count(token), 4);
    assert_eq!(unsafe { smp_cancel_token_cancel(token) }, 0);
    for worker in workers {
        assert_eq!(worker.join().unwrap(), (None, 5));
    }
    for (_, dropped) in &ready {
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
    unsafe { smp_cancel_token_free(token) };
    "#,
    );
}

#[test]
fn a_freed_or_forged_token_is_an_invalid_handle_error_and_never_silently_uncancellable() {
    run_harness(
        "cancel-invalid",
        r#"
    let token = unsafe { smp_cancel_token_new() };
    unsafe { smp_cancel_token_free(token) };

    let (future, polls, _) = Pending::new();
    assert_eq!(alef_block_on_cancellable(token, future), None, "a stale token must fail the call, not run it unguarded");
    assert_eq!(error_code(), 4, "a bad token is an invalid-handle error, not a cancellation");
    assert_eq!(polls.load(Ordering::SeqCst), 0);

    assert_eq!(unsafe { smp_cancel_token_cancel(token) }, -1);
    assert_eq!(error_code(), 4);
    assert_eq!(unsafe { smp_cancel_token_cancel(u64::MAX) }, -1);

    unsafe { smp_cancel_token_free(token) };
    assert_eq!(error_code(), 4, "freeing twice reports the stale handle");
    "#,
    );
}
