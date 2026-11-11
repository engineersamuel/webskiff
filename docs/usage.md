# Usage and troubleshooting

```sh
webskiff [--gfx kitty|shm|blocks] [--scale 1|2] [--placeholders] [url-or-search]
webskiff --version
webskiff --selftest
```

For a source build, replace `webskiff` with `target/release/webskiff`.
Do not run an old `target/release/zweb`: Cargo leaves renamed binaries behind.
The old `CMD` prompt is a sign that an outdated executable is running.

## Navigation

`o` opens an address or web search; `O` edits the current address. `t` opens
a new tab. Addresses also accept existing local files, relative to the working
directory, absolute paths, and `file://` URLs. Quote paths containing spaces.
Missing explicit paths report an error.

Arrow keys, Page Up/Down, Home/End, Alt-Left/Right, and F5 provide standard
scrolling, history, and reload alongside the Vim keys.

`/` highlights the current match while typing, case-insensitively with wrapping.
Enter accepts; `n`/`N` move next/previous; Escape restores the original selection
and scroll when cancelling. Only the current match is highlighted.
Ctrl-F leaves its prompt open; Enter/Shift-Enter moves next/previous.

Prompts support arrows, Home/End, Delete/Backspace, paste at the cursor,
Ctrl-U to clear, and Ctrl-W to delete a word. Focused page text fields receive
typing; Escape returns to browser shortcuts. Pages that focus an input on load
may therefore require Escape before browser shortcuts work.

Help (`?`/F1) is terminal text. Scroll with arrows or `j`/`k`, page with
Page Up/Down or Space, and jump with Home/End. Escape or `?` closes it.
Prompt editing takes priority over other global shortcuts; global quit works
in every mode.

## Graphics

Automatic detection selects shared-memory Kitty graphics when supported, direct
Kitty transfers otherwise, then half-block cells as a last resort.
Use `--gfx kitty` across SSH or when shared memory is not accessible to the
terminal. Shared memory is local-only.

`--scale 2` gives sharper Retina output but can require four times the pixels.
`I` or `--placeholders` replaces images with flat boxes for lower transfer cost.

## Linux sandbox startup

The experimental Linux build needs the runtime libraries listed in
[the CI workflow](../.github/workflows/ci.yml). Chromium also needs working
unprivileged user namespaces or a correctly installed sandbox helper.
Ubuntu 23.10 and later can restrict user namespaces through AppArmor.

If startup fails, check `~/.cache/zweb/zweb.log` for the engine's diagnosis.
Ask your system administrator for an approved AppArmor profile or sandbox-helper
installation. Do not run Webskiff as root or use `--no-sandbox`.

CI allows user namespaces on disposable hosted Linux runners. That setting is
not a recommendation to change your normal machine's security policy.

## Data and diagnostics

Legacy paths are intentionally unchanged:

| Purpose | Path |
|---|---|
| Keyboard configuration | `~/.config/webskiff/config.toml` |
| Chromium profile, cookies, storage | `~/.cache/zweb/profile` |
| Engine log | `~/.cache/zweb/zweb.log` |

The profile is durable data despite its current cache location. Do not clear the
whole cache directory to troubleshoot unless you intend to lose sign-ins.
The cookie key is stored in the profile, **not macOS Keychain**.
Do not assume protection equivalent to a managed desktop browser.

For an isolated test (Fish), create a disposable HOME:

```fish
set test_home (mktemp -d)
env HOME="$test_home" target/release/webskiff https://example.com
```

Keep the path printed by `echo "$test_home"` so you can remove only that
disposable directory later. This does not change your normal browser profile.

`ZWEB_TRACE=1` records timing stages; `ZWEB_DEBUG=1` records frames and damage.
Tracing also records input events: **never use it while entering passwords or
sensitive data, and never attach an unreviewed trace to an issue**.
Logs may include private URLs and do not yet have automatic rotation.

Developer switches: `ZWEB_NOPREDICT=1`, `ZWEB_NOPROGRESSIVE=1`,
`ZWEB_FPS=n` (capture default 240), and `ZWEB_ARGS="..."` for extra Chromium
switches. These are experimental and are not a supported security policy.
Do not disable the Chromium sandbox or certificate checks to make a site work.

## Upgrades and removal

Keep the executable and its `zebkit/` directory together. Replacing only the
executable can break startup or mix engine versions. Quit before upgrading.

For Homebrew installations, use:

```sh
brew upgrade engineersamuel/tap/webskiff
brew uninstall engineersamuel/tap/webskiff
```

Homebrew manages the installed program and engine together.
Uninstalling the program must not remove your configuration or profile.
Profile deletion remains an explicit, separate user action.

`zebkit-probe text|png|kitty|scroll <url>` is a developer timing tool, not the
browser. It is not included in user release packages.
