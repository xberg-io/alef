use alef::backends::magnus::trait_bridge::gen_options_field_bridge_function;
use alef::codegen::type_mapper::IdentityMapper;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};
use quote::quote;
use std::path::Path;
use std::process::Command;
use syn::visit::Visit;

#[derive(Default)]
struct OptionsConstruction {
    blocks: Vec<String>,
}

impl<'ast> Visit<'ast> for OptionsConstruction {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        let handle = block.stmts.iter().position(|statement| {
            matches!(statement, syn::Stmt::Local(local)
                if matches!(&local.pat, syn::Pat::Ident(pattern) if pattern.ident == "handle"))
        });
        if let Some(index) = handle {
            let statements = &block.stmts[index + 1..];
            self.blocks.push(quote!(#(#statements)*).to_string());
        }
        syn::visit::visit_block(self, block);
    }
}

fn generated_options_construction() -> String {
    let function = FunctionDef {
        name: "render".to_string(),
        params: vec![
            ParamDef {
                name: "html".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("RenderOptions".to_string()),
                ..Default::default()
            },
        ],
        return_type: TypeRef::String,
        ..Default::default()
    };
    let bridge = TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("visitor".to_string()),
        ..Default::default()
    };
    let code = gen_options_field_bridge_function(
        &ApiSurface::default(),
        &function,
        1,
        &bridge,
        &IdentityMapper,
        &ahash::AHashSet::new(),
        "sample_core",
    );
    let parsed = syn::parse_file(&code).expect("generated wrapper must parse");
    let mut construction = OptionsConstruction::default();
    construction.visit_file(&parsed);
    assert_eq!(
        construction.blocks.len(),
        1,
        "must compile the generated visitor options block"
    );
    construction.blocks.remove(0)
}

fn assert_external_options_constructible(definition: &str) {
    let directory = tempfile::tempdir().expect("fixture directory");
    let core = directory.path().join("core.rs");
    std::fs::write(&core, definition).expect("write core fixture");
    let library = directory.path().join("libsample_core.rlib");
    run_rustc(
        &core,
        &["--crate-name", "sample_core", "--crate-type", "rlib"],
        &library,
    );
    let consumer = directory.path().join("consumer.rs");
    let construction = generated_options_construction();
    std::fs::write(&consumer, format!(
        "fn main() {{ let handle = (); let options = {{ {construction} }}; assert_eq!(options.visitor, Some(())); }}"
    )).expect("write generated construction consumer");
    let binary = directory.path().join("consumer");
    run_rustc(
        &consumer,
        &["--extern", &format!("sample_core={}", library.display())],
        &binary,
    );
    assert!(Command::new(binary).status().expect("execute consumer").success());
}

fn run_rustc(source: &Path, arguments: &[&str], output: &Path) {
    let result = Command::new("rustc")
        .args(["--edition=2024"])
        .args(arguments)
        .arg(source)
        .arg("-o")
        .arg(output)
        .output()
        .expect("run fixture compiler");
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

#[test]
fn should_construct_external_non_exhaustive_options() {
    assert_external_options_constructible(
        "#[derive(Default)] #[non_exhaustive] pub struct RenderOptions { pub visitor: Option<()> }",
    );
}

#[test]
fn should_construct_external_options_with_private_fields() {
    assert_external_options_constructible(
        "#[derive(Default)] pub struct RenderOptions { pub visitor: Option<()>, _private: bool }",
    );
}
