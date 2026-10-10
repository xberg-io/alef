# ~keep Standalone subprocess assertions deliberately exercise private atexit hooks without pytest.
# ruff: noqa: INP001, S101, SLF001, PT017
import asyncio
import atexit
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

scenario, mode = sys.argv[1:]


def _threads() -> int:
    if sys.platform == "linux":
        return len(list(Path("/proc/self/task").iterdir()))
    return len(subprocess.check_output(["/bin/ps", "-M", "-p", str(os.getpid())], text=True).splitlines()) - 1


def _await_baseline_threads(timeout: float = 2.0) -> int:
    # ~keep Runtime teardown can lag the shutdown call on Linux, so the OS reaps a worker a
    # moment after the exit hook returns; settle before asserting instead of racing it.
    deadline = time.monotonic() + timeout
    count = _threads()
    while count != baseline and time.monotonic() < deadline:
        time.sleep(0.01)
        count = _threads()
    return count


baseline = _threads()


async def _fetch() -> str:
    return await native.fetch()


def _closed_observer() -> None:
    try:
        assert _await_baseline_threads() == baseline, (_threads(), baseline)
        try:
            asyncio.run(_fetch())
        except RuntimeError as error:
            assert "closed" in str(error), error
        else:
            raise AssertionError("async work restarted after exit hook")
        try:
            native.unread_stream()
        except RuntimeError as error:
            assert "closed" in str(error), error
        else:
            raise AssertionError("producer restarted after exit hook")
        assert _await_baseline_threads() == baseline
        sys.stdout.write("LIFECYCLE_OK\n")
        sys.stdout.flush()
    except BaseException as error:
        sys.stdout.write(f"LIFECYCLE_FAILED: {error!r}\n")
        sys.stdout.flush()


# Register before import to observe the internal hook under atexit's LIFO order. ~keep
atexit.register(_closed_observer)
import _sample as native  # noqa: E402

signal.alarm(12)
if scenario == "fork-cold":
    child = os.fork()
    if child == 0:
        signal.alarm(10)
        assert asyncio.run(_fetch()) == "done"
        atexit._run_exitfuncs()
        os._exit(0)
    assert os.waitpid(child, 0)[1] == 0
elif scenario == "fork-warm":
    assert asyncio.run(_fetch()) == "done"
    child = os.fork()
    if child == 0:
        signal.alarm(10)
        try:
            asyncio.run(_fetch())
        except RuntimeError as error:
            assert "fork" in str(error), error
        else:
            os._exit(2)
        try:
            native.unread_stream()
        except RuntimeError as error:
            assert "fork" in str(error), error
        else:
            os._exit(4)
        if mode == "managed":
            try:
                native.shutdown_async_runtime()
            except RuntimeError as error:
                assert "fork" in str(error), error
            else:
                os._exit(3)
        atexit.unregister(_closed_observer)
        # Exercise the inherited exit hook without taking any parent mutex. ~keep
        atexit._run_exitfuncs()
        os._exit(0)
    assert os.waitpid(child, 0)[1] == 0
    assert asyncio.run(_fetch()) == "done"
else:
    assert asyncio.run(_fetch()) == "done"
    if scenario == "unread":
        unread = native.unread_stream()
    if scenario == "manual" and mode == "managed":
        native.shutdown_async_runtime()
        assert asyncio.run(_fetch()) == "done"
