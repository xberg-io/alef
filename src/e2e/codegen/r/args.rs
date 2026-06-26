//! R e2e argument rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_r;
use crate::e2e::fixture::Fixture;

use super::values::{json_to_r, json_to_r_preserve_arrays};

/// Remove the named `options = …` argument (if any) from an R call-args string.
///
/// Walks the string while tracking paren/quote depth so a comma inside a nested
/// expression like `options = list(visitor = visitor)` isn't treated as the
/// arg terminator. Returns the rebuilt args string with the `options =` arg
/// dropped; callers append a fresh one.
pub(super) fn strip_options_arg(args_str: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut paren_depth: i32 = 0;
    let mut in_single = false;
    let mut in_double = false;
    for c in args_str.chars() {
        if !in_single && !in_double {
            match c {
                '(' | '[' | '{' => paren_depth += 1,
                ')' | ']' | '}' => paren_depth -= 1,
                '\'' => in_single = true,
                '"' => in_double = true,
                ',' if paren_depth == 0 => {
                    parts.push(current.trim().to_string());
                    current.clear();
                    continue;
                }
                _ => {}
            }
        } else if in_single && c == '\'' {
            in_single = false;
        } else if in_double && c == '"' {
            in_double = false;
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
        .into_iter()
        .filter(|p| !p.starts_with("options ") && !p.starts_with("options="))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) struct RArgsContext<'a> {
    pub(super) arg_name_map: Option<&'a std::collections::HashMap<String, String>>,
    pub(super) options_type: Option<&'a str>,
    pub(super) fixture: &'a Fixture,
    pub(super) config: &'a ResolvedCrateConfig,
    pub(super) type_defs: &'a [crate::core::ir::TypeDef],
    pub(super) setup_lines: &'a mut Vec<String>,
    pub(super) teardown_block: &'a mut String,
}

pub(super) fn build_args_string(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    context: RArgsContext<'_>,
) -> String {
    let RArgsContext {
        arg_name_map,
        options_type,
        fixture,
        config,
        type_defs,
        setup_lines,
        teardown_block,
    } = context;
    if args.is_empty() {
        // No declared args means the wrapper takes zero parameters. Always
        // emit an empty arg list — fixtures may carry harness metadata under
        // `input` (e.g. `setup.lazy_init_required` for Go's eager-init shim)
        // that must not leak into the R call site as a positional `list(...)`.
        return String::new();
    }

    let parts: Vec<String> = args
        .iter()
        .filter_map(|arg| {
            // Apply per-language argument renames before emitting the call.
            let arg_name: &str = arg_name_map
                .and_then(|m| m.get(&arg.name).map(String::as_str))
                .unwrap_or(&arg.name);

            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field);
            // R extendr-generated wrappers do not preserve Option<T> defaults from
            // the Rust signature — every parameter is positional and required at
            // the R level. To keep generated calls valid we must pass a placeholder
            // (`NULL` for `Option<T>`, `<OptionsType>$default()` for typed
            // configs) whenever the fixture omits an optional value.
            let val = match val {
                Some(v) if !(v.is_null() && arg.optional) => v,
                _ => {
                    if !arg.optional {
                        return None;
                    }
                    if arg.arg_type == "json_object" {
                        let r_value = r_default_for_config_arg(arg_name, options_type);
                        return Some(format!("{arg_name} = {r_value}"));
                    }
                    return Some(format!("{arg_name} = NULL"));
                }
            };
            // The extendr bindings expect owned PORs (ExternalPtr) for typed
            // config arguments — passing an R `list()` raises
            // `Expected ExternalPtr got List`. The fixtures don't carry the
            // option fields needed to round-trip through the configured type's constructor,
            // so emit `<OptionsType>$default()` whenever a `json_object` arg
            // resolves to an empty / object-shaped JSON value or NULL.
            if arg.arg_type == "json_object" && (val.is_null() || val.as_object().is_some_and(|m| m.is_empty())) {
                let r_value = r_default_for_config_arg(arg_name, options_type);
                return Some(format!("{arg_name} = {r_value}"));
            }
            // Non-empty json_object for typed config args: use `TypeName$from_json(jsonlite::toJSON(...))`
            // so the Rust function receives a proper ExternalPtr, not a list.
            // When options_type is set, always wrap; when not set, emit as plain R list.
            if arg.arg_type == "json_object" && val.is_object() {
                // If options_type is provided, wrap the object with TypeName$from_json(...)
                // regardless of whether this is an optional or required config parameter.
                if let Some(type_name) =
                    crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, val)
                {
                    // Use the `I(...)` (AsIs) wrapper for array-valued fields so
                    // `jsonlite::toJSON(..., auto_unbox = TRUE)` preserves them as
                    // JSON arrays. Without this, single-element vectors get
                    // unboxed to scalars (e.g. `c("foo")` → `"foo"`) and serde
                    // rejects them when deserializing `Vec<T>` fields.
                    let r_list = json_to_r_preserve_arrays(val, true);
                    let json_expr = if crate::e2e::codegen::value_contains_mock_url_placeholder(val) {
                        let env_key = crate::e2e::codegen::mock_url_env_key(&fixture.id);
                        let base_var = format!(".{}_mock_base_url", arg_name);
                        setup_lines.push(format!(
                            "{base_var} <- Sys.getenv(\"{env_key}\", unset = paste0(Sys.getenv(\"MOCK_SERVER_URL\"), \"/fixtures/{}\"))",
                            fixture.id
                        ));
                        format!(
                            "gsub(\"{}\", {base_var}, jsonlite::toJSON({r_list}, auto_unbox = TRUE), fixed = TRUE)",
                            crate::e2e::codegen::MOCK_URL_PLACEHOLDER
                        )
                    } else {
                        format!("jsonlite::toJSON({r_list}, auto_unbox = TRUE)")
                    };
                    let r_value = format!("{type_name}$from_json({json_expr})");
                    return Some(format!("{arg_name} = {r_value}"));
                }
                // No options_type: emit as plain R list (backward compat for optional-style args).
                let r_value = json_to_r(val, true);
                return Some(format!("{arg_name} = {r_value}"));
            }
            // `json_object` arrays are passed to extendr functions whose Rust
            // signature is `items: String` (JSON-serialized object items). The
            // wrapper has no R-list → JSON conversion, so we must serialize the
            // fixture value to a literal JSON string at test-emit time.
            //
            // Exception: when `element_type = "String"` the Rust signature is
            // `Vec<String>` (e.g. `embed_texts(texts: Vec<String>, ...)`), which
            // extendr binds as a native R character vector. Passing a JSON
            // literal there would land as a single-element character vector
            // containing the literal bytes `["a","b"]`, which is not what the
            // caller intended. Emit a plain `c("a","b")` literal instead.
            if arg.arg_type == "json_object" && val.is_array() {
                if arg.element_type.as_deref() == Some("String") {
                    // `c()` is `NULL` in R, which extendr rejects with
                    // `Expected Strings got Null` when the Rust signature is
                    // `Vec<String>`. Emit a typed empty char vector for the
                    // empty-input case so the binding sees `character(0)`.
                    let r_value = if val.as_array().is_some_and(|arr| arr.is_empty()) {
                        "character(0)".to_string()
                    } else {
                        json_to_r(val, false)
                    };
                    return Some(format!("{arg_name} = {r_value}"));
                }
                let json_literal = serde_json::to_string(val).unwrap_or_else(|_| "[]".to_string());
                let escaped = escape_r(&json_literal);
                return Some(format!("{arg_name} = \"{escaped}\""));
            }
            // `bytes` arg type: convert string fixture values into runtime
            // `readBin(...)` calls so the wrapper receives raw bytes instead
            // of an R character vector. This mirrors the Python emit_bytes_arg
            // helper and is what the extendr binding for Vec<u8> expects.
            if arg.arg_type == "bytes" {
                if let Some(raw) = val.as_str() {
                    let r_value = render_bytes_value(raw);
                    return Some(format!("{arg_name} = {r_value}"));
                }
            }
            // `file_path` arg type: fixtures encode relative paths that resolve
            // against the repo's `test_documents/` directory. Using a runtime
            // helper that anchors paths to that directory avoids fragility from
            // testthat resetting the working directory between files.
            if arg.arg_type == "file_path" {
                if let Some(raw) = val.as_str() {
                    if !raw.starts_with('/') && !raw.is_empty() {
                        let escaped = escape_r(raw);
                        return Some(format!("{arg_name} = .resolve_fixture(\"{escaped}\")"));
                    }
                }
            }
            // `test_backend` arg type: emit a test stub for trait implementations.
            if arg.arg_type == "test_backend" {
                if let Some(trait_name) = &arg.trait_name {
                    if let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) {
                        let methods: Vec<&crate::core::ir::MethodDef> = type_defs
                            .iter()
                            .find(|t| t.name == *trait_name)
                            .map(|t| t.methods.iter().collect())
                            .unwrap_or_default();
                        let emission = crate::e2e::codegen::emit_test_backend("r", trait_bridge, &methods, fixture);
                        // Emit the backend list definition before the call site.
                        if !emission.setup_block.is_empty() {
                            setup_lines.push(emission.setup_block.trim_end().to_string());
                        }
                        // Collect teardown for trait-bridge tests to clean up after assertions.
                        teardown_block.push_str(&emission.teardown_block);
                        return Some(format!("{arg_name} = {}", emission.arg_expr));
                    }
                }
                let emission = crate::e2e::codegen::TestBackendEmission::unimplemented("r");
                return Some(format!("{arg_name} = NULL # {}", emission.arg_expr));
            }
            Some(format!("{arg_name} = {}", json_to_r(val, true)))
        })
        .collect();

    parts.join(", ")
}

/// Render a `bytes` fixture value as the R expression that produces a raw
/// vector at test time. Mirrors python's `emit_bytes_arg` classifier so we can
/// support both file-path style fixtures (`"pdf/fake_memo.pdf"`) and inline
/// text payloads (`"<html>..."`). The resulting expression is dropped directly
/// into the call site, e.g. `content = readBin(.resolve_fixture("pdf/fake_memo.pdf"), ...)`.
fn render_bytes_value(raw: &str) -> String {
    if raw.starts_with('<') || raw.starts_with('{') || raw.starts_with('[') || raw.contains(' ') {
        // Inline text payload — encode to raw via charToRaw.
        let escaped = escape_r(raw);
        return format!("charToRaw(\"{escaped}\")");
    }
    let first = raw.chars().next().unwrap_or('\0');
    if first.is_ascii_alphanumeric() || first == '_' {
        if let Some(slash) = raw.find('/') {
            if slash > 0 {
                let after = &raw[slash + 1..];
                if after.contains('.') && !after.is_empty() {
                    let escaped = escape_r(raw);
                    return format!(
                        "readBin(.resolve_fixture(\"{escaped}\"), what = \"raw\", n = file.info(.resolve_fixture(\"{escaped}\"))$size)"
                    );
                }
            }
        }
    }
    // Default to inline text encoding — matches Python's InlineText branch.
    let escaped = escape_r(raw);
    format!("charToRaw(\"{escaped}\")")
}

/// Map the extractor argument name onto its R `*Config$default()` constructor.
/// Falls back to `NULL` for unknown names so optional/default config slots stay
/// absent instead of passing a plain R list to an ExternalPtr-backed DTO.
///
/// When `options_type` is provided, emit the corresponding typed default.
/// Otherwise leave the optional slot unset instead of guessing a project type.
fn r_default_for_config_arg(arg_name: &str, options_type: Option<&str>) -> String {
    if let Some(type_name) = options_type {
        return format!("{type_name}$default()");
    }
    let _ = arg_name;
    "NULL".to_string()
}

#[cfg(test)]
mod tests {
    use super::{RArgsContext, build_args_string, strip_options_arg};
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::ArgMapping;
    use crate::e2e::fixture::Fixture;
    use serde_json::json;

    #[test]
    fn strip_options_arg_preserves_nested_commas() {
        let args = "source = \"a,b\", options = list(visitor = visitor, flags = c(\"x,y\")), limit = 2";

        assert_eq!(strip_options_arg(args), "source = \"a,b\", limit = 2");
    }

    #[test]
    fn build_args_string_wraps_typed_json_object_with_preserved_arrays() {
        let input = json!({
            "options": {
                "formats": ["Pdf"],
                "enabled": true
            }
        });
        let args = vec![ArgMapping {
            name: "options".to_string(),
            field: "input.options".to_string(),
            arg_type: "json_object".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }];
        let fixture = Fixture {
            id: "typed_options".to_string(),
            ..Fixture::default()
        };
        let config = ResolvedCrateConfig::default();
        let mut setup_lines = Vec::new();
        let mut teardown_block = String::new();

        let rendered = build_args_string(
            &input,
            &args,
            RArgsContext {
                arg_name_map: None,
                options_type: Some("ExtractOptions"),
                fixture: &fixture,
                config: &config,
                type_defs: &[],
                setup_lines: &mut setup_lines,
                teardown_block: &mut teardown_block,
            },
        );

        assert!(rendered.starts_with("options = ExtractOptions$from_json(jsonlite::toJSON(list("));
        assert!(
            rendered.ends_with("), auto_unbox = TRUE))"),
            "typed options arg should round-trip through jsonlite::toJSON, got: {rendered}"
        );
        assert!(
            rendered.contains("\"formats\" = I(c(\"pdf\"))"),
            "array fields must be preserved with I(c(...)), got: {rendered}"
        );
        assert!(
            rendered.contains("\"enabled\" = TRUE"),
            "scalar fields should remain ordinary R literals, got: {rendered}"
        );
        assert!(setup_lines.is_empty());
        assert!(teardown_block.is_empty());
    }
}
