//! E2E test generation configuration types.

mod call;
mod defaults;
mod harness;
mod package;
mod root;
mod selection;

#[cfg(test)]
mod tests;

pub use call::{ArgMapping, CallConfig, CallOverride, StreamingConfig, StreamingRecipe};
pub use harness::{HarnessConfig, HarnessOverride, RouteCallForm};
pub use package::{DependencyMode, HomebrewCliTest, PackageRef, RegistryConfig};
pub use root::E2eConfig;
pub use selection::SelectWhen;
