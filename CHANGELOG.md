# Changelog

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

- No public release or Homebrew tap has been published.
- Project licensing, upstream redistribution notices, signing credentials,
  GitHub runner execution, and fresh-machine installation remain release gates.
- Real-terminal display performance and sustained video/audio sync remain open.
- Linux is experimental; no macOS Intel, Linux ARM64, or Windows support claim.
- Corporate sign-in and accessibility support have not been accepted.
