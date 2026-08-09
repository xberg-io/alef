use super::files::csharp_file_header;

pub(super) fn generate(namespace: &str, library: &str, prefix: &str, exception: &str) -> String {
    format!(
        r#"{header}using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace {namespace};

/// <summary>Verified downloadable native-component management.</summary>
public static class Components
{{
    private const string LibraryName = "{library}";

    /// <summary>Downloads, verifies, loads, and pins a configured component.</summary>
    public static void ComponentLoad(string component)
    {{
        var componentPointer = Marshal.StringToCoTaskMemUTF8(component);
        try
        {{
            if (NativeComponentLoad(componentPointer) != 0)
            {{
                throw LastComponentError();
            }}
        }}
        finally
        {{
            Marshal.FreeCoTaskMem(componentPointer);
        }}
    }}

    /// <summary>Downloads and verifies every configured component.</summary>
    public static IReadOnlyList<string> ComponentPrefetch() => ComponentPrefetch(null);

    /// <summary>Downloads and verifies one configured component.</summary>
    public static IReadOnlyList<string> ComponentPrefetch(string? component)
    {{
        var componentPointer = component is null ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(component);
        try
        {{
            var result = NativeComponentPrefetch(componentPointer);
            var json = TakeNativeString(result);
            return JsonSerializer.Deserialize<string[]>(json)
                ?? throw new {exception}("component prefetch returned null JSON");
        }}
        finally
        {{
            if (componentPointer != IntPtr.Zero)
            {{
                Marshal.FreeCoTaskMem(componentPointer);
            }}
        }}
    }}

    /// <summary>Returns missing, cached:&lt;path&gt;, or loaded:&lt;path&gt;.</summary>
    public static string ComponentStatus(string component) => CallNativeString(component, NativeComponentStatus);

    /// <summary>Returns the content-addressed cache path for a configured component.</summary>
    public static string ComponentCachePath(string component) => CallNativeString(component, NativeComponentCachePath);

    private static string CallNativeString(string component, Func<IntPtr, IntPtr> operation)
    {{
        var componentPointer = Marshal.StringToCoTaskMemUTF8(component);
        try
        {{
            return TakeNativeString(operation(componentPointer));
        }}
        finally
        {{
            Marshal.FreeCoTaskMem(componentPointer);
        }}
    }}

    private static string TakeNativeString(IntPtr result)
    {{
        if (result == IntPtr.Zero)
        {{
            throw LastComponentError();
        }}
        try
        {{
            return Marshal.PtrToStringUTF8(result)
                ?? throw new {exception}("component manager returned invalid UTF-8");
        }}
        finally
        {{
            NativeFreeString(result);
        }}
    }}

    private static {exception} LastComponentError()
    {{
        var code = NativeLastErrorCode();
        var context = NativeLastErrorContext();
        var message = context == IntPtr.Zero
            ? "component manager operation failed"
            : Marshal.PtrToStringUTF8(context) ?? "component manager returned invalid error context";
        return new {exception}(code, message);
    }}

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_component_load")]
    private static extern int NativeComponentLoad(IntPtr component);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_component_prefetch")]
    private static extern IntPtr NativeComponentPrefetch(IntPtr component);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_component_status")]
    private static extern IntPtr NativeComponentStatus(IntPtr component);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_component_cache_path")]
    private static extern IntPtr NativeComponentCachePath(IntPtr component);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_last_error_code")]
    private static extern int NativeLastErrorCode();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_last_error_context")]
    private static extern IntPtr NativeLastErrorContext();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl, EntryPoint = "{prefix}_free_string")]
    private static extern void NativeFreeString(IntPtr value);
}}
"#,
        header = csharp_file_header(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_csharp_pinvoke_component_manager() {
        let generated = generate("Demo", "demo_ffi", "demo", "DemoException");
        assert!(generated.contains("void ComponentLoad(string component)"));
        assert!(generated.contains("IReadOnlyList<string> ComponentPrefetch()"));
        assert!(generated.contains("string ComponentStatus(string component)"));
        assert!(generated.contains("EntryPoint = \"demo_component_cache_path\""));
        assert!(generated.contains("NativeFreeString(result)"));
    }
}
