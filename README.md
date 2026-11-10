<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/webskiff-dark.svg">
    <img src="assets/webskiff-light.svg" alt="Webskiff: a skiff sailing inside a terminal window" width="600" height="220">
  </picture>
</p>

<p align="center"><strong>A keyboard-first Chromium browser for your terminal.</strong></p>
<p align="center">Vim keys when you want them. Familiar browser shortcuts when you do not.</p>

Webskiff renders real web pages inside a graphics-capable terminal using
[Zebkit](https://github.com/ikey4u/zebkit). JavaScript, forms, images, and tabs
run in Chromium, not a text-only approximation.

**Preview in preparation.** The current version is `0.1.0-beta.1`.
There is no published binary release or Homebrew tap yet. Do not use this build
for sensitive accounts or treat it as an approved company browser.
Source repository: [engineersamuel/webskiff](https://github.com/engineersamuel/webskiff)
(currently private).

[Get started](#get-started) · [Keyboard guide](#keyboard-guide) ·
[Configuration](#configuration) · [Support](#support) ·
[Release process](docs/releasing.md)

## See the controls

[Download a real terminal recording](assets/demo.cast.gz) of scrolling, live
search, match navigation, tabs, hints, and help on a local fixture.
It contains Kitty graphics: replay with asciinema in a compatible 120x40 terminal,
not a text-only web player. See [recording instructions](docs/releasing.md#demo-and-announcements).

## Get started

The locally tested build is **macOS 13+ on Apple Silicon**. Install Rust using
the version in `rust-toolchain.toml`, plus [uv](https://docs.astral.sh/uv/),
`curl`, and `xz`. From this repository:

```sh
uv run --no-project python -I scripts/release.py fetch-engine
```

Then build and run (Fish syntax):

```fish
env ZEBKIT_DIR="$PWD/engine/verified/zebkit-0.4.0-aarch64-apple-darwin" cargo build --release --locked --bin webskiff
target/release/webskiff https://example.com
```

For Bash or Zsh, the same `env ... cargo build ...` command works.
Open a URL, a quoted search, or a local file:

```sh
target/release/webskiff 'terminal graphics protocol'
target/release/webskiff ./README.md
target/release/webskiff --version
```

Local files are rendered according to their content; Markdown is not converted
to HTML. Use an HTML file for a local web page.

### Homebrew and binary packages

The release workflow prepares versioned packages containing **both the browser
and its engine**. It also generates a checksum-pinned `webskiff.rb` formula for a
project-owned tap. Installation will be advertised here only after a release,
tap, signing, and fresh-machine installation have been verified.

Maintainers: see [release and Homebrew setup](docs/releasing.md).

## Keyboard guide

Press **`?`** or **`F1`** for the full list, generated from your configuration.

| Task | Vim-style | Familiar shortcut |
|---|---|---|
| Open address or web search | `o` | `Ctrl-L` |
| Find page text as you type | `/` | `Ctrl-F` |
| Next / previous match | `n` / `N` | `F3` / `Shift-F3` |
| New tab | `t` | `Ctrl-T` |
| Close tab | `x` | `Ctrl-W` |
| Next / previous browser tab | `]` / `[` | `Ctrl-Tab` / `Ctrl-Shift-Tab` |
| Follow a link with hints | `f` | Mouse click |
| Move through page controls | `Tab` / `Shift-Tab` | `Tab` / `Shift-Tab` |
| Leave page typing mode | `Esc` | `Esc` |
| Show keyboard help | `?` | `F1` |
| Quit | `Ctrl-Q` | `Ctrl-Q` |

With `/`, **Enter accepts the current match without skipping it**.
Then use `n` or `N`. Escape cancels an unaccepted search.
In page text fields, typing goes to the page; `?` remains a normal character.
Some terminals intercept browser-style shortcuts. The Vim alternatives remain
available. Bare `q` shows a quit reminder rather than closing your session.

## Configuration

Bindings load at startup from `~/.config/webskiff/config.toml`.
[config.example.toml](config.example.toml) contains the embedded defaults.
Override only the actions you want to change:

```toml
[keys.browse]
find = ["/", "s"]

[keys.global]
quit = ["Ctrl-Q"]
```

Use `[]` to disable an action. Invalid or conflicting bindings stop startup with
an error. Restart after editing. Help always shows the configured bindings.

## Support

| System | Status |
|---|---|
| macOS 13+, Apple Silicon | Locally built and tested; real-terminal acceptance remains open |
| Linux x86-64, Ubuntu 24.04 runner | Experimental CI target; not yet verified on a runner |
| macOS Intel / Linux ARM64 | No pinned upstream engine package; not enabled |
| Windows | Not supported by Webskiff's current Unix terminal implementation |

Webskiff probes for Kitty graphics and local shared-memory transfers. Other
terminals use a coarse half-block fallback. Herdr is the primary target;
other terminals and multiplexers need their own acceptance tests.

### Known limits

- Some interactions exceed the **10 ms** target. We do not claim constant
  sub-10-ms latency or parity with a desktop browser.
- Video is experimental. A tested opt-in setting improves local frame delivery
  but increases CPU/traffic risks. Real YouTube presentation and audio/video sync
  remain unverified. See [measurements and video](docs/performance.md).
- Downloads, uploads, bookmarks, and a history list are not implemented as
  supported terminal workflows.
- SSO/MFA, corporate proxies/certificates, DRM, screen readers, and IME behavior
  have not been accepted for production use.
- Pixels in a terminal are not a screen-reader interface. No WCAG conformance
  claim is made.

For startup errors, graphics options, profile paths, and logs, see the
[usage and troubleshooting guide](docs/usage.md). Report bugs with the OS,
terminal, `webskiff --version`, and a minimal public reproduction.
Do not attach cookies, credentials, private URLs, or raw profile contents.

## Development

```fish
env ZEBKIT_DIR="$PWD/engine/verified/zebkit-0.4.0-aarch64-apple-darwin" bash scripts/check.sh
```

CI checks macOS ARM64 and experimental Linux x86-64, then tests an extracted
package away from the build directory. The macOS job also runs the generated
Homebrew formula's installation method in a disposable prefix.
Tags must match the Cargo version.
Public packaging is blocked until licensing and signing prerequisites are met.

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Changelog](CHANGELOG.md) · [Third-party notices](THIRD_PARTY_NOTICES.md)

The Window wake identity is in [assets/](assets/). No logo or splash screen is
drawn during browsing. `zebkit-probe` remains a separate developer timing tool.
