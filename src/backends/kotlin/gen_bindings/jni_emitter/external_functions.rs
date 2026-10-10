fn non_unit_return_type(return_type: &TypeRef, rendered_return_type: &str) -> Option<String> {
    if matches!(return_type, TypeRef::Unit) {
        None
    } else {
        Some(rendered_return_type.to_string())
    }
}

fn push_jni_external_fun(
    out: &mut String,
    native_name: &str,
    params: &str,
    return_type: Option<String>,
    throws_class: Option<&str>,
) {
    out.push_str(&template_env::render(
        "jni_external_fun.jinja",
        crate::alef_context! {
            native_name => native_name,
            params => params,
            return_type => return_type,
            throws_class => throws_class,
        },
    ));
    out.push('\n');
}

/// Emit `external fun native{Owner}{Adapter}{Start,Next,Free}` declarations
/// for every streaming adapter with an owner type. Called from both
/// `emit_jni_bridge_object` (for the Bridge object body) and from tests.
/// `exception_class` is the simple name of the exception class emitted alongside
/// the Bridge object (e.g. `"DemoBridgeException"`).  Start and Next are annotated
/// with `@Throws` because they can propagate Rust errors; Free is infallible.
pub fn emit_streaming_jni_external_funs(out: &mut String, config: &ResolvedCrateConfig, exception_class: &str) {
    let streaming: Vec<_> = config
        .adapters
        .iter()
        .filter(|a| matches!(a.pattern, AdapterPattern::Streaming) && a.owner_type.is_some())
        .collect();
    if streaming.is_empty() {
        return;
    }
    out.push_str("\n    // JNI streaming external funs — implementations are Rust JNI shims.\n");
    for adapter in &streaming {
        let Some(owner) = adapter.owner_type.as_deref() else {
            continue;
        };
        let owner_pascal = to_pascal_case(owner);
        let adapter_pascal = to_pascal_case(&adapter.name);
        let jni_start = format!("native{owner_pascal}{adapter_pascal}Start");
        let jni_next = format!("native{owner_pascal}{adapter_pascal}Next");
        let jni_free = format!("native{owner_pascal}{adapter_pascal}Free");
        out.push('\n');
        out.push_str(&template_env::render(
            "jni_streaming_extern_comment.jinja",
            crate::alef_context! {
                owner => owner,
                adapter_name => to_lower_camel(&adapter.name),
            },
        ));
        push_jni_external_fun(
            out,
            &jni_start,
            "clientHandle: Long, requestJson: String",
            Some("Long".to_string()),
            Some(exception_class),
        );
        push_jni_external_fun(
            out,
            &jni_next,
            "streamHandle: Long",
            Some("String?".to_string()),
            Some(exception_class),
        );
        push_jni_external_fun(out, &jni_free, "streamHandle: Long", None, None);
    }
}

/// Emit `external fun native{Owner}{Method}(handle: Long, requestJson: String): <ReturnType>`
/// declarations for every visible, non-sanitized, non-static instance method on every
/// opaque client type in the API surface, plus a `external fun nativeFree{Owner}(handle: Long)`
/// destructor declaration for each client type.
///
/// Methods with no params beyond `&self` produce `(handle: Long)` with no `requestJson`.
/// `Vec<u8>` return types produce `ByteArray`; `Unit` stays `Unit`; opaque Named returns
/// produce `Long` (raw handle); everything else serialises via JSON → `String` (or `String?` for optionals).
/// `exception_class` is the simple name of the exception class so every method gets
/// an `@Throws` annotation that allows typed catch blocks to reach the error.
/// Emitted destructor names are tracked in `emitted_destructor_names` to prevent
/// duplication when handle-only types also appear in the API.
fn emit_method_jni_external_funs(
    out: &mut String,
    api: &ApiSurface,
    exclude_functions: &std::collections::HashSet<String>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    exception_class: &str,
    emitted_destructor_names: &mut std::collections::HashSet<String>,
) {
    let client_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait && t.methods.iter().any(|m| !m.sanitized && !m.is_static))
        .collect();
    if client_types.is_empty() {
        return;
    }

    let opaque_type_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    out.push_str("\n    // JNI external funs for client instance methods.\n");
    for ty in &client_types {
        emit_type_method_jni_external_funs(
            out,
            ty,
            exclude_functions,
            exception_class,
            &opaque_type_names,
            capsule_types,
        );
        let free_name = format!("nativeFree{}", to_pascal_case(&ty.name));
        push_jni_external_fun(out, &free_name, "handle: Long", None, None);
        emitted_destructor_names.insert(free_name);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_type_method_jni_external_funs(
    out: &mut String,
    type_def: &crate::core::ir::TypeDef,
    exclude_functions: &std::collections::HashSet<String>,
    exception_class: &str,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) {
    let owner_pascal = to_pascal_case(&type_def.name);
    for method in type_def
        .methods
        .iter()
        .filter(|method| !method.sanitized && !method.is_static)
    {
        if exclude_functions.contains(method.name.as_str()) {
            continue;
        }
        let native_name = format!("native{owner_pascal}{}", to_pascal_case(&method.name));
        let return_type = jni_return_type_for_method(&method.return_type, opaque_type_names, capsule_types);
        let params = method_jni_params(method);
        push_jni_external_fun(
            out,
            &native_name,
            &params,
            non_unit_return_type(&method.return_type, return_type),
            Some(exception_class),
        );
    }
}

fn method_jni_params(method: &crate::core::ir::MethodDef) -> String {
    if method.params.is_empty() {
        "handle: Long".to_string()
    } else if method.params.len() == 1 && is_binary_param_type(&method.params[0].ty) {
        format!("handle: Long, {}: ByteArray", to_lower_camel(&method.params[0].name))
    } else {
        "handle: Long, requestJson: String".to_string()
    }
}

/// Emit `external fun native{Owner}{Method}(selfJson: String[, requestJson: String])`
/// declarations for every bridgeable instance method on a *value* (data-class) type.
///
/// Value types carry no handle: the Kotlin object serialises `this` and the Rust
/// shim rebuilds the core receiver from that JSON. Selection goes through
/// [`bridgeable_value_methods`] so this list, the data-class bodies, and the
/// `Java_*` symbols the JNI shim crate exports cannot drift apart.
pub fn emit_value_method_jni_external_funs(out: &mut String, api: &ApiSurface, exception_class: &str) {
    let serde_type_names = value_bridge_serde_type_names(api);
    let value_types: Vec<_> = api
        .types
        .iter()
        .filter(|t| !t.is_opaque && !t.is_trait && !t.binding_excluded)
        .filter(|t| !bridgeable_value_methods(t, &serde_type_names).is_empty())
        .collect();
    if value_types.is_empty() {
        return;
    }

    out.push_str("\n    // JNI external funs for data-class instance methods.\n");
    for ty in &value_types {
        for method in bridgeable_value_methods(ty, &serde_type_names) {
            let native_name = bridge_method_name(&ty.name, &method.name);
            let return_type = value_method_return_type(&ty.name, method);
            let params = if method.params.is_empty() {
                "selfJson: String".to_string()
            } else {
                "selfJson: String, requestJson: String".to_string()
            };
            push_jni_external_fun(
                out,
                &native_name,
                &params,
                non_unit_return_type(&return_type, jni_return_type(&return_type)),
                Some(exception_class),
            );
        }
    }
}

/// Returns true when this function returns a configured host-native capsule type.
pub(in crate::backends::kotlin::gen_bindings::jni_emitter) fn is_capsule_function(
    func: &crate::core::ir::FunctionDef,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
) -> bool {
    let base_type = match &func.return_type {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    matches!(base_type, crate::core::ir::TypeRef::Named(name) if capsule_types.contains_key(name))
}
