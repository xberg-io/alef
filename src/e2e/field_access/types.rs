use std::collections::{HashMap, HashSet};

/// Resolves fixture field paths to language-specific accessor expressions.
#[derive(Clone)]
pub struct FieldResolver {
    pub(super) aliases: HashMap<String, String>,
    pub(super) optional_fields: HashSet<String>,
    /// The subset of `optional_fields` the consumer's own `[e2e].fields_optional` list actually
    /// named, fixed at construction time and never touched by `with_ir_fields`'s merge.
    ///
    /// `optional_fields` answers "does this classify as optional", and has to include names
    /// `with_ir_fields` merges in from the IR's own `Option<T>` declarations so an unconfigured
    /// field still guards correctly — that merge is deliberate (see `with_ir_fields`'s doc). But
    /// it means `optional_fields` can no longer answer "did the CONSUMER write this down", and a
    /// diagnostic that named `fields_optional` for an IR-only name told a consumer to delete a
    /// config entry that was never there. Kept as a separate, unmerged set so
    /// `FieldResolver::declaring_config_key` can ask the provenance question instead of reusing
    /// the classification answer. ~keep
    pub(super) config_declared_optional_fields: HashSet<String>,
    pub(super) result_fields: HashSet<String>,
    pub(super) array_fields: HashSet<String>,
    pub(super) enum_fields: HashSet<String>,
    pub(super) method_calls: HashSet<String>,
    /// Fields whose `Option<T>` inner type is a display/content union (e.g. `RichTextContent`)
    /// rather than a plain `String`. Language generators that would otherwise emit
    /// `string(*ptr)` (Go) or `Objects::toString()` (Java) for such fields will instead
    /// call the language-idiomatic text accessor (`.Text()` in Go/Java/C#, `.text()` in PHP)
    /// so the assertion compares the textual representation, not an opaque object address.
    ///
    /// Populated from `fields_display_as_text` in `alef.toml`.
    pub(super) display_as_text_fields: HashSet<String>,
    /// Aliases for error-path field access (used when assertion_type == "error").
    /// Maps fixture sub-field names (the part after "error.") to actual field names
    /// on the error type. E.g., `"status_code" -> "status_code"`.
    pub(super) error_field_aliases: HashMap<String, String>,
    /// Per-type PHP getter classification: maps an owner type's snake_case field
    /// name to whether THAT field on THAT type requires `->getCamelCase()` syntax
    /// (because the field's mapped PHP type is non-scalar and ext-php-rs emits a
    /// `#[php(getter)]` method) rather than `->camelCase` property access.
    /// Populated by `new_with_php_getters`; empty by default.
    ///
    /// Keying by (type, field) — not bare field name — is required because two
    /// different types can declare the same field name with different scalarness
    /// (e.g. `CrawlConfig.content: ContentConfig` is non-scalar while
    /// `MarkdownResult.content: String` is scalar).
    pub(super) php_getter_map: PhpGetterMap,
    /// How this language's binding exposes a tagged-union variant's payload, keyed by
    /// `(union type, variant identifier)`. Populated by the C# and Dart e2e codegen via
    /// `with_variant_accessors`; empty for every other resolver, which restores the exact
    /// pre-existing rendering.
    ///
    /// ~keep A field path that steps into a variant payload (`metadata.format.html.title`) is a
    /// narrowing, not a field read, and only some bindings can spell it as a plain chain. The
    /// renderers used to have no way to tell the two apart, so C# emitted `.Format!.Html!` --
    /// naming the variant TYPE where a property was required (CS0572) -- and Dart emitted
    /// `.format?.html?` against a sealed class that has no such getter. Both compiled nowhere.
    /// The decision that a segment crosses a variant is the resolver's; how it reads is the
    /// renderer's, which is why this carries the binding's own spelling rather than a rendered
    /// expression.
    pub(super) variant_accessors: VariantAccessorMap,
    /// Per-type Swift first-class/opaque classification, populated by the
    /// Swift e2e codegen. When non-empty, `accessor` uses
    /// `render_swift_with_first_class_map` instead of the legacy property-only
    /// `render_swift_with_optionals`, so paths that traverse from first-class
    /// types (property access) into opaque typealias types (method-call access)
    /// pick the correct syntax at each segment.
    pub(super) swift_first_class_map: SwiftFirstClassMap,
    /// Per-type Dart stringy field classification, populated by the Dart e2e
    /// codegen. Used to aggregate every readable text accessor on a `Vec<T>`
    /// element type for `contains` assertions.
    pub(super) dart_first_class_map: DartFirstClassMap,
    /// Field names reachable through the emitted binding on at least one IR type in
    /// the crate — i.e. present in `TypeDef.fields` and not `binding_excluded` there
    /// (the exact predicate `crate::codegen::shared::binding_fields` uses to decide
    /// which fields a backend like pyo3 actually attaches `#[pyo3(get)]` to). Populated
    /// via `with_ir_fields`; empty when a codegen call site hasn't wired IR data in, in
    /// which case field-availability checks fall back entirely to the hand-maintained
    /// `result_fields` config.
    pub(super) ir_reachable_fields: HashSet<String>,
    /// Field names that exist on some IR type but are `binding_excluded` there (no
    /// accessor is emitted for them in any generated binding), and are NOT reachable
    /// on any other type. Lets `is_valid_for_result` tell "this is a real struct field
    /// the IR knows is unexposed" apart from "the IR has never heard of this name"
    /// (e.g. a virtual namespace prefix or a synthetic/derived assertion field), so
    /// `result_fields` only gets the final say on names the IR is silent on.
    pub(super) ir_known_excluded_fields: HashSet<String>,
    /// Bare field names that carry `#[serde(skip_serializing_if = "...")]` on at least one IR
    /// type, i.e. fields whose JSON key may be entirely absent from the wire format even
    /// though the underlying Rust value is never itself missing (a required `Vec<T>` skipped
    /// via `Vec::is_empty`, an `Option<T>` skipped via `Option::is_none`, ...).
    ///
    /// Deliberately kept separate from `optional_fields`: that set drives
    /// `Option<T>`-shaped codegen (`.as_ref().unwrap()`, `!`, nullability) for backends that
    /// access the real typed value, where a required `Vec<T>` must NOT be treated as
    /// `Option`-like. This set exists only for backends that walk a generic parsed-JSON tree
    /// (currently the Zig e2e generator, which re-parses the FFI's serialized JSON for result
    /// shapes it has no typed accessor for) and must guard a `.get(key)` lookup instead of
    /// assuming the key exists. Populated via `with_wire_optional_fields`; empty when a codegen
    /// call site hasn't wired IR data in.
    pub(super) wire_optional_fields: HashSet<String>,
    /// IR-derived enum-field classification (`crate_type -> field -> {is-enum, next type}`),
    /// anchored at the call's declared result type. Populated via `with_ir_enum_map`; empty
    /// when a codegen call site hasn't wired IR data in, in which case `is_enum` falls back
    /// entirely to the hand-maintained `fields_enum` config. See [`IrEnumMap`] for why this
    /// is keyed by `(type, field)` rather than by bare field name.
    pub(super) ir_enum_map: IrEnumMap,
    /// Enum names whose serde wire representation has no discriminator at all.
    pub(super) wasm_untagged_enum_names: HashSet<String>,
    /// Names of IR enum types for which the Java binding backend does NOT emit a plain
    /// `enum` with a `getValue()` accessor — i.e. `!backends::java::gen_bindings::emits_get_value`
    /// (tagged- or untagged-union wrapper classes). Populated once per crate IR, from the exact
    /// predicate the Java binding backend itself uses, via `with_java_wrapper_enum_names`.
    /// Empty for every non-Java resolver and for any Java resolver built before that IR data
    /// was wired in, in which case `java_enum_emits_get_value` answers `None` (unknown) rather
    /// than assuming either shape.
    pub(super) java_wrapper_enum_names: HashSet<String>,
    /// IR enum type name -> the JavaScript discriminant property napi puts on the wire, for
    /// every enum `backends::napi` lowers to an internally-tagged `#[napi(object)]` struct
    /// rather than a `#[napi(string_enum)]`. Populated from that backend's own two authorities —
    /// `is_tagged_data_enum` (documented there as "the single authority for that verdict") and
    /// `tagged_enum_discriminant_js_name` — via `with_napi_tagged_object_enums`, so the assertion
    /// this drives cannot disagree with the emitted struct or its `.d.ts` about the shape.
    ///
    /// ~keep Absent this, node assertions on an enum-typed field compared the whole object as a
    /// scalar: `String(e.kind).includes("Function")` against `{ type: "Function" }` stringifies to
    /// `"[object Object]"` and can never match, which is what kept a real downstream crate's
    /// Node e2e gate red and its npm publish permanently blocked. Empty for every non-node
    /// resolver, in which case `napi_tagged_object_discriminant` answers `None` and the previous
    /// scalar comparison stands.
    pub(super) napi_tagged_object_enums: HashMap<String, String>,
    /// Names of IR enum types `backends::magnus` (Ruby) lowers to a plain Ruby `Hash` via
    /// `serde_json::to_value` inside `IntoValue`, rather than a `Symbol` — i.e.
    /// `backends::magnus::gen_bindings::classes::gen_enum::gen_enum`'s own `has_data` predicate
    /// (`enum_def.variants.iter().any(|v| !v.fields.is_empty())`). Populated once per crate IR,
    /// from `e2e::codegen::ruby::enum_variant_access::hash_serialized_enum_names`, via
    /// `with_ruby_hash_serialized_enum_names`. Empty for every non-Ruby resolver and for any Ruby
    /// resolver built before that IR data was wired in, in which case
    /// `ruby_enum_serialized_as_hash` answers `None` for every field — never a false skip.
    pub(super) ruby_hash_serialized_enum_names: HashSet<String>,
    /// IR-derived collection-field classification (`crate_type -> field -> {is-Vec, next
    /// type}`), anchored at the call's declared result type. Populated via
    /// `with_ir_collection_map`; empty when a codegen call site hasn't wired IR data in, in
    /// which case `is_collection_root` falls back entirely to the hand-maintained
    /// `fields_array`/`fields_optional` config. Mirrors `ir_enum_map`'s precedence exactly —
    /// see [`IrCollectionMap`] for why this is keyed by `(type, field)` rather than by bare
    /// field name.
    pub(super) ir_collection_map: IrCollectionMap,
    /// Resolver-private element-shape facts. Kept out of public [`IrCollectionMap`] so existing
    /// callers can continue constructing that map exhaustively. ~keep
    pub(super) non_string_scalar_collection_fields: HashMap<String, HashSet<String>>,
    /// IR-derived field facts about the call's OWN declared result type, anchored at that type
    /// rather than keyed by bare field name. Populated via `with_ir_result_fields`; empty
    /// (`root_type: None`) whenever a codegen call site could not resolve the call's return
    /// type, in which case every anchored answer is skipped and the flat, name-keyed
    /// `optional_fields`/`ir_reachable_fields` behaviour stands unchanged. See
    /// [`IrResultFieldMap`].
    pub(super) ir_result_field_map: IrResultFieldMap,
    /// Whether the call's own declared Rust return type resolves to a raw byte payload
    /// (`bytes::Bytes`, `Vec<u8>`, `[u8]`, `[u8; N]` — all collapsed to `TypeRef::Bytes` by
    /// `extract::type_resolver`) rather than a struct. Populated via
    /// `with_result_is_byte_payload`; `false` by default, matching every resolver built before
    /// this flag existed.
    ///
    /// ~keep This is the ONE place every backend's `is_valid_for_result` /
    /// `result_field_oracle_knows` call consults for "does a field path even make sense here" —
    /// a byte payload has no fields at all, so ANY non-empty path is unconditionally rejected the
    /// same way regardless of which backend asks. Before this flag existed the two anchored
    /// oracles (`ir_result_field_map.root_type`, `ir_collection_map.root_type`) were `None` for a
    /// byte-returning call exactly as they are for a call with no IR wired in at all —
    /// `resolve_declared_result_type`'s `named_type` helper has no `Named` leaf to report for
    /// `TypeRef::Bytes` — so the permissive "IR has never heard of this name" default silently
    /// accepted a fixture's declared struct field path against a value that is not a struct.
    /// Some backends independently learned to check the config-level `result_is_bytes` flag
    /// before this existed (java/csharp/c/zig/swift/r); others (rust, go) checked only
    /// `result_is_simple` and missed the byte-payload case entirely — two components reading the
    /// same fact and disagreeing. Anchoring the answer here, once, is what a byte-returning call
    /// site now has to opt into by passing this flag at construction rather than reimplementing
    /// its own check.
    pub(super) result_is_byte_payload: bool,
    /// Per-type Python `TypedDict`-vs-attribute-access classification, populated by the Python
    /// e2e codegen. When empty, [`PythonTypedDictMap::is_typeddict`] answers `false` for every
    /// type, which is exactly the pre-existing behaviour (attribute access everywhere) for any
    /// resolver built before this map existed.
    pub(super) python_typeddict_map: PythonTypedDictMap,
    /// IR-derived owner transitions for one key access through a map-typed Python field.
    ///
    /// ~keep This stays separate from [`PythonTypedDictMap::field_types`]: that map is public API
    /// and represents ordinary field hops, while these edges describe a different operation and
    /// must not become observable through the pre-0.79.3 public map shape. Only the Python
    /// accessor renderer consumes this resolver-private metadata.
    pub(super) python_map_value_edges: PythonMapValueEdges,
}

/// IR field facts keyed by owner type and anchored at the type a specific call returns, so a
/// field name that means different things on different structs is never conflated.
///
/// * `field_types[type_name][field_name]` — the named type `field_name` traverses into, when
///   that type is another struct the path can keep walking through.
/// * `optional_fields[type_name]` — fields of `type_name` the *generated binding* declares as
///   possibly-absent, per the [`crate::e2e::field_access::OptionalityRule`] the map was built
///   with. Not the same question as "is the Rust field `Option<T>`": see that enum.
/// * `pointer_fields[type_name]` — fields whose authoritative Go declaration starts with `*`.
///   This stays separate from optionality because slices and interfaces are nullable values,
///   while an unresolved required named field is emitted as `*json.RawMessage`.
/// * `declared_fields[type_name]` — every binding-visible field of `type_name`, i.e. the members
///   a generated accessor may legally name.
/// * `unresolvable_named_fields[type_name]` — declared fields whose type names ANOTHER user type
///   (resolves via [`crate::e2e::codegen::call_ir::named_type`]) that is not itself a struct in
///   `field_types` — a tagged union, most commonly. Distinct from a field simply absent from
///   `field_types` because its type is a scalar, `serde_json::Value`, or another opaque/foreign
///   type nobody extracted: THOSE fields never resolve to a `Named` type at all, so a path
///   stepping past them stays unjudgeable on purpose (map values and JSON blobs are legitimately
///   walkable further, just not through this map). Only a field that positively names another
///   user type the IR declined to treat as a struct member of belongs here.
/// * `display_safe_fields[type_name]` — fields of `type_name` whose declared Rust type is a bare
///   `String`, `char`, or numeric/`bool` primitive — an ALLOWLIST of the only shapes alef can
///   positively confirm implement `Display`, per
///   [`ir_result_fields::type_ref_is_display_safe`](super::super::ir_result_fields::type_ref_is_display_safe).
///   Absence means either a genuinely `Display`-unsafe declared type (a collection, `Option<_>`,
///   a `Named` struct/enum, or any other wrapped/opaque shape) or simply no evidence — the two are
///   deliberately not distinguished, because a per-item field formatter that guesses "safe" wrong
///   is a snippet that fails to compile.
/// * `root_type` — the IR type name the call's declared return type resolves to, via
///   `codegen::call_ir::resolve_declared_result_type`. `None` disables every anchored answer.
/// * `map_scalar_value_fields[type_name]` — fields of `type_name` whose declared type is a
///   `Map<K, V>` (optionally wrapped in `Option<..>`) where `V` is a plain, never-nil Go value
///   kind: a resolved struct, a resolved non-sealed enum, or a bare scalar (`string`,
///   `bool`, a numeric primitive, `Duration`). Indexing such a map (`m["key"]`) always yields a
///   concrete Go value, even when the map itself is absent or the key is missing — a nil Go map
///   read is a safe zero-value read, never a panic — so a leaf reached through this field must
///   never be treated as nilable, regardless of what `optional_fields`/`pointer_fields` say about
///   the map field itself. Excludes `Optional<V>`, `Vec<V>`/`Bytes`/`Json` (slice-backed), a
///   nested `Map<_, _>`, a sealed-interface (data enum) `V`, and any unresolved `Named` `V` —
///   all of those render as a Go pointer, slice, map, or `interface{}`, which genuinely can be
///   `nil` when read.
#[derive(Debug, Clone, Default)]
pub struct IrResultFieldMap {
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub optional_fields: HashMap<String, HashSet<String>>,
    pub pointer_fields: HashMap<String, HashSet<String>>,
    pub data_interface_fields: HashMap<String, HashSet<String>>,
    pub declared_fields: HashMap<String, HashSet<String>>,
    pub unresolvable_named_fields: HashMap<String, HashSet<String>>,
    pub display_safe_fields: HashMap<String, HashSet<String>>,
    pub map_scalar_value_fields: HashMap<String, HashSet<String>>,
    pub root_type: Option<String>,
}

/// IR-derived collection-field classification, keyed by owner type so a field named `items`
/// that is `Vec<T>` on one struct and `String` on another is never conflated. Mirrors
/// [`IrEnumMap`]'s shape exactly, but answers "is this field's declared type a `Vec`?" instead
/// of "is this field's declared type a real IR enum?".
///
/// A bare collection field (e.g. a recursive `List<DataNode> Children` with no per-element path
/// like `children[0]...` declared in `fields_array`/`fields_optional`) has no config signal at
/// all: `FieldResolver::is_array`/`is_collection_root` answer `false` for every field the
/// operator's `alef.toml` never mentioned, so backends checking `is_array(f) ||
/// is_collection_root(f)` before deciding to serialize a collection for `is_empty`/`contains`
/// fall through to `ToString()`/similar on the raw collection object — which returns the type
/// name, not the contents, so the assertion can never pass. This map lets `is_collection_root`
/// answer from the IR itself when config is silent, exactly as `ir_enum_map` already does for
/// `is_enum`.
///
/// * `field_types[type_name][field_name]` — the IR-resolved named type that `field_name`
///   traverses into, when that type is another struct the path can keep walking through.
/// * `collection_fields[type_name]` — field names on `type_name` whose declared type (after
///   unwrapping `Option`) is `Vec<T>`.
/// * `root_type` — the IR type name backing the call's result variable, resolved the same way
///   `IrEnumMap::root_type` is.
#[derive(Debug, Clone, Default)]
pub struct IrCollectionMap {
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub collection_fields: HashMap<String, HashSet<String>>,
    pub root_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmEnumRepresentation<'a> {
    External,
    Tagged { tag: &'a str },
    Untagged,
}

/// IR-derived enum-field classification, keyed by owner type so a field named `kind` that is
/// `String` on one struct and an enum on another is never conflated. Mirrors `PhpGetterMap`
/// and `SwiftFirstClassMap`'s per-type shape.
///
/// * `field_types[type_name][field_name]` — the IR-resolved named type (after unwrapping
///   `Option`/`Vec`; `Box<T>` fields already carry the unboxed named type in the IR, so no
///   separate unwrap is needed for them) that `field_name` traverses into, when that type is
///   another struct the path can keep walking through. Used to advance the type cursor one
///   path segment at a time, e.g. `choices[0].finish_reason` walks `choices` into its element
///   type before checking `finish_reason` there.
/// * `enum_fields[type_name]` — field names on `type_name` whose declared type (after the
///   same unwrapping) names a real `EnumDef` this crate declares.
/// * `root_type` — the IR type name backing the call's result variable, resolved from the
///   crate's own function/method signatures (not a hand-configured override). `None` when the
///   call's declared return type could not be resolved, in which case IR-derived enum
///   classification answers `false` for every path (the same safe default an unconfigured
///   `fields_enum` entry already has).
#[derive(Debug, Clone, Default)]
pub struct IrEnumMap {
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub enum_fields: HashMap<String, HashSet<String>>,
    /// `enum_field_types[type_name][field_name]` — the IR enum's own name for an entry also
    /// present in `enum_fields`. Lets a caller resolve WHICH enum a positively-classified
    /// field is, not just that it is one — e.g. so Java e2e codegen can check whether that
    /// specific enum is a plain `enum` (`getValue()` accessor) or a tagged/untagged-union
    /// wrapper class, per `backends::java::gen_bindings::emits_get_value`.
    pub enum_field_types: HashMap<String, HashMap<String, String>>,
    /// `variant_payload_types[enum_name][variant_name] -> (raw_field_name, payload_type_name)`
    /// for a tagged-union variant that carries exactly one field (`Variant(Payload)` or
    /// `Variant { field: Payload }`). Lets a caller resolve the concrete type a variant wraps
    /// once a path has been split at the union boundary (see
    /// `FieldResolver::tagged_union_split`), so it can keep walking the path's suffix through
    /// that payload type's own fields instead of stopping at the variant name. Multi-field
    /// variants are deliberately not recorded here: there is no single payload type to advance
    /// into, and a caller must fall back to its own "not implemented" handling for that shape.
    pub variant_payload_types: HashMap<String, HashMap<String, (String, String)>>,
    /// `variant_payload_is_collection[enum_name]` — variant names (a subset of
    /// `variant_payload_types[enum_name]`'s keys) whose single payload field's *declared* type is
    /// itself `Vec<T>` (`Variant(Vec<Item>)`), rather than a struct that merely wraps one
    /// (`Variant(Payload)`). `variant_payload_types` unwraps `Vec` the same way it unwraps
    /// `Option` when resolving the payload's named type, so it alone cannot tell those two shapes
    /// apart; this set is the shape distinction a caller needs when a fixture path names only the
    /// variant with no field inside it — asserting a collection check directly against the
    /// payload value only makes sense for the first shape. ~keep
    pub variant_payload_is_collection: HashMap<String, HashSet<String>>,
    /// `tagged_enum_wire[enum_name] -> (serde_tag, Rust variant -> serde wire value)`.
    /// Carries the exact discriminator spellings assertion generators need at runtime.
    pub tagged_enum_wire: HashMap<String, TaggedEnumWire>,
    /// Names of the IR enums that carry data on at least one variant — the exact complement of
    /// the `variants.iter().all(|v| v.fields.is_empty())` gate the Dart, Kotlin and Swift binding
    /// backends each branch on when they decide whether an enum gets a scalar, string-lowerable
    /// representation (`.wireValue` extension / `enum class` with `toWire()` / `: String` raw-value
    /// enum) or a payload-bearing one (freezed sealed union / Kotlin sealed class / Swift enum with
    /// associated values). Only the first shape has the lowering accessor, so an assertion
    /// generator that classifies a field as "enum-typed" and appends that accessor unconditionally
    /// emits an accessor the binding never declared. `enum_field_types` answers WHICH enum backs a
    /// field; this answers whether that enum is one the accessor exists on. ~keep
    pub data_carrying_enum_names: HashSet<String>,
    /// `enum_wire_variants[enum_name][serde wire value] -> Rust variant identifier`, restricted
    /// to variants whose wire value actually DIFFERS from the identifier (i.e. a
    /// `#[serde(rename)]` or `#[serde(rename_all)]` is in effect) and is unambiguous.
    ///
    /// `tagged_enum_wire` cannot answer this: it is populated only for enums carrying a
    /// `#[serde(tag = "...")]` internal tag, and it maps the other direction (variant -> wire).
    /// A generator holding a fixture's expected WIRE value and an expression that renders the
    /// RUST identifier — e.g. Rust e2e's `format!("{:?}", field)` — needs the reverse lookup for
    /// every enum, tagged or not, to compare the two surfaces without mistaking a rename for a
    /// value mismatch. Variants whose wire value equals their identifier are deliberately absent
    /// so a lookup miss means "no rename to reconcile" and the caller keeps its prior behaviour.
    /// ~keep
    pub enum_wire_variants: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TaggedEnumWire {
    pub tag: String,
    pub variants: HashMap<String, String>,
    /// Serde's adjacently-tagged `content` key (`#[serde(tag = "..", content = "..")]`), when the
    /// enum sets one. `None` means the enum is internally tagged (payload fields flattened
    /// beside the tag on the core wire format).
    pub content: Option<String>,
}

/// Per-(union type, variant) narrowing facts, supplied by the binding backend that owns the
/// spelling.
///
/// Empty is meaningful and is the default: it means "this language's renderer has no variant
/// narrowing to apply", which is the behaviour every language had before this existed.
#[derive(Debug, Clone, Default)]
pub struct VariantAccessorMap {
    /// C#: the generated `As<Variant>` property name, read from
    /// `backends::csharp::gen_bindings::variant_accessor_properties` so the resolver can never
    /// name an accessor the generator declined to emit.
    ///
    /// Dart: the flutter_rust_bridge/freezed subclass to cast to (`<Union>_<Variant>`). The
    /// authority here is frb's own naming convention rather than anything alef renders, so the
    /// spelling is supplied by the Dart e2e codegen that already relies on it.
    pub narrowing: HashMap<(String, String), String>,
    /// Dart only: the accessor for the payload inside the narrowed subclass (`field0` for a
    /// single tuple field). Absent for C#, whose `As<Variant>` yields the payload directly.
    pub payload: HashMap<(String, String), String>,
}

impl VariantAccessorMap {
    pub fn is_empty(&self) -> bool {
        self.narrowing.is_empty()
    }

    pub fn narrowing_for(&self, union_type: &str, variant: &str) -> Option<&str> {
        self.narrowing
            .get(&(union_type.to_string(), variant.to_string()))
            .map(String::as_str)
    }

    pub fn payload_for(&self, union_type: &str, variant: &str) -> Option<&str> {
        self.payload
            .get(&(union_type.to_string(), variant.to_string()))
            .map(String::as_str)
    }
}

/// Per-type PHP getter classification + chain-resolution metadata.
///
/// Holds enough information to resolve a multi-segment field path through the
/// IR's nested type graph and pick the correct accessor style at each segment:
///
/// * `getters[type_name]` — set of field names on `type_name` whose PHP binding
///   uses a `#[php(getter)]` method (caller must emit `->getCamelCase()`).
/// * `field_types[type_name][field_name]` — the IR-resolved `Named` type that
///   `field_name` traverses into, used to advance the "current type" cursor
///   for the next path segment. Absent for terminal/scalar fields.
/// * `root_type` — the IR type name backing the result variable at the start of
///   any chain. When `None`, chain traversal degrades to per-segment lookup
///   using a flattened union across all types (legacy bare-name behaviour),
///   which produces false positives when field names collide across types.
#[derive(Debug, Clone, Default)]
pub struct PhpGetterMap {
    pub getters: HashMap<String, HashSet<String>>,
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
    /// All field names per type — used to detect when the recorded `root_type`
    /// is a misclassification (a workspace-global root_type may not match the
    /// actual return type of a per-fixture call). When `owner_type` is set but
    /// `all_fields[owner_type]` doesn't contain `field_name`, the renderer
    /// falls back to the bare-name union instead of trusting the (wrong) owner.
    pub all_fields: HashMap<String, HashSet<String>>,
}

/// Swift first-class struct classification + chain-resolution metadata.
///
/// alef-backend-swift emits two flavors of binding types:
///
/// * **First-class Codable structs** — `public struct Foo: Codable { public let id: String }`.
///   Fields are Swift properties; access with `.id` (no parens).
/// * **Opaque typealiases** — `public typealias Foo = RustBridge.Foo` where the
///   RustBridge class exposes swift-bridge methods. Fields are methods;
///   access with `.id()` (parens).
///
/// The renderer needs per-segment dispatch because a path can traverse both:
/// e.g. `BatchListResponse` (first-class Codable, with `data: [BatchObject]`) →
/// indexed `[0]` → `BatchObject` (opaque typealias). At the `BatchObject` cursor
/// the renderer must switch to method-call access for `.id`, `.status`, etc.
///
/// * `first_class_types` — set of TypeDef names whose binding is a first-class
///   Codable struct. Membership = "use property access for fields on this type".
/// * `field_types[type_name][field_name]` — the IR-resolved `Named` type that
///   `field_name` traverses into.
/// * `vec_field_names` — flat set of field names whose IR type is `Vec<T>` on
///   any owner. Used by swift_count_target to keep `.count` straight on
///   RustVec-typed method-call accessors (don't inject `.toString()`).
/// * `root_type` — the IR type name backing the result variable.
///
/// Kind of a "stringy" field on an opaque DTO element type — used by the swift
/// e2e `contains` assertion to aggregate every readable text accessor on a
/// `Vec<T>` element instead of relying on a single primary accessor (which
/// often guesses wrong: e.g. `ImportInfo.source` is the module path but
/// `ImportInfo.items` carries the imported names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringyFieldKind {
    /// `field_name() -> RustString` (or `String`). Convert via `.toString()`.
    Plain,
    /// `field_name() -> Optional<RustString>`. Convert via `?.toString() ?? ""`.
    Optional,
    /// `field_name() -> RustVec<RustString>`. Iterate elements (RustStringRef
    /// → `.asStr().toString()` on each).
    Vec,
}

/// A single readable text accessor on an opaque DTO. The `name` is the Rust
/// field name (snake_case), used to derive the swift-bridge lowerCamelCase
/// method call.
#[derive(Debug, Clone)]
pub struct StringyField {
    pub name: String,
    pub kind: StringyFieldKind,
}

/// One step of navigating JSON already decoded from a swift-bridge JSON-bridged leaf's
/// `RustString`, via `JSONSerialization`.
///
/// ~keep Produced by `FieldResolver::swift_json_bridged_navigation`, which finds the JSON-bridged
/// leaf a fixture path steps past and records exactly how it steps past it, so the swift e2e
/// backend can render a real decode-and-navigate expression instead of refusing the whole
/// assertion. See that method's own doc for the walk and its deliberate limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonNavStep {
    /// A numeric array index, e.g. the `0` in `chunks[0]`.
    Index(usize),
    /// An object key, e.g. `output_format` in `metadata.output_format`.
    Key(String),
}

#[derive(Debug, Clone, Default)]
pub struct SwiftFirstClassMap {
    pub first_class_types: HashSet<String>,
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub vec_field_names: HashSet<String>,
    /// Field names whose swift-bridge getter returns one `RustString` holding the whole field
    /// JSON-encoded, per the binding generator's own `field_needs_json_bridge`.
    ///
    /// ~keep Such a leaf is a scalar string at the Swift surface: it has no `.count` and no
    /// subscript, so a count suffix, an index step and a wildcard step are all equally
    /// unspellable against it. Recorded as its own set rather than as the complement of
    /// `vec_field_names`, because that complement also contains every genuine scalar and every
    /// field the IR never described — those must not be mistaken for a JSON bridge.
    pub json_bridged_field_names: HashSet<String>,
    /// `json_bridged_by_type[type_name][field_name]` — the same fact as
    /// [`Self::json_bridged_field_names`], but keyed by the field's OWNER type.
    ///
    /// ~keep The flat set above is a bare-leaf-name index over every `TypeDef` in the crate, so
    /// one type declaring `items: Option<Vec<T>>` (which swift-bridge JSON-bridges) marks the
    /// name `items` bridged for every other type too — including a type whose `items: Vec<T>` is
    /// a genuine `RustVec`. That is the exact confusion `ir_enum`'s module doc rules out for enum
    /// classification, and it silently downgrades a real collection's emptiness assertion. This
    /// map lets a caller that can anchor a path walk to the leaf's real owner and get a
    /// type-specific answer; absence of an entry means the IR never described that field on that
    /// type, which must not be read as "not bridged".
    ///
    /// Keyed field-by-field (like [`Self::getter_optionality`]) rather than as a per-type set of
    /// bridged names, so *presence* of an entry is the separate fact from the boolean: a type
    /// whose fields are all plain would otherwise be indistinguishable from a type the scan never
    /// saw, and every caller would fall back to the poisoned flat set for exactly the types the
    /// map can answer for. ~keep
    pub json_bridged_by_type: HashMap<String, HashMap<String, bool>>,
    /// `getter_optionality[type_name][field_name]` — whether that field's swift-bridge getter on
    /// that type returns `Option<..>` rather than a bare value.
    ///
    /// ~keep Keyed by owner type, and *presence* is the separate fact from the boolean: an absent
    /// entry means the IR never described the field, which must not be read as "not optional".
    /// Recorded because `render_swift_with_first_class_map` deliberately never emits a `?` on the
    /// leaf segment — it cannot know what the caller will chain onto the accessor — so the caller
    /// appending `.toString()` is the one that has to know whether the leaf is already
    /// `Optional<RustString>`. Config-derived `optional_fields` cannot answer it: that set is
    /// keyed by bare path and drives `Option`-shaped codegen generally, whereas this is the
    /// narrower question of what the getter's declared return type is.
    pub getter_optionality: HashMap<String, HashMap<String, bool>>,
    pub root_type: Option<String>,
    /// Per-type readable text accessors. Keyed by IR TypeDef name. Used by the
    /// swift e2e `contains` assertion to aggregate every stringy field on a
    /// `Vec<T>` element type into a `contains(where: { ... })` closure that
    /// does substring matching against every text-bearing accessor. Mirrors
    /// python's `_alef_e2e_item_texts` helper.
    pub stringy_fields_by_type: HashMap<String, Vec<StringyField>>,
}

impl SwiftFirstClassMap {
    /// Returns true when fields on `type_name` should be accessed as properties
    /// (no parens), false when they should be accessed via method-call.
    ///
    /// When `type_name` is `None` the renderer defaults to method-call syntax —
    /// opaque swift-bridge types (with `.field()` methods) are the common case
    /// for unknown roots. Defaulting to `true` (property syntax) caused the
    /// e2e generator to emit `result.content` instead of `result.content()` for
    /// opaque `ExtractionResult` and similar types whose IR root type was not
    /// resolved by `swift_call_result_type`, producing a Swift compile error:
    /// "value of type '@Sendable () -> RustString' has no member 'contains'".
    pub fn is_first_class(&self, type_name: Option<&str>) -> bool {
        match type_name {
            Some(t) => self.first_class_types.contains(t),
            None => false,
        }
    }

    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// True when `field_name` appears as a `Vec<T>` (or `Option<Vec<T>>`) on
    /// any IR type. swift codegen consults this when deciding whether `.count`
    /// on a method-call accessor needs `.toString()` injected: RustVec already
    /// supports `.count` directly; RustString does not.
    pub fn is_vec_field_name(&self, field_name: &str) -> bool {
        self.vec_field_names.contains(field_name)
    }

    /// True when `field_name`'s swift-bridge getter collapses the field to a single
    /// JSON-encoded `RustString` on some IR type.
    pub fn is_json_bridged_field_name(&self, field_name: &str) -> bool {
        self.json_bridged_field_names.contains(field_name)
    }

    /// Whether `field_name`'s getter on `type_name` returns `Option<..>`. `None` when the IR
    /// never described that field on that type.
    pub fn getter_is_optional(&self, type_name: &str, field_name: &str) -> Option<bool> {
        self.getter_optionality.get(type_name)?.get(field_name).copied()
    }

    /// Whether `field_name`'s getter on `type_name` is JSON-bridged to one `RustString`. `None`
    /// when the IR never described that field on that type — see [`Self::json_bridged_by_type`]
    /// for why silence must not be read as either answer.
    pub fn json_bridged_getter(&self, type_name: &str, field_name: &str) -> Option<bool> {
        self.json_bridged_by_type.get(type_name)?.get(field_name).copied()
    }

    /// True when no per-type information is recorded.
    pub fn is_empty(&self) -> bool {
        self.first_class_types.is_empty() && self.field_types.is_empty()
    }

    /// Returns the list of stringy accessors recorded for `type_name`, or
    /// `None` if the type has no recorded stringy fields.
    pub fn stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.stringy_fields_by_type.get(type_name).map(Vec::as_slice)
    }
}

/// Python `TypedDict`-vs-attribute-access classification + chain-resolution metadata.
///
/// The pyo3 backend emits two flavors of return-type public spelling (see
/// `crate::backends::pyo3::gen_bindings::errors::is_dataclass_backed_config`):
///
/// * **`TypedDict`** — `class Foo(TypedDict, total=False): bar: int`. At runtime this is a plain
///   `dict`, so fields are accessed with `result["bar"]`.
/// * Every other shape (`@dataclass`, `pydantic.BaseModel`, `msgspec.Struct`, or the compiled
///   native `#[pyclass]`) — fields are accessed with `result.bar`.
///
/// The renderer needs per-segment dispatch because a path can traverse both: a `TypedDict`
/// return type can nest a field whose own type stays a native `#[pyclass]` (e.g. that nested
/// type is not itself `is_return_type`), and at that point subsequent segments must switch back
/// to `.field` access — mirroring `SwiftFirstClassMap`'s first-class/opaque dispatch exactly,
/// one level of indirection removed (TypedDict/attribute instead of property/method-call).
///
/// * `typeddict_types` — set of TypeDef names whose binding is a `TypedDict`. Membership = "use
///   subscript access for fields on this type".
/// * `field_types[type_name][field_name]` — the IR-resolved `Named` type that `field_name`
///   traverses into (seeing through `Option`/`Vec`, matching `ir_enum`/`ir_collection`).
/// * `root_type` — the IR type name backing the result variable.
#[derive(Debug, Clone, Default)]
pub struct PythonTypedDictMap {
    pub typeddict_types: HashSet<String>,
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
}

impl PythonTypedDictMap {
    /// True when fields on `type_name` should be accessed via subscript (`result["field"]`)
    /// rather than attribute (`result.field`). `None` (unknown owner, e.g. the root type could
    /// not be resolved, or a path segment advanced past a type the IR never described) defaults
    /// to `false` — attribute access, the pre-existing behaviour for every path before this map
    /// existed, and still correct for the common case (dataclass / native pyclass).
    pub fn is_typeddict(&self, type_name: Option<&str>) -> bool {
        match type_name {
            Some(t) => self.typeddict_types.contains(t),
            None => false,
        }
    }

    /// Returns the IR `Named` type that `field_name` traverses into for the next chain segment,
    /// or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// True when no per-type information is recorded.
    pub fn is_empty(&self) -> bool {
        self.typeddict_types.is_empty() && self.field_types.is_empty()
    }
}

pub(crate) type PythonMapValueEdges = HashMap<String, HashMap<String, String>>;

/// Internal, reusable Python accessor facts built once from a crate's IR and anchored per call.
/// Its fields are deliberately unavailable outside Alef even though [`PythonTypedDictMap`] stays
/// publicly constructible for source compatibility.
#[derive(Clone, Default)]
pub(crate) struct PythonTypedDictFacts {
    pub(super) typeddict_map: PythonTypedDictMap,
    pub(super) map_value_edges: PythonMapValueEdges,
}

/// Dart opaque type classification + chain-resolution metadata, mirroring
/// Swift's needs to track stringy field accessors on element types for
/// `Vec<T>` contains assertions. Unlike Swift, Dart doesn't distinguish
/// first-class vs opaque; we just track stringy fields per type.
#[derive(Debug, Clone, Default)]
pub struct DartFirstClassMap {
    pub field_types: HashMap<String, HashMap<String, String>>,
    pub root_type: Option<String>,
    /// Per-type readable text accessors. Used by the dart e2e `contains`
    /// assertion to aggregate every stringy field on a `Vec<T>` element type.
    pub stringy_fields_by_type: HashMap<String, Vec<StringyField>>,
}

impl DartFirstClassMap {
    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// Returns the list of stringy accessors recorded for `type_name`, or
    /// `None` if the type has no recorded stringy fields.
    pub fn stringy_fields(&self, type_name: &str) -> Option<&[StringyField]> {
        self.stringy_fields_by_type.get(type_name).map(Vec::as_slice)
    }

    /// True when no per-type information is recorded.
    pub fn is_empty(&self) -> bool {
        self.field_types.is_empty() && self.stringy_fields_by_type.is_empty()
    }
}

impl PhpGetterMap {
    /// Returns true if `(owner_type, field_name)` requires getter-method syntax.
    ///
    /// When `owner_type` is `None` (root type unknown, or chain advanced into an
    /// unmapped type), falls back to the union across all types: any type
    /// declaring `field_name` as non-scalar marks it as needing a getter. This
    /// is the legacy behaviour and is unsafe when field names collide.
    pub fn needs_getter(&self, owner_type: Option<&str>, field_name: &str) -> bool {
        if let Some(t) = owner_type {
            // Only trust the owner-type classification if the type actually declares
            // this field. A misclassified root_type (workspace-global guess that
            // doesn't match the per-fixture call's actual return type) shouldn't
            // shadow the bare-name fallback.
            let owner_has_field = self.all_fields.get(t).is_some_and(|s| s.contains(field_name));
            if owner_has_field {
                // The owner declares this field — the per-type `getters` map is
                // the authoritative answer. Returning early here prevents the
                // global bare-name union (below) from flipping a scalar field
                // (e.g. `ProcessingResult.content: String`) into a getter call
                // just because some unrelated type declares a same-named field
                // as non-scalar (e.g. `Chunk.content: Vec<Span>`).
                return self.getters.get(t).is_some_and(|fields| fields.contains(field_name));
            }
        }
        self.getters.values().any(|set| set.contains(field_name))
    }

    /// Returns the IR `Named` type that `field_name` traverses into for the
    /// next chain segment, or `None` if the field is terminal/scalar/unknown.
    pub fn advance(&self, owner_type: Option<&str>, field_name: &str) -> Option<String> {
        let owner = owner_type?;
        self.field_types.get(owner).and_then(|m| m.get(field_name).cloned())
    }

    /// True when no per-type information is recorded — equivalent to the legacy
    /// "no PHP getter resolution" code path.
    pub fn is_empty(&self) -> bool {
        self.getters.is_empty()
    }
}

/// A parsed segment of a field path.
#[derive(Debug, Clone)]
pub(super) enum PathSegment {
    /// Struct field access: `foo`
    Field(String),
    /// Array field access with explicit numeric index: `foo[N]`
    ///
    /// The `index` is the integer parsed from the bracket (e.g. `choices[2]` → index 2).
    /// When synthesised by `inject_array_indexing` the index defaults to `0`.
    ArrayField { name: String, index: usize },
    /// Map/dict key access: `foo[key]`
    MapAccess { field: String, key: String },
    /// Length/count of the preceding collection: `.length`
    Length,
}
