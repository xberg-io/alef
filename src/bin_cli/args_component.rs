use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum ComponentAction {
    /// Compile configured component feature profiles as native dynamic libraries.
    Build {
        /// Component profile names (default: every configured profile).
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Rust target triples (default: each profile's configured targets).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Build without release optimizations.
        #[arg(long)]
        debug: bool,
        /// Print build commands without executing them.
        #[arg(long)]
        dry_run: bool,
        /// Share one Cargo target directory across every component instead of using
        /// `.alef/components/build/<crate>/<component>` per component. Pass the same value to
        /// `component package`.
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// Build only the `build` rows of this plan (from `component plan`), still narrowed by
        /// `--component` and `--target`. Requires the same `--debug` the plan was made with.
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Extra `key=value` folded into the fingerprint checked against the plan; use the values
        /// given to `component plan`. Repeatable.
        #[arg(long, value_name = "KEY=VALUE", requires = "plan")]
        extra: Vec<String>,
        /// Environment variable NAME folded into the fingerprint (for example one a build script
        /// reads); use the names given to `component plan`. Repeatable.
        #[arg(long, value_name = "NAME", requires = "plan")]
        env: Vec<String>,
    },
    /// Print, per component and target, whether to build or reuse the artifact in a previous lock.
    ///
    /// Each row carries the build's input fingerprint (`input_hash`). A row is `reuse` when the
    /// previous lock has an entry with the same fingerprint and the same identity apart from
    /// `version`; the entry travels in the row as `previous`. Without `--previous-lock` every row
    /// is `build`. Select crates with `--crate`.
    Plan {
        /// Previous `components.lock.json` to reuse artifacts from.
        #[arg(long)]
        previous_lock: Option<PathBuf>,
        /// Component profile names (default: every configured profile).
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Rust target triples (default: each profile's configured targets).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Extra `key=value` folded into the fingerprint (container image, C compiler or SDK
        /// version, signing key id, ...). alef does not probe non-rustc tools; this is how their
        /// versions enter. Repeatable; pass the same values to `component build` and `package`.
        #[arg(long, value_name = "KEY=VALUE")]
        extra: Vec<String>,
        /// Environment variable NAME folded into the fingerprint in addition to the built-in
        /// build-affecting set (RUSTFLAGS, CC/CXX/AR/CFLAGS/CXXFLAGS/LDFLAGS and their target
        /// spellings, linker, SDK variables, ...). Repeatable; pass the same names to `component
        /// build` and `package`.
        #[arg(long, value_name = "NAME")]
        env: Vec<String>,
        /// Fingerprint debug builds; must match `--debug` of `build` and `package`.
        #[arg(long)]
        debug: bool,
        /// Write the plan here instead of stdout.
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Create deterministic signed component archives and artifact records.
    Package {
        /// Component profile names (default: every configured profile).
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Rust target triples (default: each profile's configured targets).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Output directory for component archives and records.
        #[arg(long, short, default_value = "dist/components")]
        output: PathBuf,
        /// Component version (default: resolved from version_from).
        #[arg(long)]
        version: Option<String>,
        /// Ed25519 private key: PKCS#8 PEM or DER (e.g. `openssl genpkey -algorithm ED25519`),
        /// or a raw 32-byte seed. Pass `-` to read the key from stdin.
        #[arg(long, requires = "key_id", conflicts_with_all = ["unsigned", "signing_key_env"])]
        signing_key: Option<PathBuf>,
        /// Name of an environment variable holding the Ed25519 private key as PKCS#8 PEM text, or
        /// base64 of PKCS#8 DER / a raw 32-byte seed.
        #[arg(long, requires = "key_id", conflicts_with_all = ["unsigned", "signing_key"])]
        signing_key_env: Option<String>,
        /// Stable ID of the public key embedded in generated binding packages.
        #[arg(long)]
        key_id: Option<String>,
        /// Produce an unsigned CI intermediate archive.
        #[arg(long, conflicts_with_all = ["signing_key", "signing_key_env"])]
        unsigned: bool,
        /// Package the library built without release optimizations.
        #[arg(long)]
        debug: bool,
        /// Explicit built library path; valid only for one profile/target.
        #[arg(long)]
        library: Option<PathBuf>,
        /// Cargo target directory the components were built into; must match the
        /// `--target-dir` given to `component build` (default: per-component directories).
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// Extra `key=value` folded into the fingerprint checked against the plan; use the values
        /// given to `component plan`. Repeatable.
        #[arg(long, value_name = "KEY=VALUE", requires = "plan")]
        extra: Vec<String>,
        /// Environment variable NAME folded into the fingerprint; use the names given to
        /// `component plan`. Repeatable.
        #[arg(long, value_name = "NAME", requires = "plan")]
        env: Vec<String>,
        /// Package only the `build` rows of this plan (from `component plan`), still narrowed by
        /// `--component` and `--target`, and record the plan's fingerprint as `input_hash`. The
        /// current inputs must still hash to the plan's fingerprint and the library must carry the
        /// `<library>.input-hash` file written by `component build --plan`. Without `--plan` no
        /// fingerprint is recorded.
        #[arg(long)]
        plan: Option<PathBuf>,
    },
    /// Generate and stage a binding-embeddable components.lock.json from signed artifact records.
    Lock {
        /// Directory containing `*.record.json` files from `component package`.
        #[arg(long, short, default_value = "dist/components")]
        input: PathBuf,
        /// Output lock manifest path (default: components.lock.json). Explicit unsigned
        /// output stays isolated instead of staging into binding packages.
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Rust target triples (default: every configured target).
        #[arg(long, value_delimiter = ',')]
        target: Vec<String>,
        /// Component profile names to lock (default: every configured profile). A lock narrowed to
        /// some components is written to `--output` only and never staged into binding packages.
        #[arg(long, value_delimiter = ',')]
        component: Vec<String>,
        /// Emit a dev lock with no `public_keys` from unsigned (or signed) records. The runtime
        /// does not verify signatures against such a lock; never ship it. Archive hashes are
        /// still checked.
        #[arg(long)]
        unsigned: bool,
        /// Override the configured `url_template` (must contain `{artifact}`); e.g.
        /// `file:///abs/dist/components/{artifact}` for a local dev lock. Required with
        /// `--unsigned` when the crate has no `component_distribution`.
        #[arg(long)]
        url_template: Option<String>,
        /// Previous lock to carry the plan's `reuse` entries over from, verbatim (URL, hashes,
        /// signature). Each entry's signing key must still be among the trusted public keys.
        #[arg(long, requires = "plan")]
        reuse: Option<PathBuf>,
        /// Plan (from `component plan`) naming the entries to reuse; `--input` then holds records
        /// for the plan's `build` rows only.
        #[arg(long, requires = "reuse")]
        plan: Option<PathBuf>,
    },
    /// Print each component contract's hash, as recorded in the component lock.
    ///
    /// One `component contract hash` line per provided contract; select crates with `--crate`.
    ContractHash,
    /// Verify artifact hashes, identities, library contents, and Ed25519 signatures.
    Verify {
        /// Artifact record file or directory containing `*.record.json` files.
        #[arg(long, short, default_value = "dist/components")]
        input: PathBuf,
    },
}
