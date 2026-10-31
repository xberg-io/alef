use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, TraitBridgeConfig};
use alef::core::ir::{PrimitiveType, TypeRef};
use quote::ToTokens;
use std::path::Path;

#[path = "support/mise_task.rs"]
mod mise_task;
#[path = "backends_dart_u64_bigint/native.rs"]
mod native;

const CORE: &str = include_str!("fixtures/dart_u64_bigint/core.rs");

fn generated(root: &Path) -> Vec<alef::core::backend::GeneratedFile> {
    generated_with_callbacks(root, false)
}

fn generated_with_callbacks(root: &Path, callback: bool) -> Vec<alef::core::backend::GeneratedFile> {
    let source = root.join("core/src/lib.rs");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    let core = if callback {
        format!("{CORE}\npub trait WideCallback: Send + Sync {{ fn value(&self, input: u64) -> u64; }}\n")
    } else {
        CORE.to_owned()
    };
    std::fs::write(&source, core).unwrap();
    let api = alef::extract::extractor::extract(&[&source], "wide-sample", "1.0.0", None).unwrap();
    let record = api.types.iter().find(|item| item.name == "WideRecord").unwrap();
    assert_eq!(record.rust_path.replace('-', "_"), "wide_sample::WideRecord");
    let probe = api.types.iter().find(|item| item.name == "Probe").unwrap();
    assert_eq!(probe.rust_path.replace('-', "_"), "wide_sample::Probe");
    assert_eq!(record.fields.len(), 9, "extract the full real Rust record");
    assert_eq!(record.fields[0].ty, TypeRef::Primitive(PrimitiveType::U64));
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"wide-sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let mut config = config.resolve().unwrap().remove(0);
    if callback {
        config.trait_bridges.push(TraitBridgeConfig {
            trait_name: "WideCallback".into(),
            ..Default::default()
        });
    }
    DartBackend.generate_bindings(&api, &config).unwrap()
}

fn emitted(root: &Path, suffix: &str) -> String {
    generated(root)
        .into_iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap()
        .content
}

fn field_type(source: &str, field: &str) -> String {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .find_map(|item| {
            if let syn::Item::Struct(item) = item {
                if item.ident == "WideRecord" {
                    return item
                        .fields
                        .iter()
                        .find(|item| item.ident.as_ref().is_some_and(|name| name == field))
                        .map(|item| item.ty.to_token_stream().to_string());
                }
            }
            None
        })
        .unwrap()
}

#[test]
fn dart_unsigned_scalar_mirrors_preserve_the_original_rust_u64() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    assert_eq!(field_type(&source, "value"), "u64");
    assert_eq!(field_type(&source, "optional"), "Option < u64 >");
}

#[test]
fn dart_unsigned_container_mirrors_preserve_full_width_recursively() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    assert_eq!(field_type(&source, "values"), "Vec < u64 >");
    assert_eq!(
        field_type(&source, "named"),
        "std :: collections :: HashMap < String , u64 >"
    );
}

#[test]
fn dart_unsigned_newtypes_remain_lossless_after_real_extraction() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    assert_eq!(field_type(&source, "wrapped"), "u64");
}

#[test]
fn dart_unsigned_function_arguments_and_results_preserve_u64() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    let file = syn::parse_file(&source).unwrap();
    let mut checked = 0;
    for item in file.items {
        let syn::Item::Fn(item) = item else { continue };
        let expected = match item.sig.ident.to_string().as_str() {
            "echo_scalar" => "u64",
            "echo_optional" => "Option < u64 >",
            "echo_vector" => "Vec < u64 >",
            "echo_map" => "std :: collections :: HashMap < String , u64 >",
            _ => continue,
        };
        let syn::FnArg::Typed(argument) = item.sig.inputs.first().unwrap() else {
            panic!("free argument")
        };
        assert_eq!(argument.ty.to_token_stream().to_string(), expected);
        let syn::ReturnType::Type(_, result) = item.sig.output else {
            panic!("non-unit return")
        };
        assert_eq!(result.to_token_stream().to_string(), expected);
        checked += 1;
    }
    assert_eq!(checked, 4, "each scalar and container function must be generated");
}

#[test]
fn dart_public_unsigned_api_uses_bigint_and_unsigned_lists() {
    let directory = tempfile::tempdir().unwrap();
    let facade = emitted(directory.path(), "lib/src/wide_sample.dart");
    assert!(
        facade.contains("BigInt value"),
        "public scalar argument must accept BigInt: {facade}"
    );
    assert!(
        facade.contains("BigInt? value"),
        "optional scalar must accept BigInt: {facade}"
    );
    assert!(
        facade.contains("Uint64List values"),
        "unsigned vector must use the lossless FRB list: {facade}"
    );
}

#[test]
fn dart_unsigned_fix_keeps_narrow_and_signed_mirrors_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    for field in ["byte", "short", "count", "signed"] {
        assert_eq!(
            field_type(&source, field),
            "i64",
            "legacy {field} widening must remain unchanged"
        );
    }
}

#[test]
fn dart_unsigned_callback_carriers_preserve_u64() {
    let directory = tempfile::tempdir().unwrap();
    let files = generated_with_callbacks(directory.path(), true);
    let source = files
        .into_iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap()
        .content;
    let compact = source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(
        compact.contains("Fn(u64)->DartFnFuture<u64>"),
        "unsigned callback narrowed: {source}"
    );
}

#[test]
fn dart_unsigned_nested_record_fields_survive_real_extraction() {
    let directory = tempfile::tempdir().unwrap();
    let source = emitted(directory.path(), "rust/src/lib.rs");
    let parsed = syn::parse_file(&source).unwrap();
    let nested = parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "WideNested" => Some(item),
            _ => None,
        })
        .unwrap();
    let actual = nested
        .fields
        .iter()
        .map(|field| field.ty.to_token_stream().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            "WideRecord",
            "Option < WideRecord >",
            "Vec < WideRecord >",
            "std :: collections :: HashMap < String , WideRecord >",
        ]
    );
}
