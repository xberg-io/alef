use crate::core::ir::{EntrypointDef, ServiceDef, TypeRef};

/// Format a Rust doc comment as a PHP docblock at the given column indent.
/// Single-line docs render as `// text`; multi-line docs render as a `/** ...
/// */` block with every line prefixed by ` * `. Blank doc lines become bare
/// ` *` separators so paragraph breaks survive.
pub(super) fn format_php_comment(text: &str, indent: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    if !trimmed.contains('\n') {
        return format!("{pad}// {trimmed}\n");
    }
    let mut out = format!("{pad}/**\n");
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            out.push_str(&pad);
            out.push_str(" *\n");
        } else {
            out.push_str(&pad);
            out.push_str(" * ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(&pad);
    out.push_str(" */\n");
    out
}

pub(super) fn render(template_name: &str, ctx: minijinja::Value) -> String {
    crate::backends::php::template_env::render(template_name, ctx)
}

/// Build the Rust constructor call for the service owner.
pub(super) fn build_ctor_call(service: &ServiceDef, owner_path: &str, _core_import: &str) -> String {
    if service.constructor.params.is_empty() {
        format!("{owner_path}::{}()", service.constructor.name)
    } else {
        format!("{owner_path}::{}()", service.constructor.name)
    }
}

/// Build the entrypoint invocation for a service method.
pub(super) fn build_ep_call(ep: &EntrypointDef, _service: &ServiceDef, _core_import: &str) -> String {
    let ep_method = &ep.method;
    let ep_args: Vec<String> = ep.params.iter().map(|p| p.name.clone()).collect();
    let args_str = ep_args.join(", ");
    let bind = if matches!(ep.return_type, TypeRef::Unit) {
        ""
    } else {
        "let _ = "
    };

    if ep.is_async {
        if args_str.is_empty() {
            format!(
                "    {bind}tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}())\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        } else {
            format!(
                "    {bind}tokio::runtime::Handle::current()\n        \
                 .block_on(owner.{ep_method}({args_str}))\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        }
    } else if ep.error_type.is_some() {
        if args_str.is_empty() {
            format!(
                "    {bind}owner.{ep_method}()\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        } else {
            format!(
                "    {bind}owner.{ep_method}({args_str})\n        \
                 .map_err(|e| PhpException::default(e.to_string()))?;\n"
            )
        }
    } else if args_str.is_empty() {
        format!("    {bind}owner.{ep_method}();\n")
    } else {
        format!("    {bind}owner.{ep_method}({args_str});\n")
    }
}

/// Convert a Rust enum path expression to a PHP class constant reference.
///
/// `"my_crate::Method::Get"` → `"Method::Get"`
/// `"Method::Get"` → `"Method::Get"`
///
/// Takes the last two `::` separated segments so that fully-qualified Rust
/// paths are trimmed to just `TypeName::Variant`.
fn rust_enum_expr_to_php(value_expr: &str) -> String {
    let parts: Vec<&str> = value_expr.split("::").collect();
    if parts.len() >= 2 {
        let type_name = parts[parts.len() - 2];
        let variant = parts[parts.len() - 1];
        format!("{type_name}::{variant}")
    } else {
        value_expr.to_owned()
    }
}

/// Build the PHP wrapper-constructor statement for a variant that has a
/// `wrapper_call`.
///
/// Returns a statement like
/// `$builder = RouteBuilder::new(Method::Get, $path);`
/// or `None` when the variant has no `wrapper_call`.
pub(super) fn build_php_wrapper_constructor_stmt(variant: &crate::core::ir::RegistrationVariant) -> Option<String> {
    use crate::core::ir::WrapperConstructorArg;
    let wc = variant.wrapper_call.as_ref()?;
    let wrapper_type = &wc.wrapper_type_name;
    let constructor = &wc.constructor_method;
    let metadata_param = &wc.metadata_param;

    let mut ctor_args: Vec<String> = Vec::new();
    for arg in &wc.args {
        match arg {
            WrapperConstructorArg::Fixed { value_expr, .. } => {
                ctor_args.push(rust_enum_expr_to_php(value_expr));
            }
            WrapperConstructorArg::Free { param } => {
                ctor_args.push(format!("${}", param.name));
            }
        }
    }
    let ctor_arg_str = ctor_args.join(", ");
    Some(format!(
        "${metadata_param} = {wrapper_type}::{constructor}({ctor_arg_str});"
    ))
}
