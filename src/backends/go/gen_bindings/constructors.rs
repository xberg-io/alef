use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::TypeDef;

/// Map a Rust FFI type string (as stored in `ConstructorParam.ty`) to its Go equivalent.
///
/// Only the types actually used in `client_constructors` configs are handled here.
/// Unmapped types fall back to `unsafe.Pointer` with a cast so compilation continues even
/// if the caller passes an exotic type — a compile warning rather than a hard stop.
pub(super) fn ffi_ty_to_go(rust_ty: &str) -> &'static str {
    let normalized = rust_ty.trim();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return "string";
    }
    if matches!(normalized, "u8" | "uint8_t") {
        return "uint8";
    }
    if matches!(normalized, "u16" | "uint16_t") {
        return "uint16";
    }
    if matches!(normalized, "u32" | "uint32_t") {
        return "uint32";
    }
    if matches!(normalized, "u64" | "uint64_t" | "usize") {
        return "uint64";
    }
    if matches!(normalized, "i8" | "int8_t") {
        return "int8";
    }
    if matches!(normalized, "i16" | "int16_t") {
        return "int16";
    }
    if matches!(normalized, "i32" | "int32_t" | "c_int") {
        return "int32";
    }
    if matches!(normalized, "i64" | "int64_t" | "isize") {
        return "int64";
    }
    if matches!(normalized, "bool") {
        return "bool";
    }
    if matches!(normalized, "f32" | "float") {
        return "float32";
    }
    if matches!(normalized, "f64" | "double") {
        return "float64";
    }
    "unsafe.Pointer"
}

/// Emit the CGO conversion for a single constructor param.
///
/// Returns a pair `(c_var_name, setup_lines)` where `c_var_name` is the expression
/// to pass to the C function and `setup_lines` are the Go statements to insert before
/// the call (CString allocation + deferred free, numeric cast, etc.).
pub(super) fn go_ctor_param_setup(go_name: &str, rust_ty: &str, ffi_prefix: &str) -> (String, String) {
    let normalized = rust_ty.trim();
    let c_name = format!("c{}{}", &go_name[..1].to_uppercase(), &go_name[1..]);

    if normalized.contains("c_char") || normalized.contains("CStr") {
        let setup = format!("\t{c_name} := C.CString({go_name})\n\tdefer C.free(unsafe.Pointer({c_name}))\n");
        (c_name, setup)
    } else if matches!(normalized, "bool") {
        let setup = format!("\t{c_name} := C.bool({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "f32" | "float") {
        let setup = format!("\t{c_name} := C.float({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "f64" | "double") {
        let setup = format!("\t{c_name} := C.double({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "u8" | "uint8_t") {
        let setup = format!("\t{c_name} := C.uint8_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "u16" | "uint16_t") {
        let setup = format!("\t{c_name} := C.uint16_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "u32" | "uint32_t") {
        let setup = format!("\t{c_name} := C.uint32_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "u64" | "uint64_t" | "usize") {
        let setup = format!("\t{c_name} := C.uint64_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "i8" | "int8_t") {
        let setup = format!("\t{c_name} := C.int8_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "i16" | "int16_t") {
        let setup = format!("\t{c_name} := C.int16_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "i32" | "int32_t" | "c_int") {
        let setup = format!("\t{c_name} := C.int32_t({go_name})\n");
        (c_name, setup)
    } else if matches!(normalized, "i64" | "int64_t" | "isize") {
        let setup = format!("\t{c_name} := C.int64_t({go_name})\n");
        (c_name, setup)
    } else {
        let _ = ffi_prefix;
        let setup = format!("\t{c_name} := {go_name}\n");
        (c_name, setup)
    }
}

/// Generate a `func New<TypeName>(params...) (*<TypeName>, error)` constructor that
/// wraps the `C.{ffi_prefix}_{type_snake}_new(...)` FFI symbol emitted by the FFI backend.
pub(super) fn gen_go_opaque_constructor(typ: &TypeDef, ffi_prefix: &str, ctor: &ClientConstructorConfig) -> String {
    use crate::codegen::naming::go_type_name;
    use heck::ToSnakeCase;

    let go_name = go_type_name(&typ.name);
    let type_snake = typ.name.to_snake_case();
    let upper_prefix = ffi_prefix.to_uppercase();
    let c_type = format!("{upper_prefix}{}", typ.name);

    let go_params: String = ctor
        .params
        .iter()
        .map(|p| format!("{} {}", p.name, ffi_ty_to_go(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");

    let mut setup = String::new();
    let c_args: Vec<String> = ctor
        .params
        .iter()
        .map(|p| {
            let (c_var, lines) = go_ctor_param_setup(&p.name, &p.ty, ffi_prefix);
            setup.push_str(&lines);
            c_var
        })
        .collect();

    let c_call_args = c_args.join(", ");

    format!(
        "// New{go_name} creates a new {go_name} handle via the FFI constructor.\n\
         func New{go_name}({go_params}) (*{go_name}, error) {{\n\
         {setup}\
         \tptr := C.{ffi_prefix}_{type_snake}_new({c_call_args})\n\
         \tif ptr == nil {{\n\
         \t\treturn nil, fmt.Errorf(\"new{go_name}: %s\", C.GoString(C.{ffi_prefix}_last_error_context()))\n\
         \t}}\n\
         \treturn &{go_name}{{ptr: unsafe.Pointer((*C.{c_type})(ptr))}}, nil\n\
         }}"
    )
}
