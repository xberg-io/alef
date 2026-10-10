# Python async runtime lifecycle

Generated Python async bindings own their Tokio runtime. Importing the extension registers an internal `atexit` hook; the runtime starts on first async use. The hook permanently closes the runtime before Python finalization, releases the GIL, and waits up to five seconds for shutdown. Later async calls or stream producers raise `RuntimeError` and cannot restart it.

Tokio cancels asynchronous tasks during shutdown. The five-second limit bounds the wait; it cannot forcibly terminate Rust work that blocks indefinitely or never yields. Such core operations must provide their own cancellation and time limits.

Set `crates.python.async_runtime = "managed"` to expose `shutdown_async_runtime()`. Manual shutdown rejects active work and stops an idle runtime; a later async call can start it again. Interpreter exit permanently closes either runtime mode.

Forking before first async use allows the child to initialize its own runtime. After async use in the parent, the child raises a `RuntimeError` explaining the unsupported inherited runtime before accessing inherited runtime locks. Use Python multiprocessing's `spawn` method, or fork before first async use. The parent remains usable.

Opaque Python handles remain thread-bound by default. The generator warns when an async parameter or receiver captures one. Configure `crates.python.send_sync_types` only for Rust handles that satisfy `Send + Sync`; Alef does not infer thread safety or automatically change it.

The native fixture in `tests/backends_pyo3_managed_runtime_test/lifecycle.rs` tests default and managed executors in subprocesses, including exit ordering, later rejected work, an unread producer, manual restart, cold fork, and warm fork. Set `ALEF_LIFECYCLE_ARTIFACTS` to preserve the compiled extensions under `managed/` and `default/`.

For the Linux interpreter-finalization race, preserve baseline and fixed native fixture directories separately, then run the opt-in interleaved comparison:

```sh
python3 tests/backends_pyo3_managed_runtime_test/stress.py \
  --baseline /absolute/path/to/baseline/managed \
  --fixed /absolute/path/to/fixed/managed \
  --runs 2000 --workers 4 --cpus 0,1
```

Use two CPUs available to the Linux process. Each arm runs 2,000 children, with 20 async calls per child; baseline, fixed, and fixed with hooks cleared are shuffled together. The output reports actual process counts, completed work, nonzero exits, signal exits, and exit-hook timing samples with median and maximum milliseconds. Two LIFO exit observers bracket the native hook; controls use the same observers without that hook. The fixed arm must produce one timing per child. These measurements include observer overhead and scheduling delay. A macOS run verifies the harness but does not establish the crash-rate reduction reported on Linux. Rebuild the preserved extensions for the interpreter and architecture used in the measurement.
