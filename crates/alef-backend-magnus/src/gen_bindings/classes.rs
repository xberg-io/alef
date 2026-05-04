//! Struct and enum code generators for the Magnus (Ruby) backend.

use ahash::AHashSet;
use alef_codegen::builder::ImplBuilder;
use alef_codegen::generators;
use alef_codegen::shared::{constructor_parts, function_params};
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{EnumDef, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use std::fmt::Write;

use crate::type_map::MagnusMapper;

use super::functions::gen_magnus_unimplemented_body;

/// Check if a field contains a type that cannot be safely passed across thread boundaries.
/// Magnus's #[magnus::wrap] requires Send + Sync bounds. Fields containing types like
/// VisitorHandle (Rc<RefCell<dyn HtmlVisitor>>) are !Send + !Sync and must be excluded.
fn is_thread_unsafe_field(field: &FieldDef) -> bool {
    matches!(&field.ty, TypeRef::Named(name) if name == "VisitorHandle")
        || matches!(field.ty, TypeRef::Optional(ref inner) if matches!(inner.as_ref(), TypeRef::Named(name) if name == "VisitorHandle"))
}

/// Generate an opaque Magnus-wrapped struct with inner Arc.
pub(super) fn gen_opaque_struct(typ: &TypeDef, core_import: &str, module_name: &str) -> String {
    let class_path = format!("{}::{}", module_name, typ.name);

    let mut out = String::with_capacity(256);
    writeln!(out, "#[derive(Clone)]").ok();
    writeln!(out, r#"#[magnus::wrap(class = "{}")]"#, class_path).ok();
    writeln!(out, "pub struct {} {{", typ.name).ok();
    let core_path = alef_codegen::conversions::core_type_path(typ, core_import);
    writeln!(out, "    inner: Arc<{}>,", core_path).ok();
    writeln!(out, "}}").ok();
    let name = &typ.name;
    writeln!(out).ok();
    // SAFETY: #[magnus::wrap] already provides IntoValue. This marker trait
    // enables use in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
    // Magnus only provides TryConvert for &T (references) on TypedData types.
    // We need TryConvert for owned T so wrapped types can be used as function parameters.
    writeln!(out, "\nimpl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let r: &{name} = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        Ok(r.clone())").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    // SAFETY: TryConvert produces an owned value via Clone, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
    out
}

/// Generate Magnus methods for an opaque struct (delegates to self.inner).
pub(super) fn gen_opaque_struct_methods(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    for method in &typ.methods {
        if !method.is_static {
            if method.is_async {
                impl_builder.add_method(&gen_opaque_async_instance_method(
                    method,
                    mapper,
                    &typ.name,
                    opaque_types,
                ));
            } else {
                impl_builder.add_method(&gen_opaque_instance_method(method, mapper, &typ.name, opaque_types));
            }
        }
    }

    impl_builder.build()
}

/// Generate an opaque sync instance method for Magnus (delegates to self.inner).
fn gen_opaque_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    use alef_codegen::shared;
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        // For owned-receiver (consuming) methods, clone the Arc's inner value before calling,
        // since we cannot move out of an Arc from a &self method.
        let is_owned_receiver = matches!(method.receiver, Some(ReceiverKind::Owned));
        let inner_access = if is_owned_receiver {
            "self.inner.as_ref().clone()".to_string()
        } else {
            "self.inner".to_string()
        };
        let core_call = format!("{inner_access}.{}({})", method.name, call_args);
        if method.error_type.is_some() {
            if matches!(method.return_type, TypeRef::Unit) {
                format!(
                    "{core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(())"
                )
            } else {
                let wrap = generators::wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    true,
                    method.returns_ref,
                    method.returns_cow,
                );
                format!(
                    "let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok({wrap})"
                )
            }
        } else {
            generators::wrap_return(
                &core_call,
                &method.return_type,
                type_name,
                opaque_types,
                true,
                method.returns_ref,
                method.returns_cow,
            )
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n    "
    } else {
        ""
    };
    format!(
        "{trait_allow}fn {}(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an opaque async instance method for Magnus (block on runtime, delegates to self.inner).
fn gen_opaque_async_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    type_name: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    use alef_codegen::shared;
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let inner_clone = "let inner = self.inner.clone();\n        ";
        let core_call = format!("inner.{}({})", method.name, call_args);
        let result_wrap = generators::wrap_return(
            "result",
            &method.return_type,
            type_name,
            opaque_types,
            true,
            method.returns_ref,
            method.returns_cow,
        );
        if method.error_type.is_some() {
            format!(
                "{inner_clone}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok({result_wrap})"
            )
        } else {
            format!(
                "{inner_clone}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ {core_call}.await }});\n        \
                 {result_wrap}"
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            method.error_type.is_some(),
        )
    };
    format!(
        "fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

/// Generate a Magnus-wrapped struct definition using the shared TypeMapper.
pub(super) fn gen_struct(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    module_name: &str,
    _api: &alef_core::ir::ApiSurface,
    generates_default: bool,
) -> String {
    use std::fmt::Write as FmtWrite;

    let class_path = format!("{}::{}", module_name, typ.name);
    let name = &typ.name;

    // Build struct definition with private fields (to avoid conflicts with method names)
    let mut out = String::with_capacity(512);
    write!(out, "#[derive(Clone, Debug, serde::Serialize, serde::Deserialize").ok();
    if !generates_default {
        write!(out, ", Default").ok();
    }
    writeln!(out, ")]").ok();
    if typ.has_default {
        writeln!(out, "#[serde(default)]").ok();
    }
    writeln!(out, r#"#[magnus::wrap(class = "{}")]"#, class_path).ok();
    writeln!(out, "pub struct {} {{", name).ok();

    for field in &typ.fields {
        // Skip visitor fields: VisitorHandle is Rc<RefCell<dyn HtmlVisitor>> which is !Send + !Sync.
        // Magnus's #[magnus::wrap] requires Send + Sync. These fields are handled separately
        // (e.g., via post-processing in the language layer or direct API calls).
        if is_thread_unsafe_field(field) {
            continue;
        }
        let field_type = if field.optional && !matches!(field.ty, TypeRef::Optional(_)) {
            mapper.optional(&mapper.map_type(&field.ty))
        } else {
            mapper.map_type(&field.ty)
        };
        // Fields are private to avoid conflicts with auto-generated method names
        writeln!(out, "    {}: {},", field.name, field_type).ok();
    }

    writeln!(out, "}}").ok();

    // SAFETY: #[magnus::wrap] already provides IntoValue. This marker trait
    // enables use in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "\nunsafe impl IntoValueFromNative for {name} {{}}").ok();
    // Magnus only provides TryConvert for &T (references) on TypedData types.
    // We need TryConvert for owned T so wrapped types can be used as function parameters.
    writeln!(out, "\nimpl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let r: &{name} = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        Ok(r.clone())").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    // SAFETY: TryConvert produces an owned value via Clone, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
    out
}

/// Generate Magnus methods for a struct.
pub(super) fn gen_struct_methods(
    typ: &TypeDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    core_import: &str,
    _generates_default: bool,
) -> String {
    let mut impl_builder = ImplBuilder::new(&typ.name);

    if !typ.fields.is_empty() {
        let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);

        // Filter out thread-unsafe fields (e.g., VisitorHandle) that cannot be used in Magnus constructors.
        let filtered_fields: Vec<FieldDef> = typ
            .fields
            .iter()
            .filter(|f| !is_thread_unsafe_field(f))
            .cloned()
            .collect();

        if !filtered_fields.is_empty() {
            // Generate config builder if type has Default semantics (accepts optional kwargs).
            // This uses unwrap_or_default() which requires Default on the type, but for types
            // where can_generate_default_impl is false we still need kwargs (too many params
            // for Magnus function! macro). The kwargs constructor handles this by using
            // the core type's Default impl via the core crate, not the binding struct's Default.
            // Magnus function! macro only supports arity -2..=15, so types with more than 15
            // fields must also use the hash-based constructor even when has_default is false.
            if typ.has_default || filtered_fields.len() > 15 {
                // Create a temporary type with filtered fields for constructor generation
                let mut filtered_typ = typ.clone();
                filtered_typ.fields = filtered_fields.clone();
                let config_method = alef_codegen::config_gen::gen_magnus_kwargs_constructor(&filtered_typ, &map_fn);
                impl_builder.add_method(&config_method);
            } else {
                let (param_list, _, assignments) = constructor_parts(&filtered_fields, &map_fn);
                let new_method = format!("fn new({param_list}) -> Self {{\n        Self {{ {assignments} }}\n    }}");
                impl_builder.add_method(&new_method);
            }
        }
    }

    for field in &typ.fields {
        // Skip thread-unsafe fields (e.g., VisitorHandle)
        if is_thread_unsafe_field(field) {
            continue;
        }
        impl_builder.add_method(&gen_field_accessor(field, mapper));
    }

    for method in &typ.methods {
        if !method.is_static {
            if method.is_async {
                impl_builder.add_method(&gen_async_instance_method(
                    method,
                    mapper,
                    typ,
                    opaque_types,
                    core_import,
                ));
            } else {
                impl_builder.add_method(&gen_instance_method(method, mapper, typ, opaque_types, core_import));
            }
        }
    }

    impl_builder.build()
}

/// Generate a field accessor method.
fn gen_field_accessor(field: &FieldDef, mapper: &MagnusMapper) -> String {
    let return_type = if field.optional {
        // Strip one Optional wrapper: when field.ty is already Optional(T) and field.optional is
        // also true (e.g. Option<Option<T>> in core), the struct field is declared as
        // Option<T> (struct codegen strips the outer Optional). The accessor must match.
        let inner_ty = match &field.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            ty => ty,
        };
        mapper.optional(&mapper.map_type(inner_ty))
    } else {
        mapper.map_type(&field.ty)
    };

    let body = if is_primitive_copy(&field.ty) {
        format!("self.{}", field.name)
    } else {
        format!("self.{}.clone()", field.name)
    };

    format!(
        "fn {}(&self) -> {} {{\n        {}\n    }}",
        field.name, return_type, body
    )
}

/// Check if a type is a Copy type (primitives and unit).
fn is_primitive_copy(ty: &alef_core::ir::TypeRef) -> bool {
    matches!(ty, alef_core::ir::TypeRef::Primitive(_) | alef_core::ir::TypeRef::Unit)
}

/// Generate an instance method binding for a non-opaque struct.
fn gen_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_codegen::shared;
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let needs_mut_receiver = method.receiver == Some(ReceiverKind::RefMut);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions = if needs_mut_receiver {
            generators::gen_lossy_binding_to_core_fields_mut(typ, core_import, false, opaque_types, false, false)
        } else {
            generators::gen_lossy_binding_to_core_fields(typ, core_import, false, opaque_types, false, false)
        };
        let core_call = format!("core_self.{}({})", method.name, call_args);
        let result_wrap = match &method.return_type {
            TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Path => ".into()".to_string(),
            // Bytes: when the core returns &Bytes (returns_ref=true), use .to_vec() since
            // Vec<u8> does not implement From<&Bytes>. For owned Bytes, .into() works.
            TypeRef::Bytes => {
                if method.returns_ref {
                    ".to_vec()".to_string()
                } else {
                    ".into()".to_string()
                }
            }
            _ => String::new(),
        };
        if method.error_type.is_some() {
            format!(
                "{field_conversions}let result = {core_call}.map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        Ok(result{result_wrap})"
            )
        } else {
            format!("{field_conversions}{core_call}{result_wrap}")
        }
    } else {
        gen_magnus_unimplemented_body(&method.return_type, &method.name, method.error_type.is_some())
    };
    let allow_attr = if !can_delegate {
        "#[allow(unused_variables)]\n    "
    } else {
        ""
    };
    let self_recv = if needs_mut_receiver { "&mut self" } else { "&self" };
    let trait_allow = if generators::is_trait_method_name(&method.name) {
        "#[allow(clippy::should_implement_trait)]\n    "
    } else {
        ""
    };
    format!(
        "{trait_allow}{allow_attr}fn {}({self_recv}, {params}) -> {return_annotation} {{\n        \
         {body}\n    }}",
        method.name
    )
}

/// Generate an async instance method binding for Magnus (block on runtime).
fn gen_async_instance_method(
    method: &MethodDef,
    mapper: &MagnusMapper,
    typ: &TypeDef,
    opaque_types: &AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_codegen::shared;
    let params = function_params(&method.params, &|ty| mapper.map_type(ty));
    let return_type = mapper.map_type(&method.return_type);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = !method.sanitized
        && method
            .params
            .iter()
            .all(|p| !p.sanitized && generators::is_simple_non_opaque_param(&p.ty))
        && shared::is_delegatable_return(&method.return_type);

    let body = if can_delegate {
        let call_args = generators::gen_call_args(&method.params, opaque_types);
        let field_conversions =
            generators::gen_lossy_binding_to_core_fields(typ, core_import, false, opaque_types, false, false);
        let result_wrap = match &method.return_type {
            TypeRef::Named(_) | TypeRef::String | TypeRef::Char | TypeRef::Path => ".into()".to_string(),
            // Bytes: when the core returns &Bytes (returns_ref=true), use .to_vec() since
            // Vec<u8> does not implement From<&Bytes>. For owned Bytes, .into() works.
            TypeRef::Bytes => {
                if method.returns_ref {
                    ".to_vec()".to_string()
                } else {
                    ".into()".to_string()
                }
            }
            _ => String::new(),
        };
        if method.error_type.is_some() {
            format!(
                "{field_conversions}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }}).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 Ok(result{result_wrap})",
                name = method.name
            )
        } else {
            format!(
                "{field_conversions}let rt = tokio::runtime::Runtime::new().map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_runtime_error(), e.to_string()))?;\n        \
                 let result = rt.block_on(async {{ core_self.{name}({call_args}).await }});\n        \
                 result{result_wrap}",
                name = method.name
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &method.return_type,
            &format!("{}_async", method.name),
            method.error_type.is_some(),
        )
    };
    format!(
        "fn {}_async(&self, {params}) -> {return_annotation} {{\n        \
         {body}\n    \
         }}",
        method.name
    )
}

/// Convert a PascalCase name to snake_case for Ruby symbol mapping.
pub(super) fn pascal_to_snake(name: &str) -> String {
    let mut result = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    result
}

/// Generate a Magnus enum definition with IntoValue and TryConvert impls.
/// Unit-variant enums are represented as Ruby Symbols for ergonomic Ruby usage.
pub(super) fn gen_enum(enum_def: &EnumDef) -> String {
    let name = &enum_def.name;
    let mut out = String::with_capacity(512);

    let has_data = enum_def.variants.iter().any(|v| !v.fields.is_empty());

    // Enum definition
    if has_data {
        // Data enum: can't be Copy, include serde tag attribute
        writeln!(out, "#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]").ok();
        if let Some(tag) = &enum_def.serde_tag {
            writeln!(out, r#"#[serde(tag = "{tag}")]"#).ok();
        }
    } else {
        writeln!(
            out,
            "#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]"
        )
        .ok();
    }
    writeln!(out, "pub enum {name} {{").ok();
    for variant in &enum_def.variants {
        if let Some(rename) = &variant.serde_rename {
            writeln!(out, r#"    #[serde(rename = "{rename}")]"#).ok();
        }
        if variant.fields.is_empty() {
            writeln!(out, "    {},", variant.name).ok();
        } else {
            // Data variant with named fields
            let fields: Vec<String> = variant
                .fields
                .iter()
                .map(|f| format!("{}: {}", f.name, field_type_for_serde(f)))
                .collect();
            writeln!(out, "    {} {{ {} }},", variant.name, fields.join(", ")).ok();
        }
    }
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Default impl for config constructor unwrap_or_default()
    if let Some(first) = enum_def.variants.first() {
        if has_data && !first.fields.is_empty() {
            // Data variant default: use Default::default() for each field
            let field_defaults: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            writeln!(out, "impl Default for {name} {{").ok();
            writeln!(
                out,
                "    fn default() -> Self {{ Self::{} {{ {} }} }}",
                first.name,
                field_defaults.join(", ")
            )
            .ok();
            writeln!(out, "}}").ok();
        } else {
            writeln!(out, "impl Default for {name} {{").ok();
            writeln!(out, "    fn default() -> Self {{ Self::{} }}", first.name).ok();
            writeln!(out, "}}").ok();
        }
        writeln!(out).ok();
    }

    // For data enums, implement IntoValue via serde_json serialization
    // and TryConvert via serde_json deserialization.
    // Uses a json_to_ruby helper to convert serde_json::Value to Magnus values.
    if has_data {
        writeln!(out, "impl magnus::IntoValue for {name} {{").ok();
        writeln!(out, "    fn into_value_with(self, handle: &Ruby) -> magnus::Value {{").ok();
        writeln!(out, "        match serde_json::to_value(&self) {{").ok();
        writeln!(out, "            Ok(v) => json_to_ruby(handle, v),").ok();
        writeln!(out, "            Err(_) => handle.qnil().into_value_with(handle),").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(out, "impl magnus::TryConvert for {name} {{").ok();
        writeln!(
            out,
            "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
        )
        .ok();
        writeln!(out, "        let s: String = magnus::TryConvert::try_convert(val)?;").ok();
        writeln!(
            out,
            "        serde_json::from_str(&s).map_err(|e| magnus::Error::new(unsafe {{ Ruby::get_unchecked() }}.exception_type_error(), e.to_string()))"
        )
        .ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        writeln!(out).ok();
        writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
        writeln!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();
        return out;
    }

    // IntoValue: convert enum variant to Ruby Symbol
    writeln!(out, "impl magnus::IntoValue for {name} {{").ok();
    writeln!(out, "    fn into_value_with(self, handle: &Ruby) -> magnus::Value {{").ok();
    writeln!(out, "        let sym = match self {{").ok();
    for variant in &enum_def.variants {
        let snake = pascal_to_snake(&variant.name);
        writeln!(out, "            {name}::{} => \"{snake}\",", variant.name).ok();
    }
    writeln!(out, "        }};").ok();
    writeln!(out, "        handle.to_symbol(sym).into_value_with(handle)").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // TryConvert: convert Ruby Symbol/String to enum variant
    writeln!(out, "impl magnus::TryConvert for {name} {{").ok();
    writeln!(
        out,
        "    fn try_convert(val: magnus::Value) -> Result<Self, magnus::Error> {{"
    )
    .ok();
    writeln!(out, "        let s: String = magnus::TryConvert::try_convert(val)?;").ok();
    writeln!(out, "        match s.as_str() {{").ok();
    for variant in &enum_def.variants {
        let snake = pascal_to_snake(&variant.name);
        writeln!(out, "            \"{snake}\" => Ok({name}::{}),", variant.name).ok();
    }
    writeln!(out, "            other => Err(magnus::Error::new(").ok();
    writeln!(
        out,
        "                unsafe {{ Ruby::get_unchecked() }}.exception_arg_error(),"
    )
    .ok();
    writeln!(out, "                format!(\"invalid {name} value: {{other}}\"),").ok();
    writeln!(out, "            )),").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
    // SAFETY: IntoValue is implemented above. This marker trait enables use
    // in Vec<T> returns from Magnus function!/method! macros.
    writeln!(out, "unsafe impl IntoValueFromNative for {name} {{}}").ok();
    // SAFETY: TryConvert produces an owned value, satisfying owned conversion.
    write!(out, "unsafe impl TryConvertOwned for {name} {{}}").ok();

    out
}

/// Map a field type to a Rust type suitable for serde deserialization in data enums.
/// Helper to recursively map inner TypeRef to serde type strings.
/// For types that need JSON marshalling (Vec<Named>, Map, etc.), returns "String"
/// to indicate they should be JSON-serialized. Otherwise returns the proper type.
fn field_type_for_serde_inner(ty: &TypeRef) -> String {
    use alef_core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::U8) => "u8".to_string(),
        TypeRef::Primitive(PrimitiveType::U16) => "u16".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::U64) => "u64".to_string(),
        TypeRef::Primitive(PrimitiveType::Usize) => "usize".to_string(),
        TypeRef::Primitive(PrimitiveType::I8) => "i8".to_string(),
        TypeRef::Primitive(PrimitiveType::I16) => "i16".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::I64) => "i64".to_string(),
        TypeRef::Primitive(PrimitiveType::Isize) => "isize".to_string(),
        TypeRef::Primitive(PrimitiveType::F32) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Duration => "u64".to_string(),
        // Named types serde-derive in the generated module — emit by name so JSON
        // arrays/objects deserialize directly via serde.
        TypeRef::Named(n) => n.clone(),
        // Recurse for Vec so Vec<Item> / Vec<String> round-trip as actual JSON arrays.
        TypeRef::Vec(inner) => format!("Vec<{}>", field_type_for_serde_inner(inner)),
        // Map keys/values may be opaque or non-serde; collapse to String and round-trip via serde_json.
        TypeRef::Map(_, _) => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", field_type_for_serde_inner(inner)),
        _ => "String".to_string(),
    }
}

fn field_type_for_serde(field: &FieldDef) -> String {
    let base = field_type_for_serde_inner(&field.ty);
    if field.optional {
        format!("Option<{base}>")
    } else {
        base
    }
}

/// Helper to filter generated code by removing field assignments for unsafe fields.
/// Removes lines where the field_name (before ':') matches an unsafe field.
fn filter_unsafe_field_assignments(full_code: &str, typ: &TypeDef) -> String {
    let mut filtered_lines = Vec::new();
    let mut in_struct_body = false;

    for line in full_code.lines() {
        let trimmed = line.trim();

        // Track if we're inside the struct initialization
        if trimmed.contains("Self {") || trimmed.contains("} {") {
            in_struct_body = true;
            filtered_lines.push(line.to_string());
            continue;
        }

        if in_struct_body {
            if trimmed.starts_with('}') {
                in_struct_body = false;
                filtered_lines.push(line.to_string());
                continue;
            }

            // Extract field name from assignment like "field_name: val.field_name.into(),"
            let field_name = if let Some(colon_idx) = trimmed.find(':') {
                trimmed[..colon_idx].trim()
            } else {
                ""
            };

            // Skip if this is a thread-unsafe field
            if typ
                .fields
                .iter()
                .any(|f| f.name == field_name && is_thread_unsafe_field(f))
            {
                continue;
            }
        }

        filtered_lines.push(line.to_string());
    }

    filtered_lines.join("\n")
}

/// Generate a custom From impl for binding → core conversion that filters thread-unsafe fields.
/// This is used instead of alef_codegen's gen_from_binding_to_core to exclude fields like
/// VisitorHandle that cannot be Send + Sync. We post-process the generated code to remove
/// unsafe field assignments.
pub(super) fn gen_from_binding_to_core_filtered(typ: &TypeDef, core_import: &str) -> String {
    // First get the full generated code from alef_codegen
    let full_code = alef_codegen::conversions::gen_from_binding_to_core(typ, core_import);

    // If there are no thread-unsafe fields, just return the full code
    if !typ.fields.iter().any(is_thread_unsafe_field) {
        return full_code;
    }

    filter_unsafe_field_assignments(&full_code, typ)
}

/// Generate a custom From impl for core → binding conversion that filters thread-unsafe fields.
/// This is used instead of alef_codegen's gen_from_core_to_binding to exclude fields like
/// VisitorHandle that cannot be Send + Sync. We post-process the generated code to remove
/// unsafe field assignments.
pub(super) fn gen_from_core_to_binding_filtered(
    typ: &TypeDef,
    core_import: &str,
    opaque_types: &AHashSet<String>,
) -> String {
    // First get the full generated code from alef_codegen
    let full_code = alef_codegen::conversions::gen_from_core_to_binding(typ, core_import, opaque_types);

    // If there are no thread-unsafe fields, just return the full code
    if !typ.fields.iter().any(is_thread_unsafe_field) {
        return full_code;
    }

    filter_unsafe_field_assignments(&full_code, typ)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
        }
    }

    fn make_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("test_lib::{name}"),
            original_rust_path: String::new(),
            fields,
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
        }
    }

    #[test]
    fn pascal_to_snake_converts_camel_case() {
        assert_eq!(pascal_to_snake("FooBar"), "foo_bar");
        assert_eq!(pascal_to_snake("PaddleOcr"), "paddle_ocr");
        assert_eq!(pascal_to_snake("Tesseract"), "tesseract");
    }

    #[test]
    fn gen_enum_unit_variants_emit_ruby_symbols() {
        let enum_def = EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Pending".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
                EnumVariant {
                    name: "Done".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: false,
            serde_tag: None,
            serde_rename_all: None,
        };
        let code = gen_enum(&enum_def);
        assert!(code.contains("enum Status"), "must emit enum definition");
        assert!(code.contains("to_symbol"), "unit enums use Ruby symbols");
        assert!(code.contains("\"pending\""), "variant snake_case symbol key");
    }

    #[test]
    fn gen_struct_emits_magnus_wrap_attribute() {
        let typ = make_typedef("Config", vec![make_field("value", TypeRef::String, false)]);
        let mapper = crate::type_map::MagnusMapper;
        let api = alef_core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
        };
        let code = gen_struct(&typ, &mapper, "TestLib", &api, false);
        assert!(code.contains("magnus::wrap"), "struct must have magnus::wrap");
        assert!(code.contains("struct Config"), "must emit struct Config");
    }

    #[test]
    fn gen_opaque_struct_emits_arc_inner() {
        let typ = make_typedef("Handle", vec![]);
        let code = gen_opaque_struct(&typ, "test_lib", "TestLib");
        assert!(code.contains("inner: Arc<"), "opaque struct must have Arc inner");
        assert!(code.contains("struct Handle"), "must emit struct Handle");
    }
}
