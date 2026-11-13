# Changelog

## Unreleased

### Bug Fixes

- Preserve forward jumplist entries when a background tab finishes navigating.
- Preserve jumplist history through rapid jumps and redirected restores; tab
  switches record the pending destination while its restore finishes in the background.

## [0.6.0](https://github.com/engineersamuel/webskiff/compare/v0.5.0...v0.6.0) (2026-11-14)


### Features

* add g u / g U address navigation and g [ / g ] history keys ([#37](https://github.com/engineersamuel/webskiff/issues/37)) ([0eb2a82](https://github.com/engineersamuel/webskiff/commit/0eb2a8218a60adb82dc3f32153c227395476ae12))

## [0.5.0](https://github.com/engineersamuel/webskiff/compare/v0.4.0...v0.5.0) (2026-11-14)


### Features

* text-filtered hints on g f ([#35](https://github.com/engineersamuel/webskiff/issues/35)) ([52c3a86](https://github.com/engineersamuel/webskiff/commit/52c3a8601f19d73962df63fa74d7b3e1c2f61458))

## [0.4.0](https://github.com/engineersamuel/webskiff/compare/v0.3.0...v0.4.0) (2026-11-14)


### Features

* reopen closed tab and alternate tab ([#33](https://github.com/engineersamuel/webskiff/issues/33)) ([fdb8f0f](https://github.com/engineersamuel/webskiff/commit/fdb8f0f4fa2d7e24f246c3748a6d01064db39ca5))

## [0.3.0](https://github.com/engineersamuel/webskiff/compare/v0.2.0...v0.3.0) (2026-11-14)


### Features

* caret visual mode with yank ([#31](https://github.com/engineersamuel/webskiff/issues/31)) ([6b565b5](https://github.com/engineersamuel/webskiff/commit/6b565b59febf96e30f5a616ba739865e2b882b6d))

## [0.2.0](https://github.com/engineersamuel/webskiff/compare/v0.1.0...v0.2.0) (2026-11-14)


### Features

* edit the focused text field in $VISUAL/$EDITOR with g e ([#28](https://github.com/engineersamuel/webskiff/issues/28)) ([7ba8255](https://github.com/engineersamuel/webskiff/commit/7ba82554e7ec374daac8facd4d1c4b26274beade))

## [0.1.0](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.5...v0.1.0) (2026-11-13)


### Features

* page history with frecency completion in the Open prompts ([#26](https://github.com/engineersamuel/webskiff/issues/26)) ([6325a83](https://github.com/engineersamuel/webskiff/commit/6325a83c6a250e9e54a56656cbcc2ba15a9ee08a))

## [0.1.0-beta.5](https://github.com/engineersamuel/webskiff/compare/v0.1.0-beta.4...v0.1.0-beta.5) (2026-11-12)


### Bug Fixes

* retain the preparation generation across async replies ([0fa1347](https://github.com/engineersamuel/webskiff/commit/0fa13470ea9e80f519a2a9a625d735fe800cbaeb))


### Performance Improvements

* force worker glyph rasterization before preparing hints ([ecfeb33](https://github.com/engineersamuel/webskiff/commit/ecfeb33d1d19492c932ed4f40a197c26effc211a))
* make hop hints (f) show in under 10 ms ([#22](https://github.com/engineersamuel/webskiff/issues/22)) ([ed695ab](https://github.com/engineersamuel/webskiff/commit/ed695ab461747d5b786387cce52da6efafbed562))
* warm hint fonts off the page script queue ([9cee08f](https://github.com/engineersamuel/webskiff/commit/9cee08f3da1635ee00f62f11d1b095f5da8f2e66))
* yield between background hint atlas operations ([ee7d9a8](https://github.com/engineersamuel/webskiff/commit/ee7d9a8bbbabcb6f76246a006bdee7cd2fb3bd97))

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
