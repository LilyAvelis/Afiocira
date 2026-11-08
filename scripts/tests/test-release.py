"""Offline release corruption checks, followed by a fully mocked publisher.

All executables below are tiny byte fixtures. No product build, git, credentials,
network requests, or external gh command is needed.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import warnings
import zipfile

ROOT = Path(__file__).resolve().parents[2]
VERSION = "2.5.0"
COMMIT = "a" * 40
ZIP_NAME = f"Afiocira-{VERSION}-Windows-x64.zip"
INSTALLER = f"Afiocira-{VERSION}-Setup-Windows-x64.exe"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def fixture(base, case):
    directory = base / case
    directory.mkdir()
    content = {name: b"fake executable" for name in (
        "Afiocira.exe", "photo-cli.exe", "AfioLauncher.exe")}
    content.update({name: b"document" for name in (
        "README.md", "CHANGELOG.md", "LICENSE", "current-version.txt")})
    # Deliberately exercise a dynamic manifest containing documentation.
    content.update({f"docs/chapter-{i}.md": b"content" for i in range(44)})
    if case == "private-utf8":
        content["Afiocira.exe"] = b"x" * 65520 + str(ROOT).encode("utf-8")
    if case == "private-utf16":
        content["photo-cli.exe"] = b"x" * 65520 + os.environ["USERPROFILE"].encode("utf-16le")
    if case == "upstream-private":
        content["upstream.exe"] = str(ROOT).encode("utf-8")
    manifest = {"version": VERSION, "source_commit": COMMIT, "files": [
        {"path": name, "bytes": len(data), "sha256": sha256(data)}
        for name, data in content.items()
    ]}
    if case == "manifest-hash":
        manifest["files"][0]["sha256"] = "0" * 64
    if case == "source":
        manifest["source_commit"] = "b" * 40
    content["PACKAGE-MANIFEST.json"] = json.dumps(manifest).encode("utf-8")
    archive = directory / ZIP_NAME
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for name, data in content.items():
            output.writestr(name, data)
        if case == "duplicate":
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                output.writestr("Afiocira.exe", b"duplicate")
    if case == "crc":
        data = bytearray(archive.read_bytes())
        # Alter only the central-directory CRC, then refresh outer SHA256SUMS.
        data[data.index(b"PK\x01\x02") + 16] ^= 1
        archive.write_bytes(data)
    (directory / INSTALLER).write_bytes(b"fake installer")
    (directory / "SHA256SUMS.txt").write_text("".join(
        f"{sha256((directory / name).read_bytes())}  {name}\n"
        for name in (ZIP_NAME, INSTALLER)
    ), encoding="ascii")
    if case == "checksum":
        (directory / INSTALLER).write_bytes(b"tampered installer")
    stage = None
    if case in ("stage", "stage-tamper"):
        stage = base / f"{case}-files"
        for name, data in content.items():
            path = stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        if case == "stage-tamper":
            (stage / "README.md").write_bytes(b"changed after staging")
    return directory, stage


def run_verify(directory, stage):
    args = ["pwsh", "-NoProfile", "-File", str(ROOT / "scripts/verify-release.ps1"),
            "-AssetsDirectory", str(directory), "-ExpectedCommit", COMMIT,
            "-Version", VERSION]
    if stage:
        args += ["-StageDirectory", str(stage)]
    return subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace")


def main():
    test_root = ROOT / "target/ci-self-tests"
    test_root.mkdir(parents=True, exist_ok=True)
    # TemporaryDirectory only removes its freshly-created child of test_root.
    with tempfile.TemporaryDirectory(prefix="release-", dir=test_root) as temporary:
        base = Path(temporary)
        expected_failures = {
            "crc": "CRC32 mismatch", "checksum": "Asset SHA256 mismatch",
            "manifest-hash": "Manifest hash or size mismatch",
            "source": "version or source commit", "duplicate": "Duplicated ZIP entry",
            "private-utf8": "Private build path", "private-utf16": "Private build path",
            "stage-tamper": "Stage differs from ZIP",
        }
        good = None
        for case in ["good", "stage", *expected_failures, "upstream-private"]:
            directory, stage = fixture(base, case)
            result = run_verify(directory, stage)
            if case in expected_failures:
                if result.returncode == 0 or expected_failures[case] not in result.stderr:
                    raise AssertionError(f"{case}: wrong failure\n{result.stdout}\n{result.stderr}")
            elif result.returncode:
                raise AssertionError(f"{case}: verification failed\n{result.stdout}\n{result.stderr}")
            if case == "good":
                good = directory
            print(f"PASS release fixture: {case}", flush=True)
        subprocess.run(["pwsh", "-NoProfile", "-File",
                        str(ROOT / "scripts/tests/test-publish-mock.ps1"),
                        "-AssetsDirectory", str(good)], check=True)


if __name__ == "__main__":
    main()
