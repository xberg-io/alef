use crate::codegen::naming::{pascal_to_snake, to_class_name};
use crate::core::ir::{FunctionDef, MethodDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

use super::support::{ffi_doxygen_block, sanitized_recoverable};

/// Returns true if a method should be skipped from C FFI wrapper generation.
///
/// Methods are skipped if they:
/// 1. Have generic type parameters (detected by parameters with Named types not in the path_map)
/// 2. Return a reference to the receiver type (builder-style methods returning `&mut Self` or `&Self`)
/// 3. Are static constructors on opaque types (handled via gen_opaque_static_constructor instead)
///
/// Such methods are handled through the service-API registration path instead of as
/// standalone C function wrappers.
pub(in crate::backends::ffi::gen_bindings) fn should_skip_method_wrapper(
    method: &MethodDef,
    typ: &TypeDef,
    path_map: &AHashMap<String, String>,
) -> bool {
    for param in &method.params {
        if let TypeRef::Named(name) = &param.ty {
            if !path_map.contains_key(name.as_str()) {
                return true;
            }
        }
    }

    if method.returns_ref {
        if let TypeRef::Named(name) = &method.return_type {
            if name == &typ.name {
                return true;
            }
        }
    }

    if typ.is_opaque
        && method.is_static
        && !matches!(method.name.as_str(), "default" | "to_json" | "from_json" | "clone")
    {
        if let TypeRef::Named(name) = &method.return_type {
            if name == &typ.name {
                return true;
            }
        }
    }

    false
}

pub(super) fn c_symbol_component(name: &str) -> String {
    pascal_to_snake(name)
}

pub(super) fn internal_class_component(name: &str) -> String {
    to_class_name(name)
}

/// Generate a `{ffi_name}_len(same params) -> usize` companion for a free function whose
/// return type maps to `*mut c_char`. The companion returns the byte length recorded by
/// the immediately preceding primary function call on the same thread.
///
/// Enables safe `[]const u8` slice construction in Zig and `MemorySegment` slicing in Java
/// FFM Panama without a NUL-scan or re-running the wrapped Rust operation.
pub(in crate::backends::ffi::gen_bindings) fn gen_free_function_len_companion(
    func: &FunctionDef,
    prefix: &str,
    _core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
) -> String {
    let fn_name_snake = c_symbol_component(&func.name);
    let ffi_name = format!("{prefix}_{fn_name_snake}_len");

    let ffi_param_count = func.params.len() + func.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count();
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let will_be_unimplemented = func.sanitized && !sanitized_recoverable(func);
    let mut params = Vec::new();
    for p in &func.params {
        let param_name = format!("_{}", p.name);
        params.push(format!(
            "    {}: {}",
            param_name,
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                _core_import,
                path_map,
                enum_names,
                false
            )
        ));
        if matches!(p.ty, TypeRef::Bytes) {
            params.push(format!("    _{}_len: usize", p.name));
        }
    }

    let synthetic_doc = format!(
        "Return the byte length of the C string most recently returned by `{prefix}_{fn_name_snake}` \
         on this thread. Returns 0 when the primary call returned null or failed before producing a \
         string. Enables safe slice construction in Zig and Java FFM Panama without a NUL-scan.\n\n\
         # Safety\n\nPointer arguments are ignored and are present only to keep the companion ABI \
         aligned with `{prefix}_{fn_name_snake}`.",
    );
    let doc_comment = ffi_doxygen_block(&synthetic_doc);

    let mut out = String::with_capacity(2048);
    out.push_str(&doc_comment);
    if let Some(ref clippy) = allow_clippy {
        out.push_str(&crate::backends::ffi::template_env::render(
            "ffi_allow_clippy_attr.jinja",
            context! { clippy => clippy.clone() },
        ));
    }
    if let Some(cfg) = func.cfg.as_deref() {
        out.push_str(&format!("#[cfg({cfg})]\n"));
    }
    out.push_str("#[unsafe(no_mangle)]\n");
    out.push_str("pub unsafe extern \"C\" fn ");
    out.push_str(&ffi_name);
    out.push_str("(\n");
    for (i, p) in params.iter().enumerate() {
        out.push_str(p);
        if i + 1 < params.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(") -> usize {\n");

    if will_be_unimplemented {
        out.push_str("    0\n}");
        return out;
    }

    out.push_str("    last_return_len()\n");
    out.push_str("\n}");
    out
}
