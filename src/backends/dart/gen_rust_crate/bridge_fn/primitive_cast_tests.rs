use super::tests::make_param;
use super::*;

fn no_sets() -> (
    std::collections::HashMap<String, String>,
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    Default::default()
}

#[test]
fn u64_param_passes_through_without_a_self_cast() {
    let (type_paths, needs_from, opaque) = no_sets();
    let cases = [
        (
            make_param("tokens", TypeRef::Primitive(PrimitiveType::U64), false, false, false),
            "tokens",
        ),
        (
            make_param("tokens", TypeRef::Primitive(PrimitiveType::U64), false, false, true),
            "tokens",
        ),
        (
            make_param("tokens", TypeRef::Primitive(PrimitiveType::U64), true, false, false),
            "&tokens",
        ),
        (
            make_param(
                "tokens",
                TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
                false,
                false,
                false,
            ),
            "tokens",
        ),
    ];
    for (p, expected) in cases {
        for checked in [false, true] {
            let mirror = dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, checked);
            let plain = dart_call_arg_checked(&p, checked);
            assert_eq!(mirror, expected, "mirror call arg for {p:?} (checked={checked})");
            assert_eq!(plain, expected, "plain call arg for {p:?} (checked={checked})");
        }
    }
}

#[test]
fn widened_integer_params_keep_their_narrowing_cast() {
    let (type_paths, needs_from, opaque) = no_sets();
    let cases = [
        (PrimitiveType::U32, "count as u32"),
        (PrimitiveType::I32, "count as i32"),
        (PrimitiveType::Usize, "count as usize"),
        (PrimitiveType::F32, "count as f32"),
    ];
    for (prim, expected) in cases {
        let p = make_param("count", TypeRef::Primitive(prim), false, false, false);
        assert_eq!(
            dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false),
            expected
        );
        assert_eq!(dart_call_arg_checked(&p, false), expected);
    }
}

#[test]
fn identity_mapped_params_emit_no_cast() {
    let (type_paths, needs_from, opaque) = no_sets();
    for prim in [PrimitiveType::I64, PrimitiveType::F64, PrimitiveType::Bool] {
        let p = make_param("value", TypeRef::Primitive(prim), false, false, false);
        assert_eq!(
            dart_call_arg_with_mirror_transmute(&p, "mylib", &type_paths, &needs_from, &opaque, false),
            "value"
        );
    }
}

#[test]
fn vec_u64_return_emits_no_element_cast() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U64)));
    assert_eq!(build_primitive_result_cast(&ty, false), "");
    let narrow = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
    assert_eq!(
        build_primitive_result_cast(&narrow, false),
        ".into_iter().map(|x| x as i64).collect::<Vec<_>>()"
    );
}
