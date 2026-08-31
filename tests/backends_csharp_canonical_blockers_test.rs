use std::path::Path;
use std::process::{Command, Output};

use alef::backends::csharp::CsharpBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{BridgeBinding, NewAlefConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{
    ApiSurface, HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef, ServiceDef, TypeDef, TypeRef,
};

const PROJECT_FILE: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
<PropertyGroup>
<TargetFramework>net8.0</TargetFramework>
<OutputType>Exe</OutputType>
<RollForward>Major</RollForward>
<Nullable>enable</Nullable>
<AllowUnsafeBlocks>true</AllowUnsafeBlocks>
<TreatWarningsAsErrors>true</TreatWarningsAsErrors>
<NuGetAudit>false</NuGetAudit>
</PropertyGroup>
</Project>"#;

fn test_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "test"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.csharp]
namespace = "Test"
"#,
    )
    .expect("valid config");
    config.resolve().expect("resolved config").remove(0)
}

fn trait_fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let api = ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "TextBackend".into(),
            rust_path: "test::TextBackend".into(),
            is_trait: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config = test_config();
    config.trait_bridges.push(TraitBridgeConfig {
        trait_name: "TextBackend".into(),
        register_fn: Some("register_text_backend".into()),
        unregister_fn: Some("unregister_text_backend".into()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    });
    (api, config)
}

fn opaque_fixture() -> ApiSurface {
    ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Resource".into(),
            rust_path: "test::Resource".into(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "consume".into(),
                receiver: Some(ReceiverKind::Owned),
                cfg: None,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn service_fixture() -> ApiSurface {
    let registration = RegistrationDef {
        method: "add_handler".into(),
        callback_param: "handler".into(),
        callback_contract: "RequestHandler".into(),
        metadata_params: vec![string_param("path")],
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        ..Default::default()
    };
    ApiSurface {
        crate_name: "test".into(),
        version: "0.1.0".into(),
        types: vec![serde_type("RequestData"), serde_type("Response")],
        services: vec![ServiceDef {
            name: "TestService".into(),
            rust_path: "test::TestService".into(),
            constructor: MethodDef {
                name: "new".into(),
                is_static: true,
                ..Default::default()
            },
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract()],
        ..Default::default()
    }
}

fn handler_contract() -> HandlerContractDef {
    HandlerContractDef {
        trait_name: "RequestHandler".into(),
        rust_path: "test::RequestHandler".into(),
        dispatch: MethodDef {
            name: "handle".into(),
            params: vec![ParamDef {
                name: "request".into(),
                ty: TypeRef::Named("RequestData".into()),
                ..Default::default()
            }],
            return_type: TypeRef::Named("Response".into()),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            ..Default::default()
        },
        wire_request_type: Some("RequestData".into()),
        wire_response_type: Some("Response".into()),
        optional_methods: vec![],
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: String::new(),
    }
}

fn string_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.into(),
        ty: TypeRef::String,
        ..Default::default()
    }
}

fn serde_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.into(),
        has_serde: true,
        ..Default::default()
    }
}

fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<GeneratedFile> {
    CsharpBackend.generate_bindings(api, config).expect("C# generation")
}

fn generate_with_services(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<GeneratedFile> {
    let mut files = generate(api, config);
    files.extend(
        CsharpBackend
            .generate_service_api(api, config)
            .expect("C# service generation"),
    );
    files
}

fn source_named<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing {suffix}"))
        .content
}

fn write_project(directory: &Path, files: &[GeneratedFile], program: &str) {
    for file in files {
        let name = file.path.file_name().expect("generated file name");
        std::fs::write(directory.join(name), &file.content).expect("generated C# source");
    }
    std::fs::write(directory.join("Test.csproj"), PROJECT_FILE).expect("C# project");
    std::fs::write(directory.join("Program.cs"), program).expect("C# program");
}

fn dotnet(directory: &Path, verb: &str) -> Output {
    Command::new("dotnet")
        .args([verb, "--nologo", "-v:quiet"])
        .current_dir(directory)
        .output()
        .expect("dotnet command")
}

/// `false` when `dotnet` is not on `PATH`. Panics instead of returning `false` when
/// `ALEF_REQUIRE_DOTNET` is set, so CI cannot silently skip the two real `dotnet run` compile
/// checks below when the runner's toolchain setup regresses.
///
/// Checks exit status, not merely that the process spawned: a version-manager shim spawns fine
/// then exits non-zero, so `.output().is_ok()` alone would report `dotnet` available when it
/// cannot actually build anything, leaving the compile checks below to fail instead of skip. ~keep
fn dotnet_available() -> bool {
    let available = Command::new("dotnet")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var_os("ALEF_REQUIRE_DOTNET").is_none(),
        "ALEF_REQUIRE_DOTNET is set but dotnet is unavailable"
    );
    available
}

fn assert_failure_safe_transfer(source: &str, take_method: &str, unavailable: &str, rollback: &str) {
    let section = source.split(take_method).nth(1).expect("transfer method");
    let section = section.split("private void").next().expect("transfer body");
    let unavailable_position = section.find(unavailable).expect("unavailable flag");
    let construction_position = section.find("new ").expect("transfer construction");
    let construction_precedes_unavailable = construction_position < unavailable_position;
    let rollback_on_failure = section.contains("catch") && section.contains(rollback);
    assert!(
        construction_precedes_unavailable || rollback_on_failure,
        "failed transfer construction strands owner:\n{section}"
    );
}

#[test]
fn bridge_dispose_waits_for_native_release_and_active_callback() {
    if !dotnet_available() {
        return;
    }
    let (api, config) = trait_fixture();
    let files = generate(&api, &config);
    let directory = tempfile::tempdir().expect("temporary bridge project");
    write_project(
        directory.path(),
        &files,
        include_str!("fixtures/csharp/bridge_dispose_during_callback.cs"),
    );
    let output = dotnet(directory.path(), "run");
    assert!(
        output.status.success(),
        "bridge lifecycle runtime failed:\nstdout:\n{}\nstderr:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        source_named(&files, "TraitBridges.cs")
    );
}

#[test]
fn unregister_cleanup_uses_native_release_state_machine() {
    let (api, config) = trait_fixture();
    let files = generate(&api, &config);
    let source = source_named(&files, "TraitBridges.cs");
    let unregister = source
        .split("public static void Unregister")
        .nth(1)
        .expect("unregister");
    let unregister = unregister
        .split("public static void Clear")
        .next()
        .unwrap_or(unregister);
    assert!(
        unregister.contains("TextBackendBridge.FreeUserData(bridge._bridgeId)"),
        "unregister bypasses native-release state machine:\n{unregister}"
    );
    assert!(
        !unregister.contains("bridge.Dispose()"),
        "premature cleanup:\n{unregister}"
    );
}

#[test]
fn transfer_construction_failure_does_not_strand_owner() {
    let opaque_files = generate(&opaque_fixture(), &test_config());
    let opaque = source_named(&opaque_files, "Resource.cs");
    assert_failure_safe_transfer(
        opaque,
        "private HandleTransfer TakeHandle()",
        "_handleUnavailable = true",
        "RollbackTransfer",
    );

    let service_files = generate_with_services(&service_fixture(), &test_config());
    let service = source_named(&service_files, "TestService.cs");
    assert_failure_safe_transfer(
        service,
        "private OwnerHandleTransfer TakeOwnerHandle()",
        "_ownerUnavailable = true",
        "RollbackOwnerTransfer",
    );
}

#[test]
fn service_registration_accepts_only_func_string_string() {
    if !dotnet_available() {
        return;
    }
    let files = generate_with_services(&service_fixture(), &test_config());
    let source = source_named(&files, "TestService.cs");
    assert!(source.contains("Func<string, string> handler"), "{source}");
    assert!(!source.contains("Delegate handler"), "{source}");
    assert_service_registration_compile_contract(&files);
}

fn assert_service_registration_compile_contract(files: &[GeneratedFile]) {
    let directory = tempfile::tempdir().expect("temporary service project");
    write_project(
        directory.path(),
        files,
        include_str!("fixtures/csharp/service_registration_func.cs"),
    );
    let valid = dotnet(directory.path(), "build");
    assert!(
        valid.status.success(),
        "Func registration must compile:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );
    std::fs::write(
        directory.path().join("Program.cs"),
        include_str!("fixtures/csharp/service_registration_delegate.cs"),
    )
    .expect("invalid registration program");
    let invalid = dotnet(directory.path(), "build");
    assert!(
        !invalid.status.success(),
        "arbitrary Delegate registration compiled unexpectedly"
    );
}
