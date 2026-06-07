/// Rewrite opaque impl-block methods whose return type is a capsule type.
///
/// The generic method generator emits `Ok({CapsuleType} { inner: Arc::new(result) })` for
/// methods returning capsule-configured types.  Because capsule types have no `#[pyclass]`
/// struct, that code does not compile.  This function replaces each such method with a
/// capsule-aware body that either calls `into_raw()` + `PyCapsule_New` (Capsule variant) or
/// constructs the Python object via the dependency capsule (ConstructFrom variant), mirroring
/// what `capsule::gen_capsule_function` does for free functions.
pub(super) fn rewrite_capsule_methods(
    impl_block: String,
    typ: &crate::core::ir::TypeDef,
    capsule_types: &std::collections::HashMap<String, crate::core::config::CapsuleTypeConfig>,
    error_converters: &[String],
) -> String {
    use crate::codegen::type_mapper::TypeMapper as _;
    use crate::core::ir::TypeRef;
    use heck::ToSnakeCase;

    let mut result = impl_block;

    for method in &typ.methods {
        // Determine whether this method's return type is a capsule type.
        let capsule_ret_name: Option<&str> = match &method.return_type {
            TypeRef::Named(n) if capsule_types.contains_key(n.as_str()) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() {
                    if capsule_types.contains_key(n.as_str()) {
                        Some(n.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };

        // Check if any parameter is a capsule type.
        let has_capsule_param = method
            .params
            .iter()
            .any(|p| matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str())));

        // Skip methods that don't involve capsules in parameters or return type.
        if capsule_ret_name.is_none() && !has_capsule_param {
            continue;
        }

        // If we're only handling parameter extraction (no return capsule), emit a simpler body.
        let cfg = capsule_ret_name.map(|n| &capsule_types[n]);

        // Build the old signature fragment that the generic generator emitted.
        let old_sig_search = if let Some(ret_name) = capsule_ret_name {
            // Methods returning capsules: search for `-> PyResult<{CapsuleTypeName}>`
            format!("-> PyResult<{ret_name}>")
        } else {
            // Methods only with capsule params: search for method name + opening paren.
            // We'll match by method name pattern and update params + body.
            format!("pub fn {}(", method.name)
        };

        // For methods returning capsules, verify the signature exists.
        if capsule_ret_name.is_some() && !result.contains(&old_sig_search) {
            continue;
        }

        // Detect capsule-type parameters and prepare extraction code.
        let mut capsule_param_extract = String::new();
        let mut call_args_parts: Vec<String> = Vec::new();

        for p in &method.params {
            let param_is_capsule = matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str()));

            if param_is_capsule {
                if let TypeRef::Named(capsule_name) = &p.ty {
                    // Generate extraction code for this capsule parameter
                    capsule_param_extract.push_str(&crate::backends::pyo3::template_env::render(
                        "pyo3_capsule_param_extract.jinja",
                        minijinja::context! {
                            param_name => p.name.as_str(),
                            capsule_name => capsule_name,
                        },
                    ));
                    call_args_parts.push(p.name.clone());
                } else {
                    // Fallback for non-Named types
                    call_args_parts.push(p.name.clone());
                }
            } else {
                let needs_borrow = p.is_ref && matches!(p.ty, TypeRef::String | TypeRef::Char);
                if needs_borrow {
                    call_args_parts.push(format!("&{}", p.name));
                } else {
                    call_args_parts.push(p.name.clone());
                }
            }
        }
        let call_args_str = call_args_parts.join(", ");

        // Build param list for the new signature.
        // Always prepend `py: pyo3::Python<'_>` since we need it for PyCapsule_New / Python calls.
        let mapper = crate::backends::pyo3::type_map::Pyo3Mapper::new();
        let mut sig_params = vec!["&self".to_string(), "py: pyo3::Python<'_>".to_string()];
        for p in &method.params {
            // Capsule-type parameters are accepted as Py<PyAny>, not as the Rust type
            let param_type = if matches!(&p.ty, TypeRef::Named(n) if capsule_types.contains_key(n.as_str())) {
                "pyo3::Py<pyo3::PyAny>".to_string()
            } else {
                mapper.map_type(&p.ty)
            };
            sig_params.push(format!("{}: {}", p.name, param_type));
        }

        // Build the #[pyo3(signature = (...))] attribute (skipped when there are no params).
        let sig_attr = if method.params.is_empty() {
            String::new()
        } else {
            let names = method
                .params
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("    #[pyo3(signature = ({names}))]\n")
        };

        // Build the inner core call (self.inner.method(args)).
        let core_call = format!("self.inner.{}({})", method.name, call_args_str);

        // Build the `.map_err(…)?` suffix when the method is fallible.
        let err_map_suffix = if method.error_type.is_some() {
            let converter = method
                .error_type
                .as_ref()
                .and_then(|et| {
                    let short = et.split("::").last().unwrap_or(et.as_str());
                    let candidate = format!("{}_to_py_err", short.to_snake_case());
                    if error_converters.iter().any(|c| c == &candidate) {
                        Some(candidate)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string())".to_string());
            format!(".map_err({converter})?")
        } else {
            String::new()
        };

        let params_str = sig_params.join(", ");
        let method_name = &method.name;

        // Generate the new method body based on capsule variant.
        // For methods with only capsule params (no return capsule), emit a simple wrapper.
        let new_body = if cfg.is_none() {
            // Method only has capsule params, no capsule return.
            // Just rewrite to extract capsule params and call the inner method.
            let return_annotation = if matches!(method.return_type, TypeRef::Unit) {
                "".to_string()
            } else {
                format!(" -> PyResult<{}>", mapper.map_type(&method.return_type))
            };
            format!(
                r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}){return_annotation} {{
{capsule_param_extract}        {core_call}{err_map_suffix}
    }}"#,
            )
        } else if let Some(cfg) = cfg {
            // Method returns a capsule (and may also have capsule params).
            match cfg {
                crate::core::config::CapsuleTypeConfig::Capsule(capsule_name_str) => {
                    let capsule_cstr = capsule_name_str.replace('.', "_").to_ascii_uppercase();
                    // If capsule_name_str is dotted (e.g. "tree_sitter.Language"), also construct the
                    // target Python type from the capsule so callers receive a real tree_sitter.Language,
                    // not the bare PyCapsule.
                    let construct = match capsule_name_str.rsplit_once('.') {
                    Some((module_path, class_name)) => format!(
                        r#"        // SAFETY: capsule_ptr is a valid, non-null Python object pointer we just created above.
        let _capsule_obj = unsafe {{ pyo3::Bound::from_owned_ptr(py, capsule_ptr) }};
        let _ts_mod = py.import("{module_path}")?;
        let _cls = _ts_mod.getattr("{class_name}")?;
        Ok(_cls.call1((_capsule_obj,))?.unbind())"#,
                    ),
                    None => {
                        "        // SAFETY: capsule_ptr is a valid, non-null Python object pointer we just created above.\n        Ok(unsafe { pyo3::Bound::from_owned_ptr(py, capsule_ptr) }.unbind())".to_string()
                    }
                };
                    format!(
                        r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        const {capsule_cstr}_NAME: &::std::ffi::CStr = c"{capsule_name_str}";
{capsule_param_extract}        let result = {core_call}{err_map_suffix};
        let raw_ptr = result.into_raw();
        // SAFETY: raw_ptr is a valid pointer derived from into_raw() on a value with program lifetime.
        let capsule_ptr = unsafe {{ pyo3::ffi::PyCapsule_New(raw_ptr as *mut _, {capsule_cstr}_NAME.as_ptr(), None) }};
        if capsule_ptr.is_null() {{
            return Err(pyo3::exceptions::PyRuntimeError::new_err("Failed to create PyCapsule"));
        }}
{construct}
    }}"#,
                    )
                }
                crate::core::config::CapsuleTypeConfig::ConstructFrom {
                    python_type,
                    construct_from,
                } => {
                    // For ConstructFrom: produce the dependency capsule by calling the matching
                    // free function, then call the Python factory to construct the target type.
                    let dep_snake = construct_from.to_snake_case();
                    let first_str_param = method.params.iter().find(|p| matches!(p.ty, TypeRef::String));
                    let dep_expr = if let Some(sp) = first_str_param {
                        format!("get_{dep_snake}(py, {}.clone())?.bind(py).clone()", sp.name)
                    } else {
                        format!("/* Unsupported: obtain {construct_from} capsule */ unreachable!()")
                    };

                    if let Some((module_path, class_name)) = python_type.rsplit_once('.') {
                        format!(
                            r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        // Construct {python_type} via Python-side factory.
        let _dep = {dep_expr};
        let _ts_mod = py.import("{module_path}")?;
        let _cls = _ts_mod.getattr("{class_name}")?;
        Ok(_cls.call1((_dep,))?.unbind())
    }}"#,
                        )
                    } else {
                        format!(
                            r#"    {sig_attr}    #[allow(clippy::missing_errors_doc)]
    pub fn {method_name}({params_str}) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {{
        // Construct {python_type} via Python-side factory.
        let _dep = {dep_expr};
        let _cls = py.eval(c"{python_type}", None, None)?;
        Ok(_cls.call1((_dep,))?.unbind())
    }}"#,
                        )
                    }
                }
            }
        } else {
            unreachable!("Method capsule config should be present when cfg.is_none() is false.");
        };

        // Find and replace the old method in the impl block.
        // The method generator emits `pub fn {name}(` at the start of a line with no
        // guaranteed leading indentation (the impl_block template wraps the content but
        // doesn't add per-line indentation).  Search for the bare `pub fn {name}(`.
        let method_start_marker = format!("pub fn {method_name}(");
        if let Some(start_idx) = result.find(&method_start_marker) {
            let attr_start = find_method_attrs_start(&result, start_idx);
            if let Some(end_idx) = find_method_end(&result, start_idx) {
                result = format!("{}{}{}", &result[..attr_start], new_body, &result[end_idx..]);
            }
        }
    }

    result
}

/// Returns true when `line` (trimmed) consists entirely of `#[…]` attribute patterns and
/// intervening whitespace — i.e. it contains no non-attribute tokens such as `impl Foo {`.
///
/// This correctly handles:
/// - A single attribute: `#[pyo3(signature = (name))]`  → true
/// - Multiple attributes on one line: `#[allow(dead_code)]  #[pyo3(get)]`  → true
/// - A block-attr + impl opener on one line: `#[pymethods]impl Foo {`  → false
fn is_method_attr_line(line: &str) -> bool {
    let mut rest = line.trim();
    if rest.is_empty() {
        return false; // handled by the blank-line branch; don't treat blank as attr
    }
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return true;
        }
        if !rest.starts_with("#[") {
            return false;
        }
        // Consume the `#[…]` span, respecting nested brackets.
        let mut depth = 0usize;
        let mut consumed = 0usize;
        let mut found_close = false;
        for (i, ch) in rest.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        consumed = i + 1;
                        found_close = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !found_close {
            return false;
        }
        rest = &rest[consumed..];
    }
}

/// Find the byte index of the start of the attribute block that precedes the `pub fn` at
/// `fn_idx`.  Walks backward line-by-line past `#[…]` attribute lines and blank lines.
/// Stops as soon as it encounters a line that is not purely made of `#[…]` attributes
/// (e.g. `#[pymethods]impl Foo {`).  Returns the byte index of the first character of the
/// first method-attribute line (or `fn_idx` when there are none).
fn find_method_attrs_start(code: &str, fn_idx: usize) -> usize {
    let before = &code[..fn_idx];
    // Collect line-start byte offsets so we can walk backward.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(before.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let mut attr_start_byte = fn_idx;
    // Walk the line-start offsets in reverse (skip the last one — that is the `pub fn` line).
    for &line_byte_start in line_starts.iter().rev() {
        let line = &before[line_byte_start..before.len().min(attr_start_byte)];
        let trimmed = line.trim_end_matches('\n').trim();
        if trimmed.is_empty() || is_method_attr_line(trimmed) {
            attr_start_byte = line_byte_start;
        } else {
            break;
        }
    }
    attr_start_byte
}

/// Find the byte index just after the closing `}` of a Rust method block whose `pub fn`
/// starts at byte `fn_idx` in `code`.
fn find_method_end(code: &str, fn_idx: usize) -> Option<usize> {
    let slice = &code[fn_idx..];
    let mut depth = 0usize;
    let mut found_open = false;
    let mut byte_offset = 0usize;
    for ch in slice.chars() {
        match ch {
            '{' => {
                depth += 1;
                found_open = true;
            }
            '}' if found_open => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    byte_offset += ch.len_utf8();
                    return Some(fn_idx + byte_offset);
                }
            }
            _ => {}
        }
        byte_offset += ch.len_utf8();
    }
    None
}
