use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

use super::super::json::snake_to_camel;
use super::helpers::resolve_node_function_name;
use super::wasm::wasm_class_name;

pub(super) struct WasmVisitorBinding {
    pub(super) options_type: String,
    pub(super) options_field: String,
    pub(super) handle_type: String,
}

pub(super) fn wasm_visitor_binding(
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

pub(super) fn apply_wasm_visitor_arg(args_str: &str, visitor_arg: &str, binding: &WasmVisitorBinding) -> String {
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
pub(super) fn node_visitor_args(args_str: &str, visitor_arg: &str) -> String {
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
    let split = last_split?;
    let head = args_str[..split].trim_end().to_string();
    let tail = args_str[split + 1..].trim_start().to_string();
    Some((head, tail))
}

/// Detect if cache isolation is needed: checks if any fixture calls `cleanCache`
/// and if a `configure` function is available.
/// Returns (has_clean_cache, has_configure).
pub(super) fn detect_cache_isolation_needs(fixtures: &[&Fixture], e2e_config: &E2eConfig) -> (bool, bool) {
    let has_clean_cache = fixtures.iter().any(|fixture| {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        resolve_node_function_name(call_config) == "cleanCache"
    });

    let has_configure = e2e_config
        .calls
        .iter()
        .any(|(_, call_config)| resolve_node_function_name(call_config) == "configure")
        || resolve_node_function_name(&e2e_config.call) == "configure";

    (has_clean_cache, has_configure)
}

/// Emit the cache isolation setup code (beforeAll/afterAll blocks).
pub(super) fn emit_cache_isolation_setup(out: &mut String) {
    let rendered = crate::e2e::template_env::render("typescript/cache_isolation_setup.jinja", minijinja::context! {});
    out.push_str(&rendered);
}
