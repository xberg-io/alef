//! Regression test for blank-line spacing in Elixir streaming-adapter wrappers.
//! Ensures `mix format --check-formatted` passes by verifying blank lines exist
//! between top-level function definitions and @doc attributes.

use alef::backends::rustler::RustlerBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;

fn make_config_with_multiple_adapters(app_name: &str) -> ResolvedCrateConfig {
    let crate_name = app_name.replace('_', "-");
    let toml = format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.elixir]
app_name = "{app_name}"

[[crates.adapters]]
name = "crawl_stream"
pattern = "streaming"
core_path = "core::crawl_stream"
owner_type = "CrawlEngine"
item_type = "CrawlChunk"

[[crates.adapters.params]]
name = "request"
type = "String"

[[crates.adapters]]
name = "batch_crawl_stream"
pattern = "streaming"
core_path = "core::batch_crawl_stream"
owner_type = "CrawlEngine"
item_type = "CrawlChunk"

[[crates.adapters.params]]
name = "request"
type = "String"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn empty_api(crate_name: &str) -> ApiSurface {
    ApiSurface {
        crate_name: crate_name.to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

fn find_module<'a>(files: &'a [alef::core::backend::GeneratedFile], app_name: &str) -> &'a str {
    let expected = format!("{app_name}.ex");
    files
        .iter()
        .find(|f| f.path.file_name().map(|n| n.to_string_lossy().to_string()).as_deref() == Some(expected.as_str()))
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| {
            let paths: Vec<_> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
            panic!("expected {expected} in generated files, got: {paths:?}");
        })
}

#[test]
fn streaming_wrappers_have_proper_blank_lines_before_doc_false() {
    let config = make_config_with_multiple_adapters("test_crawl");
    let api = empty_api("test-crawl");
    let files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let content = find_module(&files, "test_crawl");

    // After the public `crawl_stream` wrapper (ends with `end`), there must be
    // a blank line before the next `@doc false` for the internal _start helper.
    let bad_pattern = "    end\n  end\n  @doc false\n  def crawlengine_batch_crawl_stream_start";
    assert!(
        !content.contains(bad_pattern),
        "must not have zero blank lines between streaming wrappers; \
         found 'end\\n  @doc false' (no blank line). \
         Content:\n{content}"
    );

    // Verify the correct pattern: blank line between the public wrapper's `end` and `@doc false`
    let correct_pattern = "    end\n  end\n\n  @doc false\n  def crawlengine_batch_crawl_stream_start";
    assert!(
        content.contains(correct_pattern),
        "must have blank line after public wrapper before next @doc false. \
         Content:\n{content}"
    );

    // Verify the second adapter also has proper spacing
    let second_adapter_bad = "    end\n  end\n  @doc false\n  def crawlengine_batch_crawl_stream_next";
    assert!(
        !content.contains(second_adapter_bad),
        "second adapter helpers must also have blank line before @doc false. \
         Content:\n{content}"
    );
}

#[test]
fn public_streaming_wrapper_function_ends_with_proper_spacing() {
    let config = make_config_with_multiple_adapters("test_spacing");
    let api = empty_api("test-spacing");
    let files = RustlerBackend
        .generate_public_api(&api, &config)
        .expect("generate_public_api must succeed");
    let content = find_module(&files, "test_spacing");

    // Check that the public `crawl_stream` function exists
    assert!(
        content.contains("def crawl_stream(client, request)"),
        "public crawl_stream wrapper must exist; content:\n{content}"
    );

    // Check that the closing `end` of `crawl_stream` is followed by a blank line
    // then the internal `@doc false` helpers
    assert!(
        content.contains("    end\n  end\n\n  @doc false\n  def crawlengine_"),
        "public wrapper function must have proper spacing (blank line) before next definition; \
         content:\n{content}"
    );
}
