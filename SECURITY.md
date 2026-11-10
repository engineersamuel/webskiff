# Security

Webskiff is an experimental browser, not an approved replacement for a managed
desktop browser. Do not use it for sensitive accounts or company data until the
relevant review and acceptance checks are complete.

## Reporting

Do not post exploit details, credentials, profile files, or private browsing
history in public issues. Once the repository is hosted, maintainers must enable
GitHub private vulnerability reporting before public release. No private
reporting endpoint is configured in this local checkout yet.

Normal bug reports should contain only a minimal public/local reproduction,
the Webskiff version, OS, terminal, and graphics mode.

## Current review status

A bounded source review found page-controlled terminal control injection in
dialog text. Display clipping now replaces control characters before output.
The regression covers OSC, CSI, C1, and line controls in labels and prompt input.
This fix is not evidence that the whole browser is secure.

Before production use, verify actual Chromium renderer sandbox activation,
local-file origin isolation, clipboard user-activation requirements, certificate
failure behavior, permission denial, popups, and external-protocol handling.
The available upstream contracts are not substitutes for runtime tests.

The profile uses a file-based cookie key, not Keychain, and currently lives
under `~/.cache/zweb/profile`. Trace logging can record typed input; never trace
password entry or share unreviewed logs. See [data and diagnostics](docs/usage.md).

Do not use `ZWEB_ARGS` to disable sandboxing or certificate verification.
Chromium/Zebkit security updates require a new tested Webskiff package; updating
the Rust binary alone does not update the bundled engine.
