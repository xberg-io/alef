//! Fallible NAPI input coercions, kept outside shared conversion policy.

use super::containers::ContainerContext;
use crate::core::ir::{ParamDef, TypeRef};
use quote::ToTokens;
use syn::parse::Parser;

#[cfg(test)]
mod tests;

pub(in crate::backends::napi) const INVALID_ARG: &str =
    ".map_err(|error| napi::Error::new(napi::Status::InvalidArg, format!(\"invalid JSON input: {error}\")))";

pub(super) fn invalid_arg(core: &str) -> String {
    let message = format!("invalid JSON for {core}: {{error}}");
    format!(".map_err(|error| napi::Error::new(napi::Status::InvalidArg, format!({message:?})))")
}

fn binding_identifier(local: &syn::Local) -> Option<&syn::Ident> {
    let pattern = match &local.pat {
        syn::Pat::Type(typed) => typed.pat.as_ref(),
        other => other,
    };
    match pattern {
        syn::Pat::Ident(ident) => Some(&ident.ident),
        _ => None,
    }
}

fn input_binding_name(block: &syn::Block, parameter: &ParamDef) -> String {
    let owned = format!("{}_owned", parameter.name);
    if block.stmts.iter().any(|statement| {
        matches!(statement, syn::Stmt::Local(local) if binding_identifier(local).is_some_and(|name| name == &owned))
    }) {
        owned
    } else {
        format!("{}_core", parameter.name)
    }
}

pub(in crate::backends::napi) fn has_inputs(context: Option<&ContainerContext>, params: &[ParamDef]) -> bool {
    context.is_some_and(|context| params.iter().any(|parameter| context.is_fallible(&parameter.ty)))
}

pub(in crate::backends::napi) fn prepare_bindings(
    generated: String,
    context: Option<&ContainerContext>,
    params: &[ParamDef],
    signature: &[ParamDef],
    required_unwrapped: bool,
) -> String {
    bind_inputs(
        generated,
        context,
        params,
        signature,
        required_unwrapped,
        input_binding_name,
    )
}

/// Convert fallible inputs ahead of an adapter body. The body comes from a shared adapter
/// template that reads each parameter by its own name (`req.into()`), so the converted value
/// shadows the parameter: the template's `.into()` is then the identity conversion.
pub(in crate::backends::napi) fn prepare_adapter_bindings(
    generated: String,
    context: Option<&ContainerContext>,
    params: &[ParamDef],
) -> String {
    bind_inputs(generated, context, params, params, true, |_, parameter| {
        parameter.name.clone()
    })
}

fn bind_inputs(
    generated: String,
    context: Option<&ContainerContext>,
    params: &[ParamDef],
    signature: &[ParamDef],
    required_unwrapped: bool,
    binding_name: impl Fn(&syn::Block, &ParamDef) -> String,
) -> String {
    let Some(context) = context.filter(|context| has_inputs(Some(context), params)) else {
        return generated;
    };
    let mut block: syn::Block =
        syn::parse_str(&format!("{{{generated}}}")).expect("generated input bindings must parse");
    for (parameter, declared) in params.iter().zip(signature) {
        if !context.is_fallible(&parameter.ty) {
            continue;
        }
        let name = binding_name(&block, parameter);
        let optional_default = declared.optional && !parameter.optional && !required_unwrapped;
        let expression = context.input_expression(parameter, optional_default);
        let mut found = false;
        for statement in &mut block.stmts {
            let syn::Stmt::Local(local) = statement else {
                continue;
            };
            if binding_identifier(local).is_some_and(|identifier| identifier == &name) {
                assert!(!found, "duplicate generated input binding {name}");
                *local.init.as_mut().expect("input binding must have a value").expr =
                    syn::parse_str(&expression).expect("fallible input expression must parse");
                found = true;
            }
        }
        if !found {
            let mutable = if parameter.is_mut { "mut " } else { "" };
            let added: syn::Block = syn::parse_str(&format!("{{let {mutable}{name} = {expression};}}"))
                .expect("fallible input binding must parse");
            block.stmts.splice(0..0, added.stmts);
        }
    }
    block
        .stmts
        .iter()
        .map(|statement| statement.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(in crate::backends::napi) fn call_args(
    generated: String,
    context: Option<&ContainerContext>,
    params: &[ParamDef],
) -> String {
    let Some(context) = context.filter(|context| has_inputs(Some(context), params)) else {
        return generated;
    };
    let arguments = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
        .parse_str(&generated)
        .expect("generated call arguments must parse");
    assert_eq!(
        arguments.len(),
        params.len(),
        "input coercion must examine every actual call argument"
    );
    arguments
        .iter()
        .zip(params)
        .map(|(argument, parameter)| {
            let original = argument.to_token_stream().to_string();
            if !context.is_fallible(&parameter.ty) {
                return original;
            }
            let core = format!("{}_core", parameter.name);
            if original
                .split(|character: char| !character.is_alphanumeric() && character != '_')
                .any(|word| word == core)
            {
                return original;
            }
            match (parameter.optional, parameter.is_ref, parameter.is_mut) {
                (true, true, true) => format!("{core}.as_mut()"),
                (true, true, false) => format!("{core}.as_ref()"),
                (false, true, true) => format!("&mut {core}"),
                (false, true, false) => format!("&{core}"),
                _ => core,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in crate::backends::napi) fn wrap_body(body: String, has_inputs: bool, already_fallible: bool) -> String {
    if has_inputs && !already_fallible {
        format!("Ok({{ {body} }})")
    } else {
        body
    }
}

pub(in crate::backends::napi) fn binding_type(context: Option<&ContainerContext>, ty: &TypeRef) -> TypeRef {
    context.map_or_else(|| ty.clone(), |context| context.binding_type_ref(ty))
}

pub(super) fn serde_try_from_impl(binding: &str, core: &str, expression: &str) -> String {
    let error = invalid_arg(core);
    format!(
        "impl TryFrom<{binding}> for {core} {{\n    type Error = serde_json::Error;\n    fn try_from(val: {binding}) -> std::result::Result<Self, Self::Error> {{\n        {expression}\n    }}\n}}\n\
         impl {binding} {{\n    fn try_into_core(self) -> napi::Result<{core}> {{\n        <{core} as TryFrom<Self>>::try_from(self){error}\n    }}\n}}\n"
    )
}

pub(in crate::backends::napi) fn fallible_callback_return(expression: &str, failure: &str) -> String {
    format!("let __result = match {expression} {{ Ok(value) => value, Err(e) => {{ {failure} }} }};")
}
