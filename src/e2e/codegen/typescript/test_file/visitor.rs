use super::*;

#[derive(Debug, Clone)]
pub(in crate::e2e::codegen::typescript::test_file) struct WasmVisitorBinding {
    pub(in crate::e2e::codegen::typescript::test_file) options_type: String,
    pub(in crate::e2e::codegen::typescript::test_file) options_field: String,
    pub(in crate::e2e::codegen::typescript::test_file) handle_type: String,
}

pub(in crate::e2e::codegen::typescript::test_file) fn wasm_visitor_binding(
    config: &crate::core::config::ResolvedCrateConfig,
    fallback_options_type: Option<&str>,
) -> Option<WasmVisitorBinding> {
    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.options_type.is_some() && bridge.resolved_options_field().is_some())?;
    let wasm_prefix = config.wasm_type_prefix();
    let options_type = fallback_options_type
        .or(bridge.options_type.as_deref())
        .map(|name| wasm_class_name(name.strip_prefix(&wasm_prefix).unwrap_or(name), &wasm_prefix))?;
    let handle_type = bridge
        .type_alias
        .as_deref()
        .map(|name| wasm_class_name(name.strip_prefix(&wasm_prefix).unwrap_or(name), &wasm_prefix))
        .unwrap_or_else(|| format!("Wasm{}Bridge", bridge.trait_name));

    Some(WasmVisitorBinding {
        options_type,
        options_field: bridge.resolved_options_field()?.to_string(),
        handle_type,
    })
}

pub(in crate::e2e::codegen::typescript::test_file) fn apply_wasm_visitor_arg(
    args_str: &str,
    visitor_arg: &str,
    binding: &WasmVisitorBinding,
) -> String {
    let visitor_assignment = format!(
        "_u.{} = new {}({visitor_arg});",
        snake_to_camel(&binding.options_field),
        binding.handle_type
    );
    let iife = format!(
        "(() => {{ const _u = {}.default(); {visitor_assignment} return _u; }})()",
        binding.options_type
    );
    if args_str.is_empty() {
        iife
    } else if let Some(return_pos) = args_str.rfind("return _u;") {
        let (iife_body, ret_part) = args_str.split_at(return_pos);
        format!("{iife_body}{visitor_assignment} {ret_part}")
    } else if let Some(stripped) = args_str.strip_suffix(", undefined") {
        format!("{stripped}, {iife}")
    } else {
        format!("{args_str}, {iife}")
    }
}

/// Build the `(html, opts)` argument list for a Node visitor test. The NAPI binding reads
/// the visitor from `options.visitor`, so we always synthesize an options object with the
/// visitor merged in. The `as any` cast keeps strict-mode TypeScript happy because the
/// generated `VisitorHandle` field type is opaque.
pub(in crate::e2e::codegen::typescript::test_file) fn node_visitor_args(args_str: &str, visitor_arg: &str) -> String {
    if args_str.is_empty() {
        return format!("{{ visitor: {visitor_arg} as any }}");
    }
    if let Some(head) = args_str.strip_suffix(", undefined") {
        return format!("{head}, {{ visitor: {visitor_arg} as any }}");
    }
    // If args_str contains a comma, the last segment is the existing options literal —
    // spread it so the visitor merges into the user's options object.
    if let Some((head, tail)) = split_last_top_level_arg(args_str) {
        return format!("{head}, {{ ...({tail}), visitor: {visitor_arg} as any }}");
    }
    format!("{args_str}, {{ visitor: {visitor_arg} as any }}")
}

/// Split `args_str` into `(everything-before-last-comma, last-arg)`, respecting nested
/// brace/bracket/paren depth so we don't split inside an object literal or call expression.
/// Returns `None` when there's no top-level comma.
fn split_last_top_level_arg(args_str: &str) -> Option<(String, String)> {
    let mut depth: i32 = 0;
    let mut last_split: Option<usize> = None;
    let bytes = args_str.as_bytes();
    let mut in_string: Option<u8> = None;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if let Some(quote) = in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == quote {
                in_string = None;
            }
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => in_string = Some(b),
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            b',' if depth == 0 => last_split = Some(i),
            _ => {}
        }
    }
    last_split.map(|idx| {
        let head = args_str[..idx].to_string();
        let tail = args_str[idx + 1..].trim_start().to_string();
        (head, tail)
    })
}
