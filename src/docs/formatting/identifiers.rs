use super::keyword_tables::*;
use crate::core::config::Language;

/// ~keep Every documented identifier (method/function/type name) must be one the target
/// language's own toolchain would actually accept. A curated or templated name that turns
/// out to collide with a reserved word (Java's `new`, Dart's `new`, ...) fails silently in
/// the worst way: the docs render, look entirely plausible, and only a `javac`/`dart
/// analyze` run against the *real* generated code catches it -- see the tslp `new`
/// vs `create` finding this gate was built to close. Unlike a per-language name fix, this
/// keeps working when whatever mechanism produces a name (a template, a curated override,
/// config) changes to something new that happens to collide again -- it makes the whole
/// class of "wrong reserved-word identifier" impossible to emit silently, not just today's
/// instance of it. Keyword lists are the well-known reserved-word set for each language;
/// where a language's contextual/soft-keyword rules are large (Kotlin, Swift, C#), the
/// list is a conservative subset that includes every word reserved in *every* context, not
/// an exhaustive grammar -- see each arm's word count for how conservative.
///
/// These lists answer "is this word reserved", not "is it usable here": the grammatical
/// position a name occupies is a separate axis, applied by
/// `identifier_violation`/`reserved_words_bind_in_member_position`. Do not prune a word
/// from a list because some position accepts it.
pub(super) fn reserved_words(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => PYTHON_WORDS,
        Language::Node | Language::Wasm => NODE_WORDS,
        Language::Ruby => RUBY_WORDS,
        Language::Php => PHP_WORDS,
        Language::Elixir => ELIXIR_WORDS,
        Language::Go => GO_WORDS,
        // ~keep Delegates to `core::keywords::JAVA_KEYWORDS` rather than carrying a second copy
        // of the same list: the two used to disagree (this table included the reserved
        // *literals* `true`/`false`/`null`, `JAVA_KEYWORDS` did not), which is exactly the
        // "identifier gate catches it, but the code that would trigger the gate never runs
        // through the gate's own table" bug class this module's own docs warn about. Java has
        // no contextual/soft keywords beyond the JLS's fixed reserved-word set, so unlike
        // Kotlin/Swift/C#/Dart below, there is no broader position-aware superset this table
        // needs that the escaping-side table doesn't also need.
        Language::Java => crate::core::keywords::JAVA_KEYWORDS,
        Language::Csharp => CSHARP_WORDS,
        Language::Rust => RUST_WORDS,
        Language::Kotlin | Language::KotlinAndroid => KOTLIN_WORDS,
        Language::Swift => SWIFT_WORDS,
        Language::Dart => DART_WORDS,
        Language::Gleam => GLEAM_WORDS,
        Language::Zig => ZIG_WORDS,
        Language::R => R_WORDS,
        Language::Ffi | Language::C | Language::Jni => FFI_WORDS,
    }
}

/// A conservative "is this well-formed" check: alphabetic-or-underscore start, then
/// alphanumeric-or-underscore. This is a defensive backstop, not the primary rule --
/// every name this pipeline emits is derived from a Rust identifier via heck's case
/// converters, which never introduce illegal characters, so in practice only the
/// reserved-word check below ever fires.
fn has_valid_identifier_syntax(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Where in the target language's grammar a generated name lands.
///
/// A reserved word is only unusable in the slot a grammar treats as a *bare* identifier;
/// several of these languages admit keywords once the name is a member. The docs pipeline
/// emits names into both kinds of slot from the same naming helpers, so a gate that cannot
/// tell them apart has to guess -- and guessing "bare" is what made it reject
/// `static new(version: string): DownloadManager`, a member declaration the napi backend
/// really does emit into `index.d.ts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentifierPosition {
    /// A bare binding name: a top-level `function`/`def`/`fn` declaration, a type or class
    /// heading, a local, a parameter. Every language's reserved words are illegal here.
    Declaration,
    /// A member name: a class or struct method/property declaration, or the name following
    /// a `.` in a member access.
    Member,
}

impl IdentifierPosition {
    /// The word used for this position in diagnostics.
    fn label(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Member => "member",
        }
    }
}

/// Whether `lang`'s reserved words stay unusable when the name occupies member position.
///
/// True for every language except the two whose grammars explicitly free the member slot:
///
/// - **TypeScript / JavaScript** (`Node`, `Wasm`). ES5 dropped ES3's restriction on
///   reserved words after `.` and in object-literal keys; ECMA-262 defines a class
///   element's `ClassElementName` as a `PropertyName`, and `PropertyName` accepts any
///   `IdentifierName` -- keywords included. So `class A { static new() {} }` and `a.new`
///   both parse, and TypeScript inherits the rule. The napi backend depends on it:
///   `to_node_name` (codegen/naming.rs) lower-camel-cases a method name with no keyword
///   escape at all, and the `index.d.ts` it produces really does declare
///   `static new(version: string): DownloadManager`.
/// - **PHP**. The PHP 7.0 "Context Sensitive Lexer" RFC made every reserved word usable as
///   a method name, property name and class-constant name. The PHP backend depends on it
///   too -- its method emitters use a bare `method.name.to_lower_camel_case()` with no
///   keyword escape; the one escape that survives, `escape_php_reserved_constant`
///   (php/gen_bindings/types/enums.rs), covers the class-constant position the RFC left
///   partly reserved, not methods.
///
/// Dart deliberately stays on the strict side. `new` is in Dart's `RESERVED_WORD` set, and
/// a Dart member declaration takes an `identifier`, which excludes reserved words outright
/// -- unlike Dart's separate built-in-identifier set (`get`, `factory`, `library`, ...),
/// which *is* usable as an ordinary name. So `new` is illegal for Dart in either position
/// and the gate must keep saying so.
fn reserved_words_bind_in_member_position(lang: Language) -> bool {
    !matches!(lang, Language::Node | Language::Wasm | Language::Php)
}

/// Returns the reason `name` cannot legally appear at `position` in `lang`, or `None` if it
/// can.
///
/// Syntactic well-formedness is checked in both positions -- no language here accepts an
/// unquoted `1invalid` as a member name any more than as a declaration -- while the
/// reserved-word rule is relaxed exactly where `reserved_words_bind_in_member_position`
/// says the grammar relaxes it.
pub(crate) fn identifier_violation(name: &str, lang: Language, position: IdentifierPosition) -> Option<&'static str> {
    if name.is_empty() {
        return Some("empty identifier");
    }
    let reserved_applies = position == IdentifierPosition::Declaration || reserved_words_bind_in_member_position(lang);
    if reserved_applies && reserved_words(lang).contains(&name) {
        return Some("reserved word");
    }
    if !has_valid_identifier_syntax(name) {
        return Some("not a syntactically valid identifier");
    }
    None
}

/// A generated name the docs pipeline must not emit, and everything needed to find it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentifierViolation {
    pub(crate) name: String,
    pub(crate) lang: Language,
    pub(crate) position: IdentifierPosition,
    pub(crate) reason: &'static str,
    pub(crate) context: String,
}

impl std::fmt::Display for IdentifierViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            name,
            lang,
            position,
            reason,
            context,
        } = self;
        write!(
            f,
            "generated {lang:?} identifier `{name}` is invalid ({reason}) in {} position while rendering {context} \
             -- the docs pipeline must not emit an identifier a {lang:?} toolchain would reject",
            position.label()
        )
    }
}

impl std::error::Error for IdentifierViolation {}

/// The fallible form of the identifier gate: the structured error a caller can inspect,
/// propagate, or render.
pub(crate) fn check_identifier(
    name: &str,
    lang: Language,
    position: IdentifierPosition,
    context: &str,
) -> Result<(), IdentifierViolation> {
    match identifier_violation(name, lang, position) {
        None => Ok(()),
        Some(reason) => Err(IdentifierViolation {
            name: name.to_string(),
            lang,
            position,
            reason,
            context: context.to_string(),
        }),
    }
}

/// Run the gate at a renderer call site and report a violation without aborting.
///
/// This used to `panic!`, on the reasoning that an illegal identifier is a defect in
/// whatever fed the generator the name rather than a recoverable per-document condition.
/// The reasoning held; the failure mode did not. A panic unwound the whole process from
/// deep inside a docs renderer, so a single bad name in one type's method list took down
/// the stages that run after docs (orphan sweep, formatting, hash finalisation) and
/// produced no ERROR line at all -- 31k lines of clean log and a raw backtrace. Docs are
/// not a release gate, so a name the gate rejects must not cost the run everything that
/// already succeeded; it must cost exactly one loud, greppable ERROR event naming the
/// identifier, the language, the position and the render context. Threading a `Result`
/// out instead would reach ~100 call sites across the signature renderers for no extra
/// signal, since no caller between here and the docs stage can do anything but forward
/// it. Callers that *can* act on the failure use [`check_identifier`].
pub(crate) fn report_identifier_violation(name: &str, lang: Language, position: IdentifierPosition, context: &str) {
    if let Err(violation) = super::check_identifier(name, lang, position, context) {
        tracing::error!(
            identifier = %violation.name,
            language = ?violation.lang,
            position = violation.position.label(),
            reason = violation.reason,
            render_context = %violation.context,
            "{}",
            violation
        );
    }
}
