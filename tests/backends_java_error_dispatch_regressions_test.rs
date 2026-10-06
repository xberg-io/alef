#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, FunctionDef};
use support::{compile_java, extract_java_method, java_available, run_java, test_config, write_file};

const TYPED_ERROR_CODE: u32 = 1000;
const LAST_ERROR_EXCEPTION: &str = "static TestLibRsException lastErrorException(final int errCode, final String msg)";

#[test]
fn generated_error_dispatch_matches_infrastructure_taxonomy_and_guards_null_context() {
    let (source, native_lib, typed_code) = error_sources();
    let helper = extract_java_method(&source, "private static void checkLastError()");
    assert!(
        helper.contains("throw NativeLib.lastErrorException(errCode, msg);"),
        "{helper}"
    );
    let guard = helper
        .find("ctxPtr.equals(MemorySegment.NULL)")
        .expect("null context guard");
    let reinterpret = helper.find("ctxPtr.reinterpret").expect("context read");
    assert!(guard < reinterpret, "{helper}");

    let dispatch = extract_java_method(&native_lib, LAST_ERROR_EXCEPTION);
    for (code, class) in [
        (1, "ConversionErrorException"),
        (2, "CoreErrorException"),
        (3, "PanicException"),
        (5, "OperationCancelledException"),
        (typed_code, "RejectedException"),
    ] {
        assert!(
            dispatch.contains(&format!("case {code} -> new {class}(msg);")),
            "{dispatch}"
        );
    }
}

#[test]
fn generated_error_dispatch_runtime_uses_canonical_codes_and_typed_taxonomy() {
    if !java_available() {
        return;
    }
    let (source, native_lib, typed_code) = error_sources();
    let helper = extract_java_method(&source, "private static void checkLastError()");
    let dispatch = extract_java_method(&native_lib, LAST_ERROR_EXCEPTION);
    let probe = format!(
        "package com.test;\nimport java.lang.foreign.MemorySegment;\nfinal class ErrorDispatchProbe {{\n{helper}\nstatic void runCheck() throws Throwable {{ checkLastError(); }}\n}}\n"
    );
    let directory = tempfile::tempdir().expect("temporary error dispatch directory");
    write_error_sources(directory.path(), &probe, &dispatch, typed_code);
    compile_java(
        directory.path(),
        &[
            "com/test/NativeLib.java",
            "com/test/Errors.java",
            "com/test/ErrorDispatchProbe.java",
            "com/test/ErrorDispatchMain.java",
        ],
    );
    run_java(directory.path(), "com.test.ErrorDispatchMain");
}

fn write_error_sources(directory: &std::path::Path, probe: &str, dispatch: &str, typed_code: u32) {
    write_file(directory, "com/test/ErrorDispatchProbe.java", probe);
    let native_lib =
        include_str!("fixtures/java_error_dispatch_native_lib.java").replace("LAST_ERROR_EXCEPTION", dispatch);
    write_file(directory, "com/test/NativeLib.java", &native_lib);
    write_file(
        directory,
        "com/test/Errors.java",
        include_str!("fixtures/java_error_dispatch_errors.java"),
    );
    let main = include_str!("fixtures/java_error_dispatch_main.java").replace("TYPED_CODE", &typed_code.to_string());
    write_file(directory, "com/test/ErrorDispatchMain.java", &main);
}

fn error_sources() -> (String, String, u32) {
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        functions: vec![FunctionDef {
            name: "fallible".into(),
            rust_path: "test_lib::fallible".into(),
            error_type: Some("RequestError".into()),
            ..Default::default()
        }],
        errors: vec![ErrorDef {
            name: "RequestError".into(),
            rust_path: "test_lib::RequestError".into(),
            variants: vec![ErrorVariant {
                name: "Rejected".into(),
                error_code: Some(TYPED_ERROR_CODE),
                is_unit: true,
                ..Default::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        ..Default::default()
    };
    let files = JavaBackend
        .generate_bindings(&api, &test_config())
        .expect("Java error generation");
    let facade = files
        .iter()
        .find(|file| file.content.contains("checkLastError()"))
        .expect("generated error facade")
        .content
        .clone();
    let native_lib = files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("NativeLib.java"))
        .expect("generated NativeLib")
        .content
        .clone();
    (facade, native_lib, TYPED_ERROR_CODE)
}
