use super::*;

#[test]
fn test_generate_docs_empty_api() {
    let api = ApiSurface {
        crate_name: "test".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let config = make_test_config();

    let files = generate_docs(&api, &config, &[Language::Python], "docs").unwrap();
    assert_eq!(files.len(), 4);
    let lang_file = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    assert!(lang_file.content.contains("Python API Reference"));
    assert!(lang_file.content.contains("v0.1.0"));
}

#[test]
fn test_generate_docs_respects_language_excludes() {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["python", "go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.python]
exclude_functions = ["interact"]
exclude_types = ["InteractionResult"]

[crates.ffi]
exclude_functions = ["ffi_only"]
exclude_types = ["FfiHidden"]
"#,
    );
    let mut api = make_minimal_api("1.2.3");
    api.functions = vec![
        make_function("interact", vec![], TypeRef::Unit, false, None),
        make_function("scrape", vec![], TypeRef::Unit, false, None),
        make_function("ffi_only", vec![], TypeRef::Unit, false, None),
    ];
    api.types = vec![empty_type("InteractionResult"), empty_type("FfiHidden")];

    let files = generate_docs(&api, &config, &[Language::Python, Language::Go], "out").unwrap();
    let python = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-python"))
        .unwrap();
    let go = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-go"))
        .unwrap();

    assert!(!python.content.contains("interact()"));
    assert!(python.content.contains("scrape()"));
    assert!(!python.content.contains("InteractionResult"));
    assert!(!go.content.contains("ffi_only()"));
    assert!(!go.content.contains("FfiHidden"));
    assert!(go.content.contains("Interact()"));
}

/// `[crates.go].exclude_functions` must also hide the function from the generated Go docs
/// page, unioned with (not replacing) `[crates.ffi].exclude_functions` — mirrors
/// `test_generate_docs_respects_language_excludes` above, which only covers the FFI-level list. ~keep
#[test]
fn test_generate_docs_respects_go_language_exclude_functions() {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["go"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.go]
exclude_functions = ["go_only"]

[crates.ffi]
exclude_functions = ["ffi_only"]
"#,
    );
    let mut api = make_minimal_api("1.2.3");
    api.functions = vec![
        make_function("go_only", vec![], TypeRef::Unit, false, None),
        make_function("ffi_only", vec![], TypeRef::Unit, false, None),
        make_function("scrape", vec![], TypeRef::Unit, false, None),
    ];

    let files = generate_docs(&api, &config, &[Language::Go], "out").unwrap();
    let go = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-go"))
        .unwrap();

    assert!(
        !go.content.contains("GoOnly()"),
        "GoConfig::exclude_functions must hide the function from the Go docs page:\n{}",
        go.content
    );
    assert!(
        !go.content.contains("FfiOnly()"),
        "the FFI-level list must still apply alongside the Go-level list:\n{}",
        go.content
    );
    assert!(
        go.content.contains("Scrape()"),
        "a function excluded in neither list must still appear:\n{}",
        go.content
    );
}

/// `[crates.jni].exclude_functions` names a function the JNI shim crate never generates a
/// binding for, so KotlinAndroid -- the language that calls through that shim -- has no way to
/// reach it either. `language_excludes`'s `Language::KotlinAndroid` arm used to fold only
/// `[crates.kotlin_android]` and `[crates.ffi]`, never `[crates.jni]`, so a function excluded
/// only at the JNI level still showed up as "expected" on the KotlinAndroid docs page and in the
/// snippet coverage ledger with no way for a consumer to silence it. ~keep
#[test]
fn test_generate_docs_respects_jni_exclude_functions_for_kotlin_android() {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.jni]
exclude_functions = ["jni_only"]

[crates.kotlin_android]
exclude_functions = ["android_only"]
"#,
    );
    let mut api = make_minimal_api("1.2.3");
    api.functions = vec![
        make_function("jni_only", vec![], TypeRef::Unit, false, None),
        make_function("android_only", vec![], TypeRef::Unit, false, None),
        make_function("scrape", vec![], TypeRef::Unit, false, None),
    ];

    let files = generate_docs(&api, &config, &[Language::KotlinAndroid], "out").unwrap();
    let kotlin_android = files
        .iter()
        .find(|f| f.path.to_str().unwrap().contains("api-kotlin-android"))
        .unwrap();

    assert!(
        !kotlin_android.content.contains("jniOnly"),
        "JniConfig::exclude_functions must hide the function from the KotlinAndroid docs page:\n{}",
        kotlin_android.content
    );
    assert!(
        !kotlin_android.content.contains("androidOnly"),
        "the KotlinAndroid-level list must still apply alongside the JNI-level list:\n{}",
        kotlin_android.content
    );
    assert!(
        kotlin_android.content.contains("scrape"),
        "a function excluded in neither list must still appear:\n{}",
        kotlin_android.content
    );
}

#[test]
fn test_generate_docs_produces_one_file_per_language_plus_three_shared() {
    let api = make_minimal_api("1.2.3");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python, Language::Node], "out").unwrap();
    assert_eq!(files.len(), 5);
    let paths: Vec<&str> = files.iter().map(|f| f.path.to_str().unwrap()).collect();
    assert!(paths.iter().any(|p| p.contains("api-python")));
    assert!(paths.iter().any(|p| p.contains("api-typescript")));
    assert!(paths.iter().any(|p| p.contains("configuration")));
    assert!(paths.iter().any(|p| p.contains("types")));
    assert!(paths.iter().any(|p| p.contains("errors")));
}

/// `Language::Ffi` and `Language::Jni` both slug to `"c"` (`naming::lang_slug`), and their
/// content genuinely diverges -- `Jni` is an internal shim crate paired with `kotlin_android`,
/// not a real C ABI (see the long comment on `examples::sample_param_value`'s Jni arm), so its
/// render arms skip the FFI-specific "int32_t status code" return phrasing and the "C
/// representation" handle note. Before `generate_docs` learned to skip `Language::Jni` (mirroring
/// `readme::generate_readme`'s existing `Language::C | Language::Jni` skip),
/// `languages = ["ffi", "jni", "kotlin_android"]` -- a legal combination liter-llm's alef.toml
/// configures for real -- produced two `GeneratedFile`s at `api-c.md` with different content,
/// which `write_scaffold_files_report` correctly refused to write rather than pick a winner.
/// Only `Ffi` may own that path. ~keep
#[test]
fn test_generate_docs_jni_does_not_duplicate_the_c_reference_page() {
    let config = config_from_toml(
        r#"
[workspace]
languages = ["ffi", "jni", "kotlin_android"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
"#,
    );
    let mut api = make_minimal_api("1.0.0");
    api.functions = vec![make_function(
        "connect",
        vec![],
        TypeRef::Unit,
        false,
        Some("ConnectError"),
    )];
    api.types = vec![empty_type("Config")];

    let files = generate_docs(
        &api,
        &config,
        &[Language::Ffi, Language::Jni, Language::KotlinAndroid],
        "out",
    )
    .unwrap();

    let c_pages: Vec<_> = files
        .iter()
        .filter(|f| f.path.to_str().unwrap().contains("api-c"))
        .collect();
    assert_eq!(
        c_pages.len(),
        1,
        "exactly one generator must claim api-c.md, got paths: {:?}",
        c_pages.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
    );
    assert!(
        c_pages[0]
            .content
            .contains("`int32_t` status code -- `0` on success, `-1` on error"),
        "api-c.md must render the Ffi/C ABI, not the Jni one:\n{}",
        c_pages[0].content
    );
    assert!(
        c_pages[0].content.contains("**C representation:**"),
        "api-c.md must carry the FFI handle note:\n{}",
        c_pages[0].content
    );

    assert!(
        files
            .iter()
            .any(|f| f.path.to_str().unwrap().contains("api-kotlin-android")),
        "kotlin_android must still get its own reference page"
    );
    assert!(
        !files.iter().any(|f| f.path.to_str().unwrap().contains("api-jni")),
        "jni must not render an independent reference page"
    );
}

#[test]
fn test_generate_docs_all_output_files_end_with_newline() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "out").unwrap();
    for file in &files {
        assert!(
            file.content.ends_with('\n'),
            "file {:?} must end with trailing newline",
            file.path
        );
    }
}

#[test]
fn test_generate_docs_output_dir_prefix_in_all_paths() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let files = generate_docs(&api, &config, &[Language::Python], "custom/output/dir").unwrap();
    for file in &files {
        assert!(
            file.path.to_str().unwrap().starts_with("custom/output/dir"),
            "all paths must be under output_dir: {:?}",
            file.path
        );
    }
}

#[test]
fn test_generate_docs_multiple_languages_produce_correct_slugs() {
    let api = make_minimal_api("0.1.0");
    let config = make_test_config();
    let langs = [
        Language::Python,
        Language::Node,
        Language::Go,
        Language::Java,
        Language::Ruby,
    ];
    let expected_slugs = ["api-python", "api-typescript", "api-go", "api-java", "api-ruby"];
    let files = generate_docs(&api, &config, &langs, "docs/api").unwrap();
    assert_eq!(files.len(), 8);
    for slug in &expected_slugs {
        assert!(
            files.iter().any(|f| f.path.to_str().unwrap().contains(slug)),
            "expected file with slug {slug}"
        );
    }
}

#[test]
fn streaming_adapter_docs_use_language_native_stream_types() {
    let config = streaming_adapter_config("");
    let api = streaming_adapter_api(&config);
    let files = generate_docs(
        &api,
        &config,
        &[
            Language::Python,
            Language::Node,
            Language::Java,
            Language::KotlinAndroid,
            Language::Zig,
            Language::Go,
            Language::Csharp,
            Language::Swift,
            Language::Dart,
            Language::Php,
            Language::Ruby,
            Language::Elixir,
            Language::Wasm,
            Language::Ffi,
            Language::Rust,
        ],
        "out",
    )
    .unwrap();

    let python = doc_content(&files, "api-python");
    assert!(python.contains("def chat_stream(self, req: ChatCompletionRequest) -> AsyncIterator[ChatCompletionChunk]"));
    assert!(python.contains("async for chunk in stream:"));
    assert!(!python.contains("-> str"));

    let typescript = doc_content(&files, "api-typescript");
    assert!(typescript.contains("chatStream(req: ChatCompletionRequest): Promise<ChatStreamIterator>"));
    assert!(typescript.contains("for await (const chunk of stream)"));
    assert!(!typescript.contains("Promise<string>"));

    let java = doc_content(&files, "api-java");
    assert!(
        java.contains(
            "public java.util.stream.Stream<ChatCompletionChunk> chatStream(ChatCompletionRequest req) throws LiterLlmRsException"
        )
    );
    assert!(java.contains("try (var stream = instance.chatStream(new ChatCompletionRequest()))"));
    assert!(!java.contains("public String chatStream"));

    let kotlin_android = doc_content(&files, "api-kotlin-android");
    assert!(
        kotlin_android
            .contains("fun chatStream(req: ChatCompletionRequest): kotlinx.coroutines.flow.Flow<ChatCompletionChunk>")
    );
    assert!(kotlin_android.contains(".collect { chunk ->"));
    assert!(!kotlin_android.contains("fun chatStream(req: ChatCompletionRequest): String"));

    let zig = doc_content(&files, "api-zig");
    assert!(zig.contains(
        "pub fn chat_stream(self: *DefaultClient, req: []const u8) (LiterLlmError||error{OutOfMemory})!ChatCompletionChunkStream"
    ));
    assert!(zig.contains("while (try stream.next()) |chunk|"));
    assert!(!zig.contains("pub fn chatStream"));
    assert!(!zig.contains("[:0]const u8"));

    let go = doc_content(&files, "api-go");
    assert!(
        go.contains(
            "func (o *DefaultClient) ChatStream(req ChatCompletionRequest) (<-chan ChatCompletionChunk, error)"
        )
    );

    let csharp = doc_content(&files, "api-csharp");
    assert!(csharp.contains(
        "public async IAsyncEnumerable<ChatCompletionChunk> ChatStreamAsync(ChatCompletionRequest req, CancellationToken cancellationToken = default)"
    ));
    assert!(csharp.contains("await foreach (var chunk in instance.ChatStreamAsync(new ChatCompletionRequest()))"));

    let swift = doc_content(&files, "api-swift");
    assert!(
        swift.contains("public func chatStream(_ req: ChatCompletionRequest) async throws -> AsyncThrowingStream<ChatCompletionChunk, Error>")
    );
    assert!(swift.contains("for try await chunk in stream"));

    let dart = doc_content(&files, "api-dart");
    assert!(dart.contains("Stream<ChatCompletionChunk> chatStream(ChatCompletionRequest req)"));
    assert!(dart.contains("await for (final chunk in instance.chatStream(ChatCompletionRequest()))"));

    let php = doc_content(&files, "api-php");
    assert!(php.contains("public function chatStream(ChatCompletionRequest $req): array"));
    assert!(php.contains("foreach ($instance->chatStream(new ChatCompletionRequest()) as $chunk)"));
    assert!(php.contains("var_dump($chunk);"));

    let ruby = doc_content(&files, "api-ruby");
    assert!(ruby.contains("def chat_stream(req)"));
    assert!(ruby.contains("**Returns:** `ChatStreamIterator`"));

    let elixir = doc_content(&files, "api-elixir");
    // The rustler backend always names the receiver param `obj`, not `client`
    // (`gen_bindings/helpers/conversions.rs`'s `def_args.push("obj".to_string())`).
    assert!(elixir.contains("def chat_stream(obj, req)"));
    assert!(elixir.contains("**Returns:** `{:ok, Stream.t()}`"));

    let wasm = doc_content(&files, "api-wasm");
    assert!(wasm.contains("chatStream(req: ChatCompletionRequest): Promise<ChatStreamIterator>"));
    assert!(wasm.contains("const chunk = await stream.next();"));

    let ffi = doc_content(&files, "api-c");
    assert!(ffi.contains(
        "struct LITERLLMLiterllmDefaultClientChatStreamStreamHandle * literllm_default_client_chat_stream_start"
    ));

    let rust = doc_content(&files, "api-rust");
    assert!(rust.contains(
        "fn chat_stream(&self, req: ChatCompletionRequest) -> BoxFuture<'_, Result<BoxStream<'static, Result<ChatCompletionChunk>>>>"
    ));
}

#[test]
fn streaming_adapter_docs_respect_skip_languages_canonical_names() {
    let config = streaming_adapter_config("skip_languages = [\"node\"]");
    let api = streaming_adapter_api(&config);
    let files = generate_docs(&api, &config, &[Language::Node, Language::Java], "out").unwrap();

    let typescript = doc_content(&files, "api-typescript");
    assert!(!typescript.contains("chatStream("));
    assert!(!typescript.contains("Promise<ChatStreamIterator>"));

    let java = doc_content(&files, "api-java");
    assert!(
        java.contains(
            "public java.util.stream.Stream<ChatCompletionChunk> chatStream(ChatCompletionRequest req) throws LiterLlmRsException"
        )
    );
}

#[test]
fn streaming_adapter_docs_use_crate_exception_for_short_core_paths() {
    let mut config = streaming_adapter_config("");
    let api = streaming_adapter_api(&config);
    config.adapters[0].core_path = "chat_stream".to_string();
    let files = generate_docs(&api, &config, &[Language::Java], "out").unwrap();

    let java = doc_content(&files, "api-java");
    assert!(
        java.contains(
            "public java.util.stream.Stream<ChatCompletionChunk> chatStream(ChatCompletionRequest req) throws LiterLlmRsException"
        )
    );
    assert!(!java.contains("ChatStreamRsException"));
}

#[test]
fn generated_docs_hide_binding_excluded_members_outside_rust() {
    let mut api = make_minimal_api("1.6.0");
    let mut config_type = empty_type("ClientConfig");
    let mut visible_field = make_field("base_url", TypeRef::String, false, None);
    visible_field.doc = "Public base URL.".to_string();
    let mut rust_only_field = make_field("dispatch", TypeRef::String, false, None);
    rust_only_field.doc = "Rust-only dispatch profile.".to_string();
    rust_only_field.binding_excluded = true;
    rust_only_field.binding_exclusion_reason = Some("alef(skip)".to_string());
    config_type.fields = vec![visible_field, rust_only_field];

    let mut client = empty_type("DefaultClient");
    client.is_opaque = true;
    let visible_method = make_method("create", vec![], TypeRef::String, false, false, None);
    let mut rust_only_method = make_method("from_engine", vec![], TypeRef::String, false, false, None);
    rust_only_method.binding_excluded = true;
    rust_only_method.binding_exclusion_reason = Some("alef(skip)".to_string());
    client.methods = vec![visible_method, rust_only_method];

    api.types = vec![config_type, client];
    let config = streaming_adapter_config("");
    let files = generate_docs(&api, &config, &[Language::Python, Language::Rust], "out").unwrap();

    let python = doc_content(&files, "api-python");
    assert!(python.contains("base_url"));
    assert!(!python.contains("dispatch"));
    assert!(python.contains("create()"));
    assert!(!python.contains("from_engine()"));

    let rust = doc_content(&files, "api-rust");
    assert!(rust.contains("dispatch"));
    assert!(rust.contains("from_engine()"));

    let configuration = doc_content(&files, "configuration");
    assert!(configuration.contains("base_url"));
    assert!(!configuration.contains("dispatch"));

    let types = doc_content(&files, "types");
    assert!(types.contains("base_url"));
    assert!(!types.contains("dispatch"));
}

fn streaming_adapter_config(extra_adapter_fields: &str) -> ResolvedCrateConfig {
    config_from_toml(&format!(
        r#"
[workspace]
languages = ["python", "node", "java", "kotlin_android", "zig", "go", "csharp", "swift", "dart", "php", "ruby", "elixir", "wasm", "ffi", "rust"]

[[crates]]
name = "liter-llm"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "literllm"

[[crates.adapters]]
name = "chat_stream"
pattern = "streaming"
core_path = "liter_llm::DefaultClient::chat_stream"
owner_type = "DefaultClient"
item_type = "ChatCompletionChunk"
error_type = "LiterLlmError"
request_type = "liter_llm::ChatCompletionRequest"
{extra_adapter_fields}

[[crates.adapters.params]]
name = "req"
type = "ChatCompletionRequest"
"#
    ))
}

/// Takes its `crate_name` from `config` so the fixture cannot drift from the production
/// invariant that `ApiSurface::crate_name` is seeded from `[[crates]] name`
/// (`cli/pipeline/extract/raw.rs`). The Java exception class the docs print is derived from
/// it, so a fixture that disagrees would test a state that cannot occur. ~keep
fn streaming_adapter_api(config: &ResolvedCrateConfig) -> ApiSurface {
    let mut api = make_minimal_api("1.6.0");
    api.crate_name = config.name.clone();
    let mut client = empty_type("DefaultClient");
    client.is_opaque = true;
    client.methods = vec![make_method(
        "chat_stream",
        vec![make_param(
            "req",
            TypeRef::Named("ChatCompletionRequest".to_string()),
            false,
        )],
        TypeRef::String,
        true,
        false,
        Some("LiterLlmError"),
    )];
    api.types = vec![
        client,
        empty_type("ChatCompletionRequest"),
        empty_type("ChatCompletionChunk"),
    ];
    api
}

/// Regression test for a real generated-doc defect: an authored `# Example` whose source
/// fence is bare (rustdoc's implicit-Rust convention, as `html-to-markdown`'s `convert()`
/// doc comment uses) must render with a bare closing fence in `api-rust.md`, not a closing
/// fence that re-carries the `rust` language tag — which reopens a block instead of closing
/// it and corrupts every line of Markdown that follows in the rendered page. ~keep
#[test]
fn test_generate_docs_rust_authored_example_closes_fence_bare() {
    let mut api = make_minimal_api("1.0.0");
    let mut convert = make_function(
        "convert",
        vec![make_param("html", TypeRef::String, false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        Some("ConversionError"),
    );
    convert.doc = concat!(
        "Convert HTML to Markdown.\n\n",
        "# Example\n\n",
        "```\n",
        "use html_to_markdown_rs::{convert, ConversionOptions};\n",
        "let result = convert(\"<h1>Hi</h1>\", ConversionOptions::default()).unwrap();\n",
        "```",
    )
    .to_string();
    api.functions = vec![convert];
    api.types = vec![empty_type("ConversionResult")];
    let config = make_test_config();

    let files = generate_docs(&api, &config, &[Language::Rust], "docs").unwrap();
    let content = doc_content(&files, "api-rust");

    // ~keep The signature block also renders a ` ```rust ` fence, so scope the check to the
    // fence pair that immediately follows the `**Example:**` heading.
    let example_start = content
        .find("**Example:**")
        .expect("expected an Example section in the generated doc");
    let example_fence_lines: Vec<&str> = content[example_start..]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("```"))
        .take(2)
        .collect();
    assert_eq!(
        example_fence_lines,
        vec!["```rust", "```"],
        "the example's closing fence must be bare, not re-tagged with the language"
    );
}
