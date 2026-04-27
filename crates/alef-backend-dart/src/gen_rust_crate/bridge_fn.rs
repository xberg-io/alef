use alef_core::ir::{FunctionDef, TypeRef};

use super::conversions::{dart_call_arg, frb_rust_type_inner, frb_rust_type_with_source, primitive_name};

pub(crate) fn emit_bridge_fn(
    out: &mut String,
    f: &FunctionDef,
    source_crate_name: &str,
    type_paths: &std::collections::HashMap<String, String>,
) {
    // FRB v2: ordinary public functions need no annotation. A bare `#[frb]`
    // with no arguments is rejected by the macro. Don't emit it.
    let fn_name = &f.name;
    let async_kw = if f.is_async { "async " } else { "" };

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            // Use the source-crate type for Named types so the bridge fn passes
            // them straight through to the underlying Rust API. The `#[frb(mirror(T))]`
            // attribute on the mirror struct tells FRB that the local declaration
            // is layout-identical to the source type — FRB substitutes them on the
            // Dart side but the generated Rust code must still pass the original.
            let rust_ty = frb_rust_type_with_source(&p.ty, p.optional, source_crate_name, type_paths);
            format!("{}: {rust_ty}", p.name)
        })
        .collect();

    // For each parameter, build the call-site expression. The IR records whether
    // the underlying Rust function takes the value by reference (`is_ref`) and
    // the original concrete type (`original_type`). FRB widens primitives to
    // i64/f64 and Strings to `String`, but the source function may want `u16`,
    // `usize`, `&Path`, etc. — emit a cast (`as Foo`) or conversion call.
    let call_args: Vec<String> = f.params.iter().map(|p| dart_call_arg(p)).collect();

    let has_error = f.error_type.is_some();
    let return_ty = if has_error {
        format!(
            "Result<{}, String>",
            frb_rust_type_with_source(&f.return_type, false, source_crate_name, type_paths)
        )
    } else {
        frb_rust_type_with_source(&f.return_type, false, source_crate_name, type_paths)
    };

    out.push_str(&format!(
        "pub {async_kw}fn {fn_name}({}) -> {return_ty} {{\n",
        params.join(", ")
    ));

    // Resolve the call target via the IR's full rust_path, falling back to the
    // backend's bare fn name if rust_path is empty. This matches alef-backend-ffi
    // and ensures functions defined in submodules (e.g. `my_lib::utils::helper`)
    // generate correct shim calls.
    let resolved_path = if f.rust_path.is_empty() {
        format!("{source_crate_name}::{fn_name}")
    } else {
        f.rust_path.replace('-', "_")
    };
    let call = format!("{resolved_path}({})", call_args.join(", "));

    // Handle return-type mismatches: FRB widens primitives but the source fn
    // may return e.g. `u64` or `&str`. Bridge with explicit conversions.
    let result_cast = match &f.return_type {
        TypeRef::Primitive(_) => {
            let target = frb_rust_type_inner(&f.return_type);
            format!(" as {target}")
        }
        // String/Path/Char returns may be `&str`/`&Path`/`&Path` from the source;
        // call .to_string() to ensure an owned String is returned.
        TypeRef::String | TypeRef::Path | TypeRef::Char => ".to_string()".to_string(),
        // Optional<String> with returns_ref=true is alef's fallback for
        // `Option<&'static T>` where T is a non-String type (like EmbeddingPreset).
        // Use Debug format so the Dart side gets a printable representation —
        // `serde_json::to_string` would require T: Serialize which isn't always true.
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path | TypeRef::Char) && f.returns_ref =>
        {
            ".map(|v| format!(\"{:?}\", v))".to_string()
        }
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Primitive(prim) => {
                let target = primitive_name(prim);
                if target == "f64" || target == "i64" || target == "bool" {
                    String::new()
                } else {
                    format!(".into_iter().map(|x| x as {}).collect::<Vec<_>>()", frb_rust_type_inner(inner))
                }
            }
            // Vec<&str> -> Vec<String>
            TypeRef::String | TypeRef::Path | TypeRef::Char => {
                ".into_iter().map(|s| s.to_string()).collect::<Vec<_>>()".to_string()
            }
            _ => String::new(),
        },
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Path | TypeRef::Char => ".map(|s| s.to_string())".to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };

    let body = if has_error {
        if f.is_async {
            format!("    {call}.await.map(|v| v{result_cast}).map_err(|e| e.to_string())\n")
        } else {
            format!("    {call}.map(|v| v{result_cast}).map_err(|e| e.to_string())\n")
        }
    } else if f.is_async {
        if result_cast.is_empty() {
            format!("    {call}.await\n")
        } else {
            format!("    {call}.await{result_cast}\n")
        }
    } else if result_cast.is_empty() {
        format!("    {call}\n")
    } else {
        format!("    {call}{result_cast}\n")
    };

    out.push_str(&body);
    out.push_str("}\n");
}
