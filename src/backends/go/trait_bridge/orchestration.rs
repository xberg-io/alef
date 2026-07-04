use super::dispatch::{gen_plugin_trampolines, gen_trampoline};
use super::helpers::{c_callback_return_type, c_trampoline_signature, method_with_excluded_substituted};
use super::registration::{gen_clear_fn, gen_unregistration_fn};
use super::wrapper::{gen_bridge_wrapper, gen_interface_method};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::hash::{self, CommentStyle};
use crate::core::ir::{ApiSurface, TypeDef};
use heck::ToPascalCase;
use std::collections::HashSet;

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
/// `pkg_name`: Go package name (e.g., `"sample_core"`).
/// `ffi_prefix`: C function prefix (e.g., `"sample_core"`).
/// `ffi_header`: C header filename (e.g., `"sample_core.h"`).
/// `ffi_crate_dir`: path from go output dir to the FFI crate dir.
/// `to_root`: relative path from go output dir to the repo root.
/// `crate_name`: Rust FFI crate name (e.g., `"sample_core"`), used to derive C type names.
#[allow(clippy::too_many_arguments)]
pub fn gen_trait_bridges_file(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    pkg_name: &str,
    ffi_prefix: &str,
    ffi_header: &str,
    ffi_crate_dir: &str,
    to_root: &str,
    crate_name: &str,
) -> String {
    let mut out = String::with_capacity(16_384);

    // Collect names of types that are present in the IR but explicitly excluded from the
    // public binding surface (typically via `#[cfg_attr(alef, alef(skip))]` or
    // type-level config exclusions). Trait-bridge interface signatures referencing any
    // such type must fall back to `json.RawMessage`: the corresponding Go type was never
    // emitted into binding.go and would otherwise produce `undefined: <Name>` build
    // errors. The IR exposes these names in `excluded_type_paths` — that includes both
    // types stripped from `api.types` entirely and types still present with
    // `binding_excluded = true`. We also union in any `binding_excluded` type names from
    // `api.types` defensively.
    let excluded_named_types: HashSet<&str> = api
        .excluded_type_paths
        .keys()
        .map(|s| s.as_str())
        .chain(api.types.iter().filter(|t| t.binding_excluded).map(|t| t.name.as_str()))
        .collect();

    out.push_str(&hash::header(CommentStyle::DoubleSlash));
    // NOTE: package_and_cgo.jinja already emits "package {name}\n\n/*\n#cgo..."
    // so we render it directly — do NOT push a separate "/*\n" before this call.
    out.push_str(&crate::backends::go::template_env::render(
        "package_and_cgo.jinja",
        minijinja::context! {
            pkg_name => pkg_name,
            to_root => to_root,
            ffi_crate_dir => ffi_crate_dir,
            ffi_header => ffi_header,
        },
    ));
    out.push('\n');

    // Forward-declare all exported Go trampolines in the CGO preamble so that
    // C code can reference them. These are Go functions with //export directives
    // that will be linked when the Go code is compiled.
    for bridge_cfg in &config.trait_bridges {
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            let pascal = bridge_cfg.trait_name.to_pascal_case();
            for method in trait_def
                .methods
                .iter()
                .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
            {
                let export_name = format!("go{}{}", &pascal, method.name.to_pascal_case());
                let method_substituted = method_with_excluded_substituted(method, &excluded_named_types);
                let c_sig = c_trampoline_signature(&export_name, &method_substituted);
                let c_return_type = c_callback_return_type(&method_substituted);
                out.push_str(&crate::backends::go::template_env::render(
                    "extern_trampoline_decl.jinja",
                    minijinja::context! {
                        export_name => export_name,
                        c_sig => c_sig,
                        c_return_type => c_return_type,
                    },
                ));
            }
            // Plugin lifecycle trampolines
            out.push_str(&crate::backends::go::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Name",
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Version",
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Initialize",
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "plugin_trampoline_decl.jinja",
                minijinja::context! {
                    pascal => pascal.clone(),
                    method => "Shutdown",
                },
            ));
            out.push_str(&crate::backends::go::template_env::render(
                "plugin_free_user_data_extern.jinja",
                minijinja::context! {
                    pascal => &pascal,
                },
            ));
        }
    }

    // Emit C helper functions for vtable construction.
    // Each helper allocates and initializes a vtable in C, correctly populating function pointer slots.
    for bridge_cfg in &config.trait_bridges {
        if !bridge_cfg.exclude_languages.iter().any(|lang| lang == "go")
            && api.types.iter().any(|t| t.name == bridge_cfg.trait_name)
        {
            if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
                let trait_pascal = trait_def.name.to_pascal_case();
                let trait_snake = heck::AsSnakeCase(&trait_def.name).to_string();
                let vtable_constructor = format!("{}_{}_vtable_new", ffi_prefix, trait_snake);
                let crate_normalized = crate_name.replace('-', "_");
                let crate_upper = crate_normalized.to_uppercase();
                let crate_pascal = crate_normalized.to_pascal_case();
                let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");
                let vtable_methods: Vec<_> = trait_def
                    .methods
                    .iter()
                    .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
                    .collect();
                let method_field_names: Vec<String> = vtable_methods
                    .iter()
                    .map(|method| heck::AsSnakeCase(&method.name).to_string())
                    .collect();
                let method_pascal_names: Vec<String> = vtable_methods
                    .iter()
                    .map(|method| method.name.to_pascal_case())
                    .collect();

                // No need for method_casts anymore - the inline function uses generic void(*)(void) casts

                out.push_str(&crate::backends::go::template_env::render(
                    "vtable_constructor_helper.jinja",
                    minijinja::context! {
                        c_vtable_struct => &c_vtable_struct,
                        method_field_names => method_field_names,
                        method_pascal_names => method_pascal_names,
                        trait_pascal => &trait_pascal,
                        vtable_constructor => &vtable_constructor,
                    },
                ));
            }
        }
    }

    out.push_str("*/\n");
    out.push_str("import \"C\"\n");
    out.push('\n');

    out.push_str("import (\n");
    out.push_str("\t\"encoding/json\"\n");
    out.push_str("\t\"fmt\"\n");
    out.push_str("\t\"os\"\n");
    out.push_str("\t\"runtime/cgo\"\n");
    out.push_str("\t\"sync\"\n");
    out.push_str("\t\"unsafe\"\n");
    out.push_str(")\n");
    out.push('\n');

    // Generate handle registry type and instances for all trait bridges.
    // Each trait needs a registry to track cgo.Handles by name for cleanup on unregister.
    let has_trait_bridges = config.trait_bridges.iter().any(|cfg| {
        !cfg.exclude_languages.iter().any(|lang| lang == "go") && api.types.iter().any(|t| t.name == cfg.trait_name)
    });

    if has_trait_bridges {
        out.push_str("// handleRegistry tracks cgo.Handles by name to ensure proper cleanup on unregister.\n");
        out.push_str("// Without this, unregistered plugins can cause use-after-free crashes when Rust\n");
        out.push_str("// still holds vtable pointers and tries to invoke callbacks on deleted handles.\n");
        out.push_str("type handleRegistry struct {\n");
        out.push_str("\tmu      sync.Mutex\n");
        out.push_str("\thandles map[string]cgo.Handle\n");
        out.push_str("}\n");
        out.push('\n');

        out.push_str("var (\n");
        for bridge_cfg in &config.trait_bridges {
            if !bridge_cfg.exclude_languages.iter().any(|lang| lang == "go")
                && api.types.iter().any(|t| t.name == bridge_cfg.trait_name)
            {
                let trait_snake = heck::AsSnakeCase(&bridge_cfg.trait_name).to_string();
                out.push_str(&crate::backends::go::template_env::render(
                    "handle_registry_var.jinja",
                    minijinja::context! {
                        trait_snake => &trait_snake,
                    },
                ));
            }
        }
        out.push_str(")\n");
        out.push('\n');

        // Generate handle registry methods.
        out.push_str("// store adds a handle to the registry, keyed by name.\n");
        out.push_str("func (reg *handleRegistry) store(name string, handle cgo.Handle) {\n");
        out.push_str("\treg.mu.Lock()\n");
        out.push_str("\tdefer reg.mu.Unlock()\n");
        out.push_str("\tif old, ok := reg.handles[name]; ok {\n");
        out.push_str("\t\told.Delete()\n");
        out.push_str("\t}\n");
        out.push_str("\treg.handles[name] = handle\n");
        out.push_str("}\n");
        out.push('\n');

        out.push_str("// delete removes and deletes a handle from the registry by name.\n");
        out.push_str("func (reg *handleRegistry) delete(name string) {\n");
        out.push_str("\treg.mu.Lock()\n");
        out.push_str("\tdefer reg.mu.Unlock()\n");
        out.push_str("\tif handle, ok := reg.handles[name]; ok {\n");
        out.push_str("\t\tdelete(reg.handles, name)\n");
        out.push_str("\t\thandle.Delete()\n");
        out.push_str("\t}\n");
        out.push_str("}\n");
        out.push('\n');

        out.push_str("// clear removes and deletes all handles from the registry.\n");
        out.push_str("func (reg *handleRegistry) clear() {\n");
        out.push_str("\treg.mu.Lock()\n");
        out.push_str("\tdefer reg.mu.Unlock()\n");
        out.push_str("\tfor _, handle := range reg.handles {\n");
        out.push_str("\t\thandle.Delete()\n");
        out.push_str("\t}\n");
        out.push_str("\treg.handles = make(map[string]cgo.Handle)\n");
        out.push_str("}\n");
        out.push('\n');
    }

    // Generate interfaces, trampolines, and registration functions for each bridge
    for bridge_cfg in &config.trait_bridges {
        // Skip trait bridges excluded for this language
        if bridge_cfg.exclude_languages.iter().any(|lang| lang == "go") {
            continue;
        }
        if let Some(trait_def) = api.types.iter().find(|t| t.name == bridge_cfg.trait_name) {
            let trait_snake = heck::AsSnakeCase(&trait_def.name).to_string();
            gen_trait_bridge(
                &mut out,
                trait_def,
                bridge_cfg,
                ffi_prefix,
                crate_name,
                &excluded_named_types,
                &trait_snake,
            );
            out.push('\n');
        }
    }

    out
}

/// Generate one trait bridge: interface, trampolines, registration/unregistration functions.
pub(super) fn gen_trait_bridge(
    out: &mut String,
    trait_def: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    ffi_prefix: &str,
    crate_name: &str,
    excluded_named_types: &HashSet<&str>,
    #[allow(unused_variables)] trait_snake: &str,
) {
    let trait_name = &trait_def.name;
    let trait_snake = heck::AsSnakeCase(trait_name).to_string();
    let trait_pascal = trait_name.to_pascal_case();

    // Derive C VTable struct name: {CRATE_UPPER}{CratePascal}{TraitPascal}VTable
    // E.g., for crate="sample_core", trait="TextBackend": SAMPLE_CRATESampleCrateTextBackendVTable
    // Hyphens in crate names are not valid in C identifiers;
    // normalize the same way ffi_prefix does (`-` → `_`) before uppercasing.
    let crate_normalized = crate_name.replace('-', "_");
    let crate_upper = crate_normalized.to_uppercase();
    let crate_pascal = crate_normalized.to_pascal_case();
    #[allow(unused_variables)]
    let c_vtable_struct = format!("{}{}{}{}", crate_upper, crate_pascal, trait_pascal, "VTable");

    // =========================================================================
    // Go interface
    // =========================================================================
    out.push_str(&crate::backends::go::template_env::render(
        "trait_interface_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    // Plugin methods (name, version, initialize, shutdown)
    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Name returns the plugin name.",
            method => "Name",
            return_type => "string",
        },
    ));

    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Version returns the plugin version.",
            method => "Version",
            return_type => "string",
        },
    ));

    out.push_str(&crate::backends::go::template_env::render(
        "plugin_method_signature.jinja",
        minijinja::context! {
            doc => "Initialize is called when the plugin is loaded.",
            method => "Initialize",
            return_type => "error",
        },
    ));

    out.push_str(&crate::backends::go::template_env::render(
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
        let method_substituted = method_with_excluded_substituted(method, excluded_named_types);
        gen_interface_method(out, &method_substituted);
    }

    out.push_str("}\n");
    out.push('\n');

    // =========================================================================
    // Path A Bridge wrapper struct and delegating methods
    // =========================================================================
    gen_bridge_wrapper(
        out,
        trait_def,
        trait_name,
        &bridge_cfg.ffi_skip_methods,
        excluded_named_types,
    );

    // =========================================================================
    // Exported trampolines
    // =========================================================================
    for method in trait_def
        .methods
        .iter()
        .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
    {
        let export_name = format!("go{}{}", &trait_pascal, method.name.to_pascal_case());
        out.push_str(&crate::backends::go::template_env::render(
            "export_marker.jinja",
            minijinja::context! {
                name => &export_name,
            },
        ));
        let method_substituted = method_with_excluded_substituted(method, excluded_named_types);
        gen_trampoline(out, trait_name, &trait_pascal, &method_substituted);
    }

    // Plugin method trampolines
    gen_plugin_trampolines(out, trait_name, &trait_pascal);

    // =========================================================================
    // Registration function
    // =========================================================================
    out.push_str(&crate::backends::go::template_env::render(
        "register_function_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    // Collect export names for method trampolines
    let export_names: Vec<String> = trait_def
        .methods
        .iter()
        .filter(|m| !bridge_cfg.ffi_skip_methods.contains(&m.name))
        .map(|m| format!("go{}{}", &trait_pascal, m.name.to_pascal_case()))
        .collect();

    // Build the C vtable by calling the C helper function.
    // This ensures function pointers are correctly populated in C, fixing ARM64 macOS issues
    // where unsafe.Pointer casts don't work correctly for function addresses.
    let vtable_constructor = format!("{}_{}_vtable_new", ffi_prefix, trait_snake);
    out.push_str(&crate::backends::go::template_env::render(
        "vtable_allocation_via_c_helper.jinja",
        minijinja::context! {
            export_names => export_names,
            vtable_constructor => &vtable_constructor,
        },
    ));

    out.push_str(&crate::backends::go::template_env::render(
        "register_c_call.jinja",
        minijinja::context! {
            c_function => format!("{}_register_{}", ffi_prefix, trait_snake),
            ffi_prefix => ffi_prefix,
            trait_name => trait_name,
            trait_snake => trait_snake,
        },
    ));
    out.push_str("}\n");
    out.push('\n');

    // =========================================================================
    // Unregistration function
    // =========================================================================
    out.push_str(&crate::backends::go::template_env::render(
        "unregister_function_header.jinja",
        minijinja::context! {
            name => trait_name,
        },
    ));

    out.push_str(&crate::backends::go::template_env::render(
        "unregister_c_call.jinja",
        minijinja::context! {
            c_function => format!("{}_unregister_{}", ffi_prefix, trait_snake),
            ffi_prefix => ffi_prefix,
            trait_name => trait_name,
            trait_snake => trait_snake,
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
