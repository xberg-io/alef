#!/usr/bin/env python3
import json
import os
import struct
import sys
from pathlib import Path

DESCRIPTOR = json.loads("""__DESCRIPTOR__""")
MAX_FRAME = 16 * 1024 * 1024


def read_exact(size: int) -> bytes:
    """Read one frame portion or exit when the host closes the connection."""
    result = sys.stdin.buffer.read(size)
    if len(result) != size:
        sys.exit(0)
    return result


def respond(payload: bytes) -> None:
    """Return one successful response through the real stdio pipe."""
    body = json.dumps({"status": 0, "payload": list(payload)}).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(body)) + body)
    sys.stdout.buffer.flush()


while True:
    length = struct.unpack("<I", read_exact(4))[0]
    if not 0 < length <= MAX_FRAME:
        raise ValueError("invalid frame length")
    request = json.loads(read_exact(length))
    if request["op"] == "describe":
        respond(json.dumps(DESCRIPTOR).encode())
        continue
    if request["op"] != "call" or request["contract"] != "codec" or request["method"] != "transform":
        raise ValueError("unexpected contract method")
    payload = bytes(request["payload"])
    with Path(os.environ["ALEF_STDIO_AUDIT"]).open("a", encoding="utf-8") as audit:
        audit.write(json.dumps({"contract": "codec", "method": "transform", "payload": list(payload)}) + "\n")
    respond(payload[::-1])
