#!/usr/bin/env bash
set -euo pipefail

#   VERSION - semver without v prefix, e.g. 0.4.6

tag="${TAG:?TAG is required (e.g. v0.4.6)}"
version="${VERSION:?VERSION is required (e.g. 0.4.6)}"
tap_dir="${TAP_DIR:?TAP_DIR is required (path to homebrew-tap checkout)}"

formula="${tap_dir}/Formula/alef.rb"

[[ -f "$formula" ]] || {
  echo "Missing $formula" >&2
  exit 1
}

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

source_url="https://github.com/xberg-io/alef/archive/${tag}.tar.gz"
echo "Downloading source archive from $source_url..." >&2
curl -fsSL "$source_url" -o "$work_dir/source.tar.gz"
source_sha="$(shasum -a 256 "$work_dir/source.tar.gz" | awk '{print $1}')"

if [[ ! "$source_sha" =~ ^[a-f0-9]{64}$ ]]; then
  echo "Computed invalid sha256: $source_sha" >&2
  exit 1
fi

echo "Source tarball sha256: $source_sha" >&2

python3 - "$formula" "$source_url" "$source_sha" "$version" <<'PY'
import re
import sys

path, url, sha, version = sys.argv[1:5]
content = open(path).read()

bottle_start = content.find("bottle do")
if bottle_start == -1:
    head, tail = content, ""
else:
    head, tail = content[:bottle_start], content[bottle_start:]

head = re.sub(r"""^(\s*version\s+)["'][^"']*["']""", rf'\1"{version}"', head, count=1, flags=re.MULTILINE)
head = re.sub(r"""^(\s*url\s+)["'][^"']*["']""", rf'\1"{url}"', head, count=1, flags=re.MULTILINE)
head = re.sub(r"""^(\s*sha256\s+)["'][^"']*["']""", rf'\1"{sha}"', head, count=1, flags=re.MULTILINE)

open(path, "w").write(head + tail)
print(f"Updated source url + sha256 in {path}", file=sys.stderr)
PY

echo "Updated formula: $formula" >&2
