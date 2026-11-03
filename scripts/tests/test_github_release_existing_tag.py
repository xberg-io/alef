"""Existing Git tags, rather than release target metadata, bind source identity."""

import json
import unittest
from unittest.mock import patch

from scripts.tests.test_github_release_state import SPEC, RecordingApi, load_helper, rejected


class ExistingTagTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.api = RecordingApi(self.module)
        self.state = self.module.ReleaseState(SPEC, self.api)

    def draft(self):
        return {
            "id": 23,
            "tag_name": SPEC["tag"],
            "target_commitish": "main",
            "body": self.state.binding(),
            "draft": True,
            "prerelease": False,
            "assets": [],
        }

    def test_existing_tag_creation_omits_unused_target_commitish(self):
        prepared = self.state.prepare()
        assert prepared["release"]["id"] == 23
        assert len(self.api.writes) == 1
        method, endpoint, payload = self.api.writes[0]
        assert (method, endpoint) == ("POST", "/releases")
        assert payload == {
            "tag_name": SPEC["tag"],
            "name": SPEC["tag"],
            "make_latest": "false",
            "body": self.state.binding(),
            "draft": True,
            "prerelease": False,
        }

    def test_annotated_tag_and_exact_marker_admit_default_branch_release_metadata(self):
        self.api.annotated = True
        self.api.current = self.draft()
        prepared = self.state.prepare()
        assert prepared["release"] == self.api.current
        assert prepared["build_needed"] is True
        assert self.api.calls.count(("GET", "/git/tags/" + "c" * 40, None)) == 1
        assert len([call for call in self.api.calls if call[1].startswith("/contents/")]) == 2
        assert self.api.writes == []

    def test_creation_accepts_provider_default_metadata_after_source_verification(self):
        original = self.api.request

        def provider_default(method, endpoint, payload=None, content=None):
            response = original(method, endpoint, payload, content)
            if (method, endpoint) == ("POST", "/releases"):
                self.api.current["target_commitish"] = "main"
            return response

        with patch.object(self.api, "request", provider_default):
            assert self.state.prepare()["release"]["target_commitish"] == "main"
        assert len(self.api.writes) == 1
        assert "target_commitish" not in self.api.writes[0][2]
        assert self.api.current["body"] == self.state.binding()

    def test_default_metadata_does_not_authorize_wrong_source_marker(self):
        self.api.current = self.draft()
        wrong_source = dict(SPEC, source_sha="b" * 40)
        self.api.current["body"] = "<!-- alef-release-state:v1 " + json.dumps(wrong_source) + " -->"
        with rejected(self.module.ReleaseError, "different source"):
            self.state.prepare()
        assert self.api.writes == []

    def test_default_metadata_does_not_authorize_moved_or_missing_tag(self):
        self.api.current = self.draft()
        self.api.sha = "b" * 40
        with rejected(self.module.ReleaseError, "Tag no longer points"):
            self.state.prepare()
        assert self.api.writes == []
        self.api.sha = SPEC["source_sha"]
        original = self.api.request

        def missing_tag(method, endpoint, payload=None, content=None):
            if endpoint.startswith("/git/ref/tags/"):
                raise self.module.ReleaseError("missing tag", 404)
            return original(method, endpoint, payload, content)

        with patch.object(self.api, "request", missing_tag), rejected(self.module.ReleaseError, "missing tag"):
            self.state.prepare()
        assert self.api.writes == []

    def test_creation_still_rejects_tag_drift_after_post(self):
        original = self.api.request

        def drift_after_create(method, endpoint, payload=None, content=None):
            response = original(method, endpoint, payload, content)
            if (method, endpoint) == ("POST", "/releases"):
                self.api.sha = "b" * 40
            return response

        with (
            patch.object(self.api, "request", drift_after_create),
            rejected(self.module.ReleaseError, "Tag no longer points"),
        ):
            self.state.prepare()
        assert [(method, endpoint) for method, endpoint, _ in self.api.writes] == [("POST", "/releases")]

    def test_creation_still_rejects_a_different_returned_release_id(self):
        original = self.api.request

        def wrong_created_id(method, endpoint, payload=None, content=None):
            response = original(method, endpoint, payload, content)
            if (method, endpoint) == ("POST", "/releases"):
                return dict(response, id=24)
            return response

        with (
            patch.object(self.api, "request", wrong_created_id),
            rejected(self.module.ReleaseError, "prepared.*verified"),
        ):
            self.state.prepare()
        assert [(method, endpoint) for method, endpoint, _ in self.api.writes] == [("POST", "/releases")]

    def test_tag_is_rechecked_after_discovery_before_creation(self):
        original = self.api.release_page

        def drift_during_discovery(page):
            response = original(page)
            self.api.sha = "b" * 40
            return response

        with (
            patch.object(self.api, "release_page", drift_during_discovery),
            rejected(self.module.ReleaseError, "Tag no longer points"),
        ):
            self.state.prepare()
        assert self.api.writes == []
