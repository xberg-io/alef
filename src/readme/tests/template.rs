// Test module: debug output to stderr is expected here. ~keep
#![allow(clippy::print_stdout, clippy::print_stderr)]

use super::*;
use crate::readme::template::{escape_markdown_heading_text, render_performance_table};
use std::fs;
use std::path::PathBuf;

// --- escape_markdown_heading_text: general rule, not just "C#" ---

#[test]
fn should_escape_a_single_trailing_hash() {
    assert_eq!(escape_markdown_heading_text("C#"), "C\\#");
}

#[test]
fn should_escape_every_character_in_a_run_of_trailing_hashes() {
    assert_eq!(escape_markdown_heading_text("Foo##"), "Foo\\#\\#");
}

#[test]
fn should_leave_a_mid_string_hash_untouched() {
    // A `#` that isn't at the end of the name is never ambiguous with an ATX
    // closing sequence, so it must not be escaped.
    assert_eq!(escape_markdown_heading_text("C# (.NET)"), "C# (.NET)");
}

#[test]
fn should_leave_names_without_a_trailing_hash_untouched() {
    for name in ["Python", "Dart / Flutter", "Kotlin (Android)", ""] {
        assert_eq!(escape_markdown_heading_text(name), name);
    }
}

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
    let v = crate::template::to_value(&perf);
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
    let v = crate::template::to_value(&perf);
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

// A language with a `crates.readme.languages.<lang>` entry has explicitly opted
// into template-rendered README content (badges, sections, snippets). If that
// template can't actually be rendered -- a typo'd `template` filename, a missing
// `template_dir`, or any other reason `try_render_configured_readme` comes back
// empty -- generation must fail loudly instead of silently substituting the
// generic hardcoded placeholder and shipping it with the configured content
// missing and no error (#555). This pins the GENERAL rule (any misrendering of an
// explicitly configured language is an error), not a specific broken filename.
#[test]
fn should_fail_loudly_when_configured_language_template_does_not_exist() {
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
    let err = generate_readmes(&api, &config, &[Language::Python])
        .expect_err("a configured-but-unrenderable language must fail generation, not fall back silently");
    let message = err.to_string();
    assert!(
        message.contains("crates.readme.languages.python"),
        "error should name the offending config key, got: {message}"
    );
    assert!(
        !message.to_lowercase().contains("pip install"),
        "error must not carry rendered fallback content, got: {message}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

// The positive counterpart: a language configured with a template that DOES
// render must produce the rendered content, never the generic hardcoded
// placeholder -- pinning that configured sections survive generation.
#[test]
fn should_render_configured_template_content_not_the_generic_fallback() {
    let tmp = std::env::temp_dir().join("alef_readme_test_configured_renders");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(
        tmp.join("lang.md"),
        "# {{ name }}\n\n## What This Package Provides\n\nDistinctive configured section marker.\n",
    )
    .unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "python".to_string(),
        serde_json::json!({
            "template": "lang.md",
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
        files[0].content.contains("Distinctive configured section marker."),
        "expected the configured template's content, got: {}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("pip install"),
        "must not contain the generic hardcoded fallback's install instructions, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

// End-to-end: a `name` ending in a markdown-significant character survives full
// template rendering. Uses "C#" as one concrete instance of the general rule (any
// name ending in `#`), matching the real `crates.readme.languages.csharp` config.
#[test]
fn should_render_a_heading_with_trailing_hash_in_display_name_escaped() {
    let tmp = std::env::temp_dir().join("alef_readme_test_heading_hash");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("lang.md"), "# {{ name }}\n").unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "csharp".to_string(),
        serde_json::json!({
            "template": "lang.md",
            "name": "C#",
            "output_path": "packages/csharp/README.md"
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
    let files = generate_readmes(&api, &config, &[Language::Csharp]).unwrap();
    assert_eq!(files.len(), 1);
    // README output now carries a self-embedded HTML-comment header ahead of
    // the rendered body (see `template.rs`'s `~keep` note), so the escaped
    // heading is no longer the first line of `content`.
    assert!(
        files[0].content.contains("# C\\#"),
        "expected the heading's trailing `#` to be backslash-escaped so it can't be \
         mistaken for an ATX closing sequence, got: {}",
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

/// A templated FFI README with no `output_path` falls through to `paths::default_readme_path`,
/// a second copy of the FFI directory rule that drifted from the resolved config exactly as
/// `fallback.rs` did. Both consumer shapes where the crate name differs from the FFI directory
/// are pinned here, because the case where they agree (liter-llm) cannot distinguish the two
/// derivations. ~keep
#[test]
fn ffi_template_default_path_follows_configured_output_not_the_crate_name() {
    for (case, crate_name, ffi_output, expected) in [
        (
            "html_to_markdown",
            "html-to-markdown-rs",
            "crates/html-to-markdown-ffi/src/",
            "crates/html-to-markdown-ffi/README.md",
        ),
        (
            "tree_sitter_language_pack",
            "tree-sitter-language-pack",
            "crates/ts-pack-core-ffi/src/",
            "crates/ts-pack-core-ffi/README.md",
        ),
    ] {
        let tmp = std::env::temp_dir().join(format!("alef_readme_ffi_default_path_{case}"));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("t.md"), "hello").unwrap();

        let mut config = config_with_ffi_output(crate_name, ffi_output);
        let mut lang_map = std::collections::HashMap::new();
        lang_map.insert("ffi".to_string(), serde_json::json!({ "template": "t.md" }));
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
        assert_eq!(files.len(), 1, "case {case}");
        assert_eq!(files[0].path, PathBuf::from(expected), "case {case}");
        assert!(
            !files[0].path.to_string_lossy().contains(&format!("{crate_name}-ffi")),
            "case {case}: the crate-name template must not leak back in, got {:?}",
            files[0].path
        );

        let _ = fs::remove_dir_all(&tmp);
    }
}

/// The templated route's default path for Node and Wasm must follow `[crates.node]`/
/// `[crates.wasm] crate_dir` for the same reason FFI must follow `[crates.output] ffi`. Both
/// discriminating repos set `crate_dir`, and for both the crate-name formula names a directory
/// that does not exist. ~keep
#[test]
fn node_and_wasm_template_default_paths_follow_configured_crate_dir() {
    for (case, name, stem, node_pkg) in DISCRIMINATING_SHAPES {
        for (lang, lang_key, suffix) in [(Language::Node, "typescript", "node"), (Language::Wasm, "wasm", "wasm")] {
            let tmp = std::env::temp_dir().join(format!("alef_readme_default_path_{case}_{suffix}"));
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp).unwrap();
            fs::write(tmp.join("t.md"), "hello").unwrap();

            let mut config = consumer_shape(name, stem, node_pkg);
            let mut lang_map = std::collections::HashMap::new();
            lang_map.insert(lang_key.to_string(), serde_json::json!({ "template": "t.md" }));
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
            let files = generate_readmes(&api, &config, &[lang]).unwrap();
            assert_eq!(files.len(), 1, "case {case}/{suffix}");
            assert_eq!(
                files[0].path,
                PathBuf::from(format!("crates/{stem}-{suffix}/README.md")),
                "case {case}/{suffix}"
            );
            assert_ne!(
                files[0].path,
                PathBuf::from(format!("crates/{name}-{suffix}/README.md")),
                "case {case}/{suffix}: the crate-name template must not leak back in"
            );

            let _ = fs::remove_dir_all(&tmp);
        }
    }
}

/// An explicit `output_path` still wins over the derived default — the escape hatch
/// tree-sitter-language-pack currently uses to work around the defect must keep working. ~keep
#[test]
fn ffi_template_explicit_output_path_still_wins_over_the_derived_default() {
    let tmp = std::env::temp_dir().join("alef_readme_ffi_explicit_output_path");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("t.md"), "hello").unwrap();

    let mut config = config_with_ffi_output("tree-sitter-language-pack", "crates/ts-pack-core-ffi/src/");
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "ffi".to_string(),
        serde_json::json!({ "template": "t.md", "output_path": "crates/elsewhere/README.md" }),
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
    assert_eq!(files[0].path, PathBuf::from("crates/elsewhere/README.md"));

    let _ = fs::remove_dir_all(&tmp);
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
fn should_error_loudly_when_template_uses_snippets_but_none_are_configured() {
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
    let err = generate_readmes(&api, &config, &[Language::Ffi]).expect_err(
        "a template that calls include_snippet with no snippets_dir configured must fail, not render a placeholder",
    );
    let message = err.to_string();
    assert!(
        message.contains("crates.readme.snippets_dir"),
        "error must name the missing config key, got: {message}"
    );

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

// --- crates.readme.languages.<name>.snippet_language: cross-directory alias ---
//
// A README language may borrow its snippets from a differently-named snippet
// directory (e.g. `ffi` pulling from a `c/` root, since the FFI binding's
// examples *are* C code and a consumer repo already maintains one `c/`
// snippet set rather than a duplicate `ffi/` one). Regression coverage for
// the incident where xberg's `[crates.readme.languages.ffi]` entry had no
// snippet source at all: `docs-site/src/snippets/` only ever had a `c/`
// directory, never an `ffi/` one, so every `include_snippet(language)` call
// in `partials/quick_start.md.jinja` failed once alef stopped silently
// swallowing missing snippets (see `include_snippet`'s `~keep` doc comment). ~keep

#[test]
fn test_readme_snippet_language_alias_resolves_from_aliased_directory() {
    let tmp = std::env::temp_dir().join("alef_readme_test_snippet_language_alias");
    let _ = fs::remove_dir_all(&tmp);
    let snippets_dir = tmp.join("snippets");
    // Only a `c/` snippet directory exists — no `ffi/` directory anywhere.
    let c_snippet_dir = snippets_dir.join("c");
    fs::create_dir_all(&c_snippet_dir).unwrap();
    fs::write(c_snippet_dir.join("hello.c"), "int main(void) { return 0; }").unwrap();
    fs::write(tmp.join("t.md"), r#"{{ "hello.c" | include_snippet(language) }}"#).unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "ffi".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "crates/my-lib-ffi/README.md",
            "snippet_language": "c"
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
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap_or_else(|err| {
        panic!("expected `snippet_language = \"c\"` to resolve the ffi README's snippets from the `c/` directory, got error: {err}")
    });
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("int main(void) { return 0; }"),
        "Expected the aliased `c/hello.c` snippet content, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_readme_snippet_language_alias_does_not_affect_explicit_language_calls() {
    let tmp = std::env::temp_dir().join("alef_readme_test_snippet_language_alias_explicit");
    let _ = fs::remove_dir_all(&tmp);
    let snippets_dir = tmp.join("snippets");
    let c_snippet_dir = snippets_dir.join("c");
    let python_snippet_dir = snippets_dir.join("python");
    fs::create_dir_all(&c_snippet_dir).unwrap();
    fs::create_dir_all(&python_snippet_dir).unwrap();
    fs::write(c_snippet_dir.join("hello.c"), "int main(void) { return 0; }").unwrap();
    fs::write(python_snippet_dir.join("hello.py"), "print('hi')").unwrap();
    // The own-language lookup (`language`) must resolve via the alias to `c/`,
    // while an explicit literal request for a different language's snippet
    // (as a comparison callout might do) must be honoured verbatim. ~keep
    fs::write(
        tmp.join("t.md"),
        r#"{{ "hello.c" | include_snippet(language) }}
{{ "hello.py" | include_snippet("python") }}"#,
    )
    .unwrap();

    let mut config = test_config();
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "ffi".to_string(),
        serde_json::json!({
            "template": "t.md",
            "output_path": "crates/my-lib-ffi/README.md",
            "snippet_language": "c"
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
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content.contains("int main(void) { return 0; }"),
        "Expected the aliased `c/hello.c` snippet content, got: {}",
        files[0].content
    );
    assert!(
        files[0].content.contains("print('hi')"),
        "Expected the explicitly-requested `python/hello.py` snippet content, got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn should_error_when_configured_snippets_dir_does_not_exist_even_if_unused() {
    let tmp = std::env::temp_dir().join("alef_readme_test_snippets_dir_missing");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    // No `snippets/` directory is created under `tmp`, and the template below
    // never references the `include_snippet` filter: the missing directory must
    // still fail the build, matching the incident where `readme_templates/rust.md`
    // never called the filter yet `snippets_dir` pointed at a nonexistent path. ~keep
    fs::write(tmp.join("t.md"), "# {{ name }}").unwrap();

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
        snippets_dir: Some(PathBuf::from("docs/snippets")),
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: lang_map,
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
    let err = generate_readmes(&api, &config, &[Language::Python])
        .expect_err("a configured snippets_dir that does not exist on disk must be a hard error");
    let message = err.to_string();
    assert!(
        message.contains("crates.readme.snippets_dir"),
        "error must name the config key, got: {message}"
    );
    assert!(
        message.contains("docs/snippets"),
        "error must name the offending configured path, got: {message}"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn should_error_when_referenced_snippet_file_cannot_be_resolved() {
    let tmp = std::env::temp_dir().join("alef_readme_test_snippet_file_missing");
    let _ = fs::remove_dir_all(&tmp);
    let snippets_dir = tmp.join("snippets");
    fs::create_dir_all(snippets_dir.join("python")).unwrap();
    // `snippets_dir` itself exists, but `missing.py` is never written under it.
    fs::write(tmp.join("t.md"), r#"{{ "missing.py" | include_snippet("python") }}"#).unwrap();

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
    let err = generate_readmes(&api, &config, &[Language::Python])
        .expect_err("an unresolvable snippet reference must fail the build, not render a placeholder comment");
    let message = err.to_string();
    assert!(
        message.contains("python"),
        "error must name the language, got: {message}"
    );
    assert!(
        message.contains("missing.py"),
        "error must name the path, got: {message}"
    );
    assert!(
        message.to_lowercase().contains("snippets"),
        "error must name the snippets root that was searched, got: {message}"
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

// --- `install_command` may be configured without being the template's rendering source ---
//
// A README language can legitimately set `install_command` while its template's installation
// partial renders a *different*, hand-written, equivalent snippet instead (swift's SwiftPM
// `.binaryTarget` block instead of the unsupported `.package(url:, from:)` form in
// `install_command`; kotlin_android's single-quoted Gradle `implementation '...'` instead of
// the double-quoted `implementation("...")` in `install_command`). `install_command` is a
// convenience value some templates read and others intentionally supersede with better,
// language-idiomatic content -- generation must not fail just because the literal configured
// string doesn't appear verbatim in output the template deliberately wrote differently.
#[test]
fn should_not_fail_when_a_configured_install_command_is_intentionally_not_rendered_verbatim() {
    let tmp = std::env::temp_dir().join("alef_readme_superseded_install_command_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    // The template never references `install_command` -- it has its own hardcoded, equivalent
    // instructions, mirroring the real swift/kotlin_android templates.
    fs::write(
        tmp.join("test.md"),
        "# {{ name }}\n\n## Installation\n\n```gradle\nimplementation '{{ package_name }}:{{ version }}'\n```\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(tmp.clone());
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "kotlin_android".to_string(),
        serde_json::json!({
            "template": "test.md",
            "package_name": "io.xberg.literllm:liter-llm-android",
            "install_command": "implementation(\"io.xberg.literllm:liter-llm-android:{{ version }}\")",
            "output_path": "packages/kotlin-android/README.md"
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

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::KotlinAndroid])
        .expect("a template that supersedes install_command with its own content must still succeed");
    assert!(
        files[0]
            .content
            .contains("implementation 'io.xberg.literllm:liter-llm-android:0.1.0'"),
        "Got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn should_pass_when_install_command_is_configured_and_the_template_renders_it() {
    let tmp = std::env::temp_dir().join("alef_readme_present_install_command_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    fs::write(
        tmp.join("test.md"),
        "# {{ name }}\n\n## Installation\n\n```bash\n{{ install_command }}\n```\n",
    )
    .unwrap();

    let mut config = test_config();
    config.workspace_root = Some(tmp.clone());
    let mut lang_map = std::collections::HashMap::new();
    lang_map.insert(
        "zig".to_string(),
        serde_json::json!({
            "template": "test.md",
            "install_command": "zig fetch --save https://example.com/v{{ version }}.tar.gz",
            "output_path": "packages/zig/README.md"
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

    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Zig]).unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files[0]
            .content
            .contains("zig fetch --save https://example.com/v0.1.0.tar.gz"),
        "Got: {}",
        files[0].content
    );

    let _ = fs::remove_dir_all(&tmp);
}
