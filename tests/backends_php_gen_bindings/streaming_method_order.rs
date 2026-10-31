use super::*;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{MethodDef, ReceiverKind};

fn streaming_config() -> ResolvedCrateConfig {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "test_lib"

[[crates.adapters]]
name = "crawl_stream"
pattern = "streaming"
core_path = "crawl_stream"
owner_type = "CrawlEngineHandle"
item_type = "CrawlEvent"
error_type = "TestError"

[[crates.adapters]]
name = "batch_crawl_stream"
pattern = "streaming"
core_path = "batch_crawl_stream"
owner_type = "CrawlEngineHandle"
item_type = "CrawlEvent"
error_type = "TestError"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn streaming_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: true,
        is_static: false,
        error_type: Some("TestError".to_string()),
        doc: String::new(),
        sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        trait_source: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn streaming_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "CrawlEngineHandle".to_string(),
            rust_path: "test_lib::CrawlEngineHandle".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![streaming_method("crawl_stream"), streaming_method("batch_crawl_stream")],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_deserialization: Default::default(),
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![],
        ..Default::default()
    }
}

fn emitted_streaming_method_order(lib_rs: &str) -> Vec<&str> {
    ["crawl_stream", "batch_crawl_stream"]
        .into_iter()
        .filter_map(|name| lib_rs.find(&format!("pub fn {name}(")).map(|at| (at, name)))
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_values()
        .collect()
}

/// The `#[php_impl]` block must emit streaming methods in the order their adapters are declared in
/// `alef.toml`. The keys used to be collected into an `AHashSet` and iterated, and because ahash
/// seeds itself per process, regenerating an unchanged tree could swap `crawl_stream` and
/// `batch_crawl_stream` — a spurious diff that made the freshness gate intermittently red.
#[test]
fn php_streaming_methods_are_emitted_in_adapter_declaration_order() {
    let files = PhpBackend
        .generate_bindings(&streaming_api(), &streaming_config())
        .expect("php bindings must generate");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .map(|f| f.content.as_str())
        .expect("generated lib.rs must exist");

    assert_eq!(
        emitted_streaming_method_order(lib_rs),
        vec!["crawl_stream", "batch_crawl_stream"],
        "streaming methods must follow adapter declaration order:\n{lib_rs}"
    );
}

/// Guards the same defect from the other side: repeated generation inside one process must produce
/// byte-identical output. Each `AHashSet` built in a process gets its own seed, so a hash-ordered
/// emission would disagree with itself well before this loop finished.
#[test]
fn php_streaming_method_emission_is_stable_across_repeated_generation() {
    let api = streaming_api();
    let config = streaming_config();
    let generate = || {
        let files = PhpBackend
            .generate_bindings(&api, &config)
            .expect("php bindings must generate");
        files
            .iter()
            .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
            .map(|f| f.content.clone())
            .expect("generated lib.rs must exist")
    };

    let first = generate();
    for attempt in 1..16 {
        assert_eq!(generate(), first, "generation {attempt} differed from the first");
    }
}
