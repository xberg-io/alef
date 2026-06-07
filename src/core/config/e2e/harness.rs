use super::defaults::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Route registration call form for language-specific idioms in e2e harness code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteCallForm {
    /// Direct 2-arg: app.route(builder, handler) — PHP reference, C#, Go, Node, TypeScript, etc.
    Direct,
    /// Decorator 1-arg returning callable: app.route(builder)(handler) — Python verb-decorator
    Decorator,
    /// Block form: app.route(builder) { |req| handler.call(req) } — Ruby blocks
    Block,
}
/// Per-language harness config overrides.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HarnessOverride {
    /// Method to register handlers (overrides HarnessConfig.register_method)
    #[serde(default)]
    pub register_method: Option<String>,
    /// SUT app class name (overrides HarnessConfig.app_class)
    #[serde(default)]
    pub app_class: Option<String>,
    /// Method/field on RouteBuilder to set request body schema
    #[serde(default)]
    pub body_schema_setter: Option<String>,
    /// HTTP method enum/type name
    #[serde(default)]
    pub method_enum: Option<String>,
    /// Serve entrypoint method name (overrides HarnessConfig.run_method)
    #[serde(default)]
    pub run_method: Option<String>,
    /// Modules/packages to import the SUT app from (overrides HarnessConfig.imports)
    #[serde(default)]
    pub imports: Option<Vec<String>>,
    /// Expression to instantiate the default ServerConfig (overrides HarnessConfig.server_config_factory)
    #[serde(default)]
    pub server_config_factory: Option<String>,
}

/// Server-shaped e2e harness configuration for HTTP fixtures.
///
/// When HTTP fixtures are present (server-pattern testing), alef generates
/// a harness script that starts the SUT app, registers handlers per fixture,
/// and serves requests. This config provides the language-agnostic knobs that
/// control harness code generation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HarnessConfig {
    /// Module/package to import the SUT app from (e.g., "my_app")
    #[serde(default)]
    pub imports: Vec<String>,
    /// SUT app class name (e.g., "App")
    #[serde(default)]
    pub app_class: Option<String>,
    /// Method to register handlers on the app (e.g., "route" or "get"/"post"/etc.)
    #[serde(default)]
    pub register_method: Option<String>,
    /// Method/field on RouteBuilder to set request body schema (e.g., "request_schema_json")
    #[serde(default)]
    pub body_schema_setter: Option<String>,
    /// RouteBuilder class name (if distinct from return type of register_method)
    #[serde(default)]
    pub route_builder: Option<String>,
    /// HTTP method enum/type name (e.g., "Method")
    #[serde(default)]
    pub method_enum: Option<String>,
    /// Serve entrypoint method name (e.g., "run")
    #[serde(default)]
    pub run_method: Option<String>,
    /// Field name in the handler's returned response object that carries the
    /// response body payload (e.g., "body" or "content"). Defaults to "body".
    /// Configure to match the deserialization shape of the SUT's Response type.
    #[serde(default = "default_response_body_field")]
    pub response_body_field: String,
    /// Default host for SUT binding (e.g., "127.0.0.1")
    #[serde(default = "default_harness_host")]
    pub host: String,
    /// Default port for SUT binding (e.g., 8000)
    #[serde(default = "default_harness_port")]
    pub port: u16,
    /// Per-language harness overrides
    #[serde(default)]
    pub overrides: HashMap<String, HarnessOverride>,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            imports: Vec::new(),
            app_class: None,
            register_method: None,
            body_schema_setter: None,
            route_builder: None,
            method_enum: None,
            run_method: None,
            response_body_field: default_response_body_field(),
            host: default_harness_host(),
            port: default_harness_port(),
            overrides: HashMap::new(),
        }
    }
}

impl HarnessConfig {
    /// Get the effective register_method for a language, applying language-specific overrides.
    pub fn register_method_for_lang(&self, lang: &str) -> Option<String> {
        self.overrides
            .get(lang)
            .and_then(|o| o.register_method.clone())
            .or_else(|| self.register_method.clone())
    }

    /// Get the effective run_method for a language, applying language-specific overrides.
    pub fn run_method_for_lang(&self, lang: &str) -> Option<String> {
        self.overrides
            .get(lang)
            .and_then(|o| o.run_method.clone())
            .or_else(|| self.run_method.clone())
    }

    /// Get the effective app_class for a language, applying language-specific overrides.
    pub fn app_class_for_lang(&self, lang: &str) -> Option<String> {
        self.overrides
            .get(lang)
            .and_then(|o| o.app_class.clone())
            .or_else(|| self.app_class.clone())
    }

    /// Get the effective imports for a language, applying language-specific overrides.
    pub fn imports_for_lang(&self, lang: &str) -> Vec<String> {
        self.overrides
            .get(lang)
            .and_then(|o| o.imports.clone())
            .unwrap_or_else(|| self.imports.clone())
    }

    /// Get the ServerConfig factory expression for a language, applying language-specific overrides.
    /// Returns a code expression that instantiates a default ServerConfig.
    /// Backend-specific defaults: "node" → "serverConfigDefault()", "wasm" → "new WasmServerConfig()".
    /// Falls back to "new ServerConfig()" for other languages.
    pub fn server_config_factory_for_lang(&self, lang: &str) -> String {
        self.overrides
            .get(lang)
            .and_then(|o| o.server_config_factory.clone())
            .unwrap_or_else(|| match lang {
                "node" => "serverConfigDefault()".to_string(),
                "wasm" => "new WasmServerConfig()".to_string(),
                _ => "new ServerConfig()".to_string(),
            })
    }

    /// Get the import name for the ServerConfig factory, if the factory is a bare identifier.
    /// Returns the identifier to include in the import destructure when the factory
    /// expression is a function/class that must be imported.
    /// - "node" → Some("serverConfigDefault")
    /// - "wasm" → Some("WasmServerConfig")
    /// - others → None (assumes available in scope or uses `new ClassName()`)
    pub fn server_config_factory_import_for_lang(&self, lang: &str) -> Option<String> {
        match lang {
            "node" => Some("serverConfigDefault".to_string()),
            "wasm" => Some("WasmServerConfig".to_string()),
            _ => None,
        }
    }

    /// Get the route registration call form for a language (Direct 2-arg, Decorator 1-arg, or Block form).
    /// Determines how app.route(builder, handler) is emitted for language-specific idioms:
    /// - Direct: app.route(builder, handler) [PHP reference, most others]
    /// - Decorator: app.route(builder)(handler) [Python verb-decorator]
    /// - Block: app.route(builder) { |req| handler.call(req) } [Ruby block]
    pub fn harness_route_call_form_for_lang(&self, lang: &str) -> RouteCallForm {
        match lang {
            "python" => RouteCallForm::Decorator,
            "ruby" => RouteCallForm::Block,
            _ => RouteCallForm::Direct,
        }
    }

    /// Get the import style for a language's TypeScript harness.
    /// - "named" for wasm (wasm-bindgen emits only named exports)
    /// - "default" for node/typescript (Node.js and TypeScript expect default imports)
    pub fn import_style_for_lang(&self, lang: &str) -> &'static str {
        match lang {
            "wasm" => "named",
            _ => "default",
        }
    }

    /// Get the register_method for `lang` rendered in the language's idiomatic
    /// identifier case.
    ///
    /// The canonical name in `[crates.e2e.harness] register_method` (and any
    /// per-language override under `[crates.e2e.harness.overrides.<lang>]`) is
    /// stored verbatim. Each language has its own identifier convention:
    ///
    /// - **snake_case** (python, ruby, elixir, rust, php) — leave verbatim;
    ///   PHP's PSR-1 prefers camelCase but historic snake_case method names
    ///   remain idiomatic for binding wrappers.
    /// - **camelCase** (typescript / node, javascript, dart, swift, kotlin,
    ///   java) — convert `register_route` → `registerRoute`.
    /// - **PascalCase** (csharp, go) — convert `register_route` →
    ///   `RegisterRoute`. Go exports require an upper-case leading character;
    ///   C# methods are PascalCase by convention.
    ///
    /// Returns `None` when neither the per-language override nor the top-level
    /// `register_method` is set.
    pub fn register_method_idiomatic(&self, lang: &str) -> Option<String> {
        self.register_method_for_lang(lang)
            .map(|name| idiomatic_identifier(&name, lang))
    }
}

/// Convert `name` (typically snake_case) into the identifier case idiomatic
/// for `lang`. Single-word names round-trip unchanged (e.g. `route` stays
/// `route` in every language).
fn idiomatic_identifier(name: &str, lang: &str) -> String {
    use heck::{ToLowerCamelCase, ToUpperCamelCase};

    match lang {
        // Snake-case-native languages: leave as-is. PHP's PSR-1 prefers
        // camelCase for method names, but binding consumers (and the PHP
        // service-API codegen itself) historically emit snake_case methods,
        // so retain the canonical form here.
        "python" | "ruby" | "elixir" | "rust" | "php" => name.to_string(),
        // camelCase languages.
        "typescript" | "node" | "wasm" | "javascript" | "dart" | "swift" | "kotlin" | "kotlin-android" | "java" => {
            name.to_lower_camel_case()
        }
        // PascalCase languages.
        "csharp" | "go" => name.to_upper_camel_case(),
        // Unknown language: be conservative and pass through.
        _ => name.to_string(),
    }
}
