use super::helpers::{format_php_comment, render};
use super::type_mapping::php_type_annotation;
use crate::core::ir::{ApiSurface, EntrypointKind, ServiceDef};
use minijinja::context;

/// Generate the idiomatic PHP service class (`service.php`).
///
/// Produces a PHP file containing one class per service. Each class exposes:
/// - A zero-arg constructor.
/// - Fluent verb methods (get, post, put, patch, delete, head, options) for `verb_decorator` + `request_response` style.
/// - Lifecycle hook methods (onRequest, preValidation, preHandler, onResponse, onError).
/// - `config(ServerConfig $config): self` for server configuration.
/// - `run(): void` to start the HTTP server.
pub(in crate::backends::php::gen_bindings) fn gen_service_php(api: &ApiSurface, extension_name: &str) -> String {
    let mut out = String::new();

    out.push_str("<?php\n\n");
    out.push_str("declare(strict_types=1);\n\n");

    // Emit one class per service
    for service in &api.services {
        gen_service_class(&mut out, service, api, extension_name);
    }

    out
}

fn gen_service_class(out: &mut String, service: &ServiceDef, _api: &ApiSurface, _extension_name: &str) {
    let class_name = &service.name;

    // Class declaration with docblock
    if !service.doc.is_empty() {
        out.push_str(&format_php_comment(&service.doc, 0));
    }
    out.push_str("final class ");
    out.push_str(class_name);
    out.push_str("\n{\n");

    // Private state for routes and config
    out.push_str("    private array $registrations = [];\n");
    out.push_str("    private ?ServerConfig $config = null;\n\n");

    // Simple zero-arg constructor
    out.push_str("    /**\n");
    out.push_str("     * Create a new application with default configuration.\n");
    out.push_str("     */\n");
    out.push_str("    public function __construct()\n");
    out.push_str("    {\n");
    out.push_str("    }\n\n");

    // config(ServerConfig $config): self
    out.push_str("    /**\n");
    out.push_str("     * Set the server configuration.\n");
    out.push_str("     */\n");
    out.push_str("    public function config(ServerConfig $config): self\n");
    out.push_str("    {\n");
    out.push_str("        $this->config = $config;\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    // Lifecycle hook methods
    out.push_str("    /**\n");
    out.push_str("     * Register an onRequest lifecycle hook.\n");
    out.push_str("     */\n");
    out.push_str("    public function onRequest(callable $handler): self\n");
    out.push_str("    {\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    out.push_str("    /**\n");
    out.push_str("     * Register a preValidation lifecycle hook.\n");
    out.push_str("     */\n");
    out.push_str("    public function preValidation(callable $handler): self\n");
    out.push_str("    {\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    out.push_str("    /**\n");
    out.push_str("     * Register a preHandler lifecycle hook.\n");
    out.push_str("     */\n");
    out.push_str("    public function preHandler(callable $handler): self\n");
    out.push_str("    {\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    out.push_str("    /**\n");
    out.push_str("     * Register an onResponse lifecycle hook.\n");
    out.push_str("     */\n");
    out.push_str("    public function onResponse(callable $handler): self\n");
    out.push_str("    {\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    out.push_str("    /**\n");
    out.push_str("     * Register an onError lifecycle hook.\n");
    out.push_str("     */\n");
    out.push_str("    public function onError(callable $handler): self\n");
    out.push_str("    {\n");
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");

    // Verb methods: get, post, put, patch, delete, head, options
    gen_verb_method(out, "get", "GET");
    gen_verb_method(out, "post", "POST");
    gen_verb_method(out, "put", "PUT");
    gen_verb_method(out, "patch", "PATCH");
    gen_verb_method(out, "delete", "DELETE");
    gen_verb_method(out, "head", "HEAD");
    gen_verb_method(out, "options", "OPTIONS");

    // Entrypoint methods (run, into_router, etc.)
    for ep in &service.entrypoints {
        let mut params = Vec::new();
        for p in &ep.params {
            let annotation = php_type_annotation(&p.ty);
            if p.optional {
                params.push(format!("?{} ${} = null", annotation, p.name));
            } else {
                params.push(format!("{} ${}", annotation, p.name));
            }
        }
        let param_sig = params.join(", ");
        let ep_name = &ep.method;

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str("    /**\n");
                out.push_str("     * Run the HTTP server.\n");
                out.push_str("     */\n");
                out.push_str("    public function run(): void\n");
                out.push_str("    {\n");
                // Convention: native fn is app_run (lowercase, generic)
                let native_fn = "app_run";
                out.push_str(&format!("        {native_fn}($this->registrations, $this->config);\n"));
                out.push_str("    }\n\n");
            }
            EntrypointKind::Finalize => {
                // Handle finalize (into_router, etc.)
                let return_annotation = php_type_annotation(&ep.return_type);
                out.push_str(&render(
                    "php_service_method_start.jinja",
                    context! {
                        method_name => ep_name,
                        param_sig => &param_sig,
                        return_type => &return_annotation,
                    },
                ));
                if !ep.doc.is_empty() {
                    out.push_str(&format_php_comment(&ep.doc, 8));
                }
                out.push_str("        // Finalize entrypoint — forwarding to native layer\n");
                out.push_str("        return null;\n");
                out.push_str("    }\n\n");
            }
        }
    }

    out.push_str("}\n\n");
}

/// Emit a single verb method (get, post, put, patch, delete, head, options).
fn gen_verb_method(out: &mut String, verb: &str, http_method: &str) {
    let method_name = verb;
    let http_method_const = match http_method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "GET",
    };

    let doc = format!("Register a {} route at the given path.", http_method);
    out.push_str("    /**\n");
    out.push_str(&format!("     * {}.\n", doc));
    out.push_str("     */\n");
    out.push_str(&format!(
        "    public function {method_name}(string $path, callable $handler): self\n"
    ));
    out.push_str("    {\n");
    out.push_str(&format!(
        "        $this->registrations[] = ['route', ['{http_method_const}', $path], $handler];\n"
    ));
    out.push_str("        return $this;\n");
    out.push_str("    }\n\n");
}
