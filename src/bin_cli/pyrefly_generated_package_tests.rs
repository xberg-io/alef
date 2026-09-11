//! `src/snippets/validators/python.rs` runs pyrefly, but only over isolated doc-snippet
//! scratch files -- never over a real generated package. A type checker alef ships and never
//! points at its own generated output is a check that examines nothing: the pyo3 backend's
//! public-dataclass-vs-native-pyclass mismatch (task alef-310, fixed alongside this test) went
//! undetected by every automated gate alef runs on itself, and was only found by a human running
//! pyrefly by hand over a real consumer's `packages/python`. This test closes that gap by driving
//! `alef all` end to end against a real fixture -- the same dispatch path a consumer runs -- and
//! then running pyrefly over the actual `packages/python` directory it wrote, exactly as a
//! consumer's own `pyrefly check` (or `alef lint`'s `typecheck` step, see
//! `core::config::lint_defaults::default_lint_config`) would.
//!
//! This is a single dedicated test, not a step wired into every `cargo test` or `alef build`:
//! pyrefly is an external tool this dev machine happens to have, most CI machines running the
//! full `--lib` suite do not, and a type-checker pass over generated Python is exactly the kind
//! of slow, environment-dependent check that belongs in one targeted place rather than on every
//! build. It follows the same `which::which("pyrefly").is_err() { return }` skip convention
//! already used by `snippets::validators::python`'s own pyrefly-backed tests, rather than
//! inventing a second one.

use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;

const FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `maybe_result` returns `Option<ResultData>` -- never `ResultData` bare -- so it exercises the
/// extraction fix (`is_return_type` must be set through `Option`/`Vec`/`Map` wrappers, not only
/// on a bare `Named` return) exactly as much as it exercises the pyo3 codegen that reads that
/// flag. `ResultData` has no other use in this fixture, so if the flag were wrong `options.py`
/// would emit it as an input dataclass while `api.py` actually returns/receives the native
/// pyclass -- the mismatch pyrefly's `bad-return` catches.
///
/// The remaining constructs below exist to audit the three still-suppressed pyrefly codes
/// (`bad-argument-count`, `not-iterable`, `missing-attribute`; task alef-334) against real
/// generated output rather than leaving them suppressed on faith. Each targets the specific
/// generated-code shape most likely to trip its code (see `src/backends/pyo3/gen_bindings/
/// functions/converters.rs` for the `options`-dataclass-to-native-pyclass conversion path all
/// three shapes exercise):
///
/// - `Filter` is used ONLY as a function argument (`apply_filter`), so it is emitted as an
///   `options.py` dataclass whose `_to_rust_filter` conversion calls the native `Filter`
///   pyclass constructor by keyword -- the exact call site where a wrapper/native argument-count
///   desync would surface (`bad-argument-count`).
/// - `Status` is a plain (data-less) enum and `BatchInput.statuses` is `Vec<Status>` used only as
///   an argument, which routes through `simple_enum_vec_coerce.jinja`'s
///   `[_coerce_enum(_rust.Status, v) for v in accessor]` list comprehension -- the exact
///   generated shape that needs `accessor` to type as an iterable (`not-iterable`).
/// - `Person`/`Address` is a nested options dataclass (a dataclass field that is itself another
///   dataclass) used only as an argument, so the outer conversion function must chain into the
///   inner one and read fields across that boundary -- the shape most likely to read a field
///   that does not exist on the declared dataclass type (`missing-attribute`).
/// - `ValidationError` is a `#[derive(thiserror::Error)]` enum with a field-carrying variant, the
///   shape `src/extract/extractor/mod.rs`'s `is_thiserror_enum` routes to `surface.errors` (a
///   plain struct implementing `Display`/`Error` by hand is NOT recognized as an error type and
///   was NOT exercising this path before this fixture was written -- confirmed by inspecting the
///   original fixture's generated `exceptions.py`, which was empty). ~keep
const FIXTURE_SOURCE: &str = r#"
#[derive(Default)]
pub struct ResultData {
    pub label: String,
}

pub fn maybe_result(flag: bool) -> Option<ResultData> {
    if flag {
        Some(ResultData { label: "found".to_string() })
    } else {
        None
    }
}

#[derive(Default, Clone)]
pub struct Point {
    pub x: i64,
    pub y: i64,
    pub label: Option<String>,
}

impl Point {
    pub fn new(x: i64, y: i64, label: Option<String>) -> Self {
        Point { x, y, label }
    }

    pub fn translate(&self, dx: i64, dy: i64, scale: Option<i64>) -> Point {
        let factor = scale.unwrap_or(1);
        Point {
            x: self.x + dx * factor,
            y: self.y + dy * factor,
            label: self.label.clone(),
        }
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Point {
        Point::new(bytes.into().len() as i64, 0, None)
    }
}

pub fn list_points(count: i64) -> Vec<Point> {
    (0..count).map(|i| Point::new(i, i, None)).collect()
}

pub enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

pub fn describe_shape(shape: Shape) -> String {
    match shape {
        Shape::Circle { radius } => format!("circle r={radius}"),
        Shape::Rectangle { width, height } => format!("rect {width}x{height}"),
    }
}

#[derive(Default)]
pub struct Filter {
    pub min_value: i64,
    pub max_value: i64,
    pub label: Option<String>,
}

pub fn apply_filter(data: Vec<i64>, filter: Filter) -> Vec<i64> {
    data.into_iter()
        .filter(|value| *value >= filter.min_value && *value <= filter.max_value)
        .collect()
}

pub enum Status {
    Active,
    Inactive,
}

#[derive(Default)]
pub struct BatchInput {
    #[serde(default)]
    pub statuses: Vec<Status>,
}

pub fn count_active(input: BatchInput) -> i64 {
    input.statuses.iter().filter(|status| matches!(status, Status::Active)).count() as i64
}

#[derive(Default)]
pub struct Address {
    pub city: String,
}

#[derive(Default)]
pub struct Person {
    pub name: String,
    pub address: Address,
}

pub fn greet(person: Person) -> String {
    format!("hi {} from {}", person.name, person.address.city)
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("{field}: {message}")]
    InvalidField { field: String, message: String },
}

pub fn validate(value: i64) -> Result<i64, ValidationError> {
    if value < 0 {
        Err(ValidationError::InvalidField {
            field: "value".to_string(),
            message: "must be non-negative".to_string(),
        })
    } else {
        Ok(value)
    }
}

// The three constructs below target the three converter-generator defects found auditing
// two consumer repos against 0.67.6 (which removed `bad-argument-type`/`bad-return` from
// the scaffolded pyrefly suppressions on the claim that codegen now emits correct
// `_to_rust_*`/`_from_native_*` conversions for these boundaries -- this fixture proves that
// claim against the specific shapes it did not originally cover). ~keep
//
// - `ResponseTool.tool_type` carries `#[serde(rename = "type")]`, a Python reserved word. The
//   `_to_rust_response_tool` converter and the `.pyi` `__init__` stub must agree on the emitted
//   keyword-argument spelling (`type`, not `type_`) or pyrefly reports `[unexpected-keyword]`.
// - `Recipe.ingredients` is `Vec<Ingredient>` where `Ingredient` is itself a `has_default`
//   struct, so `_to_rust_recipe` must convert each element with `_to_rust_ingredient`, not pass
//   the raw `list[options.Ingredient]` straight through (pyrefly `[bad-argument-type]`).
// - `Task` has two independent optional simple-enum fields (`priority`, `mode`) on one
//   constructor call. Both are `Option<Enum>` in the native binding, so the emitted converter
//   used to route them through a `**({...} if ... else {})` omission trick that isn't needed for
//   an already-optional field -- and two such unpacks in one call is exactly the shape that made
//   pyrefly cross-assign the two enum types between the two parameters.
#[derive(Default)]
pub struct ResponseTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub label: Option<String>,
}

pub fn describe_tool(tool: ResponseTool) -> String {
    format!("{}: {}", tool.tool_type, tool.label.unwrap_or_default())
}

#[derive(Default, Clone)]
pub struct Ingredient {
    pub name: String,
}

#[derive(Default)]
pub struct Recipe {
    pub title: String,
    pub ingredients: Vec<Ingredient>,
}

pub fn total_ingredients(recipe: Recipe) -> i64 {
    recipe.ingredients.len() as i64
}

pub enum Priority {
    Low,
    High,
}

pub enum Mode {
    Fast,
    Slow,
}

#[derive(Default)]
pub struct Task {
    pub title: String,
    pub priority: Option<Priority>,
    pub mode: Option<Mode>,
}

pub fn describe_task(task: Task) -> String {
    let priority = match task.priority {
        Some(Priority::Low) => "low",
        Some(Priority::High) => "high",
        None => "unset",
    };
    let mode = match task.mode {
        Some(Mode::Fast) => "fast",
        Some(Mode::Slow) => "slow",
        None => "unset",
    };
    format!("{}: {priority}/{mode}", task.title)
}

// Three NON-`Option` enum fields, each carrying bare `#[serde(default)]`, on one config struct.
// `Task` above covers the `Option<Enum>` form; this covers the form that still routed through the
// `**({...} if x is not None else {})` omission trick. `options.py` renders each of these as the
// enum's `#[default]` variant string and never as `None`, so the guard is statically always true
// -- and pyrefly resolves an unpacked keyword against every remaining parameter, so N unpacks in
// one constructor call cost N*(N-1) `[bad-argument-type]` errors. Three fields is the smallest
// count that makes the cost visible as a cluster rather than a single pair. ~keep
#[derive(Default, Clone, Copy)]
pub enum Alignment {
    #[default]
    Start,
    End,
}

#[derive(Default, Clone, Copy)]
pub enum Density {
    #[default]
    Loose,
    Tight,
}

#[derive(Default, Clone, Copy)]
pub enum Casing {
    #[default]
    Lower,
    Upper,
}

#[derive(Default)]
pub struct LayoutSpec {
    pub title: String,
    #[serde(default)]
    pub alignment: Alignment,
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub casing: Casing,
}

pub fn describe_layout(spec: LayoutSpec) -> String {
    let alignment = match spec.alignment {
        Alignment::Start => "start",
        Alignment::End => "end",
    };
    let density = match spec.density {
        Density::Loose => "loose",
        Density::Tight => "tight",
    };
    let casing = match spec.casing {
        Casing::Lower => "lower",
        Casing::Upper => "upper",
    };
    format!("{}: {alignment}/{density}/{casing}", spec.title)
}

fn default_minimum_score() -> f64 {
    0.5
}

fn default_minimum_words() -> usize {
    10
}

fn default_require_text() -> bool {
    true
}

#[derive(Default)]
pub struct QualityThresholds {
    #[serde(default = "default_minimum_score")]
    pub minimum_score: f64,
    #[serde(default = "default_minimum_words")]
    pub minimum_words: usize,
    #[serde(default = "default_require_text")]
    pub require_text: bool,
}

pub fn assess_quality(thresholds: QualityThresholds) -> bool {
    thresholds.require_text && thresholds.minimum_words > 0 && thresholds.minimum_score > 0.0
}
"#;

const FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#;

fn write_fixture_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// The scaffolded `pyproject.toml` lives beside the python package directory (e.g.
/// `packages/python/pyproject.toml` next to `packages/python/test_lib/`), not inside it -- find
/// it by its `[tool.pyrefly]` marker rather than hard-coding the package-name-derived path.
fn find_pyrefly_project_dir(root: &std::path::Path) -> std::path::PathBuf {
    for entry in walkdir_pyproject_tomls(root) {
        let content = std::fs::read_to_string(&entry).unwrap_or_default();
        if content.contains("[tool.pyrefly]") {
            return entry
                .parent()
                .expect("pyproject.toml has a parent directory")
                .to_path_buf();
        }
    }
    panic!("no scaffolded pyproject.toml with a [tool.pyrefly] section found under {root:?}");
}

fn walkdir_pyproject_tomls(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == "pyproject.toml") {
                found.push(path);
            }
        }
    }
    found
}

fn pyrefly_executable() -> Option<std::path::PathBuf> {
    match which::which("pyrefly") {
        Ok(path) => Some(path),
        Err(error) if std::env::var_os("ALEF_REQUIRE_PYREFLY").is_some() => {
            panic!("ALEF_REQUIRE_PYREFLY is set but pyrefly is unavailable: {error}")
        }
        Err(_) => None,
    }
}

/// Runs `alef all` against a real fixture, then runs real pyrefly 1.2.0+ over the real
/// `packages/python` output it wrote -- the same directory and the same `pyproject.toml` (with
/// its scaffolded `[[tool.pyrefly.sub-config]]` suppressions) a consumer's own `pyrefly check`
/// would see. Zero errors is the regression gate: a return-type or argument-type boundary
/// mismatch reintroduced into the pyo3 backend surfaces here as a real `bad-return` or
/// `bad-argument-type`, not just in a hand-run consumer audit.
///
/// The fixture also exercises the three codes `src/scaffold/languages/python.rs` still
/// suppresses for every `**/api.py` (`bad-argument-count`, `not-iterable`, `missing-attribute`;
/// task alef-334) through their most plausible generated shapes (see `FIXTURE_SOURCE`'s doc
/// comment). Hand-corrupting the generated `_to_rust_filter`/`_to_rust_batch_input`/
/// `_to_rust_person` call sites this fixture produces (an extra positional arg, an iteration
/// over a non-iterable, a typoed attribute) reliably reproduces `[bad-argument-count]`,
/// `[not-iterable]`, and `[missing-attribute]` respectively -- proof this gate can and does
/// detect each code, not just a vacuous pass. With those codes left enabled and the fixture
/// left uncorrupted, this test currently reports zero errors, i.e. none of the three codes is
/// presently reproducible from real (uncorrupted) codegen output for the shapes this fixture
/// covers.
#[test]
fn alef_all_generated_python_package_type_checks_clean_under_pyrefly() {
    let Some(pyrefly) = pyrefly_executable() else {
        return;
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    super::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: false,
            skip_snippet_validation: true,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed against a plain python fixture");

    let api_py = root.join("packages/python/test_lib/api.py");
    assert!(
        api_py.is_file(),
        "sanity: alef all must have written api.py, got tree under {root:?}"
    );
    let stub = std::fs::read_to_string(root.join("packages/python/test_lib/test_lib.pyi"))
        .expect("alef all must write the native module stub");
    assert!(
        stub.contains("bytes: builtins.bytes"),
        "a Python-visible `bytes` parameter must keep its builtin type annotation resolvable:\n{stub}"
    );
    let project_dir = find_pyrefly_project_dir(&root);

    let output = std::process::Command::new(&pyrefly)
        .arg("check")
        .arg(&project_dir)
        .output()
        .expect("pyrefly check must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "pyrefly must report zero errors against alef's own generated package \
         (a `bad-return`/`bad-argument-type` here means the public-dataclass boundary fix \
         regressed); pyrefly stdout:\n{stdout}\npyrefly stderr:\n{stderr}"
    );
}
