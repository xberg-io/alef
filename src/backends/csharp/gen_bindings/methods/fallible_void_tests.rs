//! Fallible unit returns keep a public void API but carry an int32 native status.

use super::wrappers::{gen_wrapper_function, gen_wrapper_method};
use crate::backends::csharp::gen_bindings::pinvoke::{gen_pinvoke_for_func, gen_pinvoke_for_method};
use crate::core::ir::{FunctionDef, MethodDef, ReceiverKind, TypeRef};
use std::collections::{HashMap, HashSet};

fn function(fallible: bool, asynchronous: bool) -> FunctionDef {
    FunctionDef {
        name: "validate".into(),
        rust_path: "sample_core::validate".into(),
        return_type: TypeRef::Unit,
        error_type: fallible.then(|| "SampleError".into()),
        is_async: asynchronous,
        ..Default::default()
    }
}

fn method(fallible: bool, asynchronous: bool, instance: bool) -> MethodDef {
    MethodDef {
        name: "validate".into(),
        return_type: TypeRef::Unit,
        error_type: fallible.then(|| "SampleError".into()),
        is_async: asynchronous,
        is_static: !instance,
        receiver: instance.then_some(ReceiverKind::Ref),
        ..Default::default()
    }
}

#[test]
fn fallible_void_pinvoke_preserves_status_for_functions_and_methods() {
    for fallible in [false, true] {
        let expected = if fallible { "int" } else { "void" };
        let declarations = [
            gen_pinvoke_for_func(
                "sample_validate",
                &function(fallible, false),
                &HashSet::new(),
                &HashSet::new(),
                &HashMap::new(),
                &Default::default(),
            ),
            gen_pinvoke_for_method(
                "sample_settings_validate",
                "Validate",
                &method(fallible, false, true),
                &HashMap::new(),
                &Default::default(),
            ),
            gen_pinvoke_for_method(
                "sample_settings_validate",
                "Validate",
                &method(fallible, false, false),
                &HashMap::new(),
                &Default::default(),
            ),
        ];
        for declaration in declarations {
            assert!(
                declaration.contains(&format!("extern {expected} Validate(")),
                "{declaration}"
            );
        }
    }
}

fn wrap_function(fallible: bool, asynchronous: bool) -> String {
    let empty = HashSet::new();
    gen_wrapper_function(
        &function(fallible, asynchronous),
        "SampleException",
        "sample",
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        false,
        &[],
    )
}

fn wrap_method(fallible: bool, asynchronous: bool, instance: bool) -> String {
    let empty = HashSet::new();
    gen_wrapper_method(
        &method(fallible, asynchronous, instance),
        "SampleException",
        "sample",
        "Settings",
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &empty,
        &[],
    )
}

#[test]
fn fallible_void_wrappers_check_status_without_changing_public_return_type() {
    for asynchronous in [false, true] {
        for fallible in [false, true] {
            for code in [
                wrap_function(fallible, asynchronous),
                wrap_method(fallible, asynchronous, true),
                wrap_method(fallible, asynchronous, false),
            ] {
                let public_return = if asynchronous {
                    "public static async Task "
                } else {
                    "public static void "
                };
                assert!(code.contains(public_return), "{code}");
                assert_eq!(code.contains("var nativeResult = NativeMethods."), fallible, "{code}");
                assert_eq!(code.contains("if (nativeResult != 0)"), fallible, "{code}");
                assert_eq!(code.contains("throw GetLastError();"), fallible, "{code}");
            }
        }
    }
}

fn wrap_bridge_function(fallible: bool) -> String {
    use crate::codegen::generators::trait_bridge::BridgeFieldMatch;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{FieldDef, ParamDef};
    let bridge = TraitBridgeConfig {
        trait_name: "Visitor".into(),
        ..Default::default()
    };
    let field = FieldDef {
        name: "visitor".into(),
        ..Default::default()
    };
    let matched = BridgeFieldMatch {
        param_index: 0,
        param_name: "options".into(),
        options_type: "Options".into(),
        param_is_optional: false,
        field_name: "visitor".into(),
        field: &field,
        bridge: &bridge,
    };
    let empty = HashSet::new();
    let mut func = function(fallible, false);
    func.params.push(ParamDef {
        name: "options".into(),
        ty: TypeRef::Named("Options".into()),
        ..Default::default()
    });
    super::bridge_fields::gen_bridge_field_wrapper_function(&func, &matched, "SampleException", &empty, &empty, &empty)
}

#[test]
fn fallible_void_bridge_wrappers_check_both_native_call_paths() {
    for fallible in [false, true] {
        let code = wrap_bridge_function(fallible);
        assert_eq!(
            code.matches("var nativeResult = NativeMethods.Validate(").count(),
            if fallible { 2 } else { 0 },
            "{code}"
        );
        assert_eq!(
            code.matches("if (nativeResult != 0)").count(),
            if fallible { 2 } else { 0 },
            "{code}"
        );
    }
}
