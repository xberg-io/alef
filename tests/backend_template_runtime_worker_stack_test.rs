//! Per-backend regression coverage for the worker-stack invariant on tokio runtimes that are
//! emitted from *static code templates* rather than assembled in Rust.
//!
//! A stack overflow inside a tokio worker thread is not a catchable panic: the guard-page fault
//! aborts the whole process (SIGBUS / `EXC_BAD_ACCESS`, `KERN_PROTECTION_FAILURE` on macOS). A
//! consumer whose async work is deep — a nested archive member, a multi-stage OCR pipeline —
//! therefore kills its entire test process rather than failing one call. Tokio's ~2 MB default
//! worker stack is not enough headroom, so every emitted runtime must widen it.
//!
//! These assert on the template sources directly, which keeps them free of any per-backend
//! `ApiSurface` fixture and makes each failure point at the exact file to edit. The
//! generator-wide scan in `generated_runtime_worker_stack_invariant.rs` is the net that catches
//! backends nobody thought to list here; this file names the known ones so a regression reports
//! which backend broke.

/// Project-wide worker stack size for emitted runtimes, as it appears in generated code.
const EXPECTED_STACK_SIZE_LITERAL: &str = "16 * 1024 * 1024";

const MULTI_THREAD_BUILDER: &str = "tokio::runtime::Builder::new_multi_thread()";
const STACK_SIZE_SETTER: &str = ".thread_stack_size(";
const BANNED_CONSTRUCTOR: &str = "Runtime::new()";

fn assert_template_widens_worker_stack(template_name: &str, source: &str) {
    assert!(
        source.contains(MULTI_THREAD_BUILDER),
        "{template_name}: runtime must be built with the multi-thread builder, got:\n{source}"
    );
    assert!(
        source.contains(STACK_SIZE_SETTER),
        "{template_name}: runtime must set an explicit worker stack size — tokio's ~2 MB default \
         is overflowed by a deep consumer future, which aborts the process with SIGBUS instead of \
         raising a catchable panic. Got:\n{source}"
    );
    assert!(
        source.contains(EXPECTED_STACK_SIZE_LITERAL),
        "{template_name}: worker stack size must be the project-wide 16 MiB value, got:\n{source}"
    );
    assert!(
        !source.contains(BANNED_CONSTRUCTOR),
        "{template_name}: `{BANNED_CONSTRUCTOR}` builds a runtime with tokio's default \
         (undersized) worker stack and cannot be widened, got:\n{source}"
    );
}

/// The stack size must be a named constant. A bare literal in emitted code is both a magic
/// number in the consumer's tree and, where a template can be rendered more than once into one
/// file, indistinguishable from a duplicate definition hazard.
fn assert_stack_size_is_a_named_constant(template_name: &str, source: &str) {
    let declares_const = source.contains("_STACK_SIZE_BYTES: usize =");
    assert!(
        declares_const,
        "{template_name}: the worker stack size must be declared as a named `*_STACK_SIZE_BYTES` \
         constant rather than passed as a bare literal, got:\n{source}"
    );
    assert!(
        !source.contains(&format!("{STACK_SIZE_SETTER}{EXPECTED_STACK_SIZE_LITERAL})")),
        "{template_name}: the setter must reference the named constant, not inline the literal, \
         got:\n{source}"
    );
}

#[test]
fn elixir_rustler_templates_build_runtimes_with_an_explicit_worker_stack() {
    let templates = [
        (
            "rustler/async_infallible_body",
            include_str!("../src/backends/rustler/templates/async_infallible_body.rs.jinja"),
        ),
        (
            "rustler/async_result_body",
            include_str!("../src/backends/rustler/templates/async_result_body.rs.jinja"),
        ),
        (
            "rustler/sync_method_body",
            include_str!("../src/backends/rustler/templates/sync_method_body.rs.jinja"),
        ),
        (
            "rustler/service_api_entrypoint_call",
            include_str!("../src/backends/rustler/templates/service_api_entrypoint_call.rs.jinja"),
        ),
    ];

    for (name, source) in templates {
        assert_template_widens_worker_stack(name, source);
        assert_stack_size_is_a_named_constant(name, source);
    }
}

#[test]
fn java_jni_templates_build_runtimes_with_an_explicit_worker_stack() {
    let templates = [
        (
            "jni/entrypoint_run",
            include_str!("../src/backends/jni/templates/entrypoint_run.rs.jinja"),
        ),
        (
            "jni/entrypoint_finalize",
            include_str!("../src/backends/jni/templates/entrypoint_finalize.rs.jinja"),
        ),
        (
            "jni/runtime_helpers",
            include_str!("../src/backends/jni/templates/runtime_helpers.rs.jinja"),
        ),
    ];

    for (name, source) in templates {
        assert_template_widens_worker_stack(name, source);
        assert_stack_size_is_a_named_constant(name, source);
    }
}

#[test]
fn swift_service_wrapper_template_builds_its_runtime_with_an_explicit_worker_stack() {
    let source = include_str!("../src/backends/swift/templates/rust_service_app_wrapper.rs.jinja");
    assert_template_widens_worker_stack("swift/rust_service_app_wrapper", source);
    assert_stack_size_is_a_named_constant("swift/rust_service_app_wrapper", source);
}

#[test]
fn ffi_shared_runtime_template_builds_its_runtime_with_an_explicit_worker_stack() {
    let source = include_str!("../src/backends/ffi/templates/ffi_tokio_runtime.jinja");
    assert_template_widens_worker_stack("ffi/ffi_tokio_runtime", source);
    assert_stack_size_is_a_named_constant("ffi/ffi_tokio_runtime", source);
}

/// The Swift backend's process-wide runtime is embedded from a Rust string constant rather than
/// a template file, and every swift-bridge async wrapper drives it — so an undersized worker
/// stack there is reachable from the entire Swift surface.
#[test]
fn swift_process_wide_runtime_is_built_with_an_explicit_worker_stack() {
    let source = include_str!("../src/backends/swift/gen_rust_crate/shims.rs");
    let definition_start = source
        .find("ALEF_TOKIO_RUNTIME_DEFINITION")
        .expect("swift shims must still define the process-wide runtime snippet");
    let definition = &source[definition_start..];

    assert!(
        definition.contains(MULTI_THREAD_BUILDER),
        "swift process-wide runtime must use the multi-thread builder"
    );
    assert!(
        definition.contains(STACK_SIZE_SETTER),
        "swift process-wide runtime must set an explicit worker stack size"
    );
    assert!(
        definition.contains(EXPECTED_STACK_SIZE_LITERAL),
        "swift process-wide runtime must use the project-wide 16 MiB worker stack"
    );
}

/// Size of the thread a `block_on` call is given, where the generator owns that thread. It is a
/// different quantity from `EXPECTED_STACK_SIZE_LITERAL`: that one sizes tokio *workers*, this one
/// sizes the single thread the future is driven on. The Rustler bridge set the precedent at 32 MiB.
const EXPECTED_CALLING_THREAD_STACK_LITERAL: &str = "32 * 1024 * 1024";

const THREAD_BUILDER: &str = "std::thread::Builder::new()";
const THREAD_STACK_SIZE_SETTER: &str = ".stack_size(";
const CURRENT_THREAD_BUILDER: &str = "tokio::runtime::Builder::new_current_thread()";
const BARE_THREAD_SPAWN: &str = "std::thread::spawn(";

/// The Dart trait bridge drives host callbacks with `block_on` on a thread it spawns itself, under
/// a *current-thread* runtime. `.thread_stack_size(...)` is inert there — a current-thread runtime
/// has no workers, and that setter reaches only the blocking pool — so the widening has to happen
/// on the spawn. It previously used a bare `std::thread::spawn`, i.e. Rust's 2 MiB default, which a
/// deep callback future overflows into a process-killing SIGBUS rather than a catchable panic.
#[test]
fn dart_trait_bridge_template_widens_the_thread_its_block_on_runs_on() {
    let name = "dart/rust_trait_method_block_on";
    let source = include_str!("../src/backends/dart/templates/rust_trait_method_block_on.jinja");

    assert!(
        source.contains(CURRENT_THREAD_BUILDER),
        "{name}: expected the current-thread runtime this test is about, got:\n{source}"
    );
    assert!(
        !source.contains(BARE_THREAD_SPAWN),
        "{name}: `{BARE_THREAD_SPAWN}` gives the `block_on` thread Rust's 2 MiB default stack, \
         which a deep host-callback future overflows — and a stack overflow aborts the process \
         with SIGBUS instead of raising a catchable panic. Use \
         `{THREAD_BUILDER}{THREAD_STACK_SIZE_SETTER}<named const>).spawn(...)`. Got:\n{source}"
    );
    assert!(
        source.contains(THREAD_BUILDER) && source.contains(THREAD_STACK_SIZE_SETTER),
        "{name}: the spawn must widen the thread it creates, got:\n{source}"
    );
    assert!(
        source.contains(EXPECTED_CALLING_THREAD_STACK_LITERAL),
        "{name}: the calling thread must get the {EXPECTED_CALLING_THREAD_STACK_LITERAL} byte \
         stack the Rustler bridge established for the same job, got:\n{source}"
    );
    assert!(
        !source.contains(STACK_SIZE_SETTER),
        "{name}: `{STACK_SIZE_SETTER}` on a current-thread runtime sizes only the blocking pool \
         and not the stack `block_on` runs on — it reads as protection while providing none, so \
         it must not appear here. Got:\n{source}"
    );
    assert_stack_size_is_a_named_constant(name, source);

    // Both arms of the template build a runtime; a fix applied to only one is the shape this
    // guards against, since the infallible arm is the one most methods take. ~keep
    assert_eq!(
        source.matches(CURRENT_THREAD_BUILDER).count(),
        source.matches(THREAD_BUILDER).count(),
        "{name}: every current-thread runtime in this template must have its own widened spawn, \
         got:\n{source}"
    );
}

/// The PHP extension's shared `WORKER_RUNTIME` is where every PHP async body ultimately runs,
/// so it carries the stack guarantee for the whole PHP surface.
#[test]
fn php_shared_worker_runtime_is_built_with_an_explicit_worker_stack() {
    let source = include_str!("../src/backends/php/gen_bindings/helpers/runtime.rs");
    assert!(
        source.contains(MULTI_THREAD_BUILDER),
        "PHP shared worker runtime must use the multi-thread builder"
    );
    assert!(
        source.contains(STACK_SIZE_SETTER),
        "PHP shared worker runtime must set an explicit worker stack size"
    );
    assert!(
        source.contains(EXPECTED_STACK_SIZE_LITERAL),
        "PHP shared worker runtime must use the project-wide 16 MiB worker stack"
    );
    assert!(
        !source.contains(BANNED_CONSTRUCTOR),
        "PHP shared worker runtime must not use tokio's default (undersized) worker stack"
    );
}
