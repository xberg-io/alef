#[cfg(test)]
mod trait_bridge_runtime {
    use super::*;

    #[test]
    fn should_rethrow_the_original_callback_exception() {
        Python::initialize();
        Python::attach(|py| {
            let module = PyModule::new(py, "_sample").unwrap();
            _sample(&module).unwrap();
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("native", module).unwrap();
            py.run(
                c"
import asyncio

class Cancelled(Exception):
    pass

first = Cancelled('stopped')
second = Cancelled('stopped')
initialization = Cancelled('stopped')
unregistration = Cancelled('stopped')
clearing = Cancelled('stopped')

class Worker:
    def initialize(self):
        return None
    def shutdown(self):
        return None
    def work(self):
        raise first
    def work_async(self):
        raise second

native.register_worker(Worker())
try:
    native.invoke_worker()
except Exception as error:
    assert error is first, f'sync callback changed identity: {error!r}'
else:
    raise AssertionError('sync callback failure was swallowed')

async def check_async():
    try:
        await native.invoke_worker_async()
    except Exception as error:
        assert error is second, f'async callback changed identity: {error!r}'
    else:
        raise AssertionError('async callback failure was swallowed')

asyncio.run(check_async())

class RejectRegistration(Worker):
    def initialize(self):
        raise initialization

try:
    native.register_worker(RejectRegistration())
except Exception as error:
    assert error is initialization, f'registration callback changed identity: {error!r}'
else:
    raise AssertionError('registration callback failure was swallowed')

class RejectShutdown(Worker):
    def __init__(self, error):
        self.error = error
    def shutdown(self):
        raise self.error

for operation, expected in [(lambda: native.unregister_worker('worker'), unregistration),
                            (native.clear_workers, clearing)]:
    native.register_worker(RejectShutdown(expected))
    try:
        operation()
    except Exception as error:
        assert error is expected, f'shutdown callback changed identity: {error!r}'
    else:
        raise AssertionError('shutdown callback failure was swallowed')

try:
    native.unregister_worker('native-only')
except RuntimeError as error:
    assert type(error) is RuntimeError
    assert str(error) == 'native failure', str(error)
else:
    raise AssertionError('native unregistration failure was swallowed')

try:
    native.reject_natively()
except native.NativeFailureError as error:
    assert str(error) == 'native failure', str(error)
else:
    raise AssertionError('native failure was swallowed')
",
                Some(&globals),
                None,
            )
            .unwrap();
        });
    }
}
