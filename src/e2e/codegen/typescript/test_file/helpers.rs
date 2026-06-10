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
        .unwrap_or_else(|| snake_to_camel(&call_config.function))
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
            // Generate unregister call first to clean up Rust-side bridge
            let unregister_fn = format!("unregister{}", trait_name);
            cleanup_lines.push(format!("await {}({});", unregister_fn, var_name));
            // Then dispose the JS stub
            cleanup_lines.push(format!("await {}.dispose();", var_name));
        }
    }
    cleanup_lines.join("\n\t\t")
}

/// Extract bridge variable name and trait name from a setup line.
/// Looks for patterns like `const _bridge_foo = new _TestStub_<fixture_id>()` where
/// the fixture was generated from a trait bridge, and extracts the trait name from the
/// bridge variable name mapping.
fn extract_bridge_var_and_trait(line: &str) -> Option<(String, String)> {
    if let Some(start) = line.find("const ") {
        let after_const = &line[start + 6..];
        if let Some(end) = after_const.find(" =") {
            let var_name = after_const[..end].trim();
            if let Some(arg_name) = var_name.strip_prefix("_bridge_") {
                // Extract trait name from variable name: _bridge_{arg_name}
                // Maps to the arg's trait_name field, which we infer from common patterns:
                // _bridge_extractor -> DocumentExtractor
                // _bridge_backend -> OcrBackend (or EmbeddingBackend, etc.)
                // _bridge_processor -> PostProcessor
                // _bridge_validator -> Validator
                // _bridge_renderer -> Renderer
                // Skip "_bridge_"
                let trait_name = infer_trait_name_from_arg(arg_name);
                return Some((var_name.to_string(), trait_name));
            }
        }
    }
    None
}

/// Infer trait name from argument name using common patterns.
fn infer_trait_name_from_arg(arg_name: &str) -> String {
    match arg_name {
        "extractor" => "DocumentExtractor".to_string(),
        "backend" => "OcrBackend".to_string(), // Most common, but could be EmbeddingBackend
        "processor" => "PostProcessor".to_string(),
        "validator" => "Validator".to_string(),
        "renderer" => "Renderer".to_string(),
        _ => {
            // Fallback: capitalize first letter and append 'Backend' if not otherwise identifiable
            if let Some(base) = arg_name.strip_suffix("backend") {
                // Remove "backend"
                format!("{}Backend", capitalize_first(base))
            } else {
                capitalize_first(arg_name)
            }
        }
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
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
            Some(input)
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
            Some(input)
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
