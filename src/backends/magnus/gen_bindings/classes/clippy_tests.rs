use super::*;

#[test]
fn should_preserve_owned_core_method_name_with_targeted_borrowed_wrapper_allowance() {
    let typ = TypeDef {
        name: "Context".to_string(),
        rust_path: "sample_core::Context".to_string(),
        is_clone: true,
        ..Default::default()
    };
    for (name, expected_allowance) in [("into_owned", true), ("label", false)] {
        let method = MethodDef {
            name: name.to_string(),
            receiver: Some(ReceiverKind::Owned),
            return_type: TypeRef::String,
            ..Default::default()
        };
        let code = gen_instance_method(&method, &MagnusMapper, &typ, &AHashSet::default(), "sample_core");
        let parsed: syn::ImplItemFn = syn::parse_str(&code).expect("generated method must parse");
        assert_eq!(parsed.sig.ident, name);
        assert!(matches!(
            parsed.sig.receiver().expect("wrapper must receive self").kind,
            syn::ReceiverKind::Reference(..)
        ));
        assert_eq!(
            code.contains("clippy::wrong_self_convention"),
            expected_allowance,
            "only a consuming core into_* method needs the borrowed Ruby wrapper exception: {code}"
        );
        assert!(
            code.contains(&format!("core_self.{name}()")),
            "must call the original core API: {code}"
        );
    }
}
