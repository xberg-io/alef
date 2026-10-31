"""Offline controls for the actual Alef publication boundary."""

import ast
import base64
import hashlib
import importlib.util
import io
import json
import os
import re
import stat
import subprocess
import tarfile
import tempfile
import unittest
import warnings
import zipfile
from contextlib import chdir, contextmanager
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts/publish/github-release-state.py"
TARGETS = json.loads((ROOT / ".github/cli-targets.json").read_text())
NAMES = [f"alef-{row['target']}.{row['archive_ext']}" for row in TARGETS]
SPEC = {"schema": 1, "tag": "v0.115.11", "version": "0.115.11", "source_sha": "a" * 40, "assets": NAMES}


def archive_member(name):
    suffix = ".zip" if name.endswith(".zip") else ".tar.gz"
    return name.removesuffix(suffix) + ("/alef.exe" if suffix == ".zip" else "/alef")


def write_archive(path, payload=b"nonempty executable fixture\n", mutation=None):
    member = archive_member(path.name)
    if mutation == "missing":
        member += ".not-the-binary"
    if mutation == "empty":
        payload = b""
    if path.name.endswith(".zip"):
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr(member.rsplit("/", 1)[0] + "/", b"")
            entry = zipfile.ZipInfo(member)
            if mutation == "symlink":
                entry.create_system = 3
                entry.external_attr = (stat.S_IFLNK | 0o777) << 16
            archive.writestr(entry, payload)
            if mutation == "duplicate":
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    archive.writestr(member, payload)
    else:
        with tarfile.open(path, "w:gz") as archive:
            directory = tarfile.TarInfo(member.rsplit("/", 1)[0])
            directory.type = tarfile.DIRTYPE
            archive.addfile(directory)
            entry = tarfile.TarInfo(member)
            entry.size = len(payload)
            entry.mode = 0o755
            if mutation == "symlink":
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/unowned/target"
                entry.size = 0
            archive.addfile(entry, io.BytesIO(payload))
            if mutation == "duplicate":
                archive.addfile(entry, io.BytesIO(payload))


def archives(directory):
    directory.mkdir(parents=True)
    for name in NAMES:
        write_archive(directory / name)
    assert len(list(directory.iterdir())) == 5


def workflow_guard():
    lines = (ROOT / ".github/workflows/publish.yaml").read_text().splitlines()
    start = lines.index("      - name: Verify CLI release assets are present")
    run = next(index for index in range(start, len(lines)) if lines[index] == "        run: |") + 1
    script = []
    for line in lines[run:]:
        if line and not line.startswith("          "):
            break
        script.append(line[10:])
    return "\n".join(script)


class WorkflowBoundaryTest(unittest.TestCase):
    def guard(self, missing=None):
        with tempfile.TemporaryDirectory() as temporary:
            cwd = Path(temporary)
            archives(cwd / "dist/cli")
            if missing:
                (cwd / "dist/cli" / missing).unlink()
            (cwd / "release-spec.json").write_text(json.dumps(SPEC))
            if HELPER.exists():
                target = cwd / "scripts/publish/github-release-state.py"
                target.parent.mkdir(parents=True)
                target.write_bytes(HELPER.read_bytes())
            return subprocess.run(
                ["bash", "-euo", "pipefail", "-c", workflow_guard()],
                cwd=cwd,
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )

    def test_actual_workflow_accepts_five_nonempty_real_archives(self):
        assert len(NAMES) == 5
        assert self.guard().returncode == 0

    def test_actual_workflow_rejects_four_archives(self):
        result = self.guard(NAMES[-1])
        assert result.returncode != 0, result.stdout

    def test_upload_cannot_publish_before_completeness(self):
        workflow = (ROOT / ".github/workflows/publish.yaml").read_text()
        upload = workflow.split("\n  upload-release-assets:", 1)[1].split("\n  publish-scoop-manifest:", 1)[0]
        assert 'draft: "false"' not in upload


def job_block(name, text=None):
    text = text or (ROOT / ".github/workflows/publish.yaml").read_text()
    match = re.search(r"^  " + re.escape(name) + r":\n(?:(?!^  [a-z]).|\n)*", text, re.MULTILINE)
    return match.group() if match else ""


def job_condition(name, values):
    block = job_block(name)
    match = re.search(r"^    if: (.+)(?:\n)?", block, re.MULTILINE)
    assert match, f"no condition for {name}"
    expression = match[1]
    if expression == "|":
        lines = []
        for line in block[match.end() :].splitlines():
            if not line.startswith("      "):
                break
            lines.append(line[6:])
        expression = " ".join(lines)
    expression = expression.replace("always()", "True").replace("&&", " and ").replace("||", " or ")
    expression = re.sub(
        r"needs\.[a-z_-]+\.(?:outputs\.[a-z_]+|result)", lambda match: repr(values.get(match[0], "")), expression
    )

    def evaluate(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, (str, bool)):
            return node.value
        if isinstance(node, ast.Name) and node.id == "True":
            return True
        if isinstance(node, ast.BoolOp):
            items = [evaluate(item) for item in node.values]
            return all(items) if isinstance(node.op, ast.And) else any(items)
        if isinstance(node, ast.Compare) and len(node.ops) == 1:
            left, right = evaluate(node.left), evaluate(node.comparators[0])
            if isinstance(node.ops[0], ast.Eq):
                return left == right
            if isinstance(node.ops[0], ast.NotEq):
                return left != right
        raise AssertionError(f"unsupported workflow condition: {ast.dump(node)}")

    return bool(evaluate(ast.parse(expression.strip(), mode="eval").body))


def core_dependencies_match(text):
    expected = {
        "upload-release-assets": {"prepare", "check-github-release", "build-cli"},
        "publish-crates": {"prepare", "check-cratesio", "check-github-release", "upload-release-assets"},
        "promote": {"prepare", "check-github-release", "build-cli", "upload-release-assets", "publish-crates"},
        "public-ready": {"prepare", "promote", "publish-crates"},
        "publish-homebrew-formula": {"prepare", "check-homebrew", "publish-crates", "public-ready"},
        "publish-scoop-manifest": {"prepare", "check-scoop", "public-ready"},
    }
    for name, dependencies in expected.items():
        match = re.search(r"^    needs: \[([^\]]+)\]$", job_block(name, text), re.MULTILINE)
        if not match or set(match[1].replace(" ", "").split(",")) != dependencies:
            return False
    return True


class WorkflowDagTest(unittest.TestCase):
    def test_actual_dag_rejects_removed_asset_crate_or_public_ready_dependency(self):
        text = (ROOT / ".github/workflows/publish.yaml").read_text()
        assert core_dependencies_match(text)
        for name, dependency in (
            ("promote", "upload-release-assets"),
            ("promote", "publish-crates"),
            ("upload-release-assets", "build-cli"),
            ("publish-scoop-manifest", "public-ready"),
        ):
            block = job_block(name, text)
            line = re.search(r"^    needs: \[([^\]]+)\]$", block, re.MULTILINE)
            dependencies = [value.strip() for value in line[1].split(",")]
            assert dependency in dependencies
            dependencies.remove(dependency)
            replacement = "    needs: [" + ", ".join(dependencies) + "]"
            mutant = text.replace(block, block.replace(line[0], replacement, 1), 1)
            assert mutant != text
            assert not core_dependencies_match(mutant)

    def test_tag_manual_and_repository_dispatch_have_one_tag_key(self):
        text = (ROOT / ".github/workflows/publish.yaml").read_text()
        triggers = text.split("concurrency:", 1)[0]
        assert '  push:\n    tags: ["v*"]' in triggers
        assert "  workflow_dispatch:" in triggers
        assert "  repository_dispatch:" in triggers
        assert "  release:" not in triggers
        assert "inputs.tag || github.event.client_payload.tag || github.ref_name" in text
        assert "cancel-in-progress: false" in text

    def test_source_matrix_and_controller_revision_are_separate(self):
        prepare = job_block("prepare")
        assert '"git", "show", os.environ["SOURCE_SHA"]' in prepare
        assert "github-release-state.py source" in prepare
        assert prepare.index("id: meta") < prepare.index("id: cli-targets")
        assert "ref: ${{ github.sha }}" in prepare
        for name in ("prepare-draft", "upload-release-assets", "promote"):
            block = job_block(name)
            assert "ref: ${{ github.sha }}" in block, name
            assert "github-release-state.py" in block, name
        for name in ("build-cli", "publish-crates"):
            assert "ref: ${{ needs.prepare.outputs.checkout_ref }}" in job_block(name)

    def test_core_has_verified_order_and_only_helper_release_writes(self):
        draft, upload, crates, promote = [
            job_block(name) for name in ("prepare-draft", "upload-release-assets", "publish-crates", "promote")
        ]
        assert "contents: write" in draft
        assert "github-release-state.py prepare\n" in draft
        assert "build-cli" in upload
        assert " upload " in upload
        assert "contents: write" in upload
        assert "publish-github-release@" not in upload
        assert "create-github-app-token@" not in upload
        assert "upload-release-assets" in crates
        assert "verify-crates" in crates
        assert "alef-component-abi alef-component-runtime alef" in crates
        assert "publish-crates" in promote
        assert "upload-release-assets" in promote
        assert " promote " in promote
        assert "contents: write" in promote
        assert "Smoke CLI version and source" in job_block("build-cli")

    def test_build_dry_run_ignores_only_expected_draft_skip(self):
        values = {
            "needs.prepare.result": "success",
            "needs.prepare.outputs.release_cli": "true",
            "needs.prepare.outputs.dry_run": "true",
            "needs.check-github-release.result": "success",
            "needs.check-github-release.outputs.exists": "false",
            "needs.prepare-draft.result": "skipped",
        }
        assert job_condition("build-cli", values)
        values["needs.prepare.outputs.dry_run"] = "false"
        assert not job_condition("build-cli", values)
        values["needs.prepare-draft.result"] = "success"
        assert job_condition("build-cli", values)
        values["needs.check-github-release.outputs.exists"] = "true"
        assert not job_condition("build-cli", values)
        values["needs.prepare.outputs.dry_run"] = "true"
        assert job_condition("build-cli", values)
        values["needs.check-github-release.result"] = "failure"
        assert not job_condition("build-cli", values)

    def test_optional_taps_wait_for_authoritative_public_ready(self):
        for name in ("publish-homebrew-formula", "publish-scoop-manifest"):
            block = job_block(name)
            assert "public-ready" in block
            assert "needs.public-ready.result == 'success'" in block
        ready = job_block("public-ready")
        assert "needs: [prepare, promote, publish-crates]" in ready
        assert "github-release-state.py require-public" in ready
        assert "contents: write" not in ready

    def test_public_ready_accepts_tap_only_but_not_failed_selected_cli(self):
        values = {
            "needs.prepare.result": "success",
            "needs.prepare.outputs.release_scoop": "true",
            "needs.prepare.outputs.release_cli": "false",
            "needs.promote.result": "skipped",
        }
        assert job_condition("public-ready", values)
        values["needs.prepare.outputs.release_cli"] = "true"
        assert not job_condition("public-ready", values)
        values["needs.promote.result"] = "success"
        assert job_condition("public-ready", values)
        values["needs.prepare.outputs.release_crates"] = "true"
        assert not job_condition("public-ready", values)
        values["needs.publish-crates.result"] = "success"
        assert job_condition("public-ready", values)
        for outcome in ("", "failure", "cancelled"):
            assert not job_condition("public-ready", dict(values, **{"needs.promote.result": outcome}))

    def test_promote_rejects_unknown_failed_cancelled_selected_jobs(self):
        good = {
            "needs.prepare.result": "success",
            "needs.prepare.outputs.release_cli": "true",
            "needs.check-github-release.result": "success",
            "needs.upload-release-assets.result": "success",
            "needs.check-github-release.outputs.exists": "false",
            "needs.build-cli.result": "success",
            "needs.prepare.outputs.release_crates": "true",
            "needs.publish-crates.result": "success",
        }
        assert job_condition("promote", good)
        for key in (
            "needs.prepare.result",
            "needs.check-github-release.result",
            "needs.upload-release-assets.result",
            "needs.publish-crates.result",
        ):
            for outcome in ("", "failure", "cancelled", "skipped"):
                with self.subTest(key=key, outcome=outcome):
                    assert not job_condition("promote", dict(good, **{key: outcome}))
        existing = dict(
            good,
            **{
                "needs.check-github-release.outputs.exists": "true",
                "needs.build-cli.result": "skipped",
                "needs.upload-release-assets.result": "skipped",
            },
        )
        assert job_condition("promote", existing)


@contextmanager
def rejected(error_type):
    try:
        yield
    except error_type:
        return
    raise AssertionError(f"Expected {error_type.__name__}")


class RecordingApi:
    def __init__(self, module):
        self.module = module
        self.sha = SPEC["source_sha"]
        self.current = None
        self.calls = []
        self.error = None
        self.registry = dict.fromkeys(module.CRATES, True)
        self.annotated = False
        self.tag_cycle = False
        self.registry_calls = []
        self.source_version = SPEC["version"]
        self.source_targets = json.loads(json.dumps(TARGETS))

    def request(self, method, endpoint, payload=None, content=None):
        self.calls.append((method, endpoint, payload))
        if self.error:
            raise self.module.ReleaseError("HTTP failure", self.error)
        if endpoint.startswith("/git/ref/tags/"):
            return {
                "object": {
                    "type": "tag" if self.annotated else "commit",
                    "sha": "c" * 40 if self.annotated else self.sha,
                }
            }
        if endpoint.startswith("/git/tags/"):
            return {
                "object": {
                    "type": "tag" if self.tag_cycle else "commit",
                    "sha": "c" * 40 if self.tag_cycle else self.sha,
                }
            }
        if endpoint.startswith("/contents/"):
            path = urlsplit(endpoint).path.removeprefix("/contents/")
            assert parse_qs(urlsplit(endpoint).query) == {"ref": [self.sha]}
            data = (
                f'[package]\nversion = "{self.source_version}"\n'.encode()
                if path == "Cargo.toml"
                else json.dumps(self.source_targets).encode()
            )
            return {"encoding": "base64", "content": base64.b64encode(data).decode()}
        if endpoint.startswith("/releases/tags/"):
            if self.current is None:
                raise self.module.ReleaseError("not found", 404)
            return json.loads(json.dumps(self.current))
        if method == "POST" and endpoint == "/releases":
            self.current = dict(payload, id=23, assets=[])
            return self.current
        if method == "POST" and endpoint.startswith("/releases/23/assets?"):
            query = parse_qs(urlsplit(endpoint).query)
            digest = hashlib.sha256(content).hexdigest()
            asset = {
                "name": query["name"][0],
                "label": query["label"][0],
                "size": len(content),
                "digest": "sha256:" + digest,
                "state": "uploaded",
            }
            self.current["assets"].append(asset)
            return asset
        if method == "PATCH" and endpoint == "/releases/23":
            self.current.update(payload)
            return self.current
        raise AssertionError((method, endpoint))

    def registry_version(self, name, version):
        self.registry_calls.append((name, version))
        if self.registry[name] is None:
            raise self.module.ReleaseError("not found", 404)
        return {"version": {"crate": name, "num": version, "yanked": not self.registry[name]}}

    @property
    def writes(self):
        return [call for call in self.calls if call[0] != "GET"]


def load_helper():
    module_spec = importlib.util.spec_from_file_location("release_state", HELPER)
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    return module


class ApiBoundaryTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.requests = []
        self.body = b'{"ok": true}'
        self.failure = None
        self.http_stream = None
        self.opener_patch = patch.object(self.module, "build_opener", self.opener)
        self.opener_patch.start()
        self.addCleanup(self.opener_patch.stop)
        self.environment_patch = patch.dict(os.environ, {"GH_TOKEN": "offline-dummy-only"}, clear=True)
        self.environment_patch.start()
        self.addCleanup(self.environment_patch.stop)

    def opener(self, redirect_handler):
        outer = self

        class Opener:
            def open(self, request, timeout):
                assert timeout == 90
                outer.requests.append(request)
                if outer.failure == "redirect":
                    return redirect_handler.redirect_request(None, None, 302, "", {}, "https://untrusted.invalid")
                if isinstance(outer.failure, int):
                    outer.http_stream = io.BytesIO(b"sensitive-provider-body-not-for-logs")
                    raise HTTPError(request.full_url, outer.failure, "private-body", {}, outer.http_stream)
                return io.BytesIO(outer.body)

        return Opener()

    def test_auth_goes_only_to_fixed_github_hosts_and_registry_is_public(self):
        api = self.module.GithubApi()
        api.request("GET", "/releases/tags/v0.115.11")
        api.request("POST", "/releases/23/assets?name=example", content=b"fixture")
        api.registry_version("alef", "0.115.11")
        assert [urlsplit(request.full_url).hostname for request in self.requests] == [
            "api.github.com",
            "uploads.github.com",
            "crates.io",
        ]
        assert self.requests[0].get_header("Authorization") == "Bearer offline-dummy-only"
        assert self.requests[1].get_header("Authorization") == "Bearer offline-dummy-only"
        assert self.requests[2].get_header("Authorization") is None

    def test_api_rejects_nonobject_bad_json_oversized_responses_and_redirect(self):
        api = self.module.GithubApi()
        for body in (b"[]", b"null", b"{", b"x" * (self.module.MAX_JSON_BYTES + 1)):
            with self.subTest(body_length=len(body)), rejected(self.module.ReleaseError):
                self.body = body
                api.request("GET", "/releases/tags/v0.115.11")
        self.failure = "redirect"
        with rejected(self.module.ReleaseError):
            api.request("GET", "/releases/tags/v0.115.11")

    def test_http_errors_keep_status_but_never_provider_body_or_auth(self):
        api = self.module.GithubApi()
        for code in (401, 403, 404, 429, 500):
            self.failure = code
            caught = []
            try:
                api.request("GET", "/releases/tags/v0.115.11")
            except self.module.ReleaseError as error:
                caught.append((error.status, str(error)))
            assert caught == [(code, f"Publication API returned HTTP {code}")]


class SourceAndRegistryTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.api = RecordingApi(self.module)
        self.state = self.module.ReleaseState(SPEC, self.api)

    def test_annotated_tag_is_peeled_and_cycles_do_not_create_releases(self):
        self.api.annotated = True
        self.state.prepare()
        assert self.api.current["target_commitish"] == SPEC["source_sha"]
        prior = len(self.api.writes)
        self.api.tag_cycle = True
        with rejected(self.module.ReleaseError):
            self.state.prepare()
        assert len(self.api.writes) == prior

    def test_all_three_registry_records_are_inspected_even_if_one_missing(self):
        self.api.registry["alef-component-runtime"] = None
        state = self.state.inspect_crates()
        assert state == {"exists": False, "missing": ["alef-component-runtime"], "checked": 3}
        assert self.api.registry_calls == [(name, SPEC["version"]) for name in self.module.CRATES]
        self.api.registry["alef-component-runtime"] = True
        assert self.state.verify_crates() == {"exists": True, "missing": [], "checked": 3}
        assert self.api.writes == []

    def test_bad_registry_identity_or_yanked_record_is_not_absence(self):
        for version in (
            {"crate": "other", "num": SPEC["version"], "yanked": False},
            {"crate": "alef", "num": "0.0.0", "yanked": False},
            {"crate": "alef", "num": SPEC["version"], "yanked": True},
        ):
            self.api.registry_version = lambda _name, _version, value=version: {"version": value}
            with rejected(self.module.ReleaseError):
                self.state.inspect_crates()
        assert self.api.writes == []

    def test_taps_cannot_promote_or_use_a_missing_or_draft_release(self):
        with rejected(self.module.ReleaseError):
            self.state.require_public()
        self.state.prepare()
        prior = len(self.api.writes)
        with rejected(self.module.ReleaseError):
            self.state.require_public()
        assert len(self.api.writes) == prior

    def test_local_tagged_documents_are_used_instead_of_dirty_working_tree(self):
        with tempfile.TemporaryDirectory() as temporary, chdir(temporary):

            def git(*args):
                return subprocess.check_output(
                    ["git", "-c", "core.hooksPath=/dev/null", *args], stderr=subprocess.DEVNULL
                )

            git("init", "-q")
            Path(".github").mkdir()
            Path("Cargo.toml").write_text('[package]\nversion="0.115.11"\n')
            Path(".github/cli-targets.json").write_text(json.dumps(TARGETS))
            git("add", "Cargo.toml", ".github/cli-targets.json")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Fixture")
            git(
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "tag",
                "-am",
                "Fixture",
                SPEC["tag"],
            )
            sha = git("rev-parse", "HEAD").decode().strip()
            Path("Cargo.toml").write_text('[package]\nversion="9.0.0"\n')
            Path(".github/cli-targets.json").write_text("[]")
            assert self.module.source_spec(SPEC["tag"], sha) == dict(SPEC, source_sha=sha)

    def test_smoke_requires_exact_native_version_source_and_clean_tree_banner(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "fixture-cli"
            good = f"alef {SPEC['version']}\ncommit:  {SPEC['source_sha'][:12]} ({SPEC['source_sha']})\nbuilt:   2026-10-31T00:00:00Z\ntree:    clean\n"
            for banner in (
                good,
                good.replace(SPEC["version"], "0.0.0"),
                good.replace("clean", "DIRTY"),
                good.replace("(" + SPEC["source_sha"] + ")", "(" + "b" * 40 + ")"),
            ):
                binary.write_bytes(b"offline protocol fixture, not a native binary")
                response = subprocess.CompletedProcess([str(binary), "--version"], 0, banner, "")
                with patch.object(self.module.subprocess, "run", return_value=response) as invoked:
                    if banner == good:
                        assert self.module.smoke_binary(SPEC, binary)["verified_source_sha"] == SPEC["source_sha"]
                    else:
                        with rejected(self.module.ReleaseError):
                            self.module.smoke_binary(SPEC, binary)
                    assert invoked.call_args.args[0] == [str(binary.resolve()), "--version"]


class LatestApi(RecordingApi):
    """Model GitHub selection only for fixtures with aligned creation-date/version order."""

    def __init__(self, module, shared, version):
        super().__init__(module)
        self.shared = shared
        self.source_version = version
        self.rank = int(version.split("-")[0].split(".")[-1])

    def request(self, method, endpoint, payload=None, content=None):
        result = super().request(method, endpoint, payload, content)
        if method == "POST" and endpoint == "/releases":
            self.shared["records"][self.source_version] = (self.rank, self.current)
        if (
            method == "PATCH"
            and endpoint == "/releases/23"
            and not self.current["draft"]
            and not self.current["prerelease"]
        ):
            policy = payload.get("make_latest", "true")
            if policy == "true":
                self.shared["latest"] = self.source_version
            elif policy == "legacy":
                eligible = [
                    (rank, version)
                    for version, (rank, release) in self.shared["records"].items()
                    if not release["draft"] and not release["prerelease"]
                ]
                self.shared["latest"] = max(eligible)[1]
        return result


class LatestPolicyTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.shared = {"records": {"0.115.12": (12, {"draft": False, "prerelease": False})}, "latest": "0.115.12"}
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / "cli"
        archives(self.directory)

    def ready(self, version):
        api = LatestApi(self.module, self.shared, version)
        spec = dict(SPEC, version=version, tag="v" + version)
        state = self.module.ReleaseState(spec, api)
        state.prepare()
        state.upload(self.directory, "success")
        return state, api

    def test_historical_stable_does_not_replace_newer_latest(self):
        state, api = self.ready("0.115.11")
        state.promote("success", "success", True)
        assert self.shared["latest"] == "0.115.12"
        assert api.writes[-1][2] == {"draft": False, "make_latest": "legacy"}

    def test_new_stable_advances_with_explicit_server_policy(self):
        state, api = self.ready("0.115.13")
        state.promote("success", "success", True)
        assert self.shared["latest"] == "0.115.13"
        assert api.writes[-1][2].get("make_latest") == "legacy"

    def test_prerelease_and_draft_never_request_latest(self):
        state, api = self.ready("0.115.14-rc.1")
        assert api.writes[0][2].get("make_latest") == "false"
        assert self.shared["latest"] == "0.115.12"
        state.promote("success", "success", True)
        assert api.writes[-1][2].get("make_latest") == "false"
        assert self.shared["latest"] == "0.115.12"

    def test_complete_public_equal_retry_has_no_patch_or_latest_change(self):
        state, api = self.ready("0.115.12")
        state.promote("success", "success", True)
        before = list(api.writes)
        state.promote("skipped", "success", True)
        assert api.writes == before
        assert self.shared["latest"] == "0.115.12"

    def test_different_tag_interleavings_have_no_stale_client_latest_decision(self):
        for order in ((0, 1), (1, 0)):
            with self.subTest(order=order):
                self.shared = {"records": {}, "latest": None}
                states = [self.ready("0.115.11"), self.ready("0.115.12")]
                for index in order:
                    state, _ = states[index]
                    state.promote("success", "success", True)
                assert self.shared["latest"] == "0.115.12"
                for _, api in states:
                    assert api.writes[-1][2].get("make_latest") == "legacy"
                    assert not any(endpoint == "/releases/latest" for _, endpoint, _ in api.calls)

    def test_draft_classification_mismatch_fails_before_promotion(self):
        state, api = self.ready("0.115.14-rc.1")
        api.current["prerelease"] = False
        prior = len(api.writes)
        with rejected(self.module.ReleaseError):
            state.promote("success", "success", True)
        assert len(api.writes) == prior


class ReleaseStateTest(unittest.TestCase):
    def setUp(self):
        self.module = load_helper()
        self.api = RecordingApi(self.module)
        self.state = self.module.ReleaseState(SPEC, self.api)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name) / "cli"
        archives(self.directory)

    def complete_draft(self):
        self.state.prepare()
        self.state.upload(self.directory, "success")

    def test_prepare_creates_only_a_source_bound_draft(self):
        assert self.state.prepare()["build_needed"]
        assert self.api.current["draft"]
        assert self.api.current["target_commitish"] == SPEC["source_sha"]
        assert len(self.api.writes) == 1
        assert self.state.prepare()["build_needed"] is True
        assert len(self.api.writes) == 1

    def test_complete_five_then_verify_three_crates_before_promotion(self):
        self.complete_draft()
        assert len(self.api.current["assets"]) == 5
        assert self.api.current["draft"]
        self.state.promote("success", "success", True)
        assert not self.api.current["draft"]
        prior = len(self.api.writes)
        assert not self.state.prepare()["build_needed"]
        self.state.upload(self.directory, "success")
        self.state.promote("skipped", "success", True)
        assert len(self.api.writes) == prior

    def test_partial_retry_keeps_verified_asset_even_when_rebuild_bytes_differ(self):
        self.complete_draft()
        first = self.api.current["assets"][0].copy()
        self.api.current["assets"] = [first]
        write_archive(self.directory / NAMES[0], b"different real archived executable fixture")
        prior = len(self.api.writes)
        self.state.upload(self.directory, "success")
        assert self.api.current["assets"][0] == first
        assert len(self.api.writes) - prior == 4
        assert len(self.api.current["assets"]) == 5

    def test_local_missing_empty_extra_and_symlink_fail_before_writes(self):
        for mutation in ("missing", "empty", "extra", "symlink"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary) / "cli"
                archives(directory)
                if mutation == "missing":
                    (directory / NAMES[0]).unlink()
                elif mutation == "empty":
                    (directory / NAMES[0]).write_bytes(b"")
                elif mutation == "extra":
                    (directory / "bottle.tar.gz").write_bytes(b"bottle")
                else:
                    (directory / NAMES[0]).unlink()
                    (directory / NAMES[0]).symlink_to(directory / NAMES[1])
                with rejected(self.module.ReleaseError):
                    self.state.upload(directory, "success")
                assert self.api.writes == []

    def test_unknown_failed_cancelled_and_skipped_build_cannot_upload(self):
        for outcome in ("", "failure", "cancelled", "skipped"):
            with self.subTest(outcome=outcome), rejected(self.module.ReleaseError):
                self.state.upload(self.directory, outcome)
        assert self.api.writes == []

    def test_dry_run_builds_even_when_public_and_never_writes(self):
        self.complete_draft()
        self.state.promote("success", "success", True)
        prior = len(self.api.writes)
        dry = self.module.ReleaseState(SPEC, self.api, dry_run=True)
        assert dry.prepare()["build_needed"]
        dry.upload(self.directory, "success")
        dry.promote("success", "success", True)
        assert len(self.api.writes) == prior

    def test_tag_moved_and_http_failures_are_not_missing(self):
        self.api.sha = "b" * 40
        with rejected(self.module.ReleaseError):
            self.state.prepare()
        self.api.sha = SPEC["source_sha"]
        for code in (401, 403, 429, 500):
            self.api.error = code
            with self.subTest(code=code), rejected(self.module.ReleaseError):
                self.state.prepare()
        assert self.api.writes == []

    def test_legacy_wrong_source_wrong_asset_duplicate_and_public_partial_fail_closed(self):
        self.complete_draft()
        original = json.loads(json.dumps(self.api.current))
        for mutation in ("legacy", "source", "asset_label", "digest", "empty", "duplicate", "public_partial"):
            with self.subTest(mutation=mutation):
                self.api.current = json.loads(json.dumps(original))
                if mutation == "legacy":
                    self.api.current["body"] = "legacy notes"
                elif mutation == "source":
                    self.api.current["target_commitish"] = "b" * 40
                elif mutation == "asset_label":
                    self.api.current["assets"][0]["label"] = "unbound"
                elif mutation == "digest":
                    self.api.current["assets"][0]["digest"] = "sha256:" + "b" * 64
                elif mutation == "empty":
                    self.api.current["assets"][0]["size"] = 0
                elif mutation == "duplicate":
                    self.api.current["assets"].append(self.api.current["assets"][0].copy())
                else:
                    self.api.current["assets"].pop()
                    self.api.current["draft"] = False
                prior = len(self.api.writes)
                with rejected(self.module.ReleaseError):
                    self.state.prepare()
                assert len(self.api.writes) == prior

    def test_crates_partial_yanked_or_unknown_outcome_leaves_draft(self):
        self.complete_draft()
        for name in self.module.CRATES:
            self.api.registry[name] = False
            with self.subTest(name=name), rejected(self.module.ReleaseError):
                self.state.promote("success", "success", True)
            assert self.api.current["draft"]
            self.api.registry[name] = True
        for outcome in ("", "failure", "cancelled", "skipped"):
            with self.subTest(outcome=outcome), rejected(self.module.ReleaseError):
                self.state.promote("success", outcome, True)
        assert self.api.current["draft"]

    def test_self_declared_subset_cannot_prepare_or_promote_four_assets(self):
        subset = dict(SPEC, assets=NAMES[:-1])
        state = self.module.ReleaseState(subset, self.api)
        (self.directory / NAMES[-1]).unlink()
        with rejected(self.module.ReleaseError):
            state.prepare()
            state.upload(self.directory, "success")
            state.promote("success", "success", True)
        assert self.api.writes == []

    def test_changed_declared_version_cannot_override_tagged_cargo(self):
        self.api.source_version = "0.115.9"
        with rejected(self.module.ReleaseError):
            self.state.prepare()
        assert self.api.writes == []

    def test_plaintext_five_files_cannot_be_uploaded_as_archives(self):
        self.state.prepare()
        for path in self.directory.iterdir():
            path.write_bytes(b"not an archive")
        prior = len(self.api.writes)
        with rejected(self.module.ReleaseError):
            self.state.upload(self.directory, "success")
        assert len(self.api.writes) == prior

    def test_archive_member_missing_empty_symlink_duplicate_rejected_for_both_formats(self):
        for name in (NAMES[0], NAMES[-1]):
            for mutation in ("missing", "empty", "symlink", "duplicate"):
                with self.subTest(name=name, mutation=mutation):
                    write_archive(self.directory / name, mutation=mutation)
                    with rejected(self.module.ReleaseError):
                        self.module.local_archives(SPEC, self.directory)
                    write_archive(self.directory / name)
        assert self.api.writes == []

    def test_older_tag_uses_its_own_targets_and_version_not_current_controller_list(self):
        self.api.source_version = "0.100.0"
        self.api.source_targets = TARGETS[:2]
        older = dict(SPEC, tag="v0.100.0", version="0.100.0", assets=NAMES[:2])
        state = self.module.ReleaseState(older, self.api)
        for name in NAMES[2:]:
            (self.directory / name).unlink()
        state.prepare()
        state.upload(self.directory, "success")
        state.promote("success", "success", True)
        assert not self.api.current["draft"]
        assert len(self.api.current["assets"]) == 2
        source_reads = [endpoint for method, endpoint, _ in self.api.calls if endpoint.startswith("/contents/")]
        assert len(source_reads) == 2
        assert all(f"ref={SPEC['source_sha']}" in endpoint for endpoint in source_reads)

    def test_tag_source_metadata_failure_is_never_release_not_found(self):
        original = self.api.request

        def missing_source(method, endpoint, payload=None, content=None):
            if endpoint.startswith("/contents/"):
                raise self.module.ReleaseError("source missing", 404)
            return original(method, endpoint, payload, content)

        self.api.request = missing_source
        with rejected(self.module.ReleaseError):
            self.state.prepare()
        assert self.api.writes == []

    def test_tampered_spec_cannot_upload_or_promote_even_after_initial_admission(self):
        self.state.prepare()
        self.state.spec["assets"] = NAMES[:-1]
        (self.directory / NAMES[-1]).unlink()
        prior = len(self.api.writes)
        with rejected(self.module.ReleaseError):
            self.state.upload(self.directory, "success")
        with rejected(self.module.ReleaseError):
            self.state.promote("success", "success", True)
        assert len(self.api.writes) == prior

    def test_truncated_tar_zip_and_bad_zip_crc_are_not_valid_archives(self):
        for name in (NAMES[0], NAMES[-1]):
            path = self.directory / name
            with self.subTest(name=name):
                path.write_bytes(path.read_bytes()[:-4])
                with rejected(self.module.ReleaseError):
                    self.module.local_archives(SPEC, self.directory)
                write_archive(path)
        path = self.directory / NAMES[-1]
        before = path.read_bytes()
        assert before.count(b"nonempty executable fixture") == 1
        path.write_bytes(before.replace(b"nonempty executable fixture", b"CORRUPT! executable fixture", 1))
        with rejected(self.module.ReleaseError):
            self.module.local_archives(SPEC, self.directory)
        assert self.api.writes == []

    def test_compressed_archive_decompression_bound_is_enforced(self):
        self.module.MAX_UNPACKED_BYTES = 1
        with rejected(self.module.ReleaseError):
            self.module.local_archives(SPEC, self.directory)
        assert self.api.writes == []

    def test_tar_regular_member_name_must_be_exact_not_a_directory_spelling(self):
        path = self.directory / NAMES[0]
        with tarfile.open(path, "w:gz") as archive:
            entry = tarfile.TarInfo(archive_member(path.name) + "/")
            entry.size = 1
            archive.addfile(entry, io.BytesIO(b"x"))
        with rejected(self.module.ReleaseError):
            self.module.local_archives(SPEC, self.directory)

    def test_invalid_spec_and_force_cannot_authorize_clobber(self):
        for key, value in (
            ("assets", []),
            ("assets", [*NAMES, NAMES[0]]),
            ("source_sha", "main"),
            ("tag", "vOTHER"),
            ("schema", True),
        ):
            with self.subTest(key=key), rejected(self.module.ReleaseError):
                self.module.ReleaseState(dict(SPEC, **{key: value}), self.api)
        with rejected(self.module.ReleaseError):
            self.module.ReleaseState(SPEC, self.api, force=True)
        assert self.api.writes == []


if __name__ == "__main__":
    unittest.main()
