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
fn should_error_when_snippet_file_does_not_exist() {
    let err = include_snippet(Path::new("/nonexistent"), "python", "foo.py")
        .expect_err("missing snippet file must be a hard error, not a silent placeholder");
    let message = err.to_string();
    assert!(
        message.contains("python"),
        "error must name the language, got: {message}"
    );
    assert!(
        message.contains("foo.py"),
        "error must name the requested path, got: {message}"
    );
    assert!(
        message.contains("/nonexistent"),
        "error must name the snippets root that was searched, got: {message}"
    );
}

#[test]
fn test_template_version_in_install_command() {
    let tmp = std::env::temp_dir().join("alef_readme_test_version");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

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
        targets: std::collections::HashMap::new(),
    });
    config.workspace_root = Some(tmp.clone());

    let api = test_api();
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
    assert!(mj.get_attr("key").is_ok());
}

#[test]
fn test_generate_readmes_empty_languages() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[]).unwrap();
    assert_eq!(files.len(), 0);
}

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

/// With no `[crates.output] ffi` the `crates/{name}-ffi` convention is the honest fallback, so
/// the FFI crate's package name is `my-lib-ffi` and cargo names its artifact `libmy_lib_ffi.*`.
/// The body must spell the artifact, not the crate name: `lib{name}_ffi` produced the
/// unlinkable hyphenated `libmy-lib_ffi`, which is what liter-llm's committed README says. ~keep
#[test]
fn unconfigured_ffi_readme_links_against_the_underscored_artifact_name() {
    let config = test_config();
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert!(
        files[0].content.contains("Link against `libmy_lib_ffi`"),
        "got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("libmy-lib_ffi"),
        "a hyphenated library name can never name a cargo artifact, got:\n{}",
        files[0].content
    );
}

/// html-to-markdown's shape verbatim: the alef crate is `html-to-markdown-rs`, its FFI crate is
/// `crates/html-to-markdown-ffi`, and cargo emits `libhtml_to_markdown_ffi.{a,dylib}`. Deriving
/// either from the crate name wrote the README into `crates/html-to-markdown-rs-ffi/` — a
/// directory that is not the FFI crate, and which that repo still carries as a tombstone holding
/// nothing but the misplaced README — and told readers to link `libhtml-to-markdown-rs_ffi`,
/// which no build ever produces. Both failures are silent: a misplaced README is not a build
/// error and a wrong link line is only prose. Pinned verbatim for that reason. ~keep
#[test]
fn ffi_readme_follows_configured_output_path_html_to_markdown_shape() {
    let config = config_with_ffi_output("html-to-markdown-rs", "crates/html-to-markdown-ffi/src/");
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/html-to-markdown-ffi/README.md"));
    assert!(
        files[0].content.contains("Link against `libhtml_to_markdown_ffi`"),
        "got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("html-to-markdown-rs-ffi"),
        "the crate-name template must not leak back in, got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("libhtml-to-markdown-rs_ffi"),
        "the crate-name library template must not leak back in, got:\n{}",
        files[0].content
    );
}

/// tree-sitter-language-pack's shape verbatim: the alef crate is `tree-sitter-language-pack`
/// while its FFI crate is `crates/ts-pack-core-ffi`, emitting `libts_pack_core_ffi.*`. The
/// name-derived path and library name share no substring with the real ones. ~keep
#[test]
fn ffi_readme_follows_configured_output_path_tree_sitter_language_pack_shape() {
    let config = config_with_ffi_output("tree-sitter-language-pack", "crates/ts-pack-core-ffi/src/");
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/ts-pack-core-ffi/README.md"));
    assert!(
        files[0].content.contains("Link against `libts_pack_core_ffi`"),
        "got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("tree-sitter-language-pack-ffi"),
        "the crate-name template must not leak back in, got:\n{}",
        files[0].content
    );
}

/// liter-llm's shape: its crate name happens to equal its FFI directory stem, so the README
/// PATH was right by coincidence and this repo proves nothing about the path fix. The library
/// name is a different story — `lib{name}_ffi` yields the hyphenated `libliter-llm_ffi` that is
/// committed in that repo today, while cargo emits `libliter_llm_ffi`. Kept as the case that
/// shows a correct path does not imply a correct body. ~keep
#[test]
fn ffi_readme_liter_llm_shape_has_correct_path_but_needed_the_library_name_fix() {
    let config = config_with_ffi_output("liter-llm", "crates/liter-llm-ffi/src/");
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/liter-llm-ffi/README.md"));
    assert!(
        files[0].content.contains("Link against `libliter_llm_ffi`"),
        "got:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("libliter-llm_ffi"),
        "the hyphenated crate-name library template must not leak back in, got:\n{}",
        files[0].content
    );
}

/// `[crates.ffi] lib_name` is the explicit override every consumer repo actually sets, and it
/// must beat the directory derivation in the README exactly as it does for the Go, Java, Kotlin
/// and Dart backends that link the same artifact. ~keep
#[test]
fn ffi_readme_honors_explicit_lib_name_override() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
lib_name = "custom_artifact"

[crates.output]
ffi = "crates/some-other-ffi/src/"
"#,
    )
    .expect("valid toml");
    let config = cfg.resolve().expect("resolve ok").remove(0);
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();
    assert_eq!(files[0].path, PathBuf::from("crates/some-other-ffi/README.md"));
    assert!(
        files[0].content.contains("Link against `libcustom_artifact`"),
        "got:\n{}",
        files[0].content
    );
}

/// Node and Wasm crates live wherever `[crates.node]`/`[crates.wasm] crate_dir` says, which both
/// discriminating repos set and neither sets to `crates/{crate name}-<lang>`. `package_dir()`
/// already owns that precedence for every other consumer of these directories. ~keep
#[test]
fn node_and_wasm_readmes_follow_configured_crate_dir_not_the_crate_name() {
    for (case, name, stem, node_pkg) in DISCRIMINATING_SHAPES {
        let config = consumer_shape(name, stem, node_pkg);
        let api = test_api();

        let node = generate_readmes(&api, &config, &[Language::Node]).unwrap();
        assert_eq!(node.len(), 1, "case {case}");
        assert_eq!(
            node[0].path,
            PathBuf::from(format!("crates/{stem}-node/README.md")),
            "case {case}"
        );

        let wasm = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();
        assert_eq!(wasm.len(), 1, "case {case}");
        assert_eq!(
            wasm[0].path,
            PathBuf::from(format!("crates/{stem}-wasm/README.md")),
            "case {case}"
        );
        assert_ne!(
            wasm[0].path,
            PathBuf::from(format!("crates/{name}-wasm/README.md")),
            "case {case}: the crate-name template must not name the wasm crate's directory"
        );
    }
}

/// The Wasm analogue of the `lib{name}_ffi` defect: `npm install {crate name}-wasm` names an npm
/// package that does not exist. The published name is scoped — `@xberg-io/liter-llm-wasm`,
/// `@xberg-io/tree-sitter-language-pack-wasm` — and the scope is not derivable from the crate
/// name at all; `wasm_package_name()` defaults it from `node_package_name()`. liter-llm is
/// included precisely because its README path is right by coincidence while its install line was
/// still wrong, which is the whole point: a correct path does not imply a correct body. ~keep
#[test]
fn wasm_readme_installs_the_published_scoped_package_not_the_crate_name() {
    for (case, name, stem, node_pkg) in
        DISCRIMINATING_SHAPES
            .iter()
            .copied()
            .chain([("liter_llm", "liter-llm", "liter-llm", "@xberg-io/liter-llm")])
    {
        let config = consumer_shape(name, stem, node_pkg);
        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Wasm]).unwrap();

        assert!(
            files[0].content.contains(&format!("pnpm add {node_pkg}-wasm")),
            "case {case}, got:\n{}",
            files[0].content
        );
        assert!(
            !files[0].content.contains(&format!("pnpm add {name}-wasm")),
            "case {case}: the unscoped crate-name template installs nothing, got:\n{}",
            files[0].content
        );
        assert!(
            !files[0].content.contains(&format!("from '{name}-wasm'")),
            "case {case}: the import specifier must be the published package too, got:\n{}",
            files[0].content
        );
    }
}

/// The Rust core crate's directory stem is not the alef crate name either: h2m's crate is named
/// `html-to-markdown-rs` but lives at `crates/html-to-markdown/`, tslp's is
/// `tree-sitter-language-pack` at `crates/ts-pack-core/`. `core_crate_dir()` reads the stem from
/// `sources[0]`. The body's `cargo add {name}` stays name-derived on purpose — verified against
/// all three repos, the core crate's Cargo `[package] name` really is the alef crate name, which
/// is why html-to-markdown publishes as `html-to-markdown-rs`. Only the directory was wrong. ~keep
#[test]
fn rust_readme_follows_the_core_crate_directory_not_the_crate_name() {
    for (case, name, stem, node_pkg) in DISCRIMINATING_SHAPES {
        let mut config = consumer_shape(name, stem, node_pkg);
        let expected = format!("crates/{stem}/README.md");
        // The Rust README is skipped unless `[crates.readme.languages.rust]` carries an
        // `output_path`, and the fallback now returns that `output_path` before deriving
        // anything -- so for Rust the two are mutually exclusive by construction and this pins
        // the configured value, not the derivation. It is kept for the shapes it carries: both
        // repos whose crate name differs from their core crate directory, where a path derived
        // from either would be visibly wrong. `crate_readme_path`'s Rust arm is now unreachable
        // from both README routes; see the note there. ~keep
        let mut readme_cfg = crate::core::config::ReadmeConfig {
            template_dir: None,
            snippets_dir: None,
            config: None,
            output_pattern: None,
            discord_url: None,
            banner_url: None,
            languages: std::collections::HashMap::new(),
            targets: std::collections::HashMap::new(),
        };
        readme_cfg
            .languages
            .insert("rust".to_string(), serde_json::json!({ "output_path": &expected }));
        config.readme = Some(readme_cfg);

        let api = test_api();
        let files = generate_readmes(&api, &config, &[Language::Rust]).unwrap();
        assert_eq!(files.len(), 1, "case {case}");
        assert_eq!(files[0].path, PathBuf::from(&expected), "case {case}");
        assert_ne!(
            files[0].path,
            PathBuf::from(format!("crates/{name}/README.md")),
            "case {case}: the crate-name template must not name the core crate's directory"
        );
        assert!(
            files[0].content.contains(&format!("cargo add {name}")),
            "case {case}: the crates.io package name IS the alef crate name, got:\n{}",
            files[0].content
        );
    }
}

/// Resolve a config whose `[crates.readme]` sets no `template_dir`, so every language falls
/// through to the hardcoded generator while the config still says where each README belongs.
/// `[crates.output] ffi` is html-to-markdown's, and the crate name is html-to-markdown's, so the
/// derived FFI path (`crates/html-to-markdown-ffi/`) and the name-derived one
/// (`crates/html-to-markdown-rs-ffi/`, the tombstone that repo still carries) are two distinct
/// wrong answers to compare a configured path against. ~keep
fn config_with_untemplated_readme(readme_toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node", "ruby", "go", "ffi"]

[[crates]]
name = "html-to-markdown-rs"
sources = ["crates/html-to-markdown/src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/xberg-io/html-to-markdown"

[crates.output]
ffi = "crates/html-to-markdown-ffi/src/"

[crates.readme]
{readme_toml}
"#
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

/// A language entry carrying only `output_path` — no `template` — never reaches the templated
/// route: with no `crates.readme.template_dir` set, `try_render_configured_readme` returns
/// `None` for every language and the hardcoded generator runs instead. That generator used to
/// compute its own path and discard the configured one, so a consumer who set `output_path`
/// alone had the README written somewhere else entirely — silently, since a misplaced README is
/// not a build error. It survived because the derived path usually agrees with the configured
/// one; every case below is chosen so the two disagree, and both the `output_path` key and its
/// `output` alias are covered because the resolver accepts either. ~keep
#[test]
fn fallback_honours_configured_output_path_without_a_template_dir() {
    for (case, lang, entry_toml, expected, derived, body_marker) in [
        (
            "python",
            Language::Python,
            "[crates.readme.languages.python]\noutput_path = \"docs/bindings/python/README.md\"",
            "docs/bindings/python/README.md",
            "packages/python/README.md",
            "pip install",
        ),
        (
            "ffi",
            Language::Ffi,
            "[crates.readme.languages.ffi]\noutput_path = \"crates/relocated-ffi/README.md\"",
            "crates/relocated-ffi/README.md",
            "crates/html-to-markdown-ffi/README.md",
            "Link against `libhtml_to_markdown_ffi`",
        ),
        (
            "go_via_the_output_alias",
            Language::Go,
            "[crates.readme.languages.go]\noutput = \"packages/go/v3/README.md\"",
            "packages/go/v3/README.md",
            "packages/go/README.md",
            "go get",
        ),
        (
            "node",
            Language::Node,
            "[crates.readme.languages.typescript]\noutput_path = \"crates/relocated-node/README.md\"",
            "crates/relocated-node/README.md",
            "crates/html-to-markdown-rs-node/README.md",
            "npm install",
        ),
    ] {
        let config = config_with_untemplated_readme(entry_toml);
        let api = test_api();
        let files = generate_readmes(&api, &config, &[lang]).unwrap();

        assert_eq!(files.len(), 1, "case {case}");
        assert_eq!(files[0].path, PathBuf::from(expected), "case {case}");
        assert_ne!(
            files[0].path,
            PathBuf::from(derived),
            "case {case}: the derived path must not win over the configured `output_path`"
        );
        assert!(
            files[0].content.contains(body_marker),
            "case {case}: expected the hardcoded generator's body, got:\n{}",
            files[0].content
        );
    }
}

/// The positive control for the case above: with a `[crates.readme]` present but no configured
/// path for the language — no `output_path`, no `output`, no `output_pattern` — the derivation
/// is still the answer, whether the language has no entry at all or an entry that only carries
/// template variables. Without this, "honour the configured path" could be implemented as
/// "always take some configured path" and nothing would notice. ~keep
#[test]
fn fallback_derives_the_path_when_the_config_names_none() {
    for (case, lang, readme_toml, expected) in [
        (
            "no_entry_at_all",
            Language::Ffi,
            "",
            "crates/html-to-markdown-ffi/README.md",
        ),
        (
            "entry_without_a_path",
            Language::Python,
            "[crates.readme.languages.python]\nname = \"Python\"",
            "packages/python/README.md",
        ),
        (
            "entry_without_a_path_ffi",
            Language::Ffi,
            "[crates.readme.languages.ffi]\nname = \"FFI\"",
            "crates/html-to-markdown-ffi/README.md",
        ),
    ] {
        let config = config_with_untemplated_readme(readme_toml);
        let api = test_api();
        let files = generate_readmes(&api, &config, &[lang]).unwrap();

        assert_eq!(files.len(), 1, "case {case}");
        assert_eq!(files[0].path, PathBuf::from(expected), "case {case}");
    }
}

/// `output_pattern` is the other configured path key and was dropped by the same code path: it
/// applies to every language README, but only the templated route ever consulted it, so a
/// language with no entry of its own silently kept the derived path. liter-llm and xberg both
/// set this key; neither is affected today only because every language they build has an entry.
/// FFI is the discriminating language here — its derivation targets `crates/`, so the pattern's
/// `packages/` answer cannot be reached by accident. ~keep
#[test]
fn fallback_honours_output_pattern_when_no_entry_names_a_path() {
    let config = config_with_untemplated_readme("output_pattern = \"packages/{language}/README.md\"");
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("packages/ffi/README.md"));
}

/// An entry's own `output_path` still beats `output_pattern` in the fallback, the same
/// precedence `paths::readme_output_path` applies on the templated route. ~keep
#[test]
fn fallback_output_path_beats_output_pattern() {
    let config = config_with_untemplated_readme(
        "output_pattern = \"packages/{language}/README.md\"\n\n\
         [crates.readme.languages.ffi]\noutput_path = \"crates/relocated-ffi/README.md\"",
    );
    let api = test_api();
    let files = generate_readmes(&api, &config, &[Language::Ffi]).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, PathBuf::from("crates/relocated-ffi/README.md"));
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
    let mut config = test_config();
    let mut readme_cfg = crate::core::config::ReadmeConfig {
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
    assert_eq!(files.len(), 1);
    assert!(files[0].content.contains("Rust"));
    assert!(files[0].content.contains("cargo add"));
}

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
    assert!(
        files[0].content.contains("https://example.invalid/my-lib"),
        "Expected vendor-neutral placeholder URL, got: {}",
        files[0].content
    );
}

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

#[test]
fn test_extract_code_block_tilde_fence() {
    let md = "~~~python\nprint('hi')\n~~~\n";
    let result = extract_code_block(md);
    assert!(result.contains("~~~python"), "Got: {result}");
    assert!(result.contains("print('hi')"), "Got: {result}");
}

#[test]
fn test_include_snippet_non_md_file() {
    let tmp = std::env::temp_dir().join("alef_readme_snippet_test_py");
    let _ = fs::remove_dir_all(&tmp);
    let lang_dir = tmp.join("python");
    fs::create_dir_all(&lang_dir).unwrap();
    fs::write(lang_dir.join("example.py"), "print('hello')").unwrap();

    let result = include_snippet(&tmp, "python", "example.py").unwrap();
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

    let result = include_snippet(&tmp, "python", "example.md").unwrap();
    assert!(result.contains("```python"), "Got: {result}");
    assert!(result.contains("foo()"), "Got: {result}");

    let _ = fs::remove_dir_all(&tmp);
}
