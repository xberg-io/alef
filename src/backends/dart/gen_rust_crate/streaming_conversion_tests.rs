use crate::core::config::ResolvedCrateConfig;
use quote::ToTokens;
use std::process::Command;

fn generated_constructor(item_type: &str) -> String {
    let source = super::type_tests::stream_transport(item_type, &ResolvedCrateConfig::default());
    let parsed = syn::parse_file(&source).unwrap();
    let mut items: Vec<_> = parsed
        .items
        .into_iter()
        .filter(|item| match item {
            syn::Item::Mod(_) | syn::Item::Use(_) | syn::Item::Enum(_) => true,
            syn::Item::Impl(item) => item.trait_.is_none(),
            _ => false,
        })
        .collect();
    assert_eq!(items.len(), 4, "generated carrier module, import, wire and constructor");
    for item in &mut items {
        if let syn::Item::Mod(item) = item {
            item.attrs.retain(|attribute| !attribute.path().is_ident("frb"));
        }
    }
    let mut probe = String::from("struct SampleError;\n");
    for item in items {
        probe.push_str(&item.to_token_stream().to_string());
        probe.push('\n');
    }
    probe
}

#[test]
fn usize_stream_conversions_preserve_unchanged_map_value_types() {
    let cases = [
        "std::collections::HashMap<usize, std::time::Duration>",
        "std::collections::HashMap<usize, std::path::PathBuf>",
        "std::collections::HashMap<usize, char>",
    ];
    assert_eq!(cases.len(), 3);
    for item_type in cases {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("probe.rs");
        std::fs::write(&input, generated_constructor(item_type)).unwrap();
        let compiler = which::which_in("rustc", std::env::var_os("PATH"), directory.path()).unwrap();
        let output = Command::new(compiler)
            .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
            .arg(&input)
            .arg("--out-dir")
            .arg(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{item_type}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
