use super::config_marshalling_fixtures::{config_marshalling_api_surface, json_marshalling_api_surface};
use super::{RustlerBackend, test_config};
use crate::core::backend::Backend;
use crate::core::ir::ApiSurface;

fn generated_native(api: &ApiSurface) -> String {
    let files = RustlerBackend
        .generate_bindings(api, &test_config())
        .expect("native binding generation must succeed");
    files
        .iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .expect("expected native lib.rs")
        .content
        .clone()
}

fn generated_public(api: &ApiSurface) -> (String, String) {
    let files = RustlerBackend
        .generate_public_api(api, &test_config())
        .expect("public API generation must succeed");
    let root = files.iter().find(|file| file.path.ends_with("my_lib.ex")).unwrap();
    let builder = files.iter().find(|file| file.path.ends_with("builder.ex")).unwrap();
    (root.content.clone(), builder.content.clone())
}

#[test]
fn native_nifs_should_return_contextual_errors_for_malformed_default_typed_json() {
    let content = generated_native(&config_marshalling_api_surface());
    assert_default_contexts(&content);
    assert_default_signatures(&content);
    assert_default_conversion(&content);
    syn::parse_file(&content).expect("generated Rustler source with Vec<Config> methods must parse as Rust");
}

fn assert_default_contexts(content: &str) {
    for function_name in [
        "builder_configure",
        "builder_configure_later_async",
        "builder_configure_many",
        "builder_configure_many_later_async",
        "registry_configure_many",
        "registry_configure_many_later_async",
        "build",
        "build_async",
    ] {
        let parameter = if function_name.contains("many") {
            "configs"
        } else {
            "config"
        };
        let expected = format!("failed to deserialize parameter `{parameter}` for `{function_name}`");
        assert!(
            content.contains(&expected),
            "missing context for {function_name}:\n{content}"
        );
    }
    for parameter in ["first", "second"] {
        let expected = format!("failed to deserialize parameter `{parameter}` for `merge`");
        assert!(content.contains(&expected), "missing merge context:\n{content}");
    }
}

fn assert_default_signatures(content: &str) {
    for signature in [
        "pub fn builder_configure(resource: rustler::ResourceArc<Builder>, config: Option<String>) -> Result<(), String>",
        "pub fn build(config: Option<String>) -> Result<(), String>",
        "pub fn builder_configure_many(resource: rustler::ResourceArc<Builder>, configs: Option<String>) -> Result<(), String>",
        "pub fn registry_configure_many(obj: Registry, configs: Option<String>) -> Result<(), String>",
    ] {
        assert!(
            content.contains(signature),
            "missing fallible signature `{signature}`:\n{content}"
        );
    }
}

fn assert_default_conversion(content: &str) {
    assert!(
        content.contains("let configs_core: Vec<my_lib::Config> = configs_core_option.unwrap_or_default();"),
        "Vec<Config> JSON must deserialize into the core element type:\n{content}"
    );
    assert!(
        content.contains(".configure_many(configs_core)"),
        "Vec<Config> method call must receive the deserialized core vector:\n{content}"
    );
    assert!(
        !content.contains("serde_json::from_str::<my_lib::Config>(&s).ok()"),
        "malformed JSON must not be discarded:\n{content}"
    );
}

/// `Config::validate(&self)` is a fallible method on the non-opaque, `has_default` `Config`
/// type itself — the receiver, not just a param, carries fields alef cannot render a real
/// default for (e.g. a nested struct's `Default::default()`), so the receiver must be
/// JSON-decoded into the *core* type exactly like `build(config: Option<String>)` already is,
/// rather than decoded directly from the Elixir term via `NifMap`/`NifStruct`.
#[test]
fn native_default_typed_receiver_methods_should_json_decode_into_core_type() {
    let content = generated_native(&config_marshalling_api_surface());
    assert!(
        content.contains("pub fn config_validate(obj: String) -> Result<(), String> {"),
        "receiver must become a JSON string parameter, not the struct itself:\n{content}"
    );
    let expected_preamble = "let obj: my_lib::Config = serde_json::from_str::<my_lib::Config>(&obj)\n    \
        .map_err(|error| format!(\"failed to deserialize parameter `obj` for `config_validate`: {error}\"))?;";
    assert!(
        content.contains(expected_preamble),
        "receiver must be JSON-decoded into the core type with contextual error:\n{content}"
    );
    assert!(
        content.contains("let result = my_lib::Config::from(obj).validate().map_err(|e| e.to_string())?;"),
        "decoded core value must be passed straight into the delegated call:\n{content}"
    );
    syn::parse_file(&content).expect("generated Rustler source with a default-typed receiver must parse as Rust");
}

#[test]
fn free_functions_should_pass_mutable_named_vectors_by_mutable_reference() {
    let content = generated_native(&config_marshalling_api_surface());
    for function_name in ["mutate_configs", "mutate_configs_async"] {
        let function_start = content
            .find(&format!("pub fn {function_name}"))
            .expect("expected generated function");
        let function = &content[function_start..];
        assert!(
            function.contains("let mut configs_core: Vec<my_lib::Config> ="),
            "{function_name} must create a mutable core vector:\n{function}"
        );
        assert!(
            function.contains(&format!("my_lib::{function_name}(&mut configs_core)")),
            "{function_name} must pass the core vector by mutable reference:\n{function}"
        );
    }
}

pub(super) fn assert_opaque_methods_json_encode_named_params() {
    let (_, builder) = generated_public(&config_marshalling_api_surface());
    for expected in [
        "Native.builder_configure(obj.ref, (cond do is_nil(config) -> nil; is_binary(config) -> config; true -> Jason.encode!(config) end))",
        "Native.builder_configure_later_async(obj.ref, (cond do is_nil(config) -> nil; is_binary(config) -> config; true -> Jason.encode!(config) end))",
        "Native.builder_configure_many(obj.ref, (cond do is_nil(configs) -> nil; is_binary(configs) -> configs; true -> Jason.encode!(configs) end))",
        "Native.builder_configure_many_later_async(obj.ref, (cond do is_nil(configs) -> nil; is_binary(configs) -> configs; true -> Jason.encode!(configs) end))",
    ] {
        assert!(builder.contains(expected), "generated Builder wrapper:\n{builder}");
    }
}

pub(super) fn assert_public_sync_wrapper_contracts() {
    let (root, builder) = generated_public(&config_marshalling_api_surface());
    assert_sync_root_contract(&root);
    assert_sync_builder_contract(&builder);
}

fn assert_sync_root_contract(root: &str) {
    for expected in [
        "case MyLib.Native.build(",
        "{:ok, value} -> value",
        "{:error, error} -> raise ArgumentError, error",
        "def merge(opts \\\\ []) do",
        "case MyLib.Native.merge(\n      case Keyword.get(opts, :first)",
        "case MyLib.Native.builder_with_config(obj.ref, (cond do is_nil(config)",
        "%MyLib.Builder{ref: ref}",
        "case MyLib.Native.builder_configure_many(obj.ref, (cond do is_nil(configs)",
    ] {
        assert!(root.contains(expected), "root wrapper missing `{expected}`:\n{root}");
    }
}

fn assert_sync_builder_contract(builder: &str) {
    for expected in [
        "ref =\n      case Native.builder_with_config(",
        "{:ok, value} -> value",
        "%__MODULE__{ref: ref}",
        "case Native.builder_configure_many(obj.ref, (cond do is_nil(configs) -> nil; is_binary(configs)",
        "Native.builder_configure_many_later_async(obj.ref, (cond do is_nil(configs)",
    ] {
        assert!(
            builder.contains(expected),
            "Builder wrapper missing `{expected}`:\n{builder}"
        );
    }
    assert!(
        !builder.contains("case Native.builder_configure_many_later_async(obj.ref"),
        "async Vec<Config> public result contract must remain unchanged:\n{builder}"
    );
}

pub(super) fn assert_returns_self_result_shapes() {
    let (root, builder) = generated_public(&config_marshalling_api_surface());
    for call in [
        "Native.builder_with_config_later_async(obj.ref",
        "Native.builder_try_with_config(obj.ref",
    ] {
        assert_result_wrapper(&builder, call, "%__MODULE__{ref: ref}");
    }
    for call in [
        "MyLib.Native.builder_with_config_later_async(obj.ref",
        "MyLib.Native.builder_try_with_config(obj.ref",
    ] {
        assert_result_wrapper(&root, call, "%MyLib.Builder{ref: ref}");
    }
}

fn assert_result_wrapper(content: &str, nif_call: &str, wrapper: &str) {
    let start = content.find(nif_call).expect("expected generated NIF call");
    let function = &content[start..];
    let success = format!("{{:ok, ref}} -> {{:ok, {wrapper}}}");
    assert!(
        function.contains(&success),
        "successful ref must be Result-wrapped:\n{function}"
    );
    assert!(
        function.contains("{:error, error} -> {:error, error}"),
        "error tuples must pass through unchanged:\n{function}"
    );
    let trailing = format!("{{:error, error}} -> {{:error, error}}\n    end\n    {wrapper}");
    assert!(
        !function.contains(&trailing),
        "must not append a second wrapper:\n{function}"
    );
}

pub(super) fn assert_json_deserialization_contracts() {
    let api = json_marshalling_api_surface();
    let native = generated_native(&api);
    assert_json_contexts(&native);
    assert_json_native_contract(&native);
    syn::parse_file(&native).expect("generated Rustler source must parse as Rust");
    let (root, builder) = generated_public(&api);
    assert_json_public_contract(&root, &builder);
}

fn assert_json_contexts(native: &str) {
    for function_name in [
        "builder_set_metadata",
        "builder_set_metadata_later_async",
        "render",
        "render_async",
    ] {
        let expected = format!("failed to deserialize parameter `metadata` for `{function_name}`");
        assert!(
            native.contains(&expected),
            "missing JSON context for {function_name}:\n{native}"
        );
    }
}

fn assert_json_native_contract(native: &str) {
    for expected in [
        "pub fn render(metadata: String) -> Result<(), String>",
        "pub fn nondelegated_json(metadata: String) -> ()",
        "pub fn builder_sanitized_metadata(resource: rustler::ResourceArc<Builder>, metadata: String) -> ()",
        "pub fn nondelegated_json_async(metadata: String) -> Result<(), String>",
        "pub fn builder_sanitized_metadata_later_async(resource: rustler::ResourceArc<Builder>, metadata: String) -> Result<(), String>",
        "Err(String::from(\"Not implemented: nondelegated_json_async\"))",
        "Err(String::from(\"Not implemented: builder_sanitized_metadata_later_async\"))",
    ] {
        assert!(
            native.contains(expected),
            "native JSON contract missing `{expected}`:\n{native}"
        );
    }
}

fn assert_json_public_contract(root: &str, builder: &str) {
    for expected in [
        "case MyLib.Native.render(metadata) do",
        "MyLib.Native.render_async(metadata)",
        "case MyLib.Native.builder_set_metadata(obj.ref, metadata) do",
    ] {
        assert!(
            root.contains(expected),
            "root JSON wrapper missing `{expected}`:\n{root}"
        );
    }
    assert!(!root.contains("case MyLib.Native.render_async(metadata)"));
    for expected in [
        "case Native.builder_set_metadata(obj.ref, metadata) do",
        "Native.builder_set_metadata_later_async(obj.ref, metadata)",
    ] {
        assert!(
            builder.contains(expected),
            "Builder JSON wrapper missing `{expected}`:\n{builder}"
        );
    }
}
