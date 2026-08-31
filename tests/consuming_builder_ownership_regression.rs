use alef::backends::{csharp::CsharpBackend, zig::ZigBackend};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, ErrorDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn config() -> ResolvedCrateConfig {
    let source = r#"
[workspace]
languages = ["csharp", "zig"]

[[crates]]
name = "neutral"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "neutral"

[crates.csharp]
namespace = "Neutral"
"#;
    let config: NewAlefConfig = toml::from_str(source).expect("neutral config must parse");
    config.resolve().expect("neutral config must resolve").remove(0)
}

fn route_builder_api() -> ApiSurface {
    ApiSurface {
        crate_name: "neutral".to_string(),
        version: "0.1.0".to_string(),
        types: vec![route_builder_type()],
        functions: vec![],
        enums: vec![],
        errors: vec![build_error()],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    }
}

fn route_builder_type() -> TypeDef {
    TypeDef {
        name: "RouteBuilder".to_string(),
        rust_path: "neutral::RouteBuilder".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![consuming_method()],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

fn consuming_method() -> MethodDef {
    MethodDef {
        name: "with_cors".to_string(),
        params: vec![],
        return_type: TypeRef::Named("RouteBuilder".to_string()),
        is_async: false,
        is_static: false,
        error_type: Some("BuildError".to_string()),
        doc: "Consume this builder and return its configured replacement.".to_string(),
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn build_error() -> ErrorDef {
    ErrorDef {
        name: "BuildError".to_string(),
        rust_path: "neutral::BuildError".to_string(),
        original_rust_path: String::new(),
        variants: vec![],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn file_containing<'a>(files: &'a [GeneratedFile], suffix: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("generated output must contain {suffix}"))
        .content
}

fn compile_csharp(files: &[GeneratedFile]) {
    let Some(target_framework) = dotnet_target_framework() else {
        assert!(
            std::env::var_os("ALEF_REQUIRE_DOTNET").is_none(),
            "ALEF_REQUIRE_DOTNET is set but dotnet is unavailable"
        );
        return;
    };
    let directory = tempfile::tempdir().expect("temporary C# directory must be created");
    let project_directory = directory.path().join("packages/csharp");
    for file in files {
        let destination = directory.path().join(&file.path);
        std::fs::create_dir_all(destination.parent().expect("generated file must have a parent"))
            .expect("generated C# directory must be created");
        std::fs::write(destination, &file.content).expect("generated C# file must be written");
    }
    let project = project_directory.join("Ownership.csproj");
    std::fs::write(
        &project,
        format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>{target_framework}</TargetFramework>\
             <OutputType>Exe</OutputType><NuGetAudit>false</NuGetAudit></PropertyGroup></Project>"
        ),
    )
    .expect("C# project must be written");
    write_runtime_probe(&project_directory);
    let output = run_runtime_probe(&project_directory);
    assert!(
        output.status.success(),
        "generated consuming wrapper runtime probe must pass\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `None` when `dotnet` is not runnable, not merely when it fails to spawn: a version-manager
/// shim spawns fine then exits non-zero, and `.output().ok()?` alone discards that exit status --
/// so this used to keep parsing a failed shim's (often empty) stdout as a version string instead
/// of reporting `dotnet` unavailable, handing `compile_csharp` a bogus target framework rather
/// than the `None` that would make it skip. ~keep
fn dotnet_target_framework() -> Option<String> {
    let output = std::process::Command::new("dotnet").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let major = version.trim().split('.').next()?;
    Some(format!("net{major}.0"))
}

fn write_runtime_probe(directory: &std::path::Path) {
    let program = r#"using System;
using System.Runtime.InteropServices;
using Neutral;
internal static class Program {
    [DllImport("neutral_ffi", EntryPoint = "neutral_test_free_count")]
    private static extern int FreeCount();
    private static int Main() {
        var original = new RouteBuilder(41UL);
        var replacement = original.WithCors();
        try
        {
            _ = original.Handle;
            return 1;
        }
        catch (ObjectDisposedException)
        {
        }
        original.Dispose();
        if (FreeCount() != 0) return 2;
        replacement.Dispose();
        return FreeCount() == 1 ? 0 : 3;
    }
}"#;
    let native = r#"#include <stdint.h>
static int32_t free_count = 0;
/* uint64_t, not void *: handles cross this ABI as scalar `AlefHandle` (alef:handle-abi:1),
   and the generated P/Invoke declares `ulong`. Declaring the stub as a pointer only happens
   to work because both occupy one register on 64-bit; it would misdeclare the contract. ~keep */
uint64_t neutral_route_builder_with_cors(uint64_t handle) { return handle + 1; }
void neutral_route_builder_free(uint64_t handle) { if (handle) free_count += 1; }
int32_t neutral_last_error_code(void) { return 0; }
const char *neutral_last_error_context(void) { return ""; }
void neutral_free_string(char *ptr) { (void)ptr; }
int32_t neutral_test_free_count(void) { return free_count; }
"#;
    std::fs::write(directory.join("Program.cs"), program).expect("runtime probe must be written");
    std::fs::write(directory.join("ownership.c"), native).expect("native probe must be written");
}

fn run_runtime_probe(directory: &std::path::Path) -> std::process::Output {
    let (library, linker_args, library_path) = if cfg!(target_os = "macos") {
        ("libneutral_ffi.dylib", vec!["-dynamiclib"], "DYLD_LIBRARY_PATH")
    } else if cfg!(target_os = "linux") {
        ("libneutral_ffi.so", vec!["-shared", "-fPIC"], "LD_LIBRARY_PATH")
    } else {
        return std::process::Command::new("dotnet").arg("--version").output().unwrap();
    };
    let mut command = std::process::Command::new("cc");
    command.args(linker_args).args(["-o", library, "ownership.c"]);
    let compile = command
        .current_dir(directory)
        .output()
        .expect("native probe compiler must start");
    assert!(compile.status.success(), "native ownership probe must compile");
    std::process::Command::new("dotnet")
        .args(["run", "--nologo", "-v:quiet"])
        .env(library_path, directory)
        .current_dir(directory)
        .output()
        .expect("runtime ownership probe must start")
}

#[test]
fn csharp_invalidates_the_consumed_safe_handle_before_observing_result() {
    let files = CsharpBackend
        .generate_bindings(&route_builder_api(), &config())
        .expect("C# generation must succeed");
    let wrapper = file_containing(&files, "RouteBuilder.cs");
    let method = wrapper
        .split("public RouteBuilder WithCors")
        .nth(1)
        .expect("consuming builder method must be generated");
    let native_call = method
        .find("NativeMethods.RouteBuilderWithCors")
        .expect("native call must be emitted");
    // `TakeHandle()` (before the native call) claims the receiver exclusively and marks the
    // owner's `_safeHandle` unavailable; `handleTransfer.Commit()` (after the native call) is
    // what finalizes the transferred handle via `SetHandleAsInvalid()`, replacing the old
    // direct `_safeHandle.Invalidate()` call at this position. ~keep
    let invalidate = method
        .find("handleTransfer.Commit()")
        .expect("consumed handle must be invalidated");
    let error_check = method
        .find("LastError")
        .expect("fallible method must check native error state");

    assert!(
        wrapper.contains("SetHandleAsInvalid"),
        "SafeHandle must support non-freeing invalidation: {wrapper}"
    );
    // `SetHandleAsInvalid()` alone (no `SetHandle(IntPtr.Zero)` needed) stops the CLR from ever
    // calling `ReleaseHandle()` on a transferred-away SafeHandle, so no raw pointer zeroing is
    // required to prevent a double free. Access is instead blocked one level up, before any
    // native handle read: `ThrowIfHandleUnavailable()` gates every read behind `_handleUnavailable`
    // (set inside `TakeHandle()`'s lock, before the transfer is even handed to the caller). ~keep
    assert!(
        wrapper.contains("        if (_handleUnavailable || _safeHandle.IsClosed || _safeHandle.IsInvalid)"),
        "the consumed wrapper must gate handle access behind _handleUnavailable instead of \
         exposing its stale native handle: {wrapper}"
    );
    assert!(
        native_call < invalidate,
        "host ownership transfers only after the native call starts: {method}"
    );
    assert!(
        invalidate < error_check,
        "an error after native consumption must not retain the old owner: {method}"
    );
}

/// The emitted `RouteBuilderSafeHandle` used to declare an `internal void Invalidate()` escape
/// hatch with zero call sites -- the exact "zero call sites" shape that made `BorrowHandle`/
/// `TakeHandle` themselves look like dead code before `2a91d92f0` wired them up. That escape
/// hatch has been removed rather than left to invite the same mistake in reverse: someone
/// later wiring a call to it and bypassing the lock-guarded `BorrowHandle`/`TakeHandle`/
/// `Commit` machinery, reopening the race `2a91d92f0` closed. This pins the emitted handle
/// class to the guarded mechanism only. ~keep
#[test]
fn csharp_safehandle_omits_bare_invalidate_and_keeps_only_the_guarded_mechanism() {
    let files = CsharpBackend
        .generate_bindings(&route_builder_api(), &config())
        .expect("C# generation must succeed");
    let wrapper = file_containing(&files, "RouteBuilder.cs");

    assert!(
        !wrapper.contains("Invalidate()"),
        "the SafeHandle must not declare a bare Invalidate() escape hatch outside the \
         lock-guarded BorrowHandle/TakeHandle/Commit machinery: {wrapper}"
    );
    assert!(
        wrapper.contains("internal HandleLease BorrowHandle()"),
        "the guarded borrow path must still be emitted: {wrapper}"
    );
    assert!(
        wrapper.contains("private HandleTransfer TakeHandle()"),
        "the guarded transfer path must still be emitted: {wrapper}"
    );
    assert!(
        wrapper.contains("internal void Commit()"),
        "the transfer must still commit through the guarded HandleTransfer type: {wrapper}"
    );
}

/// `RouteBuilder.Handle` routes through `GetHandle()` / `ThrowIfHandleUnavailable()`,
/// which trips on `SafeHandle.IsInvalid`/`IsClosed`. A consumed handle is therefore
/// unreachable via `ObjectDisposedException`, not a silent zero read; the probe below
/// asserts the throw rather than an `IntPtr.Zero` comparison. ~keep
#[test]
fn csharp_consuming_wrapper_compiles_and_preserves_single_owner() {
    let files = CsharpBackend
        .generate_bindings(&route_builder_api(), &config())
        .expect("C# generation must succeed");
    compile_csharp(&files);
}

#[test]
fn zig_clears_the_consumed_handle_before_observing_result() {
    let files = ZigBackend
        .generate_bindings(&route_builder_api(), &config())
        .expect("Zig generation must succeed");
    let zig = file_containing(&files, "neutral.zig");
    let method = zig
        .split("pub fn with_cors")
        .nth(1)
        .and_then(|body| body.split("pub fn free").next())
        .expect("consuming builder method must be generated");
    let native_call = method
        .find("c.neutral_route_builder_with_cors")
        .expect("native call must be emitted");
    let invalidate = method
        .find("self._handle = 0")
        .expect("consumed handle must be cleared");
    let error_check = method
        .find("_last_error_code")
        .expect("fallible method must check native error state");

    assert!(
        native_call < invalidate,
        "host ownership transfers only after the native call starts: {method}"
    );
    assert!(
        invalidate < error_check,
        "an error after native consumption must not retain the old owner: {method}"
    );
}
