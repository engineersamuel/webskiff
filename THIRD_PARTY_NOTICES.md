# Third-party notices

Webskiff is built on [Zebkit](https://github.com/ikey4u/zebkit), by **ikey4u and
the Zebkit contributors**. Credit to them for the Chromium embedding engine.

Webskiff uses Zebkit 0.4.0 and Chromium. Zebkit's Rust crates declare
`LGPL-2.1-or-later`. Chromium includes code under many other licenses.

- [Pinned engine source](https://github.com/ikey4u/zebkit/tree/v0.4.0)
- [Engine release and platform archives](https://github.com/ikey4u/zebkit/releases/tag/v0.4.0)
- [Chromium licensing](https://www.chromium.org/developers/how-tos/get-the-code/)

Release packages include `dependencies.json`: exact Rust package versions,
declared licenses, repository URLs, and the engine archive hashes. This inventory
is not a substitute for license texts, copyright notices, or source obligations.

Webskiff's own code is licensed under [MIT](LICENSE). That license does not
relicense Zebkit, Chromium, or other third-party code.

## Redistribution approval and notice bundle

The project owner has approved redistribution. The downloaded engine archive
still does not include a complete license-notice bundle. Required license texts
and notices must be supplied under `packaging/third-party/` before public binary
packages can be prepared. Approval does not replace shipping the required notices.
Confirm LGPL source availability and replacement/relinking requirements with the
engine supplier; do not assume a source link alone satisfies all obligations.

The release tool blocks public packaging and production Homebrew formula
generation until `LICENSE`, Cargo license metadata, and the third-party notice
directory are present. Internal CI artifacts are explicitly marked
`public_distribution: false`.
