//! Shared-family additions retain old client calls and user-owned type names.

use alef::backends::{csharp::CsharpBackend, java::JavaBackend};
use alef::codegen::error_gen::gen_php_error_types;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, ErrorDef, ErrorVariant, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::process::Command;

fn family(name: &str, variant: &str) -> ErrorDef {
    ErrorDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        variants: vec![ErrorVariant {
            name: variant.into(),
            is_unit: true,
            ..Default::default()
        }],
        exception_base: None,
        original_rust_path: String::new(),
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn api(shared: bool) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        errors: vec![
            family("StorageError", "Missing"),
            family("TransportError", if shared { "Missing" } else { "Offline" }),
        ],
        ..Default::default()
    }
}

fn config(language: &str) -> ResolvedCrateConfig {
    let namespace = if language == "java" {
        "package = 'demo'"
    } else {
        "namespace = 'Demo'"
    };
    let source = format!(
        "[workspace]\nlanguages = ['{language}', 'ffi']\n[[crates]]\nname = 'demo'\n\
         sources = ['src/lib.rs']\n[crates.ffi]\nprefix = 'demo'\n[crates.{language}]\n{namespace}\n"
    );
    toml::from_str::<NewAlefConfig>(&source)
        .unwrap()
        .resolve()
        .unwrap()
        .remove(0)
}

fn add_type(api: &mut ApiSurface, name: &str) {
    api.types.push(TypeDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        has_serde: true,
        ..Default::default()
    });
}

fn assert_user_type_preserved(shared: bool) {
    let mut model = api(shared);
    add_type(&mut model, "IStorageError");
    for files in [
        JavaBackend
            .generate_bindings(&model, &config("java"))
            .expect("Java preserves user type"),
        CsharpBackend
            .generate_bindings(&model, &config("csharp"))
            .expect("C# preserves user type"),
    ] {
        let owners = files
            .iter()
            .filter(|file| file.path.file_stem().is_some_and(|stem| stem == "IStorageError"))
            .collect::<Vec<_>>();
        assert_eq!(owners.len(), 1, "user type must have exactly one output: {owners:?}");
        assert!(!owners[0].content.contains("public interface IStorageError"));
    }
    let mut php_model = api(shared);
    add_type(&mut php_model, "StorageErrorInterface");
    let php = gen_php_error_types(&php_model, "Demo", None).expect("PHP preserves user type name");
    assert!(
        !php.contains("#[php(name = \"Demo\\\\StorageErrorInterface\")]"),
        "{php}"
    );
}

#[test]
fn unique_family_marker_names_do_not_reserve_user_type_names() {
    assert_user_type_preserved(false);
}

#[test]
fn shared_family_markers_do_not_overwrite_user_type_names() {
    assert_user_type_preserved(true);
}

fn write_java_exceptions(files: &[GeneratedFile], package: &std::path::Path) -> Vec<std::path::PathBuf> {
    files
        .iter()
        .filter_map(|generated| {
            let name = generated.path.file_name().unwrap().to_string_lossy();
            if ![
                "DemoRsException.java",
                "StorageErrorException.java",
                "TransportErrorException.java",
                "MissingException.java",
                "IStorageError.java",
                "ITransportError.java",
            ]
            .contains(&name.as_ref())
            {
                return None;
            }
            let path = package.join(name.as_ref());
            std::fs::write(&path, &generated.content).unwrap();
            Some(path)
        })
        .collect()
}

#[test]
fn java_shared_introspection_preserves_legacy_untyped_null_cause_calls() {
    let mut model = api(true);
    model.errors[1].methods.push(MethodDef {
        name: "error_type".into(),
        return_type: TypeRef::String,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    });
    compile_java_probe(
        &model,
        r#"package demo;
public class LegacyNullProbe {
  public static void main(String[] args) {
    MissingException value = new MissingException("legacy", null);
    if (value.getCause() != null) throw new AssertionError("cause");
    if (!"legacy".equals(value.getMessage())) throw new AssertionError("message");
    System.out.println("LEGACY_NULL=1");
  }
}
"#,
        "LEGACY_NULL=1",
    );
}

fn compile_java_probe(model: &ApiSurface, source: &str, expected: &str) {
    let generated = JavaBackend.generate_bindings(model, &config("java")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("demo");
    std::fs::create_dir(&package).unwrap();
    let mut sources = write_java_exceptions(&generated, &package);
    assert_eq!(sources.len(), 6, "root, two bases, two markers and one shared variant");
    let probe = package.join("LegacyNullProbe.java");
    std::fs::write(&probe, source).unwrap();
    sources.push(probe);
    let compile = Command::new("javac")
        .args(&sources)
        .output()
        .expect("javac must be provisioned");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new("java")
        .arg("-cp")
        .arg(directory.path())
        .arg("demo.LegacyNullProbe")
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), expected);
}

#[test]
fn java_shared_introspection_preserves_legacy_owner_payload_constructor() {
    let mut model = api(true);
    model.errors[0].methods.push(MethodDef {
        name: "status_code".into(),
        return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::U16),
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    });
    model.errors[1].methods.push(MethodDef {
        name: "error_type".into(),
        return_type: TypeRef::String,
        receiver: Some(ReceiverKind::Ref),
        ..Default::default()
    });
    compile_java_probe(
        &model,
        r#"package demo;
public class LegacyNullProbe {
  public static void main(String[] args) {
    MissingException value = new MissingException("legacy", 200);
    if (value.getStatusCode() != 200) throw new AssertionError("payload");
    if (value.getCause() != null) throw new AssertionError("cause");
    System.out.println("LEGACY_PAYLOAD=1");
  }
}
"#,
        "LEGACY_PAYLOAD=1",
    );
}

#[test]
fn csharp_family_markers_yield_to_projected_user_type_names() {
    let mut model = api(false);
    model.errors[0].name = "GraphQLError".into();
    model.errors[0].rust_path = "demo::GraphQLError".into();
    add_type(&mut model, "IGraphQlError");
    let files = CsharpBackend.generate_bindings(&model, &config("csharp")).unwrap();
    let owners = files
        .iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "IGraphQLError.cs"))
        .collect::<Vec<_>>();
    assert!(
        owners
            .iter()
            .any(|file| !file.content.contains("public interface IGraphQLError")),
        "the projected user DTO must actually be emitted: {owners:?}"
    );
    assert_eq!(
        owners.len(),
        1,
        "projected user output path must not be overwritten: {owners:?}"
    );
    assert!(
        files
            .iter()
            .any(|file| file.content.contains("public interface IGraphQLErrorFamily"))
    );
}

#[test]
fn php_family_markers_yield_to_opaque_rust_companion_symbols() {
    let mut model = api(false);
    let name = "PhpInterfaceStorageErrorInterface";
    model.types.push(TypeDef {
        name: name.into(),
        rust_path: format!("demo::{name}"),
        is_opaque: true,
        ..Default::default()
    });
    let files = alef::backends::php::PhpBackend
        .generate_bindings(&model, &config("php"))
        .unwrap();
    let rust = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|ext| ext == "rs"))
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = syn::parse_file(&rust).expect("actual generated PHP Rust must parse");
    let mut symbols = Vec::new();
    for item in parsed.items {
        match item {
            syn::Item::Struct(item) => symbols.push(item.ident.to_string()),
            syn::Item::Trait(item) if item.attrs.iter().any(|attr| attr.path().is_ident("php_interface")) => {
                symbols.push(format!("PhpInterface{}", item.ident));
            }
            _ => {}
        }
    }
    assert!(
        rust.contains(&format!("pub struct {name}")),
        "opaque user struct was not emitted: {rust}"
    );
    assert_eq!(
        symbols.iter().filter(|symbol| *symbol == name).count(),
        1,
        "php_interface companion and opaque user struct must have distinct Rust symbols: {symbols:?}"
    );
    assert!(rust.contains("pub trait StorageErrorInterfaceFamily"), "{rust}");
}

#[test]
fn java_family_markers_yield_to_the_separately_emitted_public_facade() {
    let mut model = api(false);
    model.crate_name = "i_storage_error".into();
    let configuration = config("java");
    let mut files = JavaBackend.generate_bindings(&model, &configuration).unwrap();
    let public_api = JavaBackend.generate_public_api(&model, &configuration).unwrap();
    assert_eq!(public_api.len(), 1, "fixture must emit the real facade");
    assert_eq!(public_api[0].path.file_name().unwrap(), "IStorageError.java");
    assert!(
        public_api[0].content.contains("public final class IStorageError"),
        "the facade must actually exist: {}",
        public_api[0].content
    );
    files.extend(public_api);
    let mut paths = std::collections::HashSet::new();
    for generated in &files {
        assert!(
            paths.insert(&generated.path),
            "duplicate emitted output: {}",
            generated.path.display()
        );
    }
    assert_eq!(
        files
            .iter()
            .filter(|file| file.path.file_name().is_some_and(|name| name == "IStorageError.java"))
            .count(),
        1
    );
    assert!(
        files
            .iter()
            .any(|file| file.content.contains("public interface IStorageErrorFamily"))
    );
}
