use crate::codegen::naming::{PublicIdentifierKind, public_host_identifier};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, FieldDef, PrimitiveType, TypeDef, TypeRef};
use std::collections::{BTreeSet, HashMap};

pub(super) fn find_r_options_type<'a>(api: &'a ApiSurface, config: &ResolvedCrateConfig) -> Option<&'a TypeDef> {
    config
        .trait_bridges
        .iter()
        .filter(|bridge| bridge.bind_via == crate::core::config::BridgeBinding::OptionsField)
        .filter_map(|bridge| bridge.options_type.as_deref())
        .find_map(|type_name| api.types.iter().find(|t| t.name == type_name && !t.is_trait))
        .or_else(|| find_r_options_type_from_api(api))
}

pub(super) fn find_r_options_type_from_api(api: &ApiSurface) -> Option<&TypeDef> {
    let input_type_names = crate::codegen::conversions::input_type_names(api);
    api.types
        .iter()
        .find(|t| !t.is_trait && t.has_default && input_type_names.contains(&t.name))
}

/// Generate the `options.R` file for the R package from the configured options IR type.
///
/// Produces a roxygen-documented `conversion_options()` helper function with one parameter per
/// field (all defaulting to `NULL`). R callers use named arguments to override individual
/// settings; unset parameters remain `NULL` and are omitted from the resulting list so that the
/// Rust side applies its own defaults.
pub(super) fn gen_conversion_options_r(opts_type: &TypeDef) -> String {
    let params: Vec<String> = opts_type
        .fields
        .iter()
        .map(|f| format!("{} = NULL", f.name.trim_start_matches('_')))
        .collect();

    let fields: Vec<minijinja::Value> = opts_type
        .fields
        .iter()
        .map(|field| {
            let rname = field.name.trim_start_matches('_');
            let doc_text = if field.doc.is_empty() {
                rname.to_string()
            } else {
                let first = field.doc.lines().next().unwrap_or(rname);
                first.trim_end_matches('.').to_string()
            };

            let needs_int = matches!(
                &field.ty,
                TypeRef::Primitive(PrimitiveType::U8)
                    | TypeRef::Primitive(PrimitiveType::U16)
                    | TypeRef::Primitive(PrimitiveType::U32)
                    | TypeRef::Primitive(PrimitiveType::U64)
                    | TypeRef::Primitive(PrimitiveType::I8)
                    | TypeRef::Primitive(PrimitiveType::I16)
                    | TypeRef::Primitive(PrimitiveType::I32)
                    | TypeRef::Primitive(PrimitiveType::I64)
                    | TypeRef::Primitive(PrimitiveType::Usize)
            );
            let assign_val = if needs_int {
                format!("as.integer({rname})")
            } else {
                rname.to_string()
            };

            minijinja::context! {
                rname => rname,
                doc => doc_text,
                cfg => field.cfg.is_some(),
                assign_val => assign_val,
            }
        })
        .collect();

    crate::backends::extendr::template_env::render(
        "conversion_options.jinja",
        minijinja::context! {
            params => params,
            fields => fields,
        },
    )
}

/// Generate the Rust-side `options.rs` module with `decode_options` function.
///
/// The `decode_options` function handles input from R in three main forms:
/// 1. ExternalPtr<T> (from $default() / builder methods) — unwraps and converts to core
/// 2. NULL — uses the configured options type's default
/// 3. Named list with field names matching struct fields — decodes field by field
///
/// This allows R callers to pass `OptionsType$default()`, NULL, or a named list.
pub(super) fn gen_options_rs(api: &ApiSurface, opts_type: &TypeDef, _core_import: &str) -> String {
    let mut code = String::new();
    code.push_str("//! Option decoding for R bindings.\n\n");
    code.push_str("use extendr_api::prelude::*;\n\n");

    let type_defs: HashMap<_, _> = api.types.iter().map(|t| (t.name.as_str(), t)).collect();
    let enum_defs: HashMap<_, _> = api.enums.iter().map(|e| (e.name.as_str(), e)).collect();

    let mut enum_decoders = BTreeSet::new();
    let mut struct_decoders = BTreeSet::new();
    for field in &opts_type.fields {
        collect_option_decoder_types(
            &field.ty,
            opts_type.name.as_str(),
            &type_defs,
            &enum_defs,
            &mut enum_decoders,
            &mut struct_decoders,
        );
    }

    code.push_str("/// Helper: extract and convert a value from an R list by name.\n");
    code.push_str("fn list_get(list: &List, key: &str) -> Option<Robj> {\n");
    code.push_str("    list.iter().find(|(n, _)| *n == key).map(|(_, v)| v)\n");
    code.push_str("}\n\n");

    for enum_name in enum_decoders {
        if let Some(enum_def) = enum_defs.get(enum_name.as_str()) {
            gen_enum_decoder(&mut code, enum_def);
        }
    }

    for struct_name in struct_decoders {
        if let Some(struct_def) = type_defs.get(struct_name.as_str()) {
            gen_struct_decoder(&mut code, struct_def, &enum_defs, &type_defs);
        }
    }

    code.push_str("/// Decode an R ExternalPtr, NULL, or named list into ");
    code.push_str(&opts_type.name);
    code.push_str(".\n");
    code.push_str("///\n");
    code.push_str("/// Accepts:\n");
    code.push_str("/// - ExternalPtr of the configured options type (from $default() or builder methods) — unwraps and converts\n");
    code.push_str("/// - NULL — returns the configured options type's default\n");
    code.push_str("/// - Named list with field names matching struct fields — decodes field by field\n");
    code.push_str("///\n");
    code.push_str("/// Fields are optional: omitted fields retain their defaults. Unknown fields are ignored.\n");
    code.push_str("pub fn decode_options(options: Robj) -> std::result::Result<crate::");
    code.push_str(&opts_type.name);
    code.push_str(", String> {\n");
    code.push_str("    if options.is_null() {\n");
    code.push_str("        return Ok(crate::");
    code.push_str(&opts_type.name);
    code.push_str("::default());\n");
    code.push_str("    }\n\n");

    code.push_str("    // Accept the wrapper struct returned by the options type's default() / builder methods,\n");
    code.push_str("    // which extendr exposes as an `ExternalPtr`. The binding struct is returned directly\n");
    code.push_str("    // from the #[extendr] impl methods, so unwrap it as the binding type.\n");
    code.push_str("    if let Ok(ext) = ExternalPtr::<crate::");
    code.push_str(&opts_type.name);
    code.push_str(">::try_from(&options) {\n");
    code.push_str("        // Clone the binding struct and convert to core type via the generated From impl\n");
    code.push_str("        return Ok((*ext).clone().into());\n");
    code.push_str("    }\n\n");

    code.push_str("    // Try to decode as a named list\n");
    code.push_str("    let list = List::try_from(&options)\n");
    code.push_str("        .map_err(|e| format!(\"options must be NULL, ExternalPtr, or named list: {e}\"))?;\n");
    code.push_str("    let mut opts = crate::");
    code.push_str(&opts_type.name);
    code.push_str("::default();\n\n");

    for field in &opts_type.fields {
        gen_field_decoder(&mut code, field, &enum_defs, &type_defs);
    }

    code.push_str(
        "    // Note: visitor field is skipped — R has no visitor concept, so it remains at default None\n\n",
    );
    code.push_str("    Ok(opts)\n");
    code.push_str("}\n");

    code
}

pub(super) fn collect_option_decoder_types(
    ty: &TypeRef,
    root_type_name: &str,
    type_defs: &HashMap<&str, &TypeDef>,
    enum_defs: &HashMap<&str, &EnumDef>,
    enum_decoders: &mut BTreeSet<String>,
    struct_decoders: &mut BTreeSet<String>,
) {
    let TypeRef::Named(name) = ty else {
        if let TypeRef::Optional(inner) = ty {
            collect_option_decoder_types(
                inner,
                root_type_name,
                type_defs,
                enum_defs,
                enum_decoders,
                struct_decoders,
            );
        }
        return;
    };
    if enum_defs.contains_key(name.as_str()) {
        enum_decoders.insert(name.clone());
        return;
    }
    let Some(type_def) = type_defs.get(name.as_str()) else {
        return;
    };
    if type_def.name == root_type_name || type_def.is_opaque || type_def.is_trait {
        return;
    }
    if struct_decoders.insert(type_def.name.clone()) {
        for field in &type_def.fields {
            collect_option_decoder_types(
                &field.ty,
                root_type_name,
                type_defs,
                enum_defs,
                enum_decoders,
                struct_decoders,
            );
        }
    }
}

/// Generate an enum decoder function for the given enum definition.
pub(super) fn gen_enum_decoder(code: &mut String, enum_def: &EnumDef) {
    if enum_def.variants.iter().any(|variant| !variant.fields.is_empty()) {
        return;
    }

    let enum_name = &enum_def.name;
    let field_name_snake = r_function_component(enum_name);
    let fn_name = format!("decode_{}", field_name_snake);

    code.push_str("/// Decode a ");
    code.push_str(&field_name_snake.replace('_', " "));
    code.push_str(" enum from its string representation.\n");
    code.push_str("fn ");
    code.push_str(&fn_name);
    code.push_str("(val: Robj) -> std::result::Result<crate::");
    code.push_str(enum_name);
    code.push_str(", String> {\n");
    code.push_str("    let s = String::try_from(&val).map_err(|e| format!(\"");
    code.push_str(&field_name_snake);
    code.push_str(": {e}\"))?;\n");
    code.push_str("    match s.as_str() {\n");

    for variant in &enum_def.variants {
        code.push_str("        \"");
        code.push_str(&variant.name);
        code.push_str("\" => Ok(crate::");
        code.push_str(enum_name);
        code.push_str("::");
        code.push_str(&variant.name);
        code.push_str("),\n");
    }

    code.push_str("        _ => Err(format!(\"");
    code.push_str(&field_name_snake);
    code.push_str(": unknown variant '{}'\", s)),\n");
    code.push_str("    }\n");
    code.push_str("}\n\n");
}

/// Generate decoder for a nested options struct.
pub(super) fn gen_struct_decoder(
    code: &mut String,
    typ: &TypeDef,
    enum_defs: &HashMap<&str, &EnumDef>,
    type_defs: &HashMap<&str, &TypeDef>,
) {
    let decoder_name = format!("decode_{}", r_function_component(&typ.name));
    let label = r_function_component(&typ.name);
    code.push_str("/// Decode ");
    code.push_str(&typ.name);
    code.push_str(" from an R list.\n");
    code.push_str("fn ");
    code.push_str(&decoder_name);
    code.push_str("(val: Robj) -> std::result::Result<crate::");
    code.push_str(&typ.name);
    code.push_str(", String> {\n");
    code.push_str("    if val.is_null() {\n");
    code.push_str("        return Ok(crate::");
    code.push_str(&typ.name);
    code.push_str("::default());\n");
    code.push_str("    }\n");
    code.push_str("    let list = List::try_from(&val).map_err(|e| format!(\"");
    code.push_str(&label);
    code.push_str(": {e}\"))?;\n");
    code.push_str("    let mut opts = crate::");
    code.push_str(&typ.name);
    code.push_str("::default();\n\n");

    for field in &typ.fields {
        gen_field_decoder(code, field, enum_defs, type_defs);
    }

    code.push_str("    Ok(opts)\n");
    code.push_str("}\n\n");
}

/// Map a core type to its binding type for the R extendr backend.
/// This applies the type transformations used in the binding layer:
/// - u64, i64, usize, isize -> f64
/// - Other primitives stay the same
/// - Optional wrapping is preserved
pub(super) fn map_type_to_binding(ty: &TypeRef) -> TypeRef {
    match ty {
        TypeRef::Primitive(
            _prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
        ) => TypeRef::Primitive(PrimitiveType::F64),
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(map_type_to_binding(inner))),
        other => other.clone(),
    }
}

/// Generate field decoding logic for a single field.
pub(super) fn gen_field_decoder(
    code: &mut String,
    field: &FieldDef,
    enum_defs: &HashMap<&str, &EnumDef>,
    type_defs: &HashMap<&str, &TypeDef>,
) {
    if field.name == "visitor" {
        return;
    }

    let binding_ty = map_type_to_binding(&field.ty);

    let field_name = &field.name;
    let field_name_trim = field_name.trim_start_matches('_');

    match &binding_ty {
        TypeRef::Primitive(PrimitiveType::Bool) => {
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = bool::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::String => {
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = String::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Char => {
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = String::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Primitive(
            prim @ (PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32),
        ) => {
            let ty = match prim {
                PrimitiveType::U8 => "u8",
                PrimitiveType::U16 => "u16",
                PrimitiveType::U32 => "u32",
                PrimitiveType::I8 => "i8",
                PrimitiveType::I16 => "i16",
                PrimitiveType::I32 => "i32",
                _ => unreachable!(),
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = ");
            code.push_str(ty);
            code.push_str("::try_from(&v).map_err(|e| format!(\"");
            code.push_str(field_name_trim);
            code.push_str(": {e}\"))?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Primitive(
            prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
        ) => {
            let core_ty = match prim {
                PrimitiveType::U64 => "u64",
                PrimitiveType::I64 => "i64",
                PrimitiveType::Usize => "usize",
                PrimitiveType::Isize => "isize",
                _ => unreachable!(),
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            if field.optional {
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val as ");
                code.push_str(core_ty);
                code.push_str(");\n");
                code.push_str("        }\n");
            } else {
                code.push_str("        let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = f64_val as ");
                code.push_str(core_ty);
                code.push_str(";\n");
            }
            code.push_str("    }\n");
        }
        TypeRef::Primitive(PrimitiveType::F32 | PrimitiveType::F64) => {
            let ty = match &binding_ty {
                TypeRef::Primitive(PrimitiveType::F32) => "f32",
                _ => "f64",
            };
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            if field.optional {
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = ");
                code.push_str(ty);
                code.push_str("::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val);\n");
                code.push_str("        }\n");
            } else {
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = ");
                code.push_str(ty);
                code.push_str("::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
            }
            code.push_str("    }\n");
        }
        TypeRef::Vec(inner) => {
            if matches!(inner.as_ref(), TypeRef::String) {
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        let strings = Strings::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("        let vec: Vec<String> = strings\n");
                code.push_str("            .iter()\n");
                code.push_str("            .map(|s| s.to_string())\n");
                code.push_str("            .collect();\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = vec;\n");
                code.push_str("    }\n");
            }
        }
        TypeRef::Named(enum_name)
            if (enum_defs.contains_key(enum_name.as_str()) || type_defs.contains_key(enum_name.as_str())) =>
        {
            let fn_name = format!("decode_{}", r_function_component(enum_name));
            code.push_str("    if let Some(v) = list_get(&list, \"");
            code.push_str(field_name_trim);
            code.push_str("\") {\n");
            code.push_str("        opts.");
            code.push_str(field_name);
            code.push_str(" = ");
            code.push_str(&fn_name);
            code.push_str("(v)?;\n");
            code.push_str("    }\n");
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if enum_defs.contains_key(name.as_str()) => {
                let fn_name = format!("decode_{}", r_function_component(name));
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = Some(");
                code.push_str(&fn_name);
                code.push_str("(v)?);\n");
                code.push_str("    }\n");
            }
            TypeRef::Named(name) if type_defs.contains_key(name.as_str()) => {
                let fn_name = format!("decode_{}", r_function_component(name));
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        opts.");
                code.push_str(field_name);
                code.push_str(" = Some(");
                code.push_str(&fn_name);
                code.push_str("(v)?);\n");
                code.push_str("    }\n");
            }
            TypeRef::Primitive(
                prim @ (PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize),
            ) => {
                let core_ty = match prim {
                    PrimitiveType::U64 => "u64",
                    PrimitiveType::I64 => "i64",
                    PrimitiveType::Usize => "usize",
                    PrimitiveType::Isize => "isize",
                    _ => unreachable!(),
                };
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val as ");
                code.push_str(core_ty);
                code.push_str(");\n");
                code.push_str("        }\n");
                code.push_str("    }\n");
            }
            TypeRef::Primitive(PrimitiveType::F64) => {
                code.push_str("    if let Some(v) = list_get(&list, \"");
                code.push_str(field_name_trim);
                code.push_str("\") {\n");
                code.push_str("        if !v.is_null() {\n");
                code.push_str("            let f64_val = f64::try_from(&v).map_err(|e| format!(\"");
                code.push_str(field_name_trim);
                code.push_str(": {e}\"))?;\n");
                code.push_str("            opts.");
                code.push_str(field_name);
                code.push_str(" = Some(f64_val);\n");
                code.push_str("        }\n");
                code.push_str("    }\n");
            }
            _ => {}
        },
        _ => {}
    }
}

/// Convert a CamelCase type name to snake_case for function names.
fn r_function_component(name: &str) -> String {
    public_host_identifier(Language::R, PublicIdentifierKind::Function, name)
}
