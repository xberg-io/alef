//! Every reason the TypeScript e2e generator refuses an assertion FIELD outright, in the order
//! it applies them.
//!
//! ~keep Split out of `assertions.rs` (already over the repo's 1,000-line cap) rather than grown
//! there, matching the precedent set by `is_true_tests.rs` and `node_enum_import_tests.rs`.
//!
//! The refusals are ordered, not independent: the result-type miss is checked first because it is
//! the coarse "does this path exist at all" question, and the tagged-union crossing second
//! because it only makes sense for a path that does exist. Keeping them behind one entry point
//! means a caller cannot add a third refusal by writing another `writeln!` somewhere in the
//! render path and quietly bypassing the `FieldSkip` funnel the strict gate counts.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// The rendered `// skipped: ...` line refusing `field`, or `None` when the field survives every
/// refusal and the caller should render a real assertion for it.
///
/// The returned line carries its own trailing newline, so callers `push_str` it directly.
pub(super) fn refusal_line(field: &str, field_resolver: &FieldResolver, lang: &str) -> Option<String> {
    if !field_resolver.is_valid_for_result(field) {
        return Some(skip_line(FieldSkip::NotAvailableOnResultType, field));
    }
    // Ask the same authority gleam, dart, kotlin and swift ask -- the consumer's own
    // `fields_method_calls`, read through `FieldResolver::tagged_union_split` -- rather than
    // re-deriving "is this a union crossing" from the path's shape here. This generator used to
    // ask nobody: `test_case.rs` built its resolver with an EMPTY method-call set, so the split
    // answered `None` for every path and the boundary was rendered verbatim by an accessor
    // renderer whose only per-segment decision is `.` vs `?.`.
    //
    // A declared crossing is no longer an automatic refusal, though: `FieldResolver::
    // typescript_tagged_union_accessor` asks the IR whether THIS SPECIFIC variant shape gives
    // either binding a real member to spell -- napi flattens a single-tuple-Named-type variant
    // into a REAL named optional field, and wasm's internally-tagged `JsValue` bridging flattens
    // the payload onto the discriminant's own object. Only when the IR can't confirm either of
    // those (no IR at all, or a variant shape neither binding names) does this fall back to the
    // blanket refusal. ~keep
    if field_resolver.tagged_union_split(field).is_some()
        && field_resolver.typescript_tagged_union_accessor(field, lang, "result").is_none()
    {
        return Some(skip_line(FieldSkip::CrossesTaggedUnionBoundaryInTypescript, field));
    }
    None
}

fn skip_line(skip: FieldSkip, field: &str) -> String {
    format!("    // skipped: {}\n", skip.message(field))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::refusal_line;
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;

    /// A resolver holding nothing but the crossing declaration, mirroring what
    /// `test_file/test_case.rs` builds from `[e2e].fields_method_calls`. Empty `result_fields`
    /// makes `is_valid_for_result` accept every path, so the first refusal cannot mask the
    /// second and each test isolates the refusal it names. Carries no IR (`with_ir_enum_map` is
    /// never called), so `typescript_tagged_union_accessor` can never resolve a real member --
    /// exactly the "no IR data" gap `refusal_line`'s fallback covers.
    fn resolver_declaring(method_calls: &[&str]) -> FieldResolver {
        let declared: HashSet<String> = method_calls.iter().map(|entry| (*entry).to_string()).collect();
        let empty = HashSet::new();
        FieldResolver::new(&HashMap::new(), &empty, &empty, &empty, &declared)
    }

    /// A `fields_method_calls` entry names `<enum field path>.<variant>` -- `shape.circle` for
    /// the crossing `shape.circle.radius` walks -- exactly as `kotlin/assertions/tests.rs`
    /// declares it. ~keep
    #[test]
    fn a_declared_tagged_union_crossing_is_refused_rather_than_spelled() {
        let resolver = resolver_declaring(&["shape.circle"]);

        let line = refusal_line("shape.circle.radius", &resolver, "node")
            .expect("a declared union crossing must be refused, not rendered as an accessor");

        assert_eq!(
            line,
            "    // skipped: field 'shape.circle.radius' crosses a tagged-union variant boundary \
             (no variant member on the generated TypeScript type)\n"
        );
    }

    /// The control that stops "refuse everything" from passing: ordinary struct paths through the
    /// very same resolver must survive so the caller renders their normal accessors -- including
    /// the union field itself, which is a real member and only its VARIANT segment is not.
    #[test]
    fn an_ordinary_struct_field_is_not_refused() {
        let resolver = resolver_declaring(&["shape.circle"]);

        assert_eq!(refusal_line("summary.title", &resolver, "node"), None);
        assert_eq!(refusal_line("shape", &resolver, "node"), None);
    }

    /// The refusal is keyed on the consumer's declaration, not on "the path has three segments".
    /// With nothing declared, the same path is spelled exactly as before this refusal existed.
    #[test]
    fn an_undeclared_deep_path_is_not_refused() {
        let resolver = resolver_declaring(&[]);

        assert_eq!(refusal_line("shape.circle.radius", &resolver, "node"), None);
    }

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    /// A resolver whose IR positively resolves the crossing to a single-tuple-Named-type
    /// variant -- the shape napi flattens into a real named field and wasm's internally-tagged
    /// `JsValue` bridging flattens onto the discriminant's own object. `fields_method_calls`
    /// still has to declare the crossing (`tagged_union_split` is the first gate `refusal_line`
    /// checks), matching a real `[e2e].fields_method_calls` entry.
    fn resolver_over_format_metadata() -> FieldResolver {
        let types = vec![TypeDef {
            name: "Metadata".to_string(),
            fields: vec![field("format", TypeRef::Named("FormatMetadata".to_string()))],
            ..TypeDef::default()
        }];
        let enums = vec![EnumDef {
            name: "FormatMetadata".to_string(),
            serde_tag: Some("format_type".to_string()),
            variants: vec![EnumVariant {
                name: "Html".to_string(),
                is_tuple: true,
                fields: vec![field("_0", TypeRef::Named("HtmlMetadata".to_string()))],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }];
        resolver_declaring(&["format.html"]).with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("Metadata".to_string()),
        )
    }

    /// The fix: once the IR confirms napi gives this crossing a real named field, node must not
    /// refuse it any more.
    #[test]
    fn a_napi_reachable_crossing_is_no_longer_refused() {
        let resolver = resolver_over_format_metadata();
        assert_eq!(refusal_line("format.html.title", &resolver, "node"), None);
    }

    /// Same IR, wasm side: the flattened `JsValue` payload reaches the leaf too.
    #[test]
    fn a_wasm_reachable_crossing_is_no_longer_refused() {
        let resolver = resolver_over_format_metadata();
        assert_eq!(refusal_line("format.html.title", &resolver, "wasm"), None);
    }

    /// The control that makes the two tests above meaningful: a variant shape the IR does NOT
    /// resolve to a single Named payload (two inline fields) must still be refused, proving this
    /// is a real per-shape reachability check and not "declared crossings are never refused any
    /// more".
    #[test]
    fn a_multi_field_variant_crossing_is_still_refused() {
        let types = vec![TypeDef {
            name: "Metadata".to_string(),
            fields: vec![field("auth", TypeRef::Named("AuthConfig".to_string()))],
            ..TypeDef::default()
        }];
        let enums = vec![EnumDef {
            name: "AuthConfig".to_string(),
            serde_tag: Some("type".to_string()),
            variants: vec![EnumVariant {
                name: "Basic".to_string(),
                fields: vec![
                    field("username", TypeRef::String),
                    field("password", TypeRef::String),
                ],
                ..EnumVariant::default()
            }],
            ..EnumDef::default()
        }];
        let resolver = resolver_declaring(&["auth.basic"]).with_ir_enum_map(
            FieldResolver::ir_enum_fields(&types, &enums),
            Some("Metadata".to_string()),
        );

        assert!(
            refusal_line("auth.basic.username", &resolver, "node").is_some(),
            "a multi-field variant has no single payload type to resolve and must stay refused"
        );
    }
}
