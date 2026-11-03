use super::*;
use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

fn surface() -> ApiSurface {
    let count = MethodDef {
        name: "count".into(),
        receiver: Some(ReceiverKind::Ref),
        params: vec![ParamDef {
            name: "text".into(),
            ty: TypeRef::String,
            is_ref: true,
            ..Default::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::Usize),
        ..Default::default()
    };
    let mut checked = count.clone();
    checked.name = "checked_count".into();
    checked.has_default_impl = true;
    checked.error_type = Some("Error".into());
    ApiSurface {
        types: vec![TypeDef {
            name: "Counter".into(),
            rust_path: "sample::Counter".into(),
            is_trait: true,
            methods: vec![count, checked],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn aliases() -> PythonHostAliases {
    PythonHostAliases::from_raw(&toml::from_str("[traits.Counter]\nchecked_count = 'count'").unwrap()).unwrap()
}

#[test]
fn explicit_alias_does_not_require_a_try_prefix() {
    let aliases = aliases();
    let api = surface();
    aliases.validate(&api).unwrap();
    assert_eq!(aliases.for_trait(&api.types[0]).get("checked_count").unwrap(), "count");
}

#[test]
fn rejects_unknown_configuration_fields() {
    let raw = toml::from_str("typo = 'count'").unwrap();
    assert!(format!("{:#}", PythonHostAliases::from_raw(&raw).unwrap_err()).contains("unknown field"));
}

#[test]
fn rejects_unknown_and_ambiguous_traits() {
    let mut api = surface();
    api.types[0].name = "Other".into();
    assert!(aliases().validate(&api).unwrap_err().to_string().contains("Counter"));
    api = surface();
    let mut other = api.types[0].clone();
    other.rust_path = "other::Counter".into();
    api.types.push(other);
    assert!(aliases().validate(&api).unwrap_err().to_string().contains("ambiguous"));
}

#[test]
fn rejects_unknown_callback_and_target() {
    for index in [0, 1] {
        let mut api = surface();
        api.types[0].methods.remove(index);
        assert!(aliases().validate(&api).unwrap_err().to_string().contains("method"));
    }
}

#[test]
fn rejects_incompatible_callback_shapes() {
    for mutation in 0..6 {
        let mut api = surface();
        let method = &mut api.types[0].methods[1];
        match mutation {
            0 => method.error_type = None,
            1 => method.has_default_impl = false,
            2 => method.is_async = true,
            3 => method.receiver = Some(ReceiverKind::RefMut),
            4 => method.return_type = TypeRef::String,
            _ => method.params[0].is_ref = false,
        }
        assert!(aliases().validate(&api).is_err(), "accepted mutation {mutation}");
    }
}

#[test]
fn rejects_fallible_or_defaulted_host_target() {
    for fallible in [false, true] {
        let mut api = surface();
        if fallible {
            api.types[0].methods[0].error_type = Some("Error".into());
        } else {
            api.types[0].methods[0].has_default_impl = true;
        }
        assert!(aliases().validate(&api).is_err());
    }
}

#[test]
fn qualified_trait_alias_is_unambiguous() {
    let raw = toml::from_str("[traits.'sample::Counter']\nchecked_count = 'count'").unwrap();
    let aliases = PythonHostAliases::from_raw(&raw).unwrap();
    let api = surface();
    aliases.validate(&api).unwrap();
    assert_eq!(aliases.for_trait(&api.types[0]).len(), 1);
}

fn config() -> crate::core::config::ResolvedCrateConfig {
    crate::core::config::ResolvedCrateConfig {
        name: "sample".into(),
        core_import: Some("sample".into()),
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "Counter".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated(raw: &str) -> String {
    use crate::core::backend::Backend as _;
    let backend = crate::backends::pyo3::Pyo3Backend::with_host_aliases(&toml::from_str(raw).unwrap()).unwrap();
    backend
        .generate_bindings(&surface(), &config())
        .unwrap()
        .into_iter()
        .map(|f| f.content)
        .collect()
}

#[test]
fn aliases_emit_result_conversion_and_explicit_override_precedence() {
    let code = generated("[traits.Counter]\nchecked_count = 'count'");
    assert!(code.contains("match obj.getattr(\"checked_count\")"));
    assert!(code.contains("Ok(method) => method"));
    assert!(code.contains("obj.getattr(\"count\")?"));
    assert!(code.contains("method 'checked_count' failed"));
    assert!(!code.contains("struct PyCounterBridgeDefaultCheckedCount"));
}

#[test]
fn descriptor_errors_are_distinguished_from_missing_optional_methods() {
    let code = generated("[traits.Counter]\nchecked_count = 'count'");
    assert!(code.contains("is_instance_of::<pyo3::exceptions::PyAttributeError>(py)"));
    assert!(code.contains("call_method1(\"getattr_static\", (obj, \"checked_count\"))"));
    assert!(code.contains("Ok(_) => return Err(error)"));
    assert!(code.contains("Err(error) => return Err(error)"));
}

#[test]
fn empty_alias_configuration_preserves_all_existing_output() {
    use crate::core::backend::Backend as _;
    let original = crate::backends::pyo3::Pyo3Backend
        .generate_bindings(&surface(), &config())
        .unwrap();
    let original: String = original.into_iter().map(|f| f.content).collect();
    assert_eq!(generated(""), original);
}

#[test]
fn explicit_alias_preserves_unrelated_callback_bodies() {
    let original = generated("");
    let aliased = generated("[traits.Counter]\nchecked_count = 'count'");
    let body = |code: &str| {
        code.split("impl sample::Counter for PyCounterBridge {")
            .nth(1)
            .expect("actual bridge impl must be emitted")
            .split("fn count(")
            .nth(1)
            .expect("required callback must be emitted")
            .split("\n    }")
            .next()
            .unwrap()
            .to_owned()
    };
    assert_eq!(body(&original), body(&aliased));
}

#[test]
fn aliases_do_not_add_required_protocol_methods() {
    use crate::core::backend::Backend as _;
    let raw = toml::from_str("[traits.Counter]\nchecked_count = 'count'").unwrap();
    let backend = crate::backends::pyo3::Pyo3Backend::with_host_aliases(&raw).unwrap();
    let aliased = backend.generate_type_stubs(&surface(), &config()).unwrap();
    let original = crate::backends::pyo3::Pyo3Backend
        .generate_type_stubs(&surface(), &config())
        .unwrap();
    let bytes = |files: Vec<crate::core::backend::GeneratedFile>| -> Vec<_> {
        files.into_iter().map(|f| (f.path, f.content)).collect()
    };
    assert_eq!(bytes(aliased), bytes(original));
}

#[test]
fn unrelated_default_delegate_forwards_the_aliased_fallible_callback() {
    use crate::core::backend::Backend as _;
    let mut api = surface();
    let mut describe = api.types[0].methods[1].clone();
    describe.name = "describe".into();
    api.types[0].methods.push(describe);
    let raw = toml::from_str("[traits.Counter]\nchecked_count = 'count'").unwrap();
    let backend = crate::backends::pyo3::Pyo3Backend::with_host_aliases(&raw).unwrap();
    let code: String = backend
        .generate_bindings(&api, &config())
        .unwrap()
        .into_iter()
        .map(|f| f.content)
        .collect();
    let delegate = code
        .split("impl sample::Counter for PyCounterBridgeDefaultDescribe<'_> {")
        .nth(1)
        .expect("the unrelated default must have a Rust fallback delegate")
        .split("\n}\n")
        .next()
        .unwrap();
    assert!(delegate.contains("fn checked_count("), "{delegate}");
    assert!(
        delegate.contains("sample::Counter::checked_count(self.0, text)"),
        "{delegate}"
    );
    assert!(!code.contains("struct PyCounterBridgeDefaultCheckedCount"));
}
