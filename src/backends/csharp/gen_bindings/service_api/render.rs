use crate::backends::csharp::template_env::render;
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::ir::{ApiSurface, EntrypointDef, ParamDef, RegistrationDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};

fn entrypoint_return_representable(entrypoint: &EntrypointDef, api: &ApiSurface) -> bool {
    match &entrypoint.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(name) => is_opaque(api, name),
        _ => false,
    }
}

fn is_opaque(api: &ApiSurface, name: &str) -> bool {
    api.types.iter().any(|typ| typ.name == name && typ.is_opaque)
}

/// True when a `Named` service param crosses the C ABI as a bare scalar discriminant.
///
/// Reads `backends::ffi::type_map`, the module that decides the C signature, rather than asking
/// `api.enums` directly: enum-ness is not the deciding fact — being `Copy` is — so a non-`Copy`
/// enum is boxed as an `AlefHandle` and a `Copy` struct is not. ~keep
fn crosses_as_scalar(api: &ApiSurface, name: &str) -> bool {
    crate::backends::ffi::type_map::is_scalar_c_abi_named_type(api, name)
}

fn primitive_managed_type(primitive: &crate::core::ir::PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match primitive {
        PrimitiveType::Bool => "bool",
        PrimitiveType::U8 => "byte",
        PrimitiveType::U16 => "ushort",
        PrimitiveType::U32 => "uint",
        PrimitiveType::U64 => "ulong",
        PrimitiveType::I8 => "sbyte",
        PrimitiveType::I16 => "short",
        PrimitiveType::I32 => "int",
        PrimitiveType::I64 => "long",
        PrimitiveType::F32 => "float",
        PrimitiveType::F64 => "double",
        PrimitiveType::Usize => "nuint",
        PrimitiveType::Isize => "nint",
    }
}

fn managed_type(ty: &TypeRef, api: &ApiSurface, named_records: bool) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(primitive) => primitive_managed_type(primitive).to_owned(),
        TypeRef::Bytes => "byte[]".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(name) if named_records || is_opaque(api, name) => csharp_type_name(name),
        _ => "string".to_owned(),
    }
}

fn param_decl_list(params: &[ParamDef], api: &ApiSurface, named_records: bool) -> String {
    params
        .iter()
        .map(|param| {
            let ty = managed_type(&param.ty, api, named_records);
            format!("{ty} {}", param.name.to_lower_camel_case())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// P/Invoke parameter types for the service-API `extern`s.
///
/// A `Named` param crosses as the scalar [`HANDLE_PINVOKE_TYPE`] `AlefHandle` — the same fact
/// `marshalling::pinvoke_param_type` states for every other C# `[DllImport]`, read from the same
/// constant so the two cannot drift apart on handle width. This map is *not* a duplicate of that
/// one and must not be folded into it blindly: it still disagrees on `Usize`/`Isize`
/// (`nuint`/`nint` here, `ulong`/`long` there) and has no `Duration` arm at all, so those fall to
/// `IntPtr`. Reconciling those three positions is a live ABI question, not a refactor. ~keep
fn native_type(ty: &TypeRef, api: &ApiSurface) -> &'static str {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char => "string",
        TypeRef::Primitive(PrimitiveType::Bool) => "int",
        TypeRef::Primitive(PrimitiveType::U8) => "byte",
        TypeRef::Primitive(PrimitiveType::U16) => "ushort",
        TypeRef::Primitive(PrimitiveType::U32) => "uint",
        TypeRef::Primitive(PrimitiveType::U64) => "ulong",
        TypeRef::Primitive(PrimitiveType::I8) => "sbyte",
        TypeRef::Primitive(PrimitiveType::I16) => "short",
        TypeRef::Primitive(PrimitiveType::I32) => "int",
        TypeRef::Primitive(PrimitiveType::I64) => "long",
        TypeRef::Primitive(PrimitiveType::F32) => "float",
        TypeRef::Primitive(PrimitiveType::F64) => "double",
        TypeRef::Primitive(PrimitiveType::Usize) => "nuint",
        TypeRef::Primitive(PrimitiveType::Isize) => "nint",
        TypeRef::Named(name) if crosses_as_scalar(api, name) => "int",
        TypeRef::Named(_) => super::super::HANDLE_PINVOKE_TYPE,
        _ => "IntPtr",
    }
}

fn pinvoke_param_lines(params: &[ParamDef], api: &ApiSurface) -> String {
    params
        .iter()
        .map(|param| {
            let ty = native_type(&param.ty, api);
            format!(",\n        {ty} {}", param.name)
        })
        .collect()
}

#[derive(Default)]
struct CallMarshalling {
    declarations: String,
    setup: String,
    teardown: String,
    arg_lines: String,
}

fn call_marshalling(params: &[ParamDef], api: &ApiSurface, indent: &str) -> CallMarshalling {
    let mut result = CallMarshalling::default();
    for param in params {
        let name = param.name.to_lower_camel_case();
        match &param.ty {
            TypeRef::Named(type_name) if crosses_as_scalar(api, type_name) => {
                result.arg_lines.push_str(&format!(",\n{indent}(int){name}"));
            }
            TypeRef::Named(type_name) if is_opaque(api, type_name) => {
                result.arg_lines.push_str(&format!(",\n{indent}{name}.Handle"));
            }
            TypeRef::Named(type_name) => push_named_marshalling(&mut result, param, &name, type_name, indent),
            TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool) => {
                result.arg_lines.push_str(&format!(",\n{indent}({name} ? 1 : 0)"));
            }
            _ => result.arg_lines.push_str(&format!(",\n{indent}{name}")),
        }
    }
    result
}

/// Marshals a `Named` record param through `{Type}FromJson` into the scalar `AlefHandle`.
///
/// The local is `ulong` because `{Type}FromJson` is declared `ulong` (`extern_ptr_from_json.jinja`)
/// and `native_type` declares every non-enum `Named` service param `ulong`. Declaring the local
/// `IntPtr` — as this did before — made the assignment, the `IntPtr.Zero` guard and the
/// `.ToInt64()` narrowing all disagree with the signature they feed, which is the CS0034
/// `ulong`/`nint` break. ~keep
fn push_named_marshalling(result: &mut CallMarshalling, param: &ParamDef, name: &str, type_name: &str, indent: &str) {
    let handle = format!("{name}Handle");
    let json = format!("{name}Json");
    let from_json = format!("{}FromJson", csharp_type_name(type_name));
    let free = format!("{}Free", csharp_type_name(type_name));
    result.declarations.push_str(&format!("        ulong {handle} = 0;\n"));
    let setup_indent = begin_named_setup(result, param, name);
    result.setup.push_str(&format!(
        "{setup_indent}var {json} = FfiJsonExtensions.ToFfiJson({name});\n"
    ));
    result.setup.push_str(&format!(
        "{setup_indent}{handle} = NativeMethods.{from_json}({json});\n"
    ));
    result.setup.push_str(&format!("{setup_indent}if ({handle} == 0) {{\n"));
    result
        .setup
        .push_str(&format!("{setup_indent}    throw ResolveLastError();\n"));
    result.setup.push_str(&format!("{setup_indent}}}\n"));
    if param.optional {
        result.setup.push_str("            }\n");
    }
    result.teardown.push_str(&format!(
        "            if ({handle} != 0) NativeMethods.{free}({handle});\n"
    ));
    result.arg_lines.push_str(&format!(",\n{indent}{handle}"));
}

fn begin_named_setup<'a>(result: &mut CallMarshalling, param: &ParamDef, name: &str) -> &'a str {
    if param.optional {
        result.setup.push_str(&format!("            if ({name} != null) {{\n"));
        "                "
    } else {
        result
            .setup
            .push_str(&format!("            ArgumentNullException.ThrowIfNull({name});\n"));
        "            "
    }
}

fn service_needs_error_resolver(service: &ServiceDef, api: &ApiSurface) -> bool {
    let has_named_record = |params: &[ParamDef]| {
        params.iter().any(
            |param| matches!(&param.ty, TypeRef::Named(name) if !crosses_as_scalar(api, name) && !is_opaque(api, name)),
        )
    };
    !service.configurators.is_empty()
        || service
            .configurators
            .iter()
            .any(|method| has_named_record(&method.params))
        || service.registrations.iter().any(|registration| {
            has_named_record(&registration.metadata_params)
                || registration
                    .variants
                    .iter()
                    .any(|variant| has_named_record(&variant.signature_params))
        })
        || service
            .entrypoints
            .iter()
            .any(|entrypoint| has_named_record(&entrypoint.params))
}

pub(super) fn gen_service_cs(api: &ApiSurface, service: &ServiceDef, namespace: &str, prefix: &str) -> String {
    let mut out = render_service_header(api, service, namespace, prefix);
    render_constructor(&mut out, api, service, prefix);
    render_configurators(&mut out, api, service, prefix);
    render_registrations(&mut out, api, service, prefix);
    render_entrypoints(&mut out, api, service, prefix);
    out.push_str(&render("service_dispose_method.jinja", minijinja::context! {}));
    out.push_str(&render("service_handler_trampoline.jinja", minijinja::context! {}));
    out.push_str("}\n\n}\n");
    out
}

fn render_service_header(api: &ApiSurface, service: &ServiceDef, namespace: &str, prefix: &str) -> String {
    render(
        "service_class_header.jinja",
        minijinja::context! {
            namespace,
            service_name => &service.name,
            class_name => to_csharp_name(&service.name),
            native_free => format!("{}_{}_free", prefix.to_lowercase(), service.name.to_snake_case()),
            exception_name => format!("{}Exception", to_csharp_name(&api.crate_name)),
            needs_error_resolver => service_needs_error_resolver(service, api),
        },
    )
}

fn render_constructor(out: &mut String, api: &ApiSurface, service: &ServiceDef, prefix: &str) {
    out.push_str(&render(
        "service_constructor.jinja",
        minijinja::context! {
            service_name => &service.name,
            class_name => to_csharp_name(&service.name),
            params_decl => param_decl_list(&service.constructor.params, api, false),
            native_new => format!("{}_{}_new", prefix.to_lowercase(), service.name.to_snake_case()),
        },
    ));
}

fn render_configurators(out: &mut String, api: &ApiSurface, service: &ServiceDef, prefix: &str) {
    let service_snake = service.name.to_snake_case();
    for method in &service.configurators {
        let marshalling = call_marshalling(&method.params, api, "                ");
        out.push_str(&render(
            "service_configurator_method.jinja",
            minijinja::context! {
                class_name => to_csharp_name(&service.name),
                method_name => &method.name,
                params_decl => param_decl_list(&method.params, api, true),
                native_method => format!("{}_{}_{}", prefix.to_lowercase(), service_snake, method.name.to_snake_case()),
                declarations => marshalling.declarations,
                setup => marshalling.setup,
                teardown => marshalling.teardown,
                arg_lines => marshalling.arg_lines,
            },
        ));
    }
}

fn render_registrations(out: &mut String, api: &ApiSurface, service: &ServiceDef, prefix: &str) {
    let service_snake = service.name.to_snake_case();
    for registration in &service.registrations {
        render_registration(out, api, registration, prefix, &service_snake);
        for variant in &registration.variants {
            render_registration_variant(out, api, variant, prefix, &service_snake);
        }
    }
}

fn render_registration(
    out: &mut String,
    api: &ApiSurface,
    registration: &RegistrationDef,
    prefix: &str,
    service_snake: &str,
) {
    let marshalling = call_marshalling(&registration.metadata_params, api, "                ");
    out.push_str(&render(
        "service_registration_method.jinja",
        minijinja::context! {
            method_name => &registration.method,
            metadata_params => param_decl_list(&registration.metadata_params, api, true),
            native_method => format!("{}_{}_register_{}", prefix.to_lowercase(), service_snake, registration.method.to_snake_case()),
            declarations => marshalling.declarations,
            setup => marshalling.setup,
            teardown => marshalling.teardown,
            arg_lines => marshalling.arg_lines,
        },
    ));
}

fn render_registration_variant(
    out: &mut String,
    api: &ApiSurface,
    variant: &crate::core::ir::RegistrationVariant,
    prefix: &str,
    service_snake: &str,
) {
    let marshalling = call_marshalling(&variant.signature_params, api, "                ");
    let doc = variant
        .doc
        .clone()
        .unwrap_or_else(|| format!("Register a handler via the {} variant.", variant.name));
    out.push_str(&render(
        "service_variant_registration_method.jinja",
        minijinja::context! {
            method_name => variant.name.to_upper_camel_case(),
            doc,
            signature_params => param_decl_list(&variant.signature_params, api, true),
            native_method => format!("{}_{}_{}", prefix.to_lowercase(), service_snake, variant.name.to_snake_case()),
            declarations => marshalling.declarations,
            setup => marshalling.setup,
            teardown => marshalling.teardown,
            arg_lines => marshalling.arg_lines,
        },
    ));
}

fn render_entrypoints(out: &mut String, api: &ApiSurface, service: &ServiceDef, prefix: &str) {
    let service_snake = service.name.to_snake_case();
    for entrypoint in &service.entrypoints {
        if !entrypoint_return_representable(entrypoint, api) {
            continue;
        }
        let marshalling = call_marshalling(&entrypoint.params, api, "                ");
        let opaque = matches!(&entrypoint.return_type, TypeRef::Named(name) if is_opaque(api, name));
        out.push_str(&render(
            "service_entrypoint_method.jinja",
            minijinja::context! {
                method_name => &entrypoint.method,
                return_type => if opaque { "ulong" } else { "int" },
                params_decl => param_decl_list(&entrypoint.params, api, true),
                native_method => format!("{}_{}_ep_{}", prefix.to_lowercase(), service_snake, entrypoint.method.to_snake_case()),
                declarations => marshalling.declarations,
                setup => marshalling.setup,
                teardown => marshalling.teardown,
                arg_lines => marshalling.arg_lines,
            },
        ));
    }
}

pub(super) fn gen_native_methods_cs(api: &ApiSurface, namespace: &str, prefix: &str) -> String {
    let mut out = render("service_native_methods_header.jinja", minijinja::context! { namespace });
    for service in &api.services {
        render_native_service(&mut out, api, service, prefix);
    }
    out.push_str("}\n\n}\n");
    out
}

fn render_native_service(out: &mut String, api: &ApiSurface, service: &ServiceDef, prefix: &str) {
    let service_snake = service.name.to_snake_case();
    out.push_str(&render(
        "service_native_ctor_free.jinja",
        minijinja::context! {
            dll_name => format!("{}_ffi", prefix.to_lowercase()),
            new_method => format!("{}_{}_new", prefix.to_lowercase(), service_snake),
            free_method => format!("{}_{}_free", prefix.to_lowercase(), service_snake),
        },
    ));
    render_native_methods(out, api, &service.configurators, prefix, &service_snake);
    render_native_registrations(out, api, service, prefix, &service_snake);
    render_native_entrypoints(out, api, service, prefix, &service_snake);
}

fn render_native_methods(
    out: &mut String,
    api: &ApiSurface,
    methods: &[crate::core::ir::MethodDef],
    prefix: &str,
    service_snake: &str,
) {
    for method in methods {
        render_native_declaration(
            out,
            api,
            prefix,
            service_snake,
            NativeDeclarationSpec {
                suffix: &method.name.to_snake_case(),
                return_type: "ulong",
                base_params: "        ulong owner",
                params: &method.params,
            },
        );
    }
}

fn render_native_registrations(
    out: &mut String,
    api: &ApiSurface,
    service: &ServiceDef,
    prefix: &str,
    service_snake: &str,
) {
    const BASE: &str = "        ulong owner,\n        HandlerCallback callback,\n        \
                        HandlerResponseFree responseFree,\n        IntPtr ctx";
    for registration in &service.registrations {
        let suffix = format!("register_{}", registration.method.to_snake_case());
        render_native_declaration(
            out,
            api,
            prefix,
            service_snake,
            NativeDeclarationSpec {
                suffix: &suffix,
                return_type: "int",
                base_params: BASE,
                params: &registration.metadata_params,
            },
        );
        for variant in &registration.variants {
            render_native_declaration(
                out,
                api,
                prefix,
                service_snake,
                NativeDeclarationSpec {
                    suffix: &variant.name.to_snake_case(),
                    return_type: "int",
                    base_params: BASE,
                    params: &variant.signature_params,
                },
            );
        }
    }
}

fn render_native_entrypoints(
    out: &mut String,
    api: &ApiSurface,
    service: &ServiceDef,
    prefix: &str,
    service_snake: &str,
) {
    for entrypoint in &service.entrypoints {
        if !entrypoint_return_representable(entrypoint, api) {
            continue;
        }
        let opaque = matches!(&entrypoint.return_type, TypeRef::Named(name) if is_opaque(api, name));
        render_native_declaration(
            out,
            api,
            prefix,
            service_snake,
            NativeDeclarationSpec {
                suffix: &format!("ep_{}", entrypoint.method.to_snake_case()),
                return_type: if opaque { "ulong" } else { "int" },
                base_params: "        ulong owner",
                params: &entrypoint.params,
            },
        );
    }
}

/// Bundles the per-declaration inputs to [`render_native_declaration`] that vary by call
/// site, keeping the accumulator/context params (`out`, `api`, `prefix`, `service_snake`)
/// separate since every caller already threads those through unchanged.
struct NativeDeclarationSpec<'a> {
    suffix: &'a str,
    return_type: &'a str,
    base_params: &'a str,
    params: &'a [ParamDef],
}

fn render_native_declaration(
    out: &mut String,
    api: &ApiSurface,
    prefix: &str,
    service_snake: &str,
    spec: NativeDeclarationSpec,
) {
    out.push_str(&render(
        "service_pinvoke_declaration.jinja",
        minijinja::context! {
            dll_name => format!("{}_ffi", prefix.to_lowercase()),
            return_type => spec.return_type,
            method_name => format!("{}_{}_{}", prefix.to_lowercase(), service_snake, spec.suffix),
            base_params => spec.base_params,
            param_lines => pinvoke_param_lines(spec.params, api),
        },
    ));
}
