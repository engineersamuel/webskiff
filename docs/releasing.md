# Releasing Webskiff

The checked-in workflows prepare releases; they do not publish them.
A successful build is not permission to claim production readiness.

## Current gates

The source remote is `https://github.com/engineersamuel/webskiff.git` (private).
Personal-account GitHub access is restored. Repository-local HTTPS Git
authentication uses the GitHub CLI credential helper, not a stored token file.
`PUBLIC_RELEASE_ENABLED=false` keeps public release jobs disabled.
The current GitHub plan rejects private-repository rulesets and required release
reviewers. A protected release environment could not be configured; the partial
unprotected environment was removed. Upgrade eligibility or separately approve
public repository visibility before configuring these protections. Do not enable
release jobs without them.
This checkout has no selected project license, complete engine license bundle,
or Apple signing credentials. No hosted runner or fresh-machine installation
has been verified here.

Before public distribution:

1. Confirm ownership and publication approval; choose the project license.
   Add `LICENSE` and its SPDX expression in `Cargo.toml`.
1. Obtain and review Zebkit/Chromium and Rust dependency notices. Put the required
   texts and notices under `packaging/third-party/`. Verify LGPL obligations with
   the engine supplier. `dependencies.json` is an inventory, not legal clearance.
1. Push the reviewed source to the application repository and choose a
   `homebrew-tap` repository. The application repository already exists;
   do not make it public without separate approval.
1. Review and include all intended source files. A clean checkout must build.
   Do not commit engine binaries, profiles, credentials, or build outputs.
1. Run CI on macOS ARM64 and experimental Linux x86-64. Fix failures before
   advertising either platform.
1. Test real terminal use and downloads from the release on a separate machine.
   Keep unverified video, corporate sign-in, and accessibility claims out of
   release notes.

The checks in `scripts/release.py check-distribution` are minimum file checks,
not an automated legal review. Do not satisfy them with placeholder files.

## Versioning

`Cargo.toml` is the version source. The initial preview is `0.1.0-beta.1`.
Use `0.1.0-beta.2` for the next preview and `0.1.0` only after preview acceptance.
For versions below 1.0, use a minor bump for incompatible changes and a patch
bump for compatible fixes; document configuration changes explicitly.

To prepare a version:

1. Edit the package version in `Cargo.toml`.
1. Run `cargo update --workspace --offline` to update the workspace lock entry.
1. Update `CHANGELOG.md`, README status, and any compatibility notes.
1. Run the complete checks, then review and commit the intended changes.
1. Create a tag that matches the version exactly, for example
   `git tag -a v0.1.0-beta.1 -m 'Webskiff 0.1.0-beta.1'`.
1. Push the reviewed commit and tag only when release preparation is authorized.

`check-tag` rejects mismatches. Artifact names include version, OS, and
architecture. Existing artifact paths are never overwritten.
Never move a published tag or replace a published package: release a new version.
Release tags accept normal versions or prerelease identifiers, not `+build`
metadata.

## Local checks and packages

Use the pinned Rust toolchain, Python 3.12+, uv, curl, and xz:

```fish
set engine (uv run --no-project python -I scripts/release.py fetch-engine)
env ZEBKIT_DIR="$engine" bash scripts/check.sh
uv run --no-project python -I scripts/release.py package
uv run --no-project python -I scripts/release.py verify dist/webskiff-0.1.0-beta.1-aarch64-apple-darwin.tar.gz --selftest
```

Default packages are **internal test artifacts**, not signed public releases.
`build.json` records `public_distribution: false`.
The verifier extracts to a separate directory, clears library-path overrides,
uses a disposable HOME, and runs help/version plus the real engine self-test.
It is a relocation test, not a clean-OS acceptance test.

The executable and `zebkit/` directory must stay adjacent:

```text
webskiff-<version>-<target>/
  webskiff
  zebkit/
  config.example.toml
  README.md
  THIRD_PARTY_NOTICES.md
  dependencies.json
  build.json
```

Approved public packages also include the project license and third-party texts.
Checksums detect corruption; unsigned checksums alone do not prove publisher
identity.

## CI matrix

`packaging/engines.json` records upstream archive hashes and target status.
The workflow uses native `macos-15` ARM64 and `ubuntu-24.04` x86-64 runners.
Linux has a pinned upstream engine but Webskiff's runner results remain unverified.
There is no upstream 0.4.0 package for macOS Intel or Linux ARM64. Do not add
matrix entries or Homebrew support by guessing archive names.

The current formatter check covers new Rust modules and the build script.
Legacy application/render files have pre-existing formatting differences.
Clippy excludes the existing `items_after_test_module` finding only; do not
silently add broad lint exclusions.

Engine updates must update both Cargo pins and the engine manifest, recheck
upstream hashes/notices, and pass native runtime, keyboard, media, and
performance checks. An engine security release should trigger a prompt review,
not wait for the next feature release. Dependabot checks Cargo and Actions
weekly but does not replace monitoring the engine's release/security channels.

## Signing and draft release workflow

Configure a GitHub environment named **`release`** with required reviewers and
tag-only deployment restrictions before enabling release jobs.
Set the repository variable `PUBLIC_RELEASE_ENABLED=true` only after the legal
and platform gates are met. The workflow never publishes a draft automatically.

Provide these **environment secrets**, never repository files:

| Secret | Purpose |
|---|---|
| `MACOS_CERTIFICATE_P12` | Base64-encoded Developer ID Application certificate |
| `MACOS_CERTIFICATE_PASSWORD` | Password for the certificate export |
| `MACOS_SIGNING_IDENTITY` | Developer ID Application identity |
| `APPLE_ID` | Notarization account |
| `APPLE_TEAM_ID` | Apple developer team |
| `APPLE_APP_PASSWORD` | App-specific notarization password |

The macOS job imports credentials into a temporary keychain, signs the copied
helper/framework/browser, submits a ZIP to Apple, requires an `Accepted` result,
and removes the keychain. It does not modify the source engine cache.
The helper and browser receive the JIT entitlement Chromium needs.
Signing/notarization is implemented but cannot be tested without those credentials.

The distributed tarball holds notarized executables. Raw command-line Mach-O
files cannot have a stapled ticket like an app bundle; test the quarantined
download on a clean Mac with Gatekeeper enabled and online access. Do not advise
users to disable Gatekeeper or remove quarantine to hide a packaging failure.

Release preparation:

1. Validate tag/version and legal prerequisites.
1. Run CI and relocatable-package tests on both enabled targets.
1. Repackage the exact tested executable/engine; sign and notarize on macOS.
1. Run the final packaged-engine test again.
1. Generate `webskiff.rb` with the exact artifact URLs and SHA-256 hashes.
1. Create a **draft** GitHub Release with packages, checksums, formula, and notes.

Failed uploads leave the release in draft. Inspect partial results; do not
overwrite assets with `--clobber`. A published release is a separate manual
decision.

## Homebrew tap

The generated formula installs the binary and engine under `libexec`, then
creates an exec wrapper at `bin/webskiff`. A plain symlink failed the packaged
engine test; the wrapper invokes the real executable beside its engine.
It does not change or delete user configuration or profiles.
Its `brew test` checks the version and headless engine operation.

CI also runs the actual formula installation method in an isolated prefix and
starts the engine through its installed wrapper. This does not replace testing
Homebrew's complete download, sandbox, upgrade, and uninstall flow from the real
tap on a fresh machine.

For a local, non-publishable formula check:

```sh
uv run --no-project python -I scripts/release.py brew --repository OWNER/REPOSITORY --output dist/webskiff-internal.rb --internal-test
```

Replace `OWNER/REPOSITORY` with the actual application repository. Internal
formula URLs use local `file://` archives and must never be published.
For real releases, omit `--internal-test`; unsigned/internal packages are rejected.

On macOS, test the generated formula's actual installation method and engine:

```sh
HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 brew ruby scripts/test-brew.rb \
  "$PWD/dist/webskiff-internal.rb" \
  "$PWD/dist/webskiff-0.1.0-beta.1-aarch64-apple-darwin.tar.gz"
```

Before publishing the tap formula:

1. Test the final package independently of the tap.
1. Put the generated `webskiff.rb` under `Formula/` in the tap.
1. After release assets are publicly available, test the actual download with
   `brew install OWNER/tap/webskiff`, `brew test OWNER/tap/webskiff`,
   `brew audit --strict OWNER/tap/webskiff`, upgrade, and uninstall.
1. Confirm uninstall leaves user profile/configuration untouched.
1. Add the real install command and working release badges to the README.

Tap updates are reviewed manually. The workflow never pushes to another
repository or publishes a formula automatically.

## Demo and announcements

The logo variants and the 1200x630 social PNG/SVG are in `assets/`.
The PNG was exported with Chromium and checked visually.

Generate a real local-fixture recording without touching your profile:

```sh
uv run --no-project python -I scripts/record-demo.py target/release/webskiff assets/demo.cast.gz
```

The recorder refuses to replace an existing file. Asciicast v2 output includes
Kitty graphics, so replay it with asciinema in a compatible terminal at 120x40
cells. Web players and text-only GIF converters do not reproduce the page
images. This is a raw output recording, not an edited launch video.

With asciinema already installed, replay the checked-in recording (Fish):

```fish
set demo (mktemp -t webskiff-demo)
gzip -dc assets/demo.cast.gz > "$demo"
asciinema play "$demo"
rm "$demo"
```

Record a real 20-30 second terminal session on public/local pages showing open,
scroll, `/` search, Enter then `n`/`N`, hints, tabs, and `?` help. Use a fresh
profile. Hide notifications and review frames for private content. Do not use a
mock browser animation or include YouTube as a supported feature.

Before posting on X, include the actual release link, tested platform, terminal
requirement, preview status, and limitations. Before an internal post, also state
that this is not approval for company credentials or sensitive data. Company
SSO/MFA, corporate certificates/proxies, device policy, and profile storage need
the company's own acceptance process. Do not imply company endorsement.
Unpublished [announcement drafts](announcements.md) are ready for review after
the release gates pass.
