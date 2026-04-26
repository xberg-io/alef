use alef_core::config::TraitBridgeConfig;
use alef_core::hash::{self, CommentStyle};
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use heck::ToPascalCase;
/// Generate Go trait bridge interface, CGo callback trampolines, and registration functions.
///
/// # CGo callback strategy
///
/// CGo does not allow passing Go function values as C function pointers. The generated
/// code uses `cgo.Handle` to store the Go object and look it up in trampolines:
///
/// - A `//export go{Trait}{Method}` function receives `user_data unsafe.Pointer`.
/// - Inside the trampoline, `user_data` is converted to a `cgo.Handle` and dereferenced
///   to retrieve the Go object.
/// - The object's method is called, and the result is marshalled back to C.
/// - A static C helper in the CGo preamble constructs the vtable struct with trampoline
///   function pointers — this is valid because CGo compiles the preamble together with
///   the Go file that carries the `//export` declarations.
///
/// # Registration
///
/// `Register{Trait}(impl {Trait}) error` builds the C vtable, calls the C registration
/// function, and returns any error. The handle remains valid for the lifetime of the
/// plugin; a corresponding `Unregister{Trait}(name string) error` removes it.
use std::fmt::Write;

/// Generate the complete trait_bridges.go file content for all configured trait bridges.
///
/// `pkg_name`: Go package name (e.g., `"kreuzberg"`).
/// `ffi_prefix`: C function prefix (e.g., `"kreuzberg"`).
/// `ffi_header`: C header filename (e.g., `"kreuzberg.h"`).
/// `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// `to_root`: relative path from go output dir to the repo root.
/// `crate_name`: Rust FFI crate name (e.g., `"kreuzberg"`), used to derive C type names.
#[allow(clippy::too_many_arguments)]
pub fn gen_trait_bridges_file(
    api: &ApiSurface,
    config: &alef_core::config::AlefConfig,
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    crate_name: &str,
) -> String {
    let mut out = String::with_capacity(16_384);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    writeln!(out, "package {pkg_name}").ok();
    writeln!(out).ok();

    // CGo preamble for trait bridges
    writeln!(out, "/*").ok();
    writeln!(out, "#cgo CFLAGS: -I${{SRCDIR}}/{to_root}{ffi_crate_dir}/include").ok();
    writeln!(out, "#include \"{ffi_header}\"").ok();
    writeln!(out, "#include <stdlib.h>").ok();
    writeln!(out, "#include <string.h>").ok();
    writeln!(out).ok();

    // Forward-declare all exported Go trampolines
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            let pascal = bridge_cfg.trait_name.to_pascal_case();
            for method in &trait_def.methods {
                let export_name = format!("go{}{}", &pascal, method.name.to_pascal_case());
                let c_sig = c_trampoline_signature(&export_name, method);
                writeln!(out, "extern int32_t {}({});", export_name, c_sig).ok();
            }
            // Plugin lifecycle trampolines
            writeln!(out, "extern int32_t go{}Name(void*, char**, char**);", pascal).ok();
            writeln!(out, "extern int32_t go{}Version(void*, char**, char**);", pascal).ok();
            writeln!(out, "extern int32_t go{}Initialize(void*, char**);", pascal).ok();
            writeln!(out, "extern int32_t go{}Shutdown(void*, char**);", pascal).ok();
            writeln!(out, "extern void go{}FreeUserData(void*);", pascal).ok();
        }
    }

    writeln!(out, "*/").ok();
    writeln!(out, "import \"C\"").ok();
    writeln!(out).ok();

    writeln!(out, "import (").ok();
    writeln!(out, "\t\"encoding/json\"").ok();
    writeln!(out, "\t\"fmt\"").ok();
    writeln!(out, "\t\"runtime/cgo\"").ok();
    writeln!(out, "\t\"unsafe\"").ok();
    writeln!(out, ")").ok();
    writeln!(out).ok();

    // Generate interfaces, trampolines, and registration functions for each bridge
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            gen_trait_bridge(&mut out, trait_def, bridge_cfg, ffi_prefix, crate_name);
            writeln!(out).ok();
        }
    }

    out
}

/// Generate one trait bridge: interface, trampolines, registration/unregistration functions.
fn gen_trait_bridge(
    out: &mut String,
    trait_def: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    ffi_prefix: &str,
    crate_name: &str,
) {
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let trait_pascal = trait_name.to_pascal_case();

    // Derive C VTable struct name: {CRATE_UPPER}{CratePascal}{TraitPascal}VTable
    // E.g., for crate="kreuzberg", trait="OcrBackend": KREUZBERGKreuzbergOcrBackendVTable
    // Hyphens in crate names (e.g. "html-to-markdown") are not valid in C identifiers;
    // normalize the same way ffi_prefix does (`-` → `_`) before uppercasing.
    let crate_normalized = crate_name.replace('-', "_");
    let crate_upper = crate_normalized.to_uppercase();
    let crate_pascal = crate_normalized.to_pascal_case();
    let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

    // =========================================================================
    // Go interface
    // =========================================================================
    writeln!(
        out,
        "// {trait_name} defines the Go interface for the {trait_name} trait."
    )
    .ok();
    writeln!(out, "type {trait_name} interface {{").ok();

    // Plugin methods (name, version, initialize, shutdown)
    writeln!(out, "\t// Name returns the plugin name.").ok();
    writeln!(out, "\tName() string").ok();
    writeln!(out).ok();

    writeln!(out, "\t// Version returns the plugin version.").ok();
    writeln!(out, "\tVersion() string").ok();
    writeln!(out).ok();

    writeln!(out, "\t// Initialize is called when the plugin is loaded.").ok();
    writeln!(out, "\tInitialize() error").ok();
    writeln!(out).ok();

    writeln!(out, "\t// Shutdown is called when the plugin is unloaded.").ok();
    writeln!(out, "\tShutdown() error").ok();
    writeln!(out).ok();

    // Trait methods
    for method in &trait_def.methods {
        gen_interface_method(out, method);
    }

    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // =========================================================================
    // Exported trampolines
    // =========================================================================
    for method in &trait_def.methods {
        gen_trampoline(out, trait_name, &trait_pascal, method);
    }

    // Plugin method trampolines
    gen_plugin_trampolines(out, trait_name, &trait_pascal);

    // =========================================================================
    // Registration function
    // =========================================================================
    writeln!(
        out,
        "// Register{trait_name} registers a {trait_name} implementation with the C runtime."
    )
    .ok();
    writeln!(out, "func Register{trait_name}(impl {trait_name}) error {{").ok();
    writeln!(out, "\thandle := cgo.NewHandle(impl)").ok();
    writeln!(out).ok();

    writeln!(
        out,
        "\t// Build the C vtable  DEBUG:c_vtable_struct={}",
        c_vtable_struct
    )
    .ok();
    writeln!(out, "\tvtable := C.{}{{", c_vtable_struct).ok();

    // Set up vtable function pointers (via //export trampolines)
    for method in &trait_def.methods {
        let export_name = format!("go{}{}", &trait_pascal, method.name.to_pascal_case());
        writeln!(out, "\t\t{}: C.{},", &method.name, export_name).ok();
    }

    // Plugin method pointers (cbindgen suffixes lifecycle hooks with `_fn`).
    writeln!(out, "\t\tname_fn: C.go{}Name,", &trait_pascal).ok();
    writeln!(out, "\t\tversion_fn: C.go{}Version,", &trait_pascal).ok();
    writeln!(out, "\t\tinitialize_fn: C.go{}Initialize,", &trait_pascal).ok();
    writeln!(out, "\t\tshutdown_fn: C.go{}Shutdown,", &trait_pascal).ok();

    // free_user_data deletes the cgo.Handle when the bridge is dropped by Rust
    writeln!(out, "\t\tfree_user_data: C.go{}FreeUserData,", &trait_pascal).ok();

    writeln!(out, "\t}}").ok();
    writeln!(out).ok();

    writeln!(out, "\t// Call C registration").ok();
    writeln!(out, "\tcName := C.CString(impl.Name())").ok();
    writeln!(out, "\tdefer C.free(unsafe.Pointer(cName))").ok();
    writeln!(out).ok();

    writeln!(out, "\tvar cErr *C.char").ok();
    writeln!(out, "\trc := C.{}_register_{trait_snake}(", ffi_prefix).ok();
    writeln!(out, "\t\tcName,").ok();
    writeln!(out, "\t\tvtable,").ok();
    writeln!(out, "\t\tunsafe.Pointer(uintptr(handle)),").ok();
    writeln!(out, "\t\t&cErr,").ok();
    writeln!(out, "\t)").ok();
    writeln!(out).ok();

    writeln!(out, "\tif rc != 0 {{").ok();
    writeln!(out, "\t\tmsg := \"failed to register {trait_name}\"").ok();
    writeln!(out, "\t\tif cErr != nil {{").ok();
    writeln!(out, "\t\t\tmsg = C.GoString(cErr)").ok();
    writeln!(out, "\t\t\tC.free(unsafe.Pointer(cErr))").ok();
    writeln!(out, "\t\t}}").ok();
    writeln!(out, "\t\thandle.Delete()").ok();
    writeln!(out, "\t\treturn fmt.Errorf(\"%s\", msg)").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out).ok();

    writeln!(out, "\treturn nil").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // =========================================================================
    // Unregistration function
    // =========================================================================
    writeln!(
        out,
        "// Unregister{trait_name} unregisters a {trait_name} implementation."
    )
    .ok();
    writeln!(out, "func Unregister{trait_name}(name string) error {{").ok();
    writeln!(out, "\tcName := C.CString(name)").ok();
    writeln!(out, "\tdefer C.free(unsafe.Pointer(cName))").ok();
    writeln!(out).ok();

    writeln!(out, "\tvar cErr *C.char").ok();
    writeln!(out, "\trc := C.{}_unregister_{trait_snake}(cName, &cErr)", ffi_prefix).ok();
    writeln!(out).ok();

    writeln!(out, "\tif rc != 0 {{").ok();
    writeln!(out, "\t\tmsg := \"failed to unregister {trait_name}\"").ok();
    writeln!(out, "\t\tif cErr != nil {{").ok();
    writeln!(out, "\t\t\tmsg = C.GoString(cErr)").ok();
    writeln!(out, "\t\t\tC.free(unsafe.Pointer(cErr))").ok();
    writeln!(out, "\t\t}}").ok();
    writeln!(out, "\t\treturn fmt.Errorf(\"%s\", msg)").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out).ok();

    writeln!(out, "\treturn nil").ok();
    writeln!(out, "}}").ok();
}

/// Generate the Go interface method signature for a trait method.
fn gen_interface_method(out: &mut String, method: &MethodDef) {
    let mut params = Vec::new();
    for p in &method.params {
        let go_type = rust_to_go_type(&p.ty);
        params.push(format!("{} {}", p.name, go_type));
    }

    let return_type = if method.error_type.is_some() {
        match &method.return_type {
            TypeRef::Unit => "error".to_string(),
            _ => {
                let ret = rust_to_go_type(&method.return_type);
                format!("({}, error)", ret)
            }
        }
    } else {
        rust_to_go_type(&method.return_type)
    };

    let params_str = params.join(", ");
    writeln!(out, "\t// {}.", method.name).ok();
    writeln!(
        out,
        "\t{}({}) {}",
        method.name.to_pascal_case(),
        params_str,
        return_type
    )
    .ok();
    writeln!(out).ok();
}

/// Generate one trampoline function (implementation without //export).
/// The //export declaration is in binding.go to avoid duplicate definitions.
fn gen_trampoline(out: &mut String, trait_name: &str, trait_pascal: &str, method: &MethodDef) {
    let export_name = format!("go{}{}", trait_pascal, method.name.to_pascal_case());

    writeln!(out, "func {}(", export_name).ok();
    writeln!(out, "\tuserData unsafe.Pointer,").ok();

    for p in &method.params {
        let c_type = rust_to_c_type(&p.ty);
        writeln!(out, "\t{} {},", p.name, c_type).ok();
    }

    // Add outResult if method returns a value (non-unit return type)
    if !matches!(method.return_type, TypeRef::Unit) {
        writeln!(out, "\toutResult **C.char,").ok();
    }
    writeln!(out, "\toutError **C.char,").ok();
    writeln!(out, ") C.int32_t {{").ok();

    // Retrieve the Go object from the handle
    writeln!(out, "\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))").ok();
    writeln!(out, "\timpl, ok := handle.Value().({trait_name})").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 1  // error: invalid handle").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out).ok();

    // Convert C parameters to Go
    for p in &method.params {
        gen_param_conversion(out, p);
    }

    // Call the method
    let mut call_args = Vec::new();
    for p in &method.params {
        call_args.push(format!("go{}", capitalize(&p.name)));
    }

    writeln!(out, "\t// Call the method").ok();
    if method.error_type.is_some() {
        writeln!(
            out,
            "\tresult, err := impl.{}({})",
            method.name.to_pascal_case(),
            call_args.join(", ")
        )
        .ok();
        writeln!(out, "\tif err != nil {{").ok();
        writeln!(out, "\t\tcErr := C.CString(err.Error())").ok();
        writeln!(out, "\t\t*outError = cErr").ok();
        writeln!(out, "\t\treturn 1").ok();
        writeln!(out, "\t}}").ok();

        // Encode result
        match &method.return_type {
            TypeRef::Unit => {}
            _ => {
                writeln!(out, "\tjsonBytes, _ := json.Marshal(result)").ok();
                writeln!(out, "\tcResult := C.CString(string(jsonBytes))").ok();
                writeln!(out, "\t*outResult = cResult").ok();
            }
        }
    } else {
        writeln!(
            out,
            "\tresult := impl.{}({})",
            method.name.to_pascal_case(),
            call_args.join(", ")
        )
        .ok();

        // Encode result if not Unit
        if !matches!(&method.return_type, TypeRef::Unit) {
            writeln!(out, "\tjsonBytes, _ := json.Marshal(result)").ok();
            writeln!(out, "\tcResult := C.CString(string(jsonBytes))").ok();
            writeln!(out, "\t*outResult = cResult").ok();
        }
    }

    writeln!(out, "\treturn 0  // success").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
}

/// Generate trampolines for plugin methods: Name, Version, Initialize, Shutdown.
fn gen_plugin_trampolines(out: &mut String, trait_name: &str, trait_pascal: &str) {
    // Name trampoline
    writeln!(
        out,
        "func go{}Name(userData unsafe.Pointer, outResult **C.char, outError **C.char) C.int32_t {{",
        trait_pascal
    )
    .ok();
    writeln!(out, "\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))").ok();
    writeln!(out, "\timpl, ok := handle.Value().({trait_name})").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tname := impl.Name()").ok();
    writeln!(out, "\tcName := C.CString(name)").ok();
    writeln!(out, "\t*outResult = cName").ok();
    writeln!(out, "\treturn 0").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Version trampoline
    writeln!(
        out,
        "func go{}Version(userData unsafe.Pointer, outResult **C.char, outError **C.char) C.int32_t {{",
        trait_pascal
    )
    .ok();
    writeln!(out, "\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))").ok();
    writeln!(out, "\timpl, ok := handle.Value().({trait_name})").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\tversion := impl.Version()").ok();
    writeln!(out, "\tcVersion := C.CString(version)").ok();
    writeln!(out, "\t*outResult = cVersion").ok();
    writeln!(out, "\treturn 0").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Initialize trampoline
    writeln!(
        out,
        "func go{}Initialize(userData unsafe.Pointer, outError **C.char) C.int32_t {{",
        trait_pascal
    )
    .ok();
    writeln!(out, "\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))").ok();
    writeln!(out, "\timpl, ok := handle.Value().({trait_name})").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\terr := impl.Initialize()").ok();
    writeln!(out, "\tif err != nil {{").ok();
    writeln!(out, "\t\tcErr := C.CString(err.Error())").ok();
    writeln!(out, "\t\t*outError = cErr").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn 0").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // Shutdown trampoline
    writeln!(
        out,
        "func go{}Shutdown(userData unsafe.Pointer, outError **C.char) C.int32_t {{",
        trait_pascal
    )
    .ok();
    writeln!(out, "\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))").ok();
    writeln!(out, "\timpl, ok := handle.Value().({trait_name})").ok();
    writeln!(out, "\tif !ok {{").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\terr := impl.Shutdown()").ok();
    writeln!(out, "\tif err != nil {{").ok();
    writeln!(out, "\t\tcErr := C.CString(err.Error())").ok();
    writeln!(out, "\t\t*outError = cErr").ok();
    writeln!(out, "\t\treturn 1").ok();
    writeln!(out, "\t}}").ok();
    writeln!(out, "\treturn 0").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();

    // FreeUserData trampoline — called by Rust Drop to delete the cgo.Handle
    writeln!(out, "func go{}FreeUserData(userData unsafe.Pointer) {{", trait_pascal).ok();
    writeln!(out, "\tcgo.Handle(uintptr(unsafe.Pointer(userData))).Delete()").ok();
    writeln!(out, "}}").ok();
    writeln!(out).ok();
}

/// Build the C trampoline function signature for extern declaration in the CGo preamble.
/// Uses actual C types (not Go CGo types like `C.int32_t`).
fn c_trampoline_signature(_export_name: &str, method: &MethodDef) -> String {
    let mut params = vec!["void* user_data".to_string()];
    for p in &method.params {
        let cty = rust_to_plain_c_type(&p.ty);
        params.push(format!("{} {}", cty, p.name));
    }
    if !matches!(method.return_type, TypeRef::Unit) {
        params.push("char** out_result".to_string());
    }
    params.push("char** out_error".to_string());
    params.join(", ")
}

/// Convert a Rust TypeRef to a plain C type string (for CGo preamble extern declarations).
fn rust_to_plain_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType::*;
            match p {
                Bool => "int32_t",
                U8 => "uint8_t",
                U16 => "uint16_t",
                U32 => "uint32_t",
                U64 => "uint64_t",
                I8 => "int8_t",
                I16 => "int16_t",
                I32 => "int32_t",
                I64 => "int64_t",
                F32 => "float",
                F64 => "double",
                Usize => "size_t",
                Isize => "intptr_t",
            }
            .to_string()
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => "char*".to_string(),
        TypeRef::Bytes => "uint8_t*".to_string(),
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) => "char*".to_string(),
        TypeRef::Unit => "void".to_string(),
        TypeRef::Duration => "uint64_t".to_string(),
        _ => "char*".to_string(),
    }
}

/// Convert a Rust TypeRef to a Go type string.
fn rust_to_go_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType::*;
            match p {
                Bool => "bool",
                U8 => "uint8",
                U16 => "uint16",
                U32 => "uint32",
                U64 => "uint64",
                I8 => "int8",
                I16 => "int16",
                I32 => "int32",
                I64 => "int64",
                F32 => "float32",
                F64 => "float64",
                Usize => "uint",
                Isize => "int",
            }
            .to_string()
        }
        TypeRef::String => "string".to_string(),
        TypeRef::Char => "rune".to_string(),
        TypeRef::Bytes => "[]byte".to_string(),
        TypeRef::Optional(inner) => format!("*{}", rust_to_go_type(inner)),
        TypeRef::Vec(inner) => format!("[]{}", rust_to_go_type(inner)),
        TypeRef::Map(k, v) => format!("map[{}]{}", rust_to_go_type(k), rust_to_go_type(v)),
        TypeRef::Unit => "error".to_string(), // void → error in Go
        TypeRef::Duration => "time.Duration".to_string(),
        TypeRef::Named(_) => "map[string]interface{}".to_string(), // JSON for complex types
        _ => "interface{}".to_string(),
    }
}

/// Convert a Rust TypeRef to a C type string.
fn rust_to_c_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType::*;
            match p {
                Bool => "C.int32_t",
                U8 => "C.uint8_t",
                U16 => "C.uint16_t",
                U32 => "C.uint32_t",
                U64 => "C.uint64_t",
                I8 => "C.int8_t",
                I16 => "C.int16_t",
                I32 => "C.int32_t",
                I64 => "C.int64_t",
                F32 => "C.float",
                F64 => "C.double",
                Usize => "C.size_t",
                Isize => "C.intptr_t",
            }
            .to_string()
        }
        TypeRef::String | TypeRef::Char | TypeRef::Path => "*C.char".to_string(),
        TypeRef::Bytes => "*C.uint8_t".to_string(),
        TypeRef::Optional(_) => "*C.char".to_string(), // JSON-encoded
        TypeRef::Vec(_) => "*C.char".to_string(),      // JSON-encoded
        TypeRef::Map(_, _) => "*C.char".to_string(),   // JSON-encoded
        TypeRef::Unit => "C.void".to_string(),
        TypeRef::Duration => "C.uint64_t".to_string(),
        TypeRef::Named(_) => "*C.char".to_string(), // JSON-encoded
        _ => "*C.char".to_string(),
    }
}

/// Generate parameter conversion code (C to Go).
fn gen_param_conversion(out: &mut String, param: &alef_core::ir::ParamDef) {
    let var_name = format!("go{}", capitalize(&param.name));
    match &param.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            writeln!(out, "\tgo{} := C.GoString({})", capitalize(&param.name), param.name).ok();
            writeln!(out).ok();
        }
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_) => {
            writeln!(out, "\tvar {} interface{{}}", var_name).ok();
            writeln!(out, "\tif {} != nil {{", param.name).ok();
            writeln!(
                out,
                "\t\tjson.Unmarshal([]byte(C.GoString({})), &{})",
                param.name, var_name
            )
            .ok();
            writeln!(out, "\t}}").ok();
            writeln!(out).ok();
        }
        TypeRef::Primitive(p) => {
            use alef_core::ir::PrimitiveType::*;
            let cast = match p {
                Bool => format!("{} != 0", param.name),
                _ => {
                    // Get the Go type for this primitive
                    let go_type = match p {
                        U8 => "uint8",
                        U16 => "uint16",
                        U32 => "uint32",
                        U64 => "uint64",
                        I8 => "int8",
                        I16 => "int16",
                        I32 => "int32",
                        I64 => "int64",
                        F32 => "float32",
                        F64 => "float64",
                        Usize => "uint",
                        Isize => "int",
                        _ => "",
                    };
                    format!("{}({})", go_type, param.name)
                }
            };
            writeln!(out, "\t{} := {}", var_name, cast).ok();
            writeln!(out).ok();
        }
        _ => {
            writeln!(out, "\t{} := {}", var_name, param.name).ok();
            writeln!(out).ok();
        }
    }
}

/// Capitalize the first character of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vtable_struct_name_derivation() {
        // Test the pattern: {CRATE_UPPER}{CratePascal}{TraitPascal}VTable
        let crate_name = "kreuzberg";
        let crate_upper = crate_name.to_uppercase();
        let crate_pascal = crate_name.to_pascal_case();
        let trait_name = "OcrBackend";
        let trait_pascal = trait_name.to_pascal_case();

        let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

        assert_eq!(c_vtable_struct, "KREUZBERGKreuzbergOcrBackendVTable");
    }

    #[test]
    fn test_register_function_name_format() {
        // Test the pattern: {ffi_prefix}_register_{trait_snake}
        let ffi_prefix = "kreuzberg";
        let trait_name = "OcrBackend";
        let trait_snake = heck::AsSnakeCase(trait_name).to_string();

        let register_fn = format!("{}_register_{}", ffi_prefix, trait_snake);
        assert_eq!(register_fn, "kreuzberg_register_ocr_backend");
    }

    #[test]
    fn test_unregister_function_name_format() {
        // Test the pattern: {ffi_prefix}_unregister_{trait_snake}
        let ffi_prefix = "kreuzberg";
        let trait_name = "PostProcessor";
        let trait_snake = heck::AsSnakeCase(trait_name).to_string();

        let unregister_fn = format!("{}_unregister_{}", ffi_prefix, trait_snake);
        assert_eq!(unregister_fn, "kreuzberg_unregister_post_processor");
    }

    #[test]
    fn test_vtable_struct_name_multiple_traits() {
        // Verify correct naming for multiple traits
        let test_cases = vec![
            ("kreuzberg", "OcrBackend", "KREUZBERGKreuzbergOcrBackendVTable"),
            ("kreuzberg", "PostProcessor", "KREUZBERGKreuzbergPostProcessorVTable"),
            ("kreuzberg", "Validator", "KREUZBERGKreuzbergValidatorVTable"),
            (
                "kreuzberg",
                "EmbeddingBackend",
                "KREUZBERGKreuzbergEmbeddingBackendVTable",
            ),
        ];

        for (crate_name, trait_name, expected_struct) in test_cases {
            let crate_upper = crate_name.to_uppercase();
            let crate_pascal = crate_name.to_pascal_case();
            let trait_pascal = trait_name.to_pascal_case();
            let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

            assert_eq!(
                c_vtable_struct, expected_struct,
                "Mismatch for crate={}, trait={}",
                crate_name, trait_name
            );
        }
    }
}
