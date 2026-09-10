//! Mock server source generation for Rust e2e tests.

mod binary;
mod common_module;
mod response_body;
mod route_loading;
mod runtime_server;
mod server_module;
mod setup;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod truncation_tests;

pub use binary::render_mock_server_binary;
pub use common_module::render_common_module;
pub use server_module::render_mock_server_module;
pub use setup::render_mock_server_setup;
