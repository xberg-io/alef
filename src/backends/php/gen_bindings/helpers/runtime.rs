pub(crate) fn gen_tokio_runtime() -> String {
    "static WORKER_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    // 16 MiB: tokio's ~2 MB default worker stack can overflow on a deep extraction
    // future (a nested archive member, a multi-stage OCR pipeline), and a stack overflow
    // aborts the process with SIGBUS instead of raising a catchable panic.
    const WORKER_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(WORKER_RUNTIME_STACK_SIZE_BYTES)
        .build()
        .expect(\"Failed to create Tokio runtime\")
});"
    .to_string()
}
