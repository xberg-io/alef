"""Offline release discovery uses GitHub's published-only by-tag contract."""

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.tests.test_github_release_state import SPEC, RecordingApi, archives, load_helper, rejected


class DraftDiscoveryTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.api = RecordingApi(self.module)
        self.state = self.module.ReleaseState(SPEC, self.api)
        self.api.current = {
            "id": 23,
            "tag_name": SPEC["tag"],
            "target_commitish": SPEC["source_sha"],
            "body": self.state.binding(),
            "draft": True,
            "prerelease": False,
            "assets": [],
        }

    def test_exact_source_bound_draft_is_found_by_list_then_verified_by_id(self):
        state = self.state.inspect()
        assert state["release"] == self.api.current
        assert state["build_needed"] is True
        assert ("GET", f"/releases?per_page={self.module.RELEASE_PAGE_SIZE}&page=1", None) in self.api.calls
        assert ("GET", "/releases/23", None) in self.api.calls
        assert self.api.writes == []

    def test_existing_draft_retry_does_not_create_another_release(self):
        assert self.state.prepare()["release"]["id"] == 23
        assert self.api.writes == []

    def test_exact_draft_on_second_page_is_not_confused_with_other_tags(self):
        self.api.release_pages = {
            1: [{"id": index + 100, "tag_name": f"v0.0.{index}"} for index in range(self.module.RELEASE_PAGE_SIZE)],
            2: [self.api.current],
        }
        assert self.state.inspect()["release"]["id"] == 23
        assert ("GET", f"/releases?per_page={self.module.RELEASE_PAGE_SIZE}&page=2", None) in self.api.calls
        assert self.api.writes == []

    def test_duplicate_exact_tags_across_pages_fail_without_writes(self):
        other = [{"id": index + 100, "tag_name": f"v0.0.{index}"} for index in range(self.module.RELEASE_PAGE_SIZE - 1)]
        self.api.release_pages = {1: [self.api.current, *other], 2: [self.api.current]}
        with rejected(self.module.ReleaseError, "Ambiguous"):
            self.state.prepare()
        assert self.api.writes == []

    def test_absence_is_not_reported_until_all_pages_are_checked(self):
        self.api.current = None
        self.api.release_pages = {
            1: [{"id": index + 100, "tag_name": f"v0.0.{index}"} for index in range(self.module.RELEASE_PAGE_SIZE)],
            2: [],
        }
        assert self.state.inspect()["release"] is None
        assert ("GET", f"/releases?per_page={self.module.RELEASE_PAGE_SIZE}&page=2", None) in self.api.calls
        assert self.api.writes == []

    def test_scan_limit_fails_closed_instead_of_declaring_absence(self):
        page = [{"id": index + 100, "tag_name": f"v0.0.{index}"} for index in range(self.module.RELEASE_PAGE_SIZE)]
        self.api.release_pages = {1: page, 2: page}
        with patch.object(self.module, "MAX_RELEASE_PAGES", 2), rejected(self.module.ReleaseError, "limit"):
            self.state.prepare()
        assert self.api.writes == []

    def test_declared_scan_bound_checks_ten_thousand_releases_before_failing_closed(self):
        page = [{"id": index + 100, "tag_name": f"v0.0.{index}"} for index in range(self.module.RELEASE_PAGE_SIZE)]
        self.api.release_pages = dict.fromkeys(range(1, self.module.MAX_RELEASE_PAGES + 1), page)
        with rejected(self.module.ReleaseError, "limit"):
            self.state.inspect()
        queries = [endpoint for _, endpoint, _ in self.api.calls if endpoint.startswith("/releases?")]
        assert len(queries) == self.module.MAX_RELEASE_PAGES
        assert len(queries) * len(page) == 10_000
        assert queries == [
            f"/releases?per_page={self.module.RELEASE_PAGE_SIZE}&page={number}"
            for number in range(1, self.module.MAX_RELEASE_PAGES + 1)
        ]
        assert self.api.writes == []

    def test_malformed_listing_or_identity_never_becomes_absence(self):
        for page in (
            {},
            None,
            [None],
            [{}],
            [{"id": True, "tag_name": SPEC["tag"]}],
            [self.api.current] * (self.module.RELEASE_PAGE_SIZE + 1),
        ):
            with self.subTest(page=page):
                self.api.release_pages = {1: page}
                with rejected(self.module.ReleaseError):
                    self.state.prepare()
                assert self.api.writes == []

    def test_wrong_source_marker_or_id_is_rejected_before_any_mutation(self):
        for key, value in (
            ("body", self.state.binding().replace(SPEC["source_sha"], "b" * 40)),
            ("body", "legacy release"),
            ("id", 24),
            ("tag_name", "v0.0.0"),
        ):
            original = self.api.current.copy()
            self.api.release_pages = {1: [original]}
            self.api.current[key] = value
            with self.subTest(key=key), rejected(self.module.ReleaseError):
                self.state.prepare()
            assert self.api.writes == []
            self.api.current = original

    def test_non404_lookup_failures_are_not_treated_as_absence(self):
        original = self.api.request
        for status in (401, 403, 429, 500):

            def lookup_error(method, endpoint, payload=None, content=None, status=status):
                if endpoint.startswith("/releases/tags/"):
                    raise self.module.ReleaseError("lookup failed", status)
                return original(method, endpoint, payload, content)

            self.api.request = lookup_error
            with self.subTest(status=status), rejected(self.module.ReleaseError):
                self.state.prepare()
            assert self.api.writes == []

    def test_release_list_failures_are_not_treated_as_absence(self):
        for status in (401, 403, 404, 429, 500):

            def list_error(_page, status=status):
                raise self.module.ReleaseError("list failed", status)

            self.api.release_page = list_error
            with self.subTest(status=status), rejected(self.module.ReleaseError):
                self.state.prepare()
            assert self.api.writes == []

    def test_created_draft_is_verified_by_id_while_tag_and_list_discovery_are_stale(self):
        self.api.current = None
        self.api.release_pages = {}
        state = self.state.prepare()
        assert state["release"]["id"] == 23
        assert state["release"]["draft"] is True
        assert state["assets"] == {}
        assert state["build_needed"] is True
        assert state["public_ready"] is False
        assert [(method, endpoint) for method, endpoint, _ in self.api.writes] == [("POST", "/releases")]
        endpoints = [endpoint for _, endpoint, _ in self.api.calls]
        created = endpoints.index("/releases")
        assert endpoints[created + 1 :] == ["/git/ref/tags/" + SPEC["tag"], "/releases/23"]

    def test_five_asset_upload_discovers_83_pages_once_and_fits_the_request_budget(self):
        self.api.annotated = True
        self.api.release_pages = {
            page: [{"id": page * 100 + index, "tag_name": f"v0.{page}.{index}"} for index in range(10)]
            for page in range(1, 83)
        }
        self.api.release_pages[83] = [self.api.current]
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "cli"
            archives(directory)
            assert self.state.upload(directory, "success") == {"verified_remote": 5}
        gets = [endpoint for method, endpoint, _ in self.api.calls if method == "GET"]
        pages = [endpoint for endpoint in gets if endpoint.startswith("/releases?")]
        counts = (len(gets), len(pages), len(self.api.writes))
        assert counts == (122, 83, 5), counts
        assert pages == [f"/releases?per_page=10&page={page}" for page in range(1, 84)]
        assert gets.count("/releases/tags/" + SPEC["tag"]) == 1
        assert gets.count("/releases/23") == 12
        assert gets.count("/git/ref/tags/" + SPEC["tag"]) == 12
        assert gets.count("/git/tags/" + "c" * 40) == 12
        assert all(
            method == "POST" and endpoint.startswith("/releases/23/assets?") for method, endpoint, _ in self.api.writes
        )
        assert len(self.api.current["assets"]) == 5

    def test_admitted_release_is_refetched_by_id_and_all_binding_checks_remain_live(self):
        original = self.api.current.copy()
        for key, value in (
            ("id", 24),
            ("tag_name", "v0.0.0"),
            ("body", self.state.binding().replace(SPEC["source_sha"], "b" * 40)),
            ("body", "legacy release"),
            ("draft", 1),
            ("prerelease", True),
            ("assets", [{"name": SPEC["assets"][0], "digest": "sha256:" + "0" * 64}]),
        ):
            with self.subTest(field=key):
                self.api.current = original.copy()
                state = self.module.ReleaseState(SPEC, self.api)
                state.inspect()
                self.api.calls.clear()
                self.api.current[key] = value
                with rejected(self.module.ReleaseError):
                    state.inspect()
                assert [(method, endpoint) for method, endpoint, _ in self.api.calls] == [
                    ("GET", "/git/ref/tags/" + SPEC["tag"]),
                    ("GET", "/releases/23"),
                ]
                assert self.api.writes == []

    def test_admitted_release_deletion_or_api_failure_never_triggers_rediscovery(self):
        original = self.api.request
        for status in (404, 403, 429, 500):
            with self.subTest(status=status):
                self.api.request = original
                state = self.module.ReleaseState(SPEC, self.api)
                state.inspect()
                self.api.calls.clear()

                def fail_id(method, endpoint, payload=None, content=None, status=status):
                    if method == "GET" and endpoint == "/releases/23":
                        self.api.calls.append((method, endpoint, payload))
                        raise self.module.ReleaseError("release no longer readable", status)
                    return original(method, endpoint, payload, content)

                self.api.request = fail_id
                with rejected(self.module.ReleaseError, "no longer readable"):
                    state.inspect()
                assert [endpoint for _, endpoint, _ in self.api.calls] == [
                    "/git/ref/tags/" + SPEC["tag"],
                    "/releases/23",
                ]
                assert self.api.writes == []

    def test_repeated_inspection_still_rejects_tag_drift_before_reading_the_release(self):
        self.state.inspect()
        self.api.calls.clear()
        self.api.sha = "b" * 40
        with rejected(self.module.ReleaseError, "Tag no longer points"):
            self.state.inspect()
        assert self.api.calls == [("GET", "/git/ref/tags/" + SPEC["tag"], None)]

    def test_failed_binding_or_asset_admission_does_not_skip_discovery_on_retry(self):
        original = self.api.current.copy()
        for key, value in (("body", "legacy"), ("assets", None)):
            with self.subTest(field=key):
                self.api.current = dict(original, **{key: value})
                state = self.module.ReleaseState(SPEC, self.api)
                with rejected(self.module.ReleaseError):
                    state.inspect()
                self.api.current = original.copy()
                self.api.calls.clear()
                assert state.inspect()["release"]["id"] == 23
                assert [endpoint for _, endpoint, _ in self.api.calls] == [
                    "/git/ref/tags/" + SPEC["tag"],
                    "/releases/tags/" + SPEC["tag"],
                    "/releases?per_page=10&page=1",
                    "/releases/23",
                ]

    def test_each_release_state_independently_performs_initial_discovery(self):
        self.state.inspect()
        self.api.calls.clear()
        fresh = self.module.ReleaseState(SPEC, self.api)
        assert fresh.inspect()["release"]["id"] == 23
        assert [endpoint for _, endpoint, _ in self.api.calls if endpoint.startswith("/releases?")] == [
            "/releases?per_page=10&page=1"
        ]
