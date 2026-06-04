//! WASM free-function and utility code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name};
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

/// Check if a struct should have an Input DTO for JS object deserialization.
///
/// Input DTOs are needed to properly handle camelCase field name mapping via per-field
/// #[serde(rename)] attributes. This is necessary because serde_wasm_bindgen does not
/// honor container-level `rename_all` directives when deserializing from JsValue objects.
/// The decision is based on extracted wire metadata and field shape, not type-name suffixes.
pub(super) fn should_have_input_dto(type_def: &crate::core::ir::TypeDef) -> bool {
    type_def.has_default
        && type_def.has_serde
        && crate::codegen::shared::binding_fields(&type_def.fields).any(|field| {
            field.serde_rename.is_some()
                || type_def.serde_rename_all.is_some()
                || crate::codegen::naming::to_node_name(&field.name) != field.name
        })
}

/// Generate an Input DTO struct that deserializes from camelCase and converts to the core type.
/// Returns (input_dto_code, input_dto_name).
/// Reads actual struct fields from the `ApiSurface` TypeDef.
/// Accepts exclude_types and enabled_features to properly gate fields whose types
/// are not available in the target's feature set.
pub(super) fn gen_input_dto_for_type(
    type_name: &str,
    core_import: &str,
    type_def: &crate::core::ir::TypeDef,
) -> (String, String) {
    // Legacy signature without feature gating info — used by tests and legacy callers.
    // For actual WASM backend generation, use gen_input_dto_for_type_with_cfg.
    gen_input_dto_for_type_with_cfg(
        type_name,
        core_import,
        type_def,
        &[],
        &[],
        &std::collections::HashSet::new(),
    )
}

/// Generate an Input DTO struct with feature-gate awareness.
/// exclude_types: list of types that don't compile in the target (e.g., LayoutDetectionConfig on WASM)
/// enabled_features: list of features enabled in the target's feature set
/// non_deserializable_type_names: names of IR types whose Rust definition does not
///   implement `serde::Deserialize` — typically trait objects, type aliases over
///   `dyn Trait`, or opaque handles. Fields referencing one of these by Named type
///   are emitted with `#[serde(skip)]` so the DTO derives `Deserialize` cleanly.
pub(super) fn gen_input_dto_for_type_with_cfg(
    type_name: &str,
    core_import: &str,
    type_def: &crate::core::ir::TypeDef,
    exclude_types: &[String],
    enabled_features: &[String],
    non_deserializable_type_names: &std::collections::HashSet<String>,
) -> (String, String) {
    let input_name = format!("{}Input", type_name);
    let core_path = format!("{}::{}", core_import, type_name);

    // Map fields from the real struct definition.
    // All DTO fields are Option<T> so JS may omit them. The template assigns
    // each present field into the core type via the per-field `conv` expression
    // (in terms of the bound variable `v`), respecting the core type's Default
    // for omitted fields.
    let fields: Vec<_> = crate::codegen::shared::binding_fields(&type_def.fields)
        .map(|f| {
            let field_references_excluded = super::field_references_excluded_type(&f.ty, exclude_types);
            let field_cfg = f.cfg.as_deref();

            // Check if this field's cfg condition is satisfied by the enabled features
            let cfg_satisfied = if let Some(cfg_str) = field_cfg {
                super::cfg_condition_enabled(cfg_str, enabled_features)
            } else {
                true
            };

            // Detect fields whose Named type does not derive serde::Deserialize
            // (trait objects, type aliases over `dyn Trait`, opaque handles).
            // Optional<Named> unwraps to Named for this check.
            let inner_ty = match &f.ty {
                crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let field_references_non_deserializable = matches!(
                inner_ty,
                crate::core::ir::TypeRef::Named(name) if non_deserializable_type_names.contains(name)
            );

            // Fields whose type is excluded OR whose cfg is not satisfied OR whose type
            // is a non-deserializable opaque handle are skipped: they appear in the DTO
            // struct for symmetry, but are not deserialized from JS.
            let is_skipped = field_references_excluded || !cfg_satisfied || field_references_non_deserializable;

            let dto_ty = format!("Option<{}>", type_ref_to_dto_type(&f.ty, core_import));
            let camel_case_name = to_node_name(&f.name);

            minijinja::context! {
                name => &f.name,
                ty => &dto_ty,
                core_name => &f.name,
                serde_rename => &camel_case_name,
                conv => dto_field_conversion(&f.ty, f.sanitized),
                cfg => field_cfg,
                is_skipped => is_skipped,
            }
        })
        .collect::<Vec<_>>();

    let code = if !fields.is_empty() || !type_def.fields.is_empty() {
        crate::backends::wasm::template_env::render(
            "gen_input_dto",
            minijinja::context! {
                input_name => &input_name,
                core_path => &core_path,
                fields => &fields,
                has_default => type_def.has_default,
            },
        )
    } else {
        String::new()
    };

    (code, input_name)
}

/// Convert a TypeRef to a DTO field type string.
///
/// `Named` types are core-qualified (`{core_import}::{name}`) because the DTO is
/// deserialized via serde and converted into the core type: the core type already
/// derives `Deserialize`, and emitting the bare name would leave it unresolved in
/// the binding crate (the wasm-mapped wrapper enum is not the DTO field type).
fn type_ref_to_dto_type(ty: &crate::core::ir::TypeRef, core_import: &str) -> String {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            crate::core::ir::PrimitiveType::Bool => "bool".to_string(),
            crate::core::ir::PrimitiveType::U8 => "u8".to_string(),
            crate::core::ir::PrimitiveType::U16 => "u16".to_string(),
            crate::core::ir::PrimitiveType::U32 => "u32".to_string(),
            crate::core::ir::PrimitiveType::U64 => "u64".to_string(),
            crate::core::ir::PrimitiveType::I8 => "i8".to_string(),
            crate::core::ir::PrimitiveType::I16 => "i16".to_string(),
            crate::core::ir::PrimitiveType::I32 => "i32".to_string(),
            crate::core::ir::PrimitiveType::I64 => "i64".to_string(),
            crate::core::ir::PrimitiveType::F32 => "f32".to_string(),
            crate::core::ir::PrimitiveType::F64 => "f64".to_string(),
            crate::core::ir::PrimitiveType::Usize => "usize".to_string(),
            crate::core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", type_ref_to_dto_type(inner, core_import)),
        TypeRef::Optional(inner) => format!("Option<{}>", type_ref_to_dto_type(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            type_ref_to_dto_type(k, core_import),
            type_ref_to_dto_type(v, core_import)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => format!("{core_import}::{n}"),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Build the conversion expression turning a present DTO field value (bound as
/// the variable `v`) into the core struct field value.
///
/// Most field types convert with a plain `v.into()`: identity for matching
/// types, and `Option<T>: From<T>` papers over a core field that is `Option<_>`
/// while the DTO holds the bare `T`. Two core types have no such blanket `From`
/// from their DTO spelling and need an explicit constructor first:
/// `Duration` (DTO `u64` milliseconds) and `PathBuf` (DTO `String`). Wrapping
/// the constructed value in `Into::into` keeps the same optional-field papering
/// as the default branch, so the expression is valid whether the core field is
/// `T` or `Option<T>`.
///
/// When a field is sanitized (e.g., `Option<ConcurrencyConfig>` represented as
/// `Option<String>` for JSON serialization), use JSON deserialization instead
/// of `.into()`, which doesn't impl for the target type.
fn dto_field_conversion(ty: &crate::core::ir::TypeRef, sanitized: bool) -> String {
    use crate::core::ir::TypeRef;
    match ty {
        TypeRef::Duration => "Into::into(std::time::Duration::from_millis(v))".to_string(),
        TypeRef::Path => "Into::into(std::path::PathBuf::from(v))".to_string(),
        // Char: binding uses String, core uses char — take the first character.
        TypeRef::Char => "Into::into(v.chars().next().unwrap_or('\\0'))".to_string(),
        TypeRef::String if sanitized => {
            // Sanitized String field: the core type is a structured type (e.g., ConversionOptions)
            // serialized as JSON. Deserialize instead of using .into().
            "serde_json::from_str(&v).unwrap_or_default()".to_string()
        }
        // Vec<T>: the core field may be a Set (HashSet, AHashSet, BTreeSet) which has no
        // `From<Vec<T>>` impl. Leave `collect()` target-inferred from the core field so
        // Vec and set assignments both compile.
        TypeRef::Vec(_) => "v.into_iter().collect()".to_string(),
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Vec(_)) => {
            "v.map(|items| items.into_iter().collect())".to_string()
        }
        _ => "v.into()".to_string(),
    }
}

/// Format a doc string as rustdoc comment lines.
///
/// Returns an empty string when `doc` is empty, otherwise returns each line
/// prefixed with `/// ` and terminated with a newline, ready to prepend to an item.
///
/// Sanitizes Rust idioms (Option<T>, Vec<T>, ::, Some(), None, intra-doc links, etc.)
/// to be TS-doc idiomatic before emitting.
pub(super) fn emit_rustdoc(doc: &str) -> String {
    if doc.is_empty() {
        return String::new();
    }
    let sanitized =
        crate::codegen::doc_emission::sanitize_rust_idioms(doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::backends::wasm::template_env::render(
        "rustdoc",
        minijinja::context! {
            lines => sanitized.lines().collect::<Vec<_>>(),
        },
    )
}

/// Convert a `TypeRef` to its concrete Rust type string for use in serde deserialization
/// let-bindings. Unlike `WasmMapper::map_type`, this always returns a concrete Rust type
/// (e.g. `String`, `Vec<String>`) rather than `JsValue`. Used when emitting
/// `serde_wasm_bindgen::from_value::<T>(jsval)?` where T must be a concrete type.
pub(super) fn typeref_to_core_type_str(ty: &TypeRef) -> String {
    use crate::core::ir::PrimitiveType;
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_string(),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "bool".to_string(),
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::U32 => "u32".to_string(),
            PrimitiveType::U64 => "u64".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::I32 => "i32".to_string(),
            PrimitiveType::I64 => "i64".to_string(),
            PrimitiveType::F32 => "f32".to_string(),
            PrimitiveType::F64 => "f64".to_string(),
            PrimitiveType::Usize => "usize".to_string(),
            PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::Vec(inner) => format!("Vec<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Optional(inner) => format!("Option<{}>", typeref_to_core_type_str(inner)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            typeref_to_core_type_str(k),
            typeref_to_core_type_str(v)
        ),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Named(n) => n.to_string(),
        TypeRef::Unit => "()".to_string(),
    }
}

/// Helper: format a parameter, prefixing with _ if unused
pub(super) fn format_param_unused(name: &str, ty: &str, unused: bool) -> String {
    let prefix = if unused { "_" } else { "" };
    format!("{}{}: {}", prefix, name, ty)
}

/// Returns a type name in turbofish form for use before `::from(expr)`.
///
/// Rust requires turbofish when a type has generic parameters and sits before `::`:
///   `Vec<T>::from(x)` is a syntax error — `Vec::<T>::from(x)` is required.
/// Non-generic type names are returned unchanged.
fn to_turbofish_from(type_name: &str) -> String {
    if let Some(idx) = type_name.find('<') {
        format!("{}::{}", &type_name[..idx], &type_name[idx..])
    } else {
        type_name.to_string()
    }
}

/// Generate a free function binding with deduplication of input DTOs.
/// Returns a string containing any generated Input DTO structs (not in emitted_input_dtos set)
/// followed by the function code.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_function_with_emitted_dtos(
    func: &FunctionDef,
    mapper: &WasmMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    mutex_types: &AHashSet<String>,
    api: &crate::core::ir::ApiSurface,
    emitted_input_dtos: &AHashSet<String>,
) -> String {
    // Collect any Input DTOs needed for config-like parameters
    let mut input_dtos = String::new();
    let mut input_dto_names: HashMap<String, String> = HashMap::new();

    for p in &func.params {
        if let TypeRef::Named(name) = &p.ty {
            if !opaque_types.contains(name.as_str()) {
                // Find the TypeDef for this named type
                if let Some(type_def) = api.types.iter().find(|t| t.name == *name)
                    && should_have_input_dto(type_def)
                {
                    // Skip if already emitted (dedup)
                    if emitted_input_dtos.contains(name) {
                        input_dto_names.insert(name.clone(), format!("{}Input", name));
                        continue;
                    }
                    let (dto_code, dto_name) = gen_input_dto_for_type(name, core_import, type_def);
                    if !dto_code.is_empty() {
                        input_dtos.push_str(&dto_code);
                        input_dtos.push_str("\n\n");
                        input_dto_names.insert(name.clone(), dto_name);
                    }
                }
            }
        }
    }

    let can_delegate = crate::codegen::shared::can_auto_delegate_function(func, opaque_types);

    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !func.is_async)
        })
        .collect();

    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut attrs = emit_rustdoc(&func.doc);
    // Per-item clippy suppression: too_many_arguments when >7 params
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    if func.is_async {
        // For async functions with named params, use JsValue parameters to avoid _assertClass errors
        let has_named = crate::codegen::generators::has_named_params(&func.params, opaque_types);

        let async_params: Vec<String> = if has_named {
            func.params
                .iter()
                .map(|p| match &p.ty {
                    TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                        let mapped_ty = if p.optional {
                            "Option<JsValue>".to_string()
                        } else {
                            "JsValue".to_string()
                        };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                    _ => {
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                })
                .collect()
        } else {
            params.clone()
        };

        // Generate serde deserialization let-bindings for named non-opaque params
        let mut serde_bindings = String::new();
        if has_named {
            for p in &func.params {
                if let TypeRef::Named(name) = &p.ty {
                    if !opaque_types.contains(name.as_str()) {
                        let core_path = format!("{}::{}", core_import, name);
                        let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            let has_default = type_has_default(name, api);
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    has_default => has_default,
                                    is_mut => p.is_mut,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    }
                } else if let TypeRef::Vec(inner) = &p.ty
                    && let TypeRef::Named(name) = inner.as_ref()
                    && !opaque_types.contains(name.as_str())
                {
                    let core_path = format!("{}::{}", core_import, name);
                    if p.optional {
                        serde_bindings.push_str(&format!(
                            "let {name}_core: Option<Vec<{core_path}>> = {name}.map(|values| values.into_iter().map(Into::into).collect());\n    ",
                            name = p.name
                        ));
                    } else {
                        serde_bindings.push_str(&format!(
                            "let {name}_core: Vec<{core_path}> = {name}.into_iter().map(Into::into).collect();\n    ",
                            name = p.name
                        ));
                    }
                }
            }
        }

        let let_bindings = serde_bindings;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        // Build the return expression: handle Vec<Named> with collect pattern (turbofish),
        // plain Named with From::from, and everything else as passthrough.
        let return_expr = match &func.return_type {
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if mutex_types.contains(n.as_str()) {
                        format!(
                            "result.into_iter().map(|v| {} {{ inner: Arc::new(std::sync::Mutex::new(v)) }}).collect::<Vec<_>>()",
                            mapper.map_type(inner)
                        )
                    } else {
                        format!(
                            "result.into_iter().map(|v| {} {{ inner: Arc::new(v) }}).collect::<Vec<_>>()",
                            mapper.map_type(inner)
                        )
                    }
                }
                TypeRef::Named(_) => {
                    let inner_mapped = mapper.map_type(inner);
                    format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
                }
                _ => "result".to_string(),
            },
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let prefixed = mapper.map_type(&func.return_type);
                if mutex_types.contains(n.as_str()) {
                    format!("{prefixed} {{ inner: Arc::new(std::sync::Mutex::new(result)) }}")
                } else {
                    format!("{prefixed} {{ inner: Arc::new(result) }}")
                }
            }
            TypeRef::Named(_) => {
                format!("{}::from(result)", to_turbofish_from(&return_type))
            }
            TypeRef::Unit => "result".to_string(),
            _ => "result".to_string(),
        };
        let body = if func.error_type.is_some() {
            format!(
                "{let_bindings}let result = {core_call}.await\n        \
                 .map_err(|e| JsValue::from_str(&e.to_string()))?;\n    \
                 Ok({return_expr})"
            )
        } else {
            format!(
                "{let_bindings}let result = {core_call}.await;\n    \
                 {return_expr}"
            )
        };
        let fn_code = format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub async fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            async_params.join(", "),
            return_annotation
        );
        format!("{input_dtos}{fn_code}")
    } else if can_delegate {
        let mut let_bindings = if crate::codegen::generators::has_named_params(&func.params, opaque_types) {
            crate::codegen::generators::gen_named_let_bindings_no_promote(&func.params, opaque_types, core_import)
        } else {
            String::new()
        };
        // Nested Vec params (e.g. Vec<Vec<String>>) arrive as JsValue because wasm-bindgen
        // cannot pass them across the boundary directly. Emit a deserialization shadowing
        // binding so the core call sees a real `Vec<Vec<T>>`.
        let needs_result_wrap = func
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Vec(outer) if matches!(outer.as_ref(), TypeRef::Vec(_))))
            && func.error_type.is_none();
        for p in &func.params {
            if let TypeRef::Vec(outer_inner) = &p.ty
                && matches!(outer_inner.as_ref(), TypeRef::Vec(_))
            {
                let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                    typeref_to_core_type_str(elem.as_ref())
                } else {
                    "String".to_string()
                };
                let core_ty = format!("Vec<Vec<{elem_ty}>>");
                if p.optional {
                    let err_conv = format!(".expect(\"deserialize {}\")", p.name);
                    let_bindings.push_str(&crate::backends::wasm::template_env::render(
                        "serde_vec_nested_optional",
                        minijinja::context! {
                            param_name => &p.name,
                            core_ty => &core_ty,
                            err_conv => &err_conv,
                        },
                    ));
                    let_bindings.push_str("    ");
                } else {
                    let err_conv = format!(".expect(\"deserialize {}\")", p.name);
                    let_bindings.push_str(&crate::backends::wasm::template_env::render(
                        "serde_vec_nested_required",
                        minijinja::context! {
                            param_name => &p.name,
                            core_ty => &core_ty,
                            err_conv => &err_conv,
                        },
                    ));
                    let_bindings.push_str("    ");
                }
            }
        }
        let _ = needs_result_wrap;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&func.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let body = if func.error_type.is_some() {
            let wrap = wasm_wrap_return_fn(
                "result",
                &func.return_type,
                opaque_types,
                func.returns_ref,
                func.returns_cow,
                prefix,
                mutex_types,
            );
            format!(
                "{let_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        } else {
            format!(
                "{let_bindings}{}",
                wasm_wrap_return_fn(
                    &core_call,
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    func.returns_cow,
                    prefix,
                    mutex_types
                )
            )
        };
        let fn_code = format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        );
        format!("{input_dtos}{fn_code}")
    } else if func.error_type.is_some()
        && (func.sanitized || crate::codegen::generators::has_named_params(&func.params, opaque_types))
    {
        // Serde recovery: accept Named non-opaque params as JsValue and deserialize
        // to core types via serde_wasm_bindgen. Also handles sanitized functions (Vec<tuple>).
        // WASM binding structs don't derive Serialize/Deserialize, so we can't round-trip
        // through the binding type; instead we accept raw JsValue/Vec<String> from JS and
        // deserialize directly to core types.
        let serde_params: Vec<String> = func
            .params
            .iter()
            .map(|p| match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    // Accept as JsValue so serde_wasm_bindgen::from_value can deserialize
                    let mapped_ty = if p.optional {
                        "Option<JsValue>".to_string()
                    } else {
                        "JsValue".to_string()
                    };
                    format!("{}: {}", p.name, mapped_ty)
                }
                TypeRef::Vec(inner) => {
                    // Sanitized Vec<tuple>: accept Vec<String> (JSON encoded)
                    if matches!(inner.as_ref(), TypeRef::Named(_)) {
                        if p.optional {
                            format!("{}: Option<Vec<String>>", p.name)
                        } else {
                            format!("{}: Vec<String>", p.name)
                        }
                    } else {
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                }
                _ => {
                    let ty = mapper.map_type(&p.ty);
                    let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                    format!("{}: {}", p.name, mapped_ty)
                }
            })
            .collect();

        // Generate serde_wasm_bindgen::from_value let-bindings for Named non-opaque params
        // and Vec<String> with is_ref=true (needs texts_refs intermediate)
        let mut serde_bindings = String::new();
        for p in &func.params {
            match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    let core_path = format!("{}::{}", core_import, name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";

                    // Check if this is a config-like type that needs camelCase conversion
                    if api
                        .types
                        .iter()
                        .find(|t| t.name == *name)
                        .is_some_and(should_have_input_dto)
                    {
                        // Use the Input DTO for deserialization with camelCase support
                        let input_dto_type = input_dto_names
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| format!("{}Input", name));
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_config_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    input_dto_type => &input_dto_type,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            let has_default = type_has_default(name, api);
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_config_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    input_dto_type => &input_dto_type,
                                    has_default => has_default,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    } else {
                        // Regular named type deserialization
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_optional",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        } else {
                            let has_default = type_has_default(name, api);
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    has_default => has_default,
                                    is_mut => p.is_mut,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                    // Sanitized Vec<tuple>: deserialize from Vec<String> JSON
                    let inner_name = match inner.as_ref() {
                        TypeRef::Named(n) => n,
                        _ => "UnknownTuple",
                    };
                    let core_path = format!("{}::{}", core_import, inner_name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_named_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_named_required",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char)
                        && p.sanitized
                        && p.original_type.is_some() =>
                {
                    // Sanitized Vec<tuple>: binding accepts Vec<String> (JSON-encoded tuple items).
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_tuple_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_tuple_required",
                            minijinja::context! {
                                param_name => &p.name,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref => {
                    // Vec<String> with is_ref=true: core expects &[&str].
                    // gen_call_args_with_let_bindings emits `&{name}_refs`, so we must create
                    // the intermediate Vec<&str> binding here.
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_string_refs_optional",
                            minijinja::context! {
                                param_name => &p.name,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_string_refs_required",
                            minijinja::context! {
                                param_name => &p.name,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                TypeRef::Vec(outer_inner) if matches!(outer_inner.as_ref(), TypeRef::Vec(_)) => {
                    // Nested Vec (e.g. Vec<Vec<String>>): wasm-bindgen cannot pass this across
                    // the boundary directly, so the param arrives as JsValue. Deserialize via
                    // serde_wasm_bindgen and shadow the original binding so gen_call_args can
                    // still reference the parameter by its original name.
                    let elem_ty = if let TypeRef::Vec(elem) = outer_inner.as_ref() {
                        typeref_to_core_type_str(elem.as_ref())
                    } else {
                        "String".to_string()
                    };
                    let core_ty = format!("Vec<Vec<{elem_ty}>>");
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_nested_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                core_ty => &core_ty,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_vec_nested_required",
                            minijinja::context! {
                                param_name => &p.name,
                                core_ty => &core_ty,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
                _ => {}
            }
        }

        let call_args = wasm_serde_recovery_call_args(&func.params, opaque_types);
        let core_call = format!("{core_fn_path}({call_args})");
        let wrap = wasm_wrap_return_fn(
            "result",
            &func.return_type,
            opaque_types,
            func.returns_ref,
            func.returns_cow,
            prefix,
            mutex_types,
        );
        let body = if matches!(func.return_type, TypeRef::Unit) {
            format!("{serde_bindings}{core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok(())")
        } else {
            format!(
                "{serde_bindings}let result = {core_call}.map_err(|e| JsValue::from_str(&e.to_string()))?;\n    Ok({wrap})"
            )
        };
        let fn_code = format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            serde_params.join(", "),
            return_annotation
        );
        format!("{input_dtos}{fn_code}")
    } else {
        let body = gen_wasm_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some());
        let fn_code = format!(
            "{attrs}#[wasm_bindgen{js_name_attr}]\npub fn {}({}) -> {} {{\n    \
             {body}\n}}",
            func.name,
            params.join(", "),
            return_annotation
        );
        format!("{input_dtos}{fn_code}")
    }
}

fn wasm_serde_recovery_call_args(params: &[crate::core::ir::ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && !p.optional =>
            {
                format!("&{}_refs", p.name)
            }
            _ => generators::gen_call_args_with_let_bindings(std::slice::from_ref(p), opaque_types),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate WASM environment shims for wide-character C functions used by external scanners.
///
/// Some sample_language external scanners call C wide-character functions (`iswspace`, `iswalnum`,
/// etc.) that are not available in the WASM runtime. This emits `#[unsafe(no_mangle)] extern "C"`
/// shims that satisfy those link-time references using Rust's Unicode-aware char APIs.
///
/// Only shims whose names appear in `shim_names` are emitted.
pub(super) fn gen_env_shims(shim_names: &[String]) -> String {
    let mut out = String::from("// WASM environment shims for C scanner interop\n");

    for name in shim_names {
        let shim = match name.as_str() {
            "iswspace" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswspace(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_whitespace() as i32)\n",
                "}\n",
            ),
            "iswalnum" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalnum(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphanumeric() as i32)\n",
                "}\n",
            ),
            "towupper" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn towupper(c: u32) -> u32 {\n",
                "    char::from_u32(c).map_or(c, |ch| ch.to_uppercase().next().unwrap_or(ch) as u32)\n",
                "}\n",
            ),
            "iswalpha" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswalpha(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_alphabetic() as i32)\n",
                "}\n",
            ),
            "iswlower" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswlower(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_lowercase() as i32)\n",
                "}\n",
            ),
            "iswupper" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswupper(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_uppercase() as i32)\n",
                "}\n",
            ),
            "iswxdigit" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn iswxdigit(c: u32) -> i32 {\n",
                "    char::from_u32(c).map_or(0, |ch| ch.is_ascii_hexdigit() as i32)\n",
                "}\n",
            ),
            "towlower" => concat!(
                "#[unsafe(no_mangle)]\n",
                "pub extern \"C\" fn towlower(c: u32) -> u32 {\n",
                "    char::from_u32(c).map_or(c, |ch| ch.to_lowercase().next().unwrap_or(ch) as u32)\n",
                "}\n",
            ),
            "memchr" => concat!(
                "/// # Safety\n",
                "/// Caller must ensure `s` points to a buffer of at least `n` bytes.\n",
                "#[unsafe(no_mangle)]\n",
                "pub unsafe extern \"C\" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8 {\n",
                "    if s.is_null() { return core::ptr::null(); }\n",
                "    let needle = c as u8;\n",
                "    let slice = unsafe { core::slice::from_raw_parts(s, n) };\n",
                "    match slice.iter().position(|&b| b == needle) {\n",
                "        Some(idx) => unsafe { s.add(idx) },\n",
                "        None => core::ptr::null(),\n",
                "    }\n",
                "}\n",
            ),
            "strcmp" => concat!(
                "/// # Safety\n",
                "/// Caller must ensure both pointers are valid null-terminated C strings.\n",
                "#[unsafe(no_mangle)]\n",
                "pub unsafe extern \"C\" fn strcmp(a: *const u8, b: *const u8) -> i32 {\n",
                "    if a.is_null() || b.is_null() { return 0; }\n",
                "    let mut i = 0isize;\n",
                "    loop {\n",
                "        let ca = unsafe { *a.offset(i) };\n",
                "        let cb = unsafe { *b.offset(i) };\n",
                "        if ca != cb { return (ca as i32) - (cb as i32); }\n",
                "        if ca == 0 { return 0; }\n",
                "        i += 1;\n",
                "    }\n",
                "}\n",
            ),
            _ => continue,
        };
        out.push_str(shim);
    }

    // Trim trailing newline so the builder adds consistent spacing
    out.trim_end_matches('\n').to_string()
}

/// Generate a type-appropriate unimplemented body for WASM (no todo!()).
pub(super) fn gen_wasm_unimplemented_body(return_type: &TypeRef, fn_name: &str, has_error: bool) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(JsValue::from_str(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                crate::core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!("panic!(\"alef: {fn_name} not auto-delegatable\")"),
        }
    }
}

/// Detect whether the core-call expression already evaluates to `Arc<T>` for the
/// binding's `inner` field. Mirrors `expr_is_already_arc` in `alef-codegen`.
fn wasm_expr_is_already_arc(expr: &str) -> bool {
    let trimmed = expr.trim();
    trimmed == "self.inner"
        || trimmed == "self.inner.clone()"
        || trimmed.starts_with("self.inner.as_ref()")
        || trimmed.starts_with("self.inner.clone()")
}

/// WASM-specific return wrapping for opaque methods (adds prefix for opaque Named returns).
#[allow(clippy::too_many_arguments)]
pub(super) fn wasm_wrap_return(
    expr: &str,
    return_type: &TypeRef,
    type_name: &str,
    opaque_types: &AHashSet<String>,
    self_is_opaque: bool,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        // Self-returning opaque method
        TypeRef::Named(n) if n == type_name && self_is_opaque => {
            if wasm_expr_is_already_arc(expr) {
                format!("Self {{ inner: {expr} }}")
            } else if mutex_types.contains(type_name) {
                generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                )
            } else if returns_ref {
                format!("Self {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("Self {{ inner: Arc::new({expr}) }}")
            }
        }
        // Other opaque Named return: needs prefix
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if wasm_expr_is_already_arc(expr) {
                format!("{prefix}{n} {{ inner: {expr} }}")
            } else if mutex_types.contains(n.as_str()) {
                // wrap_return_with_mutex uses IdentityMapper, returns "{n} { inner: ... }"
                let wrapped = generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    type_name,
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                );
                // wrapped is "{n} { inner: ... }", add prefix: "{prefix}{n} { inner: ... }"
                if wrapped.starts_with(&format!("{n} {{")) {
                    format!("{prefix}{}{}", n, &wrapped[n.len()..])
                } else {
                    wrapped
                }
            } else if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        // Optional<opaque>: wrap with prefix
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        type_name,
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.map(|v| {prefix}{name} {{ {wrap_inner} }})")
                } else if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        // Vec<opaque>: wrap with prefix
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        type_name,
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ {wrap_inner} }}).collect()")
                } else if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            _ => generators::wrap_return(
                expr,
                return_type,
                type_name,
                opaque_types,
                self_is_opaque,
                returns_ref,
                returns_cow,
            ),
        },
        _ => generators::wrap_return(
            expr,
            return_type,
            type_name,
            opaque_types,
            self_is_opaque,
            returns_ref,
            returns_cow,
        ),
    }
}

/// WASM-specific return wrapping for free functions (no type_name context, adds prefix).
pub(super) fn wasm_wrap_return_fn(
    expr: &str,
    return_type: &TypeRef,
    opaque_types: &AHashSet<String>,
    returns_ref: bool,
    returns_cow: bool,
    prefix: &str,
    mutex_types: &AHashSet<String>,
) -> String {
    match return_type {
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            if wasm_expr_is_already_arc(expr) {
                format!("{prefix}{n} {{ inner: {expr} }}")
            } else if mutex_types.contains(n.as_str()) {
                // wrap_return_with_mutex with empty type_name uses IdentityMapper,
                // so it returns "{n} { inner: ... }" without prefix — add it manually
                let wrapped = generators::wrap_return_with_mutex(
                    expr,
                    return_type,
                    "",
                    opaque_types,
                    mutex_types,
                    true,
                    returns_ref,
                    returns_cow,
                );
                // wrapped is "{n} { inner: ... }", replace "{n}" with "{prefix}{n}"
                if wrapped.starts_with(&format!("{n} {{")) {
                    format!("{prefix}{}{}", n, &wrapped[n.len()..])
                } else {
                    wrapped
                }
            } else if returns_ref {
                format!("{prefix}{n} {{ inner: Arc::new({expr}.clone()) }}")
            } else {
                format!("{prefix}{n} {{ inner: Arc::new({expr}) }}")
            }
        }
        TypeRef::Named(_) => {
            if returns_cow {
                format!("{expr}.into_owned().into()")
            } else if returns_ref {
                format!("{expr}.clone().into()")
            } else {
                format!("{expr}.into()")
            }
        }
        TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
            if returns_cow && matches!(return_type, TypeRef::Bytes) {
                // Cow<[u8]> needs .into_owned() to become Vec<u8>
                format!("{expr}.into_owned()")
            } else if returns_ref {
                format!("{expr}.into()")
            } else {
                expr.to_string()
            }
        }
        TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
        TypeRef::Json => format!("{expr}.to_string()"),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        "",
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.map(|v| {prefix}{name} {{ {wrap_inner} }})")
                } else if returns_ref {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }})")
                } else {
                    format!("{expr}.map(|v| {prefix}{name} {{ inner: Arc::new(v) }})")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    format!("{expr}.map(|v| v.clone().into())")
                } else {
                    format!("{expr}.map(Into::into)")
                }
            }
            TypeRef::Path => {
                format!("{expr}.map(Into::into)")
            }
            TypeRef::String | TypeRef::Char | TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.map(Into::into)")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                if mutex_types.contains(name.as_str()) {
                    let wrap_inner = generators::wrap_return_with_mutex(
                        "v",
                        inner.as_ref(),
                        "",
                        opaque_types,
                        mutex_types,
                        true,
                        returns_ref,
                        returns_cow,
                    );
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ {wrap_inner} }}).collect()")
                } else if returns_ref {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v.clone()) }}).collect()")
                } else {
                    format!("{expr}.into_iter().map(|v| {prefix}{name} {{ inner: Arc::new(v) }}).collect()")
                }
            }
            TypeRef::Named(_) => {
                if returns_ref {
                    // `&[T]` → `Vec<U>`: use `.iter()` not `.into_iter()` to
                    // avoid clippy::into_iter_on_ref under -D warnings.
                    format!("{expr}.iter().map(|v| v.clone().into()).collect()")
                } else {
                    format!("{expr}.into_iter().map(Into::into).collect()")
                }
            }
            TypeRef::Path => {
                format!("{expr}.into_iter().map(Into::into).collect()")
            }
            TypeRef::String | TypeRef::Char => {
                if returns_ref {
                    // `&[&str]` → `Vec<String>`. `Into::into` would need
                    // `impl From<&&str> for String`, which doesn't exist.
                    format!("{expr}.iter().map(|s| s.to_string()).collect()")
                } else {
                    expr.to_string()
                }
            }
            TypeRef::Bytes => {
                if returns_ref {
                    format!("{expr}.iter().map(|b| b.to_vec()).collect()")
                } else {
                    expr.to_string()
                }
            }
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Lookup whether a named type has `Default` impl in the IR.
/// Returns true if the type is found and `has_default` is true, false otherwise.
fn type_has_default(type_name: &str, api: &ApiSurface) -> bool {
    api.types
        .iter()
        .find(|t| t.name == type_name)
        .map(|t| t.has_default)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ParamDef, TypeRef};
    use std::collections::HashMap;

    fn param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
        }
    }

    fn async_function(params: Vec<ParamDef>) -> FunctionDef {
        FunctionDef {
            name: "interact".to_string(),
            rust_path: "sample_crawler::interact".to_string(),
            original_rust_path: String::new(),
            params,
            return_type: TypeRef::Unit,
            is_async: true,
            error_type: Some("CrawlError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    #[test]
    fn gen_env_shims_emits_expected_signatures_for_all_supported_names() {
        let names: Vec<String> = [
            "iswspace",
            "iswalnum",
            "towupper",
            "iswalpha",
            "iswlower",
            "iswupper",
            "iswxdigit",
            "towlower",
            "memchr",
            "strcmp",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

        let out = gen_env_shims(&names);

        // Each shim must carry the no_mangle attribute exactly once.
        assert_eq!(out.matches("#[unsafe(no_mangle)]").count(), names.len(), "{out}");

        // Wide-char predicates: c: u32 -> i32
        for name in ["iswspace", "iswalnum", "iswalpha", "iswlower", "iswupper", "iswxdigit"] {
            let sig = format!("pub extern \"C\" fn {name}(c: u32) -> i32");
            assert!(out.contains(&sig), "missing signature `{sig}` in:\n{out}");
        }

        // Wide-char conversions: c: u32 -> u32
        for name in ["towupper", "towlower"] {
            let sig = format!("pub extern \"C\" fn {name}(c: u32) -> u32");
            assert!(out.contains(&sig), "missing signature `{sig}` in:\n{out}");
        }

        // Unsafe C-string / memory ops.
        assert!(
            out.contains("pub unsafe extern \"C\" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8"),
            "{out}"
        );
        assert!(
            out.contains("pub unsafe extern \"C\" fn strcmp(a: *const u8, b: *const u8) -> i32"),
            "{out}"
        );
    }

    #[test]
    fn gen_env_shims_ignores_unknown_names() {
        let names = vec!["not_a_real_shim".to_string()];
        let out = gen_env_shims(&names);
        assert!(!out.contains("#[unsafe(no_mangle)]"), "{out}");
    }

    #[test]
    fn async_vec_named_params_convert_to_core_vec() {
        let mapper = WasmMapper::new(HashMap::new(), "Wasm".to_string());
        let func = async_function(vec![param(
            "actions",
            TypeRef::Vec(Box::new(TypeRef::Named("PageAction".to_string()))),
        )]);
        let api = crate::core::ir::ApiSurface {
            crate_name: "sample_crawler".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: HashMap::new(),
            excluded_trait_names: std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };

        let out = gen_function_with_emitted_dtos(
            &func,
            &mapper,
            "sample_crawler",
            &AHashSet::new(),
            "Wasm",
            &AHashSet::new(),
            &api,
            &AHashSet::new(),
        );

        assert!(out.contains("actions: Vec<WasmPageAction>"));
        assert!(
            out.contains(
                "let actions_core: Vec<sample_crawler::PageAction> = actions.into_iter().map(Into::into).collect();"
            ),
            "{out}"
        );
        assert!(out.contains("sample_crawler::interact(actions_core).await"), "{out}");
    }

    #[test]
    fn input_dtos_dedup_flag_skips_generation() {
        // Bug 1 fix: gen_function_with_emitted_dtos accepts a set of already-emitted DTOs
        // and skips re-generating them. When a config type is in the emitted set,
        // it should not be generated again.
        let _emitted_dtos: AHashSet<String> = ["OcrConfig".to_string()].iter().cloned().collect();
        use crate::core::ir::{CoreWrapper, FieldDef, PrimitiveType, TypeDef};

        let make_type = |name: &str, field_name: &str, has_default: bool, has_serde: bool| TypeDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                name: field_name.to_string(),
                ty: TypeRef::Primitive(PrimitiveType::U32),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            doc: String::new(),
            cfg: None,
            is_trait: false,
            has_default,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde,
            super_traits: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };

        assert!(should_have_input_dto(&make_type("OcrOptions", "max_depth", true, true)));
        assert!(!should_have_input_dto(&make_type("OcrConfig", "depth", true, true)));
        assert!(!should_have_input_dto(&make_type(
            "ExtractionOptions",
            "max_depth",
            false,
            true
        )));
        assert!(!should_have_input_dto(&make_type(
            "ExtractionOptions",
            "max_depth",
            true,
            false
        )));
    }

    #[test]
    fn vec_vec_string_collect_has_explicit_type() {
        // Bug 2 fix: when converting Vec<(String, String)> to Vec<Vec<String>>,
        // the .collect() must have an explicit type ascription so Rust can infer
        // the target type even when assigned to JsValue fields.
        use crate::codegen::conversions::field_conversion_from_core;

        // Test the conversion code for Vec<Vec<String>> (sanitized from Vec<(String, String)>)
        let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        let conv = field_conversion_from_core("attributes", &ty, false, true, &AHashSet::new());

        // The conversion must include an explicit type on collect()
        assert!(
            conv.contains("collect::<Vec<Vec<String>>>"),
            "collect() must have explicit type ascription for Vec<Vec<String>>: {conv}"
        );

        // Test optional variant
        let ty_opt = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(
            TypeRef::String,
        ))))));
        let conv_opt = field_conversion_from_core("attributes", &ty_opt, true, true, &AHashSet::new());
        assert!(
            conv_opt.contains("collect::<Vec<Vec<String>>>"),
            "optional variant must also have explicit type: {conv_opt}"
        );
    }

    #[test]
    fn sanitized_string_field_uses_json_deserialize() {
        // Bug fix: when a field is sanitized to Option<String> (e.g., ConversionOptions,
        // ConcurrencyConfig, CancellationToken), the From impl must JSON-deserialize
        // instead of using .into() (which has no impl for these structured types).
        let ty_string = TypeRef::String;

        // Non-sanitized String field: use .into()
        let conv_normal = dto_field_conversion(&ty_string, false);
        assert_eq!(conv_normal, "v.into()", "non-sanitized String should use .into()");

        // Sanitized String field: use JSON deserialization
        let conv_sanitized = dto_field_conversion(&ty_string, true);
        assert_eq!(
            conv_sanitized, "serde_json::from_str(&v).unwrap_or_default()",
            "sanitized String should use JSON deserialization: {conv_sanitized}"
        );
    }

    #[test]
    fn dto_vec_field_conversion_uses_target_inferred_collect() {
        // Regression: WASM input DTOs deserialize sequence-shaped fields as Vec<T>, but
        // the core field may be Vec<T> or a set-like collection. Wrapping collect() in
        // Into::into is ambiguous for Vec<T>; forcing collect::<Vec<_>>() fails for sets.
        let ty = TypeRef::Vec(Box::new(TypeRef::String));

        let conv = dto_field_conversion(&ty, false);

        assert_eq!(conv, "v.into_iter().collect()");
        assert!(
            !conv.contains("collect::<Vec<_>>()"),
            "collection target must be inferred from the core field: {conv}"
        );
        assert!(
            !conv.contains("Into::into"),
            "plain Vec fields must not wrap target-inferred collect in Into::into: {conv}"
        );
    }

    #[test]
    fn dto_optional_vec_field_conversion_uses_target_inferred_collect() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));

        let conv = dto_field_conversion(&ty, false);

        assert_eq!(conv, "v.map(|items| items.into_iter().collect())");
        assert!(
            !conv.contains("collect::<Vec<_>>()"),
            "optional collection target must be inferred from the core field: {conv}"
        );
    }

    #[test]
    fn gen_input_dto_excludes_binding_excluded_fields() {
        // Regression: gen_input_dto_for_type previously iterated type_def.fields
        // directly without filtering binding_excluded fields, causing trait-object
        // and other non-marshalable fields to appear in the generated Input DTO.
        // The generated From impl then emitted serde_json::from_str into the trait
        // object, producing uncompilable Rust in consumer wasm bindings.
        use crate::core::ir::{CoreWrapper, FieldDef};

        let make_field = |name: &str, ty: TypeRef, binding_excluded: bool, sanitized: bool| FieldDef {
            name: name.to_string(),
            ty,
            optional: true,
            default: None,
            doc: String::new(),
            sanitized,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded,
            binding_exclusion_reason: None,
            original_type: None,
        };

        let type_def = crate::core::ir::TypeDef {
            name: "CrawlConfig".to_string(),
            rust_path: "sample_crawler::CrawlConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                // Normal field — must appear in the DTO.
                make_field(
                    "max_depth",
                    TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
                    false,
                    false,
                ),
                // binding_excluded trait-object field — must NOT appear in the DTO.
                make_field("bypass", TypeRef::String, true, true),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };

        let (code, _name) = gen_input_dto_for_type("CrawlConfig", "sample_crawler", &type_def);

        assert!(
            code.contains("max_depth"),
            "normal field must appear in input DTO: {code}"
        );
        assert!(
            !code.contains("bypass"),
            "binding_excluded field must not appear in input DTO: {code}"
        );
    }

    #[test]
    fn feature_gated_fields_get_cfg_guards() {
        // Regression test: gen_input_dto_for_type_with_cfg should emit #[cfg(...)]
        // guards on fields whose type is only available when certain features are enabled.
        // This prevents generating bindings that reference non-existent types.
        use crate::core::ir::{CoreWrapper, FieldDef};

        let make_field = |name: &str, ty: TypeRef, cfg: Option<String>| FieldDef {
            name: name.to_string(),
            ty,
            optional: true,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            original_type: None,
        };

        let type_def = crate::core::ir::TypeDef {
            name: "ExtractionConfig".to_string(),
            rust_path: "mylib::ExtractionConfig".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                // Always-enabled field
                make_field(
                    "enabled",
                    TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
                    None,
                ),
                // Feature-gated field: only available when "layout" feature is enabled
                make_field(
                    "layout_config",
                    TypeRef::Named("LayoutDetectionConfig".to_string()),
                    Some("feature = \"layout\"".to_string()),
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
        };

        // Generate without the "layout" feature enabled
        let (code_no_layout, _) = gen_input_dto_for_type_with_cfg(
            "ExtractionConfig",
            "mylib",
            &type_def,
            &[],                        // No excluded types
            &["streaming".to_string()], // Only "streaming" is enabled, NOT "layout"
            &std::collections::HashSet::new(),
        );

        // The layout_config field should have a cfg guard since "layout" is not enabled
        assert!(
            code_no_layout.contains("#[cfg(feature = \"layout\")]"),
            "Feature-gated field should have #[cfg] guard when feature not enabled: {}",
            code_no_layout
        );
        // It should also have #[serde(skip)] since it's not deserializable without the feature
        assert!(
            code_no_layout.contains("#[serde(skip)]"),
            "Feature-gated field should have #[serde(skip)]: {}",
            code_no_layout
        );

        // Generate WITH the "layout" feature enabled
        let (code_with_layout, _) = gen_input_dto_for_type_with_cfg(
            "ExtractionConfig",
            "mylib",
            &type_def,
            &[],                     // No excluded types
            &["layout".to_string()], // "layout" IS enabled
            &std::collections::HashSet::new(),
        );

        // Now the layout_config field should NOT be skipped (cfg is satisfied)
        assert!(
            !code_with_layout.contains("layout_config: {{ field.ty }},\n{%- endfor %}"),
            "When feature is enabled, field should not be skipped: {}",
            code_with_layout
        );
        // It should still have the cfg guard for extra safety
        assert!(
            code_with_layout.contains("#[cfg(feature = \"layout\")]"),
            "Field should still have cfg guard even when enabled: {}",
            code_with_layout
        );
    }

    #[test]
    fn to_turbofish_from_inserts_turbofish_for_generic_type() {
        assert_eq!(to_turbofish_from("Vec<WasmEntity>"), "Vec::<WasmEntity>");
        assert_eq!(to_turbofish_from("Option<WasmFoo>"), "Option::<WasmFoo>");
        assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
        assert_eq!(to_turbofish_from("HashMap<String, i64>"), "HashMap::<String, i64>");
    }

    #[test]
    fn to_turbofish_from_bare_named_type_is_unchanged() {
        // A non-generic type name must pass through unchanged so bare Named returns
        // still produce BareType::from(result), not BareType::::<>::from(result).
        assert_eq!(to_turbofish_from("WasmEntity"), "WasmEntity");
        assert_eq!(to_turbofish_from("ExtractionResult"), "ExtractionResult");
    }

    #[test]
    fn type_has_default_lookup_returns_correct_value() {
        // Regression test for bug #1: types without Default should not emit ::default()
        // in WASM function parameter deserialization templates.
        // This test validates that the type_has_default helper correctly identifies
        // the has_default flag from the IR and handles missing types gracefully.

        use crate::core::ir::ApiSurface;

        // Create a minimal API surface with empty types list
        let api = ApiSurface {
            crate_name: "test".to_string(),
            version: "1.0.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: std::collections::HashMap::new(),
            excluded_trait_names: std::collections::HashSet::new(),
            handler_contracts: vec![],
            services: vec![],
            unsupported_public_items: Vec::new(),
        };

        // type_has_default should return false for unknown types
        assert!(
            !type_has_default("NonExistentType", &api),
            "Unknown type should return false"
        );
        // and for empty API
        assert!(
            !type_has_default("AnyType", &api),
            "Empty API should return false for any type"
        );
    }
}
