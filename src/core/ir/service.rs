use serde::{Deserialize, Serialize};

use super::items::{MethodDef, ParamDef, ReceiverKind};
use super::type_ref::TypeRef;

/// Describes an owner/builder type that acts as a service configurator and runner.
///
/// A service has:
/// - A `constructor` that creates the initial owner instance.
/// - Zero or more `configurators` — chaining methods that set options without callbacks.
/// - Zero or more `registrations` — methods that bind a callback to a route/channel/slot.
/// - One or more `entrypoints` — `run` (long-lived async) or `finalize` (consuming transform).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDef {
    /// Short name of the owner type (e.g. `"App"`).
    pub name: String,
    /// Fully-qualified Rust path to the owner type (e.g. `"my_crate::App"`).
    pub rust_path: String,
    /// The constructor method (e.g. `new`).
    pub constructor: MethodDef,
    /// Chaining methods that mutate configuration but take no callback.
    pub configurators: Vec<MethodDef>,
    /// Registration methods that accept a callback and optional metadata.
    pub registrations: Vec<RegistrationDef>,
    /// Long-lived run entrypoints or transforming finalize methods.
    pub entrypoints: Vec<EntrypointDef>,
    /// Documentation extracted from the owner type.
    pub doc: String,
    /// Optional `#[cfg(...)]` condition string.
    pub cfg: Option<String>,
}

/// A parsed path-segment constraint extracted from route path strings.
///
/// Backends use this to emit idiomatic typed path-parameter bindings and router
/// constraint annotations. The normalization pass converts all input syntaxes
/// (`{name:type}`, `{name}`, `:name`) to the canonical axum form `{name}` in the
/// emitted router path, while retaining constraint metadata here for per-backend use.
///
/// # Supported type constraints
///
/// | `type_constraint` value | Semantics |
/// |---|---|
/// | `"int"` | Parse path segment as a signed integer (i64 on 64-bit platforms). |
/// | `"uuid"` | Validate UUID v4 format (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`). |
/// | `"slug"` | Alphanumeric + hyphens/underscores only; no spaces. |
/// | `"path"` | Greedy — captures the remainder of the path including `/`. |
/// | custom regex | Any other value is treated as a verbatim pattern backends may expose as documentation. |
/// | `None` | No constraint — any non-empty segment matches. |
///
/// Backends that do not support a particular constraint may ignore `type_constraint`
/// and emit a plain parameter binding; validation then happens in the Rust core layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ParameterConstraint {
    /// The path-parameter name after normalization (e.g. `"id"`, `"slug"`).
    pub name: String,
    /// Optional type constraint for this parameter (e.g. `"int"`, `"uuid"`).
    /// `None` means no structural constraint beyond matching a non-empty path segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_constraint: Option<String>,
}

impl ParameterConstraint {
    /// Parse path-parameter constraints from a route path string.
    ///
    /// Accepts three input syntaxes and normalizes to the canonical axum form in the
    /// returned path string. Returns (normalized_path, constraints).
    ///
    /// | Input syntax | Example | Normalized output |
    /// |---|---|---|
    /// | Named with type | `{id:int}` | `{id}` |
    /// | Named without type | `{id}` | `{id}` |
    /// | Colon prefix (Express/Sinatra style) | `:id` | `{id}` |
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::ParameterConstraint;
    ///
    /// let (path, constraints) = ParameterConstraint::parse_parameters("/users/{id:int}/posts/{slug}");
    /// assert_eq!(path, "/users/{id}/posts/{slug}");
    /// assert_eq!(constraints[0].name, "id");
    /// assert_eq!(constraints[0].type_constraint.as_deref(), Some("int"));
    /// assert_eq!(constraints[1].name, "slug");
    /// assert!(constraints[1].type_constraint.is_none());
    /// ```
    pub fn parse_parameters(path: &str) -> (String, Vec<ParameterConstraint>) {
        let mut constraints = Vec::new();
        let mut normalized = String::with_capacity(path.len());
        let mut chars = path.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    inner.push(c);
                }
                let (name, type_constraint) = if let Some(colon_pos) = inner.find(':') {
                    let n = inner[..colon_pos].trim().to_owned();
                    let t = inner[colon_pos + 1..].trim().to_owned();
                    (n, if t.is_empty() { None } else { Some(t) })
                } else {
                    (inner.trim().to_owned(), None)
                };
                if !name.is_empty() {
                    constraints.push(ParameterConstraint {
                        name: name.clone(),
                        type_constraint,
                    });
                    normalized.push('{');
                    normalized.push_str(&name);
                    normalized.push('}');
                }
            } else if ch == ':' && chars.peek().is_some_and(|c| c.is_alphabetic() || *c == '_') {
                let mut name = String::new();
                while chars.peek().is_some_and(|c| c.is_alphanumeric() || *c == '_') {
                    name.push(chars.next().unwrap());
                }
                if !name.is_empty() {
                    constraints.push(ParameterConstraint {
                        name: name.clone(),
                        type_constraint: None,
                    });
                    normalized.push('{');
                    normalized.push_str(&name);
                    normalized.push('}');
                }
            } else {
                normalized.push(ch);
            }
        }

        (normalized, constraints)
    }
}

/// How the handler callable receives the request in a registration variant.
///
/// This controls the shape of the generated wrapper that binds an incoming request
/// to the host-language callback registered on a route or channel. Backends dispatch
/// on this enum when selecting which template or code-generation path to use for the
/// handler body.
///
/// The default is [`HandlerShape::BareCallable`], which preserves the pre-existing
/// behaviour for consumers that have not declared an explicit handler shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HandlerShape {
    /// The handler is a plain callable that receives the full request DTO directly.
    ///
    /// This is the default and matches the pre-existing codegen behaviour across all
    /// backends: `fn handler(req: RequestData) -> Response`.
    #[default]
    BareCallable,
    /// The handler receives a single opaque context object that bundles request and
    /// response into one ergonomic handle.
    ///
    /// Idiomatic in: Hono (TypeScript `c`), Vapor (Swift `req`), Javalin (Java `ctx`),
    /// ASP.NET Minimal API (C# `HttpContext`). The context object exposes typed accessors
    /// for path parameters, query, headers, body, and response helpers.
    ContextObject,
    /// The handler receives separate request and response objects as two arguments.
    ///
    /// Idiomatic in: Express (`(req, res) => void`), chi Go
    /// (`http.HandlerFunc(w http.ResponseWriter, r *http.Request)`). The response object
    /// is written to rather than returned.
    RequestResponse,
    /// The handler is a function whose parameter list is introspected at registration
    /// time to bind path parameters, query parameters, and body fields by type annotation.
    ///
    /// Idiomatic in: FastAPI / Litestar (Python) where `async def h(id: int, q: str) -> Model`
    /// causes the framework to extract `id` from path, `q` from query, and inject them by
    /// name. Backends implementing this shape emit the introspection adapter at registration
    /// time rather than at call time.
    IntrospectParams,
}

/// A registration method on a service — binds a host-language callback to a slot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistrationDef {
    /// Method name on the owner type (e.g. `"add_route"`).
    pub method: String,
    /// The parameter name that carries the callback (e.g. `"handler"`).
    pub callback_param: String,
    /// Name of the [`HandlerContractDef`] this callback must satisfy
    /// (references [`HandlerContractDef::trait_name`]).
    pub callback_contract: String,
    /// Non-callback parameters (e.g. path pattern, HTTP method).
    pub metadata_params: Vec<ParamDef>,
    /// How `self` is received (`&self`, `&mut self`, or owned).
    pub receiver: Option<ReceiverKind>,
    /// Return type of the registration method.
    pub return_type: TypeRef,
    /// Error type if the registration is fallible.
    pub error_type: Option<String>,
    /// Documentation extracted from the method.
    pub doc: String,
    /// Named shortcuts over this registration with pre-resolved pinned values.
    /// Empty for registrations declared without `[[registrations.variants]]`.
    #[serde(default)]
    pub variants: Vec<RegistrationVariant>,
    /// Parsed path-parameter constraints extracted from the registration's path
    /// metadata parameter at configuration time.
    ///
    /// Populated by the extractor when the registration has a string metadata parameter
    /// whose value carries constraint syntax (e.g. `{id:int}`). Backends use this to
    /// emit typed path-parameter bindings and router-level validation annotations.
    /// Empty when no path string is associated with this registration base.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_param_constraints: Vec<ParameterConstraint>,
    /// How the handler callable receives the incoming request in generated code.
    ///
    /// Backends dispatch on this value to select the appropriate emission template.
    /// Defaults to [`HandlerShape::BareCallable`] for backward compatibility.
    #[serde(default)]
    pub handler_shape: HandlerShape,
}

/// How backends should emit a registration variant's host-language surface.
///
/// This controls whether the variant is exposed as a builder/decorator factory
/// (returning a callable that accepts the handler), a direct verb-style method
/// (accepting the handler inline), or both — or as a platform-idiomatic alternative
/// such as an attribute/annotation scanner or a nested routing DSL.
///
/// The default is [`RegistrationVariantStyle::Hybrid`], which preserves the
/// pre-existing behaviour of emitting both forms so existing consumers are not
/// broken by this IR extension.
///
/// Per-language overrides are expressed in [`RegistrationVariantLanguageOverride`] blocks
/// inside the variant's `alef.toml` entry, which win over the variant-global style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationVariantStyle {
    /// Emit only the builder/decorator-factory form.
    ///
    /// Example: Python `app.get(path)` returns a decorator that can be applied to a
    /// function, or Ruby `app.get(path)` accepts a block as the handler.
    Builder,
    /// Emit only the verb-decorator form — a single method whose last positional
    /// argument is the handler callable.
    ///
    /// Example: Python `app.get(path, handler)`, Flask/FastAPI `@app.route(path)`,
    /// Sinatra-style `app.get(path) { |req| … }`.
    ///
    /// In Python this means a single method that doubles as both the direct form
    /// (`app.get(path, handler)`) and the decorator form (`@app.get(path)`) when
    /// `handler` is `None` — see [`RegistrationVariantStyle::Decorator`] for the
    /// fully-overloaded single-method variant.
    VerbDecorator,
    /// Emit both [`RegistrationVariantStyle::Builder`] and
    /// [`RegistrationVariantStyle::VerbDecorator`] forms.
    ///
    /// This is the default, preserving backward compatibility for consumers that
    /// have not yet declared an explicit `style`.
    #[default]
    Hybrid,
    /// Emit a Python-style overloaded decorator where the same method acts as both
    /// a direct registration (`app.get(path, handler)`) and a decorator factory
    /// (`@app.get(path)`) by returning a callable when the handler argument is absent.
    ///
    /// The generated method signature is approximately:
    /// ```python
    /// def get(self, path: str, handler: Callable | None = None):
    ///     if handler is None:
    ///         def decorator(fn): self._register(path, fn); return fn
    ///         return decorator
    ///     self._register(path, handler)
    ///     return self
    /// ```
    ///
    /// This single method covers both `app.get("/p", h)` and `@app.get("/p")` at the
    /// same call site, matching Flask, FastAPI, and Starlette idioms.
    Decorator,
    /// Emit a class- or method-level annotation/attribute that marks a handler and
    /// an `app.mount(ControllerClass)` scanner that discovers all annotated handlers
    /// via reflection.
    ///
    /// This style generates:
    /// - One annotation/attribute type per HTTP verb (e.g. `#[Get('/path')]` in PHP,
    ///   `@GetMapping("/path")` in Java, `[HttpGet("/path")]` in C#).
    /// - An `app.mount(T)` method that scans the class `T` for annotated methods and
    ///   registers them with the underlying router.
    ///
    /// Backends that do not support reflection (Go, Zig, C FFI) should fall back to
    /// [`RegistrationVariantStyle::Hybrid`] when this style is requested.
    Attribute,
    /// Emit a nested routing DSL where route registrations live inside a closure or
    /// macro block rather than as direct method calls on the owner.
    ///
    /// This style generates a top-level DSL entry point (e.g. a Kotlin `@DslMarker`
    /// function, an Elixir `use Mod` macro, or a Ktor `routing {}` block) whose body
    /// contains the verb registrations.
    ///
    /// Example shapes:
    /// - Kotlin Ktor: `routing { get("/users/{id}") { val id = call.parameters["id"] } }`
    /// - Elixir: `use Router; get "/users/:id" do … end`
    ///
    /// Backends that do not support DSL-style nesting should fall back to
    /// [`RegistrationVariantStyle::Hybrid`].
    Dsl,
}

/// Per-language override for a registration variant's surface style and method naming.
///
/// Each backend looks up its canonical language name in the variant's `language_overrides`
/// map and, when present, uses the override's values instead of the variant-global defaults.
/// Fields absent from the override fall through to the variant-level setting.
///
/// # alef.toml example
///
/// ```toml
/// [[crates.services.registrations.variants]]
/// name = "get"
/// fixed = { method = "GET" }
/// style = "hybrid"
///
/// [crates.services.registrations.variants.languages.csharp]
/// style = "attribute"
/// method_prefix = "Map"   # emits MapGet, MapPost, …
///
/// [crates.services.registrations.variants.languages.kotlin]
/// style = "dsl"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RegistrationVariantLanguageOverride {
    /// Override for the emission style in this language.
    /// When absent, falls through to the variant-global `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<RegistrationVariantStyle>,
    /// Override for the handler shape in this language.
    /// When absent, falls through to the base registration's `handler_shape`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_shape: Option<HandlerShape>,
    /// Language-specific prefix prepended to the verb method name.
    ///
    /// Example: `"Map"` for C# produces `MapGet`, `MapPost`, `MapPut`, … rather than
    /// the default lowercased `get`, `post`, `put`. When absent, no prefix is added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_prefix: Option<String>,
}

/// Resolved surface attributes for one registration variant in the context of a specific backend.
///
/// Produced by [`RegistrationVariant::resolved_for`]. Backends read these three fields
/// instead of accessing `variant.style`, `base_reg.handler_shape`, and manually
/// constructing the method name — the resolver applies per-language overrides consistently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVariant<'a> {
    /// The emission style to use for this backend.
    ///
    /// This is the per-language override's `style` when an entry exists for the
    /// requested language and it sets a style; otherwise the variant-global `style`.
    pub style: RegistrationVariantStyle,
    /// The handler shape to use for this backend.
    ///
    /// This is the per-language override's `handler_shape` when an entry exists and it
    /// sets a shape; otherwise the base registration's `handler_shape`.
    pub handler_shape: HandlerShape,
    /// Optional language-specific prefix prepended to the verb method name.
    ///
    /// Example: `"Map"` for C# produces `MapGet`, `MapPost`, `MapPut`. `None` means no
    /// prefix is added and the variant's own `name` is used verbatim.
    pub method_prefix: Option<&'a str>,
}

/// A named shortcut over a [`RegistrationDef`] with one or more pinned
/// parameter values resolved at extract time.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistrationVariant {
    /// Shortcut name as declared in `alef.toml` (e.g. `"get"`).
    pub name: String,
    /// Resolved overrides — one per pinned base metadata-param.
    pub overrides: Vec<RegistrationVariantOverride>,
    /// When the base registration consumes a wrapper-typed metadata param whose
    /// type carries a static constructor (e.g. `route(builder: RouteBuilder, …)`
    /// where `RouteBuilder::new(method, path)` exists), the extractor pre-builds
    /// the call recipe here. Backends render this directly — composing the
    /// constructor invocation with `Fixed` args substituted in and `Free` args
    /// pulled from the variant's own signature.
    ///
    /// `None` when no metadata param matches the wrapper pattern; in that case
    /// `overrides` targets the base metadata params directly and the variant's
    /// signature is just the non-overridden subset.
    #[serde(default)]
    pub wrapper_call: Option<WrapperConstructorCall>,
    /// The variant's user-facing signature in canonical order: the non-fixed
    /// constructor args (when `wrapper_call` is set) or the non-overridden base
    /// metadata params (otherwise). Each entry carries the host param's name +
    /// type so backends can render them in language-idiomatic shape.
    pub signature_params: Vec<ParamDef>,
    /// Optional documentation for the variant. When absent, backends emit a
    /// generic docstring referencing the base registration.
    #[serde(default)]
    pub doc: Option<String>,
    /// How backends should expose this variant's host-language surface.
    ///
    /// Defaults to [`RegistrationVariantStyle::Hybrid`] so that consumers which
    /// do not declare an explicit `style` in `alef.toml` get the previous
    /// behaviour (both direct-method and decorator-factory forms emitted).
    #[serde(default)]
    pub style: RegistrationVariantStyle,
    /// Per-language style overrides keyed by canonical language name
    /// (e.g. `"csharp"`, `"kotlin"`, `"python"`).
    ///
    /// A backend looks up its name here first; when an entry is found its `style`,
    /// `handler_shape`, and `method_prefix` win over the variant-global values.
    /// Missing keys fall through to the variant-global defaults transparently.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub language_overrides: std::collections::HashMap<String, RegistrationVariantLanguageOverride>,
}

impl RegistrationVariant {
    /// Resolve the effective `(style, handler_shape, method_prefix)` tuple for `language`.
    ///
    /// Looks up `language` in [`Self::language_overrides`]. When an override entry is
    /// found, each of its three optional fields wins over the corresponding variant-level
    /// or registration-level default:
    ///
    /// | field | per-language override | fallback |
    /// |---|---|---|
    /// | `style` | `override.style` | `self.style` |
    /// | `handler_shape` | `override.handler_shape` | `base_handler_shape` |
    /// | `method_prefix` | `override.method_prefix.as_deref()` | `None` |
    ///
    /// The `base_handler_shape` argument is the `handler_shape` field on the parent
    /// [`RegistrationDef`] — pass it as `&reg.handler_shape`.
    ///
    /// # Examples
    ///
    /// ```
    /// use alef::core::ir::{
    ///     HandlerShape, RegistrationVariant, RegistrationVariantLanguageOverride,
    ///     RegistrationVariantStyle,
    /// };
    /// use std::collections::HashMap;
    ///
    /// let mut overrides = HashMap::new();
    /// overrides.insert(
    ///     "csharp".to_owned(),
    ///     RegistrationVariantLanguageOverride {
    ///         style: Some(RegistrationVariantStyle::Attribute),
    ///         handler_shape: None,
    ///         method_prefix: Some("Map".to_owned()),
    ///     },
    /// );
    /// let variant = RegistrationVariant {
    ///     name: "get".to_owned(),
    ///     style: RegistrationVariantStyle::Hybrid,
    ///     language_overrides: overrides,
    ///     ..RegistrationVariant::default()
    /// };
    ///
    /// let resolved = variant.resolved_for("csharp", HandlerShape::BareCallable);
    /// assert_eq!(resolved.style, RegistrationVariantStyle::Attribute);
    /// assert_eq!(resolved.handler_shape, HandlerShape::BareCallable);
    /// assert_eq!(resolved.method_prefix, Some("Map"));
    ///
    /// // Language with no override falls back to variant-level defaults.
    /// let resolved = variant.resolved_for("python", HandlerShape::IntrospectParams);
    /// assert_eq!(resolved.style, RegistrationVariantStyle::Hybrid);
    /// assert_eq!(resolved.handler_shape, HandlerShape::IntrospectParams);
    /// assert_eq!(resolved.method_prefix, None);
    /// ```
    pub fn resolved_for(&self, language: &str, base_handler_shape: HandlerShape) -> ResolvedVariant<'_> {
        if let Some(lang_override) = self.language_overrides.get(language) {
            ResolvedVariant {
                style: lang_override.style.unwrap_or(self.style),
                handler_shape: lang_override.handler_shape.unwrap_or(base_handler_shape),
                method_prefix: lang_override.method_prefix.as_deref(),
            }
        } else {
            ResolvedVariant {
                style: self.style,
                handler_shape: base_handler_shape,
                method_prefix: None,
            }
        }
    }

    /// Return the idiomatic method name for this variant in `language`.
    ///
    /// When the language override declares a `method_prefix`, it is prepended to
    /// [`Self::name`] (e.g. `prefix="Map"` + `name="get"` → `"Mapget"`). Callers are
    /// responsible for applying language-specific casing on top (e.g. `to_upper_camel_case`
    /// for the Go/Java/C# convention of capitalising the verb: `"MapGet"`).
    ///
    /// When no prefix is configured the variant name is returned as-is.
    pub fn method_name_for<'a>(&'a self, language: &str) -> std::borrow::Cow<'a, str> {
        if let Some(lang_override) = self.language_overrides.get(language) {
            if let Some(prefix) = &lang_override.method_prefix {
                return std::borrow::Cow::Owned(format!("{}{}", prefix, self.name));
            }
        }
        std::borrow::Cow::Borrowed(&self.name)
    }
}

/// A resolved pin: the param being overridden and the expression to substitute
/// at the call site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationVariantOverride {
    /// Name of the param this override pins. When `wrapper_call` is set on the
    /// parent variant this matches a wrapper-constructor param; otherwise it
    /// matches a base [`RegistrationDef::metadata_params`] entry.
    pub param_name: String,
    /// Verbatim expression substituted for the param at the call site. For
    /// enum overrides this is the fully-qualified Rust path
    /// (e.g. `"my_crate::Method::GET"`); for other types it is the raw value
    /// expression supplied by the library author.
    pub value_expr: String,
}

/// Pre-resolved recipe for building a wrapper metadata-param value via a
/// static constructor with a mix of pinned and free arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapperConstructorCall {
    /// Name of the base metadata param the constructor result is bound to
    /// (e.g. `"builder"`).
    pub metadata_param: String,
    /// Fully-qualified Rust path of the wrapper type
    /// (e.g. `"my_crate::RouteBuilder"`).
    pub wrapper_type_path: String,
    /// Bare type name of the wrapper (e.g. `"RouteBuilder"`).
    pub wrapper_type_name: String,
    /// Constructor method name on the wrapper type (e.g. `"new"`).
    pub constructor_method: String,
    /// Constructor args in declared order. Each is either fixed (with a value
    /// expression substituted in) or free (taken from the variant's signature).
    pub args: Vec<WrapperConstructorArg>,
}

/// One argument in a [`WrapperConstructorCall`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WrapperConstructorArg {
    /// Pinned at the call site to `value_expr`.
    Fixed {
        /// Name of the wrapper constructor param (e.g. `"method"`).
        param_name: String,
        /// Verbatim expression substituted at the call site.
        value_expr: String,
    },
    /// Pulled from the variant's own signature at the call site.
    Free {
        /// Definition of the free param — name + type drive both the variant's
        /// signature and the constructor call argument position.
        param: ParamDef,
    },
}

/// An entrypoint on a service — either a long-lived async `run` or a consuming `finalize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrypointDef {
    /// Method name (e.g. `"run"` or `"into_router"`).
    pub method: String,
    /// Whether this is a blocking runner or a transforming finalizer.
    pub kind: EntrypointKind,
    /// True when the method is `async` or returns a `Future`.
    pub is_async: bool,
    /// Non-self parameters accepted by the entrypoint (e.g. socket address).
    pub params: Vec<ParamDef>,
    /// Return type (e.g. `()` for `run`, `Router` for `finalize`).
    pub return_type: TypeRef,
    /// Error type if the entrypoint is fallible.
    pub error_type: Option<String>,
    /// Documentation extracted from the method.
    pub doc: String,
}

/// Discriminates between the two kinds of service entrypoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointKind {
    /// A long-running async method that drives the service until shutdown.
    Run,
    /// A consuming transform that converts the builder into another type (e.g. a router).
    Finalize,
}

/// An async trait that registered service callbacks must satisfy.
///
/// Does **not** duplicate the trait's [`crate::core::ir::TypeDef`] already in
/// [`crate::core::ir::ApiSurface::types`]; instead it cross-references by name and adds service-specific metadata:
/// the dispatch method, optional overrides, and the wire DTO names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerContractDef {
    /// Trait name as it appears in the surface (e.g. `"Handler"`).
    pub trait_name: String,
    /// Fully-qualified Rust path (e.g. `"my_crate::Handler"`).
    pub rust_path: String,
    /// The primary async dispatch method backends must implement.
    pub dispatch: MethodDef,
    /// Methods with default implementations that backends may optionally override.
    pub optional_methods: Vec<MethodDef>,
    /// Name of the wire request DTO the dispatch method receives (e.g. `"RequestData"`).
    /// When `None`, the dispatch method signature is used verbatim.
    pub wire_request_type: Option<String>,
    /// Name of the wire response DTO the dispatch method returns (e.g. `"ResponseData"`).
    /// When `None`, the dispatch method return type is used verbatim.
    pub wire_response_type: Option<String>,
    /// Verbatim parameter declarations inserted before the wire request parameter in the
    /// generated dispatch signature and ignored in the body. Empty by default.
    pub dispatch_extra_params: Vec<String>,
    /// Name of the wire request parameter in the generated dispatch signature
    /// (defaults to `"request"` when `None`).
    pub wire_param_name: Option<String>,
    /// Verbatim return type for the generated dispatch future's `Output`. When `None`,
    /// the bridge synthesizes `Result<{wire_response}, Box<dyn Error + Send + Sync>>`.
    pub dispatch_return_type: Option<String>,
    /// Path to a library function converting the bridge outcome into
    /// [`Self::dispatch_return_type`]. When `None`, the wire response is returned directly.
    pub response_adapter: Option<String>,
    /// Documentation extracted from the trait.
    pub doc: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_variant(style: RegistrationVariantStyle) -> RegistrationVariant {
        RegistrationVariant {
            name: "get".to_owned(),
            style,
            ..RegistrationVariant::default()
        }
    }

    #[test]
    fn resolved_for_no_override_returns_variant_defaults() {
        let variant = make_variant(RegistrationVariantStyle::VerbDecorator);
        let resolved = variant.resolved_for("python", HandlerShape::BareCallable);
        assert_eq!(resolved.style, RegistrationVariantStyle::VerbDecorator);
        assert_eq!(resolved.handler_shape, HandlerShape::BareCallable);
        assert_eq!(resolved.method_prefix, None);
    }

    #[test]
    fn resolved_for_no_override_propagates_base_handler_shape() {
        let variant = make_variant(RegistrationVariantStyle::Hybrid);
        let resolved = variant.resolved_for("napi", HandlerShape::ContextObject);
        assert_eq!(resolved.handler_shape, HandlerShape::ContextObject);
        assert_eq!(resolved.style, RegistrationVariantStyle::Hybrid);
    }

    #[test]
    fn resolved_for_language_override_wins_over_variant_style() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "csharp".to_owned(),
            RegistrationVariantLanguageOverride {
                style: Some(RegistrationVariantStyle::Attribute),
                handler_shape: None,
                method_prefix: Some("Map".to_owned()),
            },
        );
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            style: RegistrationVariantStyle::Hybrid,
            language_overrides: overrides,
            ..RegistrationVariant::default()
        };

        let resolved = variant.resolved_for("csharp", HandlerShape::BareCallable);
        assert_eq!(
            resolved.style,
            RegistrationVariantStyle::Attribute,
            "override style should win"
        );
        assert_eq!(
            resolved.handler_shape,
            HandlerShape::BareCallable,
            "base shape should be kept"
        );
        assert_eq!(resolved.method_prefix, Some("Map"), "prefix from override");
    }

    #[test]
    fn resolved_for_language_override_handler_shape_wins() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "kotlin".to_owned(),
            RegistrationVariantLanguageOverride {
                style: Some(RegistrationVariantStyle::Dsl),
                handler_shape: Some(HandlerShape::ContextObject),
                method_prefix: None,
            },
        );
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            style: RegistrationVariantStyle::Builder,
            language_overrides: overrides,
            ..RegistrationVariant::default()
        };

        let resolved = variant.resolved_for("kotlin", HandlerShape::BareCallable);
        assert_eq!(resolved.style, RegistrationVariantStyle::Dsl);
        assert_eq!(
            resolved.handler_shape,
            HandlerShape::ContextObject,
            "override shape should win"
        );
        assert_eq!(resolved.method_prefix, None);
    }

    #[test]
    fn resolved_for_unrelated_language_falls_back_to_defaults() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "csharp".to_owned(),
            RegistrationVariantLanguageOverride {
                style: Some(RegistrationVariantStyle::Attribute),
                handler_shape: Some(HandlerShape::ContextObject),
                method_prefix: Some("Map".to_owned()),
            },
        );
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            style: RegistrationVariantStyle::Hybrid,
            language_overrides: overrides,
            ..RegistrationVariant::default()
        };

        let resolved = variant.resolved_for("python", HandlerShape::IntrospectParams);
        assert_eq!(resolved.style, RegistrationVariantStyle::Hybrid);
        assert_eq!(resolved.handler_shape, HandlerShape::IntrospectParams);
        assert_eq!(resolved.method_prefix, None);
    }

    #[test]
    fn resolved_for_partial_override_only_style_no_shape() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "python".to_owned(),
            RegistrationVariantLanguageOverride {
                style: Some(RegistrationVariantStyle::Decorator),
                handler_shape: None,
                method_prefix: None,
            },
        );
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            style: RegistrationVariantStyle::Hybrid,
            language_overrides: overrides,
            ..RegistrationVariant::default()
        };

        let resolved = variant.resolved_for("python", HandlerShape::RequestResponse);
        assert_eq!(
            resolved.style,
            RegistrationVariantStyle::Decorator,
            "style override applies"
        );
        assert_eq!(
            resolved.handler_shape,
            HandlerShape::RequestResponse,
            "shape falls back to base"
        );
    }

    #[test]
    fn method_name_for_no_override_returns_variant_name() {
        let variant = make_variant(RegistrationVariantStyle::Hybrid);
        assert_eq!(variant.method_name_for("go").as_ref(), "get");
    }

    #[test]
    fn method_name_for_prefix_override_prepends_prefix() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "csharp".to_owned(),
            RegistrationVariantLanguageOverride {
                style: None,
                handler_shape: None,
                method_prefix: Some("Map".to_owned()),
            },
        );
        let variant = RegistrationVariant {
            name: "get".to_owned(),
            style: RegistrationVariantStyle::Hybrid,
            language_overrides: overrides,
            ..RegistrationVariant::default()
        };
        assert_eq!(variant.method_name_for("csharp").as_ref(), "Mapget");
    }
}
