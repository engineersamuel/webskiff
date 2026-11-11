# Releasing Webskiff

Source-built Homebrew installation is the primary distribution path.
Apple signing is optional future work for prebuilt Webskiff downloads.
The application's checked-in workflows prepare binary releases; they do not publish them.
A successful build is not permission to claim production readiness.

## Current gates

The source remote is `https://github.com/engineersamuel/webskiff.git` (public).
The project owner approved public source and redistribution. The tracked source
and all 20 existing commits passed a local redacted Gitleaks scan before the
visibility change. This scan is not a guarantee that every sensitive item is absent.
Repository-local HTTPS Git
authentication uses the GitHub CLI credential helper, not a stored token file.
`PUBLIC_RELEASE_ENABLED=false` keeps public release jobs disabled.
The protected `release` environment requires approval from `engineersamuel`,
allows only `v*` tags, and does not allow administrator bypass. Public visibility
removed the previous plan restriction on these protections.
Webskiff's own code is licensed under MIT, with `LICENSE` and Cargo SPDX metadata
in place. Zebkit is credited in the README and third-party notices. A complete
reviewed engine license bundle is still needed before Webskiff binary redistribution.
Apple signing credentials are not required for source-built Homebrew installation.
Native macOS and Linux CI have passed the build,
keyboard, and relocated-engine tests; macOS also passed the isolated Homebrew test.
See the [verified native run](https://github.com/engineersamuel/webskiff/actions/runs/42898490128)
for source commit `3869c74`. The separate
[tap workflow](https://github.com/engineersamuel/homebrew-tap/actions/workflows/test.yml)
tests the full source-install path on fresh macOS ARM64 runners.
The [source-install candidate check](https://github.com/engineersamuel/homebrew-tap/actions/runs/42940350241)
passed source build, strict audit, installed engine/keyboard checks, and uninstall
on macOS 15.8.2 ARM64. The
[public-tap check](https://github.com/engineersamuel/homebrew-tap/actions/runs/42940766526)
uses the exact `brew install engineersamuel/tap/webskiff` command without a local tap override.

Before distributing prebuilt Webskiff binaries:

1. Retain the owner's publication and redistribution approval. Keep separate any
   company approval needed for using sensitive accounts or deploying internally.
1. Bundle the required Zebkit/Chromium and Rust dependency license texts and
   notices under `packaging/third-party/`. Preserve the source/relinking material
   required by the approved redistribution terms. `dependencies.json` is an
   inventory, not a substitute for these materials.
1. Keep source-tap installation separate from binary packaging. The source tap
   downloads the engine directly from upstream and builds Webskiff locally;
   it does not use or weaken the signed-binary release gates.
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
The Ubuntu job builds and exercises the real browser keyboard controls. It allows
unprivileged user namespaces only on the disposable hosted runner so Chromium
can keep its sandbox enabled. Linux remains experimental; see
[sandbox startup requirements](usage.md#linux-sandbox-startup).
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

## Source-built Homebrew tap

The tap is [engineersamuel/homebrew-tap](https://github.com/engineersamuel/homebrew-tap).
Users install with `brew install engineersamuel/tap/webskiff`.
It currently targets macOS on Apple Silicon only. Homebrew supplies the Rust
build dependency. No Apple account, certificate, or paid membership is needed.
The Chromium engine is still a pinned upstream binary, not built from source.

Generate a formula from the checkout matching the intended source commit:

```fish
set source_commit (git rev-parse HEAD)
uv run --no-project python -I scripts/release.py brew-source \
  --repository engineersamuel/webskiff --commit "$source_commit" \
  --output dist/Formula/webskiff.rb
```

Push that source commit to GitHub first. The generator requires a full immutable
commit SHA, downloads the real source archive, computes its SHA-256, and checks
that its Cargo and engine metadata match the local checkout. The engine resource
URL and hash come from `packaging/engines.json`. Mutable refs and existing output
paths are rejected. Bump the application version before a later tap release;
do not silently change the source behind an already published version.

The formula uses Homebrew's standard locked Cargo installation, installs the
engine beside the executable under `libexec/bin`, and creates an exec wrapper
at `bin/webskiff`. A plain symlink failed engine startup and is not used.
The example configuration, MIT license, and third-party credits are installed
under `share/webskiff`. The formula does not change or remove user files.

For tap updates:

1. Generate the formula and put it under `Formula/webskiff.rb` in a tap branch.
1. Run the tap's **Source install** workflow on that candidate. It uses actual
   `brew install --build-from-source`, the normal build sandbox, formula style
   and strict audit checks, `brew test`, an installed-engine self-test, and all
   three real-browser keyboard checks against the installed wrapper.
1. Confirm uninstall removes the wrapper but preserves user configuration and
   profile sentinels. An initial install cannot prove upgrades from an older
   version; test upgrades when a second version exists.
1. Review and approve the tap update before merging its PR. Do not merge without
   the owner's explicit approval for that specific merge.
1. Dispatch the workflow on `main` with `public-tap=true`. A separate fresh Mac
   runner then installs from the real public tap without a local override.
1. Review a real compatible terminal on a separate Mac before broad release or
   announcements. Hosted PTY checks do not prove physical display, media, SSO,
   accessibility, or company-policy acceptance.

The workflow does not use `--no-sandbox`, remove quarantine, disable Gatekeeper,
publish bottles, or need Apple secrets. Source installation is independent of
`PUBLIC_RELEASE_ENABLED`, which continues to disable prebuilt release jobs.
`brew test` checks CLI version/help. Chromium cannot start its sandbox inside
Homebrew's test sandbox, so the engine and browser checks run separately through
the installed wrapper with disposable profiles. Chromium's sandbox stays enabled.

## Optional signing and draft binary release workflow

The source-tap path above does not need this setup. Keep these steps for a future
decision to distribute signed, notarized prebuilt Webskiff downloads.

The GitHub environment named **`release`** is configured with owner approval,
`v*` tag-only deployment restrictions, and administrator bypass disabled.
The owner may approve their own run; this is a single-maintainer project.
Set the repository variable `PUBLIC_RELEASE_ENABLED=true` only after the legal
and platform gates are met. The workflow never publishes a draft automatically.

### One-time Apple setup

These steps must be done by the account owner. Do not send certificates, private
keys, or passwords to the assistant or commit them to the repository.

1. Use an active [Apple Developer Program](https://developer.apple.com/programs/)
   membership. Apple requires the **Account Holder** role to create the locally
   issued Developer ID certificate used by this workflow.
1. On your Mac, open **Keychain Access > Certificate Assistant > Request a
   Certificate from a Certificate Authority**. Enter your email and a key name,
   leave CA Email Address blank, and choose **Saved to disk**. See
   [Apple's CSR instructions](https://developer.apple.com/help/account/certificates/create-a-certificate-signing-request).
1. In [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list),
   add a **Developer ID Application** certificate using that CSR. Do not choose
   Apple Development, Developer ID Installer, or a cloud-managed certificate.
   Download the `.cer` file and double-click it on the Mac that created the CSR.
   See [Apple's Developer ID instructions](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/).
1. In Keychain Access, select **My Certificates**. Find
   `Developer ID Application: NAME (TEAMID)` and confirm it has a private key.
   Export that certificate and its private key as
   `webskiff-signing.p12`, with an export password. Keep the file and password secure.
1. At [account.apple.com](https://account.apple.com), open **Sign-In and Security >
   App-Specific Passwords** and generate a password named `Webskiff notarization`.
   Two-factor authentication is required. Use this password, not your normal
   Apple Account password. See [Apple's password instructions](https://support.apple.com/en-us/102654).
1. Open [repository environment settings](https://github.com/engineersamuel/webskiff/settings/environments),
   select **release**, and add the six **Environment secrets** below. Use the
   Apple Developer account's Membership details for the Team ID.

| Secret | Purpose |
|---|---|
| `MACOS_CERTIFICATE_P12` | Base64-encoded `.p12` containing the certificate and private key |
| `MACOS_CERTIFICATE_PASSWORD` | Password for the certificate export |
| `MACOS_SIGNING_IDENTITY` | Exact `Developer ID Application: NAME (TEAMID)` identity |
| `APPLE_ID` | Apple Account email used for notarization |
| `APPLE_TEAM_ID` | Apple developer team |
| `APPLE_APP_PASSWORD` | App-specific notarization password |

To copy the encoded `.p12` for its GitHub secret, run this yourself on your Mac
(Fish syntax), using the actual export path:

```fish
base64 -i "$HOME/Downloads/webskiff-signing.p12" | pbcopy
```

Paste only into `MACOS_CERTIFICATE_P12` in GitHub, not into chat. When all six
secrets are saved, report only **"Apple signing secrets are configured."**
Do not enable `PUBLIC_RELEASE_ENABLED` yet.

### Signing and release validation

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

## Optional binary-package Homebrew formula

The generated formula installs the binary and engine under `libexec`, then
creates an exec wrapper at `bin/webskiff`. A plain symlink failed the packaged
engine test; the wrapper invokes the real executable beside its engine.
It does not change or delete user configuration or profiles.
Its `brew test` checks version/help. Run engine checks separately to avoid
nesting Chromium's sandbox inside Homebrew's test sandbox.

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
