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
///
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
    config: &alef_core::config::ResolvedCrateConfig,
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    crate_name: &str,
) -> String {
    let mut out = String::with_capacity(16_384);

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    // NOTE: package_and_cgo.jinja already emits "package {name}\n\n/*\n#cgo..."
    // so we render it directly — do NOT push a separate "/*\n" before this call.
    out.push_str(&crate::template_env::render(
        "package_and_cgo.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            to_root => to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_header => ffi_header,
        },
    ));
    out.push('\n');

    // Forward-declare all exported Go trampolines
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            let pascal = bridge_cfg.trait_name.to_pascal_case();
            for method in trait_def
                .methods
                .iter()
                .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
            {
                let export_name = format!("go{}{}", &pascal, method.name.to_pascal_case());
                let c_sig = c_trampoline_signature(&export_name, method);
                out.push_str(&crate::template_env::render(
                    "extern_trampoline_decl.jinja",
                    minijinja::context! {
                        export_name => export_name,
                        c_sig => c_sig,
                    },
                ));
            }
            // Plugin lifecycle trampolines
            out.push_str(&crate::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Name",
                },
            ));
            out.push_str(&crate::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Version",
                },
            ));
            out.push_str(&crate::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Initialize",
                },
            ));
            out.push_str(&crate::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Shutdown",
                },
            ));
            out.push_str(&crate::template_env::render(
                "plugin_free_user_data_extern.jinja",
                minijinja::context! {
                    pascal => &pascal,
                },
            ));
        }
    }

    out.push_str("*/\n");
    out.push_str("import \"C\"\n");
    out.push('\n');

    out.push_str("import (\n");
    out.push_str("\t\"encoding/base64\"\n");
    out.push_str("\t\"encoding/json\"\n");
    out.push_str("\t\"fmt\"\n");
    out.push_str("\t\"runtime/cgo\"\n");
    out.push_str("\t\"unsafe\"\n");
    out.push_str(")\n");
    out.push('\n');

    // Generate interfaces, trampolines, and registration functions for each bridge
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            gen_trait_bridge(&mut out, trait_def, bridge_cfg, ffi_prefix, crate_name);
            out.push('\n');
        }
    }

    out
}

/// Generate one trait bridge: interface, trampolines, registration/unregistration functions.
fn gen_trait_bridge(
    out: &mut String,
    trait_def: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
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
    out.push_str(&crate::template_env::render(
        "trait_interface_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    // Plugin methods (name, version, initialize, shutdown)
    out.push_str(&crate::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Name returns the plugin name.",
            method => "Name",
            return_type => "string",
        },
    ));

    out.push_str(&crate::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Version returns the plugin version.",
            method => "Version",
            return_type => "string",
        },
    ));

    out.push_str(&crate::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Initialize is called when the plugin is loaded.",
            method => "Initialize",
            return_type => "error",
        },
    ));

    out.push_str(&crate::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Shutdown is called when the plugin is unloaded.",
            method => "Shutdown",
            return_type => "error",
        },
    ));

    // Trait methods (skip FFI-incompatible ones — they have no C VTable slot).
    for method in trait_def
        .methods
        .iter()
        .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
    {
        gen_interface_method(out, method);
    }

    out.push_str("}\n");
    out.push('\n');

    // =========================================================================
    // Exported trampolines
    // =========================================================================
    for method in trait_def
        .methods
        .iter()
        .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
    {
        let export_name = format!("go{}{}", &trait_pascal, method.name.to_pascal_case());
        out.push_str(&crate::template_env::render(
            "export_marker.jinja",
            minijinja::context! {
                name => &export_name,
            },
        ));
        out.push('\n');
        gen_trampoline(out, trait_name, &trait_pascal, method);
    }

    // Plugin method trampolines
    gen_plugin_trampolines(out, trait_name, &trait_pascal);

    // =========================================================================
    // Registration function
    // =========================================================================
    out.push_str(&crate::template_env::render(
        "register_function_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    out.push_str(&crate::template_env::render(
        "vtable_struct_init.jinja",
        minijinja::context! {
            c_vtable_struct => &c_vtable_struct,
        },
    ));

    // Set up vtable function pointers (via //export trampolines).
    // cgo declares function pointers as *[0]byte, so cast via unsafe.Pointer.
    // Skip FFI-incompatible methods — they have no C VTable slot.
    for method in trait_def
        .methods
        .iter()
        .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
    {
        let export_name = format!("go{}{}", &trait_pascal, method.name.to_pascal_case());
        out.push_str(&crate::template_env::render(
            "register_vtable_method_field.jinja",
            minijinja::context! {
                method_name => &method.name,
                export_name => export_name,
            },
        ));
    }

    // Plugin method pointers (cbindgen suffixes lifecycle hooks with `_fn`).
    out.push_str(&crate::template_env::render(
        "plugin_trampoline_lifecycle.jinja",
        minijinja::context! {
            field => "name_fn",
            pascal => &trait_pascal,
            method => "Name",
        },
    ));
    out.push_str(&crate::template_env::render(
        "plugin_trampoline_lifecycle.jinja",
        minijinja::context! {
            field => "version_fn",
            pascal => &trait_pascal,
            method => "Version",
        },
    ));
    out.push_str(&crate::template_env::render(
        "plugin_trampoline_lifecycle.jinja",
        minijinja::context! {
            field => "initialize_fn",
            pascal => &trait_pascal,
            method => "Initialize",
        },
    ));
    out.push_str(&crate::template_env::render(
        "plugin_trampoline_lifecycle.jinja",
        minijinja::context! {
            field => "shutdown_fn",
            pascal => &trait_pascal,
            method => "Shutdown",
        },
    ));

    // free_user_data deletes the cgo.Handle when the bridge is dropped by Rust
    out.push_str(&crate::template_env::render(
        "vtable_free_user_data_field.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
        },
    ));

    out.push_str("\t}\n");
    out.push('\n');

    out.push_str(&crate::template_env::render(
        "register_c_call.jinja",
        minijinja::context! {
            c_function => format!("{}_register_{}", ffi_prefix, trait_snake),
            trait_name => trait_name,
        },
    ));
    out.push_str("}\n");
    out.push('\n');

    // =========================================================================
    // Unregistration function
    // =========================================================================
    out.push_str(&crate::template_env::render(
        "unregister_function_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    out.push_str(&crate::template_env::render(
        "unregister_c_call.jinja",
        minijinja::context! {
            c_function => format!("{}_unregister_{}", ffi_prefix, trait_snake),
            trait_name => trait_name,
        },
    ));
    out.push_str("}\n");

    // =========================================================================
    // Config-driven unregistration / clear functions (opt-in via bridge_cfg)
    // =========================================================================
    let unregister_block = gen_unregistration_fn(bridge_cfg, ffi_prefix, trait_name);
    if !unregister_block.is_empty() {
        out.push('\n');
        out.push_str(&unregister_block);
    }

    let clear_block = gen_clear_fn(bridge_cfg, ffi_prefix, trait_name);
    if !clear_block.is_empty() {
        out.push('\n');
        out.push_str(&clear_block);
    }
}

/// Generate a config-driven unregistration wrapper.
///
/// Returns an empty string when `bridge_cfg.unregister_fn` is `None`.
/// Otherwise emits a Go function whose name is `bridge_cfg.unregister_fn`,
/// accepting a `name string` parameter and calling the C-exported
/// `{ffi_prefix}_unregister_{trait_snake}` function via cgo.
fn gen_unregistration_fn(bridge_cfg: &TraitBridgeConfig, ffi_prefix: &str, trait_name: &str) -> String {
    let Some(fn_name) = bridge_cfg.unregister_fn.as_deref() else {
        return String::new();
    };
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let c_function = format!("{}_unregister_{}", ffi_prefix, trait_snake);

    let mut out = String::new();
    out.push_str(&crate::template_env::render(
        "unregister_fn_header.jinja",
        minijinja::context! {
            fn_name => fn_name,
            trait_name => trait_name,
        },
    ));
    out.push_str(&crate::template_env::render(
        "unregister_c_call.jinja",
        minijinja::context! {
            c_function => c_function,
            trait_name => trait_name,
        },
    ));
    out.push_str("}\n");
    out
}

/// Generate a config-driven clear-all wrapper.
///
/// Returns an empty string when `bridge_cfg.clear_fn` is `None`.
/// Otherwise emits a Go function whose name is `bridge_cfg.clear_fn`,
/// taking no arguments and calling the C-exported
/// `{ffi_prefix}_clear_{trait_snake}` function via cgo.
fn gen_clear_fn(bridge_cfg: &TraitBridgeConfig, ffi_prefix: &str, trait_name: &str) -> String {
    let Some(fn_name) = bridge_cfg.clear_fn.as_deref() else {
        return String::new();
    };
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let c_function = format!("{}_clear_{}", ffi_prefix, trait_snake);

    let mut out = String::new();
    out.push_str(&crate::template_env::render(
        "clear_function_header.jinja",
        minijinja::context! {
            fn_name => fn_name,
            name => trait_name,
        },
    ));
    out.push_str(&crate::template_env::render(
        "clear_c_call.jinja",
        minijinja::context! {
            c_function => c_function,
            trait_name => trait_name,
        },
    ));
    out.push_str("}\n");
    out
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
    out.push_str(&crate::template_env::render(
        "trait_interface_method.jinja",
        minijinja::context! {
            doc => &method.name,
            method_name => method.name.to_pascal_case(),
            params => params_str,
            return_type => return_type,
        },
    ));
    out.push('\n');
}

/// Generate one trampoline function (implementation without //export).
/// The //export declaration is in binding.go to avoid duplicate definitions.
fn gen_trampoline(out: &mut String, trait_name: &str, trait_pascal: &str, method: &MethodDef) {
    let export_name = format!("go{}{}", trait_pascal, method.name.to_pascal_case());

    let mut params = vec!["userData unsafe.Pointer".to_string()];
    for p in &method.params {
        let c_type = rust_to_c_type(&p.ty);
        params.push(format!("{} {}", p.name, c_type));
    }
    if !matches!(method.return_type, TypeRef::Unit) {
        params.push("outResult **C.char".to_string());
    }
    params.push("outError **C.char".to_string());

    out.push_str(&crate::template_env::render(
        "trampoline_signature.jinja",
        minijinja::context! {
            name => export_name,
            params => params,
        },
    ));
    out.push('\n');

    // Retrieve the Go object from the handle
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1  // error: invalid handle\n");
    out.push_str("\t}\n");
    out.push('\n');

    // Convert C parameters to Go
    for p in &method.params {
        gen_param_conversion(out, p);
    }

    // Call the method
    let mut call_args = Vec::new();
    for p in &method.params {
        call_args.push(format!("go{}", capitalize(&p.name)));
    }

    out.push_str("\t// Call the method\n");
    if method.error_type.is_some() {
        // Method returns (value?, error)
        match &method.return_type {
            TypeRef::Unit => {
                // Just returns error
                out.push_str(&crate::template_env::render(
                    "impl_method_call_err.jinja",
                    minijinja::context! {
                        method => method.name.to_pascal_case(),
                        args => call_args.join(", "),
                    },
                ));
                out.push('\n');
            }
            _ => {
                // Returns (value, error)
                out.push_str(&crate::template_env::render(
                    "impl_method_call_result_err.jinja",
                    minijinja::context! {
                        method => method.name.to_pascal_case(),
                        args => call_args.join(", "),
                    },
                ));
                out.push('\n');
            }
        }
        out.push_str("\tif err != nil {\n");
        out.push_str("\t\tcErr := C.CString(err.Error())\n");
        out.push_str("\t\t*outError = cErr\n");
        out.push_str("\t\treturn 1\n");
        out.push_str("\t}\n");

        // Encode result if not Unit
        if !matches!(&method.return_type, TypeRef::Unit) {
            out.push_str("\tjsonBytes, _ := json.Marshal(result)\n");
            out.push_str("\tcResult := C.CString(string(jsonBytes))\n");
            out.push_str("\t*outResult = cResult\n");
        }
    } else {
        // Method returns only value (no error)
        out.push_str(&crate::template_env::render(
            "impl_method_call_result.jinja",
            minijinja::context! {
                method => method.name.to_pascal_case(),
                args => call_args.join(", "),
            },
        ));
        out.push('\n');

        // Encode result if not Unit
        if !matches!(&method.return_type, TypeRef::Unit) {
            out.push_str("\tjsonBytes, _ := json.Marshal(result)\n");
            out.push_str("\tcResult := C.CString(string(jsonBytes))\n");
            out.push_str("\t*outResult = cResult\n");
        }
    }

    out.push_str("\treturn 0  // success\n");
    out.push_str("}\n");
    out.push('\n');
}

/// Generate trampolines for plugin methods: Name, Version, Initialize, Shutdown.
fn gen_plugin_trampolines(out: &mut String, trait_name: &str, trait_pascal: &str) {
    // Name trampoline
    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Name"),
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Name",
            params => "userData unsafe.Pointer, outResult **C.char, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\tname := impl.Name()\n");
    out.push_str("\tcName := C.CString(name)\n");
    out.push_str("\t*outResult = cName\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Version trampoline
    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Version"),
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Version",
            params => "userData unsafe.Pointer, outResult **C.char, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\tversion := impl.Version()\n");
    out.push_str("\tcVersion := C.CString(version)\n");
    out.push_str("\t*outResult = cVersion\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Initialize trampoline
    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Initialize"),
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Initialize",
            params => "userData unsafe.Pointer, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\terr := impl.Initialize()\n");
    out.push_str("\tif err != nil {\n");
    out.push_str("\t\tcErr := C.CString(err.Error())\n");
    out.push_str("\t\t*outError = cErr\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // Shutdown trampoline
    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}Shutdown"),
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "plugin_method_trampoline_header.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
            method => "Shutdown",
            params => "userData unsafe.Pointer, outError **C.char",
        },
    ));
    out.push('\n');
    out.push_str("\thandle := cgo.Handle(uintptr(unsafe.Pointer(userData)))\n");
    out.push_str(&crate::template_env::render(
        "handle_type_assertion.jinja",
        minijinja::context! {
            type_name => trait_name,
        },
    ));
    out.push('\n');
    out.push_str("\tif !ok {\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\terr := impl.Shutdown()\n");
    out.push_str("\tif err != nil {\n");
    out.push_str("\t\tcErr := C.CString(err.Error())\n");
    out.push_str("\t\t*outError = cErr\n");
    out.push_str("\t\treturn 1\n");
    out.push_str("\t}\n");
    out.push_str("\treturn 0\n");
    out.push_str("}\n");
    out.push('\n');

    // FreeUserData trampoline — called by Rust Drop to delete the cgo.Handle
    out.push_str(&crate::template_env::render(
        "export_marker.jinja",
        minijinja::context! {
            name => format!("go{trait_pascal}FreeUserData"),
        },
    ));
    out.push('\n');
    out.push_str(&crate::template_env::render(
        "plugin_free_user_data_func.jinja",
        minijinja::context! {
            pascal => &trait_pascal,
        },
    ));
    out.push('\n');
    out.push_str("\tcgo.Handle(uintptr(unsafe.Pointer(userData))).Delete()\n");
    out.push_str("}\n");
    out.push('\n');
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
            out.push_str(&crate::template_env::render(
                "go_string_cast.jinja",
                minijinja::context! {
                    name => capitalize(&param.name),
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
        }
        TypeRef::Bytes => {
            // Bytes are JSON-encoded (base64) like other complex types across FFI
            out.push_str(&crate::template_env::render(
                "var_bytes_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                },
            ));
            out.push_str(&crate::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str("\t\tvar b64str string\n");
            out.push_str(&crate::template_env::render(
                "json_unmarshal_unsafe.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push_str("\t\tif decoded, err := base64.StdEncoding.DecodeString(b64str); err == nil {\n");
            out.push_str(&crate::template_env::render(
                "var_assign.jinja",
                minijinja::context! {
                    var => &var_name,
                    expr => "decoded",
                },
            ));
            out.push_str("\t\t}\n");
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Vec(_) => {
            // Vec types unmarshal directly from JSON array
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str(&crate::template_env::render(
                "json_unmarshal_simple.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                    var_name => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Map(_, _) | TypeRef::Named(_) => {
            // Map and named types unmarshal as map[string]interface{}
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str("\t\tvar rawData interface{}\n");
            out.push_str(&crate::template_env::render(
                "json_unmarshal_rawdata.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push_str("\t\tif m, ok := rawData.(map[string]interface{}); ok {\n");
            out.push_str(&crate::template_env::render(
                "var_assign_m.jinja",
                minijinja::context! {
                    var => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t\t}\n");
            out.push_str("\t}\n");
            out.push('\n');
        }
        TypeRef::Optional(_) => {
            // Optional types
            let go_type = rust_to_go_type(&param.ty);
            out.push_str(&crate::template_env::render(
                "var_type_decl.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    type_name => &go_type,
                },
            ));
            out.push_str(&crate::template_env::render(
                "if_nil_check.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push_str("\t\tvar rawData interface{}\n");
            out.push_str(&crate::template_env::render(
                "json_unmarshal_rawdata.jinja",
                minijinja::context! {
                    param => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push_str("\t\tif m, ok := rawData.(map[string]interface{}); ok {\n");
            out.push_str(&crate::template_env::render(
                "var_assign_m.jinja",
                minijinja::context! {
                    var => &var_name,
                },
            ));
            out.push('\n');
            out.push_str("\t\t}\n");
            out.push_str("\t}\n");
            out.push('\n');
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
            out.push_str(&crate::template_env::render(
                "var_assign_cast.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    cast => &cast,
                },
            ));
            out.push('\n');
            out.push('\n');
        }
        _ => {
            out.push_str(&crate::template_env::render(
                "var_assign_cast.jinja",
                minijinja::context! {
                    var_name => &var_name,
                    cast => param.name.as_str(),
                },
            ));
            out.push('\n');
            out.push('\n');
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

    #[test]
    fn gen_unregistration_fn_returns_empty_when_none() {
        let cfg = alef_core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            unregister_fn: None,
            clear_fn: None,
            ..Default::default()
        };
        let result = gen_unregistration_fn(&cfg, "kreuzberg", "OcrBackend");
        assert!(result.is_empty(), "expected empty output when unregister_fn is None");
    }

    #[test]
    fn gen_unregistration_fn_emits_wrapper_when_set() {
        let cfg = alef_core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            unregister_fn: Some("unregister_ocr_backend".to_string()),
            clear_fn: None,
            ..Default::default()
        };
        let result = gen_unregistration_fn(&cfg, "kreuzberg", "OcrBackend");
        assert!(
            !result.is_empty(),
            "expected non-empty output when unregister_fn is set"
        );
        assert!(
            result.contains("func unregister_ocr_backend(name string) error"),
            "generated function signature not found in:\n{result}"
        );
        assert!(
            result.contains("C.kreuzberg_unregister_ocr_backend"),
            "C call not found in:\n{result}"
        );
    }

    #[test]
    fn gen_clear_fn_returns_empty_when_none() {
        let cfg = alef_core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            unregister_fn: None,
            clear_fn: None,
            ..Default::default()
        };
        let result = gen_clear_fn(&cfg, "kreuzberg", "OcrBackend");
        assert!(result.is_empty(), "expected empty output when clear_fn is None");
    }

    #[test]
    fn gen_clear_fn_emits_wrapper_when_set() {
        let cfg = alef_core::config::TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            unregister_fn: None,
            clear_fn: Some("clear_ocr_backends".to_string()),
            ..Default::default()
        };
        let result = gen_clear_fn(&cfg, "kreuzberg", "OcrBackend");
        assert!(!result.is_empty(), "expected non-empty output when clear_fn is set");
        assert!(
            result.contains("func clear_ocr_backends() error"),
            "generated function signature not found in:\n{result}"
        );
        assert!(
            result.contains("C.kreuzberg_clear_ocr_backend"),
            "C call not found in:\n{result}"
        );
    }
}
