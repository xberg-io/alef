import asyncio
import contextvars
import os
import subprocess
import sys
import time
from collections.abc import Awaitable, Callable
from contextlib import suppress
from pathlib import Path
from typing import Protocol, cast


class _Native(Protocol):
    def shutdown_async_runtime(self) -> None: ...
    def fetch(self) -> asyncio.Future[str]: ...
    def context_echo(self, callback: Callable[[], Awaitable[str]]) -> Awaitable[str]: ...


native = cast("_Native", globals()["native"])


def _thread_count() -> int:
    if sys.platform == "linux":
        return len(list(Path("/proc/self/task").iterdir()))
    return len(subprocess.check_output(["/bin/ps", "-M", "-p", str(os.getpid())]).splitlines()) - 1


def _await_thread_count(expected: int, timeout: float = 2.0) -> int:
    # ~keep Linux can retain a joined worker in /proc briefly; a deadline still rejects persistent thread leaks.
    deadline = time.monotonic() + timeout
    count = _thread_count()
    while count != expected and time.monotonic() < deadline:
        time.sleep(0.01)
        count = _thread_count()
    return count


def _fd_count() -> int:
    return len(list(Path("/proc/self/fd" if sys.platform == "linux" else "/dev/fd").iterdir()))


async def _verify() -> None:
    native.shutdown_async_runtime()
    threads, fds = _thread_count(), _fd_count()
    assert await native.fetch() == "done"
    native.shutdown_async_runtime()
    assert _await_thread_count(threads) == threads
    # ~keep Tokio's signal registry retains one process-global socket pair, outside runtime ownership.
    assert _fd_count() <= fds + 2, (fds, _fd_count())
    fds = _fd_count()
    for _ in range(3):
        assert await native.fetch() == "done"
        assert _thread_count() > threads
        native.shutdown_async_runtime()
        assert _await_thread_count(threads) == threads
        assert _fd_count() == fds, (fds, _fd_count())
    pending = native.fetch()
    try:
        native.shutdown_async_runtime()
    except RuntimeError as error:
        message = str(error)
    else:
        raise AssertionError("shutdown must reject active work")
    assert "active" in message
    pending.cancel()
    with suppress(asyncio.CancelledError):
        await pending
    await asyncio.sleep(0)
    native.shutdown_async_runtime()
    assert _await_thread_count(threads) == threads
    assert _fd_count() == fds
    assert await native.fetch() == "done"
    native.shutdown_async_runtime()
    value = contextvars.ContextVar("value", default="unset")
    value.set("preserved")

    async def callback() -> str:
        return value.get()

    assert await native.context_echo(callback) == "preserved"
    native.shutdown_async_runtime()


asyncio.run(_verify())
