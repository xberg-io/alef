use crate::backends::java::type_map::java_ffi_type;
use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};
use ahash::AHashSet;
use heck::ToSnakeCase;

/// Returns true when the function's Rust return type is `Result<Vec<u8>>` (or
/// `Result<Option<Vec<u8>>>`). The FFI layer emits these as the out-param
/// convention: `(inputs..., out_ptr: *mut *mut u8, out_len: *mut usize,
/// out_cap: *mut usize) -> i32`.
pub(crate) fn is_bytes_result(func: &FunctionDef) -> bool {
    func.return_type.returns_bytes_out_params()
}

/// Check if the return type is a string-like type that requires pointer-based
/// FFI return handling (allocate + free pattern). `Optional<String>` and
/// `Optional<Path>` reduce to a nullable pointer with the same handling — the
/// boxed Java type is also `String`/`Path`, so the wrapper signature is
/// unchanged from the non-optional case.
pub(crate) fn is_ffi_string_return(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
        ),
        _ => false,
    }
}

/// Returns the Java cast expression to apply to `MethodHandle.invoke()` return value.
///
/// The cast must match the widened ValueLayout from `java_ffi_type()`. When the
/// FunctionDescriptor uses a wider layout than the logical Java type (e.g. JAVA_LONG
/// for i32, JAVA_LONG for i16), `invoke()` boxes the wider type. We emit a chain of
/// casts to unbox then narrow back to the target type.
///
/// Examples:
/// - JAVA_LONG return (i32 → long): `(int)(long)` unboxes Long then narrows to int
/// - JAVA_LONG return (i16 → long): `(short)(long)` unboxes Long then narrows to short
/// - JAVA_LONG return (bool → long): `(int)(long)` unboxes Long then narrows to int
///
/// The bool arm previously emitted a bare `(long)`, on the reasoning that the `!= 0`
/// comparison in [`java_ffi_return_expr`] "handles narrowing". It does not: `!= 0` on a
/// `long` compares all 64 bits. The native function returns `int32_t` while the descriptor
/// declares JAVA_LONG, so the upper half is whatever happened to be in the register — and
/// any non-zero garbage there turns a native `false` into a Java `true`. A boolean-returning
/// FFI call could therefore never report false. Narrowing to `int` first discards the
/// undefined half, matching the i32 arm directly above.
///
/// The fix belongs here, at the call site, and NOT in the layout: `java_ffi_type` maps Bool
/// to JAVA_LONG deliberately and is pinned by a test, because narrowing the *descriptor* to
/// JAVA_INT reintroduces a JBR-on-Windows `ClassCastException`
/// (`ValueLayouts$OfIntImpl` cannot be cast to `ValueLayout$OfLong`). Widened layout,
/// narrowed call site. ~keep
pub(crate) fn java_ffi_return_cast(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "(int)(long)",
            PrimitiveType::U8 | PrimitiveType::I8 => "(byte)(long)",
            PrimitiveType::U16 | PrimitiveType::I16 => "(short)(long)",
            PrimitiveType::U32 | PrimitiveType::I32 => "(int)(long)",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "(long)",
            PrimitiveType::F32 => "(float)",
            PrimitiveType::F64 => "(double)",
        },
        TypeRef::Duration => "(long)",
        _ => "(MemorySegment)",
    }
}

pub(crate) fn java_ffi_return_expr(ty: &TypeRef, var_name: &str) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{var_name} != 0"),
        _ => var_name.to_string(),
    }
}

pub(crate) fn gen_ffi_layout_with_enums(ty: &TypeRef, enum_names: &AHashSet<String>) -> String {
    match ty {
        TypeRef::Primitive(prim) => java_ffi_type(prim).to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Bytes => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Optional(inner) => gen_ffi_layout_with_enums(inner, enum_names),
        TypeRef::Vec(_) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Map(_, _) => "ValueLayout.ADDRESS".to_string(),
        TypeRef::Named(name) => {
            if enum_names.contains(name.as_str()) {
                "ValueLayout.JAVA_LONG".to_string()
            } else {
                "ValueLayout.ADDRESS".to_string()
            }
        }
        TypeRef::Unit => "".to_string(),
        TypeRef::Duration => "ValueLayout.JAVA_LONG".to_string(),
    }
}

/// Build the Jackson `writer` expression that preserves generic element-type
/// info for a `Vec<T>` / `Map<K, V>` Java parameter. Without this, `MAPPER.
/// writeValueAsString(list)` erases T at runtime, dropping `@JsonTypeInfo`
/// discriminators on polymorphic element types like the tagged-enum DTO
/// `PageAction`. The returned expression is a `ObjectWriter`.
fn build_collection_writer_for(inner: &TypeRef, outer: &TypeRef, _opaque_types: &AHashSet<String>) -> String {
    let elem_class = java_class_literal_for(inner);
    match outer {
        TypeRef::Vec(_) => format!(
            "MAPPER.writerFor(MAPPER.getTypeFactory().constructCollectionType(java.util.List.class, {elem_class}))"
        ),
        TypeRef::Map(k, _) => {
            let key_class = java_class_literal_for(k);
            format!(
                "MAPPER.writerFor(MAPPER.getTypeFactory().constructMapType(java.util.Map.class, {key_class}, {elem_class}))"
            )
        }
        _ => "MAPPER.writer()".to_string(),
    }
}

/// Render the Java `Class<?>` literal (e.g., `String.class`, `PageAction.class`,
/// `Integer.class`) for a Rust IR type. Used to construct typed Jackson
/// CollectionType / MapType so polymorphic element types serialize correctly.
fn java_class_literal_for(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String.class".to_string(),
        TypeRef::Bytes => "byte[].class".to_string(),
        TypeRef::Path => "java.nio.file.Path.class".to_string(),
        TypeRef::Json => "Object.class".to_string(),
        TypeRef::Unit => "Void.class".to_string(),
        TypeRef::Duration => "Long.class".to_string(),
        TypeRef::Primitive(prim) => match prim {
            PrimitiveType::Bool => "Boolean.class".to_string(),
            PrimitiveType::U8 | PrimitiveType::I8 => "Byte.class".to_string(),
            PrimitiveType::U16 | PrimitiveType::I16 => "Short.class".to_string(),
            PrimitiveType::U32 | PrimitiveType::I32 => "Integer.class".to_string(),
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                "Long.class".to_string()
            }
            PrimitiveType::F32 => "Float.class".to_string(),
            PrimitiveType::F64 => "Double.class".to_string(),
        },
        TypeRef::Named(name) => format!("{name}.class"),
        TypeRef::Optional(inner) => java_class_literal_for(inner),
        TypeRef::Vec(_) => "java.util.List.class".to_string(),
        TypeRef::Map(_, _) => "java.util.Map.class".to_string(),
    }
}

fn render_marshaled_value(out: &mut String, template: &str, name: &str) {
    let cname = format!("c{name}");
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! { cname, name },
    ));
}

fn marshal_named(
    out: &mut String,
    name: &str,
    type_name: &str,
    opaque: bool,
    optional: bool,
    prefix: &str,
    resources: &str,
) {
    if opaque {
        let template = if optional {
            "marshal_optional_opaque_handle.jinja"
        } else {
            "marshal_opaque_handle.jinja"
        };
        render_marshaled_value(out, template, name);
        return;
    }
    let type_upper = type_name.to_snake_case().to_uppercase();
    let prefix_upper = prefix.to_uppercase();
    let template = if optional {
        "marshal_optional_named_type.jinja"
    } else {
        "marshal_named_type.jinja"
    };
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! {
            cname => format!("c{name}"), name,
            from_json_handle => format!("NativeLib.{prefix_upper}_{type_upper}_FROM_JSON"),
            free_handle => format!("NativeLib.{prefix_upper}_{type_upper}_FREE"), resources,
        },
    ));
}

fn optional_primitive_carrier(primitive: &PrimitiveType) -> (&'static str, &'static str) {
    match primitive {
        PrimitiveType::U64 | PrimitiveType::Usize => ("long", "-1L"),
        PrimitiveType::I64 | PrimitiveType::Isize => ("long", "Long.MAX_VALUE"),
        PrimitiveType::U32 => ("int", "-1"),
        PrimitiveType::I32 => ("int", "Integer.MAX_VALUE"),
        PrimitiveType::U16 => ("short", "(short) -1"),
        PrimitiveType::I16 => ("short", "Short.MAX_VALUE"),
        PrimitiveType::U8 => ("byte", "(byte) -1"),
        PrimitiveType::I8 => ("byte", "Byte.MAX_VALUE"),
        PrimitiveType::F32 => ("float", "Float.NaN"),
        PrimitiveType::F64 => ("double", "Double.NaN"),
        PrimitiveType::Bool => ("int", "0"),
    }
}

fn marshal_optional_primitive(out: &mut String, name: &str, primitive: &PrimitiveType) {
    let (prim_kw, none_lit) = optional_primitive_carrier(primitive);
    let value_expr = if matches!(primitive, PrimitiveType::Bool) {
        format!("({name} ? 1 : 0)")
    } else {
        name.to_string()
    };
    out.push_str(&crate::backends::java::template_env::render(
        "marshal_optional_primitive.jinja",
        minijinja::context! { cname => format!("c{name}"), name, prim_kw, none_lit, value_expr },
    ));
}

fn marshal_optional(
    out: &mut String,
    name: &str,
    inner: &TypeRef,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    resources: &str,
) {
    match inner {
        TypeRef::String | TypeRef::Char => render_marshaled_value(out, "marshal_optional_string.jinja", name),
        TypeRef::Path => render_marshaled_value(out, "marshal_optional_path.jinja", name),
        TypeRef::Bytes => render_marshaled_value(out, "marshal_optional_bytes.jinja", name),
        TypeRef::Named(type_name) => marshal_named(
            out,
            name,
            type_name,
            opaque_types.contains(type_name),
            true,
            prefix,
            resources,
        ),
        TypeRef::Primitive(primitive) => marshal_optional_primitive(out, name, primitive),
        _ => {}
    }
}

pub(crate) fn marshal_param_to_ffi(
    out: &mut String,
    name: &str,
    ty: &TypeRef,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    resources: &str,
) {
    match ty {
        TypeRef::String | TypeRef::Char => render_marshaled_value(out, "marshal_string.jinja", name),
        TypeRef::Path => render_marshaled_value(out, "marshal_path.jinja", name),
        TypeRef::Bytes => render_marshaled_value(out, "marshal_bytes.jinja", name),
        TypeRef::Named(type_name) => marshal_named(
            out,
            name,
            type_name,
            opaque_types.contains(type_name),
            false,
            prefix,
            resources,
        ),
        TypeRef::Optional(inner) => marshal_optional(out, name, inner, opaque_types, prefix, resources),
        TypeRef::Vec(inner) | TypeRef::Map(_, inner) => {
            let java_writer = build_collection_writer_for(inner, ty, opaque_types);
            out.push_str(&crate::backends::java::template_env::render(
                "marshal_vec_map.jinja",
                minijinja::context! { cname => format!("c{name}"), name, java_writer },
            ));
        }
        _ => {}
    }
}

pub(crate) fn opaque_lease_resource(name: &str, ty: &TypeRef, opaque_types: &AHashSet<String>) -> Option<String> {
    let (type_name, optional) = match ty {
        TypeRef::Named(type_name) => (type_name, false),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(type_name) => (type_name, true),
            _ => return None,
        },
        _ => return None,
    };
    if !opaque_types.contains(type_name) {
        return None;
    }
    let cname = format!("c{name}");
    let expression = if optional {
        format!("{name} != null ? {name}.borrowHandle() : null")
    } else {
        format!("{name}.borrowHandle()")
    };
    Some(format!("             var {cname}Lease = {expression}"))
}

/// Generate the FFI argument(s) for a parameter.
///
/// Most parameters map to a single FFI argument. However, some expand to multiple:
/// - `Bytes` expands to (pointer, length)
/// - Bytes input parameters in bytes-result functions similarly expand
///
/// Returns a vector of argument expressions to be passed to MethodHandle.invoke().
pub(crate) fn ffi_param_args(name: &str, ty: &TypeRef, _opaque_types: &AHashSet<String>) -> Vec<String> {
    match ty {
        TypeRef::Bytes => {
            let cname = "c".to_string() + name;
            vec![cname.clone(), format!("{}Len", cname)]
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
            let cname = "c".to_string() + name;
            vec![cname.clone(), format!("{}Len", cname)]
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => vec!["c".to_string() + name],
        TypeRef::Json => {
            vec![name.to_string()]
        }
        TypeRef::Named(_) => vec!["c".to_string() + name],
        TypeRef::Vec(_) | TypeRef::Map(_, _) => vec!["c".to_string() + name],
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Named(_) => {
                vec!["c".to_string() + name]
            }
            TypeRef::Json => {
                vec![name.to_string()]
            }
            TypeRef::Primitive(_) => vec!["c".to_string() + name],
            _ => vec![name.to_string()],
        },
        TypeRef::Primitive(PrimitiveType::Bool) => vec![format!("({name} ? 1 : 0)")],
        _ => vec![name.to_string()],
    }
}

/// Append the FFI layout of every parameter to `layouts`.
///
/// A `Bytes` parameter (bare or `Optional`) occupies an address/length pair; everything else takes
/// one layout. This is the single construction of that rule: the byte-buffer descriptor path, the
/// ordinary free-function path, and the opaque-method path all build from it, so a downcall
/// descriptor can never disagree with a sibling about a parameter's ABI width. Callers pass a Vec
/// rather than receiving one because the method path pre-seeds the receiver's `ADDRESS`. ~keep
pub(crate) fn push_param_layouts(params: &[ParamDef], enum_names: &AHashSet<String>, layouts: &mut Vec<String>) {
    for param in params {
        match &param.ty {
            TypeRef::Bytes => {
                layouts.push("ValueLayout.ADDRESS".to_string());
                layouts.push("ValueLayout.JAVA_LONG".to_string());
            }
            TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => {
                layouts.push("ValueLayout.ADDRESS".to_string());
                layouts.push("ValueLayout.JAVA_LONG".to_string());
            }
            other => layouts.push(gen_ffi_layout_with_enums(other, enum_names)),
        }
    }
}

pub(crate) fn gen_function_descriptor(return_layout: &str, param_layouts: &[String]) -> String {
    if return_layout.is_empty() {
        if param_layouts.is_empty() {
            "FunctionDescriptor.ofVoid()".to_string()
        } else {
            format!("FunctionDescriptor.ofVoid({})", param_layouts.join(", "))
        }
    } else {
        if param_layouts.is_empty() {
            format!("FunctionDescriptor.of({})", return_layout)
        } else {
            format!("FunctionDescriptor.of({}, {})", return_layout, param_layouts.join(", "))
        }
    }
}

struct HelperNeeds {
    check_last_error: bool,
    read_cstring: bool,
    read_bytes: bool,
    read_json_list: bool,
    object_mapper: bool,
    native_resources: bool,
}

impl HelperNeeds {
    fn from_output(output: &str) -> Self {
        let read_json_list = output.contains("readJsonList(");
        Self {
            check_last_error: output.contains("checkLastError()"),
            read_cstring: output.contains("readCString("),
            read_bytes: output.contains("readBytes("),
            read_json_list,
            object_mapper: output.contains("MAPPER.") || read_json_list,
            native_resources: output.contains("new NativeResources()"),
        }
    }

    fn is_empty(&self) -> bool {
        !self.check_last_error
            && !self.read_cstring
            && !self.read_bytes
            && !self.read_json_list
            && !self.object_mapper
            && !self.native_resources
    }
}

/// Single source of truth for Java's fixed FFI infrastructure exception classes.
///
/// Every entry here becomes BOTH a generated `<name>.java` class (`JavaBackend::generate_bindings`
/// in `gen_bindings/mod.rs`, which also derives its `infrastructure_exception_names` skip-set from
/// this array) AND a `case <code> -> throw new <name>(msg);` arm in `checkLastError()`'s switch
/// (`emit_error_helper` below). Driving both from one array makes it structurally impossible to
/// add, rename, or drop an infrastructure exception in only one of the two places — the failure
/// mode that once left `InvalidInputException.java` on disk with no reachable dispatch arm after
/// the class was renamed to `ConversionErrorException` without updating both sites in lockstep. ~keep
pub(crate) const INFRASTRUCTURE_ERROR_CLASSES: [(&str, u32, &str); 3] = [
    (
        "ConversionErrorException",
        crate::core::ir::ApiSurface::FFI_ERROR_CODE_CONVERSION,
        "Exception thrown when an FFI value conversion fails.",
    ),
    (
        "CoreErrorException",
        crate::core::ir::ApiSurface::FFI_ERROR_CODE_UNKNOWN,
        "Exception thrown when the Rust core reports an unknown error.",
    ),
    (
        "PanicException",
        crate::core::ir::ApiSurface::FFI_ERROR_CODE_PANIC,
        "Exception thrown when a Rust panic is contained at the FFI boundary.",
    ),
];

fn emit_error_helper(out: &mut String, prefix: &str, class_name: &str, api: &crate::core::ir::ApiSurface) {
    let mut error_codes: Vec<(u32, String)> = INFRASTRUCTURE_ERROR_CLASSES
        .iter()
        .map(|(name, code, _doc)| (*code, (*name).to_string()))
        .collect();
    error_codes.extend(
        api.error_taxonomy()
            .iter()
            .map(|entry| (entry.code, format!("{}Exception", entry.variant))),
    );
    out.push_str(&crate::backends::java::template_env::render(
        "helper_check_last_error.jinja",
        minijinja::context! {
            prefix_upper => prefix.to_uppercase(),
            class_name => class_name,
            error_codes => error_codes,
        },
    ));
}

fn emit_simple_helper(out: &mut String, template: &str) {
    out.push_str(&crate::backends::java::template_env::render(
        template,
        minijinja::context! {},
    ));
}

fn emit_read_json_list_helper(out: &mut String, prefix: &str, class_name: &str) {
    let free_handle = format!("NativeLib.{}_FREE_STRING", prefix.to_uppercase());
    out.push_str(&crate::backends::java::template_env::render(
        "helper_read_json_list.jinja",
        minijinja::context! {
            class_name => class_name,
            free_handle => free_handle,
        },
    ));
}

pub(crate) fn gen_helper_methods(out: &mut String, prefix: &str, class_name: &str, api: &crate::core::ir::ApiSurface) {
    let needs = HelperNeeds::from_output(out);
    if needs.is_empty() {
        return;
    }

    out.push_str(&crate::backends::java::template_env::render(
        "gen_helper_methods_header.jinja",
        minijinja::context! {},
    ));
    out.push('\n');

    if needs.native_resources {
        emit_simple_helper(out, "helper_native_resources.jinja");
    }

    if needs.check_last_error {
        emit_error_helper(out, prefix, class_name, api);
    }

    if needs.object_mapper {
        emit_simple_helper(out, "helper_object_mapper.jinja");
    }

    if needs.read_cstring {
        emit_simple_helper(out, "helper_read_cstring.jinja");
    }

    if needs.read_bytes {
        emit_simple_helper(out, "helper_read_bytes.jinja");
    }

    if needs.read_json_list {
        emit_read_json_list_helper(out, prefix, class_name);
    }
}

#[cfg(test)]
mod typed_error_tests {
    use super::*;

    #[test]
    fn helper_maps_taxonomy_code_to_variant_exception() {
        let error = crate::core::ir::ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample::RequestError".to_string(),
            variants: vec![crate::core::ir::ErrorVariant {
                error_code: Some(100),
                name: "InvalidInput".to_string(),
                is_unit: true,
                ..Default::default()
            }],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let api = crate::core::ir::ApiSurface {
            errors: vec![error],
            ..Default::default()
        };
        let code = api.error_taxonomy()[0].code;
        let mut output = "checkLastError()".to_string();

        gen_helper_methods(&mut output, "sample", "Sample", &api);

        assert!(output.contains(&format!("case {code} -> throw new InvalidInputException(msg);")));
        assert!(output.contains("case 1 -> throw new ConversionErrorException(msg);"));
        assert!(output.contains("case 2 -> throw new CoreErrorException(msg);"));
        assert!(output.contains("case 3 -> throw new PanicException(msg);"));
        let null_guard = output
            .find("ctxPtr.equals(MemorySegment.NULL)")
            .expect("null context guard");
        let reinterpret = output.find("ctxPtr.reinterpret").expect("context read");
        assert!(null_guard < reinterpret);
    }

    /// Regression for a collision where a Java infrastructure exception was renamed in the
    /// switch-generating list but not in the file-generating list (or vice versa), leaving two
    /// classes claiming the same numeric code. `INFRASTRUCTURE_ERROR_CLASSES` is the single
    /// array both lists now read from, so this pins its codes and class names as pairwise unique. ~keep
    #[test]
    fn infrastructure_error_classes_have_unique_codes_and_names() {
        let mut codes: Vec<u32> = INFRASTRUCTURE_ERROR_CLASSES.iter().map(|(_, code, _)| *code).collect();
        let mut names: Vec<&str> = INFRASTRUCTURE_ERROR_CLASSES.iter().map(|(name, _, _)| *name).collect();

        let code_count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            code_count,
            "infrastructure error codes must be pairwise unique"
        );

        let name_count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            name_count,
            names.len(),
            "infrastructure exception class names must be pairwise unique"
        );
    }

    /// Regression for the same class of bug from the switch-dispatch side: every infrastructure
    /// exception class, plus every explicitly taxonomy-numbered user error variant, must get
    /// exactly one `case <code> -> throw new <name>(msg);` arm — no class emitted with a code
    /// that never appears in the switch (unreachable), and no two classes sharing a code (a
    /// duplicate-`case` compile error in the generated Java). ~keep
    #[test]
    fn every_declared_exception_has_a_unique_reachable_dispatch_arm() {
        let error = crate::core::ir::ErrorDef {
            name: "RequestError".to_string(),
            rust_path: "sample::RequestError".to_string(),
            variants: vec![
                crate::core::ir::ErrorVariant {
                    error_code: Some(100),
                    name: "InvalidInput".to_string(),
                    is_unit: true,
                    ..Default::default()
                },
                crate::core::ir::ErrorVariant {
                    error_code: Some(101),
                    name: "Timeout".to_string(),
                    is_unit: true,
                    ..Default::default()
                },
            ],
            original_rust_path: String::new(),
            doc: String::new(),
            methods: Vec::new(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };
        let api = crate::core::ir::ApiSurface {
            errors: vec![error],
            ..Default::default()
        };
        let mut output = "checkLastError()".to_string();
        gen_helper_methods(&mut output, "sample", "Sample", &api);

        let expected: Vec<(u32, String)> = INFRASTRUCTURE_ERROR_CLASSES
            .iter()
            .map(|(name, code, _)| (*code, (*name).to_string()))
            .chain(
                api.error_taxonomy()
                    .iter()
                    .map(|entry| (entry.code, format!("{}Exception", entry.variant))),
            )
            .collect();
        assert_eq!(expected.len(), 5, "expected 3 infrastructure + 2 taxonomy exceptions");

        for (code, class_name) in &expected {
            let arm = format!("case {code} -> throw new {class_name}(msg);");
            assert!(
                output.contains(&arm),
                "missing reachable dispatch arm for {class_name} (code {code}): {arm}"
            );
        }

        let case_count = output.matches("case ").count();
        assert_eq!(
            case_count,
            expected.len(),
            "generated switch must have exactly one case per declared exception, no extras and no drops"
        );
    }
}

#[cfg(test)]
mod bool_return_narrowing_tests {
    use super::*;

    /// A boolean FFI return must be narrowed to `int` before the `!= 0` comparison.
    ///
    /// The descriptor declares JAVA_LONG while the native function returns `int32_t`, so the
    /// upper 32 bits are undefined. Comparing the untruncated `long` against 0 turns any
    /// garbage in that half into a spurious `true`, meaning such a call can never report
    /// false. Reported against two separate consumers' generated bindings. ~keep
    #[test]
    fn a_bool_return_narrows_before_the_zero_comparison() {
        assert_eq!(
            java_ffi_return_cast(&TypeRef::Primitive(PrimitiveType::Bool)),
            "(int)(long)",
            "a bare (long) leaves the undefined upper half in the != 0 comparison"
        );
    }

    /// The companion half: the *layout* must stay JAVA_LONG. Narrowing the descriptor instead
    /// of the call site reintroduces a JBR-on-Windows ClassCastException. ~keep
    #[test]
    fn a_bool_layout_stays_widened() {
        assert_eq!(
            crate::backends::java::type_map::java_ffi_type(&PrimitiveType::Bool),
            "ValueLayout.JAVA_LONG"
        );
    }

    /// Every other integral return already narrows; bool is now consistent with them.
    #[test]
    fn integral_returns_all_narrow_from_the_widened_layout() {
        for (prim, expected) in [
            (PrimitiveType::I8, "(byte)(long)"),
            (PrimitiveType::I16, "(short)(long)"),
            (PrimitiveType::I32, "(int)(long)"),
            (PrimitiveType::Bool, "(int)(long)"),
        ] {
            assert_eq!(java_ffi_return_cast(&TypeRef::Primitive(prim)), expected);
        }
    }
}
