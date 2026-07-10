//! Dedicated C# visitor-bridge generation (callbacks-struct ABI, "Path 1").
//!
//! A visitor trait bridge (configured with `context_type` + `result_type`) is wired
//! through the canonical visitor-callbacks struct shared with the Go and Java bindings
//! — NOT the generic trait-bridge vtable ("Path 2") that targets
//! `{prefix}_register_{trait}`.
//!
//! The unmanaged block emitted here is `user_data` followed by one function pointer per
//! visit method. Each callback receives `(ctx, user_data, ...params..., out_custom, out_len)`
//! and returns an i32 visit-result code; for the string-payload variants it writes a heap
//! C string into `*out_custom` (and its byte length into `*out_len`) that the Rust side
//! takes ownership of and frees with the system allocator.
//!
//! This mirrors `crates/.../lib.rs` `HtmVisitorCallbacks` field-for-field and is verified
//! against the C# visitor e2e suite.

use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::ir::{PrimitiveType, TypeDef, TypeRef};
use heck::ToLowerCamelCase;
use std::collections::HashSet;

/// Per-parameter marshalling spec for one non-context visit parameter.
struct ParamSpec {
    /// Unmanaged delegate parameter declaration(s), e.g. `IntPtr text` or
    /// `IntPtr cells, UIntPtr cellsCount`.
    delegate_decl: String,
    /// Expression that converts the unmanaged param(s) to the managed interface argument,
    /// e.g. `Str(text)`, `ordered != 0`, `DecodeStringArray(cells, cellsCount)`.
    call_arg: String,
}

/// Map a non-context visit parameter to its unmanaged delegate decl + managed call arg.
fn param_spec(ty: &TypeRef, camel: &str) -> ParamSpec {
    match ty {
        TypeRef::String => ParamSpec {
            delegate_decl: format!("IntPtr {camel}"),
            call_arg: format!("Str({camel})"),
        },
        TypeRef::Primitive(PrimitiveType::Bool) => ParamSpec {
            delegate_decl: format!("int {camel}"),
            call_arg: format!("{camel} != 0"),
        },
        TypeRef::Primitive(PrimitiveType::U32) => ParamSpec {
            delegate_decl: format!("uint {camel}"),
            call_arg: camel.to_string(),
        },
        TypeRef::Primitive(PrimitiveType::I32) => ParamSpec {
            delegate_decl: format!("int {camel}"),
            call_arg: camel.to_string(),
        },
        TypeRef::Primitive(PrimitiveType::Usize | PrimitiveType::U64) => ParamSpec {
            delegate_decl: format!("UIntPtr {camel}"),
            call_arg: format!("(ulong){camel}"),
        },
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String) => ParamSpec {
            delegate_decl: format!("IntPtr {camel}, UIntPtr {camel}Count"),
            call_arg: format!("DecodeStringArray({camel}, {camel}Count)"),
        },
        _ => ParamSpec {
            delegate_decl: format!("IntPtr {camel}"),
            call_arg: format!("Str({camel})"),
        },
    }
}

/// Emit the `interface I{Trait}` declaration matching the visitor trait surface.
fn emit_interface(out: &mut String, trait_pascal: &str, trait_def: &TypeDef, visible: &HashSet<&str>) {
    out.push_str("/// <summary>\n");
    out.push_str(&format!(
        "/// Bridge interface for {trait_pascal} trait implementation via native FFI\n"
    ));
    out.push_str("/// </summary>\n");
    out.push_str(&format!("public interface I{trait_pascal} {{\n"));
    for method in &trait_def.methods {
        let pascal = to_csharp_name(&method.name);
        let params = method
            .params
            .iter()
            .map(|p| {
                let ty = crate::backends::csharp::trait_bridge::csharp_type_visible_pub(&p.ty, visible);
                format!("{} {}", ty, to_csharp_name(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("\n    /// <summary>{}</summary>\n", method.name));
        out.push_str(&format!("    VisitResult {pascal}({params});\n"));
    }
    out.push_str("}\n\n");
}

/// Generate the interface + callbacks-struct bridge class for a visitor trait bridge.
///
/// Appends to `out`. `result_type`/`context_type` names are taken from config and assumed
/// to be already emitted as C# types (`VisitResult`, `NodeContext`) by `gen_visitor`.
pub fn gen_visitor_bridge(out: &mut String, trait_name: &str, trait_def: &TypeDef, visible_type_names: &HashSet<&str>) {
    let trait_pascal = csharp_type_name(trait_name);

    emit_interface(out, &trait_pascal, trait_def, visible_type_names);

    let bridge = format!("{trait_pascal}Bridge");
    let num_methods = trait_def.methods.len();
    let slot_count = num_methods + 1;

    let mut delegate_decls = String::new();
    let mut write_slots = String::new();
    let mut callbacks = String::new();

    for (idx, method) in trait_def.methods.iter().enumerate() {
        let pascal = to_csharp_name(&method.name);
        let fn_name = format!("{pascal}Fn");
        let cb_name = format!("{pascal}Callback");
        let slot = idx + 1;

        let rest: Vec<&crate::core::ir::ParamDef> = method.params.iter().skip(1).collect();

        let mut delegate_params: Vec<String> = Vec::new();
        let mut call_args: Vec<String> = vec!["DecodeContext(ctx)".to_string()];
        for p in &rest {
            let camel = p.name.to_lower_camel_case();
            let spec = param_spec(&p.ty, &camel);
            delegate_params.push(spec.delegate_decl);
            call_args.push(spec.call_arg);
        }

        let mut sig_parts = vec!["IntPtr ctx".to_string(), "IntPtr userData".to_string()];
        sig_parts.extend(delegate_params.iter().cloned());
        sig_parts.push("IntPtr outCustom".to_string());
        sig_parts.push("IntPtr outLen".to_string());
        delegate_decls.push_str("    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]\n");
        delegate_decls.push_str(&format!(
            "    private delegate int {fn_name}({});\n",
            sig_parts.join(", ")
        ));

        write_slots.push_str(&format!("        WriteSlot({slot}, new {fn_name}({cb_name}));\n"));

        callbacks.push_str(&format!(
            "    private int {cb_name}({}) =>\n        Dispatch(userData, outCustom, outLen, v => v.{pascal}({}));\n",
            sig_parts.join(", "),
            call_args.join(", ")
        ));
    }

    out.push_str(&format!(
        r#"/// <summary>
/// Manages the native HtmVisitorCallbacks struct and managed delegates for an
/// I{trait_pascal} implementation.
///
/// ABI (Path 1, the canonical visitor callbacks struct shared with Go and Java):
/// the unmanaged block is `user_data` followed by one function pointer per visit
/// method. Each callback receives `(ctx, user_data, ...params..., out_custom, out_len)`
/// and returns an i32 visit-result code (0=Continue, 1=Custom, 2=Skip,
/// 3=PreserveHtml, 4=Error). For Custom/Error the callback writes a heap C string
/// into *out_custom and its byte length into *out_len; the Rust side takes ownership
/// and frees it.
/// </summary>
public sealed class {bridge} : IDisposable {{

    internal readonly I{trait_pascal} _impl;
    private readonly GCHandle _implHandle;
    // Pointer to the unmanaged HtmVisitorCallbacks struct (user_data + {num_methods} fn pointers).
    internal IntPtr _vtable;
    private bool _disposed;
    // Keep all delegates alive for the lifetime of the bridge: Rust holds raw function
    // pointers obtained via GetFunctionPointerForDelegate, which become invalid if the
    // delegate is collected.
    private readonly List<object> _delegateRoots;
    internal readonly IntPtr _bridgeId;
    private int _callbackRefCount = 0;

    // Static registry: maps bridge ID (used as the FFI user_data) to bridge instance,
    // so callbacks can recover the managed impl and the bridge stays alive while Rust
    // holds the ID.
    internal static readonly Dictionary<IntPtr, {bridge}> _bridgeRegistry = new();
    internal static int _nextBridgeId = 1;
    internal static readonly object _registryLock = new();

    // Number of pointer-sized slots: user_data + {num_methods} visit-method function pointers.
    private const int CallbackSlotCount = {slot_count};

    // Mirror of the FFI `HtmContext` repr(C) struct.
    [StructLayout(LayoutKind.Sequential)]
    private struct HtmContextNative {{
        public int NodeType;
        public IntPtr TagName;
        public UIntPtr Depth;
        public UIntPtr IndexInParent;
        public IntPtr ParentTag;
        public int IsInline;
    }}

    // --- Callback delegate signatures (ctx, user_data, ...params..., out_custom, out_len) ---

{delegate_decls}
    public {bridge}(I{trait_pascal} impl) {{
        _impl = impl ?? throw new ArgumentNullException(nameof(impl));
        _implHandle = GCHandle.Alloc(impl, GCHandleType.Normal);
        _delegateRoots = new List<object>(CallbackSlotCount);
        _vtable = IntPtr.Zero;
        _disposed = false;
        lock (_registryLock) {{
            _bridgeId = new IntPtr(_nextBridgeId++);
        }}
        BuildCallbacks();
    }}

    private void WriteSlot(int slot, Delegate fn) {{
        _delegateRoots.Add(fn);
        Marshal.WriteIntPtr(_vtable, IntPtr.Size * slot, Marshal.GetFunctionPointerForDelegate(fn));
    }}

    private void BuildCallbacks() {{
        _vtable = Marshal.AllocHGlobal(IntPtr.Size * CallbackSlotCount);
        // Slot 0: user_data — the registry id callbacks use to recover this bridge.
        Marshal.WriteIntPtr(_vtable, 0, _bridgeId);
        // Slots 1..{num_methods}: function pointers, in HtmVisitorCallbacks field order.
{write_slots}    }}

    private void IncrementCallbackRef() {{
        lock (_registryLock) {{
            _callbackRefCount++;
        }}
    }}

    private void DecrementCallbackRef() {{
        lock (_registryLock) {{
            if (_callbackRefCount > 0) {{
                _callbackRefCount--;
            }}
            if (_callbackRefCount == 0 && _disposed) {{
                _bridgeRegistry.Remove(_bridgeId);
            }}
        }}
    }}

    // --- Marshalling helpers ---

    private static string Str(IntPtr p) =>
        p == IntPtr.Zero ? string.Empty : (Marshal.PtrToStringUTF8(p) ?? string.Empty);

    private static NodeContext DecodeContext(IntPtr ctx) {{
        if (ctx == IntPtr.Zero) {{
            return new NodeContext(default, string.Empty, 0, 0, null, false);
        }}
        var native = Marshal.PtrToStructure<HtmContextNative>(ctx);
        var parentTag = native.ParentTag == IntPtr.Zero ? null : Marshal.PtrToStringUTF8(native.ParentTag);
        return new NodeContext(
            (NodeType)native.NodeType,
            Str(native.TagName),
            (ulong)native.Depth,
            (ulong)native.IndexInParent,
            parentTag,
            native.IsInline != 0);
    }}

    private static List<string> DecodeStringArray(IntPtr arr, UIntPtr count) {{
        var list = new List<string>();
        if (arr == IntPtr.Zero) {{
            return list;
        }}
        ulong n = (ulong)count;
        for (ulong i = 0; i < n; i++) {{
            var p = Marshal.ReadIntPtr(arr, (int)(i * (ulong)IntPtr.Size));
            list.Add(Str(p));
        }}
        return list;
    }}

    // Encodes a VisitResult into the FFI return code + out_custom/out_len protocol.
    // For Custom/Error, allocates a heap C string via NativeMemory (CRT malloc) so the
    // Rust side can take ownership and free it with the system allocator — matching Go's
    // C.CString and Java's global-arena allocation.
    private static unsafe int EncodeResult(VisitResult result, IntPtr outCustom, IntPtr outLen) {{
        switch (result) {{
            case VisitResult.Continue:
                return 0;
            case VisitResult.Custom c:
                WriteCustom(c.Value, outCustom, outLen);
                return 1;
            case VisitResult.Skip:
                return 2;
            case VisitResult.PreserveHtml:
                return 3;
            case VisitResult.Error e:
                WriteCustom(e.Value, outCustom, outLen);
                return 4;
            default:
                return 0;
        }}
    }}

    private static unsafe void WriteCustom(string value, IntPtr outCustom, IntPtr outLen) {{
        var bytes = Encoding.UTF8.GetBytes(value ?? string.Empty);
        byte* buf = (byte*)NativeMemory.Alloc((nuint)(bytes.Length + 1));
        for (int i = 0; i < bytes.Length; i++) {{
            buf[i] = bytes[i];
        }}
        buf[bytes.Length] = 0;
        if (outCustom != IntPtr.Zero) {{
            Marshal.WriteIntPtr(outCustom, (IntPtr)buf);
        }}
        if (outLen != IntPtr.Zero) {{
            Marshal.WriteIntPtr(outLen, (IntPtr)bytes.Length);
        }}
    }}

    // Shared dispatch: recover the bridge from the registry by user_data, invoke the
    // user's visit method, and encode the result. Any exception falls back to Continue
    // so a faulty visitor never unwinds across the FFI boundary or corrupts output.
    private static int Dispatch(IntPtr userData, IntPtr outCustom, IntPtr outLen, Func<I{trait_pascal}, VisitResult> invoke) {{
        {bridge}? bridge = null;
        lock (_registryLock) {{
            if (_bridgeRegistry.TryGetValue(userData, out var found)) {{
                bridge = found;
                bridge.IncrementCallbackRef();
            }}
        }}
        if (bridge == null) {{
            return 0;
        }}
        try {{
            return EncodeResult(invoke(bridge._impl), outCustom, outLen);
        }} catch {{
            return 0;
        }} finally {{
            try {{ bridge.DecrementCallbackRef(); }} catch {{ /* bridge already removed */ }}
        }}
    }}

    // --- Callbacks ---

{callbacks}
    public void Dispose() {{
        if (_disposed) return;
        _disposed = true;

        if (_vtable != IntPtr.Zero) {{
            Marshal.FreeHGlobal(_vtable);
            _vtable = IntPtr.Zero;
        }}

        if (_implHandle.IsAllocated) {{
            _implHandle.Free();
        }}
        // _delegateRoots is managed; the GC reclaims it once the bridge is unreachable.
    }}
}}

"#
    ));
}
