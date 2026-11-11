# Changelog

## [0.1.0-beta.4](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.3...v0.1.0-beta.4) (2026-11-11)


### Features

* mark find matches and open the link at the current match ([#17](https://github.com/engineersamuel/webskiff/issues/17)) ([8834f13](https://github.com/engineersamuel/webskiff/commit/8834f133cf1f30b4bc3d8d82174fe78b47b7286e))


### Bug Fixes

* publish source previews through draft-aware release lookup ([#12](https://github.com/engineersamuel/webskiff/issues/12)) ([f58dcc2](https://github.com/engineersamuel/webskiff/commit/f58dcc2d4a7cc6ad2e04354fb0dd2fbe4d7dfe06))

## [0.1.0-beta.3](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.2...v0.1.0-beta.3) (2026-11-11)


### Bug Fixes

* stop Chromium killing pages that call setAppBadge ([#16](https://github.com/engineersamuel/webskiff/issues/16)) ([714187f](https://github.com/engineersamuel/webskiff/commit/714187fb231379592c90053d4ceb83ef97a05a12))

## [0.1.0-beta.2](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.1...v0.1.0-beta.2) (2026-11-11)


### Features

* automate beta release PRs and tested Homebrew updates ([#8](https://github.com/engineersamuel/webskiff/issues/8)) ([a433a29](https://github.com/engineersamuel/webskiff/commit/a433a292b4264f0eef4ba18ebddff2726086e763))

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
