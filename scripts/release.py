"""Build and verify relocatable Webskiff packages. Requires Python 3.12+."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
USER_AGENT = "OpenAI File Downloader, XaiImageApiFetch/1.0"
SEMVER = re.compile(
    r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
)


def manifest():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())


def version():
    value = manifest()["package"]["version"]
    if not SEMVER.fullmatch(value):
        raise ValueError(f"invalid release version: {value}")
    return value


def engines():
    data = json.loads((ROOT / "packaging/engines.json").read_text())
    expected = f'={data["version"]}'
    cargo = manifest()
    if (cargo["dependencies"]["zebkit"] != expected
            or cargo["build-dependencies"]["zebkit-build"] != expected):
        raise ValueError("Cargo.toml and packaging/engines.json engine versions differ")
    return data


def host_target():
    systems = {"Darwin": "apple-darwin", "Linux": "unknown-linux-gnu"}
    machines = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}
    return f"{machines[platform.machine()]}-{systems[platform.system()]}"


def target_info(target):
    data = engines()
    if target not in data["targets"]:
        raise ValueError(f"no verified upstream engine for target: {target}")
    return data, data["targets"][target]


def checksum(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_tag(tag):
    if tag != f"v{version()}":
        raise ValueError(f"tag {tag!r} must match Cargo.toml: v{version()}")


def metadata():
    return json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT, text=True,
    ))


def fetch_engine(target, archive=None):
    data, info = target_info(target)
    name = f"zebkit-{data['version']}-{target}"
    parent = ROOT / "engine"
    parent.mkdir(exist_ok=True)
    destination = parent / "verified" / name
    marker = destination / ".webskiff-sha256"
    if destination.exists():
        if marker.is_file() and marker.read_text().strip() == info["sha256"]:
            return destination
        raise ValueError(f"unverified engine directory already exists: {destination}")
    with tempfile.TemporaryDirectory(prefix="download-", dir=parent) as scratch:
        scratch = Path(scratch)
        if archive is None:
            archive = scratch / f"{name}.tar.xz"
            url = f"https://github.com/ikey4u/zebkit/releases/download/v{data['version']}/{name}.tar.xz"
            subprocess.run(
                ["curl", "--fail", "--location", "--silent", "--show-error",
                 "--retry", "3", "--user-agent", USER_AGENT,
                 "--output", str(archive), url],
                check=True,
            )
        if checksum(archive) != info["sha256"]:
            raise ValueError(f"engine checksum mismatch: {archive}")
        unpack_archive(archive, scratch)
        unpacked = scratch / name
        if not unpacked.is_dir():
            raise ValueError(f"engine archive has no {name} directory")
        (unpacked / marker.name).write_text(info["sha256"] + "\n")
        destination.parent.mkdir(exist_ok=True)
        unpacked.rename(destination)
    return destination


def unpack_archive(archive, destination):
    with tarfile.open(archive, "r:*") as stream:
        stream.extractall(destination, filter="data")


def dependency_inventory():
    return {
        "application": {"name": "webskiff", "version": version()},
        "engine": engines(),
        "packages": [
            {key: package.get(key) for key in ("name", "version", "license", "repository")}
            for package in metadata()["packages"]
        ],
    }


def check_distribution_approval():
    package = manifest()["package"]
    if not package.get("license") or not (ROOT / "LICENSE").is_file():
        raise ValueError("public distribution requires an approved project license in Cargo.toml and LICENSE")
    legal = ROOT / "packaging/third-party"
    if not legal.is_dir() or not any(legal.iterdir()):
        raise ValueError("public distribution requires reviewed engine and dependency license notices in packaging/third-party")


def copy_documentation(bundle):
    for item in (
        "README.md", "config.example.toml", "THIRD_PARTY_NOTICES.md",
        "CHANGELOG.md", "SECURITY.md", "CONTRIBUTING.md", "AGENTS.md", "HANDOFF.md",
    ):
        shutil.copy2(ROOT / item, bundle / item)
    for item in ("assets", "docs"):
        shutil.copytree(ROOT / item, bundle / item)
    for item in ("LICENSE", "packaging/third-party"):
        source = ROOT / item
        if source.is_file():
            shutil.copy2(source, bundle / source.name)
        elif source.is_dir():
            shutil.copytree(source, bundle / "third-party")


def package(target, output, release=False):
    target_info(target)
    if target != host_target():
        raise ValueError("build and package natively on the target system")
    if release:
        check_distribution_approval()
    binary = ROOT / "target/release/webskiff"
    engine = binary.parent / "zebkit"
    if not binary.is_file() or not engine.is_dir():
        raise ValueError("build with cargo build --release --locked --bin webskiff first")
    description = subprocess.check_output([binary, "--version"], text=True).strip()
    if (f"webskiff {version()} " not in description or target not in description
            or f"zebkit {engines()['version']};" not in description):
        raise ValueError(f"stale or wrong-target executable: {description}")
    output.mkdir(parents=True, exist_ok=True)
    name = f"webskiff-{version()}-{target}"
    archive = output / f"{name}.tar.gz"
    if archive.exists():
        raise ValueError(f"refusing to replace release artifact: {archive}")
    with tempfile.TemporaryDirectory(prefix="webskiff-package-") as scratch:
        bundle = Path(scratch) / name
        bundle.mkdir()
        shutil.copy2(binary, bundle / "webskiff")
        shutil.copytree(engine, bundle / "zebkit", symlinks=True)
        copy_documentation(bundle)
        (bundle / "dependencies.json").write_text(json.dumps(dependency_inventory(), indent=2) + "\n")
        if target.endswith("apple-darwin") and release:
            sign_macos(bundle)
        (bundle / "build.json").write_text(json.dumps({
            "version": version(), "target": target, "description": description,
            "public_distribution": release, "engine": engines()["version"],
        }, indent=2) + "\n")
        with tarfile.open(archive, "x:gz") as stream:
            stream.add(bundle, arcname=name)
    archive.with_name(archive.name + ".sha256").write_text(f"{checksum(archive)}  {archive.name}\n")
    return archive


def sign_macos(bundle):
    identity = os.environ.get("MACOS_SIGNING_IDENTITY")
    if not identity:
        raise ValueError("public macOS packages require MACOS_SIGNING_IDENTITY")
    framework = bundle / "zebkit/Zebkit.framework"
    helper = framework / "Versions/A/Helpers/Zebkit Helper.app"
    common = ["codesign", "--force", "--timestamp", "--options", "runtime", "--sign", identity]
    entitlements = ROOT / "packaging/macos-entitlements.plist"
    subprocess.run([*common, "--entitlements", str(entitlements), str(helper)], check=True)
    subprocess.run([*common, str(framework)], check=True)
    subprocess.run([*common, "--entitlements", str(entitlements), str(bundle / "webskiff")], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(framework)], check=True)
    subprocess.run(["codesign", "--verify", "--strict", str(bundle / "webskiff")], check=True)
    with tempfile.TemporaryDirectory(prefix="webskiff-notary-") as scratch:
        upload = Path(scratch) / "webskiff.zip"
        subprocess.run(["ditto", "-c", "-k", "--keepParent", str(bundle), str(upload)], check=True)
        result = subprocess.check_output(
            ["xcrun", "notarytool", "submit", str(upload), "--wait",
             "--keychain-profile", "webskiff-notary", "--output-format", "json"],
            text=True,
        )
        response = json.loads(result)
        if response.get("status") != "Accepted":
            raise ValueError(f"notarization did not pass: {response}")


def verify(archive, selftest=False):
    archive = archive.resolve()
    expected = archive.with_name(archive.name + ".sha256").read_text().split()
    if expected != [checksum(archive), archive.name]:
        raise ValueError(f"package checksum mismatch: {archive}")
    with tempfile.TemporaryDirectory(prefix="webskiff-relocate-") as scratch:
        scratch = Path(scratch)
        unpack_archive(archive, scratch)
        bundles = list(scratch.iterdir())
        if len(bundles) != 1 or not bundles[0].is_dir():
            raise ValueError("release archive must contain one bundle directory")
        bundle = bundles[0]
        record = json.loads((bundle / "build.json").read_text())
        if record["target"] != host_target() or record["version"] != version():
            raise ValueError("package target/version mismatch")
        environment = os.environ.copy()
        environment.pop("DYLD_LIBRARY_PATH", None)
        environment.pop("LD_LIBRARY_PATH", None)
        environment["HOME"] = str(scratch / "home")
        Path(environment["HOME"]).mkdir()
        binary = bundle / "webskiff"
        for args in (["--version"], ["--help"]):
            subprocess.run([binary, *args], cwd=scratch, env=environment, check=True, timeout=30)
        if selftest:
            result = subprocess.run(
                [binary, "--selftest"], cwd=scratch, env=environment,
                check=True, capture_output=True, text=True, timeout=90,
            )
            print(result.stdout)
            if "ALL PASSED" not in result.stdout or "TIMEOUT" in result.stdout:
                raise ValueError("packaged engine self-test did not pass")
    return record


def brew_clause(repository, archive, require_release):
    target = archive.name.removeprefix(f"webskiff-{version()}-").removesuffix(".tar.gz")
    target_info(target)
    with tarfile.open(archive, "r:gz") as stream:
        records = [item for item in stream.getmembers() if item.name.endswith("/build.json")]
        if len(records) != 1:
            raise ValueError("archive must contain one build.json")
        record = json.load(stream.extractfile(records[0]))
    if require_release and not record["public_distribution"]:
        raise ValueError(f"cannot advertise an unsigned/internal artifact: {archive.name}")
    if record["target"] != target or record["version"] != version():
        raise ValueError(f"archive metadata mismatch: {archive.name}")
    conditions = {
        "aarch64-apple-darwin": "on_macos do\n    on_arm do",
        "x86_64-unknown-linux-gnu": "on_linux do\n    on_intel do",
    }
    url = (
        f"https://github.com/{repository}/releases/download/v{version()}/{archive.name}"
        if require_release else archive.resolve().as_uri()
    )
    return f'  {conditions[target]}\n      url "{url}"\n      sha256 "{checksum(archive)}"\n    end\n  end'


def brew_formula(repository, directory, output, require_release=True):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("repository must be owner/name")
    if require_release:
        check_distribution_approval()
    archives = sorted(directory.glob(f"webskiff-{version()}-*.tar.gz"))
    if not archives:
        raise ValueError("no release archives found for the current version")
    clauses = [brew_clause(repository, archive, require_release) for archive in archives]
    license_id = manifest()["package"].get("license")
    license_line = f'  license "{license_id}"\n' if license_id else ""
    text = f'''class Webskiff < Formula
  desc "Keyboard-first Chromium browser for your terminal"
  homepage "https://github.com/{repository}"
  version "{version()}"
{license_line}
{chr(10).join(clauses)}

  on_macos do
    depends_on macos: :ventura
    depends_on arch: :arm64
  end

  on_linux do
    depends_on "alsa-lib"
    depends_on "at-spi2-core"
    depends_on "cairo"
    depends_on "cups"
    depends_on "dbus"
    depends_on "libdrm"
    depends_on "libx11"
    depends_on "libxcb"
    depends_on "libxcomposite"
    depends_on "libxdamage"
    depends_on "libxext"
    depends_on "libxfixes"
    depends_on "libxkbcommon"
    depends_on "libxrandr"
    depends_on "mesa"
    depends_on "nspr"
    depends_on "nss"
    depends_on "pango"
  end

  def install
    libexec.install "webskiff", "zebkit"
    bin.write_exec_script libexec/"webskiff"
    (bin/"webskiff").chmod 0755
    pkgshare.install "config.example.toml", "THIRD_PARTY_NOTICES.md", "dependencies.json", "build.json"
  end

  test do
    assert_match "webskiff #{{version}}", shell_output("#{{bin}}/webskiff --version")
    assert_match "ALL PASSED", shell_output("#{{bin}}/webskiff --selftest")
  end
end
'''
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x") as stream:
        stream.write(text)
    return output


def github_request(url, data, content_type="application/json"):
    request = urllib.request.Request(
        url, data=data, method="POST",
        headers={
            "User-Agent": USER_AGENT,
            "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
            "Content-Type": content_type,
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(request, timeout=180) as response:
        return json.load(response)


def draft_release(repository, tag, directory):
    check_tag(tag)
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("repository must be owner/name")
    check_distribution_approval()
    archives = sorted(directory.glob(f"webskiff-{version()}-*.tar.gz"))
    targets = set()
    for archive in archives:
        brew_clause(repository, archive, require_release=True)
        expected = archive.with_name(archive.name + ".sha256").read_text().split()
        if expected != [checksum(archive), archive.name]:
            raise ValueError(f"release checksum mismatch: {archive}")
        targets.add(archive.name.removeprefix(f"webskiff-{version()}-").removesuffix(".tar.gz"))
    if targets != set(engines()["targets"]):
        raise ValueError("draft release requires a verified package for every enabled target")
    formula = directory / "webskiff.rb"
    if not formula.is_file():
        raise ValueError("generate the Homebrew formula before creating a draft")
    response = github_request(
        f"https://api.github.com/repos/{repository}/releases",
        json.dumps({
            "tag_name": tag, "name": f"Webskiff {version()}",
            "draft": True, "prerelease": "-" in version(),
            "body": (ROOT / "CHANGELOG.md").read_text(),
        }).encode(),
    )
    release_id = response["id"]
    files = [*archives, *(archive.with_name(archive.name + ".sha256") for archive in archives), formula]
    for file in files:
        url = (
            f"https://uploads.github.com/repos/{repository}/releases/{release_id}/assets?"
            + urllib.parse.urlencode({"name": file.name})
        )
        github_request(url, file.read_bytes(), "application/octet-stream")
    print(response["html_url"])


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    commands = cli.add_subparsers(dest="command", required=True)
    commands.add_parser("version")
    tag = commands.add_parser("check-tag")
    tag.add_argument("tag")
    fetch = commands.add_parser("fetch-engine")
    fetch.add_argument("--target", default=host_target())
    fetch.add_argument("--archive", type=Path)
    pack = commands.add_parser("package")
    pack.add_argument("--target", default=host_target())
    pack.add_argument("--output", type=Path, default=ROOT / "dist")
    pack.add_argument("--release", action="store_true", help="Require legal approval and macOS signing")
    check = commands.add_parser("verify")
    check.add_argument("archive", type=Path)
    check.add_argument("--selftest", action="store_true")
    brew = commands.add_parser("brew")
    brew.add_argument("--repository", required=True)
    brew.add_argument("--directory", type=Path, default=ROOT / "dist")
    brew.add_argument("--output", type=Path, required=True)
    brew.add_argument("--internal-test", action="store_true", help="Generate a non-publishable test formula")
    commands.add_parser("check-distribution")
    draft = commands.add_parser("draft")
    draft.add_argument("--repository", required=True)
    draft.add_argument("--tag", required=True)
    draft.add_argument("--directory", type=Path, default=ROOT / "dist")
    return cli


def main():
    cli = parser()
    args = cli.parse_args()
    try:
        if args.command == "version":
            print(version())
        elif args.command == "check-tag":
            check_tag(args.tag)
        elif args.command == "fetch-engine":
            print(fetch_engine(args.target, args.archive))
        elif args.command == "package":
            print(package(args.target, args.output, args.release))
        elif args.command == "verify":
            verify(args.archive, args.selftest)
        elif args.command == "brew":
            print(brew_formula(args.repository, args.directory, args.output, not args.internal_test))
        elif args.command == "draft":
            draft_release(args.repository, args.tag, args.directory)
        else:
            check_distribution_approval()
    except (ValueError, KeyError, OSError, tarfile.TarError, subprocess.SubprocessError) as error:
        cli.exit(1, f"release: {error}\n")


if __name__ == "__main__":
    main()
