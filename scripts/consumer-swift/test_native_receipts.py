import json
import tempfile
import unittest
from pathlib import Path

from assertions import raises
from native_receipts import collect, package_name


def write_archive(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"!<arch>\n" + b"\0" * 64)


def receipt(package_id, out_dir, linked_libs, linked_paths):
    return {
        "reason": "build-script-executed",
        "package_id": package_id,
        "linked_libs": linked_libs,
        "linked_paths": linked_paths,
        "cfgs": [],
        "env": [],
        "out_dir": str(out_dir),
    }


class NativeReceipts(unittest.TestCase):
    def test_package_name_reads_registry_and_both_path_id_forms(self):
        assert package_name("registry+https://github.com/rust-lang/crates.io-index#ort-sys@2.0.0-rc.14") == "ort-sys"
        assert package_name("path+file:///w/xberg/crates/xberg-tesseract#1.4.0") == "xberg-tesseract"
        assert package_name("path+file:///w/xberg/crates/ocr#xberg-tesseract@1.4.0") == "xberg-tesseract"

    def test_actual_path_tesseract_and_cache_linked_ort_are_collected(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            tesseract_out = root / "target/release/build/xberg-tesseract-9186/out"
            write_archive(tesseract_out / "tesseract/lib/libtesseract.a")
            write_archive(tesseract_out / "leptonica/lib/libleptonica.a")
            ort_out = root / "target/release/build/ort-sys-591b/out"
            ort_out.mkdir(parents=True)
            ort_cache = root / "ort.pyke.io/dfbin/aarch64-apple-darwin/4033"
            write_archive(ort_cache / "libonnxruntime.a")
            invocation = root / "release-cargo/1-866"
            invocation.mkdir(parents=True)
            rows = [
                receipt(
                    "path+file:///w/xberg/crates/xberg-tesseract#1.4.0",
                    tesseract_out,
                    ["static=tesseract", "static=leptonica", "c++"],
                    ["native=" + str(tesseract_out / "tesseract/lib")],
                ),
                receipt(
                    "registry+https://github.com/rust-lang/crates.io-index#ort-sys@2.0.0-rc.14",
                    ort_out,
                    ["c++", "static=onnxruntime"],
                    ["native=" + str(ort_cache)],
                ),
            ]
            (invocation / "stdout.log").write_text("\n".join(json.dumps(row) for row in rows) + "\n")
            result = collect(root / "release-cargo")
            names = {row["package_id"]: {Path(p).name for p in row["archives"]} for row in result}
            assert names == {
                rows[0]["package_id"]: {"libtesseract.a", "libleptonica.a"},
                rows[1]["package_id"]: {"libonnxruntime.a"},
            }

    def test_ort_without_a_linked_static_archive_is_rejected(self):
        cases = [
            ("libunrelated.a", ["static=onnxruntime"]),
            ("libonnxruntime.a", ["dylib=onnxruntime"]),
        ]
        for archive_name, linked_libs in cases:
            with self.subTest(archive=archive_name, linked_libs=linked_libs), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                tesseract_out = root / "tesseract/out"
                write_archive(tesseract_out / "libtesseract.a")
                write_archive(tesseract_out / "libleptonica.a")
                ort_out = root / "ort/out"
                write_archive(ort_out / archive_name)
                invocation = root / "release-cargo/1-1"
                invocation.mkdir(parents=True)
                rows = [
                    receipt("path+file:///w/crates/xberg-tesseract#1.4.0", tesseract_out, [], []),
                    receipt("registry+https://example#ort-sys@2.0.0", ort_out, linked_libs, []),
                ]
                (invocation / "stdout.log").write_text("\n".join(json.dumps(row) for row in rows) + "\n")
                with raises(RuntimeError, "static ORT"):
                    collect(root / "release-cargo")


if __name__ == "__main__":
    unittest.main()
