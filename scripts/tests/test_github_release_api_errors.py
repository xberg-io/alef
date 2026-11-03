"""Publication failures retain useful HTTP context without leaking credentials."""

import io
import json
import os
import runpy
import tempfile
import unittest
from contextlib import redirect_stderr
from http.client import IncompleteRead
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError

from scripts.tests.test_github_release_state import HELPER, SPEC, load_helper


class TrackedErrorBody(io.BytesIO):
    def __init__(self, value, read_error=None):
        super().__init__(value)
        self.read_sizes = []
        self.read_error = read_error

    def read(self, size=-1):
        self.read_sizes.append(size)
        if self.read_error is not None:
            raise self.read_error
        return super().read(size)


class HttpErrorDiagnosticsTest(unittest.TestCase):
    def setUp(self):
        self.body = b'{"message":"Resource not accessible by integration"}'
        self.headers = {}
        self.read_error = None
        environment = patch.dict(os.environ, {"GH_TOKEN": "offline-dummy-only"}, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        opener = patch("urllib.request.build_opener", self.opener)
        opener.start()
        self.addCleanup(opener.stop)
        self.module = load_helper()

    def opener(self, _redirect_handler):
        outer = self

        class Opener:
            def open(self, request, timeout):
                assert timeout == 90
                outer.stream = TrackedErrorBody(outer.body, outer.read_error)
                raise HTTPError(request.full_url, 403, "private-reason", outer.headers, outer.stream)

        return Opener()

    def failed_request(self, endpoint="/releases"):
        try:
            self.module.GithubApi().request("POST", endpoint)
        except self.module.ReleaseError as error:
            return error
        raise AssertionError("Expected an HTTP publication failure")

    def test_safe_message_and_only_numeric_rate_limit_headers_are_reported(self):
        self.body = json.dumps({"message": "Resource not accessible by integration", "secret": "omit-me"}).encode()
        self.headers = {
            "Retry-After": "60",
            "X-RateLimit-Limit": "5000",
            "X-RateLimit-Remaining": "0",
            "X-RateLimit-Reset": "1793664000",
            "Authorization": "Bearer header-secret",
            "Set-Cookie": "cookie-secret",
        }
        error = self.failed_request("/releases?access_token=query-secret")
        assert error.status == 403
        assert str(error) == (
            "Publication API POST /repos/xberg-io/alef/releases returned HTTP 403; "
            'message="Resource not accessible by integration"; retry-after=60; x-ratelimit-limit=5000; '
            "x-ratelimit-remaining=0; x-ratelimit-reset=1793664000"
        )
        assert self.stream.closed

    def test_invalid_or_oversized_body_and_unsafe_header_values_are_omitted(self):
        self.headers = {"Retry-After": "60\nsecret", "X-RateLimit-Remaining": "header-secret"}
        bodies = [b"private html", b"\xff", b"[]", b'{"message":42}', b"[" * 1500, b"x" * 4097]
        for body in bodies:
            with self.subTest(body_length=len(body)):
                self.body = body
                error = self.failed_request()
                assert str(error) == "Publication API POST /repos/xberg-io/alef/releases returned HTTP 403"
                assert self.stream.closed
                assert self.stream.read_sizes == [4097]

    def test_credentials_urls_and_control_characters_are_redacted_from_message(self):
        self.body = json.dumps(
            {
                "message": (
                    "offline-dummy-only https://user:url-password@example.invalid/?token=url-secret "
                    "Authorization: Bearer bearer-secret github_pat_privateToken access_token=assigned-secret\n\x1b"
                )
            }
        ).encode()
        message = str(self.failed_request("/releases?token=query-secret"))
        for forbidden in (
            "offline-dummy-only",
            "url-password",
            "url-secret",
            "bearer-secret",
            "privateToken",
            "assigned-secret",
            "query-secret",
            "Authorization",
            "\n",
            "\x1b",
        ):
            assert forbidden not in message, forbidden
        assert "[redacted]" in message

    def test_message_is_bounded_and_path_credentials_are_redacted_before_truncation(self):
        self.body = json.dumps({"message": "x" * 700}).encode()
        error = self.failed_request("/releases/offline-dummy-only?token=query-secret")
        assert str(error) == (
            'Publication API POST /repos/xberg-io/alef/releases/[redacted] returned HTTP 403; message="'
            + "x" * 512
            + '"'
        )

    def cli_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            spec = Path(temporary) / "release-spec.json"
            spec.write_text(json.dumps(SPEC))
            stderr = io.StringIO()
            exit_code = None
            with patch("sys.argv", [str(HELPER), "prepare", "--spec", str(spec)]), redirect_stderr(stderr):
                try:
                    runpy.run_path(str(HELPER), run_name="__main__")
                except SystemExit as error:
                    exit_code = error.code
        return exit_code, stderr.getvalue()

    def test_actual_cli_exits_nonzero_with_the_safe_http_failure_reason(self):
        exit_code, stderr = self.cli_failure()
        assert exit_code == 1
        assert stderr == (
            "Publication API GET /repos/xberg-io/alef/git/ref/tags/v0.115.11 returned HTTP 403; "
            'message="Resource not accessible by integration"\n'
        )

    def test_truncated_error_body_preserves_http_status_and_safe_cli_failure(self):
        self.read_error = IncompleteRead(b'{"message":"offline-dummy-only partial-secret', 100)
        self.headers = {"Retry-After": "60"}
        with self.subTest(boundary="request"):
            error = self.failed_request()
            assert error.status == 403
            assert str(error) == "Publication API POST /repos/xberg-io/alef/releases returned HTTP 403; retry-after=60"
            assert self.stream.closed
            assert self.stream.read_sizes == [4097]
        with self.subTest(boundary="cli"):
            exit_code, stderr = self.cli_failure()
            assert exit_code == 1
            assert stderr == (
                "Publication API GET /repos/xberg-io/alef/git/ref/tags/v0.115.11 returned HTTP 403; retry-after=60\n"
            )
            assert self.stream.closed
            assert self.stream.read_sizes == [4097]
