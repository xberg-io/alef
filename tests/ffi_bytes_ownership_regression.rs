use alef::backends::{csharp::CsharpBackend, ffi::FfiBackend, zig::ZigBackend};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ReceiverKind, TypeDef, TypeRef};

fn config() -> ResolvedCrateConfig {
    let source = r#"
[workspace]
languages = ["ffi", "csharp", "zig"]

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

fn upload_file_api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "neutral".to_string(),
        version: "0.1.0".to_string(),
        types: vec![upload_file_type()],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    }
}

fn upload_file_type() -> TypeDef {
    TypeDef {
        name: "UploadFile".to_string(),
        rust_path: "neutral::UploadFile".to_string(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![borrowed_bytes_method()],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
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

fn borrowed_bytes_method() -> MethodDef {
    MethodDef {
        name: "as_bytes".to_string(),
        params: vec![],
        return_type: TypeRef::Bytes,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: "Borrow the exact file bytes, including embedded NULs.".to_string(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: true,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
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

#[test]
fn borrowed_bytes_cross_ffi_as_an_owned_length_delimited_copy() {
    let files = FfiBackend
        .generate_bindings(&upload_file_api(), &config())
        .expect("FFI generation must succeed");
    let lib = file_containing(&files, "lib.rs");
    let method = lib
        .split("fn neutral_upload_file_as_bytes")
        .nth(1)
        .expect("borrowed bytes method must be generated");

    assert!(
        method.contains("out_ptr: *mut *mut u8"),
        "missing owned output pointer: {method}"
    );
    assert!(method.contains("out_len: *mut usize"), "missing byte length: {method}");
    assert!(
        method.contains("out_cap: *mut usize"),
        "missing allocation metadata: {method}"
    );
    assert!(
        method.contains("into_boxed_slice()"),
        "borrowed bytes must be copied into an owned buffer: {method}"
    );
    assert!(
        lib.contains("fn neutral_free_bytes("),
        "owned byte result must have a matching free function"
    );
    assert!(
        !method.contains(".as_ptr() as *mut u8"),
        "borrowed core storage must never escape as caller-owned memory: {method}"
    );
}

#[test]
fn csharp_preserves_embedded_nuls_and_frees_only_the_owned_copy() {
    let files = CsharpBackend
        .generate_bindings(&upload_file_api(), &config())
        .expect("C# generation must succeed");
    let native = file_containing(&files, "NativeMethods.cs");
    let wrapper = file_containing(&files, "UploadFile.cs");

    assert!(
        native.contains("out IntPtr outPtr"),
        "P/Invoke must expose the owned output pointer: {native}"
    );
    assert!(
        native.contains("out UIntPtr outLen"),
        "P/Invoke must expose the exact byte length: {native}"
    );
    assert!(
        wrapper.contains("Marshal.Copy"),
        "wrapper must copy by explicit length: {wrapper}"
    );
    assert!(
        wrapper.contains("NativeMethods.FreeBytes"),
        "wrapper must release the owned native copy: {wrapper}"
    );
    assert!(
        !wrapper.contains("PtrToStringUTF8(outPtr)"),
        "arbitrary bytes, including embedded NULs, must not be scanned as UTF-8: {wrapper}"
    );
}

#[test]
fn zig_and_ffi_agree_on_the_owned_bytes_out_parameter_abi() {
    let files = ZigBackend
        .generate_bindings(&upload_file_api(), &config())
        .expect("Zig generation must succeed");
    let zig = file_containing(&files, "neutral.zig");

    assert!(
        zig.contains("neutral_upload_file_as_bytes(handle, &_out_ptr, &_out_len, &_out_cap)"),
        "Zig must call the length-delimited owned-buffer ABI: {zig}"
    );
    assert!(
        zig.contains("neutral_free_bytes"),
        "Zig must free the returned owned copy: {zig}"
    );
}
