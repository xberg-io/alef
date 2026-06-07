use super::super::*;
use super::*;
use crate::readme::fallback::capitalize_first;
use crate::readme::template::{extract_code_block, include_snippet, json_to_minijinja_value, render_performance_table};
use minijinja::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn test_generate_python_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/python/README.md"));
    assert!(files[0].content.contains("Python"));
    assert!(files[0].content.contains("pip install"));
}

#[test]
fn test_generate_node_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Node]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-node/README.md"));
    assert!(files[0].content.contains("Node.js"));
}

#[test]
fn test_generate_multiple_readmes() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python, Language::Node]).unwrap();
    assert_eq!(files.len(), 2);
}

#[test]
fn test_extract_code_block() {
    let md = "Some text\n\n```python\nprint('hello')\n```\n\nMore text";
    let result = extract_code_block(md);
    assert!(result.contains("```python"));
    assert!(result.contains("print('hello')"));
}

#[test]
fn test_extract_code_block_no_block() {
    let md = "Just plain text";
    let result = extract_code_block(md);
    assert_eq!(result, "Just plain text");
}

#[test]
fn test_render_performance_table_empty() {
    let v = Value::from(Vec::<Value>::new());
    let result = render_performance_table(&v, "test");
    assert!(result.is_empty());
}

#[test]
fn test_include_snippet_missing() {
    let result = include_snippet(Path::new("/nonexistent"), "python", "foo.py");
    assert!(result.contains("snippet not found"));
}

#[test]
fn test_template_version_in_install_command() {
    // Verify that {{ version }} placeholders in per-language config strings
    // (e.g. Java pom.xml snippet, Elixir mix.exs dep) are rendered.
    let tmp = std::env::temp_dir().join("alef_readme_test_version");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    // Minimal template that just outputs the install_command
    fs::write(tmp.join("test.md"), "{{ install_command }}").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "java".to_string(),
        serde_json::json!({
            "template": "test.md",
            "install_command": "<version>{{ version }}</version>",
            "output_path": "packages/java/README.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: lang_map,
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api(); // version = "0.1.0"
    let files = generate_readmes(&api, &config, &[Language::Java]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("<version>0.1.0</version>"),
        "Expected version placeholder to be rendered, got: {}",
        files[0].content,
    );
    assert!(
        !files[0].content.contains("{{ version }}"),
        "Raw template placeholder should not remain in output",
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_json_to_minijinja_value_primitives() {
    let json: serde_json::Value = serde_json::from_str(r#"{"key": "value", "num": 42, "flag": true}"#).unwrap();
    let mj = json_to_minijinja_value(&json);
    // The value should be an object accessible by attribute
    assert!(mj.get_attr("key").is_ok());
}

// --- generate_readmes: empty language list ---

#[test]
fn test_generate_readmes_empty_languages() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[]).unwrap();
    assert_eq!(files.len(), 0);
}

// --- hardcoded fallback: remaining language variants ---

#[test]
fn test_generate_ruby_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ruby]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/ruby/README.md"));
    assert!(files[0].content.contains("Ruby"));
    assert!(files[0].content.contains("gem install"));
}

#[test]
fn test_generate_php_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Php]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/php/README.md"));
    assert!(files[0].content.contains("PHP"));
    assert!(files[0].content.contains("composer require"));
}

#[test]
fn test_generate_elixir_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Elixir]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/elixir/README.md"));
    assert!(files[0].content.contains("Elixir"));
    assert!(files[0].content.contains("mix.exs"));
}

#[test]
fn test_generate_go_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Go]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/go/README.md"));
    assert!(files[0].content.contains("Go"));
    assert!(files[0].content.contains("go get"));
}

#[test]
fn test_generate_java_readme_hardcoded() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Java]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/java/README.md"));
    assert!(files[0].content.contains("Java"));
    assert!(files[0].content.contains("pom.xml"));
}

#[test]
fn test_generate_csharp_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Csharp]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/csharp/README.md"));
    assert!(files[0].content.contains("C#"));
    assert!(files[0].content.contains("dotnet add package"));
}

#[test]
fn test_generate_ffi_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-ffi/README.md"));
    assert!(files[0].content.contains("FFI"));
}

#[test]
fn test_generate_wasm_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-wasm/README.md"));
    assert!(files[0].content.contains("WebAssembly"));
}

#[test]
fn test_generate_r_readme() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::R]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/r/README.md"));
    assert!(files[0].content.contains("install.packages"));
}

#[test]
fn test_generate_rust_readme_skipped_by_default() {
    // Rust is the source crate — the canonical README lives at
    // `crates/<name>/README.md` and is hand-managed. alef must NOT emit a
    // `packages/rust/README.md` stub unless the consumer explicitly opts in.
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
    assert!(
        files.is_empty(),
        "Rust README should be skipped by default, got: {:?}",
        files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn test_generate_rust_readme_emitted_when_explicitly_configured() {
    // When `[readme.languages.rust].output_path` is set, alef honors the
    // explicit opt-in and emits the README at that path.
    let mut config = test_config();
    let mut readme_cfg = crate::core::config::ReadmeConfig {
        template_dir: None,
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::new(),
    };
    readme_cfg.languages.insert(
        "rust".to_string(),
        serde_json::json!({ "output_path": "crates/my-lib/README.md" }),
    );
    config.readme = Some(readme_cfg);
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
    assert_eq!(files.len(), 1);
    // Hardcoded fallback ignores output_path (it only honors output_pattern via
    // readme_output_path which is only called from the template path). The
    // opt-in gate is what matters here — the file is emitted, not silently dropped.
    assert!(files[0].content.contains("Rust"));
    assert!(files[0].content.contains("cargo add"));
}

// --- hardcoded fallback: no scaffold config (default description/repository) ---

#[test]
fn test_generate_readme_without_scaffold_uses_placeholder() {
    let mut config = test_config();
    config.scaffold = None;
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("Bindings for my-lib"),
        "Expected default description, got: {}",
        files[0].content
    );
    // When [scaffold] repository is unset, the README falls back to a
    // vendor-neutral placeholder (`example.invalid`) rather than smuggling
    // another organization's URL into the output.
    assert!(
        files[0].content.contains("https://example.invalid/my-lib"),
        "Expected vendor-neutral placeholder URL, got: {}",
        files[0].content
    );
}

// --- capitalize_first ---

#[test]
fn test_capitalize_first_normal() {
    assert_eq!(capitalize_first("hello"), "Hello");
}

#[test]
fn test_capitalize_first_empty() {
    assert_eq!(capitalize_first(""), "");
}

#[test]
fn test_capitalize_first_already_upper() {
    assert_eq!(capitalize_first("World"), "World");
}

// --- extract_code_block: tilde fence ---

#[test]
fn test_extract_code_block_tilde_fence() {
    let md = "~~~python\nprint('hi')\n~~~\n";
    let result = extract_code_block(md);
    assert!(result.contains("~~~python"), "Got: {result}");
    assert!(result.contains("print('hi')"), "Got: {result}");
}

// --- include_snippet: file exists ---

#[test]
fn test_include_snippet_non_md_file() {
    let tmp = std::env::temp_dir().join("alef_readme_snippet_test_py");
    let _ = fs::remove_dir_all(&tmp);
    let lang_dir = tmp.join("python");
    fs::create_dir_all(&lang_dir).unwrap();
    fs::write(lang_dir.join("example.py"), "print('hello')").unwrap();

    let result = include_snippet(&tmp, "python", "example.py");
    assert!(result.contains("```py"), "Got: {result}");
    assert!(result.contains("print('hello')"), "Got: {result}");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_include_snippet_md_file_extracts_code_block() {
    let tmp = std::env::temp_dir().join("alef_readme_snippet_test_md");
    let _ = fs::remove_dir_all(&tmp);
    let lang_dir = tmp.join("python");
    fs::create_dir_all(&lang_dir).unwrap();
    fs::write(
        lang_dir.join("example.md"),
        "Some prose\n\n```python\nfoo()\n```\n\nMore prose",
    )
    .unwrap();

    let result = include_snippet(&tmp, "python", "example.md");
    assert!(result.contains("```python"), "Got: {result}");
    assert!(result.contains("foo()"), "Got: {result}");

    let _ = fs::remove_dir_all(&tmp);
}
