use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{MethodDef, TypeRef};
use crate::e2e::escape::escape_java;
use crate::e2e::fixture::{CallbackAction, Fixture};
use heck::{ToLowerCamelCase, ToUpperCamelCase};

pub(super) fn build_java_visitor(
    setup_lines: &mut Vec<String>,
    visitor_spec: &crate::e2e::fixture::VisitorSpec,
    class_name: &str,
    binding: &JavaVisitorBinding,
) -> String {
    setup_lines.push(format!("class _TestVisitor implements {} {{", binding.trait_type));
    for (method_name, action) in &visitor_spec.callbacks {
        emit_java_visitor_method(setup_lines, method_name, action, class_name, binding);
    }
    setup_lines.push("}".to_string());
    setup_lines.push("var visitor = new _TestVisitor();".to_string());
    "visitor".to_string()
}

#[derive(Debug, Clone)]
pub(super) struct JavaVisitorBinding {
    pub(super) options_type: String,
    pub(super) options_field: String,
    pub(super) trait_type: String,
    pub(super) context_type: String,
    pub(super) result_type: String,
    pub(super) methods: Vec<JavaVisitorMethod>,
    pub(super) has_missing_method_metadata: bool,
}

#[derive(Debug, Clone)]
pub(super) struct JavaVisitorMethod {
    pub(super) name: String,
    pub(super) params: String,
}

pub(super) fn java_visitor_binding(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    visitor_spec: Option<&crate::e2e::fixture::VisitorSpec>,
    fallback_options_type: Option<&str>,
) -> Option<JavaVisitorBinding> {
    let bridge = config
        .trait_bridges_for(crate::core::config::Language::Java)
        .find(|bridge| bridge.options_type.is_some() && bridge.resolved_options_field().is_some())?;
    let trait_def = type_defs.iter().find(|type_def| type_def.name == bridge.trait_name);
    let callback_methods: Vec<&MethodDef> = visitor_spec
        .map(|spec| {
            spec.callbacks
                .keys()
                .filter_map(|name| {
                    trait_def.and_then(|type_def| type_def.methods.iter().find(|method| method.name == *name))
                })
                .collect()
        })
        .unwrap_or_default();
    let methods = visitor_spec
        .map(|spec| {
            spec.callbacks
                .keys()
                .filter_map(|name| {
                    trait_def
                        .and_then(|type_def| type_def.methods.iter().find(|method| method.name == *name))
                        .map(java_visitor_method)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let has_missing_method_metadata = visitor_spec.is_some_and(|spec| methods.len() != spec.callbacks.len());
    Some(JavaVisitorBinding {
        options_type: fallback_options_type
            .or(bridge.options_type.as_deref())
            .map(str::to_string)?,
        options_field: bridge.resolved_options_field()?.to_string(),
        trait_type: bridge.trait_name.clone(),
        context_type: bridge.context_type.clone().or_else(|| {
            callback_methods
                .iter()
                .find_map(|method| first_named_param_type(method))
        })?,
        result_type: bridge.result_type.clone().or_else(|| {
            callback_methods
                .iter()
                .find_map(|method| named_type(&method.return_type))
        })?,
        methods,
        has_missing_method_metadata,
    })
}

fn java_visitor_method(method: &MethodDef) -> JavaVisitorMethod {
    let params = method
        .params
        .iter()
        .map(|param| format!("{} {}", java_visitor_type(&param.ty), param.name.to_lower_camel_case()))
        .collect::<Vec<_>>()
        .join(", ");
    JavaVisitorMethod {
        name: method.name.clone(),
        params,
    }
}

fn java_visitor_type(ty: &crate::core::ir::TypeRef) -> String {
    use crate::backends::java::type_map::java_type;
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) => java_visitor_type(inner),
        TypeRef::Vec(inner) => format!("java.util.List<{}>", java_visitor_type(inner)),
        TypeRef::Map(key, value) => {
            format!(
                "java.util.Map<{}, {}>",
                java_visitor_type(key),
                java_visitor_type(value)
            )
        }
        _ => java_type(ty).into_owned(),
    }
}

pub(super) fn java_visitor_imports(
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixtures: &[&Fixture],
) -> std::collections::BTreeSet<String> {
    let mut imports = std::collections::BTreeSet::new();
    for fixture in fixtures.iter().filter(|fixture| fixture.visitor.is_some()) {
        if let Some(binding) = java_visitor_binding(config, type_defs, fixture.visitor.as_ref(), None) {
            imports.insert(binding.trait_type);
            imports.insert(binding.context_type);
            imports.insert(binding.result_type);
        }
    }
    imports
}

pub(super) fn first_named_param_type(method: &MethodDef) -> Option<String> {
    method.params.iter().find_map(|param| named_type(&param.ty))
}

pub(super) fn named_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Named(name) => Some(name.clone()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => named_type(inner),
        TypeRef::Map(key, value) => named_type(key).or_else(|| named_type(value)),
        _ => None,
    }
}

pub(super) fn apply_java_visitor_arg(
    setup_lines: &mut Vec<String>,
    args_str: &str,
    args: &[crate::e2e::config::ArgMapping],
    visitor_var: &str,
    binding: &JavaVisitorBinding,
) -> String {
    let wither = format!("with{}", binding.options_field.to_upper_camel_case());
    if let Some(options_arg) = args
        .iter()
        .find(|arg| arg.arg_type == "json_object" && args_str.split(", ").any(|part| part == arg.name))
    {
        setup_lines.push(format!(
            "{} = {}.{}({});",
            options_arg.name, options_arg.name, wither, visitor_var
        ));
        return args_str.to_string();
    }

    // Records emit `withVisitor` on the Builder, not the record itself; use
    // the builder-chain pattern (`Options.builder().withVisitor(v).build()`).
    let options_expr = format!("{}.builder().{}({}).build()", binding.options_type, wither, visitor_var);

    // When the fixture provided no options value, `build_args_and_setup` already
    // inlined `{OptionsType}.builder().build()` as the trailing positional arg.
    // Replace that empty default with the visitor-bound builder expression rather
    // than appending a second options arg (which would produce the wrong arity).
    let default_options_expr = format!("{}.builder().build()", binding.options_type);
    if let Some(stripped) = args_str.strip_suffix(&format!(", {default_options_expr}")) {
        return format!("{stripped}, {options_expr}");
    }
    if args_str == default_options_expr {
        return options_expr;
    }

    if args_str.is_empty() {
        options_expr
    } else if let Some(stripped) = args_str.strip_suffix(", null") {
        format!("{stripped}, {options_expr}")
    } else {
        format!("{args_str}, {options_expr}")
    }
}

/// Emit a Java visitor method for a callback action.
pub(super) fn emit_java_visitor_method(
    setup_lines: &mut Vec<String>,
    method_name: &str,
    action: &CallbackAction,
    _class_name: &str,
    binding: &JavaVisitorBinding,
) {
    let camel_method = method_to_camel(method_name);
    let params = binding
        .methods
        .iter()
        .find(|method| method.name == method_name)
        .map(|method| method.params.clone())
        .unwrap_or_else(|| format!("{} context", binding.context_type));

    // Determine action type and values for template
    let (action_type, action_value, format_args) = match action {
        CallbackAction::Skip => ("skip", String::new(), Vec::new()),
        CallbackAction::Continue => ("continue", String::new(), Vec::new()),
        CallbackAction::PreserveHtml => ("preserve_html", String::new(), Vec::new()),
        CallbackAction::Custom { output } => ("custom_literal", escape_java(output), Vec::new()),
        CallbackAction::CustomTemplate { template, .. } => {
            // Extract {placeholder} names from the template (in order of appearance).
            let mut format_str = String::with_capacity(template.len());
            let mut format_args: Vec<String> = Vec::new();
            let mut chars = template.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '{' {
                    // Collect identifier chars until '}'.
                    let mut name = String::new();
                    let mut closed = false;
                    for inner in chars.by_ref() {
                        if inner == '}' {
                            closed = true;
                            break;
                        }
                        name.push(inner);
                    }
                    if closed && !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        let camel_name = name.as_str().to_lower_camel_case();
                        format_args.push(camel_name);
                        format_str.push_str("%s");
                    } else {
                        // Not a simple placeholder — emit literally.
                        format_str.push('{');
                        format_str.push_str(&name);
                        if closed {
                            format_str.push('}');
                        }
                    }
                } else {
                    format_str.push(ch);
                }
            }
            let escaped = escape_java(&format_str);
            if format_args.is_empty() {
                ("custom_literal", escaped, Vec::new())
            } else {
                ("custom_formatted", escaped, format_args)
            }
        }
    };

    // Emit the unsupported-diagnostic stub only when method metadata is missing.
    // ~keep The Jinja template renders `new {result_type}.Skip()` / `.Continue()` /
    // `.PreserveHtml()` / `.Custom(...)` — direct construction of the variant record
    // nested inside the sealed interface, not a static-factory call. alef's Java
    // backend emits every sum-type variant as `record Variant(...) implements
    // ResultType { }` nested in `ResultType` — see `gen_java_tagged_union` in
    // `backends/java/gen_bindings/types/enums.rs` and `visit_result.jinja` in
    // `backends/java/templates/` (the dedicated visitor-result generator used when a
    // `[[trait_bridges]]` entry resolves). Both paths nest the record the same way, so
    // `new ResultType.Variant(args)` compiles regardless of which one produced the
    // type — unlike a `.variant()` static-factory call, which only exists on the
    // dedicated visitor-result path and is absent from the generic tagged-union
    // shape. The host project opts in to trait-bridge visitor generation by setting
    // `[[trait_bridges]].result_type` in `alef.toml`.
    let unsupported_diagnostic = binding
        .has_missing_method_metadata
        .then(|| format!("visitor fixture callback '{method_name}' requires explicit e2e method metadata"));

    let rendered = crate::e2e::template_env::render(
        "java/visitor_method.jinja",
        minijinja::context! {
            camel_method,
            params,
            result_type => &binding.result_type,
            action_type,
            action_value,
            format_args => format_args,
            unsupported_diagnostic => unsupported_diagnostic,
        },
    );
    // ~keep `visitor_method.jinja` can render more than one physical line; pushing
    // it as one `setup_lines` entry would only get the outer template's indent on
    // the first — see `split_rendered_lines` in `snippet.rs`.
    setup_lines.extend(super::snippet::split_rendered_lines(&rendered));
}

/// Convert snake_case method names to Java camelCase.
pub(super) fn method_to_camel(snake: &str) -> String {
    snake.to_lower_camel_case()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> JavaVisitorBinding {
        JavaVisitorBinding {
            options_type: "RenderOptions".into(),
            options_field: "visitor".into(),
            trait_type: "TextVisitor".into(),
            context_type: "VisitContext".into(),
            result_type: "VisitResult".into(),
            methods: vec![JavaVisitorMethod {
                name: "visit_text".into(),
                params: "String text".into(),
            }],
            has_missing_method_metadata: false,
        }
    }

    /// ~keep alef's Java backend nests every sum-type variant as `record Variant(...)
    /// implements ResultType { }` and never guarantees a static factory (see the
    /// `~keep` comment on `emit_java_visitor_method`), so a unit variant must be
    /// constructed with `new ResultType.Variant()`, not the Dart-style
    /// `ResultType.variant()` static-factory call.
    #[test]
    fn unit_variant_uses_direct_record_construction() {
        let mut setup = Vec::new();
        emit_java_visitor_method(&mut setup, "visit_text", &CallbackAction::Skip, "Doc", &binding());
        let rendered = setup.join("\n");

        assert!(rendered.contains("new VisitResult.Skip()"), "{rendered}");
        assert!(!rendered.contains("VisitResult.skip()"), "{rendered}");
    }

    /// ~keep A payload-carrying variant (`Custom`) takes the same
    /// `new Outer.Variant(args)` form, with the escaped string as the record's
    /// positional constructor argument.
    #[test]
    fn payload_variant_uses_direct_record_construction_with_args() {
        let mut setup = Vec::new();
        let action = CallbackAction::Custom {
            output: "replacement".into(),
        };
        emit_java_visitor_method(&mut setup, "visit_text", &action, "Doc", &binding());
        let rendered = setup.join("\n");

        assert!(
            rendered.contains("new VisitResult.Custom(\"replacement\")"),
            "{rendered}"
        );
        assert!(!rendered.contains("VisitResult.custom("), "{rendered}");
    }

    /// ~keep Regression guard: `java/snippet_body.jinja` prepends the method body's
    /// indent to each `setup_lines` *entry* once, not to every physical line inside
    /// it (see `split_rendered_lines` in `snippet.rs`). Any producer that hands back
    /// a rendered block containing an embedded newline and pushes it as a single
    /// `setup_lines` entry would put every line after the first at column 0 once
    /// wrapped by the outer template. `emit_java_visitor_method` must route its
    /// render through `split_rendered_lines` so every element it contributes is a
    /// single physical line, whatever the template's own line count happens to be.
    #[test]
    fn visitor_method_emission_never_pushes_an_embedded_newline() {
        let mut setup = Vec::new();
        emit_java_visitor_method(&mut setup, "visit_text", &CallbackAction::Skip, "Doc", &binding());

        assert!(!setup.is_empty(), "{setup:?}");
        for line in &setup {
            assert!(!line.contains('\n'), "setup_lines entry embeds a newline: {line:?}");
        }
        assert!(
            setup.iter().any(|line| line.contains("new VisitResult.Skip()")),
            "{setup:?}"
        );
    }
}
