use super::*;

pub(in crate::e2e::codegen::typescript::test_file) fn is_typescript_primitive_element_type(element_type: &str) -> bool {
    matches!(
        element_type,
        "string"
            | "String"
            | "&str"
            | "number"
            | "float"
            | "f32"
            | "f64"
            | "int"
            | "integer"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "boolean"
            | "bool"
            | "bytes"
            | "Uint8Array"
    )
}

/// Resolve the function name for a call config, applying node-specific overrides.
pub(in crate::e2e::codegen::typescript) fn resolve_node_function_name(
    call_config: &crate::e2e::config::CallConfig,
) -> String {
    call_config
        .overrides
        .get("node")
        .and_then(|o| o.function.clone())
        .unwrap_or_else(|| underscore_camel_case(&call_config.function))
}

/// Resolve the function name for a call config in one of the JavaScript-shaped bindings.
///
/// This renderer serves both node and wasm, which import from different packages and may
/// therefore spell the same call differently. Reading node's override for a wasm snippet emits
/// node's spelling, and emits an empty identifier outright for a call that keeps its identity
/// only in per-language overrides — `function = ""` at the base is a legitimate shape. Node
/// remains the fallback because the two bindings agree on a name far more often than not.
pub(in crate::e2e::codegen::typescript) fn resolve_js_function_name(
    lang: &str,
    call_config: &crate::e2e::config::CallConfig,
) -> String {
    call_config
        .overrides
        .get(lang)
        .and_then(|value| value.function.as_deref())
        .filter(|function| !function.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| resolve_node_function_name(call_config))
}

/// Return the package-level helper function name to import for a method_result method,
/// or `None` if the method maps to a property access (no import needed).
pub(super) fn ts_method_helper_import(method_name: &str) -> Option<String> {
    match method_name {
        "has_error_nodes" => Some("treeHasErrorNodes".to_string()),
        "error_count" | "tree_error_count" => Some("treeErrorCount".to_string()),
        "tree_to_sexp" => Some("treeToSexp".to_string()),
        "contains_node_type" => Some("treeContainsNodeType".to_string()),
        "find_nodes_by_type" => Some("findNodesByType".to_string()),
        "run_query" => Some("runQuery".to_string()),
        _ => None,
    }
}

/// Extract bridge variable names from setup lines and generate cleanup code.
/// Also generates unregister calls for trait bridges to properly clean up Rust-side state.
pub(in crate::e2e::codegen::typescript::test_file) fn extract_bridge_cleanup(setup_lines: &[String]) -> String {
    let mut cleanup_lines = Vec::new();
    for line in setup_lines {
        if let Some((var_name, trait_name)) = extract_bridge_var_and_trait(line) {
            // Generate unregister call first to clean up Rust-side bridge.
            // Unregister expects the plugin name string, not the bridge object.
            let unregister_fn = format!("unregister{}", trait_name);
            cleanup_lines.push(format!("await {}({}.name());", unregister_fn, var_name));
            // Then dispose the JS stub to release TSFN references
            cleanup_lines.push(format!("await {}.dispose();", var_name));
        }
    }
    cleanup_lines.join("\n\t\t")
}

/// Extract bridge variable name and trait name from a setup line.
/// Looks for patterns like `const _bridge_foo = new _TestStub_<fixture_id>()` where
/// the fixture was generated from a trait bridge. Extracts the trait name from the
/// fixture ID in the stub class name (e.g., register_document_extractor -> DocumentExtractor).
fn extract_bridge_var_and_trait(line: &str) -> Option<(String, String)> {
    let start = line.find("const ")?;
    let after_const = &line[start + 6..];
    let end = after_const.find(" =")?;
    let var_name = after_const[..end].trim();
    if !var_name.starts_with("_bridge_") {
        return None;
    }
    // Extract fixture ID from stub class name: _TestStub_{fixture_id}
    // Pattern: new _TestStub_{fixture_id}()
    let stub_start = line.find("new _TestStub_")?;
    let after_stub = &line[stub_start + 14..]; // len("new _TestStub_") == 14
    let paren_end = after_stub.find("()")?;
    let fixture_id = &after_stub[..paren_end];
    // Extract trait name from fixture ID
    // Example: register_document_extractor -> DocumentExtractor
    let trait_name = extract_trait_from_fixture_id(fixture_id)?;
    Some((var_name.to_string(), trait_name))
}

/// Extract trait name from fixture ID.
/// Examples:
/// - register_document_extractor_trait_bridge -> DocumentExtractor
/// - register_embedding_backend_trait_bridge -> EmbeddingBackend
/// - register_ocr_backend_trait_bridge -> OcrBackend
/// - register_post_processor_trait_bridge -> PostProcessor
fn extract_trait_from_fixture_id(fixture_id: &str) -> Option<String> {
    // Remove _trait_bridge suffix if present
    let base = fixture_id.strip_suffix("_trait_bridge").unwrap_or(fixture_id);

    // Remove register_ prefix if present
    let after_register = base.strip_prefix("register_").unwrap_or(base);

    // Convert snake_case to PascalCase
    let trait_name = snake_to_pascal_case(after_register);

    // Validate that it's a known trait name
    match trait_name.as_str() {
        "DocumentExtractor" | "EmbeddingBackend" | "OcrBackend" | "PostProcessor" | "Validator" | "Renderer"
        | "RerankerBackend" => Some(trait_name),
        _ => None,
    }
}

/// Convert snake_case to PascalCase.
/// Examples: document_extractor -> DocumentExtractor, ocr_backend -> OcrBackend
fn snake_to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Check whether any arg at index `idx` or later has a non-null value in `input`.
pub(in crate::e2e::codegen::typescript::test_file) fn has_later_arg_value(
    args: &[ArgMapping],
    from_idx: usize,
    input: &serde_json::Value,
) -> bool {
    args[from_idx..].iter().any(|arg| {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            input.get(field)
        };
        !matches!(val, None | Some(serde_json::Value::Null))
    })
}

/// Check if any arg with bytes type has a string path value that needs file reading.
pub(in crate::e2e::codegen::typescript::test_file) fn has_bytes_file_reads(
    input: &serde_json::Value,
    args: &[ArgMapping],
) -> bool {
    args.iter().any(|arg| {
        if arg.arg_type != "bytes" {
            return false;
        }
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = if field == "input" {
            Some(input.get("extract_input").unwrap_or(input))
        } else {
            input.get(field)
        };
        matches!(val, Some(serde_json::Value::String(_)))
    })
}

/// Check if any arg is a test_backend (trait bridge), requiring async test function.
pub(in crate::e2e::codegen::typescript::test_file) fn has_trait_bridge_args(args: &[ArgMapping]) -> bool {
    args.iter().any(|arg| arg.arg_type == "test_backend")
}

pub(in crate::e2e::codegen::typescript::test_file) fn strip_setup_metadata(
    input: &serde_json::Value,
) -> serde_json::Value {
    // New fixture schema wraps the call input DTO under `extract_input` alongside a
    // sibling `mock_responses` array; unwrap it so the runtime input is the DTO.
    // Flat fixtures (input IS the DTO) have no `extract_input` key.
    let input = input.get("extract_input").unwrap_or(input);
    match input {
        serde_json::Value::Object(map) => {
            let mut cleaned = map.clone();
            cleaned.remove("setup");
            serde_json::Value::Object(cleaned)
        }
        other => other.clone(),
    }
}

pub(in crate::e2e::codegen::typescript::test_file) fn canonical_ts_type_name(
    lang: &str,
    type_name: &str,
    config: &crate::core::config::ResolvedCrateConfig,
) -> String {
    if lang == "node" {
        type_name
            .strip_prefix(&config.node_type_prefix())
            .unwrap_or(type_name)
            .to_string()
    } else {
        type_name.to_string()
    }
}

/// Build the qualified `enum_fields` key used for automatically inferred entries:
/// the owning IR struct name plus the field name. Qualifying by owner prevents two
/// same-named fields on unrelated structs (e.g. `TranscriptionConfig.model:
/// WhisperModel` and `LlmConfig.model: String`) from colliding in the single
/// `enum_fields` map threaded through snippet rendering — `infer_enum_fields` walks
/// the whole reachable type graph from a call's options/argument types into one
/// shared map, so a bare field name is not a safe key.
pub(in crate::e2e::codegen::typescript::test_file) fn enum_field_key(owner_type: &str, field: &str) -> String {
    format!("{owner_type}::{field}")
}

/// Resolve the enum type name for `field`, preferring an owner-qualified match
/// (populated by `infer_enum_fields`) over a bare-name match (populated by
/// alef.toml's `enum_fields` overrides, which predate the owner-qualified form and
/// are authored by a human scoped to one call/type context rather than inferred
/// automatically across the whole type graph).
///
/// `owner_type` is `None` when the caller has no IR-resolved owning type for
/// `field` at this point (e.g. array elements re-entering `node_value_expression`
/// with a synthetic empty field name) — only the bare-name form is consulted then.
pub(in crate::e2e::codegen::typescript::test_file) fn resolve_enum_type<'a>(
    enum_fields: &'a std::collections::HashMap<String, String>,
    owner_type: Option<&str>,
    field: &str,
    camel_field: &str,
) -> Option<&'a String> {
    if let Some(owner_type) = owner_type
        && let Some(enum_type) = enum_fields
            .get(enum_field_key(owner_type, field).as_str())
            .or_else(|| enum_fields.get(enum_field_key(owner_type, camel_field).as_str()))
    {
        return Some(enum_type);
    }
    enum_fields.get(field).or_else(|| enum_fields.get(camel_field))
}
