use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// The parameter list and return-type shape of every generated Magnus trait-bridge constructor.
/// This is the single decision point for the constructor's arity and fallibility: `bridge_wrap`
/// below builds its `{struct_name}::new(...)` call (args and the trailing `?`) from these
/// constants, and both constructor-definition emitters -- the full bridge path's
/// `trait_bridge_constructor.rs.jinja` (via `bridge_generator::gen_constructor`) and the
/// visitor-bridge path's `visitor_bridge.rs.jinja` (via `gen_visitor_bridge`) -- render their
/// `fn new(...)` signature from these same strings instead of retyping it. A future change to
/// arity or fallibility is made once here and takes effect everywhere, instead of requiring two
/// independently maintained copies that can silently drift apart -- which is exactly how the
/// visitor-bridge definition was left behind on the pre-#292 one-arg infallible shape after the
/// call site here moved to two args and a `?`. ~keep
pub(super) const BRIDGE_CTOR_PARAMS: &str = "rb_obj: magnus::Value, name: String";
pub(super) const BRIDGE_CTOR_RETURN_TYPE: &str = "Result<Self, magnus::Error>";
pub(super) const BRIDGE_CTOR_IS_FALLIBLE: bool = true;

/// A `Named` type, or a `Named` type behind one layer of `Optional`, extracts to its name;
/// anything else (primitives, `Vec`, opaque handles, deeper nesting) is not a candidate for the
/// `let {name}_core = ...` binding `gen_bridge_function` emits for non-opaque named params. Shared
/// by the fallibility check and the binding emission below so the two consult the exact same
/// notion of "named", instead of two hand-written pattern matches that can drift apart. ~keep
fn named_type_name(ty: &crate::core::ir::TypeRef) -> Option<&str> {
    match ty {
        crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
        crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Use a companion `name: String` parameter as the bridge's cached plugin identity when present.
/// Per-call callback bridges generally have no identity and retain the historical empty name.
fn bridge_name_expr(func: &crate::core::ir::FunctionDef, bridge_param_idx: usize) -> &'static str {
    let has_name = func.params.iter().enumerate().any(|(idx, param)| {
        idx != bridge_param_idx
            && param.name == "name"
            && !param.optional
            && matches!(param.ty, crate::core::ir::TypeRef::String)
    });
    if has_name { "name.clone()" } else { "String::new()" }
}

/// Generate a Magnus free function that has one parameter replaced by `magnus::Value` (a trait
/// bridge). The bridge is constructed before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &std::collections::HashSet<&str>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Rb", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));
    let bridge_name = bridge_name_expr(func, bridge_param_idx);
    let (bridge_handle_type, bridge_value) = if bridge_param.core_wrapper == crate::core::ir::CoreWrapper::Arc {
        (
            format!("std::sync::Arc<dyn {handle_path}>"),
            "std::sync::Arc::new(bridge)".to_string(),
        )
    } else {
        (
            handle_path.clone(),
            "std::sync::Arc::new(std::sync::Mutex::new(bridge))".to_string(),
        )
    };

    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<magnus::Value>", p.name));
            } else {
                sig_parts.push(format!("{}: magnus::Value", p.name));
            }
        } else {
            let promoted = (is_optional && idx > bridge_param_idx) || func.params[..idx].iter().any(|pp| pp.optional);
            let ty = if p.optional || promoted {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);

    // Non-bridge params that get a `let {name}_core = ...` binding below: a `Named`/`Optional<Named>`
    // param that isn't already an opaque handle. Computed once and reused by both the fallibility
    // check and `serde_bindings` itself, so they can't independently drift the way `has_error` used
    // to (it used to recompute a lookalike condition that dropped this entirely). ~keep
    let deser_params: Vec<_> = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            *idx != bridge_param_idx && named_type_name(&p.ty).is_some_and(|n| !opaque_types.contains(n))
        })
        .collect();

    // Of those, only a "default type" (`is_dt` below) skips the `?` — it gets a plain `.into()`.
    // Every other entry goes through a fallible `serde_json::from_str(...)?` / `.transpose()?`, so
    // the annotation must be `Result`-shaped whenever at least one does, independent of
    // `func.error_type`. ~keep
    let params_need_fallible_deser = deser_params
        .iter()
        .copied()
        .any(|(_, p)| !default_types.contains(named_type_name(&p.ty).unwrap_or_default()));

    // Derived from `BRIDGE_CTOR_IS_FALLIBLE` above rather than hardcoded, so the `?` here and the
    // constructor definitions' `Result`-shaped return type can never independently drift. ~keep
    let ctor_try = if BRIDGE_CTOR_IS_FALLIBLE { "?" } else { "" };

    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{bridge_handle_type}> = match {param_name} {{\n        \
             Some(v) if !v.is_nil() => {{\n            \
             let bridge = {struct_name}::new(v, {bridge_name}){ctor_try};\n            \
             Some({bridge_value} as {bridge_handle_type})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name}, {bridge_name}){ctor_try};\n        \
             {bridge_value} as {bridge_handle_type}\n    \
             }};"
        )
    };

    // `has_error` must reflect every fallible operation this function's body actually emits.
    // `bridge_wrap` above always constructs the bridge via `{struct_name}::new(...)?` (the
    // constructor validates required methods and builds the runtime dispatcher, both fallible
    // since #292 made trait-bridge constructors return `Result`) — read that fact from the
    // generated code itself rather than asserting it as a separate constant, so a future change
    // to `bridge_wrap` that removes or conditions the `?` is picked up here automatically instead
    // of needing a second, independent edit that can drift out of sync. ~keep
    let bridge_construction_is_fallible = bridge_wrap.contains('?');
    let has_error = func.error_type.is_some() || params_need_fallible_deser || bridge_construction_is_fallible;
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), e.to_string()))";

    let serde_bindings: String = deser_params
        .into_iter()
        .map(|(_, p)| {
            let name = &p.name;
            let named_type = named_type_name(&p.ty).unwrap_or_default().to_string();
            let core_path = format!("{core_import}::{named_type}");
            let is_dt = default_types.contains(named_type.as_str());
            if is_dt {
                if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                    format!("let {name}_core: Option<{core_path}> = {name}.map(Into::into);\n    ")
                } else {
                    format!("let {name}_core: {core_path} = {name}.into();\n    ")
                }
            } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                format!(
                    "let {name}_core: Option<{core_path}> = {name}.as_deref().filter(|s| *s != \"nil\").map(|s| serde_json::from_str(s){err_conv}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_core: {core_path} = serde_json::from_str(&{name}){err_conv}?;\n    "
                )
            }
        })
        .collect();

    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == bridge_param_idx {
                return p.name.clone();
            }
            match &p.ty {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => format!("{}_core", p.name),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if opaque_types.contains(n.as_str()) {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect();
    let call_args_str = call_args.join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args_str})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        // The core call itself already returns `Result`, so chaining `.map`/`.map_err` directly
        // onto it is already `Result`-typed — no extra `Ok(..)` wrap needed. ~keep
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        // The core call returns a bare value, but `bridge_construction_is_fallible` above is
        // always true for this generator (`bridge_wrap`'s constructor call is unconditionally
        // fallible; `serde_bindings` may add its own `?`s on top), so `has_error` is always true
        // here too and the signature above is `Result`-shaped. The tail expression must match
        // that: wrap the plain call in `Ok(..)` instead of handing back a bare value. ~keep
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}Ok({core_call})")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}let val = {core_call};\n    Ok({return_wrap})")
        }
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str("#[allow(unused_variables)]\n");
    let sig = format!("pub fn {func_name}({params_str}) -> {ret} {{\n    {body}\n}}\n");
    out.push_str(&sig);

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::magnus::type_map::MagnusMapper;
    use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

    /// Depth-aware arity of the parenthesized call/signature immediately following `needle` in
    /// `text` (top-level comma count + 1) -- so a nested-paren arg like `String::new()` isn't
    /// mistaken for an extra one.
    fn arity_after(text: &str, needle: &str) -> usize {
        let start = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not found in:\n{text}"))
            + needle.len();
        let mut depth = 1usize;
        let mut top_level_commas = 0usize;
        for ch in text[start..].chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ',' if depth == 1 => top_level_commas += 1,
                _ => {}
            }
        }
        top_level_commas + 1
    }

    /// Byte offset just past the closing paren of the parenthesized call/signature that starts
    /// right after `needle` in `text`.
    fn close_paren_after(text: &str, needle: &str) -> usize {
        let start = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not found in:\n{text}"))
            + needle.len();
        let mut depth = 1usize;
        for (i, ch) in text[start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return start + i + 1;
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced parens after {needle:?} in:\n{text}");
    }

    /// Whether a generated `{struct}::new(...)` CALL SITE is fallible: the first character
    /// after the closing paren is `?`.
    fn call_site_is_fallible(text: &str, needle: &str) -> bool {
        text[close_paren_after(text, needle)..].starts_with('?')
    }

    /// Whether a generated `pub fn new(...) -> ReturnType {` constructor DEFINITION is
    /// fallible: its return type (the text between the closing paren and the opening brace)
    /// names `Result`.
    fn definition_is_fallible(text: &str, needle: &str) -> bool {
        let after_close = close_paren_after(text, needle);
        let brace = text[after_close..]
            .find('{')
            .unwrap_or_else(|| panic!("no constructor body opening brace after {needle:?} in:\n{text}"));
        text[after_close..after_close + brace].contains("Result")
    }

    /// The regression this test guards: a visitor-shaped bridge (`is_visitor_bridge` in
    /// `bridge_generator::gen_trait_bridge` -- `type_alias` set, no `register_fn`/`super_trait`,
    /// every method carrying a Rust default impl) is exactly the shape PR #292's call-site change
    /// reached but its definition emitter (`gen_visitor_bridge`) did not: the call site here always
    /// emits a two-arg, `?`-suffixed `{struct}::new(value, name)` for every magnus bridge parameter,
    /// so every constructor DEFINITION must match that arity and fallibility -- independent of
    /// whether it takes the visitor-bridge path or the full trait-bridge path. ~keep
    #[test]
    fn visitor_bridge_constructor_matches_call_site_arity_and_fallibility() {
        let (api, trait_type, bridge_cfg) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();

        let definition = super::super::gen_trait_bridge(
            &trait_type,
            &bridge_cfg,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge definition should generate");
        assert!(
            definition.contains("pub fn new("),
            "visitor bridge must declare a constructor:\n{definition}"
        );
        let def_arity = arity_after(&definition, "pub fn new(");
        let def_is_fallible = definition_is_fallible(&definition, "pub fn new(");

        let func = FunctionDef {
            name: "inspect".to_string(),
            rust_path: "sample_core::inspect".to_string(),
            params: vec![ParamDef {
                name: "walker".to_string(),
                ty: TypeRef::Named("DocumentWalkerHandle".to_string()),
                optional: true,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        };
        let call_site = gen_bridge_function(
            &api,
            &func,
            0,
            &bridge_cfg,
            &MagnusMapper,
            &ahash::AHashSet::default(),
            &std::collections::HashSet::new(),
            "sample_core",
        );

        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Rb", &bridge_cfg);
        let ctor_call_needle = format!("{struct_name}::new(");
        assert!(
            call_site.contains(&ctor_call_needle),
            "call site must construct the bridge via {ctor_call_needle:?}:\n{call_site}"
        );
        let call_arity = arity_after(&call_site, &ctor_call_needle);
        let call_fallible = call_site_is_fallible(&call_site, &ctor_call_needle);

        assert_eq!(
            def_arity, call_arity,
            "constructor definition arity ({def_arity}) must match call site arity ({call_arity}); \
             definition:\n{definition}\ncall site:\n{call_site}"
        );
        assert_eq!(
            def_is_fallible, call_fallible,
            "constructor definition fallibility ({def_is_fallible}) must match call site \
             fallibility ({call_fallible}); definition:\n{definition}\ncall site:\n{call_site}"
        );
    }
}
