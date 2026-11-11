# Changelog

## [0.1.0-beta.2](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.1...v0.1.0-beta.2) (2026-11-11)


### Features

* automate beta release PRs and tested Homebrew upgrades ([910e00e](https://github.com/engineersamuel/webskiff/commit/910e00e8d6c75c7f41dd8cb81284e47d4cd64578))


### Bug Fixes

* preserve release runs and document automatic preview publication ([83cae86](https://github.com/engineersamuel/webskiff/commit/83cae86703c3ffb1d9a86bf9867363d08fc47733))
* verify scoped installation access rather than user permissions ([c2dd0e9](https://github.com/engineersamuel/webskiff/commit/c2dd0e992898820c920731a3368b09d503e0692b))

## 0.1.0-beta.1 - Unreleased

### Browser

- Live current-match page search with `/`, Enter to accept, and `n`/`N` to move.
- Familiar browser shortcuts alongside Vim-style navigation.
- Editable prompts, page focus navigation, and safe quit behavior.
- Startup TOML keyboard configuration and help generated from actual bindings.
- Local-file addressing and explicit command-line validation.
- `--version` includes Webskiff, Zebkit, Chromium, target, and build commit.
- Page-derived terminal display text cannot emit raw control characters.

### Release preparation

- Window wake logo variants, an icon, and a social card.
- Native macOS ARM64 and experimental Linux x86-64 build workflows.
- Pinned, checksum-verified engine downloads and relocatable packages.
- Packaged-engine smoke tests, dependency inventories, and SHA-256 sidecars.
- Protected macOS signing/notarization and draft-only release automation.
- Homebrew formula generation from exact release artifacts.

### Known limitations

- A source-built Homebrew tap is available for macOS on Apple Silicon.
- Webskiff's own code is MIT-licensed. Public prebuilt binary downloads still need
  complete upstream notices and the separate platform signing checks.
- Real-terminal display performance and sustained video/audio sync remain open.
- Linux is experimental; no macOS Intel, Linux ARM64, or Windows support claim.
- Corporate sign-in and accessibility support have not been accepted.
