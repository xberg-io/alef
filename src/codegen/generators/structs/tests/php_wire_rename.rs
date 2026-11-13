use super::{base_cfg, plain_field, type_with_fields};
use crate::codegen::generators::{gen_struct, gen_struct_with_per_field_attrs, gen_struct_with_rename};
use crate::codegen::type_mapper::IdentityMapper;
use crate::core::ir::TypeRef;
use crate::test_support::cargo_artifacts::{add_dependency_search_paths, dependency_rlibs};
use quote::ToTokens;

#[test]
fn all_struct_emitters_preserve_wire_rename_without_changing_host_field_names() {
    let mut field = plain_field("call_type", TypeRef::String);
    field.serde_rename = Some("type".to_string());
    let typ = type_with_fields("SampleCall", vec![field], Default::default());
    let cfg = crate::codegen::generators::RustBindingConfig {
        struct_attrs: &["serde(rename_all = \"camelCase\")"],
        ..base_cfg()
    };
    let emitted = [
        gen_struct(&typ, &IdentityMapper, &cfg),
        gen_struct_with_rename(&typ, &IdentityMapper, &cfg, |_| Vec::new(), |_| None),
        gen_struct_with_per_field_attrs(&typ, &IdentityMapper, &cfg, |_| Vec::new()),
    ];
    assert_eq!(emitted.len(), 3);
    for source in emitted {
        assert!(source.contains("serde(rename = \"type\")"), "{source}");
        assert!(source.contains("pub call_type: String"), "{source}");
        assert!(source.contains("serde(rename_all = \"camelCase\")"), "{source}");
    }
}

fn generated_php_struct() -> syn::ItemStruct {
    use crate::backends::php::PhpBackend;
    use crate::core::backend::Backend;
    use crate::core::config::ResolvedCrateConfig;

    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core.rs");
    std::fs::write(&core, include_str!("php_wire_rename_core.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[core.as_path()], "sample_core", "1.0.0", None).unwrap();
    assert_eq!(api.types.len(), 1);
    assert_eq!(api.types[0].fields[0].serde_rename.as_deref(), Some("type"));
    let source = root.path().join("php/src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        root.path().join("php/Cargo.toml"),
        r#"[package]
name = "sample-php"
version = "1.0.0"
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
"#,
    )
    .unwrap();
    let mut config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        ..Default::default()
    };
    config.output_paths.insert("php".to_string(), source.clone());
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let binding = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let parsed = syn::parse_file(&binding.content).unwrap();
    parsed
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "SampleCall" => Some(item),
            _ => None,
        })
        .unwrap()
}

#[test]
fn php_generated_dto_preserves_wire_keys_aliases_and_typed_core_conversion() {
    let mut structure = generated_php_struct();
    let field = structure.fields.iter().next().unwrap();
    assert_eq!(field.ident.as_ref().unwrap(), "call_type");
    assert!(
        field
            .attrs
            .iter()
            .any(|attr| { attr.path().is_ident("php") && attr.to_token_stream().to_string().contains("\"callType\"") })
    );
    structure
        .attrs
        .retain(|attr| attr.path().is_ident("serde") || attr.path().is_ident("derive"));
    for field in &mut structure.fields {
        field.attrs.retain(|attr| attr.path().is_ident("serde"));
    }
    let declaration = structure.to_token_stream().to_string();
    assert!(declaration.contains("call_type"));
    assert_eq!(declaration.matches("\"callType\"").count(), 1);
    assert_eq!(declaration.matches("alias = \"call_type\"").count(), 1);
    run_roundtrip(&declaration, true);
    let mut removed = 0;
    for field in &mut structure.fields {
        field.attrs.retain(|attr| {
            let rename = attr.to_token_stream().to_string().contains("rename");
            removed += usize::from(rename);
            !rename
        });
    }
    assert_eq!(removed, 1);
    run_roundtrip(&structure.to_token_stream().to_string(), false);
}

fn run_roundtrip(declaration: &str, expected_success: bool) {
    let root = tempfile::tempdir().unwrap();
    let compiler = which::which_in("rustc", std::env::var_os("PATH"), root.path()).unwrap();
    let code = crate::codegen::template_env::render(
        "struct_field_wire_probe.rs.jinja",
        crate::alef_context! { declaration => declaration },
    );
    let source = root.path().join("probe.rs");
    let binary = root.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source, code).unwrap();
    let mut command = std::process::Command::new(compiler);
    command
        .arg("--edition=2024")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .current_dir(root.path());
    for name in ["serde", "serde_json"] {
        let library = dependency_rlibs(name)
            .into_iter()
            .next()
            .expect("compiled Rust dependency");
        command.arg("--extern").arg(format!("{name}={}", library.display()));
    }
    let result = add_dependency_search_paths(&mut command).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let executed = std::process::Command::new(binary)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(
        executed.status.success(),
        expected_success,
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    if !expected_success {
        assert!(String::from_utf8_lossy(&executed.stderr).contains("wire key type must populate call_type"));
    }
}
