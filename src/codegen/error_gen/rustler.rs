use crate::core::ir::ErrorDef;

use super::shared::{duration_methods, error_variant_wildcard_pattern, millis_expr, to_snake_case};

/// Core path of an error type as the generated NIF crate names it.
fn core_error_path(error: &ErrorDef, core_import: &str) -> String {
    if error.rust_path.is_empty() {
        format!("{core_import}::{}", error.name)
    } else {
        error.rust_path.replace('-', "_")
    }
}

/// Errors the Elixir binding exposes as exception structs, in a fixed order.
pub fn rustler_typed_errors(errors: &[ErrorDef]) -> Vec<&ErrorDef> {
    let mut typed: Vec<&ErrorDef> = errors.iter().filter(|error| !error.binding_excluded).collect();
    typed.sort_by(|a, b| a.name.cmp(&b.name));
    typed
}

/// Name of the Elixir exception module for `error`, below the app module.
///
/// The app prefix is dropped from the error name (`LiterLlmError` in app `LiterLlm` becomes
/// `LiterLlm.Error`). When two errors would share a name, the full error name is kept.
pub fn rustler_error_module_name(error: &ErrorDef, app_module: &str, errors: &[&ErrorDef]) -> String {
    let short = match error.name.strip_prefix(app_module) {
        Some(rest) if !rest.is_empty() => rest.to_string(),
        _ => error.name.clone(),
    };
    let taken =
        errors
            .iter()
            .filter(|other| other.name != error.name)
            .any(|other| match other.name.strip_prefix(app_module) {
                Some(rest) if !rest.is_empty() => rest == short,
                _ => other.name == short,
            });
    if taken { error.name.clone() } else { short }
}

/// Name of the function that converts a core error into the NIF error.
pub fn rustler_converter_fn_name(error: &ErrorDef) -> String {
    format!("{}_to_nif_err", to_snake_case(&error.name))
}

/// Generate a converter that maps a core error to `AlefNifError::Typed`.
///
/// The kind is the snake_case variant name. The status is the error's `status_code` method
/// (0 when the error has none). The retry delay is the error's `retry_after` method in whole
/// milliseconds (absent when the error has none).
pub fn gen_rustler_error_converter(error: &ErrorDef, core_import: &str, exception_module: &str) -> String {
    let rust_path = core_error_path(error, core_import);
    let fn_name = rustler_converter_fn_name(error);

    let mut arms = Vec::with_capacity(error.variants.len() + 1);
    for variant in &error.variants {
        let pattern = error_variant_wildcard_pattern(&rust_path, variant);
        arms.push(format!("        {pattern} => \"{}\",", to_snake_case(&variant.name)));
    }
    arms.push("        #[allow(unreachable_patterns)]\n        _ => \"unknown\",".to_string());

    let status = if error.methods.iter().any(|m| m.name == "status_code") {
        "e.status_code()"
    } else {
        "0"
    };
    let retry = match duration_methods(error)
        .into_iter()
        .find(|(m, _)| m.name == "retry_after")
    {
        Some((_, true)) => millis_expr("e.retry_after()", true, "u64"),
        Some((_, false)) => format!("Some({})", millis_expr("e.retry_after()", false, "u64")),
        None => "None".to_string(),
    };

    format!(
        "/// Convert a `{rust_path}` into the NIF error with its kind, status and retry delay.\n\
         fn {fn_name}(e: {rust_path}) -> AlefNifError {{\n    \
             let kind = match &e {{\n{arms}\n    }};\n    \
             AlefNifError::Typed {{\n        \
                 module: \"Elixir.{exception_module}\",\n        \
                 kind,\n        \
                 message: e.to_string(),\n        \
                 status: {status},\n        \
                 retry_after_ms: {retry},\n    \
             }}\n\
         }}",
        arms = arms.join("\n"),
    )
}

/// Generate `AlefNifError` and `alef_nif_err`, the one conversion every NIF call site uses.
///
/// A core error that has a typed converter keeps its kind, status and retry delay and reaches
/// Elixir as an exception struct. Any other `Display` error stays a plain message string.
pub fn gen_rustler_error_support(errors: &[&ErrorDef], core_import: &str) -> String {
    let mut arms = String::new();
    for error in errors {
        arms.push_str(&format!(
            "    let boxed = match boxed.downcast::<{}>() {{\n        Ok(typed) => return {}(*typed),\n        Err(other) => other,\n    }};\n",
            core_error_path(error, core_import),
            rustler_converter_fn_name(error),
        ));
    }
    format!(
        "/// Error a NIF returns. `Typed` reaches Elixir as an exception struct with the error\n\
         /// kind, HTTP status and retry delay as fields; `Message` reaches Elixir as a string.\n\
         pub enum AlefNifError {{\n    \
             Typed {{\n        \
                 module: &'static str,\n        \
                 kind: &'static str,\n        \
                 message: String,\n        \
                 status: u16,\n        \
                 retry_after_ms: Option<u64>,\n    \
             }},\n    \
             Message(String),\n\
         }}\n\
         \n\
         impl From<String> for AlefNifError {{\n    \
             fn from(message: String) -> Self {{\n        \
                 AlefNifError::Message(message)\n    \
             }}\n\
         }}\n\
         \n\
         impl From<AlefNifError> for String {{\n    \
             fn from(error: AlefNifError) -> Self {{\n        \
                 match error {{\n            \
                     AlefNifError::Typed {{ message, .. }} | AlefNifError::Message(message) => message,\n        \
                 }}\n    \
             }}\n\
         }}\n\
         \n\
         impl AlefNifError {{\n    \
             /// Build the exception struct term. Fails only when the VM cannot make an atom.\n    \
             fn encode_typed<'a>(&self, env: rustler::Env<'a>) -> rustler::NifResult<rustler::Term<'a>> {{\n        \
                 let AlefNifError::Typed {{ module, kind, message, status, retry_after_ms }} = self else {{\n            \
                     return Err(rustler::Error::BadArg);\n        \
                 }};\n        \
                 let atom = |name: &str| rustler::types::atom::Atom::from_str(env, name);\n        \
                 let keys = [\n            \
                     atom(\"__struct__\")?.encode(env),\n            \
                     atom(\"__exception__\")?.encode(env),\n            \
                     atom(\"kind\")?.encode(env),\n            \
                     atom(\"message\")?.encode(env),\n            \
                     atom(\"status\")?.encode(env),\n            \
                     atom(\"retry_after_ms\")?.encode(env),\n        \
                 ];\n        \
                 let values = [\n            \
                     atom(module)?.encode(env),\n            \
                     true.encode(env),\n            \
                     atom(kind)?.encode(env),\n            \
                     message.encode(env),\n            \
                     status.encode(env),\n            \
                     retry_after_ms.encode(env),\n        \
                 ];\n        \
                 rustler::Term::map_from_arrays(env, &keys, &values)\n    \
             }}\n\
         }}\n\
         \n\
         impl rustler::Encoder for AlefNifError {{\n    \
             fn encode<'a>(&self, env: rustler::Env<'a>) -> rustler::Term<'a> {{\n        \
                 match self {{\n            \
                     AlefNifError::Message(message) => message.encode(env),\n            \
                     AlefNifError::Typed {{ message, .. }} => self\n                \
                         .encode_typed(env)\n                \
                         .unwrap_or_else(|_| message.encode(env)),\n        \
                 }}\n    \
             }}\n\
         }}\n\
         \n\
         /// Convert any error a NIF call returns into an `AlefNifError`.\n\
         fn alef_nif_err<E: std::fmt::Display + 'static>(e: E) -> AlefNifError {{\n    \
             let message = e.to_string();\n    \
             let boxed: Box<dyn std::any::Any> = Box::new(e);\n\
         {arms}    \
             drop(boxed);\n    \
             AlefNifError::Message(message)\n\
         }}"
    )
}
