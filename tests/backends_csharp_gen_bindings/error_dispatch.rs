use super::*;

fn error_api(name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![ErrorDef {
            name: name.to_string(),
            rust_path: format!("test::{name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "InvalidInput".to_string(),
                error_code: None,
                message_template: Some("invalid input: {0}".to_string()),
                fields: vec![],
                has_source: false,
                has_from: false,
                is_unit: true,
                is_tuple: false,
                doc: String::new(),
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        excluded_type_paths: std::collections::BTreeMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn dispatcher_for(error_name: &str) -> String {
    CsharpBackend
        .generate_bindings(&error_api(error_name), &minimal_csharp_config("test"))
        .unwrap()
        .into_iter()
        .find(|file| file.content.contains("internal static Exception FromLastError"))
        .expect("a generated file must carry the FromLastError dispatcher")
        .content
}

#[test]
fn test_error_helper_preserves_base_error_acronym_class_name() {
    let dispatcher = dispatcher_for("GraphQLError");

    assert!(
        dispatcher.contains("if (code == 2) return WithNativeCode(new GraphQLErrorException(message), code);"),
        "{dispatcher}"
    );
    assert!(!dispatcher.contains("GraphQlErrorException"));
}

/// FFI code 1 is infrastructure-owned and must not be hijacked by a user variant. ~keep
#[test]
fn test_invalid_input_variant_does_not_hijack_ffi_conversion_error_code() {
    let dispatcher = dispatcher_for("RequestError");

    assert!(
        !dispatcher.contains("code == 1"),
        "FFI code 1 must not dispatch to a user variant: {dispatcher}"
    );
    assert!(
        dispatcher.contains("if (message.StartsWith(\"invalid input:\")) return WithNativeCode(new InvalidInputException(message), code);"),
        "InvalidInput must dispatch by message prefix: {dispatcher}"
    );
}

#[test]
fn test_typed_exceptions_preserve_native_codes_and_legacy_constructors() {
    if !dotnet_is_runnable() {
        assert!(std::env::var_os("ALEF_REQUIRE_DOTNET").is_none(), "dotnet is required");
        return;
    }
    let files = CsharpBackend
        .generate_bindings(&error_api("RequestError"), &minimal_csharp_config("test"))
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut exception_count = 0;
    for file in files {
        if file.content.contains("public class ") && file.content.contains("Exception : ") {
            let filename = file.path.file_name().unwrap();
            std::fs::write(directory.path().join(filename), file.content).unwrap();
            exception_count += 1;
        }
    }
    assert_eq!(exception_count, 3);
    std::fs::write(
        directory.path().join("Program.cs"),
        include_str!("../fixtures/csharp_error_codes.cs"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("Probe.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType>
<TargetFramework>net8.0</TargetFramework><RollForward>LatestMajor</RollForward>
<NuGetAudit>false</NuGetAudit></PropertyGroup></Project>"#,
    )
    .unwrap();
    let output = std::process::Command::new("dotnet")
        .args(["run", "--project", "Probe.csproj", "--verbosity", "quiet"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
