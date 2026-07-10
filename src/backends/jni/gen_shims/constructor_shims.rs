/// Emit a constructor shim for an opaque client type.
///
/// The `client_constructors` workspace config supplies the body template and
/// the ordered list of parameters.  Each parameter whose `ty` contains
/// `c_char` is received as `JString` and unmarshalled via `jstring_to_string`;
/// other parameter types are received as their JNI primitive equivalent.
///
/// The emitted shim returns `jlong` (a `Box::into_raw` pointer) on success or
/// `0` on failure (with a JNI exception pending).
fn emit_constructor_shim(
    out: &mut String,
    symbol: &str,
    ty: &TypeDef,
    config: &ResolvedCrateConfig,
    ctor: &ClientConstructorConfig,
) {
    let type_name = &ty.name;
    let core_prefix = core_use_path(config);

    let mut param_sigs = String::new();
    let mut unmarshal = String::new();
    let mut call_args = Vec::new();

    for param in &ctor.params {
        let rust_name = param.name.replace('-', "_");
        if param.ty.contains("c_char") {
            param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
            unmarshal.push_str(&render_string_unmarshal(&rust_name, "0"));
            call_args.push(rust_name.clone());
        } else {
            param_sigs.push_str(&render_param_decl(&rust_name, "jlong"));
            call_args.push(rust_name.clone());
        }
    }

    let body_expr = ctor
        .body
        .replace("{type_name}", type_name)
        .replace("{source_path}", &format!("{core_prefix}::{type_name}"));

    let call_expr = if call_args.is_empty() || body_expr.contains('(') {
        body_expr.clone()
    } else {
        format!("{}({})", body_expr, call_args.join(", "))
    };

    out.push_str(&template_env::render(
        "constructor_shim.rs.jinja",
        context! {
            symbol => symbol,
            param_sigs => param_sigs,
            unmarshal => unmarshal,
            call_expr => call_expr,
        },
    ));
}

/// Emit the destructor shim for an opaque type.
fn emit_destructor_shim(out: &mut String, symbol: &str, type_name: &str) {
    out.push_str(&template_env::render(
        "destructor_shim.rs.jinja",
        context! {
            symbol => symbol,
            type_name => type_name,
        },
    ));
}
