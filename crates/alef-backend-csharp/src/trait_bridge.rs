/// C# trait bridge support via P/Invoke.
///
/// For C# backends that use C FFI (FFI dependency), this module generates
/// P/Invoke declarations for trait bridge registration/unregistration functions
/// to inject into NativeMethods.cs.
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::TypeDef;
use heck::ToSnakeCase;
use std::fmt::Write;

/// Generate P/Invoke trait bridge declarations for NativeMethods.cs.
///
/// For each trait bridge in the config, returns a C# P/Invoke declaration
/// for the register and unregister functions.
pub fn gen_native_methods_trait_bridges(
    _namespace: &str,
    prefix: &str,
    bridges: &[(String, &TraitBridgeConfig, &TypeDef)],
) -> String {
    let mut out = String::with_capacity(1024);

    if bridges.is_empty() {
        return out;
    }

    writeln!(out).ok();
    writeln!(out, "    // Trait Bridge FFI").ok();

    for (trait_name, _config, _trait_def) in bridges {
        let trait_snake = trait_name.to_snake_case();
        let register_fn = format!("{prefix}_register_{trait_snake}");
        let unregister_fn = format!("{prefix}_unregister_{trait_snake}");

        writeln!(out).ok();
        writeln!(
            out,
            "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{register_fn}\")]"
        )
        .ok();
        writeln!(
            out,
            "    internal static extern int Register{}([MarshalAs(UnmanagedType.LPUTF8Str)] string name, IntPtr vtable, IntPtr userData, out IntPtr outError);",
            trait_name
        )
        .ok();
        writeln!(out).ok();
        writeln!(
            out,
            "    [DllImport(LibName, CallingConvention = CallingConvention.Cdecl, EntryPoint = \"{unregister_fn}\")]"
        )
        .ok();
        writeln!(
            out,
            "    internal static extern int Unregister{}([MarshalAs(UnmanagedType.LPUTF8Str)] string name, out IntPtr outError);",
            trait_name
        )
        .ok();
    }

    out
}
