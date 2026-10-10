use super::*;

// ---------------------------------------------------------------------------
// ~keep Synthetic reproduction of the tslp `DownloadManager` constructor audit (measured
// against a real `javac` run, not inference -- see formatting.rs's `reserved_words` doc
// comment). The canary consumer does not exercise `[crates.constructors]` -- no such
// function exists in the canary -- so this cannot be verified against that consumer's own
// generated output; per the coordinator, a repo that doesn't exercise a code path cannot
// clear it. These tests build the exact colliding shape by hand: a static method named
// `new` on an opaque type, which is what a curated constructor renders as today (see the "not
// yet modeled" finding for item 1 -- the docs pipeline has no awareness of
// `ClientConstructorConfig`).
// ---------------------------------------------------------------------------

fn download_manager_new_method() -> crate::core::ir::MethodDef {
    make_method(
        "new",
        vec![make_param("version", TypeRef::String, false)],
        TypeRef::Named("DownloadManager".to_string()),
        false,
        true,
        None,
    )
}

/// ~keep Java's real defect used to be a reserved-word collision: before 28f310259, `new`
/// had no arm in the docs' Java keyword-rename table (only `default` did), so a curated
/// opaque type's default constructor reached the identifier gate as the raw word `new` and
/// panicked, aborting the whole docs run. The table is now mirrored from the backend's
/// `safe_java_method_name`, so this shape renders as `create` -- same outcome class as the
/// Go/C#/Zig/Elixir siblings below. Asserting a failure here would pin the bug back in.
#[test]
fn test_java_constructor_named_new_is_renamed_to_create_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Java,
        TEST_PREFIX,
    );
    assert!(sig.contains("create"), "{sig}");
    assert!(!sig.contains("new"), "{sig}");
}

/// ~keep The property the original (now-removed) `should_panic` test was actually
/// protecting: the identifier gate itself still rejects a raw, un-renamed Java reserved
/// word. `render_method_signature` can no longer exercise this for Java -- `func_name`'s
/// keyword table now renames every `JAVA_KEYWORDS` entry, `new` included, before it can
/// reach the gate (see `test_func_name_java_output_passes_the_identifier_gate` in
/// naming.rs) -- so this calls the gate directly instead of routing through a constructor
/// path that no longer produces an unrenamed keyword. It asserts the returned error rather
/// than a panic: the gate reports now, it does not abort, so a `should_panic` test here
/// would fail for the wrong reason and a bare call would assert nothing at all.
#[test]
fn test_identifier_gate_still_rejects_a_raw_java_keyword() {
    let violation = crate::docs::formatting::check_identifier(
        "new",
        Language::Java,
        crate::docs::formatting::IdentifierPosition::Member,
        "a test context",
    )
    .expect_err("`new` is a Java reserved word in every position");
    assert_eq!(violation.reason, "reserved word");
    assert!(violation.to_string().contains("member position"), "{violation}");
}

/// ~keep Supersedes the earlier version of this test, which pinned Dart's defect as
/// unfixed ("no Dart rename exists yet") and asserted the gate rejecting the raw word. That
/// was correct at the time but is no longer the ground truth: `func_name` now carries a
/// `(Language::Dart, "new") => "create"` arm (naming.rs), mirroring the Java `new` -> `create`
/// fix, so a static `new` returning `Self` renders as `create` and never reaches the gate as
/// the raw reserved word at all. This is a rename, not the TypeScript/PHP member-position
/// relaxation: the docs pipeline already renders a legal Dart declaration shape for this case
/// (`static Future<T> {name}(...)`) -- only the identifier `new` itself was illegal, unlike
/// Swift's `init`, which is a declaration-keyword mismatch no rename can fix (see
/// `is_swift_static_constructor` in signatures.rs and the Swift tests below). Before this fix,
/// the docs pipeline shipped `static Future<DownloadManager> new(String version)` at three
/// sites in one real consumer's `api-dart.md`, none of which compile.
#[test]
fn test_dart_constructor_named_new_is_renamed_to_create_by_the_identifier_gate() {
    let name = crate::docs::naming::method_name("DownloadManager", "new", Language::Dart, TEST_PREFIX);
    assert_eq!(name, "create");

    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Dart,
        TEST_PREFIX,
    );
    assert_eq!(sig, "static Future<DownloadManager> create(String version)");
}

/// ~keep The Dart fix is a rename applied upstream in `func_name`, not a relaxation of the
/// gate itself -- unlike TypeScript/PHP's genuine member-position relaxation, Dart's `new`
/// must still be rejected by `check_identifier` when something other than `func_name`'s table
/// feeds it in raw (a hand-written override, a different curated path). This is the positive
/// control that separates "fixed by renaming upstream" from "fixed by disarming the check":
/// it fails the moment the gate itself stops rejecting `new` in Dart member position.
#[test]
fn test_identifier_gate_still_rejects_raw_new_in_dart_member_position() {
    let violation = crate::docs::formatting::check_identifier(
        "new",
        Language::Dart,
        crate::docs::formatting::IdentifierPosition::Member,
        "a test context",
    )
    .expect_err("`new` is a Dart reserved word in member position");
    assert_eq!(violation.reason, "reserved word");
}

/// ~keep Swift's `init` is a declaration keyword, not a reserved identifier: `public static
/// func init(...)` is a syntax error regardless of what the name is escaped or renamed to, so
/// neither a rename nor a member-position relaxation can fix it -- the *declaration shape*
/// has to change. `is_swift_static_constructor` (signatures.rs) diverts a static `new`
/// returning `Self` to a real initializer -- no `static`, no `func`, no name -- before the
/// identifier gate ever sees `init` as a member-position candidate. Asserts the exact emitted
/// text, not just `contains("init")`: `public static func init(...)` also contains "init" and
/// would pass a weaker assertion while remaining exactly the syntax error this exists to fix.
/// Measured against the real consumer trigger: a fatal `alef all` abort on
/// `generated Swift identifier \`init\` is invalid (reserved word) in member position`.
#[test]
fn test_swift_constructor_named_new_is_promoted_to_a_real_initializer() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Swift,
        TEST_PREFIX,
    );
    assert_eq!(sig, "public init(version: String)");
}

/// ~keep The gate is not disarmed to make the Swift fix work: the constructor path above
/// never feeds it the word `init` at all, and a genuinely illegal Swift identifier --
/// including the literal word `init` used outside that promoted path (an ordinary instance
/// method, or a static method whose return type isn't the owning type) -- must still be
/// rejected. This is what proves the fix is "the emitter stopped emitting `init` as a member
/// name" rather than "the gate stopped checking for it".
#[test]
fn test_identifier_gate_still_rejects_raw_init_in_swift_member_position() {
    let violation = crate::docs::formatting::check_identifier(
        "init",
        Language::Swift,
        crate::docs::formatting::IdentifierPosition::Member,
        "a test context",
    )
    .expect_err("`init` is a Swift declaration keyword, not a legal member identifier");
    assert_eq!(violation.reason, "reserved word");
}

/// ~keep Negative control for `is_swift_static_constructor`: an instance method (no
/// `static`) named `new` is not a constructor shape, even if it otherwise matches -- Swift
/// has no rule against an *instance* method named `new` (it is not reserved), so this must
/// render as an ordinary member, not an initializer.
#[test]
fn test_swift_instance_method_named_new_is_not_promoted_to_an_initializer() {
    let method = make_method(
        "new",
        vec![make_param("version", TypeRef::String, false)],
        TypeRef::Named("DownloadManager".to_string()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "DownloadManager", Language::Swift, TEST_PREFIX);
    assert_eq!(sig, "public func new(version: String) -> DownloadManager");
}

/// ~keep A real consumer docs crash, end to end. `static new(version: string): DownloadManager`
/// is what the napi backend really writes into a generated `index.d.ts`, and it is valid
/// TypeScript -- ES5 freed reserved words in `PropertyName` position, which is what a class
/// element's name is. Rendering it used to abort the entire docs run with a raw panic and no
/// ERROR line; it must now render, and the gate must agree it is legal.
#[test]
fn test_typescript_constructor_named_new_renders_instead_of_aborting_the_docs_run() {
    let name = crate::docs::naming::method_name("DownloadManager", "new", Language::Node, TEST_PREFIX);
    assert_eq!(name, "new");
    assert_eq!(
        crate::docs::formatting::check_identifier(
            &name,
            Language::Node,
            crate::docs::formatting::IdentifierPosition::Member,
            "a method signature",
        ),
        Ok(())
    );

    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Node,
        TEST_PREFIX,
    );
    assert!(sig.contains("new("), "{sig}");
}

/// ~keep PHP's failure on this input was the same false positive as TypeScript's: the PHP
/// 7.0 context-sensitive lexer made every reserved word usable as a method name, and the
/// PHP backend emits `method.name.to_lower_camel_case()` with no keyword escape, so
/// `public function new(...)` is exactly what ships.
#[test]
fn test_php_constructor_named_new_renders_instead_of_aborting_the_docs_run() {
    assert_eq!(
        crate::docs::formatting::check_identifier(
            "new",
            Language::Php,
            crate::docs::formatting::IdentifierPosition::Member,
            "a method signature",
        ),
        Ok(())
    );

    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Php,
        TEST_PREFIX,
    );
    assert!(sig.contains("new("), "{sig}");
}

/// ~keep Two defects, not one: Go's real bug (on the default, unconfigured opaque-
/// constructor path the peer actually measured -- a plain FFI export run through Go's
/// generic name mapping, not `[crates.constructors]`) is shape, not a reserved word --
/// `New` is legal Go. The gate correctly does not fire here. This asserts the fully
/// source-verified real shape: no receiver (`gen_method_wrapper`'s static template) and a
/// pointer-wrapped return (`go_optional_type` pointer-wraps every Named return,
/// unconditionally -- see signatures.rs's `go_return_type`).
#[test]
fn test_go_constructor_named_new_is_not_caught_by_the_identifier_gate() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Go,
        TEST_PREFIX,
    );
    assert_eq!(sig, "func DownloadManagerNew(version string) *DownloadManager");
}

/// ~keep C#'s real defect (a fabricated static factory instead of a real constructor) is a
/// name-*selection* problem: `func_name` PascalCases `new` to `New`, which collides with
/// nothing. The gate cannot and does not catch this class of bug, so the docs used to publish
/// `public DownloadManager New(string version)` -- a member `gen_opaque_type` never emits,
/// because `is_static_constructor` diverts this exact shape to
/// `opaque_static_constructor_signature.jinja` (`public {{ class_name }}({{ param_list }})`).
/// Asserting only `contains("New")`, as this test originally did, passed on the wrong output.
#[test]
fn test_csharp_constructor_named_new_is_promoted_to_a_real_constructor() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Csharp,
        TEST_PREFIX,
    );
    assert_eq!(sig, "public DownloadManager(string version)");
}

/// ~keep Zig's real defect is shape (a method rendering over what the backend actually
/// emits as a free function `new_download_manager`) -- `new` is not reserved in Zig, so the
/// gate stays silent and the old `contains("new")` assertion passed on the broken
/// `pub fn new(...)` just as happily as on the correct name.
#[test]
fn test_zig_constructor_named_new_renders_as_a_type_suffixed_free_function() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert_eq!(sig, "pub fn new_download_manager(version: []const u8) DownloadManager");
}

/// ~keep Elixir is tslp's free positive control: the one language whose real idiom
/// (`DownloadManager.new/1`) happens to coincide with what the template emits, because
/// Elixir does not reserve `new` either way. The one success and the five failures share
/// the same mechanism -- a template that never checked -- which is the strongest form of
/// the "formula, not lookup" claim this whole round has been building.
#[test]
fn test_elixir_constructor_named_new_is_the_positive_control() {
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Elixir,
        TEST_PREFIX,
    );
    assert!(sig.contains("new"), "{sig}");
}

// ---------------------------------------------------------------------------
// ~keep Cross-checks against the backends themselves, in the shape of
// `test_func_name_java_matches_backend_safe_java_method_name` (naming.rs). The assertions
// above pin a literal string, which stops the *current* divergence from coming back but
// cannot notice the backend moving underneath them; these call the backend's own predicate
// and its own templates, so a rename on either side fails here first.
// ---------------------------------------------------------------------------

/// The docs' `is_csharp_static_constructor` is a hand-copy of the C# backend's
/// `is_static_constructor`. Pin them together over the full clause matrix -- name, staticness,
/// arity and return type each independently flip the answer, and a sampled cross-check would
/// leave three of the four free to drift.
#[test]
fn test_csharp_docs_constructor_predicate_matches_the_backend_predicate() {
    use crate::backends::csharp::gen_bindings::types::constructors::is_static_constructor;

    let string_param = || vec![make_param("version", TypeRef::String, false)];
    let owner = TypeRef::Named("DownloadManager".to_string());
    let cases = vec![
        ("new", string_param(), owner.clone(), true),
        ("new", vec![], owner.clone(), true),
        ("new", string_param(), owner.clone(), false),
        ("new", string_param(), TypeRef::String, true),
        ("new", string_param(), TypeRef::Named("Other".to_string()), true),
        (
            "new",
            string_param(),
            TypeRef::Named("crate::DownloadManager".to_string()),
            true,
        ),
        ("create", string_param(), owner.clone(), true),
        ("with_cache_dir", string_param(), owner, true),
    ];
    for (name, params, return_type, is_static) in cases {
        let method = make_method(name, params, return_type, false, is_static, None);
        assert_eq!(
            is_csharp_static_constructor(&method, "DownloadManager"),
            is_static_constructor(&method, "DownloadManager"),
            "docs and the C# backend disagree about `{name}` (is_static={is_static})"
        );
    }
}

/// The Zig static-method name is a formula (`{method}_{type}`) owned by
/// `opaque_static_signature.jinja`. Render that template with the backend's own inputs and
/// require the documented signature to open with the identifier it produces.
#[test]
fn test_zig_docs_static_name_matches_the_backend_template() {
    let backend = crate::backends::zig::template_env::render(
        "opaque_static_signature.jinja",
        crate::alef_context! {
            method_snake => "new",
            type_snake => "download_manager",
            params => "",
            return_ty => "DownloadManager",
        },
    );
    let backend_head = backend
        .split('(')
        .next()
        .expect("template always emits an opening paren");
    let sig = render_method_signature(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert!(
        sig.starts_with(&format!("{backend_head}(")),
        "docs `{sig}` does not open with the backend's `{backend_head}(`"
    );
}

/// Go's static shape is likewise a formula owned by `method_signature_static.jinja`
/// (`func {receiver_type}{method_name}(`). Both the signature *and* the example on the same
/// page must resolve to it -- the page used to print `func DownloadManagerNew(...)` above an
/// example calling `DownloadManager.New(...)`, contradicting itself.
#[test]
fn test_go_docs_signature_and_example_both_match_the_backend_template() {
    let backend = crate::backends::go::template_env::render(
        "method_signature_static.jinja",
        crate::alef_context! {
            receiver_type => "DownloadManager",
            method_name => "New",
            params => "",
            return_type => "",
        },
    );
    let backend_head = backend
        .split('(')
        .next()
        .expect("template always emits an opening paren")
        .trim_start_matches("func ")
        .to_string();
    let method = download_manager_new_method();
    let sig = render_method_signature(&method, "DownloadManager", Language::Go, TEST_PREFIX);
    assert!(
        sig.starts_with(&format!("func {backend_head}(")),
        "docs signature `{sig}` does not name the backend's `{backend_head}`"
    );
    let example = crate::docs::examples::render_method_example(&method, "DownloadManager", Language::Go, TEST_PREFIX);
    assert!(
        example.contains(&format!("{backend_head}(\"value\")")),
        "docs example must call the package-level `{backend_head}`: {example}"
    );
    assert!(
        !example.contains("DownloadManager.New("),
        "docs example still uses member syntax Go has no equivalent for: {example}"
    );
}

/// The C# example must construct with `new`, matching the constructor the signature arm now
/// documents. `DownloadManager.New(...)` named a member `gen_opaque_type` never emits.
#[test]
fn test_csharp_docs_example_constructs_with_new() {
    let example = crate::docs::examples::render_method_example(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Csharp,
        TEST_PREFIX,
    );
    assert!(example.contains("new DownloadManager(\"value\")"), "{example}");
    assert!(!example.contains("DownloadManager.New("), "{example}");
}

/// The Zig example must call the same top-level `new_download_manager` the signature names,
/// not `DownloadManager.new(...)` -- Zig has no method syntax for a function the backend emits
/// outside the struct.
#[test]
fn test_zig_docs_example_calls_the_top_level_function() {
    let example = crate::docs::examples::render_method_example(
        &download_manager_new_method(),
        "DownloadManager",
        Language::Zig,
        TEST_PREFIX,
    );
    assert!(example.contains("new_download_manager("), "{example}");
    assert!(!example.contains("DownloadManager.new("), "{example}");
}

// ---------------------------------------------------------------------------
// ~keep The class this whole gate exists to close is not "constructors named `new`" -- it is
// "any emitted `pub fn` whose name is a keyword in the target language". A second,
// independently measured consumer tree found the general case on ordinary (non-constructor)
// methods: `pub fn global() -> &'static Registry` collided with Python's `global`, and
// `pub fn get(&self, id: &str) -> Option<&Preset>` (repeated across five registries) collided
// with Dart's `get` and Kotlin's (soft-keyword, conservatively listed) `get`. None of these are
// static, none return the owning type -- there is no constructor shape to divert, so
// `func_name`'s generic reserved-word sweep (naming.rs) is what has to catch them, not a
// per-shape renderer branch like Swift's `is_swift_static_constructor`.
// ---------------------------------------------------------------------------

/// ~keep A free function is judged in Declaration position, not Member -- `render_python_fn_sig`
/// always has, this is not new behavior this round introduced. What's new is that `func_name`
/// now escapes the collision instead of emitting the raw reserved word for the gate to reject.
#[test]
fn test_python_free_function_named_global_is_escaped_by_func_name() {
    assert_eq!(
        crate::docs::naming::func_name("global", Language::Python, TEST_PREFIX),
        "global_"
    );
    let func = crate::docs::test_helpers::make_function("global", vec![], TypeRef::Unit, false, None);
    let sig = crate::docs::signatures::render_function_signature(
        &func,
        Language::Python,
        TEST_PREFIX,
        TEST_CRATE_NAME,
        &crate::core::ir::ApiSurface::default(),
    );
    assert!(sig.contains("global_"), "{sig}");
    assert!(!sig.contains("def global("), "{sig}");
}

/// ~keep An ordinary instance method, not a constructor: `is_dart`-anything predicate never
/// enters into this at all, because there is none to write -- `get` collides in every Dart
/// position regardless of shape, so the escape has to live in `func_name`, upstream of every
/// per-language renderer, the same place the `new` fix lives.
#[test]
fn test_dart_instance_method_named_get_is_escaped_by_func_name() {
    let name = crate::docs::naming::method_name("Registry", "get", Language::Dart, TEST_PREFIX);
    assert_eq!(name, "get_");

    let method = make_method(
        "get",
        vec![make_param("id", TypeRef::String, false)],
        TypeRef::Optional(Box::new(TypeRef::Named("Preset".to_string()))),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Registry", Language::Dart, TEST_PREFIX);
    assert!(sig.contains("get_("), "{sig}");
    assert!(!sig.contains(" get("), "{sig}");
}

/// ~keep Kotlin's `get` is a soft keyword, not a hard reserved word (`KOTLIN_KEYWORDS` in
/// `core::keywords`, the list the real backend's own escape helper checks, does not contain
/// it) -- but `formatting.rs`'s `reserved_words(Kotlin)` deliberately lists it anyway, as a
/// conservative superset covering every contextual position, not just the ones this docs
/// layer has modeled (see that function's doc comment). The escape here follows the gate's
/// conservative verdict, not the backend's narrower one: escaping a word Kotlin might have
/// accepted unescaped is a compiling, slightly-less-idiomatic method name; failing to escape
/// a word Kotlin actually rejects is exactly the defect this whole gate exists to catch.
/// `identifier_violation` is the single source of truth both sides read, so the escape and the
/// gate cannot disagree about which words need it.
#[test]
fn test_kotlin_instance_method_named_get_is_escaped_by_func_name() {
    assert_eq!(
        crate::docs::naming::func_name("get", Language::Kotlin, TEST_PREFIX),
        "get_"
    );
    assert_eq!(
        crate::docs::formatting::check_identifier(
            "get_",
            Language::Kotlin,
            crate::docs::formatting::IdentifierPosition::Member,
            "a method signature",
        ),
        Ok(())
    );
}

/// ~keep Negative control: the generic sweep must not touch TypeScript or PHP, whose
/// member-position relaxation depends on `func_name` handing back the raw word for the
/// per-language renderer (`render_method_signature_with_override`) to judge in Member
/// position -- `func_name` itself has no position parameter, so escaping here would remove
/// the raw `new` those two languages are supposed to keep. Independently confirmed by a third
/// repo's whole-tree grep finding `static new(): TokenCounter` as a legitimate TypeScript hit.
#[test]
fn test_generic_reserved_word_sweep_does_not_touch_node_or_php() {
    for lang in [Language::Node, Language::Wasm, Language::Php] {
        assert_eq!(
            crate::docs::naming::func_name("new", lang, TEST_PREFIX),
            "new",
            "{lang:?} must keep emitting the raw word -- its member-position relaxation depends on it"
        );
    }
}

/// ~keep Negative control for the sweep itself: an ordinary, non-colliding method name must
/// pass through every language touched by this round completely unchanged. Without this, a
/// sweep that accidentally escaped everything (an inverted guard, a wrong position) would
/// still pass every positive test above.
#[test]
fn test_generic_reserved_word_sweep_leaves_ordinary_names_untouched() {
    for lang in [
        Language::Python,
        Language::Ruby,
        Language::Elixir,
        Language::R,
        Language::Go,
        Language::Csharp,
        Language::Kotlin,
        Language::KotlinAndroid,
        Language::Swift,
        Language::Dart,
        Language::Gleam,
        Language::Zig,
    ] {
        let name = crate::docs::naming::func_name("classify_link", lang, TEST_PREFIX);
        assert!(!name.ends_with('_'), "{lang:?}: {name}");
    }
}
