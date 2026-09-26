//! Every `NativeMethods.X(...)` the C# backend emits must have a `[DllImport]` declaration in the
//! same run's `NativeMethods.cs`.
//!
//! A name used in a `.jinja` template and declared nowhere is `CS0117` — the generated package
//! does not compile at all, no matter how correct every signature around it is. Nothing else in
//! this repo cross-checks the two sides: the emitters that write the calls and the emitters that
//! write the declarations live in different modules, are gated on different predicates, and are
//! covered by tests that only ever look at one side.
//!
//! The fixture below deliberately exercises a `bind_via = "options_field"` trait bridge, whose
//! wrapper is the only shape that reaches `bridge_field_register.jinja` /
//! `bridge_field_unregister.jinja` / `bridge_field_inject.jinja`. Those three templates called
//! `{Trait}BridgeNew`, `{Trait}BridgeFree` and `{Options}Set{Field}` with no declaration behind
//! them, which is what this file exists to keep fixed. ~keep

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::gen_bridge_field::gen_options_set_bridge;
use alef::backends::ffi::trait_bridge::{bridge_new_free_symbols, gen_bridge_new_free};
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{BridgeBinding, FfiConfig, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef,
};
use std::collections::{BTreeSet, HashMap};

const CRATE_NAME: &str = "sample_crate";
const TRAIT_NAME: &str = "NodeVisitor";
const OPTIONS_TYPE: &str = "RenderOptions";
const OPTIONS_FIELD: &str = "visitor";
const CONTEXT_TYPE: &str = "VisitContext";
const RESULT_TYPE: &str = "VisitOutcome";

/// The C# spelling of the FFI crate's `AlefHandle`.
/// `the_ffi_handle_registry_still_declares_a_sixty_four_bit_handle` below pins this to the FFI
/// template's own `type AlefHandle = u64;` rather than letting the two sides state the width
/// independently. ~keep
const HANDLE_CS_TYPE: &str = "ulong";

/// A trait bridge bound to a field of an options struct, plus the one free function that takes
/// that struct — the minimum shape that makes the C# backend emit a bridge-field wrapper.
fn surface() -> ApiSurface {
    let visit = MethodDef {
        name: "visit_node".to_owned(),
        params: vec![ParamDef {
            name: "node".to_owned(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(alef::core::ir::PrimitiveType::Bool),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };

    ApiSurface {
        crate_name: CRATE_NAME.to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            TypeDef {
                name: OPTIONS_TYPE.to_owned(),
                rust_path: format!("{CRATE_NAME}::{OPTIONS_TYPE}"),
                fields: vec![
                    FieldDef {
                        name: "width".to_owned(),
                        ty: TypeRef::Primitive(alef::core::ir::PrimitiveType::U32),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: OPTIONS_FIELD.to_owned(),
                        ty: TypeRef::Optional(Box::new(TypeRef::String)),
                        ..FieldDef::default()
                    },
                ],
                is_clone: true,
                ..TypeDef::default()
            },
            TypeDef {
                name: TRAIT_NAME.to_owned(),
                rust_path: format!("{CRATE_NAME}::{TRAIT_NAME}"),
                methods: vec![visit],
                is_trait: true,
                ..TypeDef::default()
            },
        ],
        functions: vec![FunctionDef {
            name: "render".to_owned(),
            rust_path: format!("{CRATE_NAME}::render"),
            params: vec![ParamDef {
                name: "options".to_owned(),
                ty: TypeRef::Named(OPTIONS_TYPE.to_owned()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

/// `[ffi]` is left unset, so `visitor_callbacks` reads as `false`. That is the branch in which
/// nothing else declares the `{Options}Set{Field}` setter, even though the FFI crate exports it
/// for every options-field bridge. ~keep
fn config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: CRATE_NAME.to_owned(),
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: TRAIT_NAME.to_owned(),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some(OPTIONS_TYPE.to_owned()),
            options_field: Some(OPTIONS_FIELD.to_owned()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn generate() -> Vec<GeneratedFile> {
    CsharpBackend
        .generate_bindings(&surface(), &config())
        .expect("generation should succeed")
}

/// The same bridge shape with `[ffi] visitor_callbacks = true`. That flips the C# backend onto a
/// different pair of emitters for the *same* three call sites: `bridge_field_register_visitor`
/// / `bridge_field_unregister_visitor` call `VisitorCreate` / `VisitorFree` instead of
/// `{Trait}BridgeNew` / `{Trait}BridgeFree`, and the setter is declared by
/// `native_methods_visitor.jinja` instead of `native_methods_trait_bridges.jinja`. The
/// visitor-off fixture above therefore never examines any of it.
///
/// `context_type` / `result_type` are what make `methods::bridge_fields` treat this as a visitor
/// bridge at all; the trait's own method signature is left alone deliberately, so the fixture
/// stays the minimum that reaches the changed emitters. ~keep
fn visitor_surface() -> ApiSurface {
    let mut api = surface();
    api.types.push(TypeDef {
        name: CONTEXT_TYPE.to_owned(),
        rust_path: format!("{CRATE_NAME}::{CONTEXT_TYPE}"),
        fields: vec![FieldDef {
            name: "depth".to_owned(),
            ty: TypeRef::Primitive(alef::core::ir::PrimitiveType::Usize),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    });
    api.enums.push(EnumDef {
        name: RESULT_TYPE.to_owned(),
        rust_path: format!("{CRATE_NAME}::{RESULT_TYPE}"),
        variants: vec![
            EnumVariant {
                name: "Proceed".to_owned(),
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Skip".to_owned(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    });
    api
}

fn visitor_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: CRATE_NAME.to_owned(),
        ffi: Some(FfiConfig {
            prefix: None,
            error_style: "last_error".to_owned(),
            header_name: None,
            lib_name: None,
            visitor_callbacks: true,
            features: None,
            extra_features: Vec::new(),
            serde_rename_all: None,
            exclude_functions: Vec::new(),
            exclude_types: Vec::new(),
            capsule_types: HashMap::new(),
            rename_fields: HashMap::new(),
            plugin_error_constructor: None,
            target_dep_overrides: Vec::new(),
            excluded_default_features: Vec::new(),
        }),
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: TRAIT_NAME.to_owned(),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some(OPTIONS_TYPE.to_owned()),
            options_field: Some(OPTIONS_FIELD.to_owned()),
            context_type: Some(CONTEXT_TYPE.to_owned()),
            result_type: Some(RESULT_TYPE.to_owned()),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    }
}

fn generate_with_visitor_callbacks() -> Vec<GeneratedFile> {
    CsharpBackend
        .generate_bindings(&visitor_surface(), &visitor_config())
        .expect("generation with visitor callbacks should succeed")
}

fn csharp_files(files: &[GeneratedFile]) -> Vec<(&str, &str)> {
    files
        .iter()
        .filter_map(|generated| {
            let name = generated.path.file_name()?.to_str()?;
            name.ends_with(".cs").then_some((name, generated.content.as_str()))
        })
        .collect()
}

/// Identifier immediately preceding `(`, reading right to left from `text`.
fn leading_identifier(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

/// Every member name declared on `NativeMethods` that a `NativeMethods.X(...)` expression can
/// legally resolve to: the `[DllImport]` externs and the marshalled callback delegate types.
fn declared_members(files: &[(&str, &str)]) -> BTreeSet<String> {
    let mut declared = BTreeSet::new();
    for (_, content) in files {
        for line in content.lines() {
            let trimmed = line.trim_start();
            let rest = trimmed
                .strip_prefix("internal static extern ")
                .or_else(|| trimmed.strip_prefix("public delegate "))
                .or_else(|| trimmed.strip_prefix("internal delegate "));
            let Some(rest) = rest else { continue };
            let Some((_return_type, after_type)) = rest.split_once(' ') else {
                continue;
            };
            if let Some(name) = leading_identifier(after_type.trim_start()) {
                declared.insert(name.to_owned());
            }
        }
    }
    declared
}

/// Every `NativeMethods.X(` call site in the generated C#, as `(file, member)`.
fn called_members<'a>(files: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut calls = Vec::new();
    for (name, content) in files {
        for line in content.lines() {
            for (index, _) in line.match_indices("NativeMethods.") {
                let after = &line[index + "NativeMethods.".len()..];
                let Some(member) = leading_identifier(after) else {
                    continue;
                };
                if after[member.len()..].starts_with('(') {
                    calls.push((*name, member.to_owned()));
                }
            }
        }
    }
    calls
}

#[test]
fn every_native_method_called_by_generated_csharp_is_declared_in_native_methods() {
    let generated = generate();
    let files = csharp_files(&generated);
    let declared = declared_members(&files);
    let calls = called_members(&files);

    // Non-vacuity, asserted before the difference: a run that parsed no declarations, or that
    // never rendered the bridge templates, would satisfy an empty-difference assertion while
    // examining nothing. The three names below are exactly the call sites that had no
    // declaration, so they also pin the fixture to the shape this test was written for. ~keep
    assert!(
        declared.len() >= 4,
        "parsed {} P/Invoke declarations — the declaration scanner stopped recognising the \
         emitted shape: {declared:?}",
        declared.len()
    );
    assert!(
        calls.len() >= 5,
        "found {} `NativeMethods.*` call sites — the call scanner stopped recognising the \
         emitted shape: {calls:?}",
        calls.len()
    );
    for required in [
        "NodeVisitorBridgeNew",
        "NodeVisitorBridgeFree",
        "RenderOptionsSetVisitor",
    ] {
        assert!(
            calls.iter().any(|(_, member)| member == required),
            "the fixture must reach the options-field bridge templates, which call `{required}`; \
             without it this cross-check never examines the branch it exists for: {calls:?}"
        );
    }

    let undeclared: Vec<&(&str, String)> = calls
        .iter()
        .filter(|(_, member)| !declared.contains(member.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "generated C# calls native methods that `NativeMethods.cs` never declares — this is \
         CS0117 and the package does not compile:\n{undeclared:#?}\ndeclared: {declared:?}"
    );
}

#[test]
fn options_field_bridge_pinvokes_name_the_entry_points_the_ffi_crate_exports() {
    let generated = generate();
    let files = csharp_files(&generated);
    let native_methods = files
        .iter()
        .find(|(name, _)| *name == "NativeMethods.cs")
        .expect("NativeMethods.cs must be generated")
        .1;

    let pascal_prefix = "SampleCrate";
    let (new_fn, free_fn) = bridge_new_free_symbols(CRATE_NAME, pascal_prefix, TRAIT_NAME);
    let ffi_source = gen_bridge_new_free(CRATE_NAME, pascal_prefix, TRAIT_NAME);

    // The names are only half the contract. Assert against the FFI crate's own emitted Rust that
    // `_new` returns `AlefHandle` and `_free` consumes one, because that is what makes `ulong`
    // (not `IntPtr`, and not `int`) the correct P/Invoke spelling — a mismatched marshalling
    // width is silent memory corruption rather than a compile error. ~keep
    assert!(
        ffi_source.contains(&format!("pub unsafe extern \"C\" fn {new_fn}(")) && ffi_source.contains("-> AlefHandle {"),
        "the FFI emitter must still return `AlefHandle` from `{new_fn}`:\n{ffi_source}"
    );
    assert!(
        ffi_source.contains(&format!("pub unsafe extern \"C\" fn {free_fn}(handle: AlefHandle)")),
        "the FFI emitter must still take `AlefHandle` in `{free_fn}`:\n{ffi_source}"
    );

    for expected in [
        format!("EntryPoint = \"{new_fn}\""),
        format!("EntryPoint = \"{free_fn}\""),
        format!("EntryPoint = \"{CRATE_NAME}_options_set_{OPTIONS_FIELD}\""),
        "internal static extern ulong NodeVisitorBridgeNew(IntPtr vtable, IntPtr userData);".to_owned(),
        "internal static extern void NodeVisitorBridgeFree(ulong handle);".to_owned(),
        "internal static extern void RenderOptionsSetVisitor(ulong options, ulong bridge);".to_owned(),
    ] {
        assert!(
            native_methods.contains(&expected),
            "missing `{expected}` from the generated declarations:\n{native_methods}"
        );
    }
}

/// One `[DllImport]` declaration, reduced to the ABI facts a call site has to agree with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PinvokeDeclaration {
    entry_point: String,
    member: String,
    return_type: String,
    param_types: Vec<String>,
}

/// Parse `NativeMethods.cs` into its declared signatures.
///
/// The declaration is the only place the C# side states the ABI shape of a symbol, so the type
/// dimension has to be read back out of the emitted text — an emitter that computes the right
/// type and then renders the wrong one is exactly the failure being guarded. Attribute prefixes
/// (`[MarshalAs(...)]`) and by-ref modifiers (`out`) stay in the parameter's type string: both
/// are part of the marshalling contract, not decoration. ~keep
fn pinvoke_declarations(native_methods: &str) -> Vec<PinvokeDeclaration> {
    let mut declarations = Vec::new();
    let mut pending_entry_point: Option<String> = None;
    for line in native_methods.lines() {
        let trimmed = line.trim();
        if let Some((_, rest)) = trimmed.split_once("EntryPoint = \"")
            && let Some((entry_point, _)) = rest.split_once('"')
        {
            pending_entry_point = Some(entry_point.to_owned());
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("internal static extern ") else {
            continue;
        };
        let Some(entry_point) = pending_entry_point.take() else {
            continue;
        };
        let (Some(open), Some(close)) = (rest.find('('), rest.rfind(')')) else {
            continue;
        };
        let Some((return_type, member)) = rest[..open].trim().rsplit_once(char::is_whitespace) else {
            continue;
        };
        let param_types = rest[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|param| !param.is_empty())
            .filter_map(|param| {
                param
                    .rsplit_once(char::is_whitespace)
                    .map(|(ty, _)| ty.trim().to_owned())
            })
            .collect();
        declarations.push(PinvokeDeclaration {
            entry_point,
            member: member.trim().to_owned(),
            return_type: return_type.trim().to_owned(),
            param_types,
        });
    }
    declarations
}

fn native_methods_source(files: &[GeneratedFile]) -> String {
    csharp_files(files)
        .iter()
        .find(|(name, _)| *name == "NativeMethods.cs")
        .expect("NativeMethods.cs must be generated")
        .1
        .to_owned()
}

fn declaration_of<'a>(declarations: &'a [PinvokeDeclaration], member: &str) -> &'a PinvokeDeclaration {
    declarations
        .iter()
        .find(|declaration| declaration.member == member)
        .unwrap_or_else(|| {
            panic!("`{member}` must be declared — without it this test examines nothing: {declarations:#?}")
        })
}

/// `HANDLE_CS_TYPE` is `ulong` because the FFI crate's handle is 64-bit, not because `ulong`
/// reads well. Pin it to the FFI template's own definition so a width change there fails here
/// instead of silently corrupting every handle-carrying declaration in this backend. ~keep
#[test]
fn the_ffi_handle_registry_still_declares_a_sixty_four_bit_handle() {
    let handle_registry = include_str!("../src/backends/ffi/templates/handle_registry.rs.jinja");
    let rust_width = handle_registry
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("type AlefHandle = ")
                .and_then(|rest| rest.strip_suffix(';'))
        })
        .unwrap_or_else(|| {
            panic!("the FFI handle registry must still declare `type AlefHandle = <int>;`:\n{handle_registry}")
        });
    let expected_cs_type = match rust_width {
        "u64" => "ulong",
        "u32" => "uint",
        other => panic!(
            "the FFI handle is now `{other}`; pick the matching C# integer type for \
             `HANDLE_PINVOKE_TYPE` before this backend can declare it"
        ),
    };
    assert_eq!(
        HANDLE_CS_TYPE, expected_cs_type,
        "the FFI handle is `{rust_width}`, which marshals as C# `{expected_cs_type}`"
    );
}

/// The visitor-off fixture never renders `bridge_field_register_visitor.jinja` or
/// `native_methods_visitor.jinja`, so the same cross-check has to run again with the flag set.
#[test]
fn every_native_method_called_with_visitor_callbacks_enabled_is_declared() {
    let generated = generate_with_visitor_callbacks();
    let files = csharp_files(&generated);
    let declared = declared_members(&files);
    let calls = called_members(&files);

    assert!(
        declared.len() >= 4,
        "parsed {} P/Invoke declarations — the declaration scanner stopped recognising the \
         emitted shape: {declared:?}",
        declared.len()
    );
    for required in ["VisitorCreate", "VisitorFree", "RenderOptionsSetVisitor"] {
        assert!(
            calls.iter().any(|(_, member)| member == required),
            "the visitor fixture must reach the visitor bridge templates, which call `{required}`; \
             without it this cross-check never examines the branch it exists for: {calls:?}"
        );
    }

    let undeclared: Vec<&(&str, String)> = calls
        .iter()
        .filter(|(_, member)| !declared.contains(member.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "generated C# calls native methods that `NativeMethods.cs` never declares — this is \
         CS0117 and the package does not compile:\n{undeclared:#?}\ndeclared: {declared:?}"
    );
}

/// The FFI crate spells every visitor position `AlefHandle`. Assert that against the FFI's own
/// emitted text — the setter from the real emitter, `visitor_create` / `visitor_free` from the
/// emitter's source, since `backends::ffi::gen_visitor` is a private module and has no callable
/// entry point from an integration test. Either way the C# spelling below breaks the day the
/// FFI width changes, which is the whole point of asserting it here rather than against this
/// backend's own constant. ~keep
#[test]
fn visitor_pinvokes_declare_the_handle_width_the_ffi_crate_exports() {
    let api = visitor_surface();
    let trait_def = api
        .types
        .iter()
        .find(|typ| typ.name == TRAIT_NAME)
        .expect("the fixture must carry the bridge trait");
    let ffi_setter = gen_options_set_bridge(
        CRATE_NAME,
        CRATE_NAME,
        trait_def,
        &HashMap::new(),
        alef::backends::ffi::gen_bridge_field::OptionsFieldBridgeTarget {
            field_name: OPTIONS_FIELD,
            options_type_name: OPTIONS_TYPE,
            use_callbacks_visitor: true,
        },
    );
    let ffi_setter_signature = format!(
        "pub unsafe extern \"C\" fn {CRATE_NAME}_options_set_{OPTIONS_FIELD}(options: AlefHandle, visitor: AlefHandle)"
    );
    assert!(
        ffi_setter.contains(&ffi_setter_signature),
        "the FFI emitter must still take two `AlefHandle`s in the options setter:\n{ffi_setter}"
    );

    let visitor_emitter = include_str!("../src/backends/ffi/gen_visitor/binding_emission.rs");
    for required in [
        "pub unsafe extern \"C\" fn {prefix}_visitor_create(",
        ") -> AlefHandle {{",
        "pub unsafe extern \"C\" fn {prefix}_visitor_free(visitor: AlefHandle) {{",
    ] {
        assert!(
            visitor_emitter.contains(required),
            "the FFI visitor emitter no longer emits `{required}`; the C# declarations below were \
             derived from that signature"
        );
    }

    let generated = generate_with_visitor_callbacks();
    let declarations = pinvoke_declarations(&native_methods_source(&generated));
    assert!(
        declarations.len() >= 6,
        "parsed {} declarations — the signature parser stopped recognising the emitted shape: \
         {declarations:#?}",
        declarations.len()
    );

    let create = declaration_of(&declarations, "VisitorCreate");
    assert_eq!(create.entry_point, format!("{CRATE_NAME}_visitor_create"));
    assert_eq!(
        create.return_type, HANDLE_CS_TYPE,
        "`{CRATE_NAME}_visitor_create` returns `AlefHandle`, not a pointer: {create:#?}"
    );
    assert_eq!(
        create.param_types,
        vec!["IntPtr".to_owned()],
        "the callbacks argument really is a `*const {{Prefix}}VisitorCallbacks`: {create:#?}"
    );

    let free = declaration_of(&declarations, "VisitorFree");
    assert_eq!(free.entry_point, format!("{CRATE_NAME}_visitor_free"));
    assert_eq!(free.return_type, "void");
    assert_eq!(
        free.param_types,
        vec![HANDLE_CS_TYPE.to_owned()],
        "`{CRATE_NAME}_visitor_free` consumes an `AlefHandle`: {free:#?}"
    );

    let setter = declaration_of(&declarations, "RenderOptionsSetVisitor");
    assert_eq!(setter.entry_point, format!("{CRATE_NAME}_options_set_{OPTIONS_FIELD}"));
    assert_eq!(setter.return_type, "void");
    assert_eq!(
        setter.param_types,
        vec![HANDLE_CS_TYPE.to_owned(), HANDLE_CS_TYPE.to_owned()],
        "both setter parameters are `AlefHandle`: {setter:#?}"
    );
}

/// The CS1503 in structural form, derived only from the emitted text: the options handle passed
/// to the setter is produced by `{Options}FromJson`, and the visitor handle by `VisitorCreate`.
/// Whatever those two return is what the setter must accept. This holds no matter what spelling
/// the backend picks, so it survives a future width change that the assertions above would need
/// updating for. ~keep
#[test]
fn the_setter_accepts_exactly_what_its_two_arguments_are_declared_to_return() {
    let generated = generate_with_visitor_callbacks();
    let declarations = pinvoke_declarations(&native_methods_source(&generated));

    let from_json = declaration_of(&declarations, "RenderOptionsFromJson");
    let create = declaration_of(&declarations, "VisitorCreate");
    let setter = declaration_of(&declarations, "RenderOptionsSetVisitor");
    assert_eq!(
        setter.param_types.len(),
        2,
        "the options setter takes an options handle and a visitor handle: {setter:#?}"
    );

    assert_eq!(
        setter.param_types[0], from_json.return_type,
        "`bridge_field_setup.jinja` passes the `{}` returned by `RenderOptionsFromJson` straight \
         into `RenderOptionsSetVisitor`, which is declared to take `{}` — that is CS1503",
        from_json.return_type, setter.param_types[0]
    );
    assert_eq!(
        setter.param_types[1], create.return_type,
        "`bridge_field_register_visitor.jinja` passes the `{}` returned by `VisitorCreate` \
         straight into `RenderOptionsSetVisitor`, which is declared to take `{}` — that is CS1503",
        create.return_type, setter.param_types[1]
    );
}

/// The null check on a native handle has to be derived from the same fact as the declaration
/// that produced it: `IntPtr.Zero` cannot be compared against a `ulong` (CS0019), and `0`
/// against an `IntPtr` is equally wrong. Read both out of the emitted text. ~keep
#[test]
fn the_visitor_bridge_guard_matches_what_visitor_create_is_declared_to_return() {
    let generated = generate_with_visitor_callbacks();
    let files = csharp_files(&generated);
    let declarations = pinvoke_declarations(&native_methods_source(&generated));
    let create = declaration_of(&declarations, "VisitorCreate");

    let mut guards: Vec<String> = Vec::new();
    for (_, content) in &files {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("if (bridgeHandle == ") {
                guards.push(trimmed.to_owned());
            }
        }
    }
    assert!(
        !guards.is_empty(),
        "the fixture must reach the bridge-handle guard; without it this test examines nothing"
    );

    let expected_sentinel = if create.return_type == "IntPtr" {
        "IntPtr.Zero"
    } else {
        "0"
    };
    for guard in &guards {
        assert!(
            guard.contains(&format!("== {expected_sentinel})")),
            "`VisitorCreate` is declared to return `{}`, so its guard must compare against \
             `{expected_sentinel}`: {guard}",
            create.return_type
        );
    }
}
