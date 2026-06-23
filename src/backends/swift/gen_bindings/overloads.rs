use crate::backends::swift::naming::swift_rust_shim_ident as swift_ident;
use crate::backends::swift::type_map::SwiftMapper;
use crate::core::ir::{ApiSurface, EnumDef, FunctionDef, PrimitiveType, TypeRef};
use heck::ToLowerCamelCase;

pub(super) fn emit_json_string_overloads(
    api: &ApiSurface,
    exclude_types: &std::collections::HashSet<String>,
    out: &mut String,
) {
    use heck::AsSnakeCase;

    let json_overload_candidates: Vec<(&FunctionDef, usize, &str)> = api
        .functions
        .iter()
        .flat_map(|func| {
            if !func.is_async
                && func.name.ends_with("_sync")
                && api
                    .functions
                    .iter()
                    .any(|f| f.is_async && f.name == format!("{}_async", &func.name[..func.name.len() - 5]))
            {
                return vec![];
            }

            // Skip functions whose signature references a type filtered out of the
            // DTO emitter — the matching Rust bridge symbol and Swift type are both
            // absent, so any forwarder referencing them won't compile.
            if super::forwarders::function_references_excluded_type(func, exclude_types) {
                return vec![];
            }

            func.params
                .iter()
                .enumerate()
                .filter_map(move |(idx, param)| {
                    if let TypeRef::Named(type_name) = &param.ty {
                        if let Some(typ) = api.types.iter().find(|t| &t.name == type_name) {
                            if typ.has_serde && !typ.is_opaque {
                                return Some((func, idx, type_name.as_str()));
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if json_overload_candidates.is_empty() {
        return;
    }

    out.push_str("// MARK: - JSON-String Convenience Overloads\n");
    out.push_str("// These overloads accept JSON-encoded config parameters and decode them automatically.\n");
    out.push_str("// Enables e2e tests to pass JSON strings directly without typed config construction.\n\n");

    emit_load_bytes_from_path_or_utf8(out);

    let mut emitted_funcs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut func_to_configs: std::collections::HashMap<String, Vec<(usize, &str)>> = std::collections::HashMap::new();

    for (func, config_param_idx, config_type_name) in &json_overload_candidates {
        func_to_configs
            .entry(func.name.clone())
            .or_default()
            .push((*config_param_idx, *config_type_name));
    }

    for (func, _config_param_idx, _config_type_name) in json_overload_candidates {
        if !emitted_funcs.insert(func.name.clone()) {
            continue;
        }

        if !func.is_async {
            let sync_name = func.name.clone();
            let has_async = api
                .functions
                .iter()
                .any(|f| f.is_async && f.name == format!("{}_async", sync_name));
            if has_async {
                continue;
            }
        }

        let mut config_params = func_to_configs.get(&func.name).cloned().unwrap_or_default();
        config_params.sort_by_key(|(idx, _)| *idx);

        let swift_func_name = swift_ident(&func.name.to_lower_camel_case());

        let mut param_strs: Vec<String> = Vec::new();
        let mut json_local_names: std::collections::HashMap<usize, (String, String)> = std::collections::HashMap::new();

        for (i, param) in func.params.iter().enumerate() {
            let param_name = param.name.to_lower_camel_case();
            let config_json_name = config_params.iter().find(|(idx, _)| *idx == i).map(|(_, ty_name)| {
                let type_snake = AsSnakeCase(ty_name).to_string();
                format!("{type_snake}_from_json").to_lower_camel_case()
            });

            if let Some(json_fn_name) = config_json_name.clone() {
                if config_params.iter().any(|(idx, _)| *idx == i) {
                    let type_var_name = param_name.clone();
                    param_strs.push(format!("_ {type_var_name}Json: String"));
                    json_local_names.insert(i, (json_fn_name, type_var_name));
                }
            } else {
                let ty_str = if param.optional {
                    format!("{}?", swift_type_name(&param.ty))
                } else {
                    swift_type_name(&param.ty)
                };
                param_strs.push(format!("_ {param_name}: {ty_str}"));
            }
        }

        let params_sig = param_strs.join(", ");
        let return_ty = swift_return_type(&func.return_type);
        let async_clause = if func.is_async { " async" } else { "" };
        let throws_clause = " throws";
        let return_suffix = "";

        let mut call_args: Vec<String> = Vec::new();
        for (i, param) in func.params.iter().enumerate() {
            let param_name = param.name.to_lower_camel_case();
            // Check if the original Rust parameter name starts with underscore (positional arg).
            // If so, emit positional arguments (no parameter label) in the call.
            let is_positional = param.name.starts_with('_');

            if is_positional {
                // Emit positional argument (no label)
                if let Some((_, type_var_name)) = json_local_names.get(&i) {
                    call_args.push(type_var_name.clone());
                } else {
                    call_args.push(param_name.clone());
                }
            } else {
                // Emit named argument
                if let Some((_, type_var_name)) = json_local_names.get(&i) {
                    call_args.push(format!("{param_name}: {type_var_name}"));
                } else {
                    call_args.push(format!("{param_name}: {param_name}"));
                }
            }
        }
        let call_args_str = call_args.join(", ");

        let mut decode_lines = String::new();
        let mut sorted_positions: Vec<_> = json_local_names.keys().copied().collect();
        sorted_positions.sort();
        for pos in sorted_positions {
            if let Some((json_fn_name, type_var_name)) = json_local_names.get(&pos) {
                decode_lines.push_str(&crate::backends::swift::template_env::render(
                    "swift_json_decode_line.swift.jinja",
                    minijinja::context! {
                        json_fn_name => json_fn_name,
                        type_var_name => type_var_name,
                    },
                ));
            }
        }
        let await_kw = if func.is_async { "await " } else { "" };
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_json_string_overload.swift.jinja",
            minijinja::context! {
                function_name => &swift_func_name,
                params => &params_sig,
                async_clause => async_clause,
                throws_clause => throws_clause,
                return_type => &return_ty,
                decode_lines => decode_lines,
                await_kw => await_kw,
                call_args => &call_args_str,
                return_suffix => &return_suffix,
            },
        ));
    }
}

pub(super) fn emit_load_bytes_from_path_or_utf8(out: &mut String) {
    out.push_str("/// Resolves a string argument as either a file path or literal UTF-8 content.\n");
    out.push_str("/// Searches: current working directory, ALEF_TEST_DOCUMENTS_DIR env var,\n");
    out.push_str("/// and ancestor `test_documents/` or `fixtures/` directories (up to 16 levels).\n");
    out.push_str("/// If no file is found, treats the string as UTF-8 content and returns its bytes.\n");
    out.push_str("private func _loadBytesFromPathOrUtf8(_ pathOrContent: String) throws -> [UInt8] {\n");
    out.push_str("    let fm = FileManager.default\n");
    out.push_str("    var roots: [String] = [fm.currentDirectoryPath]\n");
    out.push_str("    if let envRoot = ProcessInfo.processInfo.environment[\"ALEF_TEST_DOCUMENTS_DIR\"] {\n");
    out.push_str("        roots.append(envRoot)\n");
    out.push_str("    }\n");
    out.push_str("    var walker = URL(fileURLWithPath: fm.currentDirectoryPath)\n");
    out.push_str("    for _ in 0..<16 {\n");
    out.push_str("        roots.append(walker.appendingPathComponent(\"test_documents\").path)\n");
    out.push_str("        roots.append(walker.appendingPathComponent(\"fixtures\").path)\n");
    out.push_str("        let parent = walker.deletingLastPathComponent()\n");
    out.push_str("        if parent.path == walker.path { break }\n");
    out.push_str("        walker = parent\n");
    out.push_str("    }\n");
    out.push_str(
        "    let candidates = [pathOrContent] + roots.map { ($0 as NSString).appendingPathComponent(pathOrContent) }\n",
    );
    out.push_str("    for path in candidates {\n");
    out.push_str(
        "        if fm.fileExists(atPath: path), let data = try? Data(contentsOf: URL(fileURLWithPath: path)) {\n",
    );
    out.push_str("            return [UInt8](data)\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("    return [UInt8](pathOrContent.utf8)\n");
    out.push_str("}\n\n");
}

pub(super) fn emit_from_json_forwarders(
    api: &ApiSurface,
    exclude_types: &std::collections::HashSet<String>,
    mapper: &SwiftMapper,
    exclude_fields: &std::collections::HashSet<String>,
    known_dto_names: &std::collections::HashSet<String>,
    out: &mut String,
) {
    use heck::AsSnakeCase;

    let struct_candidates: Vec<&str> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde)
        .filter(|t| !exclude_types.contains(&t.name))
        .map(|t| t.name.as_str())
        .collect();

    let enum_candidates: Vec<&str> = api
        .enums
        .iter()
        .filter(|e| e.has_serde && !exclude_types.contains(&e.name))
        .map(|e| e.name.as_str())
        .collect();

    if struct_candidates.is_empty() && enum_candidates.is_empty() {
        return;
    }

    out.push_str("// MARK: - From-JSON Helpers\n");
    out.push_str("// Public helpers that decode JSON into first-class Swift types.\n");
    out.push_str("// First-class struct types (Codable) use JSONDecoder directly.\n");
    out.push_str("// Opaque RustBridge types forward to RustBridge.\n\n");

    let first_class_set: std::collections::HashSet<&str> = api
        .types
        .iter()
        .filter(|t| !t.is_trait && super::dto::can_emit_first_class_struct(t, mapper, exclude_fields, known_dto_names))
        .map(|t| t.name.as_str())
        .collect();

    for type_name in struct_candidates {
        let type_snake = AsSnakeCase(type_name).to_string();
        let swift_name = format!("{type_snake}_from_json").to_lower_camel_case();
        if first_class_set.contains(type_name) {
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_from_json_decode.swift.jinja",
                minijinja::context! {
                    function_name => &swift_name,
                    type_name => type_name,
                },
            ));
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_from_json_bridge.swift.jinja",
                minijinja::context! {
                    function_name => &swift_name,
                    type_name => type_name,
                },
            ));
        }
    }

    let codable_enum_set: std::collections::HashSet<&str> = api
        .enums
        .iter()
        .filter(|e| e.has_serde && enum_emits_codable(e, known_dto_names))
        .map(|e| e.name.as_str())
        .collect();
    for enum_name in enum_candidates {
        let enum_snake = AsSnakeCase(enum_name).to_string();
        let swift_name = format!("{enum_snake}_from_json").to_lower_camel_case();
        if codable_enum_set.contains(enum_name) {
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_from_json_decode.swift.jinja",
                minijinja::context! {
                    function_name => &swift_name,
                    type_name => enum_name,
                },
            ));
        } else {
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_from_json_bridge.swift.jinja",
                minijinja::context! {
                    function_name => &swift_name,
                    type_name => enum_name,
                },
            ));
        }
    }
}

pub(super) fn enum_emits_codable(en: &EnumDef, known_dto_names: &std::collections::HashSet<String>) -> bool {
    if !en.has_serde {
        return false;
    }
    let all_unit = en.variants.iter().all(|v| v.fields.is_empty());
    if all_unit {
        return true;
    }
    super::enums::all_variants_codable_safe(en, known_dto_names)
}

pub(super) fn emit_bytes_overloads(func: &FunctionDef, _all_names: &std::collections::HashSet<&str>, out: &mut String) {
    let swift_inner = swift_ident(&func.name.to_lower_camel_case());
    let wrapper_name = if swift_inner.ends_with("Sync") {
        swift_inner[..swift_inner.len() - 4].to_string()
    } else {
        swift_inner.clone()
    };
    let inner_call = swift_inner.clone();

    let trailing_params: Vec<&crate::core::ir::ParamDef> = func.params.iter().skip(1).collect();

    let return_ty = swift_return_type(&func.return_type);
    let throws_clause = if func.error_type.is_some() { " throws" } else { "" };
    let return_suffix = swift_return_conversion_suffix(&func.return_type);

    let trailing_param_text = render_trailing_params(trailing_params.iter().copied());
    let trailing_args = render_trailing_args(trailing_params.iter().copied());

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_bytes_string_overload.jinja",
        minijinja::context! {
            wrapper_name => &wrapper_name,
            trailing_params => &trailing_param_text,
            throws_clause => throws_clause,
            return_ty => &return_ty,
            inner_call => &inner_call,
            trailing_args => &trailing_args,
            return_suffix => &return_suffix,
        },
    ));

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_bytes_array_overload.jinja",
        minijinja::context! {
            wrapper_name => &wrapper_name,
            trailing_params => &trailing_param_text,
            throws_clause => throws_clause,
            return_ty => &return_ty,
            inner_call => &inner_call,
            trailing_args => &trailing_args,
            return_suffix => &return_suffix,
        },
    ));
}

pub(super) fn emit_path_overload(func: &FunctionDef, _all_names: &std::collections::HashSet<&str>, out: &mut String) {
    let swift_inner = swift_ident(&func.name.to_lower_camel_case());
    let wrapper_name = if swift_inner.ends_with("Sync") {
        swift_inner[..swift_inner.len() - 4].to_string()
    } else {
        swift_inner.clone()
    };
    let inner_call = swift_inner.clone();

    let trailing_params: Vec<&crate::core::ir::ParamDef> = func.params.iter().skip(1).collect();
    let return_ty = swift_return_type(&func.return_type);
    let throws_clause = if func.error_type.is_some() { " throws" } else { "" };
    let return_suffix = swift_return_conversion_suffix(&func.return_type);

    let trailing_param_text = render_trailing_params_with_defaults(trailing_params.iter().copied());
    let trailing_args = render_trailing_args(trailing_params.iter().copied());

    out.push_str(&crate::backends::swift::template_env::render(
        "swift_path_overload.jinja",
        minijinja::context! {
            wrapper_name => &wrapper_name,
            trailing_params => &trailing_param_text,
            throws_clause => throws_clause,
            return_ty => &return_ty,
            inner_call => &inner_call,
            trailing_args => &trailing_args,
            return_suffix => &return_suffix,
        },
    ));
}

pub(super) fn render_trailing_params<'a>(params: impl Iterator<Item = &'a crate::core::ir::ParamDef>) -> String {
    let mut out = String::new();
    for p in params {
        let swift_name = p.name.to_lower_camel_case();
        let ty_str = if p.optional {
            format!("{}?", swift_type_name(&p.ty))
        } else {
            swift_type_name(&p.ty)
        };
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_trailing_param.jinja",
            minijinja::context! {
                swift_name => &swift_name,
                ty_str => &ty_str,
            },
        ));
    }
    out
}

pub(super) fn render_trailing_params_with_defaults<'a>(
    params: impl Iterator<Item = &'a crate::core::ir::ParamDef>,
) -> String {
    let mut out = String::new();
    for p in params {
        let swift_name = p.name.to_lower_camel_case();
        if p.optional {
            let ty_str = swift_type_name(&p.ty);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_trailing_param_optional_default.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                    ty_str => &ty_str,
                },
            ));
        } else {
            let ty_str = swift_type_name(&p.ty);
            out.push_str(&crate::backends::swift::template_env::render(
                "swift_trailing_param.jinja",
                minijinja::context! {
                    swift_name => &swift_name,
                    ty_str => &ty_str,
                },
            ));
        }
    }
    out
}

pub(super) fn render_trailing_args<'a>(params: impl Iterator<Item = &'a crate::core::ir::ParamDef>) -> String {
    let mut out = String::new();
    for p in params {
        let swift_name = p.name.to_lower_camel_case();
        out.push_str(&crate::backends::swift::template_env::render(
            "swift_trailing_arg.jinja",
            minijinja::context! {
                swift_name => &swift_name,
            },
        ));
    }
    out
}

pub(super) fn swift_type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "[UInt8]".to_string(),
        TypeRef::Path => "String".to_string(),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Optional(inner) => format!("{}?", swift_type_name(inner)),
        TypeRef::Vec(inner) => format!("[{}]", swift_type_name(inner)),
        TypeRef::Map(k, v) => format!("[{}: {}]", swift_type_name(k), swift_type_name(v)),
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Bool",
            PrimitiveType::U8 => "UInt8",
            PrimitiveType::U16 => "UInt16",
            PrimitiveType::U32 => "UInt32",
            PrimitiveType::U64 => "UInt64",
            PrimitiveType::I8 => "Int8",
            PrimitiveType::I16 => "Int16",
            PrimitiveType::I32 => "Int32",
            PrimitiveType::I64 => "Int64",
            PrimitiveType::Usize => "UInt",
            PrimitiveType::Isize => "Int",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
        }
        .to_string(),
        TypeRef::Unit => "Void".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "Duration".to_string(),
        TypeRef::Char => "Character".to_string(),
    }
}

pub(super) fn swift_return_type(ty: &TypeRef) -> String {
    swift_type_name(ty)
}

pub(super) fn swift_return_conversion_suffix(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String => ".toString()".to_string(),
        TypeRef::Bytes => ".map { $0 }".to_string(),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(_)) => ".map { $0 }".to_string(),
        _ => String::new(),
    }
}

pub(super) fn convenience_name_shadows_bridge(func: &FunctionDef) -> bool {
    let swift_inner = swift_ident(&func.name.to_lower_camel_case());
    let wrapper_name = if swift_inner.ends_with("Sync") {
        swift_inner[..swift_inner.len() - 4].to_string()
    } else {
        swift_inner.clone()
    };
    wrapper_name == swift_inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeDef;

    /// A non-opaque serde DTO whose fields are not all first-class-supported (e.g. a
    /// `Map` field) is NOT a first-class Codable struct, so its `*FromJson` helper
    /// delegates to `RustBridge.{type}FromJson`. That bridge symbol only exists when
    /// the Rust bridge crate compiled the type for the active feature set. After
    /// `with_cfg_filtered` drops a cfg-gated type whose feature is off, the high-level
    /// `emit_from_json_forwarders` pass must NOT emit a dangling `RustBridge` reference
    /// for it — while a non-gated bridge type must still get its forwarder.
    #[test]
    fn from_json_forwarders_skip_cfg_filtered_bridge_types() {
        use crate::core::ir::FieldDef;

        // A Map field makes the type non-first-class, forcing the bridge branch.
        fn bridge_serde_ty(name: &str, cfg: Option<&str>) -> TypeDef {
            TypeDef {
                name: name.to_string(),
                is_opaque: false,
                has_serde: true,
                cfg: cfg.map(str::to_string),
                fields: vec![FieldDef {
                    name: "table".to_string(),
                    ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            }
        }

        let mut api = ApiSurface::default();
        api.types.push(bridge_serde_ty("PdfMetadata", None));
        api.types.push(bridge_serde_ty("Preset", Some("feature = \"presets\"")));

        let configured: std::collections::HashSet<&str> = ["pdf"].into_iter().collect();
        let filtered = api.with_cfg_filtered(&configured);

        let mapper = SwiftMapper;
        let exclude_types = std::collections::HashSet::new();
        let exclude_fields = std::collections::HashSet::new();
        let known_dto_names = std::collections::HashSet::new();
        let mut out = String::new();
        emit_from_json_forwarders(
            &filtered,
            &exclude_types,
            &mapper,
            &exclude_fields,
            &known_dto_names,
            &mut out,
        );

        // Satisfied opaque type keeps its bridge forwarder.
        assert!(
            out.contains("RustBridge.pdfMetadataFromJson"),
            "satisfied opaque type must keep its bridge forwarder. Got:\n{out}"
        );
        // cfg-gated opaque type (feature off) must produce no dangling bridge reference.
        assert!(
            !out.contains("presetFromJson") && !out.contains("RustBridge.Preset"),
            "cfg-filtered type must not emit a dangling RustBridge reference. Got:\n{out}"
        );
    }
}
