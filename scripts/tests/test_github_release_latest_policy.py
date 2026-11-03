"""Source-bound controls for keeping maintenance releases out of GitHub Latest."""

import base64
import tempfile
import unittest
from contextlib import chdir
from pathlib import Path
from urllib.parse import urlsplit

if __package__:
    from .test_github_release_state import SPEC, RecordingApi, archives, load_helper, rejected
else:
    from test_github_release_state import SPEC, RecordingApi, archives, load_helper, rejected


class MetadataApi(RecordingApi):
    def __init__(self, module, metadata=""):
        super().__init__(module)
        self.metadata = metadata

    def request(self, method, endpoint, payload=None, content=None):
        response = super().request(method, endpoint, payload, content)
        if method == "GET" and urlsplit(endpoint).path == "/contents/Cargo.toml":
            manifest = base64.b64decode(response["content"]).decode() + self.metadata
            response["content"] = base64.b64encode(manifest.encode()).decode()
        return response


def release_policy(value):
    return "\n[package.metadata.alef.release]\nmake_latest = " + value + "\n"


class MaintenanceLatestPolicyTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / "cli"
        archives(self.directory)

    def ready(self, metadata="", version=SPEC["version"]):
        api = MetadataApi(self.module, metadata)
        api.source_version = version
        spec = dict(SPEC, version=version, tag="v" + version)
        state = self.module.ReleaseState(spec, api)
        state.prepare()
        state.upload(self.directory, "success")
        return (state, api)

    def test_false_source_policy_excludes_stable_promotion_from_latest(self):
        state, api = self.ready(release_policy("false"))
        state.promote("success", "success", True)
        assert api.writes[-1][2] == {"draft": False, "make_latest": "false"}

    def test_true_source_policy_preserves_stable_server_selection(self):
        state, api = self.ready(release_policy("true"))
        state.promote("success", "success", True)
        assert api.writes[-1][2] == {"draft": False, "make_latest": "legacy"}

    def test_absent_source_policy_preserves_stable_server_selection(self):
        state, api = self.ready()
        state.promote("success", "success", True)
        assert api.writes[-1][2] == {"draft": False, "make_latest": "legacy"}

    def test_prereleases_cannot_select_latest_with_either_boolean(self):
        for value in ("true", "false"):
            with self.subTest(value=value):
                state, api = self.ready(release_policy(value), "0.115.16-rc.1")
                assert api.writes[0][2]["make_latest"] == "false"
                state.promote("success", "success", True)
                assert api.writes[-1][2]["make_latest"] == "false"

    def test_controller_manifest_cannot_override_the_tagged_source_policy(self):
        for source, controller, expected in (("false", "true", "false"), ("true", "false", "legacy")):
            with self.subTest(source=source):
                controller_root = Path(self.temporary.name) / ("controller-" + source)
                controller_root.mkdir()
                (controller_root / "Cargo.toml").write_text(release_policy(controller))
                with chdir(controller_root):
                    state, api = self.ready(release_policy(source))
                    state.promote("success", "success", True)
                assert api.writes[-1][2]["make_latest"] == expected
                source_reads = [endpoint for _, endpoint, _ in api.calls if endpoint.startswith("/contents/")]
                assert len(source_reads) == 2
                assert all("ref=" + SPEC["source_sha"] in endpoint for endpoint in source_reads)

    def test_nonboolean_policy_is_rejected_before_any_publication_write(self):
        for value in ('"false"', '"legacy"', "0", "1", "[]", "{}"):
            with self.subTest(value=value):
                api = MetadataApi(self.module, release_policy(value))
                state = self.module.ReleaseState(SPEC, api)
                with rejected(self.module.ReleaseError, match="Latest policy"):
                    state.prepare()
                assert api.writes == []

    def test_malformed_policy_tables_are_rejected_before_any_publication_write(self):
        for metadata in (
            "\n[package.metadata]\nalef = false\n",
            "\n[package.metadata.alef]\nrelease = false\n",
            "\n[package.metadata.alef.release]\nmake-latest = false\n",
        ):
            with self.subTest(metadata=metadata):
                api = MetadataApi(self.module, metadata)
                state = self.module.ReleaseState(SPEC, api)
                with rejected(self.module.ReleaseError, match="Latest policy"):
                    state.prepare()
                assert api.writes == []

    def test_moved_tag_cannot_authorize_maintenance_promotion(self):
        state, api = self.ready(release_policy("false"))
        api.sha = "b" * 40
        prior = list(api.writes)
        with rejected(self.module.ReleaseError, match="Tag no longer points"):
            state.promote("success", "success", True)
        assert api.writes == prior

    def test_wrong_source_version_cannot_authorize_the_policy(self):
        api = MetadataApi(self.module, release_policy("false"))
        api.source_version = "0.115.16"
        state = self.module.ReleaseState(SPEC, api)
        with rejected(self.module.ReleaseError, match="Tag/version/source SHA mismatch"):
            state.prepare()
        assert api.writes == []

    def test_complete_public_maintenance_retry_does_not_patch_release_state(self):
        state, api = self.ready(release_policy("false"))
        state.promote("success", "success", True)
        prior = list(api.writes)
        state.promote("skipped", "success", True)
        assert api.writes == prior


if __name__ == "__main__":
    unittest.main()
