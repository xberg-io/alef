#!/usr/bin/env python3
"""A stdio sidecar that speaks protocol 2 when the host offers it, serving calls concurrently.

SCENARIO is `binary`, `binary/<max_in_flight>`, `json` (an old sidecar: protocol 1 only), or
`json-with-limit` (an invalid protocol-1 descriptor carrying `max_in_flight`).
"""

import json
import os
import struct
import sys
import threading
import time

DESCRIPTOR = json.loads("""__DESCRIPTOR__""")
SCENARIO = "__SCENARIO__"
MAGIC = 0xA2
CALL = 1
RESPONSE = 2
PREFIX = struct.Struct("<BBHQII")
WRITE_LOCK = threading.Lock()
STATE_LOCK = threading.Lock()
STATE = {"calls": 0, "active": 0, "peak": 0}


def read_exact(size: int) -> bytes:
    """Read exactly `size` bytes or exit when the host closes its pipe."""
    data = sys.stdin.buffer.read(size)
    if len(data) != size:
        sys.exit(0)
    return data


def write(data: bytes) -> None:
    """Write one whole frame; frames from concurrent handlers never interleave."""
    with WRITE_LOCK:
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()


def respond_json(status: int, payload: bytes) -> None:
    """Write a protocol-1 response frame."""
    body = json.dumps({"status": status, "payload": list(payload)}).encode()
    write(struct.pack("<I", len(body)) + body)


def respond_binary(request_id: int, status: int, payload: bytes) -> None:
    """Write a protocol-2 response frame."""
    write(PREFIX.pack(MAGIC, RESPONSE, 0, request_id, 4, len(payload)) + struct.pack("<i", status) + payload)


def handle(method: str, payload: bytes, mode: str) -> tuple[int, bytes]:
    """Answer one call."""
    with STATE_LOCK:
        STATE["calls"] += 1
        calls = STATE["calls"]
    if method == "sleep":
        time.sleep(float(payload))
        return 0, payload
    if method == "hang":
        time.sleep(60)
    if method == "exit":
        os._exit(7)
    if method == "failure":
        return 3, b"producer failed"
    if method == "count":
        return 0, str(calls).encode()
    if method == "pid":
        return 0, str(os.getpid()).encode()
    if method == "mode":
        return 0, mode.encode()
    if method == "peak":
        with STATE_LOCK:
            return 0, str(STATE["peak"]).encode()
    return 0, payload


def serve_call(request_id: int, method: str, payload: bytes) -> None:
    """Serve one protocol-2 call on its own thread, tracking how many run at once."""
    with STATE_LOCK:
        STATE["active"] += 1
        STATE["peak"] = max(STATE["peak"], STATE["active"])
    try:
        if method == "bad_id":
            respond_binary(request_id + 1000, 0, b"stray")
            return
        if method == "garbage":
            write(b"\x00" * PREFIX.size)
            return
        status, body = handle(method, payload, "binary")
        respond_binary(request_id, status, body)
    finally:
        with STATE_LOCK:
            STATE["active"] -= 1


def serve_binary() -> None:
    """Read protocol-2 call frames and dispatch each to a thread."""
    while True:
        magic, kind, flags, request_id, header_len, payload_len = PREFIX.unpack(read_exact(PREFIX.size))
        if magic != MAGIC or kind != CALL or flags != 0:
            sys.exit(2)
        header = read_exact(header_len)
        payload = read_exact(payload_len)
        contract_len = struct.unpack_from("<H", header)[0]
        method = header[2 + contract_len :].decode()
        threading.Thread(target=serve_call, args=(request_id, method, payload), daemon=True).start()


def serve_json() -> None:
    """Serve protocol-1 envelopes one at a time."""
    while True:
        size = struct.unpack("<I", read_exact(4))[0]
        request = json.loads(read_exact(size))
        status, body = handle(request["method"], bytes(request["payload"]), "json")
        respond_json(status, body)


def main() -> None:
    """Answer describe in protocol 1, then switch to the negotiated protocol."""
    size = struct.unpack("<I", read_exact(4))[0]
    if json.loads(read_exact(size))["op"] != "describe":
        sys.exit(3)
    offered = os.environ.get("ALEF_STDIO_PROTOCOLS", "").split(",")
    binary = SCENARIO.startswith("binary") and "2" in offered
    if binary:
        DESCRIPTOR["protocol_version"] = 2
        if "/" in SCENARIO:
            DESCRIPTOR["max_in_flight"] = int(SCENARIO.split("/", 1)[1])
    if SCENARIO == "json-with-limit":
        DESCRIPTOR["max_in_flight"] = 4
    respond_json(0, json.dumps(DESCRIPTOR).encode())
    if binary:
        serve_binary()
    else:
        serve_json()


main()
