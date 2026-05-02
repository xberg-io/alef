//! Python API wrapper function generation: `api.py`.

use ahash::{AHashMap, AHashSet};
use alef_codegen::doc_emission::doc_first_paragraph_joined;
use alef_codegen::generators;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::ApiSurface;

use super::enums::{Wrapping, sanitize_python_doc};
use super::types::collect_named_types;

/// Generate api.py — wrapper functions that convert Python types to Rust binding types.
///
/// For each function parameter whose type is a `has_default` struct (e.g. `ConversionOptions`),
/// we generate a `_to_rust_{snake_name}` converter that maps the Python `@dataclass` instance
/// to the Rust binding's pyclass by passing every field as a keyword argument.
pub(super) fn gen_api_py(
    api: &ApiSurface,
    module_name: &str,
    package_name: &str,
    trait_bridges: &[alef_core::config::TraitBridgeConfig],
    dto: &alef_core::config::DtoConfig,
) -> String {
    use alef_core::config::PythonDtoStyle;
    use alef_core::ir::TypeRef;
    use heck::ToSnakeCase;

    // When output_style is TypedDict, types with is_return_type=true are emitted as
    // TypedDict classes (plain dicts at runtime). Converters for those types must use
    // `value.get("field")` dict access instead of `value.field` attribute access.
    let output_style = dto.python_output_style();

    // Collect bridge param names so they can be typed as `object | None` instead of
    // `str | None`. The IR sanitizes trait handle types to String, but callers pass
    // arbitrary Python objects implementing the visitor protocol.
    let bridge_param_names: ahash::AHashSet<&str> =
        trait_bridges.iter().filter_map(|b| b.param_name.as_deref()).collect();

    // Build lookup for options-field bridges: options_type_name → (visitor_kwarg_name, field_name).
    // When a function parameter's type matches an options-field bridge's `options_type`, we add
    // a `visitor: object | None = None` convenience kwarg to the Python wrapper.
    let options_field_bridges: AHashMap<&str, (&str, &str)> = trait_bridges
        .iter()
        .filter(|b| b.bind_via == alef_core::config::BridgeBinding::OptionsField)
        .filter_map(|b| {
            let options_type = b.options_type.as_deref()?;
            let param_name = b.param_name.as_deref()?;
            let field_name = b.resolved_options_field()?;
            Some((options_type, (param_name, field_name)))
        })
        .collect();

    // Build lookup: type_name → TypeDef for has_default types
    let default_types: AHashMap<String, &alef_core::ir::TypeDef> = api
        .types
        .iter()
        .filter(|t| t.has_default && !t.name.ends_with("Update"))
        .map(|t| (t.name.clone(), t))
        .collect();

    // Collect enum names for conversion detection
    let enum_names: AHashSet<&str> = api.enums.iter().map(|e| e.name.as_str()).collect();

    // Separate data enums (tagged unions exposed as dict-accepting structs) from simple int enums.
    // Data enums are passed through as dicts; simple enums need string→variant lookup.
    let data_enum_names: AHashSet<&str> = api
        .enums
        .iter()
        .filter(|e| generators::enum_has_data_variants(e))
        .map(|e| e.name.as_str())
        .collect();

    // Determine which has_default types are referenced by function parameters (directly or nested)
    let mut needed_converters: Vec<String> = Vec::new();
    let mut visited: AHashSet<String> = AHashSet::new();

    fn collect_needed(
        type_name: &str,
        default_types: &AHashMap<String, &alef_core::ir::TypeDef>,
        needed: &mut Vec<String>,
        visited: &mut AHashSet<String>,
    ) {
        if !visited.insert(type_name.to_string()) {
            return;
        }
        if let Some(typ) = default_types.get(type_name) {
            // First collect nested types so they appear before the parent converter.
            // `classify_param_type` recursively unwraps Optional/Vec layers so a
            // `Vec<HasDefault>` field still discovers the leaf converter.
            for field in &typ.fields {
                if let Some((name, _)) = classify_param_type(&field.ty) {
                    if default_types.contains_key(name) {
                        collect_needed(name, default_types, needed, visited);
                    }
                }
            }
            needed.push(type_name.to_string());
        }
    }

    for func in &api.functions {
        for param in &func.params {
            // `classify_param_type` unwraps Optional/Vec/Optional<Vec> layers
            // so a `Vec<HasDefault>` parameter still triggers converter emission
            // for the leaf type.
            if let Some((name, _)) = classify_param_type(&param.ty) {
                collect_needed(name, &default_types, &mut needed_converters, &mut visited);
            }
        }
    }

    // Collect all type names referenced in function signatures (params + returns)
    // that aren't converters — these need to be imported too.
    let mut all_type_imports: AHashSet<String> = AHashSet::new();
    for type_name in &needed_converters {
        all_type_imports.insert(type_name.clone());
    }
    for func in &api.functions {
        for param in &func.params {
            collect_named_types(&param.ty, &mut all_type_imports);
        }
        // Collect return type references so they are imported and can be used as bare
        // names in annotations. This avoids `_rust.`-prefixed return types which cause
        // type checkers to see a different type than the public re-export.
        collect_named_types(&func.return_type, &mut all_type_imports);
    }

    let mut out = String::with_capacity(4096);
    out.push_str(&hash::header(CommentStyle::Hash));
    out.push_str("\"\"\"Public API for conversion.\"\"\"\n\n");
    out.push_str(&format!("import {package_name}.{module_name} as _rust\n"));

    // Split type imports: opaque/error types and non-options types come from the native module,
    // has_default dataclass types come from .options.
    let opaque_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| t.is_opaque)
        .map(|t| t.name.clone())
        .collect();
    let error_names: AHashSet<String> = api.errors.iter().map(|e| e.name.clone()).collect();
    // Collect types that appear as return types of functions/methods — these live in the
    // native module, not options.py.
    let return_type_names: AHashSet<String> = {
        fn collect_named_types(ty: &alef_core::ir::TypeRef, out: &mut AHashSet<String>) {
            match ty {
                alef_core::ir::TypeRef::Named(name) => {
                    out.insert(name.clone());
                }
                alef_core::ir::TypeRef::Optional(inner) | alef_core::ir::TypeRef::Vec(inner) => {
                    collect_named_types(inner, out)
                }
                alef_core::ir::TypeRef::Map(k, v) => {
                    collect_named_types(k, out);
                    collect_named_types(v, out);
                }
                _ => {}
            }
        }
        let mut names = AHashSet::new();
        for func in &api.functions {
            collect_named_types(&func.return_type, &mut names);
        }
        for ty in &api.types {
            for method in &ty.methods {
                collect_named_types(&method.return_type, &mut names);
            }
        }
        // Transitively include field types of native types (they arrive from the native module).
        let mut changed = true;
        while changed {
            changed = false;
            for ty in &api.types {
                if names.contains(&ty.name) || ty.is_opaque {
                    for field in &ty.fields {
                        let before = names.len();
                        collect_named_types(&field.ty, &mut names);
                        if names.len() > before {
                            changed = true;
                        }
                    }
                }
            }
        }
        names
    };
    // Types that exist in options.py: has_default structs (excluding Update types and return
    // types — return types are defined in the native module, not options.py).
    let options_type_names: AHashSet<String> = api
        .types
        .iter()
        .filter(|t| {
            t.has_default && !t.name.ends_with("Update") && !t.is_return_type && !return_type_names.contains(&t.name)
        })
        .map(|t| t.name.clone())
        .collect();
    // All non-enum IR type names (used to distinguish structs from enums in classification).
    let all_ir_type_names: AHashSet<String> = api.types.iter().map(|t| t.name.clone()).collect();
    // Enums that options.py actually exports: plain (non-data) unit enums referenced by
    // has_default struct fields. Data enums and enums not referenced by config structs live
    // in the native module, not options.py — so they must be imported from the native module.
    let options_enum_names: AHashSet<String> = {
        let mut set = AHashSet::new();
        for typ in api
            .types
            .iter()
            .filter(|t| t.has_default && !t.name.ends_with("Update"))
        {
            for field in &typ.fields {
                let inner_name = match &field.ty {
                    TypeRef::Named(n) => Some(n.as_str()),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            Some(n.as_str())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(name) = inner_name {
                    if enum_names.contains(name) && !data_enum_names.contains(name) {
                        set.insert(name.to_string());
                    }
                }
            }
        }
        set
    };

    let all_enum_names: AHashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();
    let mut options_imports: Vec<&str> = Vec::new();
    let mut native_imports: Vec<&str> = Vec::new();
    for name in &all_type_imports {
        let is_options = options_type_names.contains(name) || options_enum_names.contains(name);
        let is_native = opaque_names.contains(name)
            || error_names.contains(name)
            || (all_ir_type_names.contains(name) && !is_options)
            // Enums not in options_enum_names live in the native module.
            || (all_enum_names.contains(name) && !options_enum_names.contains(name));
        if is_native {
            native_imports.push(name.as_str());
        } else {
            options_imports.push(name.as_str());
        }
    }

    // Import types used in function signatures at runtime (not under TYPE_CHECKING)
    // since they appear as parameter/return type annotations in generated wrapper functions.
    // Sort for deterministic codegen — `all_type_imports` is an AHashSet, so iteration
    // order changes between runs; without sorting, hash-based caching always misses.
    native_imports.sort_unstable();
    options_imports.sort_unstable();
    if !native_imports.is_empty() {
        out.push_str(&format!("\nfrom .{module_name} import {}\n", native_imports.join(", ")));
    }
    if !options_imports.is_empty() {
        out.push_str(&format!("\nfrom .options import {}\n", options_imports.join(", ")));
    }
    out.push_str("\n\n");

    // Generate converter functions for each needed has_default type
    for type_name in &needed_converters {
        let typ = default_types[type_name];
        let snake = type_name.to_snake_case();

        // When the output style is TypedDict, any value passed to a `_to_rust_*` converter
        // may arrive as a plain dict (either because it IS a TypedDict itself, or because
        // it was nested inside one and extracted with `.get()`). Use `value.get("field")`
        // for all field accesses in TypedDict mode to avoid AttributeError on plain dicts.
        let is_typeddict = output_style == PythonDtoStyle::TypedDict;

        // Helper: emit `value.field` or `value.get("field")` depending on the type kind.
        let field_access = |name: &str| -> String {
            if is_typeddict {
                format!("value.get(\"{name}\")")
            } else {
                format!("value.{name}")
            }
        };

        // Check if this type has an options-field bridge (e.g. ConversionOptions.visitor).
        // If so, the converter gains a `_visitor_override: object | None = None` param.
        let bridge_visitor_field = options_field_bridges.get(type_name.as_str()).copied();

        // Build the converter signature.
        // When there's a visitor override param, always use multi-line form.
        if bridge_visitor_field.is_some() {
            out.push_str(&format!(
                "def _to_rust_{snake}(\n    value: {type_name} | None,\n    _visitor_override: object | None = None,\n) -> _rust.{type_name} | None:\n"
            ));
        } else {
            // Single-line: "def _to_rust_{snake}(value: {type_name} | None) -> _rust.{type_name} | None:"
            // Prefix "def _to_rust_" (13) + snake + "(value: " (8) + type_name + " | None) -> _rust." (18)
            // + type_name + " | None:" (8) = 47 + snake.len + 2 * type_name.len
            let sig_len = 47 + snake.len() + 2 * type_name.len();
            if sig_len > 100 {
                out.push_str(&format!(
                    "def _to_rust_{snake}(\n    value: {type_name} | None,\n) -> _rust.{type_name} | None:\n"
                ));
            } else {
                out.push_str(&format!(
                    "def _to_rust_{snake}(value: {type_name} | None) -> _rust.{type_name} | None:\n"
                ));
            }
        }
        out.push_str(&format!(
            "    \"\"\"Convert Python {type_name} to Rust binding type.\"\"\"\n"
        ));
        out.push_str("    if value is None:\n");
        if let Some((kwarg_name, _field_name)) = bridge_visitor_field {
            // When value is None but visitor override is provided, construct a default instance.
            out.push_str(&format!(
                "        if _visitor_override is not None:\n            return _rust.{type_name}({kwarg_name}=_visitor_override)\n        return None\n"
            ));
        } else {
            out.push_str("        return None\n");
        }
        out.push_str(&format!("    return _rust.{type_name}(\n"));

        for field in &typ.fields {
            // Check if the field's type is itself a has_default Named type (needs nested conversion)
            let inner_named = match &field.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(nested_name) = inner_named {
                if default_types.contains_key(nested_name) {
                    let nested_snake = nested_name.to_snake_case();
                    let accessor = field_access(&field.name);
                    out.push_str(&format!(
                        "        {}=_to_rust_{nested_snake}({accessor}),\n",
                        field.name
                    ));
                    continue;
                }
                // Single enum field: convert str -> Rust enum
                if enum_names.contains(&nested_name) {
                    if data_enum_names.contains(&nested_name) {
                        // Data enum (tagged union): PyO3 constructor accepts a dict directly.
                        // If the caller already holds a _rust.{EnumName} instance (e.g. from a
                        // previous conversion), pass it through to avoid a double-wrap error;
                        // otherwise wrap the dict via the PyO3 constructor.
                        let accessor = field_access(&field.name);
                        if matches!(&field.ty, TypeRef::Optional(_)) || field.optional {
                            out.push_str(&format!(
                                "        {name}=({accessor} if isinstance({accessor}, _rust.{enum_name}) else _rust.{enum_name}({accessor})) if {accessor} is not None else None,\n",
                                name = field.name,
                                enum_name = nested_name,
                            ));
                        } else {
                            out.push_str(&format!(
                                "        {name}={accessor} if isinstance({accessor}, _rust.{enum_name}) else _rust.{enum_name}({accessor}),\n",
                                name = field.name,
                                enum_name = nested_name,
                            ));
                        }
                    } else {
                        // Simple int enum: the input value is a _rust.ConversionOptions (PyO3
                        // struct), so its enum fields are already the correct _rust.EnumType
                        // instances.  Pass them through directly — no string lookup needed.
                        let accessor = field_access(&field.name);
                        out.push_str(&format!("        {name}={accessor},\n", name = field.name,));
                    }
                    continue;
                }
            }

            // Vec<Enum> field: convert list[str] -> list[RustEnum]
            if let TypeRef::Vec(inner) = &field.ty {
                if let TypeRef::Named(enum_name) = inner.as_ref() {
                    if enum_names.contains(&enum_name.as_str()) {
                        let accessor = field_access(&field.name);
                        if data_enum_names.contains(&enum_name.as_str()) {
                            // Data enum list: each element is a dict passed to the PyO3 constructor.
                            out.push_str(&format!(
                                "        {name}=[_rust.{enum_name}(v) for v in {accessor}],\n",
                                name = field.name,
                            ));
                        } else {
                            // Simple int enum list: elements are already _rust.EnumType instances
                            // since the input value is a PyO3 struct.  Pass the list through.
                            out.push_str(&format!("        {name}={accessor},\n", name = field.name,));
                        }
                        continue;
                    }
                }
            }

            // Check if this field is the options-field bridge field (visitor handle).
            // When it is, use the _visitor_override if provided, else fall back to value.field.
            if let Some((kwarg_name, field_name)) = bridge_visitor_field {
                if field.name == field_name {
                    out.push_str(&format!(
                        "        {field_name}=_visitor_override if _visitor_override is not None else {accessor},\n",
                        accessor = field_access(field_name),
                    ));
                    let _ = kwarg_name; // used above in the None branch
                    continue;
                }
            }
            let accessor = field_access(&field.name);
            out.push_str(&format!("        {name}={accessor},\n", name = field.name));
        }

        out.push_str("    )\n\n\n");
    }

    // Generate wrapper for each function
    for func in &api.functions {
        // Build Python-side params applying seen_optional promotion.
        //
        // Python syntax requires params with defaults to follow params without defaults.
        // The PyO3 binding uses seen_optional promotion: once any optional param appears
        // in the Rust function signature, all subsequent params also get `= None` defaults
        // (wrapped in Option<T>). The Python wrapper must mirror this so callers can omit
        // those trailing params.
        //
        // Algorithm:
        //   1. Walk params in IR order, track seen_optional.
        //   2. A param is "promoted" if it is NOT optional in the IR but seen_optional is
        //      already true (an earlier param was optional).
        //   3. Partition into truly-required (not optional, not promoted) and
        //      all-with-defaults (optional || promoted).
        //   4. Emit truly-required first, then all-with-defaults — satisfying Python syntax.
        let mut seen_optional_so_far = false;
        let mut promoted_params: ahash::AHashSet<String> = ahash::AHashSet::new();
        for param in &func.params {
            if param.optional {
                seen_optional_so_far = true;
            } else if seen_optional_so_far {
                // This param is not optional in the IR but comes after an optional param
                // → the PyO3 binding promotes it to Option<T>; the Python wrapper must too.
                promoted_params.insert(param.name.clone());
            }
        }

        let mut sig_parts = Vec::new();
        let is_with_default = |p: &&alef_core::ir::ParamDef| p.optional || promoted_params.contains(&p.name);
        let (required, optional): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in required.iter().chain(optional.iter()) {
            // Bridge params have their IR type sanitized to String, but callers pass
            // arbitrary Python objects implementing the visitor protocol — use `object`.
            let base_type = if bridge_param_names.contains(param.name.as_str()) {
                "object".to_string()
            } else {
                crate::type_map::python_type(&param.ty)
            };
            let needs_default = param.optional || promoted_params.contains(&param.name);
            let py_type = if needs_default {
                if base_type.ends_with("| None") {
                    format!("{} = None", base_type)
                } else {
                    format!("{} | None = None", base_type)
                }
            } else {
                base_type
            };
            sig_parts.push(format!("{}: {}", param.name, py_type));
        }

        // Detect if this function has an options-field bridge (visitor embedded in options).
        // When it does, add a convenience `visitor: object | None = None` kwarg.
        // We track: (options_param_name, options_type_name, visitor_kwarg_name, field_name).
        let options_field_visitor_kwarg: Option<(&str, &str, &str)> = func.params.iter().find_map(|p| {
            let type_name = match &p.ty {
                alef_core::ir::TypeRef::Named(n) => Some(n.as_str()),
                alef_core::ir::TypeRef::Optional(inner) => {
                    if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            }?;
            let (kwarg_name, _field_name) = options_field_bridges.get(type_name)?;
            Some((p.name.as_str(), type_name, *kwarg_name))
        });
        if let Some((_, _, kwarg_name)) = options_field_visitor_kwarg {
            sig_parts.push(format!("{kwarg_name}: object | None = None"));
        }

        let return_type_str = crate::type_map::python_type(&func.return_type);
        // Async pyo3 functions return a coroutine — the Python wrapper must be `async def`
        // so that `result = await fn(...)` works correctly and type checkers see the right type.
        let def_keyword = if func.is_async { "async def" } else { "def" };
        let has_builtin_param = sig_parts
            .iter()
            .any(|p| crate::gen_stubs::is_python_builtin_name(p.split(':').next().unwrap_or("").trim()));
        let single_line = format!(
            "{def_keyword} {}({}) -> {}:\n",
            func.name,
            sig_parts.join(", "),
            return_type_str
        );
        if single_line.len() <= 100 && !has_builtin_param {
            out.push_str(&single_line);
        } else {
            out.push_str(&format!("{def_keyword} {}(\n", func.name));
            for param in &sig_parts {
                let name = param.split(':').next().unwrap_or("").trim();
                if crate::gen_stubs::is_python_builtin_name(name) {
                    out.push_str(&format!("    {},  # noqa: A002\n", param));
                } else {
                    out.push_str(&format!("    {},\n", param));
                }
            }
            out.push_str(&format!(") -> {}:\n", return_type_str));
        }
        {
            let doc_with_period = if !func.doc.is_empty() {
                let doc_first_para = doc_first_paragraph_joined(&func.doc);
                let doc_sanitized = sanitize_python_doc(&doc_first_para);
                // `    """..."""` is 10 chars of overhead; period may add 1 more char.
                // Limit content to 89 chars so that with a trailing period the full line stays ≤100.
                let doc_content = if doc_sanitized.len() > 89 {
                    doc_sanitized[..89].to_string()
                } else {
                    doc_sanitized
                };
                if doc_content.ends_with('.') {
                    doc_content
                } else {
                    format!("{}.", doc_content)
                }
            } else {
                use heck::ToSnakeCase;
                let snake = func.name.to_snake_case();
                let sentence = snake.replace('_', " ");
                let mut chars = sentence.chars();
                let capitalized = match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                };
                format!("{}.", capitalized)
            };
            out.push_str(&format!("    \"\"\"{doc_with_period}\"\"\"\n"));
        }

        // For each param that has a converter, emit a local conversion variable.
        // Use the same required-first, optional-last order as the Python signature so that
        // positional calls to the native function match the pyo3 signature declaration.
        //
        // We classify the param's type by unwrapping `Optional`/`Vec` layers down to the
        // leaf `Named` type. The classification determines whether a scalar conversion or
        // a list-comprehension conversion is generated.
        // Each entry is (param_name, value_expr) — used to build keyword-argument calls so
        // that the generated `_rust.fn(path=path, config=_rust_config, ...)` form is
        // independent of the pyo3 signature parameter order.
        let mut call_args: Vec<(String, String)> = Vec::new();
        let (req_params, opt_params): (Vec<_>, Vec<_>) = func.params.iter().partition(|p| !is_with_default(p));
        for param in req_params.iter().chain(opt_params.iter()) {
            let class = classify_param_type(&param.ty);

            if let Some((name, wrapping)) = class {
                let pname = &param.name;
                let var = format!("_rust_{pname}");
                // A param is "optional" for the conversion guard when:
                //   - its IR type is Optional/OptionalVec, OR
                //   - the IR param itself is optional, OR
                //   - it was promoted to optional via seen_optional (comes after an optional param).
                let is_promoted = promoted_params.contains(pname.as_str());
                let optional =
                    matches!(wrapping, Wrapping::Optional | Wrapping::OptionalVec) || param.optional || is_promoted;
                let is_collection = matches!(wrapping, Wrapping::Vec | Wrapping::OptionalVec);

                // has_default struct: Python-side conversion via _to_rust_<snake>().
                if default_types.contains_key(name) {
                    let snake = name.to_snake_case();
                    // When this param is the options param of an options-field bridge, pass the
                    // visitor kwarg name as _visitor_override so the converter injects it.
                    let scalar_expr = if options_field_bridges.contains_key(name) {
                        if let Some((_, _, kwarg_name)) = options_field_visitor_kwarg {
                            format!("_to_rust_{snake}({pname}, _visitor_override={kwarg_name})")
                        } else {
                            format!("_to_rust_{snake}({pname})")
                        }
                    } else {
                        format!("_to_rust_{snake}({pname})")
                    };
                    if is_collection {
                        let element_expr = format!("_to_rust_{snake}(__item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(&mut out, &var, pname, &body, optional);
                    } else {
                        // When this param is the options param of an options-field bridge, the
                        // converter handles all None cases itself — emit an unconditional call
                        // so that `visitor=visitor` is forwarded even when `options is None`.
                        let bridge_optional = optional
                            && !(options_field_bridges.contains_key(name) && options_field_visitor_kwarg.is_some());
                        emit_param_conversion(&mut out, &var, pname, &scalar_expr, bridge_optional);
                        // Required scalar (not optional and not promoted): failed converter → raise.
                        // Promoted params are treated as optional at the Python level, so do not raise.
                        if !param.optional && !is_promoted && !is_collection {
                            out.push_str(&format!(
                                "    if {var} is None:\n        msg = \"{pname} conversion returned None\"\n        raise ValueError(msg)\n"
                            ));
                        }
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
                // Data enum (tagged union): wrap with `_rust.<EnumName>(value)` if not already.
                if data_enum_names.contains(name) {
                    let scalar_expr =
                        format!("(_rust.{name}({pname}) if not isinstance({pname}, _rust.{name}) else {pname})");
                    if is_collection {
                        let element_expr =
                            format!("(_rust.{name}(__item) if not isinstance(__item, _rust.{name}) else __item)");
                        let body = format!("[{element_expr} for __item in {pname}]");
                        emit_param_conversion(&mut out, &var, pname, &body, optional);
                    } else {
                        emit_param_conversion(&mut out, &var, pname, &scalar_expr, optional);
                    }
                    call_args.push((pname.clone(), var));
                    continue;
                }
            }
            call_args.push((param.name.clone(), param.name.clone()));
        }

        // Use keyword arguments so the call is independent of the pyo3 signature order.
        // This ensures wrapper-side required/optional reordering doesn't misalign slots.
        let kwargs: Vec<String> = call_args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // Async pyo3 functions return a coroutine that must be awaited by the Python caller.
        let return_prefix = if func.is_async { "await " } else { "" };
        out.push_str(&format!(
            "    return {return_prefix}_rust.{}({})\n\n\n",
            func.name,
            kwargs.join(", ")
        ));
    }

    // Emit pass-through wrappers for trait-bridge registration functions.
    // These functions are emitted as #[pyfunction] in the native Rust module but are not in
    // api.functions — they must be re-exported via api.py so callers can use the public package
    // path (e.g. `kreuzberg.register_ocr_backend`) rather than `kreuzberg._kreuzberg.register_ocr_backend`.
    for register_fn in crate::trait_bridge::collect_bridge_register_fns(trait_bridges) {
        out.push_str(&format!(
            "def {register_fn}(backend: object) -> None:\n    \"\"\"Register a {register_fn} backend.\"\"\"\n    return _rust.{register_fn}(backend=backend)\n\n\n"
        ));
    }

    out
}
pub(super) fn classify_param_type(ty: &alef_core::ir::TypeRef) -> Option<(&str, Wrapping)> {
    use alef_core::ir::TypeRef;
    match ty {
        TypeRef::Named(n) => Some((n.as_str(), Wrapping::Plain)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Optional)),
            TypeRef::Vec(vec_inner) => match vec_inner.as_ref() {
                TypeRef::Named(n) => Some((n.as_str(), Wrapping::OptionalVec)),
                _ => None,
            },
            _ => None,
        },
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some((n.as_str(), Wrapping::Vec)),
            _ => None,
        },
        _ => None,
    }
}

/// Emit a `{var} = {body}` line, guarded by `if {pname} is not None else None`
/// when the parameter is optional.
pub(super) fn emit_param_conversion(out: &mut String, var: &str, pname: &str, body: &str, optional: bool) {
    if optional {
        out.push_str(&format!("    {var} = {body} if {pname} is not None else None\n"));
    } else {
        out.push_str(&format!("    {var} = {body}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_param_type, emit_param_conversion};
    use alef_core::ir::TypeRef;

    /// classify_param_type returns Plain for a bare Named type.
    #[test]
    fn classify_param_type_returns_plain_for_named() {
        let ty = TypeRef::Named("Foo".to_string());
        let result = classify_param_type(&ty);
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "Foo");
    }

    /// classify_param_type returns None for a primitive type.
    #[test]
    fn classify_param_type_returns_none_for_primitive() {
        let ty = TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool);
        assert!(classify_param_type(&ty).is_none());
    }

    /// emit_param_conversion emits a guarded None check when optional.
    #[test]
    fn emit_param_conversion_guards_optional() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", true);
        assert!(out.contains("if x is not None else None"));
    }

    /// emit_param_conversion emits a direct assignment when not optional.
    #[test]
    fn emit_param_conversion_direct_when_required() {
        let mut out = String::new();
        emit_param_conversion(&mut out, "_rust_x", "x", "convert(x)", false);
        assert!(!out.contains("if x is not None"));
        assert!(out.contains("_rust_x = convert(x)"));
    }
}
