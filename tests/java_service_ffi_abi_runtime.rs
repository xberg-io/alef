use alef::backends::{ffi::FfiBackend, java::JavaBackend};
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef,
    RegistrationVariant, ServiceDef, TypeRef, WrapperConstructorArg, WrapperConstructorCall,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const JAVA_PACKAGE: &str = "com.example";
const EXPECTED_VARIANT_STATUS: i32 = 17;

fn config() -> ResolvedCrateConfig {
    let parsed: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi", "java"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.example"
"#,
    )
    .expect("parse fixture config");
    parsed.resolve().expect("resolve fixture config").remove(0)
}

fn constructor() -> MethodDef {
    MethodDef {
        name: "new".to_owned(),
        is_static: true,
        return_type: TypeRef::Unit,
        doc: "Create the service.".to_owned(),
        ..MethodDef::default()
    }
}

fn string_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_owned(),
        ty: TypeRef::String,
        ..ParamDef::default()
    }
}

fn route_registration() -> RegistrationDef {
    let path = string_param("path");
    RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![path.clone()],
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        variants: vec![RegistrationVariant {
            name: "get".to_owned(),
            signature_params: vec![path.clone()],
            wrapper_call: Some(WrapperConstructorCall {
                metadata_param: "path".to_owned(),
                wrapper_type_path: "test::Route".to_owned(),
                wrapper_type_name: "Route".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![WrapperConstructorArg::Free { param: path }],
                options: Vec::new(),
            }),
            doc: Some("Register a GET handler.".to_owned()),
            ..RegistrationVariant::default()
        }],
        ..RegistrationDef::default()
    }
}

fn run_entrypoint() -> EntrypointDef {
    EntrypointDef {
        method: "run".to_owned(),
        kind: EntrypointKind::Run,
        is_async: false,
        params: vec![string_param("address")],
        return_type: TypeRef::Unit,
        error_type: Some("ServiceError".to_owned()),
        doc: "Run the service.".to_owned(),
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

fn surface() -> ApiSurface {
    ApiSurface {
        crate_name: "test".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "test::TestService".to_owned(),
            constructor: constructor(),
            configurators: vec![],
            registrations: vec![route_registration()],
            entrypoints: vec![run_entrypoint()],
            doc: "Neutral service ABI fixture.".to_owned(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract()],
        ..ApiSurface::default()
    }
}

fn output_or_panic(action: &str, output: Output) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_java_sources(root: &Path, files: &[alef::GeneratedFile]) -> PathBuf {
    let package_dir = root.join("com/example");
    fs::create_dir_all(&package_dir).expect("create package directory");
    for file in files {
        let name = file.path.file_name().expect("generated Java filename");
        fs::write(package_dir.join(name), &file.content).expect("write generated Java source");
    }
    package_dir
}

fn write_native_fixture(root: &Path) -> PathBuf {
    let source = root.join("service_fixture.c");
    let library = root.join("libservice_fixture.dylib");
    fs::write(
        &source,
        r#"
#include <stdint.h>
#include <string.h>

typedef char *(*handler_callback_t)(void *, const char *);
typedef void (*handler_response_free_t)(char *);

uint64_t test_test_service_new(void) { return 41; }
void test_test_service_free(uint64_t owner) { (void)owner; }

int32_t test_test_service_get(
    uint64_t owner,
    handler_callback_t callback,
    handler_response_free_t response_free,
    void *context,
    const char *path
) {
    char *response = callback(context, "{}");
    int response_ok = response != NULL && strcmp(response, "{}") == 0;
    response_free(response);
    return owner == 41 && response_ok && strcmp(path, "/ok") == 0 ? 17 : -1;
}

int32_t test_test_service_ep_run(uint64_t owner, const char *address) {
    return owner == 41 && strcmp(address, "127.0.0.1:3000") == 0 ? 0 : -1;
}
"#,
    )
    .expect("write C fixture");
    let output = Command::new("cc")
        .args(["-dynamiclib", "-o"])
        .arg(&library)
        .arg(&source)
        .output()
        .expect("run C compiler");
    output_or_panic("compile C fixture", output);
    library
}

fn write_java_harness(package_dir: &Path) -> PathBuf {
    let harness = package_dir.join("ServiceAbiHarness.java");
    fs::write(
        &harness,
        format!(
            r#"package {JAVA_PACKAGE};

public final class ServiceAbiHarness {{
    public static void main(String[] args) {{
        System.load(args[0]);
        try (var service = new TestService()) {{
            int status = service.get("/ok", request -> "{{}}");
            if (status != {EXPECTED_VARIANT_STATUS}) {{
                throw new AssertionError("unexpected variant status: " + status);
            }}
            service.run("127.0.0.1:3000");
        }}
    }}
}}
"#
        ),
    )
    .expect("write Java harness");
    harness
}

fn compile_and_run_java(root: &Path, package_dir: &Path, library: &Path) {
    let sources: Vec<PathBuf> = fs::read_dir(package_dir)
        .expect("read Java package directory")
        .map(|entry| entry.expect("read Java source entry").path())
        .collect();
    let output = Command::new("javac")
        .args(["-d"])
        .arg(root)
        .args(&sources)
        .output()
        .expect("run javac");
    output_or_panic("compile generated Java service", output);
    let output = Command::new("java")
        .arg("--enable-native-access=ALL-UNNAMED")
        .args(["-cp"])
        .arg(root)
        .arg(format!("{JAVA_PACKAGE}.ServiceAbiHarness"))
        .arg(library)
        .output()
        .expect("run Java service harness");
    output_or_panic("run generated Java service", output);
}

#[test]
fn generated_java_service_matches_ffi_symbols_and_carriers_at_runtime() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let api = surface();
    let config = config();
    let java_files = JavaBackend
        .generate_service_api(&api, &config)
        .expect("generate Java service API");
    let java_bindings = JavaBackend
        .generate_bindings(&api, &config)
        .expect("generate Java bindings");
    let native_lib = java_bindings
        .iter()
        .find(|file| file.path.ends_with("NativeLib.java"))
        .expect("generated NativeLib.java");
    assert!(native_lib.content.contains("\"test_test_service_get\""));
    assert!(!native_lib.content.contains("\"test_test_service_register_route_get\""));
    let ffi_files = FfiBackend
        .generate_service_api(&api, &config)
        .expect("generate FFI service API");
    let ffi_source = &ffi_files[0].content;
    assert!(ffi_source.contains("fn test_test_service_get("));
    assert!(!ffi_source.contains("fn test_test_service_register_route_get("));

    let temp = tempfile::tempdir().expect("create fixture directory");
    let package_dir = write_java_sources(temp.path(), &java_files);
    write_java_harness(&package_dir);
    let library = write_native_fixture(temp.path());
    compile_and_run_java(temp.path(), &package_dir, &library);
}
