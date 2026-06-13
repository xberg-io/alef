use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Lifecycle hooks
// ---------------------------------------------------------------------------

/// A lifecycle hook contract — a named callback slot registered on the service owner.
///
/// The host-language surface generated from this entry is an `app.on_<name>(fn)` style
/// registration method (or its language-idiomatic equivalent) that binds a user-supplied
/// callback into the request-processing pipeline.
///
/// Backends emit the hook registration method alongside the service entrypoints; the
/// generated wrapper stores the callback and the native layer invokes it at the
/// appropriate pipeline phase.
///
/// # alef.toml example
///
/// ```toml
/// [[crates.lifecycle_hooks]]
/// name = "on_request"
/// callback_contract = "RequestHook"
/// doc = "Called before any other processing for each inbound request."
///
/// [[crates.lifecycle_hooks]]
/// name = "on_error"
/// callback_contract = "ErrorHook"
/// doc = "Called when a handler returns an error."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleHookDef {
    /// Canonical hook name used to derive the registration method name.
    ///
    /// Backends prepend the language-idiomatic registration verb:
    /// - `"on_request"` → Python `app.on_request(fn)`, TypeScript `app.onRequest(fn)`,
    ///   Ruby `app.on_request { |req| … }`, Elixir `App.on_request(app, fn)`.
    pub name: String,
    /// Name of the callback contract (trait/interface) that the hook function must satisfy.
    ///
    /// References a [`crate::core::ir::HandlerContractDef::trait_name`] entry in the
    /// same [`crate::core::ir::ApiSurface`].
    /// Backends use this to generate the correct type annotation for the callback parameter.
    pub callback_contract: String,
    /// Documentation extracted from the hook definition.
    #[serde(default)]
    pub doc: String,
    /// Whether the hook callback is invoked asynchronously.
    ///
    /// When `true`, backends in async-first languages (Python/asyncio, TypeScript/Node,
    /// Kotlin coroutines) emit `async` callback types and `await` the hook result.
    /// Defaults to `false` for synchronous hooks.
    #[serde(default)]
    pub is_async: bool,
}

// ---------------------------------------------------------------------------
// WebSocket / SSE first-class contracts
// ---------------------------------------------------------------------------

/// A WebSocket route registration contract.
///
/// Backends generate an `app.websocket(path, handler_fn)` method (or its idiomatic
/// equivalent) from this entry. Because `WebSocketHandler` uses `impl Future` return
/// types that are not object-safe, the IR uses a concrete wrapper struct name
/// (`handler_wrapper_type`) instead of a trait object so backends can emit a concrete
/// monomorphisation rather than an `Arc<dyn Trait>` bridge.
///
/// # alef.toml example
///
/// ```toml
/// [[crates.websocket_routes]]
/// handler_wrapper_type = "WebSocketHandlerWrapper"
/// socket_type = "WebSocketConnection"
/// doc = "Register a WebSocket upgrade handler at the given path."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketRouteDef {
    /// Name of the concrete Rust wrapper struct that the binding layer must instantiate
    /// to pass a host-language callable into the native router (e.g. `"WebSocketHandlerWrapper"`).
    ///
    /// This is a concrete type — not a trait object — so backends do not need to
    /// produce `Arc<dyn WebSocketHandler>` (which would require the trait to be object-safe).
    pub handler_wrapper_type: String,
    /// Name of the WebSocket connection type passed to the handler on every connection
    /// (e.g. `"WebSocketConnection"`). Backends use this to generate typed socket
    /// parameter annotations in the handler signature.
    pub socket_type: String,
    /// Documentation for the generated `app.websocket(...)` method.
    #[serde(default)]
    pub doc: String,
}

/// An SSE (Server-Sent Events) route registration contract.
///
/// Backends generate an `app.sse(path, producer_fn)` method (or its idiomatic
/// equivalent) from this entry. Like WebSocket handlers, SSE producers use RPITIT
/// return types (`impl AsyncIterator`) that are not object-safe; the IR names the
/// concrete wrapper type so backends emit a concrete monomorphisation.
///
/// # alef.toml example
///
/// ```toml
/// [[crates.sse_routes]]
/// producer_wrapper_type = "SseProducerWrapper"
/// event_type = "SseEvent"
/// doc = "Register an SSE event producer at the given path."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseRouteDef {
    /// Name of the concrete Rust wrapper struct that the binding layer instantiates
    /// to pass a host-language producer callable into the native router
    /// (e.g. `"SseProducerWrapper"`).
    ///
    /// Concrete wrapper rather than `Arc<dyn SseProducer>` because the producer
    /// returns `impl AsyncIterator`, which is not dyn-compatible in Rust.
    pub producer_wrapper_type: String,
    /// Name of the SSE event type yielded by the producer (e.g. `"SseEvent"`).
    ///
    /// Backends use this to generate typed annotations for the items the producer
    /// function yields.
    pub event_type: String,
    /// Documentation for the generated `app.sse(...)` method.
    #[serde(default)]
    pub doc: String,
}

// ---------------------------------------------------------------------------
// Cross-binding error types
// ---------------------------------------------------------------------------

/// HTTP status code classification for a [`ErrorTypeDef`].
///
/// Each variant maps to one of the most common HTTP error codes and carries the
/// corresponding numeric code for backends that need to emit a `status_code()` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpStatus {
    /// 400 Bad Request.
    BadRequest,
    /// 401 Unauthorized.
    Unauthorized,
    /// 403 Forbidden.
    Forbidden,
    /// 404 Not Found.
    NotFound,
    /// 409 Conflict.
    Conflict,
    /// 422 Unprocessable Entity (validation failure, RFC 9457 ProblemDetails).
    UnprocessableEntity,
    /// 429 Too Many Requests.
    TooManyRequests,
    /// 500 Internal Server Error.
    InternalServerError,
    /// An explicit numeric status code not covered by the named variants above.
    Custom(u16),
}

impl HttpStatus {
    /// Returns the numeric HTTP status code.
    pub fn as_u16(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::UnprocessableEntity => 422,
            Self::TooManyRequests => 429,
            Self::InternalServerError => 500,
            Self::Custom(code) => code,
        }
    }
}

/// A cross-binding error type emitted as a native exception class in every language.
///
/// Each entry describes one member of the exception hierarchy that every backend must
/// emit. Backends produce native exception/error classes whose `status_code()` method
/// returns [`ErrorTypeDef::http_status`] and whose serialization produces an RFC 9457
/// ProblemDetails JSON body.
///
/// # alef.toml example
///
/// ```toml
/// [[crates.error_types]]
/// name = "NotFoundError"
/// http_status = "not_found"
/// doc = "Raised when the requested resource does not exist."
///
/// [[crates.error_types]]
/// name = "ValidationError"
/// http_status = "unprocessable_entity"
/// problem_details_type = "https://example.com/problems/validation"
/// doc = "Raised when input validation fails. Carries a list of field errors."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorTypeDef {
    /// PascalCase error class name emitted in every binding language
    /// (e.g. `"NotFoundError"`, `"ValidationError"`).
    pub name: String,
    /// The HTTP status code this error maps to in the response.
    pub http_status: HttpStatus,
    /// Optional RFC 9457 ProblemDetails `type` URI for this error class.
    ///
    /// When present, backends include `type = "<uri>"` in the serialized ProblemDetails
    /// body. When absent, backends emit a generic type URI derived from the error name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem_details_type: Option<String>,
    /// Documentation for the generated error class.
    #[serde(default)]
    pub doc: String,
}
