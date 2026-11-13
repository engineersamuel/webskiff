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

**Experimental preview.** Check your installed version with `webskiff --version`.
Homebrew builds Webskiff from source; there is no prebuilt Webskiff release. Do not use this build
for sensitive accounts or treat it as an approved company browser.

[Get started](#get-started) · [Watch Webskiff](#watch-webskiff) · [Keyboard guide](#keyboard-guide) ·
[Configuration](#configuration) · [Support](#support) ·
[Build and test](https://github.com/engineersamuel/webskiff/actions/workflows/ci.yml) ·
[Release process](docs/releasing.md)

## Get started

Requires **macOS 13+ on Apple Silicon**. Intel Macs and Linux are not supported
by the Homebrew formula.
For full-page graphics, use a terminal that supports the Kitty graphics
protocol, for example [Kitty](https://sw.kovidgoyal.net/kitty/),
[Ghostty](https://ghostty.org/), or [WezTerm](https://wezterm.org/).
Webskiff does not support Sixel. A Sixel-only terminal gets the coarse
half-block fallback, not full-page graphics.

### First install

Add the project tap explicitly, then install Webskiff:

```sh
brew tap engineersamuel/tap
brew install engineersamuel/tap/webskiff
webskiff --version
```

Open a page:

```sh
webskiff https://www.google.com
```

### Upgrade an existing installation

```sh
brew update
brew upgrade engineersamuel/tap/webskiff
webskiff --version
```

`brew update` refreshes tap metadata; `brew upgrade` updates an installed
Webskiff. On a new Mac, use the **First install** steps instead.

The [project tap](https://github.com/engineersamuel/homebrew-tap) builds the Rust
browser locally and installs its checksum-pinned Zebkit/Chromium engine.
Homebrew installs Rust as a build dependency. The engine is prebuilt upstream;
this is not a source build of Chromium. No paid Apple Developer membership or
Webskiff signing certificate is needed.

This formula is not in Homebrew's main repository.

## Watch Webskiff

This real terminal recording opens Google, searches for `webskiff zebkit`,
then uses `Esc`, `f`, and hint `e` to open Zebkit's GitHub page.
Search results and hint letters can change.

<details open>
<summary>Google search and keyboard-hint GIF (23 seconds; click to hide)</summary>

![Webskiff opening Google, searching for webskiff zebkit, showing link hints, and opening Zebkit on GitHub](assets/google-search.gif)

Close this section to hide the animation. The final frame is held briefly.

</details>

## Keyboard guide

Press **`?`** or **`F1`** for the full list, generated from your configuration.

| Task | Vim-style | Familiar shortcut |
|---|---|---|
| Open address or web search | `o` | `Ctrl-L` |
| Find page text as you type | `/` | `Ctrl-F` |
| Next / previous match | `n` / `N` | `F3` / `Shift-F3` |
| New tab | `t` | `Ctrl-T` |
| Close tab | `x` | `Ctrl-W` |
| Reopen closed tab (`reopen_tab`) | `X` | `Ctrl-Shift-T` |
| Next / previous browser tab | `]` / `[` | `Ctrl-Tab` / `Ctrl-Shift-Tab` |
| Previous active tab (`alternate_tab`) | `^` | — |
| Follow a link with hints | `f` | Mouse click |
| Filtered hints: letters filter by link text, digits pick fixed labels, Enter picks the first match | `g f` | Mouse click |
| Select and copy page text | `v`, move, `y` | Mouse select is not possible |
| Up one address level / address root | `g u` / `g U` | — |
| Move through page controls | `Tab` / `Shift-Tab` | `Tab` / `Shift-Tab` |
| Leave page typing mode | `Esc` | `Esc` |
| Show keyboard help | `?` | `F1` |
| Quit | `Ctrl-Q` | `Ctrl-Q` |

With `/`, **Enter accepts the current match without skipping it**.
Then use `n` or `N`. Escape cancels an unaccepted search.
In page text fields, typing goes to the page; `?` remains a normal character.
Some terminals intercept browser-style shortcuts. The Vim alternatives remain
available. Bare `q` shows a quit reminder rather than closing your session.
With `v`, selection starts at the find match, else at the first visible text.
Move with `h`/`l`, `j`/`k`, `w`/`b`, `0`/`$`. `y` copies through OSC 52;
selections over 100,000 base64 bytes are refused. `Esc` cancels.

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
| macOS 13+, Apple Silicon | Native CI and fresh macOS 15.8.2 Homebrew source-install, engine, keyboard, and uninstall tests pass; real-terminal acceptance remains open |
| Linux x86-64, Ubuntu 24.04 runner | Native CI build, keyboard, and relocated-engine checks pass; experimental, with [sandbox setup requirements](docs/usage.md#linux-sandbox-startup) |
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
- Downloads, uploads, and bookmarks are not implemented as supported terminal
  workflows. Page history is kept for address completion only: Tab in the `o`
  and `t` prompts. There is no history list view. See
  [usage](docs/usage.md#data-and-diagnostics) for its file.
- SSO/MFA, corporate proxies/certificates, DRM, screen readers, and IME behavior
  have not been accepted for production use.
- Pixels in a terminal are not a screen-reader interface. No WCAG conformance
  claim is made.

For startup errors, graphics options, profile paths, and logs, see the
[usage and troubleshooting guide](docs/usage.md). Report bugs with the OS,
terminal, `webskiff --version`, and a minimal public reproduction.
Do not attach cookies, credentials, private URLs, or raw profile contents.

## Credits and license

Built on [Zebkit](https://github.com/ikey4u/zebkit). Credit to **ikey4u and the
Zebkit contributors** for the Chromium embedding engine that makes Webskiff possible.

Webskiff's own code is licensed under [MIT](LICENSE). Zebkit, Chromium, and other
dependencies retain their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.md).

## Contributing

To build from this checkout or run the development checks, see
[CONTRIBUTING.md](CONTRIBUTING.md).

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Changelog](CHANGELOG.md) · [Third-party notices](THIRD_PARTY_NOTICES.md)
