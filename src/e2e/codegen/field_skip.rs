//! The one funnel for "this assertion's field cannot be asserted here" skip markers.
//!
//! ~keep Every backend used to `writeln!` its own prose for a dropped field assertion and
//! [`super::fail_on_unavailable_field_markers`] used to recognise that prose with two hand-written
//! substring patterns. Backends that invented other wording (dart/swift's tagged-union boundary,
//! ruby's serialized-enum accessor, every `result_is_simple` branch, ...) were therefore emitted
//! but never counted, so arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY` examined a fraction of the
//! skips it appeared to cover — a pass was indistinguishable from health.
//!
//! [`FieldSkip`] closes that by construction: the same per-variant [`Shape`] both renders the
//! human-readable message and recognises it, and `ALL` is generated from the same macro arm as
//! the variant list, so a variant cannot exist without the strict gate counting it. Adding a
//! backend wording means adding a variant here, which automatically extends the gate.
//!
//! Each variant keeps its backend's exact original wording — the reason text is useful to
//! consumers and is deliberately *not* unified. Only the recognition path is shared, and the
//! reason prose sits entirely outside it.
//!
//! The comment syntax and indentation stay at the call site (`// `, `# `, `/* */`), so a rendered
//! line is `<indent><comment-open> skipped: <FieldSkip::message(field)>`.

/// The rendered text on either side of the quoted field name for one registered wording.
struct Shape {
    before: &'static str,
    after: &'static str,
}

/// Why a field assertion was dropped, and therefore whether dropping it is defensible.
///
/// ~keep The axis is *not* "which backend emitted it" but "could an edit to the fixture or the
/// alef config have made this assertion run?".
///
/// [`SkipClass::LanguageLimitation`] names a property of the target language, ABI or declared
/// call shape. No fixture edit makes the assertion expressible, so rendering a comment is the
/// only honest option and the fixture author is not at fault.
///
/// [`SkipClass::AuthoringGap`] names a *lookup that failed*: a field path that did not resolve
/// against a result type or virtual-field set which is itself derived from the same IR the
/// fixture is written against. That is drift between fixture and config, it is fixable, and
/// rendering a green test for it is the defect this module exists to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipClass {
    /// A resolution failure a fixture or config edit can fix. Fatal unless explicitly opted out.
    AuthoringGap,
    /// ~keep alef itself cannot express this assertion yet — a missing generator feature, not a
    /// mistake in the fixture. Never fatal: a consumer cannot fix it from their own `alef.toml`,
    /// so failing their build on it would only force a blanket opt-out and rebuild the silent
    /// skip with extra steps. Counted in its own bucket instead, because this debt is alef's.
    GeneratorGap,
    /// A real property of the language, ABI or declared call shape. Counted, never fatal.
    LanguageLimitation,
}

macro_rules! field_skip_variants {
    ($($(#[$meta:meta])* $variant:ident : $class:ident => ($before:expr, $after:expr $(,)?)),+ $(,)?) => {
        /// A registered reason a field assertion was dropped from generated e2e code.
        ///
        /// Out of scope by design: skips whose cause is the *assertion type* rather than the
        /// field (`unsupported assertion type on synthetic field '<name>'`, `unsupported
        /// traversal assertion ...`, `'<name>' assertion missing value`). Those are a different
        /// defect — a bad assertion shape, not an unreachable field — and must not be conflated
        /// with this one. ~keep
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum FieldSkip {
            $($(#[$meta])* $variant,)+
        }

        impl FieldSkip {
            /// Every variant. Generated from the same macro arm as the variant list so the
            /// recognition set can never fall behind the render set. ~keep
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            const fn shape(self) -> Shape {
                match self {
                    $(Self::$variant => Shape { before: $before, after: $after },)+
                }
            }

            /// Whether dropping this assertion is a defensible language limit or a fixable gap.
            ///
            /// Read from the same macro arm as the wording, so a new variant cannot be added
            /// without its author deciding — in the same edit — which side of the line it sits
            /// on. There is no default. ~keep
            pub(crate) const fn class(self) -> SkipClass {
                match self {
                    $(Self::$variant => SkipClass::$class,)+
                }
            }
        }
    };
}

field_skip_variants! {
    /// ~keep The canonical gap: `FieldResolver::is_valid_for_result` did not find the path. The
    /// resolver is fed from the same IR the binding types are generated from, so a miss means the
    /// fixture names a field that no longer exists (or never did), not that the language is
    /// incapable of asserting it.
    NotAvailableOnResultType: AuthoringGap => ("field ", " not available on result type"),
    /// ~keep zig's JSON-struct return shape, where the result is parsed JSON rather than a binding
    /// type. The struct is generated from the IR, so an unresolved path is drift, not a zig limit —
    /// parsed JSON can express any shape the fixture could name.
    NotAvailableOnJsonStructResult: AuthoringGap => ("field ", " not available on the JSON-struct result"),
    /// ~keep The call declares `result_is_simple`, so the binding returns a bare scalar and a dotted
    /// path has nowhere to live. A property of the declared call shape, not a fixture typo.
    NotAvailableWhenResultIsSimple: LanguageLimitation => ("field ", " not available when result_is_simple"),
    /// ~keep The C ABI flattens to primitives and opaque handles; deeply nested and richly typed
    /// fields are genuinely unreachable across it. No fixture edit changes that.
    NotAvailableInCFfi: LanguageLimitation => ("field ", " not available in C FFI"),
    NotAvailableOnGoProcessingResult: AuthoringGap => ("field ", " not available on Go ProcessingResult"),
    NotAvailableOnPythonProcessingResult: AuthoringGap => ("field ", " not available on Python ProcessingResult"),
    NotAvailableOnRubyProcessingResult: AuthoringGap => ("field ", " not available on Ruby ProcessingResult"),
    NotAvailableOnRProcessingResult: AuthoringGap => ("field ", " not available on R ProcessingResult"),
    NotAvailableOnNodeProcessingResult: AuthoringGap => ("field ", " not available on Node JsProcessingResult"),
    /// ~keep csharp builds its reason into a `skipped_reason` context variable that
    /// `templates/csharp/assertion.jinja` prefixes with `skipped: `, so this wording never appears
    /// on a source line next to the word `skipped:` — grepping for the marker text misses it.
    NotAvailableOnGeneratedCsharpResultType: AuthoringGap => (
        "field ",
        " not available on the generated C# result type",
    ),
    NotAvailableOnDartResultType: AuthoringGap => ("field ", " not available on dart result type"),
    NotAvailableOnElixirResultType: AuthoringGap => ("field ", " not available on Elixir result type"),
    /// ~keep A streaming call yields an event sequence, not a struct, so no field path can name a
    /// property of it. This is a missing generator feature (a first-class stream assertion), not a
    /// fixture mistake — see the `StreamingAssertionOnUnsupportedField` note.
    NotAvailableOnStreamingResultType: GeneratorGap => ("field ", " not available on streaming result type"),
    /// ~keep Simple-result sibling of `NotAvailableWhenResultIsSimple`; same declared-call-shape
    /// reason, different backend's wording.
    NotApplicableForSimpleResultType: LanguageLimitation => ("field ", " not applicable for simple result type"),
    NotAccessibleOnSimpleResultType: LanguageLimitation => ("field ", " not accessible on simple result type"),
    /// ~keep Despite the `result_is_simple` prefix this is the *resolver* rejecting the path, not
    /// the simple-result shape rejecting it — the wording pairs `result_is_simple for field` with
    /// `not available on result type`, which is the `NotAvailableOnResultType` oracle talking.
    ResultIsSimpleForFieldNotAvailable: AuthoringGap => (
        "result_is_simple for field ",
        " not available on result type",
    ),
    /// ~keep Reaching the field would require narrowing a tagged union to one variant first, which
    /// neither language's type system lets generated straight-line assertion code do.
    CrossesTaggedUnionBoundaryInDart: LanguageLimitation => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Dart)",
    ),
    CrossesTaggedUnionBoundaryInSwift: LanguageLimitation => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Swift)",
    ),
    /// ~keep The node (NAPI) and wasm (wasm-bindgen) e2e suites are emitted by one TypeScript
    /// generator, and neither binding gives the variant segment a member to spell. NAPI lowers a
    /// data enum to a single FLATTENED `#[napi(object)]` struct — a discriminant field plus every
    /// variant's own fields as optional siblings (see `backends::napi::gen_bindings::types`'s
    /// `gen_enum` doc and its `gen_tagged_enum_struct_variant_emits_field_names` test), so
    /// `format.excel.sheetCount` has no `excel` property at all and is `TS2339`, not a narrowing
    /// problem. wasm-bindgen's structural `.d.ts` union (`backends::wasm::gen_bindings::ts_union`)
    /// does keep the members apart, but a straight-line assertion cannot narrow to one of them,
    /// which is the same reason Dart and Swift record.
    ///
    /// Classified with Ruby's and PHP's `EnumVariantAccessorNotAvailableIn*` — a property of the
    /// binding's chosen representation, which no `alef.toml` edit changes — rather than as an
    /// `AuthoringGap`, which is fatal and would fail a consumer for a fixture path that is right.
    CrossesTaggedUnionBoundaryInTypescript: LanguageLimitation => (
        "field ",
        " crosses a tagged-union variant boundary (no variant member on the generated TypeScript type)",
    ),
    /// ~keep Unlike Dart/Swift's boundary, Kotlin sealed classes ARE narrowable by pattern
    /// matching — `kotlin/assertions.rs` renders real `is <Union>.<Variant> ->` narrowing for
    /// every single-payload variant `FieldResolver::union_variant_payload` resolves. This fires
    /// only for what that lowering cannot yet handle (a multi-field variant, or a union type the
    /// IR never anchored), so it is alef's own gap, not a property of Kotlin's type system.
    UnionTraversalNotImplementedForKotlin: GeneratorGap => (
        "field ",
        " crosses a tagged-union variant boundary alef does not yet lower for this variant shape in Kotlin",
    ),
    /// ~keep The field or its type is excluded from the binding by explicit config. The exclusion
    /// is already a visible, deliberate decision, so the skip is its honest consequence.
    ExcludedFromSwiftBinding: LanguageLimitation => (
        "field ",
        " references a field or type excluded from the Swift binding",
    ),
    /// ~keep swift-bridge JSON-bridges `Option<Vec<T>>`, `Vec<Vec<_>>` and map getters to a single
    /// `RustString`, which has no `.count`, so a trailing `.length`/`.count`/`.size` on such a leaf
    /// cannot be expressed. `swift/assertions.rs` is right to refuse it — but it used to refuse
    /// with [`FieldSkip::NotAvailableOnResultType`]'s wording, an `AuthoringGap`, and therefore
    /// FATAL, even though its guard runs only *after* `is_valid_for_result` accepted the path.
    /// The backend and the strict gate were asserting opposite things about one resolvable field,
    /// with nothing comparing them: the backend dropped the assertion as an honest ABI limit while
    /// the gate demanded the consumer fix a field path that was never wrong.
    ///
    /// The reason is a property of the swift-bridge ABI, so it is a limitation, and classifying it
    /// here rather than leaving it to a per-fixture `skip` declaration is what stops the collision
    /// recurring: the generator already knows the fact, so the generator owns the verdict.
    CountOnJsonBridgedLeafInSwift: LanguageLimitation => (
        "field ",
        " has no countable Swift leaf (swift-bridge JSON-bridges it to RustString)",
    ),
    /// ~keep A string-key (map) subscript decodes its parent's JSON-bridged `RustString` getter
    /// into `[String: String]`, so the value it yields is a plain Swift `String` — nothing a
    /// further `RustVec` subscript can act on. `json_bridged_traversal_skip` already refuses this
    /// shape when the swift-bridge scan positively classified the map field as JSON-bridged, but
    /// that classification is only ever populated from IR data; a resolver built without IR
    /// (config-only fixtures, or a call site that never wired `with_ir_fields`) never refuses, so
    /// the mixed path still reaches `swift/accessors.rs::materialise_vec_temporaries`, which
    /// reports the hazard by returning `None` instead of hoisting a `RustVec` subscript against a
    /// `String`.
    MixedMapThenVecTraversalInSwift: LanguageLimitation => (
        "field ",
        " mixes a JSON-bridged map subscript with a further RustVec subscript in Swift",
    ),
    /// ~keep `FieldResolver::swift_json_bridged_navigation` DID resolve a decode-and-navigate
    /// expression for this field -- the field is proven reachable, unlike every other variant in
    /// this table that fires because a path is unspellable. What is missing is a renderer turning
    /// that reachable, already-decoded JSON value into this ONE assertion type's Swift check.
    /// `GeneratorGap`, not `LanguageLimitation`: a future alef release can add the renderer with no
    /// consumer-side change, which is exactly what distinguishes this from
    /// [`Self::CountOnJsonBridgedLeafInSwift`] -- that one fires when navigation itself is
    /// impossible (a wildcard or map-key bracket segment), this one fires when navigation
    /// succeeded and only the assertion-type renderer is missing. `after` is empty so the call
    /// site can append which assertion type it was, mirroring
    /// [`Self::StreamingAssertionOnUnsupportedField`]'s rider convention.
    NavigatedJsonBridgedAssertionTypeNotSupportedInSwift: GeneratorGap => ("navigated JSON-bridged field ", ""),
    /// ~keep Was `NestedArrayWildcardNotSupportedInZig` (" not supported in zig"), emitted by the
    /// one backend that had ever guarded the case. Every other backend now refuses it too, so the
    /// wording is language-neutral and single-sourced through `nested_wildcard_skip_line`. The
    /// old text is a prefix of the new one, so anything grepping for it still matches.
    NestedArrayWildcardNotSupported: LanguageLimitation => (
        "nested array-wildcard field ",
        " not supported",
    ),
    ArrayElementNotSupportedInGleam: LanguageLimitation => (
        "array element field ",
        " not yet supported in Gleam e2e",
    ),
    /// ~keep Magnus serializes every data-carrying enum through `serde_json::Value` into a
    /// Symbol-keyed Ruby `Hash`. The Ruby e2e renderer reaches a proven single field beneath a
    /// single-payload variant directly in that flattened Hash. This marker remains for richer
    /// suffixes (nested/indexed payload traversal) that the renderer deliberately cannot spell;
    /// those are Alef generator debt, never a fixture or `alef.toml` authoring error.
    EnumVariantAccessorNotAvailableInRuby: GeneratorGap => (
        "enum variant accessor ",
        " not available on Ruby (serialized to Hash)",
    ),
    /// ~keep PHP's counterpart, but reached by asking the binding backend rather than by matching
    /// a path. `backends::php` lowers an IR enum three ways: an internally tagged data enum
    /// becomes a flat `#[php_class]` whose variant payloads ARE readable properties, while a
    /// unit-variant enum becomes a plain `string` and an `#[serde(untagged)]` data enum is
    /// bridged as a JSON `string`. Only the last two are member-less, and only they produce this
    /// skip — `php/enum_variant_access.rs` walks the same type graph the accessor renderer walks
    /// to decide which one a path reached.
    ///
    /// This replaced a guard keyed on the literal path `metadata.format.` that emitted
    /// [`FieldSkip::NotAvailableOnResultType`] — an `AuthoringGap`, therefore FATAL under the
    /// strict gate — for a reason no consumer could ever act on. A PHP `string` does not grow a
    /// variant accessor because someone edited `alef.toml`, so the verdict is a limitation.
    EnumVariantAccessorNotAvailableInPhp: LanguageLimitation => (
        "enum variant accessor ",
        " not available in PHP (enum lowered to a string, not a class)",
    ),
    /// ~keep The flat class DOES expose this variant payload, as the read-only property ext-php-rs
    /// registers from `#[php(getter)] pub fn get_<flat>` — under the RAW snake_case ident, with no
    /// case conversion, unlike a struct's `#[php(prop, name = to_php_name(..))]`. The shared
    /// accessor renderer (`field_access::optional_renderers::render_php_with_getters`) lowerCamel-
    /// cases every path segment, which is right for struct props and wrong for these, so a
    /// multi-word flat name would emit `->fictionBook` for a property called `fiction_book`.
    ///
    /// This is alef's debt, not the consumer's and not PHP's: the accessor exists and a renderer
    /// that knew the difference could reach it. Bucketed as a generator gap so the summary names
    /// the right owner, and refused rather than emitted wrong — a green assertion against a
    /// property that does not exist is the failure mode this whole funnel exists to stop.
    FlatEnumPropertyNotSpellableInPhp: GeneratorGap => (
        "enum variant accessor ",
        " not yet spellable in PHP (flat-class properties keep their snake_case name)",
    ),
    /// ~keep Reworded from a fixed string ("enum field serialization differs in Ruby") that named
    /// no quoted field and so was structurally uncountable — the strict gate could never have seen
    /// it whatever patterns it matched. The reason is unchanged; only the field is now named.
    EnumSerializationDiffersInRuby: LanguageLimitation => ("field ", " enum serialization differs in Ruby"),
    /// ~keep The python streaming adapter has no accessor for this virtual field. Same missing
    /// feature as `StreamingAssertionOnUnsupportedField`, seen from the python side.
    NoPythonStreamingAccessor: GeneratorGap => ("streaming field ", ": no python accessor"),
    /// ~keep This is the variant that hid whole expected-event-sequence assertions — a fixture
    /// asserting a traversal order rendered a comment and the test still passed.
    ///
    /// It is nonetheless a `GeneratorGap`, not an `AuthoringGap`: a streaming call returns an
    /// event sequence rather than a struct, so *no* field mapping can ever express an assertion
    /// over that sequence. Consumers cannot fix these from their own config — the fix is a
    /// first-class streaming assertion type in alef. Failing their build on it would force a
    /// blanket opt-out, which is the silent skip again with more ceremony. Counted loudly instead,
    /// in a bucket that names alef as the owner.
    StreamingAssertionOnUnsupportedField: GeneratorGap => ("streaming assertion on unsupported field ", ""),
    /// Emitted by `templates/{java,php}/synthetic_assertion.jinja`, which cannot call into Rust;
    /// registered here so the strict gate still counts it. Declared-call-shape reason. ~keep
    ResultIsSimpleNotOnSimpleResultType: LanguageLimitation => (
        "result_is_simple, field ",
        " not on simple result type",
    ),
    /// Emitted by `templates/java/synthetic_assertion.jinja`. ~keep
    NotAvailableOnJavaResultType: AuthoringGap => ("field ", " not available on Java result type"),
    /// Emitted by `templates/php/synthetic_assertion.jinja`. ~keep
    NotAvailableOnPhpResultType: AuthoringGap => ("field ", " not available on PHP result type"),
    /// Emitted by `templates/r/synthetic_assertion.jinja`. ~keep
    NotAvailableOnRResultType: AuthoringGap => ("field ", " not available on R result type"),
    /// ~keep A Zig enum compares with `==`/`@tagName`, but the fixture's declared `equals` value
    /// is the serde wire spelling, not the Zig-cased tag identifier `public_host_identifier`
    /// would produce for it — reproducing that mapping correctly requires resolving the exact
    /// `EnumDef` backing this field (its `serde_rename_all`/per-variant `serde_rename`), which
    /// the field-path classification this skip guards does not carry. Mirrors gleam's
    /// `EnumEquals...` skip for the same newly IR-classified case (see `8d199c0bf`): the JSON
    /// path already handles enum `equals` correctly (it compares against the raw wire string
    /// directly), so this is a `GeneratorGap` on the typed-struct path only, not a fixture bug.
    EnumEqualsNotSupportedOnZigTypedResult: GeneratorGap => (
        "enum field ",
        " comparison not yet supported on zig's typed-struct result",
    ),
    /// ~keep The same shape as [`Self::EnumEqualsNotSupportedOnZigTypedResult`], for the four
    /// targets whose binding lowers a *unit-only* enum to a scalar carrying the serde wire value
    /// (dart `.wireValue`, kotlin_android `.toWire()`, the kotlin/JVM Java facade's `.getValue()`,
    /// swift `.rawValue`) but lowers a data-carrying one to a payload union that declares no such
    /// member. Withholding the accessor alone is not a fix: the field then falls through to the
    /// generic string pipeline and is compared to the fixture literal, which is a Dart/Kotlin
    /// comparison against a wrapper object's `toString()` (always false at runtime) and a Swift
    /// type mismatch that does not compile. Refusing the assertion outright is the only honest
    /// option left at these call sites.
    ///
    /// `GeneratorGap`, not `LanguageLimitation`: all four languages *can* discriminate a union
    /// variant — `kotlin/discriminated.rs` already emits `is <Union>.<Variant>` for a path that
    /// crosses one — so a future alef release can lower this exactly. What is missing is the
    /// wire-value-to-variant resolution at this call site, which sees only a field path; and for
    /// `#[serde(untagged)]` there is no discriminator on the wire to resolve against at all.
    PayloadUnionHasNoScalarWireAccessor: GeneratorGap => (
        "enum field ",
        " is a payload-carrying union with no scalar wire accessor in this binding",
    ),
}

impl FieldSkip {
    /// The human-readable marker body for `field`, to be written after a backend's own
    /// `<comment-open> skipped: ` prefix.
    pub(crate) fn message(self, field: &str) -> String {
        let Shape { before, after } = self.shape();
        format!("{before}'{field}'{after}")
    }

    /// The field name a single rendered line names, if the line carries any registered wording.
    ///
    /// Production code uses [`Self::extract_classified`]; this narrower form exists so the
    /// recognition tests below can assert *what* is matched without also restating every
    /// variant's classification. ~keep
    #[cfg(test)]
    pub(crate) fn extract(line: &str) -> Option<&str> {
        Self::extract_classified(line).map(|(field, _)| field)
    }

    /// The field name *and* the variant that named it, so a caller can tell an authoring gap
    /// from a language limitation without re-matching the wording itself.
    pub(crate) fn extract_classified(line: &str) -> Option<(&str, Self)> {
        Self::ALL
            .iter()
            .find_map(|variant| variant.field_in(line).map(|field| (field, *variant)))
    }

    /// ~keep Every occurrence of `before` is tried, not just the first: `before` is often the bare
    /// `"field "`, which also occurs inside longer phrases ("synthetic field ", "for field "), so
    /// stopping at the first hit would miss a line whose earlier `field ` is not the quoted one.
    fn field_in(self, line: &str) -> Option<&str> {
        let Shape { before, after } = self.shape();
        for (start, _) in line.match_indices(before) {
            let rest = &line[start + before.len()..];
            let Some(quoted) = rest.strip_prefix('\'') else {
                continue;
            };
            let Some(end) = quoted.find('\'') else {
                continue;
            };
            if quoted[end + 1..].starts_with(after) {
                return Some(&quoted[..end]);
            }
        }
        None
    }
}

/// The `skipped:` line refusing a doubly-nested bracket-wildcard path, or `None` when
/// `element_sub_path` carries no second wildcard.
///
/// `FieldResolver::wildcard_split` consumes the FIRST `[].` only, so for `pages[].links[].url`
/// it hands back the element sub-path `links[].url`. Every backend then builds its per-element
/// accessor from that sub-path — and `parse_path` lowers the surviving `[]` to index 0, so the
/// emitted loop covers `pages` while the check inside it silently reads `links[0]`. A fixture
/// claiming "every element" then passes on element zero alone, in a language whose output is
/// green.
///
/// Refusing is the deliberate answer, not a placeholder: a nested quantifier is expressible in
/// most of these languages but not all (Go emits statements rather than an expression, R goes
/// through `vapply`, Dart's element accessor renders the wildcard as the syntactically invalid
/// `links![]`), so emitting it would land unevenly — and an index-0 fallback in the backends
/// that could not keep up is exactly the false green this function exists to remove. `zig` had
/// already made that call for itself; this is the same call for everyone, single-sourced so the
/// predicate and the wording cannot drift apart between backends.
///
/// The returned line carries no trailing newline: callers `writeln!` it, or return it where a
/// rendered line is expected. Indentation and comment syntax stay at the call site, matching the
/// rest of this module. ~keep
pub(crate) fn nested_wildcard_skip_line(
    indent: &str,
    comment_open: &str,
    field: &str,
    element_sub_path: &str,
) -> Option<String> {
    if !element_sub_path.contains("[]") {
        return None;
    }
    Some(format!(
        "{indent}{comment_open} skipped: {}",
        FieldSkip::NestedArrayWildcardNotSupported.message(field)
    ))
}

#[cfg(test)]
mod tests {
    use super::{FieldSkip, SkipClass, nested_wildcard_skip_line};

    /// ~keep The classification is what decides whose build breaks, so it is pinned here rather
    /// than left implicit in the variant list. These four are the load-bearing calls:
    ///
    /// - `NotAvailableOnResultType` is the canonical fixable gap and must stay fatal, or this
    ///   whole change is inert.
    /// - the streaming wordings must NOT be fatal: a stream is an event sequence, so no field
    ///   mapping can express an assertion over it and a consumer cannot fix it from their config.
    ///   Making them fatal would force a blanket opt-out and rebuild the silent skip.
    /// - a tagged-union boundary is a real type-system limit and was never anyone's mistake.
    #[test]
    fn the_load_bearing_classifications_are_pinned() {
        assert_eq!(FieldSkip::NotAvailableOnResultType.class(), SkipClass::AuthoringGap);
        assert_eq!(
            FieldSkip::StreamingAssertionOnUnsupportedField.class(),
            SkipClass::GeneratorGap
        );
        assert_eq!(FieldSkip::NoPythonStreamingAccessor.class(), SkipClass::GeneratorGap);
        assert_eq!(
            FieldSkip::NotAvailableOnStreamingResultType.class(),
            SkipClass::GeneratorGap
        );
        assert_eq!(
            FieldSkip::CrossesTaggedUnionBoundaryInDart.class(),
            SkipClass::LanguageLimitation
        );
        assert_eq!(
            FieldSkip::CrossesTaggedUnionBoundaryInTypescript.class(),
            SkipClass::LanguageLimitation,
            "NAPI flattens a data enum into one object with no variant member, so no fixture or \
             alef.toml edit reaches the path; an authoring gap here would fail a correct consumer"
        );
        assert_eq!(FieldSkip::NotAvailableInCFfi.class(), SkipClass::LanguageLimitation);
        assert_eq!(
            FieldSkip::CountOnJsonBridgedLeafInSwift.class(),
            SkipClass::LanguageLimitation,
            "this guard fires only on fields the resolver ACCEPTED; classifying it as an authoring \
             gap makes the backend and the strict gate contradict each other about one field"
        );
        assert_eq!(
            FieldSkip::NavigatedJsonBridgedAssertionTypeNotSupportedInSwift.class(),
            SkipClass::GeneratorGap,
            "the field was proven reachable by swift_json_bridged_navigation -- only the \
             assertion-type renderer is missing, which is alef's own gap, not a consumer's or a \
             property of Swift"
        );
        assert_eq!(
            FieldSkip::EnumVariantAccessorNotAvailableInRuby.class(),
            SkipClass::GeneratorGap,
            "backends::magnus already generates a native per-variant accessor method for \
             internally tagged data enums (gen_enum's per-variant impl block) — it is simply \
             never registered with the Ruby class in module_init.rs. Magnus can register \
             arbitrary instance methods, so this is alef's own unfinished wiring, not a property \
             of Ruby or magnus; a LanguageLimitation verdict would misattribute it and block a \
             future alef release from ever closing it without a reclassification"
        );
    }

    /// ~keep `StreamingAssertionOnUnsupportedField`'s `after` is deliberately empty, so a caller
    /// may append its own reason to the rendered line and still be recognised. That is not an
    /// accident of the table — it is the only path a streaming sub-kind with its own semantics has
    /// for carrying that semantics into the emitted file. `stream_complete` is the live example:
    /// it is a registered streaming-virtual field whose skip reads "this stream's chunks carry no
    /// terminal finish_reason", a reason no other streaming field produces, and it is emitted with
    /// exactly this rider by `ruby/examples.rs` and `csharp/streaming.rs`. If a future
    /// reason-carrying path drops the rider or fixes the wording's tail, `stream_complete` is the
    /// sub-kind that falls out of the count — so the rider is pinned here.
    #[test]
    fn a_trailing_reason_does_not_break_recognition() {
        let line = format!(
            "    # skipped: {}; this stream's chunks carry no terminal finish_reason, so \
             completion is not observable here",
            FieldSkip::StreamingAssertionOnUnsupportedField.message("stream_complete")
        );
        assert_eq!(
            FieldSkip::extract_classified(&line),
            Some(("stream_complete", FieldSkip::StreamingAssertionOnUnsupportedField)),
            "got: {line}"
        );
    }

    /// Every class must actually be populated: a table where one arm went empty would silently
    /// collapse the three-way distinction back into a two-way one.
    #[test]
    fn every_class_is_represented() {
        for class in [
            SkipClass::AuthoringGap,
            SkipClass::GeneratorGap,
            SkipClass::LanguageLimitation,
        ] {
            assert!(
                FieldSkip::ALL.iter().any(|variant| variant.class() == class),
                "no variant is classified {class:?}"
            );
        }
    }

    /// The load-bearing invariant: render and recognition read the same `Shape`, so anything a
    /// backend can emit through the funnel is by construction something the strict gate counts.
    #[test]
    fn every_variant_round_trips_through_extract() {
        for variant in FieldSkip::ALL {
            let rendered = format!("    // skipped: {}", variant.message("outer.inner.leaf"));
            assert_eq!(
                FieldSkip::extract(&rendered),
                Some("outer.inner.leaf"),
                "variant {variant:?} rendered `{rendered}` but the gate did not recognise it"
            );
        }
    }

    #[test]
    fn tagged_union_boundary_wordings_are_recognised() {
        let dart = "    // skipped: field 'tags' crosses a tagged-union variant boundary (not expressible in Dart)";
        assert_eq!(FieldSkip::extract(dart), Some("tags"));
        let swift = "    // skipped: field 'tags' crosses a tagged-union variant boundary (not expressible in Swift)";
        assert_eq!(FieldSkip::extract(swift), Some("tags"));
    }

    #[test]
    fn ruby_serialized_enum_accessor_wording_is_recognised() {
        let line = "    # skipped: enum variant accessor 'format.excel' not available on Ruby (serialized to Hash)";
        assert_eq!(FieldSkip::extract(line), Some("format.excel"));
    }

    #[test]
    fn result_is_simple_template_wording_is_recognised() {
        let line = "        // skipped: result_is_simple, field 'metadata.title' not on simple result type";
        assert_eq!(FieldSkip::extract(line), Some("metadata.title"));
    }

    /// Negative control: an unsupported *assertion type* is a different defect and stays uncounted,
    /// even though the line contains `field '<name>'`.
    #[test]
    fn unsupported_assertion_type_wordings_stay_uncounted() {
        let synthetic = "\t// skipped: unsupported assertion type on synthetic field 'embeddings'";
        assert_eq!(FieldSkip::extract(synthetic), None);
        let traversal = "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'";
        assert_eq!(FieldSkip::extract(traversal), None);
        let streaming = "    // skipped: assertion type 'count_min' on field 'chunks' not yet supported for streaming";
        assert_eq!(FieldSkip::extract(streaming), None);
        let scalar = "        // skipped: field 'content' is a scalar String without meaningful .count";
        assert_eq!(FieldSkip::extract(scalar), None);
    }

    #[test]
    fn a_line_with_no_marker_is_not_recognised() {
        assert_eq!(FieldSkip::extract("    assert result.count == 1"), None);
        assert_eq!(FieldSkip::extract("        // skipped: field is a scalar String"), None);
    }

    /// An earlier `field ` inside a longer phrase must not shift which quote pair is read.
    #[test]
    fn extracts_the_quoted_name_not_a_later_phrase() {
        let line = "  # skipped: result_is_simple for field 'metadata' not available on result type";
        assert_eq!(FieldSkip::extract(line), Some("metadata"));
    }

    #[test]
    fn should_refuse_when_the_element_sub_path_still_carries_a_wildcard() {
        assert_eq!(
            nested_wildcard_skip_line("    ", "//", "pages[].links[].url", "links[].url").as_deref(),
            Some("    // skipped: nested array-wildcard field 'pages[].links[].url' not supported")
        );
    }

    /// A single wildcard leaves nothing behind on the element side, and that is the case every
    /// backend must keep quantifying over — refusing it would delete working coverage. ~keep
    #[test]
    fn should_not_refuse_a_single_wildcard() {
        assert_eq!(nested_wildcard_skip_line("    ", "#", "links[].url", "url"), None);
    }

    /// An explicit numeric index is a different, correct feature: `pages[].links[0].url` names
    /// element zero because the fixture author asked for it. ~keep
    #[test]
    fn should_not_refuse_an_explicit_inner_index() {
        assert_eq!(
            nested_wildcard_skip_line("    ", "#", "pages[].links[0].url", "links[0].url"),
            None
        );
    }

    /// The refusal must round-trip through the strict field-availability gate, or the honest
    /// gap is invisible to the very tooling that counts gaps. ~keep
    #[test]
    fn the_nested_wildcard_refusal_is_counted_by_the_strict_gate() {
        let line = nested_wildcard_skip_line("  ", "#", "pages[].links[].url", "links[].url").unwrap();
        assert_eq!(FieldSkip::extract(&line), Some("pages[].links[].url"));
    }
}
