#!/usr/bin/env python3
"""Publish only source-bound, complete Alef CLI releases; never retag or replace assets."""

import argparse
import base64
import gzip
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tarfile
import tomllib
import zipfile
import zlib
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import HTTPRedirectHandler, Request, build_opener

CRATES = ("alef-component-abi", "alef-component-runtime", "alef")
REPOSITORY = "xberg-io/alef"
MAX_JSON_BYTES = 2 * 1024 * 1024
MAX_ARCHIVE_BYTES = 256 * 1024 * 1024
MAX_UNPACKED_BYTES = 512 * 1024 * 1024
READ_CHUNK_BYTES = 1024 * 1024
MARKER = re.compile(r"<!-- alef-release-state:v1 (\{[^\n]*\}) -->")


class ReleaseError(Exception):
    def __init__(self, message, status=None):
        super().__init__(message)
        self.status = status


def validate_spec(spec):
    if not isinstance(spec, dict) or set(spec) != {"schema", "tag", "version", "source_sha", "assets"}:
        raise ReleaseError("Invalid source specification fields")
    if type(spec["schema"]) is not int or spec["schema"] != 1:
        raise ReleaseError("Unsupported source specification schema")
    version = spec["version"]
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
        raise ReleaseError("Invalid release version")
    if spec["tag"] != "v" + version or not re.fullmatch(r"[0-9a-f]{40}", str(spec["source_sha"])):
        raise ReleaseError("Tag/version/source SHA mismatch")
    assets = spec["assets"]
    if not isinstance(assets, list) or not assets or len(assets) != len(set(assets)):
        raise ReleaseError("Expected a nonempty unique archive set")
    if any(
        not isinstance(name, str) or not re.fullmatch(r"alef-[a-z0-9_-]+\.(?:tar\.gz|zip)", name) for name in assets
    ):
        raise ReleaseError("Unsafe archive name")


def canonical_spec(tag, sha, manifest_bytes, targets_bytes):
    try:
        manifest = tomllib.loads(manifest_bytes.decode())
        targets = json.loads(targets_bytes)
        spec = {
            "schema": 1,
            "tag": tag,
            "version": manifest["package"]["version"],
            "source_sha": sha,
            "assets": [f"alef-{row['target']}.{row['archive_ext']}" for row in targets],
        }
        validate_spec(spec)
        return spec
    except (ValueError, KeyError, TypeError, UnicodeError):
        raise ReleaseError("Invalid canonical tagged Cargo/CLI-target metadata") from None


def read_bounded(stream):
    total = 0
    while chunk := stream.read(READ_CHUNK_BYTES):
        total += len(chunk)
        if total > MAX_UNPACKED_BYTES:
            raise ReleaseError("CLI archive decompression exceeds limit")
    return total


def verify_tar(path, expected):
    with gzip.open(path, "rb") as stream:
        read_bounded(stream)
    seen = set()
    binary = False
    with tarfile.open(path, "r:gz") as archive:
        for entry in archive:
            name = entry.name.rstrip("/")
            if name in seen:
                raise ReleaseError("Duplicate CLI archive member")
            seen.add(name)
            if entry.isdir() and name == expected.split("/")[0]:
                continue
            if entry.name != expected or entry.type not in (tarfile.REGTYPE, tarfile.AREGTYPE):
                raise ReleaseError("Unexpected or nonregular CLI archive member")
            if not 0 < entry.size <= MAX_UNPACKED_BYTES:
                raise ReleaseError("Empty or oversized CLI archive member")
            with archive.extractfile(entry) as stream:
                if read_bounded(stream) != entry.size:
                    raise ReleaseError("Truncated CLI archive member")
            binary = True
    if not binary:
        raise ReleaseError("Expected CLI archive member is missing")


def verify_zip(path, expected):
    seen = set()
    binary = False
    with zipfile.ZipFile(path) as archive:
        for entry in archive.infolist():
            name = entry.filename.rstrip("/")
            if name in seen:
                raise ReleaseError("Duplicate CLI archive member")
            seen.add(name)
            mode = stat.S_IFMT(entry.external_attr >> 16)
            if entry.is_dir() and name == expected.split("/")[0] and mode in (0, stat.S_IFDIR):
                continue
            if name != expected or entry.is_dir() or mode not in (0, stat.S_IFREG):
                raise ReleaseError("Unexpected or nonregular CLI archive member")
            if not 0 < entry.file_size <= MAX_UNPACKED_BYTES:
                raise ReleaseError("Empty or oversized CLI archive member")
            with archive.open(entry) as stream:
                if read_bounded(stream) != entry.file_size:
                    raise ReleaseError("Truncated CLI archive member")
            binary = True
    if not binary:
        raise ReleaseError("Expected CLI archive member is missing")


def verify_archive(path):
    root = path.name.removesuffix(".zip").removesuffix(".tar.gz")
    expected = root + ("/alef.exe" if "-windows-" in root else "/alef")
    try:
        (verify_zip if path.name.endswith(".zip") else verify_tar)(path, expected)
    except (tarfile.TarError, zipfile.BadZipFile, zlib.error, EOFError, OSError, RuntimeError, ValueError):
        raise ReleaseError(f"Malformed CLI archive: {path.name}") from None


def local_archives(spec, directory):
    validate_spec(spec)
    if not directory.is_dir() or {path.name for path in directory.iterdir()} != set(spec["assets"]):
        raise ReleaseError("Local CLI archives do not match the complete expected set")
    result = {}
    for name in spec["assets"]:
        path = directory / name
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= MAX_ARCHIVE_BYTES:
            raise ReleaseError(f"Invalid CLI archive: {name}")
        verify_archive(path)
        with path.open("rb") as stream:
            result[name] = hashlib.file_digest(stream, "sha256").hexdigest()
    return result


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        raise ReleaseError("Publication API redirect refused")


class GithubApi:
    def __init__(self):
        self.token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
        if not self.token:
            raise ReleaseError("GH_TOKEN or GITHUB_TOKEN is required")

    def _request(self, url, method="GET", payload=None, content=None, authenticated=True):
        headers = {"Accept": "application/vnd.github+json", "User-Agent": "alef-publisher"}
        if authenticated:
            headers["Authorization"] = "Bearer " + self.token
            headers["X-GitHub-Api-Version"] = "2022-11-28"
        data = content
        if payload is not None:
            data = json.dumps(payload).encode()
        if data is not None:
            headers["Content-Type"] = "application/json" if content is None else "application/octet-stream"
        request = Request(url, data=data, headers=headers, method=method)
        try:
            with build_opener(NoRedirect()).open(request, timeout=90) as response:
                body = response.read(MAX_JSON_BYTES + 1)
        except HTTPError as error:
            raise ReleaseError(f"Publication API returned HTTP {error.code}", error.code) from None
        except (URLError, TimeoutError, OSError):
            raise ReleaseError("Publication API transport failed; no automatic retry") from None
        if len(body) > MAX_JSON_BYTES:
            raise ReleaseError("Publication API response exceeds limit")
        try:
            value = json.loads(body)
            if not isinstance(value, dict):
                raise ReleaseError("Publication API returned a non-object response")
            return value
        except (ValueError, UnicodeError):
            raise ReleaseError("Publication API returned invalid JSON") from None

    def request(self, method, endpoint, payload=None, content=None):
        host = "uploads.github.com" if content is not None else "api.github.com"
        return self._request(f"https://{host}/repos/{REPOSITORY}{endpoint}", method, payload, content)

    def registry_version(self, name, version):
        return self._request(f"https://crates.io/api/v1/crates/{name}/{quote(version, safe='')}", authenticated=False)


class ReleaseState:
    def __init__(self, spec, api, dry_run=False, force=False):
        validate_spec(spec)
        if force:
            raise ReleaseError("force_republish cannot authorize replacement; dispatch a missing-only retry")
        self.spec = json.loads(json.dumps(spec))
        self.api = api
        self.dry_run = dry_run
        self._canonical_spec = None

    def check_tag(self):
        ref = self.api.request("GET", "/git/ref/tags/" + quote(self.spec["tag"], safe=""))
        obj = ref["object"]
        for _ in range(8):
            if obj.get("type") == "commit":
                if obj.get("sha") != self.spec["source_sha"]:
                    raise ReleaseError("Tag no longer points at the admitted source SHA")
                self.check_source()
                return
            if obj.get("type") != "tag" or not re.fullmatch(r"[0-9a-f]{40}", str(obj.get("sha"))):
                raise ReleaseError("Tag does not resolve to a commit")
            obj = self.api.request("GET", "/git/tags/" + obj["sha"])["object"]
        raise ReleaseError("Tag indirection exceeds limit")

    def read_source_file(self, path):
        endpoint = "/contents/" + path + "?" + urlencode({"ref": self.spec["source_sha"]})
        response = self.api.request("GET", endpoint)
        if response.get("encoding") != "base64":
            raise ReleaseError("Canonical tagged source content is unavailable")
        try:
            return base64.b64decode("".join(response["content"].split()), validate=True)
        except (ValueError, KeyError, TypeError):
            raise ReleaseError("Invalid canonical tagged source content") from None

    def check_source(self):
        if self._canonical_spec is None:
            self._canonical_spec = canonical_spec(
                self.spec["tag"],
                self.spec["source_sha"],
                self.read_source_file("Cargo.toml"),
                self.read_source_file(".github/cli-targets.json"),
            )
        if self.spec != self._canonical_spec:
            raise ReleaseError("Specification differs from immutable tagged Cargo/CLI-target metadata")

    def binding(self):
        return "<!-- alef-release-state:v1 " + json.dumps(self.spec, sort_keys=True) + " -->"

    def checked_assets(self, release):
        verified = {}
        if not isinstance(release.get("assets"), list):
            raise ReleaseError("Publication API omitted the asset inventory")
        for asset in release["assets"]:
            if not isinstance(asset, dict):
                raise ReleaseError("Publication API returned an invalid asset entry")
            name = asset.get("name")
            if name not in self.spec["assets"]:
                continue
            digest = asset.get("digest", "")
            if name in verified or not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
                raise ReleaseError(f"Duplicate or missing authoritative asset digest: {name}")
            label = f"alef-source:{self.spec['source_sha']};{digest}"
            if (
                asset.get("label") != label
                or asset.get("state") != "uploaded"
                or type(asset.get("size")) is not int
                or asset["size"] <= 0
            ):
                raise ReleaseError(f"Unverified source-bound CLI asset: {name}")
            verified[name] = digest
        return verified

    def inspect(self):
        self.check_tag()
        if self.dry_run:
            return {"release": None, "assets": {}, "build_needed": True, "public_ready": False}
        try:
            release = self.api.request("GET", "/releases/tags/" + quote(self.spec["tag"], safe=""))
        except ReleaseError as error:
            if error.status == 404:
                return {"release": None, "assets": {}, "build_needed": True, "public_ready": False}
            raise
        matches = MARKER.findall(release.get("body") or "")
        if len(matches) != 1 or json.loads(matches[0]) != self.spec:
            raise ReleaseError("Existing release is legacy/unbound or belongs to a different source")
        if (
            release.get("tag_name") != self.spec["tag"]
            or release.get("target_commitish") != self.spec["source_sha"]
            or type(release.get("draft")) is not bool
            or type(release.get("id")) is not int
            or release.get("prerelease") is not ("-" in self.spec["version"])
        ):
            raise ReleaseError("Existing release identity does not match the source")
        assets = self.checked_assets(release)
        complete = set(assets) == set(self.spec["assets"])
        if not release["draft"] and not complete:
            raise ReleaseError("Incomplete public release requires separately reviewed repair")
        return {
            "release": release,
            "assets": assets,
            "build_needed": not complete,
            "public_ready": complete and not release["draft"],
        }

    def prepare(self):
        state = self.inspect()
        if self.dry_run or state["release"] is not None:
            return state
        self.check_tag()
        self.api.request(
            "POST",
            "/releases",
            {
                "tag_name": self.spec["tag"],
                "target_commitish": self.spec["source_sha"],
                "name": self.spec["tag"],
                "make_latest": "false",
                "body": self.binding(),
                "draft": True,
                "prerelease": "-" in self.spec["version"],
            },
        )
        return self.inspect()

    def upload(self, directory, build_result):
        if build_result != "success":
            raise ReleaseError("Upload requires a successful complete CLI build matrix")
        digests = local_archives(self.spec, directory)
        state = self.inspect()
        if self.dry_run:
            return {"verified_local": len(digests)}
        if state["release"] is None:
            raise ReleaseError("A source-bound draft must be prepared before upload")
        for name, digest in digests.items():
            state = self.inspect()
            if name in state["assets"]:
                continue
            release = state["release"]
            query = urlencode({"name": name, "label": f"alef-source:{self.spec['source_sha']};sha256:{digest}"})
            self.api.request(
                "POST", f"/releases/{release['id']}/assets?{query}", content=(directory / name).read_bytes()
            )
            if self.inspect()["assets"].get(name) != "sha256:" + digest:
                raise ReleaseError(f"Uploaded asset failed authoritative digest verification: {name}")
        if self.inspect()["build_needed"]:
            raise ReleaseError("Uploaded release is still incomplete")
        return {"verified_remote": len(digests)}

    def inspect_crates(self):
        self.check_tag()
        missing = []
        for name in CRATES:
            try:
                response = self.api.registry_version(name, self.spec["version"])
            except ReleaseError as error:
                if error.status != 404:
                    raise
                missing.append(name)
                continue
            version = response.get("version", {})
            if (
                not isinstance(version, dict)
                or version.get("crate") != name
                or version.get("num") != self.spec["version"]
                or version.get("yanked") is not False
            ):
                raise ReleaseError(f"Registry has not verified requested crate: {name}")
        return {"exists": not missing, "missing": missing, "checked": len(CRATES)}

    def verify_crates(self):
        result = self.inspect_crates()
        if not result["exists"]:
            raise ReleaseError("Requested crates are missing: " + ", ".join(result["missing"]))
        return result

    def require_public(self):
        if not self.inspect()["public_ready"]:
            raise ReleaseError("Optional taps require an already-public complete source-bound CLI release")
        return {"public_ready": True}

    def promote(self, build_result, crates_result, crates_requested):
        if build_result not in ("success", "skipped") or (crates_requested and crates_result != "success"):
            raise ReleaseError("Unknown/failed selected job outcome cannot promote a release")
        if self.dry_run:
            self.check_tag()
            return {"dry_run": True}
        state = self.inspect()
        if state["release"] is None or state["build_needed"]:
            raise ReleaseError("All source-bound CLI assets must be verified before promotion")
        if crates_requested:
            self.verify_crates()
        if not state["release"]["draft"]:
            return {"already_public": True}
        self.check_tag()
        self.api.request(
            "PATCH",
            f"/releases/{state['release']['id']}",
            {"draft": False, "make_latest": "false" if "-" in self.spec["version"] else "legacy"},
        )
        final = self.inspect()
        if final["release"]["draft"]:
            raise ReleaseError("Release promotion was not confirmed")
        return {"promoted": True}


def source_spec(tag, sha):
    if not isinstance(tag, str) or not tag.startswith("v") or not re.fullmatch(r"[0-9a-f]{40}", str(sha)):
        raise ReleaseError("Invalid immutable tag source input")

    def source(path):
        return subprocess.check_output(["git", "show", f"{sha}:{path}"], timeout=30)

    resolved = (
        subprocess.check_output(["git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"], timeout=30)
        .decode()
        .strip()
    )
    if resolved != sha:
        raise ReleaseError("Requested tag does not match resolved source SHA")
    return canonical_spec(tag, sha, source("Cargo.toml"), source(".github/cli-targets.json"))


def smoke_binary(spec, binary):
    validate_spec(spec)
    output = subprocess.run(
        [str(binary.resolve(strict=True)), "--version"], capture_output=True, text=True, timeout=30, check=False
    )
    lines = output.stdout.splitlines()
    expected_commit = f"commit:  {spec['source_sha'][:12]} ({spec['source_sha']})"
    if (
        output.returncode != 0
        or len(lines) != 4
        or lines[0] != "alef " + spec["version"]
        or lines[1] != expected_commit
        or not lines[2].startswith("built:   ")
        or lines[3] != "tree:    clean"
    ):
        raise ReleaseError("Native CLI version/source/clean-tree smoke failed")
    return {"verified_version": spec["version"], "verified_source_sha": spec["source_sha"]}


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=(
            "source",
            "validate-local",
            "smoke",
            "inspect",
            "prepare",
            "upload",
            "promote",
            "inspect-crates",
            "verify-crates",
            "require-public",
        ),
    )
    parser.add_argument("--spec", type=Path, default=Path("release-spec.json"))
    parser.add_argument("--tag")
    parser.add_argument("--source-sha")
    parser.add_argument("--directory", type=Path, default=Path("dist/cli"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--build-result", default="")
    parser.add_argument("--crates-result", default="")
    parser.add_argument("--crates-requested", action="store_true")
    return parser.parse_args()


def main():
    args = arguments()
    if args.command == "source":
        spec = source_spec(args.tag, args.source_sha)
        args.spec.write_text(json.dumps(spec, sort_keys=True) + "\n")
        return
    spec = json.loads(args.spec.read_text())
    if args.command == "validate-local":
        print(json.dumps(local_archives(spec, args.directory), sort_keys=True))
        return
    if args.command == "smoke":
        print(json.dumps(smoke_binary(spec, args.binary), sort_keys=True))
        return
    state = ReleaseState(spec, GithubApi(), dry_run=args.dry_run, force=args.force)
    if args.command == "upload":
        result = state.upload(args.directory, args.build_result)
    elif args.command == "promote":
        result = state.promote(args.build_result, args.crates_result, args.crates_requested)
    else:
        result = getattr(state, args.command.replace("-", "_"))()
    if isinstance(result, dict):
        result = {key: value for key, value in result.items() if key != "release"}
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        message = str(error) if isinstance(error, ReleaseError) else "Invalid publication input or source metadata"
        print(message, file=sys.stderr)
        sys.exit(1)
