//! Cross-backend ABI parity: every C# `[DllImport]` parameter must have the same width and
//! signedness as the C signature alef's own FFI backend emits for the same IR parameter.
//!
//! These tests deliberately never spell the expected C# type for a case directly. They compare
//! the two emitters against each other through a shared width/signedness lattice, so a future
//! change that moves *both* sides in step still passes while a change that moves only one fails.

use crate::backends::ffi::type_map::{c_param_type_with_paths_and_enums, scalar_c_abi_param_named_types};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use std::collections::{HashMap, HashSet};

/// The width class of a scalar that crosses the C ABI.
///
/// [`Width::Pointer`] is deliberately not `Width::Bits(64)`: `uintptr_t` and `uint64_t` coincide
/// only on 64-bit targets, and collapsing them would make this lattice agree with a declaration
/// that is wrong everywhere else. ~keep
#[derive(Debug, PartialEq, Eq)]
enum Width {
    Bits(u32),
    Pointer,
}

/// Width and signedness of a scalar. `None` means "not a scalar" (a pointer, a string, or a
/// value this lattice deliberately does not rank).
type ScalarShape = Option<(Width, bool)>;

const HANDLE_BITS: u32 = 64;
const DISCRIMINANT_BITS: u32 = 32;

/// The shape of a type as alef's C FFI backend spells it in Rust (what cbindgen renders).
fn c_scalar_shape(c_type: &str) -> ScalarShape {
    match c_type {
        "AlefHandle" | "u64" => Some((Width::Bits(HANDLE_BITS), false)),
        "i64" => Some((Width::Bits(HANDLE_BITS), true)),
        "u32" => Some((Width::Bits(32), false)),
        "i32" => Some((Width::Bits(DISCRIMINANT_BITS), true)),
        "u16" => Some((Width::Bits(16), false)),
        "i16" => Some((Width::Bits(16), true)),
        "u8" => Some((Width::Bits(8), false)),
        "i8" => Some((Width::Bits(8), true)),
        "usize" => Some((Width::Pointer, false)),
        "isize" => Some((Width::Pointer, true)),
        _ => None,
    }
}

/// The shape of a type as the C# backend spells it in a `[DllImport]` declaration.
fn csharp_scalar_shape(csharp_type: &str) -> ScalarShape {
    match csharp_type {
        "ulong" => Some((Width::Bits(HANDLE_BITS), false)),
        "long" => Some((Width::Bits(HANDLE_BITS), true)),
        "uint" => Some((Width::Bits(32), false)),
        "int" => Some((Width::Bits(DISCRIMINANT_BITS), true)),
        "ushort" => Some((Width::Bits(16), false)),
        "short" => Some((Width::Bits(16), true)),
        "byte" => Some((Width::Bits(8), false)),
        "sbyte" => Some((Width::Bits(8), true)),
        "nuint" => Some((Width::Pointer, false)),
        "nint" => Some((Width::Pointer, true)),
        "bool" => Some((Width::Bits(8), false)),
        _ => None,
    }
}

fn scalar_enum(name: &str) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        is_copy: true,
        variants: vec![
            EnumVariant {
                name: "First".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Second".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn record_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("sample_core::{name}"),
        ..Default::default()
    }
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn optional_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        optional: true,
        ..param(name, ty)
    }
}

/// A fieldless enum that is deliberately **not** `Copy`.
///
/// The FFI crate lowers any all-fieldless enum to an `i32` discriminant and emits a
/// `<enum>_from_i32` helper for it, `Copy` or not; a `Copy`-only view of "which named types are
/// scalars" therefore disagrees with the header it is supposed to describe. A consumer's
/// HTTP-method enum, fieldless but carrying a hand-written impl, is exactly this shape. ~keep
fn fieldless_non_copy_enum(name: &str) -> EnumDef {
    EnumDef {
        is_copy: false,
        ..scalar_enum(name)
    }
}

/// A neutral fixture: one `Copy` discriminant enum, one JSON-backed record, and a function
/// taking both plus a scalar primitive.
fn fixture() -> ApiSurface {
    ApiSurface {
        crate_name: "sample_core".to_string(),
        enums: vec![scalar_enum("Mode")],
        types: vec![record_type("Settings")],
        functions: vec![FunctionDef {
            name: "build_report".to_string(),
            rust_path: "sample_core::build_report".to_string(),
            params: vec![
                param("mode", TypeRef::Named("Mode".to_string())),
                optional_param("fallback_mode", TypeRef::Named("Mode".to_string())),
                param("settings", TypeRef::Named("Settings".to_string())),
                param("depth", TypeRef::Primitive(PrimitiveType::U64)),
                param("verbose", TypeRef::Primitive(PrimitiveType::Bool)),
            ],
            return_type: TypeRef::Named("Settings".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The declared parameter types of a `[DllImport]`, in order, with any `MarshalAs` prefix and
/// the trailing identifier stripped. Returning `Vec` lets the parity tests below prove they
/// compared real declarations rather than two empty lists.
fn declared_param_types(declaration: &str) -> Vec<String> {
    const SIGNATURE_MARKER: &str = "internal static extern ";
    let Some((_, signature)) = declaration.split_once(SIGNATURE_MARKER) else {
        return Vec::new();
    };
    let Some((_, after_open)) = signature.split_once('(') else {
        return Vec::new();
    };
    let Some((inside, _)) = after_open.rsplit_once(')') else {
        return Vec::new();
    };
    inside
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let without_attribute = entry.rsplit_once(']').map_or(entry, |(_, rest)| rest).trim();
            without_attribute
                .rsplit_once(char::is_whitespace)
                .map_or(without_attribute, |(ty, _)| ty)
                .trim()
                .to_string()
        })
        .collect()
}

fn csharp_declaration(api: &ApiSurface) -> String {
    super::pinvoke::gen_pinvoke_for_func(
        "sample_ffi_build_report",
        &api.functions[0],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &scalar_c_abi_param_named_types(api),
    )
}

fn c_declaration_types(api: &ApiSurface) -> Vec<String> {
    let scalar_named_types = scalar_c_abi_param_named_types(api);
    let path_map = AHashMap::new();
    api.functions[0]
        .params
        .iter()
        .map(|parameter| {
            c_param_type_with_paths_and_enums(
                &parameter.ty,
                &api.crate_name,
                &path_map,
                &scalar_named_types,
                parameter.is_mut,
            )
            .into_owned()
        })
        .collect()
}

#[test]
fn should_declare_every_scalar_pinvoke_param_at_the_width_the_c_signature_uses() {
    let api = fixture();
    let csharp_types = declared_param_types(&csharp_declaration(&api));
    let c_types = c_declaration_types(&api);

    assert_eq!(
        csharp_types.len(),
        5,
        "expected five declared C# params, got {csharp_types:?}"
    );
    assert_eq!(c_types.len(), 5, "expected five C param types, got {c_types:?}");

    for (index, (csharp_type, c_type)) in csharp_types.iter().zip(c_types.iter()).enumerate() {
        let c_shape = c_scalar_shape(c_type);
        if c_shape.is_none() {
            continue;
        }
        assert_eq!(
            csharp_scalar_shape(csharp_type),
            c_shape,
            "param {index}: C# declares `{csharp_type}` but the C signature declares `{c_type}`",
        );
    }
}

#[test]
fn should_declare_a_copy_enum_param_as_the_c_discriminant_not_an_alef_handle() {
    let api = fixture();
    let csharp_types = declared_param_types(&csharp_declaration(&api));
    let c_types = c_declaration_types(&api);

    assert_eq!(
        c_types[0], "i32",
        "fixture precondition: the C signature must use a discriminant"
    );
    assert_eq!(
        csharp_scalar_shape(&csharp_types[0]),
        Some((Width::Bits(DISCRIMINANT_BITS), true))
    );
    assert_eq!(
        csharp_scalar_shape(&csharp_types[1]),
        Some((Width::Bits(DISCRIMINANT_BITS), true))
    );
}

#[test]
fn should_still_declare_a_json_backed_record_param_as_an_alef_handle() {
    let api = fixture();
    let csharp_types = declared_param_types(&csharp_declaration(&api));
    let c_types = c_declaration_types(&api);

    assert_eq!(c_types[2], "AlefHandle");
    assert_eq!(
        csharp_scalar_shape(&csharp_types[2]),
        Some((Width::Bits(HANDLE_BITS), false))
    );
}

/// A data-carrying enum has no discriminant to pass, so it is boxed as a handle. This is the
/// negative control for [`a_fieldless_non_copy_enum_param_is_declared_at_the_discriminant_width`]
/// below: without it, "fieldless" could be dropped from the rule and both sides would still
/// agree — on the wrong width, for every enum in the surface.
#[test]
fn should_not_treat_a_data_carrying_enum_as_a_c_abi_discriminant() {
    let mut api = fixture();
    api.enums[0].is_copy = false;
    api.enums[0].variants[0].fields = vec![crate::core::ir::FieldDef {
        name: "payload".to_string(),
        ty: TypeRef::String,
        ..Default::default()
    }];
    let scalar_named_types = scalar_c_abi_param_named_types(&api);

    assert!(
        !scalar_named_types.contains("Mode"),
        "an enum with a data-carrying variant is boxed as a handle, so it must not be a discriminant",
    );
    assert_eq!(c_declaration_types(&api)[0], "AlefHandle");
    assert_eq!(
        csharp_scalar_shape(&declared_param_types(&csharp_declaration(&api))[0]),
        Some((Width::Bits(HANDLE_BITS), false)),
    );
}

/// The set is built from "can the FFI lower this to a discriminant", which is `Copy`-ness for a
/// struct and all-variants-fieldless for an enum.
///
/// It used to be built from `Copy`-ness alone, and this test asserted exactly that. The FFI
/// backend never implemented that rule for parameters: every one of its parameter-typing call
/// sites (`gen_opaque_static_constructor`, `gen_method_wrapper`, `gen_free_function`) is handed
/// the all-fieldless-enum set and emits `int32_t` for a member of it, `Copy` or not. The old
/// assertion survived only because these tests fed the *same* `Copy`-only set to both sides,
/// fabricating the C half instead of reading what the FFI emits — so a `Copy`-only view of the
/// world was self-consistent here and contradicted by the header on disk. ~keep
#[test]
fn should_build_the_scalar_named_type_set_from_lowerability_not_copy_ness_alone() {
    let mut api = fixture();
    api.types[0].is_copy = true;
    api.enums[0].is_copy = false;
    let scalar_named_types: AHashSet<String> = scalar_c_abi_param_named_types(&api);

    assert!(
        scalar_named_types.contains("Mode"),
        "a fieldless enum is lowerable to a discriminant whether or not it is Copy"
    );
    assert!(
        scalar_named_types.contains("Settings"),
        "a Copy struct is still a discriminant-width param"
    );
}

#[test]
fn should_declare_a_bool_param_at_the_c_int_width_not_a_one_byte_managed_bool() {
    let api = fixture();
    let csharp_types = declared_param_types(&csharp_declaration(&api));
    let c_types = c_declaration_types(&api);

    assert_eq!(
        c_types[4], "i32",
        "fixture precondition: a Rust bool crosses the C ABI as i32"
    );
    assert_eq!(csharp_scalar_shape(&csharp_types[4]), c_scalar_shape(&c_types[4]));
}

#[test]
fn should_pass_a_bool_argument_as_the_c_int_the_declaration_expects() {
    let optional_bool = optional_param("verbose", TypeRef::Primitive(PrimitiveType::Bool));
    let required = super::marshalling::native_call_arg(
        &TypeRef::Primitive(PrimitiveType::Bool),
        "verbose",
        false,
        &HashSet::new(),
    );
    let optional =
        super::marshalling::native_call_arg(&optional_bool.ty, "verbose", optional_bool.optional, &HashSet::new());

    assert_eq!(required, "(verbose ? 1 : 0)");
    assert_eq!(optional, "((verbose ?? false) ? 1 : 0)");
}

/// Return-position parity for `Optional<T>`.
///
/// The C# `[DllImport]` return type must be a pointer exactly when the FFI crate returns a
/// pointer for the same IR type, and a scalar exactly when it returns a scalar. Declaring a
/// scalar as `IntPtr` is not a cosmetic mismatch: the generated wrapper reads the integer bit
/// pattern as a UTF-8 string pointer and hands it to `FreeString`, which for a value like
/// `Some(52_428_800)` is an arbitrary-address free. That shipped, so this is the regression that
/// should have existed. Like the parameter tests above, it never spells a C# type directly —
/// it asks both emitters and compares. ~keep
#[test]
fn optional_return_pointerness_matches_the_ffi_crate() {
    use crate::backends::csharp::gen_bindings::marshalling::pinvoke_return_type;
    use crate::backends::ffi::type_map::c_return_type;

    let shapes = vec![
        TypeRef::Primitive(PrimitiveType::U64),
        TypeRef::Primitive(PrimitiveType::I32),
        TypeRef::Primitive(PrimitiveType::F64),
        TypeRef::Primitive(PrimitiveType::Bool),
        TypeRef::Duration,
        TypeRef::String,
        TypeRef::Path,
        TypeRef::Json,
        TypeRef::Bytes,
        TypeRef::Named("Config".to_string()),
        TypeRef::Vec(Box::new(TypeRef::String)),
        TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        TypeRef::Optional(Box::new(TypeRef::Duration)),
        TypeRef::Optional(Box::new(TypeRef::String)),
    ];

    for inner in shapes {
        let optional = TypeRef::Optional(Box::new(inner.clone()));
        let c_type = c_return_type(&optional, "core_crate");
        // `AlefHandle` is `type AlefHandle = u64` (handle_registry.rs.jinja) -- an integer key
        // into a registry, NOT a pointer. Classifying it as one would have flagged C#'s correct
        // `ulong` declaration as the bug. ~keep
        let ffi_returns_pointer = c_type.contains('*');
        let csharp_declares_pointer = pinvoke_return_type(&optional) == "IntPtr";

        assert_eq!(
            csharp_declares_pointer,
            ffi_returns_pointer,
            "Option<{inner:?}>: the FFI crate returns `{c_type}` but C# declares \
             `{}`. A scalar declared as IntPtr is read as a pointer and freed.",
            pinvoke_return_type(&optional)
        );
    }
}

/// A fieldless enum that is not `Copy` still crosses as a discriminant, so the C# declaration
/// must be `int`. Declaring the handle width here is not a mere type error: the wrapper casts
/// `(int)`, the header says `int32_t`, and only the declaration between them said `ulong` --
/// which is a live ABI violation, not just a CS1503.
#[test]
fn a_fieldless_non_copy_enum_param_is_declared_at_the_discriminant_width() {
    let mut api = fixture();
    api.enums = vec![fieldless_non_copy_enum("Mode")];

    let csharp_types = declared_param_types(&csharp_declaration(&api));
    let c_types = c_declaration_types(&api);

    assert_eq!(
        csharp_scalar_shape(&csharp_types[0]),
        c_scalar_shape(&c_types[0]),
        "C# declared `{}` for a fieldless non-Copy enum the C signature spells `{}`",
        csharp_types[0],
        c_types[0]
    );
    assert_eq!(
        csharp_scalar_shape(&csharp_types[0]),
        Some((Width::Bits(DISCRIMINANT_BITS), true)),
        "both sides agreed, but on the handle width rather than the discriminant width"
    );
}
