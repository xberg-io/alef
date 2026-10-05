use super::super::functions::gen_function_wrapper;
use super::super::methods::gen_method_wrapper;
use super::*;
use crate::core::ir::{FunctionDef, MethodDef, PrimitiveType, TypeDef};
use std::collections::HashSet;

fn client() -> TypeDef {
    TypeDef {
        name: "Client".to_string(),
        rust_path: "demo::Client".to_string(),
        is_opaque: true,
        ..TypeDef::default()
    }
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..ParamDef::default()
    }
}

fn async_method(name: &str, params: Vec<ParamDef>, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async: true,
        error_type: Some("DemoError".to_string()),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

fn render_method(method: &MethodDef) -> String {
    let empty_refs: HashSet<&str> = HashSet::new();
    let empty: HashSet<String> = HashSet::new();
    gen_method_wrapper(&client(), method, "demo", &empty_refs, &empty, &empty, &empty)
}

fn render_function(func: &FunctionDef) -> String {
    let empty_refs: HashSet<&str> = HashSet::new();
    let empty: HashSet<String> = HashSet::new();
    gen_function_wrapper(
        func,
        "demo",
        &empty_refs,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
    )
}

fn position(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not found in:\n{haystack}"))
}

#[test]
fn async_method_gets_a_with_context_variant_and_a_forwarding_plain_wrapper() {
    let out = render_method(&async_method(
        "fetch",
        vec![param("millis", TypeRef::Primitive(PrimitiveType::U32))],
        TypeRef::String,
    ));

    assert!(
        out.contains(
            "func (h *Client) Fetch(millis uint32) (string, error) {\n\treturn h.FetchWithContext(context.Background(), millis)\n}"
        ),
        "the plain wrapper keeps its signature and forwards with a background context:\n{out}"
    );
    assert!(
        out.contains("func (h *Client) FetchWithContext(ctx context.Context, millis uint32) (string, error) {"),
        "{out}"
    );
    assert_eq!(out.matches("func (h *Client) Fetch(").count(), 1, "{out}");
    assert_eq!(
        out.matches("runtime.LockOSThread()").count(),
        1,
        "only the worker locks a thread:\n{out}"
    );

    let fast_path = position(&out, "if err := ctx.Err(); err != nil {\n\t\treturn \"\", err\n\t}");
    let token = position(&out, "cancelToken := C.demo_cancel_token_new()");
    let watcher = position(
        &out,
        "case <-ctx.Done():\n\t\t\tC.demo_cancel_token_cancel(cancelToken)",
    );
    let call = position(&out, "C.demo_client_fetch_cancellable(h.ptr, cMillis, cancelToken)");
    assert!(
        fast_path < token && token < watcher && watcher < call,
        "an already-done ctx must return before any token exists, and the watcher must run before the call:\n{out}"
    );

    let stop_watcher = position(
        &out,
        "close(cancelDone)\n\t\t<-cancelStopped\n\t\tC.demo_cancel_token_free(cancelToken)",
    );
    assert!(
        stop_watcher < call,
        "the cleanup is deferred, so it is registered before the call, and it stops the watcher before freeing the token:\n{out}"
    );
    assert!(out.contains("lastErrorContext(ctx)"), "{out}");
    assert_eq!(
        out.matches("lastError()").count(),
        1,
        "the only plain read is the token-allocation failure, which is not a cancellation; every \
         read after the native call must go through the context-aware helper:\n{out}"
    );
    assert!(
        position(&out, "lastError()") < position(&out, "cancelDone := make"),
        "the plain read must be the one guarding token creation:\n{out}"
    );
    assert!(
        !out.contains("C.demo_client_fetch("),
        "the uncancellable symbol must not be called:\n{out}"
    );
}

#[test]
fn bytes_results_and_unit_results_pass_the_token_last() {
    let bytes = render_method(&async_method("download", vec![], TypeRef::Bytes));
    assert!(
        bytes.contains("C.demo_client_download_cancellable(h.ptr, &outPtr, &outLen, &outCap, cancelToken)"),
        "the token follows the out-params, matching the FFI parameter order:\n{bytes}"
    );
    assert!(bytes.contains("return nil, lastErrorContext(ctx)"), "{bytes}");

    let unit = render_method(&async_method("close", vec![], TypeRef::Unit));
    assert!(
        unit.contains("C.demo_client_close_cancellable(h.ptr, cancelToken)"),
        "a zero-argument call must not gain a leading comma:\n{unit}"
    );
    assert!(unit.contains("return lastErrorContext(ctx)"), "{unit}");
    assert!(
        unit.contains("func (h *Client) CloseWithContext(ctx context.Context) error {"),
        "{unit}"
    );
    assert!(
        unit.contains("\t\treturn err\n\t}\n\tcancelToken"),
        "a unit result returns the bare error:\n{unit}"
    );
}

#[test]
fn static_async_method_and_async_function_get_variants_too() {
    let mut method = async_method("connect", vec![], TypeRef::String);
    method.is_static = true;
    method.receiver = None;
    let out = render_method(&method);
    assert!(
        out.contains("return ClientConnectWithContext(context.Background())"),
        "{out}"
    );
    assert!(
        out.contains("func ClientConnectWithContext(ctx context.Context) (string, error) {"),
        "{out}"
    );
    assert!(out.contains("C.demo_client_connect_cancellable(cancelToken)"), "{out}");

    let function = FunctionDef {
        name: "download".to_string(),
        params: vec![param("url", TypeRef::String)],
        return_type: TypeRef::String,
        is_async: true,
        error_type: Some("DemoError".to_string()),
        ..FunctionDef::default()
    };
    let out = render_function(&function);
    assert!(
        out.contains("return DownloadWithContext(context.Background(), url)"),
        "{out}"
    );
    assert!(
        out.contains("func DownloadWithContext(ctx context.Context, url string) (string, error) {"),
        "{out}"
    );
    assert!(out.contains("C.demo_download_cancellable(curl, cancelToken)"), "{out}");
    assert_eq!(out.matches("lastError()").count(), 1, "{out}");
}

#[test]
fn sync_calls_without_an_error_return_and_shadowing_params_are_left_alone() {
    let mut sync = async_method("peek", vec![], TypeRef::String);
    sync.is_async = false;
    let out = render_method(&sync);
    assert!(!out.contains("WithContext") && !out.contains("cancel"), "{out}");
    assert!(out.contains("C.demo_client_peek(h.ptr)"), "{out}");

    let mut infallible = async_method("tick", vec![], TypeRef::Primitive(PrimitiveType::U32));
    infallible.error_type = None;
    let out = render_method(&infallible);
    assert!(
        !out.contains("WithContext"),
        "a signature with no error return has nothing to report a cancellation through:\n{out}"
    );
    assert!(out.contains("C.demo_client_tick(h.ptr)"), "{out}");

    for shadowing in ["ctx", "cancel_token", "cancel_done", "cancel_stopped"] {
        let out = render_method(&async_method(
            "fetch",
            vec![param(shadowing, TypeRef::String)],
            TypeRef::String,
        ));
        assert!(
            !out.contains("WithContext"),
            "a parameter named `{shadowing}` would collide with a local the variant declares:\n{out}"
        );
    }
}

#[test]
fn optional_scalar_results_keep_the_uncancellable_presence_path() {
    let out = render_method(&async_method(
        "limit",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
    ));
    assert!(
        !out.contains("WithContext"),
        "the presence companion re-runs the call without a token, so no variant may claim to cancel it:\n{out}"
    );
}

#[test]
fn token_arg_is_appended_after_the_last_argument_only() {
    assert_eq!(with_token_arg("C.f()"), "C.f(cancelToken)");
    assert_eq!(with_token_arg("C.f(a)"), "C.f(a, cancelToken)");
    assert_eq!(
        with_token_arg("C.f(a, &outPtr, &outLen, &outCap)"),
        "C.f(a, &outPtr, &outLen, &outCap, cancelToken)"
    );
    assert_eq!(with_token_arg("C.f(g(x))"), "C.f(g(x), cancelToken)");
}

/// Every cancel-token or `_cancellable` symbol the generated Go calls must be one the FFI backend
/// exports under that exact spelling; Go referencing a symbol the FFI crate never exported is a
/// link error in consumer code.
#[test]
fn every_cancellation_symbol_go_calls_is_exported_by_the_ffi_backend() {
    use crate::core::backend::Backend;

    let config: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            methods: vec![
                async_method(
                    "fetch",
                    vec![param("millis", TypeRef::Primitive(PrimitiveType::U32))],
                    TypeRef::String,
                ),
                async_method("download", vec![], TypeRef::Bytes),
            ],
            ..client()
        }],
        functions: vec![FunctionDef {
            name: "fetch_all".to_string(),
            rust_path: "demo::fetch_all".to_string(),
            return_type: TypeRef::String,
            is_async: true,
            error_type: Some("DemoError".to_string()),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let go = crate::backends::go::GoBackend.generate_bindings(&api, &config).unwrap();
    let go = &go.iter().find(|f| f.path.ends_with("binding.go")).unwrap().content;
    let ffi = crate::backends::ffi::FfiBackend
        .generate_bindings(&api, &config)
        .unwrap();
    let ffi = &ffi.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;

    let called: Vec<&str> = go
        .split("C.demo_")
        .skip(1)
        .map(|rest| rest.split('(').next().unwrap())
        .filter(|symbol| symbol.ends_with("_cancellable") || symbol.starts_with("cancel_token_"))
        .collect();
    assert!(
        called.len() >= 6,
        "expected the three token calls plus the three cancellable exports, got {called:?}"
    );
    for symbol in called {
        assert!(
            ffi.contains(&format!("pub unsafe extern \"C\" fn demo_{symbol}(")),
            "Go calls `demo_{symbol}` but the FFI backend does not export it"
        );
    }
}

#[test]
fn context_helper_and_import_are_emitted_only_for_packages_with_a_cancellable_call() {
    use crate::core::backend::Backend;

    let config: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "go"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "demo"

[crates.go]
module = "example.invalid/demo"
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let mut method = async_method("fetch", vec![], TypeRef::String);
    let make_api = |method: &MethodDef| ApiSurface {
        crate_name: "demo".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            methods: vec![method.clone()],
            ..client()
        }],
        ..ApiSurface::default()
    };
    let binding = |api: &ApiSurface| {
        let files = crate::backends::go::GoBackend.generate_bindings(api, &config).unwrap();
        files
            .iter()
            .find(|f| f.path.ends_with("binding.go"))
            .unwrap()
            .content
            .clone()
    };

    let with = binding(&make_api(&method));
    assert!(
        with.contains("func lastErrorContext(ctx context.Context) error {"),
        "{with}"
    );
    assert!(
        with.contains("int32(C.demo_last_error_code()) == 5"),
        "the helper keys on the reserved Cancelled code:\n{with}"
    );
    assert!(with.contains("\"context\""), "{with}");

    method.is_async = false;
    let without = binding(&make_api(&method));
    assert!(!without.contains("lastErrorContext"), "{without}");
    assert!(!without.contains("\"context\""), "{without}");
}
