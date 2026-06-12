//! Go e2e visitor fixture emission.

use crate::codegen::naming::go_param_name;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::go_string_literal;
use crate::e2e::fixture::CallbackAction;
use std::fmt::Write as FmtWrite;

use super::test_backend::{method_to_camel, stub_go_type_with_context};

// ---------------------------------------------------------------------------
// Visitor generation
// ---------------------------------------------------------------------------

/// Derive a unique, exported Go struct name for a visitor from a fixture ID.
///
/// E.g. `visitor_continue_default` → `visitorContinueDefault` (unexported, avoids
/// polluting the exported API of the test package while still being package-level).
pub(super) fn visitor_struct_name(fixture_id: &str) -> String {
    use heck::ToUpperCamelCase;
    // Use UpperCamelCase so Go treats it as exported — required for method sets.
    format!("testVisitor{}", fixture_id.to_upper_camel_case())
}

/// Emit a package-level Go struct declaration and all its visitor methods.
///
/// The struct embeds `BaseVisitor` to satisfy all interface methods not
/// explicitly overridden by the fixture callbacks.
pub(super) fn emit_go_visitor_struct(
    out: &mut String,
    struct_name: &str,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    import_alias: &str,
    binding: Option<&GoVisitorBinding>,
) {
    let _ = writeln!(out, "type {struct_name} struct{{");
    let _ = writeln!(out, "\t{import_alias}.BaseVisitor");
    let _ = writeln!(out, "}}");
    for (method_name, action) in &visitor_spec.callbacks {
        let method = binding.and_then(|binding| binding.method(method_name));
        emit_go_visitor_method(out, struct_name, method_name, action, import_alias, method);
    }
}

pub(super) struct GoVisitorBinding {
    methods: Vec<GoVisitorMethod>,
}

impl GoVisitorBinding {
    fn method(&self, name: &str) -> Option<&GoVisitorMethod> {
        self.methods.iter().find(|method| method.name == name)
    }
}

struct GoVisitorMethod {
    name: String,
    result_type: String,
    params: String,
    pointer_params: std::collections::HashSet<String>,
}

pub(super) fn resolve_go_visitor_binding(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    import_alias: &str,
) -> Option<GoVisitorBinding> {
    let trait_name = config
        .trait_bridges
        .iter()
        .find_map(|bridge| bridge.result_type.as_ref().map(|_| bridge.trait_name.as_str()))?;
    let trait_def = type_defs.iter().find(|type_def| type_def.name == trait_name)?;
    let result_type = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.trait_name == trait_name)
        .and_then(|bridge| bridge.result_type.clone())
        .or_else(|| {
            visitor_spec.callbacks.keys().find_map(|name| {
                trait_def
                    .methods
                    .iter()
                    .find(|method| method.name == *name)
                    .and_then(|method| named_type(&method.return_type))
            })
        })?;
    let methods = visitor_spec
        .callbacks
        .keys()
        .filter_map(|name| {
            trait_def
                .methods
                .iter()
                .find(|method| method.name == *name)
                .map(|method| go_visitor_method(method, &result_type, import_alias))
        })
        .collect();
    Some(GoVisitorBinding { methods })
}

fn go_visitor_method(method: &crate::core::ir::MethodDef, result_type: &str, import_alias: &str) -> GoVisitorMethod {
    let mut pointer_params = std::collections::HashSet::new();
    let params = method
        .params
        .iter()
        .map(|param| {
            let name = go_param_name(&param.name);
            let base_ty = stub_go_type_with_context(
                &param.ty,
                &std::collections::HashSet::new(),
                import_alias,
                &std::collections::HashSet::new(),
            );
            // Honour `param.optional` so the stub signature matches the binding's
            // Visitor interface, which exposes optional params as pointers
            // (e.g. `src *string`).
            let ty = if param.optional && !base_ty.starts_with('*') && base_ty != "json.RawMessage" {
                format!("*{base_ty}")
            } else {
                base_ty
            };
            if ty.starts_with('*') {
                pointer_params.insert(name.clone());
            }
            format!("{name} {ty}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    GoVisitorMethod {
        name: method.name.clone(),
        result_type: result_type.to_string(),
        params,
        pointer_params,
    }
}

fn named_type(ty: &crate::core::ir::TypeRef) -> Option<String> {
    match ty {
        crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
        crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => named_type(inner),
        crate::core::ir::TypeRef::Map(key, value) => named_type(key).or_else(|| named_type(value)),
        _ => None,
    }
}

/// Emit a Go visitor method for a callback action on the named struct.
fn emit_go_visitor_method(
    out: &mut String,
    struct_name: &str,
    method_name: &str,
    action: &CallbackAction,
    import_alias: &str,
    method: Option<&GoVisitorMethod>,
) {
    let camel_method = method_to_camel(method_name);
    let params = method
        .map(|method| method.params.clone())
        .unwrap_or_else(|| format!("_ {import_alias}.SyntaxContext"));
    let result_type_name = method
        .map(|method| method.result_type.as_str())
        .unwrap_or("WalkDecision");
    let result_type = method
        .map(|method| format!("{import_alias}.{}", method.result_type))
        .unwrap_or_else(|| format!("{import_alias}.WalkDecision"));

    let _ = writeln!(out, "func (v *{struct_name}) {camel_method}({params}) {result_type} {{");
    if method.is_none() {
        let _ = writeln!(
            out,
            "\tpanic(\"go visitor fixture '{method_name}' requires trait_bridge result_type and IR method metadata\")"
        );
        let _ = writeln!(out, "}}");
        return;
    }
    if result_type_name != "WalkDecision" {
        let _ = writeln!(
            out,
            "\tpanic(\"go visitor fixture '{method_name}' requires explicit e2e result-action metadata for result type '{result_type_name}'\")"
        );
        let _ = writeln!(out, "}}");
        return;
    }
    match action {
        CallbackAction::Skip => {
            let _ = writeln!(out, "\treturn {import_alias}.WalkDecisionSkip()");
        }
        CallbackAction::Continue => {
            let _ = writeln!(out, "\treturn {import_alias}.WalkDecisionContinue()");
        }
        CallbackAction::PreserveHtml => {
            let _ = writeln!(out, "\treturn {import_alias}.WalkDecisionPreserveHTML()");
        }
        CallbackAction::Custom { output } => {
            let escaped = go_string_literal(output);
            let _ = writeln!(out, "\treturn {import_alias}.WalkDecisionCustom({escaped})");
        }
        CallbackAction::CustomTemplate { template, .. } => {
            // Convert {var} placeholders to %s format verbs and collect arg names.
            // E.g. `QUOTE: "{text}"` → fmt.Sprintf("QUOTE: \"%s\"", text)
            //
            // For pointer-typed params (e.g. `src *string`), dereference with `*`
            // — the test fixtures always supply a non-nil value for methods that
            // fire a custom template, so this is safe in practice.
            let ptr_params = method
                .map(|method| method.pointer_params.iter().map(String::as_str).collect())
                .unwrap_or_default();
            let (fmt_str, fmt_args) = template_to_sprintf(template, &ptr_params);
            let escaped_fmt = go_string_literal(&fmt_str);
            if fmt_args.is_empty() {
                let _ = writeln!(out, "\treturn {import_alias}.WalkDecisionCustom({escaped_fmt})");
            } else {
                let args_str = fmt_args.join(", ");
                let _ = writeln!(
                    out,
                    "\treturn {import_alias}.WalkDecisionCustom(fmt.Sprintf({escaped_fmt}, {args_str}))"
                );
            }
        }
    }
    let _ = writeln!(out, "}}");
}

/// Convert a `{var}` template string into a `fmt.Sprintf` format string and argument list.
///
/// For example, `QUOTE: "{text}"` becomes `("QUOTE: \"%s\"", vec!["text"])`.
///
/// Placeholder names in the template use snake_case (matching fixture field names); they
/// are converted to Go camelCase parameter names using `go_param_name` so they match the
/// generated visitor method signatures (e.g. `{input_type}` → `inputType`).
///
/// `ptr_params` — camelCase names of parameters that are `*string`; these are
/// dereferenced with `*` when used as `fmt.Sprintf` arguments.  The fixtures that
/// use `custom_template` on pointer-param methods always supply a non-nil value.
fn template_to_sprintf(template: &str, ptr_params: &std::collections::HashSet<&str>) -> (String, Vec<String>) {
    let mut fmt_str = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // Collect placeholder name until '}'.
            let mut name = String::new();
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
                name.push(inner);
            }
            fmt_str.push_str("%s");
            // Convert snake_case placeholder to Go camelCase to match method param names.
            let go_name = go_param_name(&name);
            // Dereference pointer params so fmt.Sprintf receives a string value.
            let arg_expr = if ptr_params.contains(go_name.as_str()) {
                format!("*{go_name}")
            } else {
                go_name
            };
            args.push(arg_expr);
        } else {
            fmt_str.push(c);
        }
    }
    (fmt_str, args)
}
