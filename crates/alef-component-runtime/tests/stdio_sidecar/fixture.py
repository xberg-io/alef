#!/usr/bin/env python3
import json
import os
import struct
import sys
import time

DESCRIPTOR = json.loads("""__DESCRIPTOR__""")
SCENARIO = "__SCENARIO__"
CALLS = 0


def respond(status: int, payload: bytes) -> None:
    """Write a complete response frame."""
    body = json.dumps({"status": status, "payload": list(payload)}).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(body)) + body)
    sys.stdout.buffer.flush()


def read_exact(size: int) -> bytes:
    """Read a request or exit when the host closes its pipe."""
    data = sys.stdin.buffer.read(size)
    if len(data) != size:
        sys.exit(0)
    return data


while True:
    size = struct.unpack("<I", read_exact(4))[0]
    request = json.loads(read_exact(size))
    if request["op"] == "describe":
        if SCENARIO == "bad_hash":
            DESCRIPTOR["provides"][0]["contract_hash"] = "00" * 32
        if SCENARIO == "bad_version":
            DESCRIPTOR["protocol_version"] = 99
        if SCENARIO == "bad_identity":
            DESCRIPTOR["component_id"] = "impostor"
        if SCENARIO == "bad_feature":
            DESCRIPTOR["feature_set_hash"] = "00" * 32
        respond(0, json.dumps(DESCRIPTOR).encode())
        continue
    CALLS += 1
    method = request["method"]
    if method == "hang":
        time.sleep(60)
    elif method == "exit":
        sys.exit(7)
    elif method == "truncated":
        sys.stdout.buffer.write(struct.pack("<I", 20) + b"abc")
        sys.stdout.buffer.flush()
        sys.exit(0)
    elif method == "oversized":
        sys.stdout.buffer.write(struct.pack("<I", 16 * 1024 * 1024 + 1))
        sys.stdout.buffer.flush()
    elif method == "malformed":
        sys.stdout.buffer.write(struct.pack("<I", 1) + b"{")
        sys.stdout.buffer.flush()
    elif method == "failure":
        respond(3, b"producer failed")
    elif method == "count":
        respond(0, str(CALLS).encode())
    elif method == "pid":
        respond(0, str(os.getpid()).encode())
    else:
        respond(0, bytes(request["payload"]))
