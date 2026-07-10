use super::super::*;
use super::*;
use crate::readme::template::render_performance_table;
use minijinja::Value;
use std::fs;
use std::path::PathBuf;

// --- render_performance_table: ops/sec table ---

#[test]
fn test_render_performance_table_ops_sec() {
    let perf = serde_json::json!({
        "platform": "Apple M2",
        "function": "parse",
        "note": "single-threaded",
        "benchmarks": [
            {"name": "small.json", "size": "1 KB", "ops_sec": 12345},
            {"name": "large.json", "size": "1 MB", "ops_sec": 42}
        ]
    });
    let v = Value::from_serialize(&perf);
    let result = render_performance_table(&v, "parse");
    assert!(result.contains("Apple M2"), "Got: {result}");
    assert!(result.contains("| Document | Size | Ops/sec |"), "Got: {result}");
    assert!(result.contains("small.json"), "Got: {result}");
    assert!(result.contains("large.json"), "Got: {result}");
}

#[test]
fn test_render_performance_table_throughput() {
    let perf = serde_json::json!({
        "platform": "Linux x86-64",
        "function": "extract",
        "note": "4 threads",
        "benchmarks": [
            {
                "name": "doc.pdf",
                "size": "2 MB",
                "latency": "10ms",
                "throughput": "100 MB/s"
            }
        ]
    });
    let v = Value::from_serialize(&perf);
    let result = render_performance_table(&v, "extract");
    assert!(
        result.contains("| Document | Size | Latency | Throughput |"),
        "Got: {result}"
    );
    assert!(result.contains("doc.pdf"), "Got: {result}");
    assert!(result.contains("100 MB/s"), "Got: {result}");
    assert!(
        result.contains("4 threads\n\n| Document"),
        "Expected blank line between context and table header. Got: {result}"
    );
}

#[test]
fn test_template_with_output_pattern() {
    let tmp = std::env::temp_dir().join("alef_readme_test_output_pattern");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("lang.md"), "# {{ name }}").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "lang.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: Some("docs/{language}/README.md".to_string()),
        discord_url: None,
        banner_url: None,
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("docs/python/README.md"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_missing_template_falls_back() {
    let tmp = std::env::temp_dir().join("alef_readme_test_missing_tmpl");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "nonexistent.md",
            "output_path": "packages/python/README.md"
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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("pip install"),
        "Expected hardcoded fallback content, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_no_lang_entry_falls_back() {
    let tmp = std::env::temp_dir().join("alef_readme_test_no_lang_entry");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    let mut config = test_config();
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::new(),
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].content.contains("pip install"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_yaml_config() {
    let tmp = std::env::temp_dir().join("alef_readme_test_yaml_cfg");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(tmp.join("tmpl.md"), "version={{ version }}").unwrap();
    let yaml_content = r#"
languages:
  python:
    template: tmpl.md
    output_path: packages/python/README.md
"#;
    fs::write(tmp.join("readme.yaml"), yaml_content).unwrap();

    let mut config = test_config();
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: Some(PathBuf::from("readme.yaml")),
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::new(),
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("version=0.1.0"),
        "Expected rendered version, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_discord_and_banner_url() {
    let tmp = std::env::temp_dir().join("alef_readme_test_discord_banner");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(tmp.join("t.md"), "{{ discord_url }}|{{ banner_url }}").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "packages/python/README.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: Some("https://discord.gg/test".to_string()),
        banner_url: Some("https://img.example.com/banner.png".to_string()),
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("https://discord.gg/test"),
        "Got: {}",
        files[0].content
    );
    assert!(
        files[0].content.contains("https://img.example.com/banner.png"),
        "Got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_no_scaffold_uses_defaults() {
    let tmp = std::env::temp_dir().join("alef_readme_test_no_scaffold");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(tmp.join("t.md"), "{{ description }}|{{ repository }}|{{ license }}").unwrap();

    let mut config = test_config();
    config.scaffold = None;
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "packages/python/README.md"
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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("Bindings for my-lib"),
        "Got: {}",
        files[0].content
    );
    assert!(
        files[0].content.contains("https://example.invalid/my-lib"),
        "Got: {}",
        files[0].content
    );
    assert!(files[0].content.contains("MIT"), "Got: {}", files[0].content);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_trailing_newline_not_doubled() {
    let tmp = std::env::temp_dir().join("alef_readme_test_trailing_newline");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(tmp.join("t.md"), "hello\n").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "packages/python/README.md"
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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].content.ends_with('\n'), "Must end with newline");
    assert!(
        !files[0].content.ends_with("\n\n"),
        "Must not have double trailing newline, got: {:?}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_default_readme_path_ffi() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-ffi/README.md"));
}

#[test]
fn test_default_readme_path_wasm() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-wasm/README.md"));
}

#[test]
fn test_default_readme_path_node() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Node]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib-node/README.md"));
}

#[test]
fn test_default_readme_path_rust_when_explicitly_configured() {
    let mut config = test_config();
    let mut readme_cfg = ReadmeConfig {
        template_dir: None,
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::new(),
        targets: std::collections::HashMap::new(),
    };
    readme_cfg.languages.insert(
        "rust".to_string(),
        serde_json::json!({ "output_path": "crates/my-lib/README.md" }),
    );
    config.readme = Some(readme_cfg);
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/my-lib/README.md"));
}

#[test]
fn test_readme_target_root_and_rust_readme_are_generated() {
    let tmp = std::env::temp_dir().join("alef_readme_test_root_target");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("root.md"), "# {{ name }} root").unwrap();
    fs::write(tmp.join("rust.md"), "# {{ name }} rust").unwrap();

    let mut config = test_config();
    let mut languages = std::collections::HashMap::new();
    languages.insert(
        "rust".to_string(),
        serde_json::json!({
            "template": "rust.md",
            "output_path": "crates/my-lib/README.md"
        }),
    );
    let mut targets = std::collections::HashMap::new();
    targets.insert(
        "root".to_string(),
        serde_json::json!({
            "template": "root.md",
            "output_path": "README.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages,
        targets,
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
    let paths = files.iter().map(|file| file.path.clone()).collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![PathBuf::from("crates/my-lib/README.md"), PathBuf::from("README.md")]
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_readme_target_requires_output_path() {
    let tmp = std::env::temp_dir().join("alef_readme_test_root_target_output");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("root.md"), "# {{ name }} root").unwrap();

    let mut config = test_config();
    let mut targets = std::collections::HashMap::new();
    targets.insert("root".to_string(), serde_json::json!({ "template": "root.md" }));
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: std::collections::HashMap::new(),
        targets,
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let err = generate_readmes(&api, &config, &[]).unwrap_err();
    assert!(
        err.to_string().contains("requires `output_path` or `output`"),
        "unexpected error: {err}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_readme_duplicate_output_path_is_rejected() {
    let tmp = std::env::temp_dir().join("alef_readme_test_root_target_duplicate");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("root.md"), "# {{ name }} root").unwrap();
    fs::write(tmp.join("lang.md"), "# {{ name }} lang").unwrap();

    let mut config = test_config();
    let mut languages = std::collections::HashMap::new();
    languages.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "lang.md",
            "output_path": "README.md"
        }),
    );
    let mut targets = std::collections::HashMap::new();
    targets.insert(
        "root".to_string(),
        serde_json::json!({
            "template": "root.md",
            "output_path": "README.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages,
        targets,
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let err = generate_readmes(&api, &config, &[Language::Python]).unwrap_err();
    assert!(
        err.to_string().contains("duplicate README output path"),
        "unexpected error: {err}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_output_key_alias() {
    let tmp = std::env::temp_dir().join("alef_readme_test_output_alias");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("t.md"), "hello").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output": "custom/path/README.md"
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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("custom/path/README.md"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_default_path_fallthrough() {
    let tmp = std::env::temp_dir().join("alef_readme_test_default_path");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("t.md"), "hello").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert("python".to_string(), serde_json::json!({ "template": "t.md" }));
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/python/README.md"));

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_readme_missing_snippets_renders_gracefully() {
    let tmp = std::env::temp_dir().join("alef_readme_test_missing_snippets");
    let _ = fs::remove_dir_all(&tmp);
    let partials_dir = tmp.join("partials");
    fs::create_dir_all(&partials_dir).unwrap();

    fs::write(
        partials_dir.join("quick_start.md.jinja"),
        "{{ snippets.basic_extraction | include_snippet(language) }}",
    )
    .unwrap();
    fs::write(
        tmp.join("language_package.md"),
        "{% include 'partials/quick_start.md.jinja' %}",
    )
    .unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "ffi".to_string(),
        serde_json::json!({
            "template": "language_package.md",
            "output_path": "crates/my-lib-ffi/README.md",
            "name": "FFI"
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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files.len(), 1);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_template_include_snippet_filter() {
    let tmp = std::env::temp_dir().join("alef_readme_test_snippet_filter");
    let _ = fs::remove_dir_all(&tmp);
    let snippets_dir = tmp.join("snippets");
    let lang_snippet_dir = snippets_dir.join("python");
    fs::create_dir_all(&lang_snippet_dir).unwrap();
    fs::write(lang_snippet_dir.join("hello.py"), "print('hi')").unwrap();
    fs::write(tmp.join("t.md"), r#"{{ "hello.py" | include_snippet("python") }}"#).unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "packages/python/README.md"
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(tmp.clone()),
        snippets_dir: Some(PathBuf::from("snippets")),
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("print('hi')"),
        "Expected snippet content, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_alef_all_and_cold_readme_produce_same_output() {
    let tmp = std::env::temp_dir().join("alef_sty5_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::create_dir_all(tmp.join("templates")).unwrap();

    let template_content = r#"# {{name}}

{{description}}

## Features

- Item 1
- Item 2

{% if performance %}
## Performance

{{ performance | render_performance_table(name) }}
{% endif %}

## Installation

{{ install_command }}
"#;
    fs::write(tmp.join("templates/test.md"), template_content).unwrap();

    let mut config = test_config();
    config.workspace_root = Some(tmp.clone());

    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "test.md",
            "output_path": "packages/python/README.md",
            "install_command": "pip install my-lib==0.1.0",
            "performance": {
                "platform": "Apple M4",
                "function": "convert()",
                "note": "Test doc",
                "benchmarks": [
                    {
                        "name": "Small",
                        "size": "10KB",
                        "latency": "1.0ms",
                        "throughput": "10 MB/s"
                    },
                    {
                        "name": "Large",
                        "size": "1MB",
                        "latency": "10.0ms",
                        "throughput": "100 MB/s"
                    }
                ]
            }
        }),
    );
    config.readme = Some(ReadmeConfig {
        template_dir: Some(PathBuf::from("templates")),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });

    let api = test_api();

    let cold_files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(cold_files.len(), 1);
    let cold_content = &cold_files[0].content;

    let warm_files = generate_readmes(&api, &config, &[Language::Python]).unwrap();
    assert_eq!(warm_files.len(), 1);
    let warm_content = &warm_files[0].content;

    if cold_content != warm_content {
        eprintln!("=== COLD OUTPUT ===\n{}\n", cold_content);
        eprintln!("=== WARM OUTPUT ===\n{}\n", warm_content);
        eprintln!("=== DIFF (cold vs warm) ===");
        for (i, (c, w)) in cold_content.lines().zip(warm_content.lines()).enumerate() {
            if c != w {
                eprintln!("Line {}: COLD: {}", i + 1, c);
                eprintln!("Line {}: WARM: {}", i + 1, w);
            }
        }
    }
    assert_eq!(
        cold_content, warm_content,
        "README generation must be deterministic: alef readme and alef all must produce identical output (STY-5 regression)"
    );

    let _ = fs::remove_dir_all(&tmp);
}
