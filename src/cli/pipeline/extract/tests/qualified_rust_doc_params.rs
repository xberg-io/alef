use super::{sanitize_unknown_types, strip_binding_excluded};

#[test]
fn qualified_rust_only_enum_sample_from_extraction_typechecks_with_an_ambiguous_short_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lib_rs = dir.path().join("lib.rs");
    let source = r#"
        pub mod one {
            #[cfg_attr(alef, alef(skip))]
            pub enum Policy { One }
        }

        pub mod two {
            #[cfg_attr(alef, alef(skip))]
            pub enum Policy { Two }
        }

        #[cfg_attr(alef, alef(skip))]
        pub fn choose(policy: two::Policy) {
            let _ = policy;
        }
        "#;
    std::fs::write(&lib_rs, source).expect("write fixture");

    let mut api =
        crate::extract::extractor::extract(&[lib_rs.as_path()], "sample", "0.0.0", None).expect("extract fixture");
    strip_binding_excluded(&mut api).expect("strip binding exclusions");
    sanitize_unknown_types(&mut api);

    let choose = api
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .expect("choose function");
    assert_eq!(choose.params[0].original_type.as_deref(), Some("two::Policy"));

    let config: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
        [workspace]
        languages = ["rust"]

        [[crates]]
        name = "sample"
        sources = ["src/lib.rs"]
        "#,
    )
    .expect("valid config");
    let config = config.resolve().expect("resolved config").remove(0);
    let files = crate::docs::generate_docs(&api, &config, &[crate::core::config::Language::Rust], "out")
        .expect("generate docs");
    let rust = files
        .iter()
        .find(|file| file.path.ends_with("api-rust.md"))
        .expect("Rust reference");
    let snippet = rust
        .content
        .split("```rust")
        .skip(1)
        .find(|block| block.contains("choose(") && block.contains(';'))
        .and_then(|block| block.split("```").next())
        .expect("choose example");
    assert!(
        snippet.contains("choose(two::Policy::Two);") || snippet.contains("choose(sample::two::Policy::Two);"),
        "qualified example must select the second enum: {snippet}"
    );

    let library = dir.path().join("libsample.rlib");
    let library_output = std::process::Command::new("rustc")
        .current_dir(dir.path())
        .args(["--edition=2024", "--crate-name", "sample", "--crate-type=lib"])
        .arg(&lib_rs)
        .arg("-o")
        .arg(&library)
        .output()
        .expect("run rustc for fixture library");
    assert!(
        library_output.status.success(),
        "fixture library must compile: {}",
        String::from_utf8_lossy(&library_output.stderr)
    );

    let main_rs = dir.path().join("main.rs");
    std::fs::write(&main_rs, format!("use sample::*;\nfn main() {{\n{snippet}\n}}\n"))
        .expect("write generated example");
    let example_output = std::process::Command::new("rustc")
        .current_dir(dir.path())
        .args(["--edition=2024"])
        .arg(&main_rs)
        .arg("--extern")
        .arg(format!("sample={}", library.display()))
        .arg("-o")
        .arg(dir.path().join("example"))
        .output()
        .expect("run rustc for generated example");
    assert!(
        example_output.status.success(),
        "generated Rust example must typecheck: {}\n{snippet}",
        String::from_utf8_lossy(&example_output.stderr)
    );
}
