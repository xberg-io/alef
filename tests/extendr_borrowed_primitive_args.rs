use alef::backends::extendr::ExtendrBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use alef::core::validation::validate_api_surface;
use alef::extract::extractor::extract;
use quote::ToTokens;
use std::path::Path;
use std::process::Command;

const CORE: &str = r#"
pub fn by_value(value: u32) -> u32 { value }
pub fn no_cast_borrowed(value: &i32) -> i32 { *value }
pub fn depth(value: &u32) -> u32 { *value }
pub fn wide(value: &u64) -> u64 { *value }
pub fn mutated(value: &mut u32) -> u32 { *value += 1; *value }
"#;

fn extracted_api(root: &Path) -> ApiSurface {
    let source = root.join("lib.rs");
    std::fs::write(&source, CORE).expect("write actual public Rust input");
    let api = extract(&[source.as_path()], "test_lib", "1.0.0", None).expect("extract public scalar functions");
    assert_eq!(api.functions.len(), 5);
    assert!(api.unsupported_public_items.is_empty());
    let validation = validate_api_surface(&api);
    assert!(!validation.has_errors(), "{}", validation.format_errors());
    for (name, primitive, borrowed, mutable) in [
        ("by_value", PrimitiveType::U32, false, false),
        ("no_cast_borrowed", PrimitiveType::I32, true, false),
        ("depth", PrimitiveType::U32, true, false),
        ("wide", PrimitiveType::U64, true, false),
        ("mutated", PrimitiveType::U32, true, true),
    ] {
        let function = api
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("extracted function");
        assert!(!function.sanitized);
        assert_eq!(function.params.len(), 1);
        let parameter = &function.params[0];
        assert_eq!(parameter.ty, TypeRef::Primitive(primitive));
        assert_eq!(parameter.is_ref, borrowed);
        assert_eq!(parameter.is_mut, mutable);
        assert!(!parameter.optional);
        assert!(!parameter.sanitized);
    }
    api
}

fn generated_functions(api: &ApiSurface) -> Vec<syn::ItemFn> {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["r"]
[[crates]]
name = "test-lib"
sources = ["lib.rs"]
[crates.r]
package_name = "testlib"
"#,
    )
    .expect("ordinary supported R configuration");
    let resolved = config.resolve().expect("resolve R configuration");
    let files = ExtendrBackend
        .generate_bindings(api, &resolved[0])
        .expect("emit actual R backend output");
    let library = files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("generated Rust library");
    let parsed = syn::parse_file(&library.content).expect("parse actual generated Rust");
    let mut functions = Vec::new();
    for item in parsed.items {
        if let syn::Item::Fn(mut function) = item
            && ["by_value", "no_cast_borrowed", "depth", "wide", "mutated"]
                .iter()
                .any(|name| function.sig.ident == *name)
        {
            // Isolate Rust argument typing; the R registration macro is outside this boundary.
            let registrations = function
                .attrs
                .iter()
                .filter(|attribute| attribute.path().is_ident("extendr"))
                .count();
            assert_eq!(registrations, 1);
            function.attrs.retain(|attribute| !attribute.path().is_ident("extendr"));
            functions.push(function);
        }
    }
    assert_eq!(functions.len(), 5, "compile all five real emitted wrappers");
    functions
}

fn compile_and_run(root: &Path, label: &str, functions: &[syn::ItemFn], main: &str) {
    let emitted = functions
        .iter()
        .map(|function| function.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let source = root.join(format!("{label}.rs"));
    let binary = root.join(format!("{label}{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(
        &source,
        format!("pub mod test_lib {{ {CORE} }}\n{emitted}\nfn main() {{ {main} }}"),
    )
    .expect("write actual generated functions beside their actual core signatures");
    let compiled = Command::new("rustc")
        .args(["--edition=2024", "--crate-name", label])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile emitted Rust call boundary with pinned rustc");
    assert!(
        compiled.status.success(),
        "generated {label} must compile:\n{}\nsource:\n{}",
        String::from_utf8_lossy(&compiled.stderr),
        std::fs::read_to_string(&source).expect("diagnostic source")
    );
    let executed = Command::new(&binary)
        .output()
        .expect("execute compiled scalar controls");
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}

#[test]
fn should_compile_accepted_borrowed_remapped_primitive_calls() {
    let root = tempfile::tempdir().expect("isolated Rust call-boundary fixture");
    let api = extracted_api(root.path());
    let functions = generated_functions(&api);
    let (borrowed, controls): (Vec<_>, Vec<_>) = functions.into_iter().partition(|function| {
        ["depth", "wide", "mutated"]
            .iter()
            .any(|name| function.sig.ident == *name)
    });
    assert_eq!(controls.len(), 2);
    assert_eq!(borrowed.len(), 3);
    compile_and_run(
        root.path(),
        "positive_controls",
        &controls,
        "assert_eq!(by_value(33), 33); assert_eq!(no_cast_borrowed(44), 44);",
    );
    compile_and_run(
        root.path(),
        "borrowed_remapped_after_two_positive_controls",
        &borrowed,
        "assert_eq!(depth(55), 55); assert_eq!(wide(66.0), 66.0); assert_eq!(mutated(77), 78);",
    );
}
