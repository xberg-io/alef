//! Emits service App struct wrappers for the swift-bridge crate.
//!
//! Each service with registrations gets a wrapper struct that exposes:
//! - `App { inner: tokio::sync::Mutex<Option<spikard::App>> }`
//! - `pub fn new() -> Self`
//! - `pub fn config(&mut self) -> ()`
//! - `pub fn run(self) -> Result<(), String>`

use crate::core::ir::ApiSurface;

/// Generate App wrapper struct and impl for all services with registrations.
pub fn emit_service_app_wrappers(api: &ApiSurface) -> String {
    let mut out = String::new();

    if api.services.is_empty() {
        return out;
    }

    for service in &api.services {
        if service.registrations.is_empty() {
            continue;
        }

        let service_name = &service.name;

        out.push_str(&format!(
            "/// Wrapper for {service_name} service instance.\n\
             /// Holds the inner service in a blocking mutex to allow \
             mutable access\n\
             /// across FFI boundaries.\n\
             pub struct {service_name} {{\n\
             \x20\x20\x20\x20pub inner: tokio::sync::Mutex<Option<spikard::App>>,\n\
             }}\n\n"
        ));

        out.push_str(&format!(
            "impl {service_name} {{\n\
             \x20\x20\x20\x20/// Create a new service instance.\n\
             \x20\x20\x20\x20pub fn new() -> Self {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20Self {{\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20inner: tokio::sync::Mutex::new(Some(spikard::App::new())),\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
             \x20\x20\x20\x20}}\n\n"
        ));

        out.push_str(
            "    /// Configure the service.\n\
             \x20\x20\x20\x20pub fn config(&mut self) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20// Placeholder for future configuration.\n\
             \x20\x20\x20\x20}\n\n",
        );

        // The bridge declares `fn app_run(self: &mut App) -> String;` because swift-bridge 0.1.59
        // does not parse by-value `self: App` consume-self in `extern "Rust"` blocks. The
        // wrapper's `run` therefore takes `&mut self`, `take()`s the inner App out of the
        // Mutex (single-shot consume), and returns a String envelope describing success or
        // the error (Result<T, E> is not bridgeable across this swift-bridge version).
        out.push_str(
            "    /// Run the service (blocking, drives the Tokio runtime).\n\
             \x20\x20\x20\x20///\n\
             \x20\x20\x20\x20/// Returns an empty string on success or the error message.\n\
             \x20\x20\x20\x20pub fn run(&mut self) -> String {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20let rt = match tokio::runtime::Runtime::new() {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(rt) => rt,\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => return format!(\"runtime error: {:?}\", e),\n\
             \x20\x20\x20\x20\x20\x20\x20\x20};\n\
             \x20\x20\x20\x20\x20\x20\x20\x20rt.block_on(async {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let mut guard = self.inner.lock().await;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if let Some(app) = guard.take() {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match app.run().await {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(()) => String::new(),\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => format!(\"{:?}\", e),\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20} else {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\"service already consumed\".to_string()\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20})\n\
             \x20\x20\x20\x20}\n\
             }\n\n",
        );
    }

    out
}
