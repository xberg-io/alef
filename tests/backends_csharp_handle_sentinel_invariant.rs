//! Every C# comparison against a native handle must use the sentinel that pairs with the
//! `[DllImport]` return type the value actually came from.
//!
//! `AlefHandle` is a scalar `uint64_t`, so a `ulong`-returning P/Invoke has to be checked against
//! `0`; a genuinely pointer-typed value (a `char*` result, a `GCHandle`, `SafeHandle`'s own
//! `IntPtr` slot) has to be checked against `IntPtr.Zero`. C# has no unambiguous `==` between
//! `ulong` and `nint`, so mixing the two is `CS0034`, not a style nit.
//!
//! The tests below assert BOTH directions. A one-directional test would pass just as happily if
//! every `IntPtr.Zero` in the emitters were blanket-replaced with `0`, which would break every
//! pointer-returning wrapper instead. ~keep

use alef::backends::csharp::CsharpBackend;
use alef::core::backend::{Backend, GeneratedFile};
use alef::core::config::{AdapterConfig, AdapterParam, AdapterPattern, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FieldDef, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::HashMap;

const HANDLE_TY: &str = "ulong";
const POINTER_TY: &str = "IntPtr";

fn record(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_owned(),
        rust_path: format!("sample_crate::{name}"),
        fields: vec![FieldDef {
            name: "label".to_owned(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        is_clone: true,
        ..TypeDef::default()
    }
}

/// An opaque type carrying, in one class, every return shape whose null check the emitters pick a
/// sentinel for: a streaming method, a handle-returning lookup, and a string-returning lookup.
fn surface() -> ApiSurface {
    let streaming = MethodDef {
        name: "stream_records".to_owned(),
        doc: "Stream records".to_owned(),
        params: vec![ParamDef {
            name: "request".to_owned(),
            ty: TypeRef::Named("RecordQuery".to_owned()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("Record".to_owned()))),
        is_async: true,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };
    let find_record = MethodDef {
        name: "find_record".to_owned(),
        return_type: TypeRef::Optional(Box::new(TypeRef::Named("Record".to_owned()))),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };
    let find_label = MethodDef {
        name: "find_label".to_owned(),
        return_type: TypeRef::Optional(Box::new(TypeRef::String)),
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    };

    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "sample_crate".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            record("Record"),
            record("RecordQuery"),
            TypeDef {
                name: "Engine".to_owned(),
                rust_path: "sample_crate::Engine".to_owned(),
                methods: vec![streaming, find_record, find_label],
                is_opaque: true,
                is_return_type: true,
                doc: "Opaque engine".to_owned(),
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

fn config() -> ResolvedCrateConfig {
    let mut config = ResolvedCrateConfig {
        name: "sample_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    config.adapters.push(AdapterConfig {
        name: "stream_records".to_owned(),
        pattern: AdapterPattern::Streaming,
        core_path: "sample_crate::Engine::stream_records".to_owned(),
        params: vec![AdapterParam {
            name: "request".to_owned(),
            ty: "sample_crate::RecordQuery".to_owned(),
            optional: false,
        }],
        returns: Some("Record".to_owned()),
        error_type: None,
        owner_type: Some("Engine".to_owned()),
        item_type: Some("Record".to_owned()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: None,
        skip_languages: Vec::new(),
    });
    config
}

fn generate() -> Vec<GeneratedFile> {
    CsharpBackend
        .generate_bindings(&surface(), &config())
        .expect("generation should succeed")
}

fn file<'a>(files: &'a [GeneratedFile], name: &str) -> &'a str {
    files
        .iter()
        .find(|f| f.path.ends_with(name))
        .unwrap_or_else(|| panic!("expected {name} to be generated"))
        .content
        .as_str()
}

/// `cs_name -> declared return type` for every `[DllImport]` in `NativeMethods.cs`.
fn pinvoke_return_types(native_methods: &str) -> HashMap<String, String> {
    let mut declarations = HashMap::new();
    for line in native_methods.lines() {
        let Some(rest) = line.trim_start().strip_prefix("internal static extern ") else {
            continue;
        };
        let Some((return_type, rest)) = rest.split_once(' ') else {
            continue;
        };
        let Some((cs_name, _)) = rest.split_once('(') else {
            continue;
        };
        declarations.insert(cs_name.trim().to_owned(), return_type.trim().to_owned());
    }
    declarations
}

fn trailing_identifier(text: &str) -> Option<&str> {
    let end = text.trim_end().len();
    let start = text[..end]
        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map_or(0, |index| index + 1);
    if start < end { Some(&text[start..end]) } else { None }
}

/// One handle comparison whose left operand was traced back to a `[DllImport]` return type.
struct Classified {
    variable: String,
    pinvoke_return_type: String,
    sentinel: &'static str,
    line: String,
}

/// Walks one generated file top to bottom, remembering for each local the return type of the
/// `NativeMethods.*` call that most recently produced it, and reports every sentinel comparison
/// made against such a local.
///
/// Rolling state rather than a whole-file map is deliberate: `nativeResult` is reused by every
/// wrapper in a class and legitimately holds a `ulong` in one method and an `IntPtr` in the next,
/// so a file-wide map would collapse the two and mis-report whichever it overwrote. A local is
/// always assigned before it is compared, so the most recent assignment is the right one. Locals
/// never assigned from a P/Invoke — `SafeHandle`'s inherited `handle`, `GCHandle` pointers — stay
/// unknown and are left alone. ~keep
fn classify_handle_comparisons(content: &str, pinvokes: &HashMap<String, String>) -> (Vec<Classified>, usize) {
    let mut in_scope: HashMap<String, String> = HashMap::new();
    let mut classified = Vec::new();
    let mut handle_assignments = 0usize;

    for line in content.lines() {
        if let Some((assigned, called)) = line.split_once("= NativeMethods.")
            && let Some(name) = trailing_identifier(assigned)
            && let Some((callee, _)) = called.split_once('(')
            && let Some(return_type) = pinvokes.get(callee.trim())
        {
            if return_type.as_str() == HANDLE_TY {
                handle_assignments += 1;
            }
            in_scope.insert(name.to_owned(), return_type.clone());
            continue;
        }

        for operator in ["==", "!="] {
            let Some((left, right)) = line.split_once(operator) else {
                continue;
            };
            let sentinel = if right.trim_start().starts_with('0') {
                "0"
            } else if right.contains("IntPtr.Zero") {
                POINTER_TY
            } else {
                continue;
            };
            let Some(name) = trailing_identifier(left) else {
                continue;
            };
            if let Some(pinvoke_return_type) = in_scope.get(name) {
                classified.push(Classified {
                    variable: name.to_owned(),
                    pinvoke_return_type: pinvoke_return_type.clone(),
                    sentinel,
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    (classified, handle_assignments)
}

#[test]
fn every_native_handle_comparison_uses_the_sentinel_its_pinvoke_return_type_pairs_with() {
    let files = generate();
    let pinvokes = pinvoke_return_types(file(&files, "NativeMethods.cs"));
    assert!(
        pinvokes.values().any(|ty| ty.as_str() == HANDLE_TY) && pinvokes.values().any(|ty| ty.as_str() == POINTER_TY),
        "the fixture must exercise both a handle-returning and a pointer-returning P/Invoke, \
         otherwise this invariant examines nothing: {pinvokes:?}"
    );

    let mut total_classified = 0usize;
    let mut total_handle_assignments = 0usize;
    for generated in &files {
        let Some(name) = generated.path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".cs") {
            continue;
        }
        let (classified, handle_assignments) = classify_handle_comparisons(&generated.content, &pinvokes);
        total_handle_assignments += handle_assignments;
        total_classified += classified.len();
        for comparison in classified {
            let expected = if comparison.pinvoke_return_type == POINTER_TY {
                POINTER_TY
            } else {
                "0"
            };
            assert_eq!(
                comparison.sentinel, expected,
                "{name}: `{}` holds the `{}` result of a P/Invoke but is compared against `{}` \
                 — C# has no unambiguous `==` between `ulong` and `nint`:\n{}",
                comparison.variable, comparison.pinvoke_return_type, comparison.sentinel, comparison.line
            );
        }
    }

    assert!(
        total_handle_assignments >= 4,
        "expected the streaming and lookup wrappers to assign several `ulong` handle locals; \
         saw {total_handle_assignments} — the parser stopped recognising the emitted shape"
    );
    assert!(
        total_classified >= 4,
        "expected several classified handle comparisons; found {total_classified} — a run that \
         examined nothing is indistinguishable from a healthy one"
    );
}

#[test]
fn streaming_wrappers_null_check_scalar_handles_while_safe_handle_keeps_intptr_zero() {
    let files = generate();
    let engine = file(&files, "Engine.cs");

    for expected in [
        "if (requestHandle == 0)",
        "ulong streamHandle;",
        "if (streamHandle == 0)",
        "if (chunkHandle == 0)",
    ] {
        assert!(
            engine.contains(expected),
            "the streaming wrapper consumes `ulong` P/Invoke results and must null-check them \
             against `0`; missing `{expected}`:\n{engine}"
        );
    }

    // The other half of the contract: `SafeHandle`'s own storage slot really is an `IntPtr`, and
    // a `char*` return really is a pointer. Blanket-replacing `IntPtr.Zero` with `0` would break
    // both, so assert them here rather than only asserting the scalar direction. ~keep
    assert!(
        engine.contains("public override bool IsInvalid => handle == IntPtr.Zero;"),
        "SafeHandle's inherited `IntPtr` slot must keep its pointer sentinel:\n{engine}"
    );
    assert!(
        engine.contains("if (nativeResult == IntPtr.Zero)"),
        "a `char*`-returning P/Invoke must keep its pointer sentinel:\n{engine}"
    );
    assert!(
        engine.contains("if (nativeResult == 0)"),
        "a handle-returning P/Invoke must use the scalar sentinel in the same file:\n{engine}"
    );
}

#[test]
fn streaming_adapter_wrapper_takes_a_scalar_owner_and_decodes_item_handles() {
    let files = generate();
    let converter = file(&files, "SampleCrateConverter.cs");

    for expected in [
        "ulong engine",
        "if (iterHandle == 0) throw GetLastError();",
        "if (itemHandle == 0) break;",
        "NativeMethods.RecordToJson(itemHandle)",
        "NativeMethods.RecordFree(itemHandle)",
    ] {
        assert!(
            converter.contains(expected),
            "the adapter wrapper's `_start`/`_next` are declared `ulong` and `_next` yields a \
             registered item handle, not a `char*`; missing `{expected}`:\n{converter}"
        );
    }
    assert!(
        !converter.contains("Marshal.PtrToStringUTF8(itemHandle)"),
        "an item handle is not a string pointer:\n{converter}"
    );
}
