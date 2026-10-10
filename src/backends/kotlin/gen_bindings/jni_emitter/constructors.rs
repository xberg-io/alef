/// Check if a type string refers to an enum in the API surface.
fn is_enum_param(ty: &str, enum_names: &std::collections::HashSet<&str>) -> bool {
    enum_names.contains(ty)
}

/// Extract the Named type from a TypeRef (e.g., "RouteBuilder" from TypeRef::Named("RouteBuilder")).
/// Used to identify enum parameter types.
fn extract_named_type(ty: &str) -> Option<&str> {
    if !ty.is_empty() && ty.chars().next().unwrap().is_uppercase() {
        Some(ty)
    } else {
        None
    }
}
/// Emit `external fun nativeNew<TypeName>(params...): Long` declarations in the
/// Bridge object for every entry in `config.client_constructors` that names an
/// opaque type in the API surface.
///
/// Each `*const c_char` param maps to `String`; enum (Named) params that refer to
/// enums in the API surface map to `Int`; other param types are mapped to `Long`.
/// The return type is always `Long` (raw Box pointer).
fn emit_constructor_jni_external_funs(
    out: &mut String,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exception_class: &str,
) {
    let opaque_names: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| t.is_opaque && !t.is_trait)
        .map(|t| t.name.as_str())
        .collect();

    let enum_names: std::collections::HashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    let mut sorted: Vec<(&str, &ClientConstructorConfig)> = config
        .client_constructors
        .iter()
        .filter(|(name, _)| opaque_names.contains(name.as_str()))
        .map(|(name, ctor)| (name.as_str(), ctor))
        .collect();
    sorted.sort_by_key(|(name, _)| *name);

    if sorted.is_empty() {
        return;
    }

    out.push_str("\n    // JNI constructor external funs — implementations are Rust JNI shims.\n");
    for (type_name, ctor) in sorted {
        let native_name = format!("nativeNew{}", to_pascal_case(type_name));
        let params: Vec<String> = ctor
            .params
            .iter()
            .map(|p| {
                let kt_ty = if p.ty.contains("c_char") {
                    "String".to_string()
                } else if is_enum_param(&p.ty, &enum_names) {
                    "Int".to_string()
                } else {
                    "Long".to_string()
                };
                let param_name = to_lower_camel(&p.name);
                format!("{param_name}: {kt_ty}")
            })
            .collect();
        let params_str = params.join(", ");
        push_jni_external_fun(
            out,
            &native_name,
            &params_str,
            Some("Long".to_string()),
            Some(exception_class),
        );
    }
}

/// Emit a `fun create(params...): TypeName` factory method inside the
/// companion object of the JNI client class.  Calls `Bridge.nativeNew<TypeName>(...)`
/// and wraps the returned `Long` handle in a new instance.
///
/// Enum-typed parameters are converted to their discriminant (`ordinal`) when passed
/// to the native function.
fn emit_jni_client_factory(
    class_name: &str,
    bridge_name: &str,
    ctor: &ClientConstructorConfig,
    api: &ApiSurface,
    out: &mut String,
) {
    let native_name = format!("nativeNew{}", to_pascal_case(class_name));

    let enum_names: std::collections::HashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    let params: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            let kt_ty = if p.ty.contains("c_char") {
                "String".to_string()
            } else if is_enum_param(&p.ty, &enum_names) {
                let enum_name = extract_named_type(&p.ty).unwrap_or("Any");
                enum_name.to_string()
            } else {
                "Long".to_string()
            };
            let param_name = to_lower_camel(&p.name);
            format!("{param_name}: {kt_ty}")
        })
        .collect();

    let call_args: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            let param_name = to_lower_camel(&p.name);
            if is_enum_param(&p.ty, &enum_names) {
                format!("{param_name}.ordinal")
            } else {
                param_name
            }
        })
        .collect();

    let params_str = params.join(", ");
    let call_args_str = call_args.join(", ");
    out.push_str(&template_env::render(
        "jni_client_constructor.jinja",
        crate::alef_context! {
            params => params_str,
            class_name => class_name,
            bridge_name => bridge_name,
            native_name => native_name,
            call_args => call_args_str,
        },
    ));
}
