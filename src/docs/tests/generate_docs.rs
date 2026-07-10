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
        excluded_type_paths: ::std::collections::HashMap::new(),
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
    let api = streaming_adapter_api();
    let config = streaming_adapter_config("");
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
    assert!(elixir.contains("def chat_stream(client, req)"));
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
    let api = streaming_adapter_api();
    let config = streaming_adapter_config("skip_languages = [\"node\"]");
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
    let api = streaming_adapter_api();
    let mut config = streaming_adapter_config("");
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

fn streaming_adapter_api() -> ApiSurface {
    let mut api = make_minimal_api("1.6.0");
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

fn doc_content<'a>(files: &'a [crate::core::backend::GeneratedFile], slug: &str) -> &'a str {
    let expected_name = format!("{slug}.md");
    files
        .iter()
        .find(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == expected_name)
        })
        .map(|file| file.content.as_str())
        .unwrap_or_else(|| panic!("missing generated doc file for {slug}"))
}
