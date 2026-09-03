use std::collections::{HashMap, HashSet};

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

/// A `Named` type resolves to either a struct or an enum definition. Both carry field data
/// that can nest a file input, so `Named` resolution must consult whichever one matches. ~keep
#[derive(Clone, Copy)]
enum NamedDef<'a> {
    Struct(&'a TypeDef),
    Enum(&'a EnumDef),
}

type NamedIndex<'a> = HashMap<&'a str, NamedDef<'a>>;

/// Build a single by-name lookup over both structs and enums, once per top-level call.
/// `Named` resolution then costs an O(1) map lookup instead of a linear scan of the
/// registry at every level of recursion. ~keep
fn build_named_index<'a>(type_defs: &'a [TypeDef], enums: &'a [EnumDef]) -> NamedIndex<'a> {
    let mut index = HashMap::with_capacity(type_defs.len() + enums.len());
    for definition in type_defs {
        index.insert(definition.name.as_str(), NamedDef::Struct(definition));
    }
    for definition in enums {
        index.insert(definition.name.as_str(), NamedDef::Enum(definition));
    }
    index
}

/// The fixture-independent half of a file-input scan: the by-name index over the crate's structs
/// and enums.
///
/// Nothing in the index depends on the fixture or on the target language, yet every generator asks
/// the same question once per fixture -- and every generator asks it. Holding the index in a value
/// the generator builds once turns that O(languages * fixtures * definitions) rebuild into one
/// build per generator, with an O(1) lookup per fixture. ~keep
pub(super) struct FileInputScan<'a> {
    index: NamedIndex<'a>,
}

impl<'a> FileInputScan<'a> {
    pub(super) fn new(type_defs: &'a [TypeDef], enums: &'a [EnumDef]) -> Self {
        Self {
            index: build_named_index(type_defs, enums),
        }
    }

    pub(super) fn fixture_uses_test_documents(&self, fixture: &Fixture, call: &CallConfig) -> bool {
        scan_fixture(&self.index, fixture, call).0
    }
}

/// The un-hoisted form, retained so the behaviour tests below can state "these definitions, this
/// fixture" as a single call. Generators must go through `FileInputScan` instead: they ask this
/// question once per fixture, and this entry point rebuilds the whole index every time. ~keep
#[cfg(test)]
pub(super) fn fixture_uses_test_documents(
    fixture: &Fixture,
    call: &CallConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> bool {
    FileInputScan::new(type_defs, enums).fixture_uses_test_documents(fixture, call)
}

/// Scan one fixture against a prebuilt index, reporting the answer alongside the number of
/// `Named` resolutions the traversal entered.
///
/// The count is the traversal's unit of work and its complexity witness: a type graph where the
/// same value is reachable by many paths (flattened fields and untagged/internally tagged enum
/// variants all recurse against the SAME JSON value) revisits named types once per path, which is
/// exponential in the number of diamonds. Returning the count lets a regression test bound it
/// instead of timing it. ~keep
fn scan_fixture(index: &NamedIndex<'_>, fixture: &Fixture, call: &CallConfig) -> (bool, usize) {
    let mut walk = DocumentWalk::new(index);
    let found = fixture.resolved_args(call).iter().any(|argument| {
        if !fixture.docs_files_for_arg(&argument.field).is_empty() || argument.arg_type == "file_path" {
            return true;
        }

        let value = super::resolve_field(&fixture.input, &argument.field);
        if argument.arg_type == "bytes" {
            return value.as_str().is_some_and(is_relative_document_path);
        }
        if argument.arg_type != "json_object" {
            return false;
        }

        let Some(element_type) = argument.element_type.as_deref() else {
            return false;
        };
        match value.as_array() {
            Some(values) => values.iter().any(|element| walk.resolve_named(element, element_type)),
            None => walk.resolve_named(value, element_type),
        }
    });
    (found, walk.named_resolutions)
}

/// Identity of a JSON sub-value, forming half of a memo key.
///
/// Every value a scan reaches is borrowed out of the one live `Fixture::input` tree (or the
/// `'static` `Value::Null` `resolve_field` falls back to when a path misses), and the memo lives
/// no longer than the scan. No referent can therefore be dropped while an entry naming it is
/// still readable, so an address cannot be recycled for a different value mid-scan. ~keep
fn value_identity(value: &serde_json::Value) -> usize {
    std::ptr::from_ref(value).addr()
}

/// One fixture's traversal state: the cycle guard, the memo, and the work counter.
///
/// `active` and `memo` answer two different questions and must not be merged. `active` is
/// path-scoped and only ever *cuts* recursion; `memo` is scan-scoped and caches finished answers.
/// A result computed while a cut occurred anywhere beneath it is NOT cacheable, because that
/// `false` reflects the path taken rather than the value/type pair alone -- see `resolve_named`. ~keep
struct DocumentWalk<'defs, 'index> {
    index: &'index NamedIndex<'defs>,
    active: HashSet<&'defs str>,
    memo: HashMap<(usize, &'defs str), bool>,
    cycle_cut: bool,
    named_resolutions: usize,
}

impl<'defs, 'index> DocumentWalk<'defs, 'index> {
    fn new(index: &'index NamedIndex<'defs>) -> Self {
        Self {
            index,
            active: HashSet::new(),
            memo: HashMap::new(),
            cycle_cut: false,
            named_resolutions: 0,
        }
    }

    fn typed_value(&mut self, value: &serde_json::Value, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Bytes => value.as_str().is_some_and(is_relative_document_path),
            TypeRef::Optional(inner) => self.typed_value(value, inner),
            TypeRef::Vec(inner) => value
                .as_array()
                .is_some_and(|values| values.iter().any(|value| self.typed_value(value, inner))),
            TypeRef::Map(_, value_type) => value
                .as_object()
                .is_some_and(|values| values.values().any(|value| self.typed_value(value, value_type))),
            TypeRef::Named(name) => self.resolve_named(value, name),
            _ => false,
        }
    }

    /// Resolve a `Named` type against the combined struct/enum index, memoizing per
    /// (value identity, type name) and guarding against cycles.
    ///
    /// A `#[serde(flatten)]` field recurses against the SAME JSON value (see `fields`) rather than
    /// a smaller sub-value, so -- unlike the rest of this traversal -- recursion here is not
    /// bounded by the shrinking size of the JSON value. Two consequences follow. A self-referential
    /// definition reached only through flattened fields would recurse forever, which `active`
    /// prevents; and a definition reachable by many distinct paths is otherwise re-walked once per
    /// path, which the memo collapses to once per (value, name).
    ///
    /// The memo is consulted BEFORE `active`: a cached answer was computed without ever consulting
    /// the cycle guard (see below), so it is the path-independent answer and is correct to return
    /// even for a name currently on the active path.
    ///
    /// `cycle_cut` is what keeps the two mechanisms compatible. A subtree whose evaluation hit the
    /// guard returned `false` *because of the path*, not because of the value, so caching it would
    /// leak a path-dependent answer into a sibling branch that has no such cycle. Only results
    /// computed with no cut anywhere beneath them are stored -- those never read `active` at all,
    /// so they equal what an unguarded traversal would compute. ~keep
    fn resolve_named(&mut self, value: &serde_json::Value, name: &str) -> bool {
        self.named_resolutions += 1;
        let index = self.index;
        let Some((&definition_name, definition)) = index.get_key_value(name) else {
            return false;
        };
        let key = (value_identity(value), definition_name);
        if let Some(&cached) = self.memo.get(&key) {
            return cached;
        }
        if !self.active.insert(definition_name) {
            self.cycle_cut = true;
            return false;
        }

        let outer_cut = std::mem::replace(&mut self.cycle_cut, false);
        let found = match *definition {
            NamedDef::Struct(struct_def) => {
                self.fields(value, &struct_def.fields, struct_def.serde_rename_all.as_deref())
            }
            NamedDef::Enum(enum_def) => self.enum_value(value, enum_def),
        };
        let inner_cut = self.cycle_cut;

        self.cycle_cut = outer_cut || inner_cut;
        self.active.remove(definition_name);
        if !inner_cut {
            self.memo.insert(key, found);
        }
        found
    }

    /// Walk an object's fields against a JSON object value, shared by struct bodies and
    /// struct-shaped enum variants (both are "named fields against an object" in the same way). ~keep
    fn fields(&mut self, value: &serde_json::Value, fields: &[FieldDef], rename_all: Option<&str>) -> bool {
        let Some(object) = value.as_object() else {
            return false;
        };
        fields.iter().any(|field| {
            if field.serde_flatten {
                // Flattened fields have no wire key of their own -- their sub-fields appear
                // inline in the SAME parent object, so recurse against `value`, not a nested
                // sub-value. ~keep
                return self.typed_value(value, &field.ty);
            }
            let wire_name =
                crate::codegen::naming::wire_field_name(&field.name, field.serde_rename.as_deref(), rename_all);
            let Some(field_value) = object.get(&field.name).or_else(|| object.get(&wire_name)) else {
                return false;
            };
            self.typed_value(field_value, &field.ty)
        })
    }

    fn enum_value(&mut self, value: &serde_json::Value, definition: &EnumDef) -> bool {
        definition
            .variants
            .iter()
            .any(|variant| self.variant(value, definition, variant))
    }

    fn variant(&mut self, value: &serde_json::Value, definition: &EnumDef, variant: &EnumVariant) -> bool {
        let Some(candidate) = variant_payload(value, definition, variant) else {
            return false;
        };
        if variant.is_tuple {
            return self.tuple_variant(candidate, &variant.fields);
        }
        // `definition.serde_rename_all` cases the enum's VARIANT names (used above, in
        // `variant_payload`) -- a different serde namespace from how this variant's own payload
        // FIELDS are cased. `EnumVariant` carries no per-variant field-casing rule in the IR, so
        // there is no correct value to pass here; borrowing the enum's rule produced false matches
        // whenever a field happened to collide with that unrelated casing. Pass `None` so only the
        // raw field name and each field's own explicit `serde_rename` are tried (both still handled
        // by `fields`'s `.or_else` fallback). ~keep
        self.fields(candidate, &variant.fields, None)
    }

    fn tuple_variant(&mut self, candidate: &serde_json::Value, fields: &[FieldDef]) -> bool {
        if let [only] = fields {
            return self.typed_value(candidate, &only.ty);
        }
        let Some(values) = candidate.as_array() else {
            return false;
        };
        fields
            .iter()
            .zip(values.iter())
            .any(|(field, value)| self.typed_value(value, &field.ty))
    }
}

/// Locate the sub-value that carries a variant's payload, per the enum's serde tagging style. ~keep
fn variant_payload<'a>(
    value: &'a serde_json::Value,
    definition: &EnumDef,
    variant: &EnumVariant,
) -> Option<&'a serde_json::Value> {
    if definition.serde_untagged {
        return Some(value);
    }
    let Some(tag_key) = &definition.serde_tag else {
        let wire_name = crate::codegen::naming::wire_variant_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        );
        return value.get(&wire_name);
    };
    // Internally/adjacently tagged: the tag key's value must actually name THIS variant before
    // its fields are walked. Two variants can otherwise share a field name and have the wrong
    // one's type "accidentally" match the selected variant's real payload. ~keep
    if !tag_matches_variant(value, tag_key, definition, variant) {
        return None;
    }
    match &definition.serde_content {
        Some(content_key) => value.get(content_key),
        // Internally tagged: variant fields sit inline in the same object as the tag key. ~keep
        None => Some(value),
    }
}

/// True when `value`'s tag key names exactly this variant, via the SAME wire-name derivation
/// `wire_variant_value` computes above for externally tagged enums -- routing through the
/// central helper here, rather than re-deriving casing locally, is what keeps a comparison
/// mistake from silently missing a real variant match, and thus a real file input. ~keep
fn tag_matches_variant(value: &serde_json::Value, tag_key: &str, definition: &EnumDef, variant: &EnumVariant) -> bool {
    let wire_name = crate::codegen::naming::wire_variant_value(
        &variant.name,
        variant.serde_rename.as_deref(),
        definition.serde_rename_all.as_deref(),
    );
    value.get(tag_key).and_then(serde_json::Value::as_str) == Some(wire_name.as_str())
}

fn is_relative_document_path(value: &str) -> bool {
    if value.starts_with('<') || value.starts_with('{') || value.starts_with('[') || value.contains(' ') {
        return false;
    }
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_alphanumeric() && first != '_' {
        return false;
    }
    value
        .find('/')
        .map(|slash| &value[slash + 1..])
        .is_some_and(|suffix| !suffix.is_empty() && suffix.contains('.'))
}

#[cfg(test)]
mod cycle_guard_tests;
#[cfg(test)]
mod cycle_memo_taint_tests;
#[cfg(test)]
mod tag_and_shape_tests;
#[cfg(test)]
mod tag_value_discrimination_tests;
#[cfg(test)]
mod variant_field_rename_all_tests;
#[cfg(test)]
mod wide_dag_memo_tests;

#[cfg(test)]
mod tests {
    use crate::core::config::e2e::{ArgMapping, CallConfig};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn object_arg() -> ArgMapping {
        ArgMapping {
            name: "request".into(),
            field: "input".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: Some("SampleRequest".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn request_type() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn nested_bytes_file_path_requires_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"content": "documents/sample.bin"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    #[test]
    fn nested_inline_bytes_do_not_require_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"content": "inline text"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    #[test]
    fn batch_nested_bytes_file_path_requires_test_document_working_directory() {
        let mut argument = object_arg();
        argument.field = "input.requests".into();
        let fixture = Fixture {
            input: serde_json::json!({
                "requests": [
                    {"content": "inline text"},
                    {"content": "documents/sample.bin"}
                ]
            }),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![argument],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_type()],
            &[]
        ));
    }

    /// Externally tagged (serde default) enum with one struct-shaped variant carrying bytes. ~keep
    fn event_enum() -> EnumDef {
        EnumDef {
            name: "SampleEvent".into(),
            variants: vec![EnumVariant {
                name: "Uploaded".into(),
                fields: vec![FieldDef {
                    name: "file".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn request_with_event_type() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "event".into(),
                ty: TypeRef::Named("SampleEvent".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn enum_variant_payload_bytes_file_path_requires_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"event": {"Uploaded": {"file": "documents/sample.bin"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    #[test]
    fn enum_variant_payload_inline_bytes_do_not_require_test_document_working_directory() {
        let fixture = Fixture {
            input: serde_json::json!({"event": {"Uploaded": {"file": "inline text"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    #[test]
    fn enum_variant_mismatched_tag_key_does_not_require_test_document_working_directory() {
        // Control: the JSON payload names a variant that does not exist on the enum, so
        // no variant's wire key resolves and no field should be considered reachable. ~keep
        let fixture = Fixture {
            input: serde_json::json!({"event": {"SomethingElse": {"file": "documents/sample.bin"}}}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_event_type()],
            &[event_enum()]
        ));
    }

    fn flattened_details_type() -> TypeDef {
        TypeDef {
            name: "SampleDetails".into(),
            fields: vec![FieldDef {
                name: "attachment".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn request_with_flattened_details() -> TypeDef {
        TypeDef {
            name: "SampleRequest".into(),
            fields: vec![FieldDef {
                name: "details".into(),
                ty: TypeRef::Named("SampleDetails".into()),
                serde_flatten: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn flattened_named_field_bytes_file_path_requires_test_document_working_directory() {
        // `details` is flattened, so its `attachment` field appears at the SAME level as
        // the parent object, not nested under a `"details"` key. ~keep
        let fixture = Fixture {
            input: serde_json::json!({"attachment": "documents/sample.bin"}),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[request_with_flattened_details(), flattened_details_type()],
            &[]
        ));
    }
}

/// Coverage for the `docs.presentation.files` detection path, distinct from every other test in
/// this module: those all author a `bytes`-typed field's fixture VALUE as the relative path
/// string itself (`TypeRef::Bytes => value.as_str().is_some_and(is_relative_document_path)`).
/// A fixture is equally free to write the real byte literals inline and record the file
/// separately for docs purposes (`docs.presentation.files: [{field, path}]`) — the shape
/// `fixtures/batch/bytes_happy.json` actually uses in a real downstream crate, with the doc file
/// entry nested under an ARRAY argument element (`"/inputs/1/bytes"`). `docs_files_for_arg`'s
/// prefix-match against that argument's `field` (`"input.inputs"` -> base `"/inputs"`) is what
/// `scan_fixture`'s early-return check consults BEFORE ever walking the argument's element
/// type, so this must resolve correctly independent of `ExtractInput`'s own struct/enum shape.
/// ~keep
#[cfg(test)]
mod docs_presentation_array_scan_tests {
    use crate::core::config::e2e::{ArgMapping, CallConfig};
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};
    use crate::e2e::fixture::Fixture;

    fn batch_arg() -> ArgMapping {
        ArgMapping {
            name: "inputs".into(),
            field: "input.inputs".into(),
            arg_type: "json_object".into(),
            optional: false,
            owned: true,
            element_type: Some("ExtractInput".into()),
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn extract_input_type() -> TypeDef {
        TypeDef {
            name: "ExtractInput".into(),
            fields: vec![FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn docs_presentation_file_nested_in_a_batch_array_element_requires_test_document_working_directory() {
        let fixture_json = serde_json::json!({
            "id": "extract_batch_bytes_happy",
            "input": {
                "inputs": [
                    {"kind": "bytes", "bytes": [72, 101], "mime_type": "text/plain"},
                    {"kind": "bytes", "bytes": [60, 104, 116], "mime_type": "text/html"}
                ]
            },
            "docs": {
                "topic": "batch",
                "presentation": {
                    "files": [{"field": "/inputs/1/bytes", "path": "html/html.html"}]
                }
            }
        });
        let fixture: Fixture = serde_json::from_value(fixture_json).expect("parse fixture");
        assert_eq!(
            fixture.docs_files_for_arg("input.inputs"),
            vec![crate::e2e::fixture::FixtureDocsFileInput {
                field: "/1/bytes".to_string(),
                path: "html/html.html".to_string(),
            }],
            "the nested doc file entry must resolve relative to the argument's own field prefix"
        );
        let call = CallConfig {
            args: vec![batch_arg()],
            ..Default::default()
        };
        assert!(
            super::fixture_uses_test_documents(&fixture, &call, &[extract_input_type()], &[]),
            "a docs.presentation.files entry nested under an array element must still require \
             the test-documents working directory"
        );
    }

    /// Control: no `docs.presentation.files` entry at all, and the inline byte literals carry
    /// no relative path string for `is_relative_document_path` to match either -- must NOT
    /// require the test-documents working directory.
    #[test]
    fn inline_bytes_with_no_docs_presentation_file_does_not_require_test_document_working_directory() {
        let fixture_json = serde_json::json!({
            "id": "extract_batch_bytes_inline_only",
            "input": {
                "inputs": [{"kind": "bytes", "bytes": [72, 101], "mime_type": "text/plain"}]
            }
        });
        let fixture: Fixture = serde_json::from_value(fixture_json).expect("parse fixture");
        let call = CallConfig {
            args: vec![batch_arg()],
            ..Default::default()
        };
        assert!(!super::fixture_uses_test_documents(
            &fixture,
            &call,
            &[extract_input_type()],
            &[]
        ));
    }
}
