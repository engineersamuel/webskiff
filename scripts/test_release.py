"""Regression tests for the release tool. Run with unittest discovery."""

import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="webskiff-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def archive(self, target="aarch64-apple-darwin", public=False):
        name = f"webskiff-{release.version()}-{target}"
        archive = self.root / f"{name}.tar.gz"
        data = json.dumps({
            "version": release.version(), "target": target, "public_distribution": public,
        }).encode()
        with tarfile.open(archive, "w:gz") as stream:
            record = tarfile.TarInfo(f"{name}/build.json")
            record.size = len(data)
            stream.addfile(record, io.BytesIO(data))
        return archive

    def test_tag_matches_cargo_and_rejects_malformed_prerelease(self):
        release.check_tag(f"v{release.version()}")
        for tag in ("v9.9.9", "0.1.0", "v0.1.0;echo bad"):
            with self.assertRaises(ValueError):
                release.check_tag(tag)
        for value in ("01.1.0", "0.1.0-01", "0.1", "0.1.0-"):
            self.assertIsNone(release.SEMVER.fullmatch(value))

    def test_engine_version_and_target_are_pinned(self):
        release.target_info("aarch64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "no verified upstream engine"):
            release.target_info("aarch64-unknown-linux-gnu")
        cargo = release.manifest()
        cargo["dependencies"]["zebkit"] = "=999"
        with patch.object(release, "manifest", return_value=cargo):
            with self.assertRaisesRegex(ValueError, "versions differ"):
                release.engines()

    def test_engine_download_rejects_wrong_checksum_before_extracting(self):
        archive = self.root / "engine.tar.xz"
        archive.write_bytes(b"not the approved engine")
        (self.root / "packaging").mkdir()
        shutil_data = release.ROOT / "packaging/engines.json"
        (self.root / "packaging/engines.json").write_bytes(shutil_data.read_bytes())
        cargo = release.manifest()
        with patch.object(release, "ROOT", self.root), patch.object(release, "manifest", return_value=cargo):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                release.fetch_engine("aarch64-apple-darwin", archive)
        self.assertFalse((self.root / "engine/verified").exists())

    def test_archive_cannot_extract_outside_destination(self):
        archive = self.root / "bad.tar.gz"
        with tarfile.open(archive, "w:gz") as stream:
            member = tarfile.TarInfo("../outside")
            member.size = 3
            stream.addfile(member, io.BytesIO(b"bad"))
        with self.assertRaises(tarfile.FilterError):
            release.unpack_archive(archive, self.root / "extract")
        self.assertFalse((self.root / "outside").exists())

    def test_formula_uses_exact_artifact_hash_and_keeps_engine_beside_binary(self):
        archive = self.archive()
        output = self.root / "Formula/webskiff.rb"
        release.brew_formula("example/webskiff", self.root, output, require_release=False)
        formula = output.read_text()
        self.assertIn(release.checksum(archive), formula)
        self.assertIn(archive.resolve().as_uri(), formula)
        self.assertIn('libexec.install "webskiff", "zebkit"', formula)
        self.assertIn('bin.write_exec_script libexec/"webskiff"', formula)
        with self.assertRaises(FileExistsError):
            release.brew_formula("example/webskiff", self.root, output, require_release=False)

    def test_formula_refuses_internal_packages_for_public_installation(self):
        self.archive()
        with patch.object(release, "check_distribution_approval"):
            with self.assertRaisesRegex(ValueError, "unsigned/internal"):
                release.brew_formula("example/webskiff", self.root, self.root / "webskiff.rb")
        with self.assertRaisesRegex(ValueError, "owner/name"):
            release.brew_formula('bad/"repository', self.root, self.root / "bad.rb", False)

    def source_archive(self, commit, cargo=None, engine=None):
        archive = self.root / "source.tar.gz"
        documents = {
            "Cargo.toml": (release.ROOT / "Cargo.toml").read_bytes(),
            "packaging/engines.json": (release.ROOT / "packaging/engines.json").read_bytes(),
        }
        if cargo is not None:
            documents["Cargo.toml"] = cargo
        if engine is not None:
            documents["packaging/engines.json"] = engine
        with tarfile.open(archive, "w:gz") as stream:
            for name, data in documents.items():
                member = tarfile.TarInfo(f"webskiff-{commit}/{name}")
                member.size = len(data)
                stream.addfile(member, io.BytesIO(data))
        return archive

    def test_source_formula_builds_locally_with_pinned_source_and_engine_without_signing(self):
        commit = "a" * 40
        archive = self.source_archive(commit)
        output = self.root / "Formula/webskiff.rb"

        def download(command, **kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(archive.read_bytes())

        with patch.object(release.subprocess, "run", side_effect=download), patch.object(
            release, "check_distribution_approval", side_effect=AssertionError("binary-only gate"),
        ), patch.dict(os.environ, {}, clear=True):
            release.source_brew_formula("example/webskiff", commit, output)
        formula = output.read_text()
        self.assertIn(f"/archive/{commit}.tar.gz", formula)
        self.assertIn(release.checksum(archive), formula)
        self.assertIn(release.engines()["targets"]["aarch64-apple-darwin"]["sha256"], formula)
        self.assertIn('system "cargo", "install", *std_cargo_args(root: libexec), "--bin", "webskiff"', formula)
        self.assertIn('(libexec/"bin").install "target/release/zebkit"', formula)
        self.assertIn('bin.write_exec_script libexec/"bin/webskiff"', formula)
        self.assertIn('shell_output("#{bin}/webskiff --help")', formula)
        self.assertNotIn("--selftest", formula)

    def test_source_formula_rejects_mutable_refs_mismatched_metadata_and_overwrites(self):
        output = self.root / "webskiff.rb"
        with patch.object(release.subprocess, "run") as download:
            with self.assertRaisesRegex(ValueError, "immutable commit"):
                release.source_brew_formula("example/webskiff", "main", output)
        download.assert_not_called()
        commit = "b" * 40

        def download(command, **kwargs):
            Path(command[command.index("--output") + 1]).write_bytes(archive.read_bytes())

        for mismatch in ({"cargo": b'[package]\nversion = "99.0.0"\n'}, {"engine": b"{}"}):
            with self.subTest(mismatch=mismatch):
                archive = self.source_archive(commit, **mismatch)
                with patch.object(release.subprocess, "run", side_effect=download):
                    with self.assertRaisesRegex(ValueError, "metadata differs"):
                        release.source_brew_formula("example/webskiff", commit, output)
        self.assertFalse(output.exists())
        output.write_text("existing formula")
        with self.assertRaises(FileExistsError):
            release.source_brew_formula("example/webskiff", commit, output)

    def test_public_distribution_fails_without_approved_license(self):
        with patch.object(release, "ROOT", self.root), patch.object(
            release, "manifest", return_value={"package": {}},
        ):
            with self.assertRaisesRegex(ValueError, "approved project license"):
                release.check_distribution_approval()

    def test_verify_rejects_tampered_packages_before_launching(self):
        archive = self.archive()
        archive.with_name(archive.name + ".sha256").write_text(f"{'0' * 64}  {archive.name}\n")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            release.verify(archive)

    def test_notarization_must_be_accepted(self):
        with patch.dict(os.environ, {"MACOS_SIGNING_IDENTITY": "test identity"}):
            with patch.object(release.subprocess, "run"), patch.object(
                release.subprocess, "check_output", return_value='{"status":"Invalid"}',
            ):
                with self.assertRaisesRegex(ValueError, "notarization did not pass"):
                    release.sign_macos(self.root)

    def test_release_can_only_create_a_draft_with_all_targets(self):
        for target in release.engines()["targets"]:
            archive = self.archive(target=target, public=True)
            archive.with_name(archive.name + ".sha256").write_text(
                f"{release.checksum(archive)}  {archive.name}\n",
            )
        (self.root / "webskiff.rb").write_text("formula")
        with patch.object(release, "check_distribution_approval"), patch.object(
            release, "github_request", return_value={"id": 123, "html_url": "draft-url"},
        ) as request:
            release.draft_release("example/webskiff", f"v{release.version()}", self.root)
        url, data = request.call_args_list[0].args
        self.assertEqual(url, "https://api.github.com/repos/example/webskiff/releases")
        payload = json.loads(data)
        self.assertIs(payload["draft"], True)
        self.assertIs(payload["prerelease"], True)
        self.assertEqual(request.call_count, 6)

    def test_release_with_missing_targets_never_contacts_github(self):
        with patch.object(release, "check_distribution_approval"), patch.object(
            release, "github_request",
        ) as request:
            with self.assertRaisesRegex(ValueError, "every enabled target"):
                release.draft_release("example/webskiff", f"v{release.version()}", self.root)
        request.assert_not_called()


if __name__ == "__main__":
    unittest.main()
