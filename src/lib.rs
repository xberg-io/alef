//! alef — polyglot binding generator.
//!
//! Top-level module re-exports for the consolidated `alef` crate.
//! Each module corresponds to one of the former workspace member crates
//! (alef-core, alef-codegen, ...). See README and CHANGELOG (v0.18.0)
//! for the consolidation rationale.
//!
//! ## Extension API
//!
//! Consumers who need domain-specific codegen (e.g. HTTP service bindings)
//! implement [`Extension`] and call [`run_with_extensions`] instead of `main`:
//!
//! ```rust,no_run
//! fn main() -> std::process::ExitCode {
//!     alef::run_with_extensions(vec![])
//! }
//! ```

#![allow(missing_docs)]

pub mod adapters;
pub mod backends;
pub mod bin_cli;
pub mod cli;
pub mod codegen;
pub mod core;
pub mod docs;
pub mod e2e;
pub mod extensions;
pub mod extract;
pub mod publish;
pub mod readme;
pub mod scaffold;
pub mod snippets;

pub use core::extension::{Extension, ExtensionConfig};
pub use core::template_env::TemplateEnv;
pub use extensions::template::TemplateExtension;

// Convenience re-exports for downstream extensions that own e2e / domain codegen.
pub use core::backend::GeneratedFile;
pub use core::config::{E2eConfig, Language, ResolvedCrateConfig};
pub use core::ir::{ApiSurface, EnumDef, TypeDef};
pub use e2e::fixture::{Fixture, FixtureGroup, group_fixtures, load_fixtures};

/// Run the alef CLI, threading the given extensions through the pipeline.
///
/// The built-in [`TemplateExtension`] is always prepended so consumers who
/// pass `vec![]` still get `[[extensions.template]]` block support.
///
/// # Example
///
/// ```rust,no_run
/// fn main() -> std::process::ExitCode {
///     alef::run_with_extensions(vec![])
/// }
/// ```
pub fn run_with_extensions(mut extensions: Vec<Box<dyn Extension>>) -> std::process::ExitCode {
    use clap::Parser;

    // Always prepend the built-in TemplateExtension.
    extensions.insert(0, Box::new(TemplateExtension));

    let cli = bin_cli::args::Cli::parse();
    bin_cli::helpers::init_tracing(cli.verbose, cli.quiet, cli.no_color);

    if cli.jobs > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.jobs)
            .build_global()
            .ok();
    }

    #[cfg(feature = "dylib-loader")]
    match extensions::dylib::load_dylib_extensions_from_config(&cli.config) {
        Ok(mut dylib_extensions) => extensions.append(&mut dylib_extensions),
        Err(e) => {
            eprintln!("error: {e:#}");
            return std::process::ExitCode::FAILURE;
        }
    }

    // Store extensions in a process-global so the pipeline can access them
    // from rayon worker threads (which have their own thread-locals). A
    // `thread_local!` here would leave the workers seeing an empty list —
    // every parallel `generate()` call would then skip extension emission.
    let _ = EXTENSIONS.set(extensions);

    match bin_cli::dispatch::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Active extensions for the current pipeline run.
///
/// Populated by [`run_with_extensions`] before dispatch; accessed by
/// pipeline stages via [`with_extensions`]. Process-global (not
/// `thread_local!`) so rayon worker threads see the same list.
pub(crate) static EXTENSIONS: std::sync::OnceLock<Vec<Box<dyn Extension>>> = std::sync::OnceLock::new();

/// Run `f` with an immutable reference to the active extensions list.
pub(crate) fn with_extensions<F, R>(f: F) -> R
where
    F: FnOnce(&[Box<dyn Extension>]) -> R,
{
    static EMPTY: Vec<Box<dyn Extension>> = Vec::new();
    f(EXTENSIONS.get().unwrap_or(&EMPTY))
}
