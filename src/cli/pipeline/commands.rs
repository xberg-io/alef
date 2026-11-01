mod build;
mod clean;
mod lint;
mod setup;
mod test;
mod test_apps;
mod update;

pub(crate) use build::build_with_environment;
pub(crate) use build::canonical_frb_generated;
pub(crate) use build::run_post_build_for_generation;
pub use build::{StagingProfile, build, run_post_build};
pub use clean::clean;
pub use lint::{fmt, fmt_post_generate, lint};
pub use setup::setup;
pub use test::test;
pub use test_apps::test_apps_run;
pub use update::update;
