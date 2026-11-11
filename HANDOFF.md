# Webskiff handoff

Webskiff is a web browser that runs inside a terminal. It uses zebkit (Chromium 157 packaged
as a library) to draw pages off-screen. It sends the pixels to the terminal as
Kitty graphics, and sends the terminal's keys and mouse to the page.
This folder is a prototype and is separate from textward.

**Read `AGENTS.md` first. Speed comes first: every interaction under 10 ms,
and no change may make anything slower without the user's approval.**

## Build, run, test

    ZEBKIT_DIR=$PWD/engine/zebkit-0.4.0-aarch64-apple-darwin cargo build --release
    target/release/webskiff [url-or-search] [--gfx kitty|shm|blocks] [--scale 1|2] [--placeholders]
    target/release/webskiff --selftest                       # must print ALL PASSED (15 checks)
    ZEBKIT_DIR=... cargo test --release --bin webskiff       # address and render unit tests
    uv run --no-project python -I bench/keyboard.py target/release/webskiff --check-only
    uv run --no-project python -I bench/keyboard.py target/release/webskiff --config-check
    uv run --no-project python -I bench/keyboard.py target/release/webskiff --enhanced-check
    uv run --no-project python -I bench/keyboard.py target/release/webskiff
    cd bench && uv run --no-project python -I inter.py ../target/release/webskiff

- The Cargo package and default browser binary are named `webskiff`. The timing tool
  remains `zebkit-probe`. Browser source paths, runtime labels, `ZWEB_*` switches,
  logs, and profile paths retain their legacy `zweb` names.
- Existing local files open from CLI arguments and the `o` / `t` prompts.
  Relative paths use the working directory; spaces and URL characters are escaped.
  Missing explicit paths report an error instead of being treated as hostnames.

- Engine: the prebuilt zebkit v0.4.0 for macOS arm64, unpacked in `engine/` (git-ignored).
  The download is cached in `~/.cache/zebkit-dl`.
- Log: `~/.cache/zweb/zweb.log`. `ZWEB_TRACE=1` logs the time of each stage of an interaction.
- Profile: `~/.cache/zweb/profile`. The cookie key is kept in a file there (`--zebkit-cookie-storage=file`),
  not in the macOS Keychain, because the Keychain prompt blocked every page load.
- Debug switches: `ZWEB_NOPREDICT`, `ZWEB_NOPROGRESSIVE`, `ZWEB_FPS=n` (default 240),
  `ZWEB_ARGS="..."` (adds Chromium switches), `ZWEB_DEBUG` (logs every frame).
- The user runs zweb in **herdr**, which is built on Ghostty's engine and does pass Kitty images through.

## Code map (`src/bin/zweb/`)

| File | What |
|---|---|
| `main.rs` | Terminal setup, graphics detection, Chromium switches, threads (engine on main, input thread, render thread) |
| `app.rs` | `App`: tabs, modes (Browse/Insert/Hint), commands and keys, prompt, status bar, scroll prediction requests, placeholders (`I`) |
| `render.rs` | Render thread: page split into tiles (cell row × ~16-col band) keyed by xxh3 hash, reuse via Kitty `a=p`, scroll prediction + "tape" of seen rows, progressive photo tiles, parallel zlib encode |
| `hints.js` | hop.nvim-style hints drawn inside the page (dim overlay, code replaces a target's first characters); read ahead when the page is still |
| `keys.rs` | crossterm key → DOM key events |
| `config.rs` | Startup-only TOML configuration, modifier-aware per-mode bindings, and generated help |
| `input.rs` | UTF-8 prompt cursor editing |
| `find.js` | Native current-match live search, with a saved origin for query edits and cancellation |
| `selftest.rs` | Headless checks of typing, paste, hints, `<select>` popups, `window.open`, dialogs |
| `trace.rs` | `ZWEB_TRACE` timing |
| `src/main.rs` | `zebkit-probe`, the original timing tool (text, png, kitty, scroll) |

`bench/keyboard.py --check-only` drives the real browser in a PTY against a local page:
live search, match navigation, prompt editing, page focus, standard shortcuts,
safe quit, and the help overlay. `--config-check` tests remapped keys and help.
Without flags it reports p50/p90 latency and bytes per action. Each run uses a
temporary HOME; performance runs use the same page without the title-change
listeners used by the assertions. Run both binaries without checks when comparing
performance, so graphics image IDs start at the same value.

Keyboard defaults are compiled from `config.example.toml`; user overrides load
from `~/.config/webskiff/config.toml` once before terminal or engine startup.
The config is not watched. Every app-owned shortcut, including prompt, hint, and
help controls, is configurable. Website keyboard behavior is still handled by
Chromium. Global quit takes priority in every mode; prompt editing takes priority
over other global shortcuts (so Ctrl-W deletes a word in prompts).

## Where speed stands (simulated terminal, 120×40; p50 / p90)

| Action | Time |
|---|---|
| j / k / wheel | ~1 / ~1.5 ms (shown at once from tiles the terminal already holds) |
| b (page up) | ~6–7 / ~7.5 ms |
| Space (page down) | ~8 / up to ~60 ms (new content must wait for Chrome to draw it) |
| Typing a character | ~4–6 / ~7 ms |
| f (hop hints) | ~10–15 / ~20–29 ms (dimming changes the whole screen, so Chrome redraws it all) |
| Switching tabs | ~3 / ~4 ms |

Key speed settings: `--disable-smooth-scrolling --disable-gpu-vsync --disable-gpu-compositing --hide-scrollbars`.
`--disable-frame-rate-limit` was rejected: it used 240% CPU on idle pages with an animation.

### Keyboard changes: validation and performance review (2026-11-10)

Implemented live current-match find, modifier-aware standard shortcuts alongside
Vim keys, editable prompts, page focus with Tab, safe bare `q`, and a scrollable
terminal-text help overlay generated from the startup configuration. Defaults
come from `config.example.toml`; a missing user config keeps the defaults, while
invalid configuration stops startup before raw terminal mode.

Functional checks pass: 12 Rust unit tests, real-PTY keyboard checks, real-PTY
custom-config checks, and `webskiff --selftest` (`ALL PASSED`). New Rust modules
pass rustfmt. Strict Clippy passes with two existing findings excluded:
`int_plus_one` in `clip` and `items_after_test_module` in `app.rs`.
Repository-wide formatting already differs from rustfmt; no broad formatting
change was applied.

Measured with `bench/keyboard.py`, fresh temporary profiles, a local static page,
Kitty direct output, and a 120x40 PTY. Both binaries ran without assertion
instrumentation. Latency is key to first page-image output, except prompt/help
rows, which measure their terminal text. Byte counts include status updates.
These are small, local samples, not a guarantee for all pages or terminals.

| Action | Before p50/p90 (ms) | After p50/p90 (ms) | Bytes before p50/p90 | Bytes after p50/p90 |
|---|---|---|---|---|
| Scroll down | 0.20 / 0.27 | 0.22 / 0.30 | 9340 / 9340 | 9340 / 9340 |
| Scroll up | 3.80 / 39.52 | 3.38 / 4.40 | 9022 / 9340 | 9022 / 9340 |
| Open help (text) | 0.18 / 0.23 | 0.18 / 0.19 | 2204 / 2522 | 2425 / 2425 |
| Close help | 2.91 / 4.43 | 3.02 / 3.34 | 113627 / 113815 | 113497 / 113627 |
| Page typing | 3.21 / 3.91 | 2.31 / 5.89 | 2088 / 2558 | 2112 / 2558 |
| Switch distinct tabs | 1.10 / 2.63 | 1.09 / 2.08 | 1548 / 1548 | 1548 / 1548 |
| Open prompt (text) | 1.21 / 5.35 | 0.39 / 0.81 | 320 / 640 | 320 / 640 |
| Type seven prompt characters (text) | 0.52 / 2.07 | 0.42 / 0.55 | 2240 / 2240 | 2240 / 2240 |
| Type one find character | No page update | 2.87 / 6.63 | 320 / 320 | 1081 / 3556 |
| Accept find (text) | 0.72 / 3.84 | 0.49 / 0.94 | 1308 / 3690 | 636 / 636 |
| Next match | 3.34 / 5.24 | 4.11 / 86.87 | 1397 / 4032 | 1079 / 4032 |
| Previous match | 3.14 / 4.38 | 3.82 / 5.92 | 1308 / 1397 | 1130 / 1397 |

**Performance acceptance is pending.** Live find necessarily sends page updates
that did not occur while typing before; current-match native selection is used
instead of the more expensive all-match highlighting. Config-generated help
also sends 221 more median bytes in this run. Earlier baseline runs also showed
approximately 85 ms native-find frames, so the occasional long frame is not
unique to this change. The under-10-ms target is not met for every interaction.
The user was asked to approve the measured output cost but was unavailable.
Changes have since been committed; performance/output approval remains open.

### Reported missing highlight and raw input (2026-11-10)

The reported screenshot showed `CMD`, not the new `FIND` label. The running
process was `target/release/zweb`, dated November 9; the updated executable was
`target/release/webskiff`, dated November 10. The live-find regression check fails
against that old `zweb` binary and passes against `webskiff`. Cargo leaves the
old executable behind when a binary is renamed; restart using `webskiff`.

An additional PTY test negotiates enhanced keyboard input, types `Browser` one
character at a time, verifies selected text and next/previous positions, sends
enhanced Ctrl-C/Escape and the screenshot's mouse-report sequences, and checks
raw-mode flags and their restoration at exit. It passes against `webskiff`;
raw-code echo was not reproduced in that build. The user's running process
was not stopped or replaced. No app input or rendering code changed for this
investigation.

## Video playback investigation (2026-11-10)

The user reported choppy YouTube video. `bench/video.py` reproduces choppy
playback without network access: ffmpeg generates a 960x540 VP8 video, then
the actual browser plays it in a 120x40 PTY (1920x1280 terminal pixels).
Shared-memory mode uses the existing automatic scale 2. A simulated terminal
reads and unlinks each shared-memory transfer. The harness asserts playback
starts, then optionally checks delivery rate, p90 cadence, and reported
video drops. It measures the browser's output, not Herdr's display.

The red-capable command was run and failed against the current executable:

```sh
uv run --no-project python -I bench/video.py target/release/webskiff --check
```

It delivered 14.73 fps from a 30 fps source, with 121 reported drops in the
sample and 80.15/85.88 ms frame gaps (p50/p90). Encoding took only
1.61/1.95 ms. Ranked hypotheses were the 240 fps capture setting, disabled GPU
compositing, then transfer cost. Tests at capture rates of 30 and 60 still
delivered about 14-15 fps. Disabling background throttling also failed.
Direct Kitty transport still dropped frames (17.61 fps, 99 reported drops).

Adding Chromium's `--disable-frame-rate-limit` through the existing `ZWEB_ARGS`
resolved the local delivery fault without changing the executable:

| Source / transport | Delivered fps, before / after | Frame gap p50/p90 ms, before / after | Encode p50/p90 ms, before / after | Terminal KiB/s, before / after |
|---|---|---|---|---|
| 30 fps / shm | 14.73 / 29.99 | 80.15/85.88 / 33.26/34.32 | 1.61/1.95 / 1.51/1.70 | 422.06 / 862.84 |
| 60 fps / shm | 23.73 / 60.00 | 57.69/77.43 / 16.77/17.62 | 1.54/1.91 / 1.53/1.78 | 732.09 / 1831.33 |
| 30 fps / direct | 17.61 / 29.99 | 63.88/83.97 / 33.19/34.24 | 3.21/3.99 / 1.92/2.20 | 2461.82 / 4179.39 |

All three option-enabled cases passed the delivery/drop checks. A repeat of
30 fps shm delivered 29.97 fps with 33.25/34.19 ms gaps and no reported drops.
The 60 fps run also reported zero drops, though its JS video-frame callbacks
did not run once per delivered frame; those counters are reported separately.
Total video-frame counts include dropped frames and are not decode counts.

The flag increases output by delivering frames previously missed. Shared
pixels rose from 51.18 to 103.12 MiB/s at 30 fps, and 85.97 to 213.95 MiB/s
at 60 fps. It affects all animations, not just video. Approval to enable it
by default was requested, but the user was unavailable. **No runtime defaults,
capture rates, GPU switches, rendering code, user config, or running browser
were changed.** The README documents an opt-in launch command.

Keyboard comparisons with/without this option used `bench/keyboard.py`:

| Action | Before p50/p90 ms | Option p50/p90 ms |
|---|---|---|
| Scroll down | 0.22/0.28 | 0.27/0.31 |
| Scroll up | 3.64/17.05 | 3.78/4.20 |
| Open help (text) | 0.19/0.26 | 0.23/0.26 |
| Close help | 3.17/3.43 | 3.06/3.87 |
| Page typing | 4.58/6.68 | 3.98/6.08 |
| Tab switch | 0.86/1.41 | 1.34/3.56 |
| Open prompt (text) | 0.62/0.68 | 0.69/1.28 |
| Prompt typing (text) | 0.66/1.22 | 0.67/3.08 |
| Find typing | 4.17/5.80 | 3.28/5.34 |
| Accept find (text) | 0.70/0.91 | 0.67/1.13 |
| Next match | 3.82/6.71 | 4.94/8.32 |
| Previous match | 4.26/90.21 | 2.88/4.56 |

Small samples show both increases and decreases, not proof of no regression.
Most per-action byte counts were unchanged; video output clearly increased.
Raw keyboard results are in the session's `files/video-keyboard-before.json`
and `files/video-keyboard-after.json`. The sub-10-ms target is not claimed.

**Still needs an actual Herdr/YouTube check:** terminal image decoding/upload,
display cadence, buffering, audio/video sync, and sustained playback under
real page load. Local video delivery is fixed by the opt-in flag; the user's
full YouTube symptom is not yet verified.

Final validation passed: video delivery/cadence/drop assertions at 30 and
60 fps over shm, 30 fps over direct Kitty, the keyboard behavior checks with
the option enabled, Python syntax, and `git diff --check`. The engine self-test
printed `ALL PASSED`. It also logged macOS `CVDisplayLinkCreateWithCGDisplay`
errors (`CVReturn: -6670`); the relation of these display-clock warnings to
the limiter fault is not established. No Rust runtime code changed, so no
rebuild was needed.

## Release preparation: 0.1.0-beta.1

Implemented the selected Window wake identity, branded README, SVG logo variants,
1200x630 social PNG, real local-fixture terminal recording, and unpublished X and
internal announcement drafts. Usage, performance, security, contribution,
third-party notice, and release guides describe the preview limits explicitly.

Added validated CLI help/version, Cargo-sourced version/tag checks, exact engine
archive hashes, dependency inventories, relocatable internal packages, signing/
notarization preparation, native macOS ARM64 and experimental Linux x86-64 CI,
and protected draft-only releases. The user selected MIT for Webskiff's own code;
`LICENSE` and Cargo SPDX metadata are in place, with explicit Zebkit credit in
the README and third-party notices. Public packaging still requires reviewed
third-party notices. These file checks are not legal
clearance. The selected source remote is the public
`https://github.com/engineersamuel/webskiff.git`, with source pushed to `main`.
Native macOS CI passed the build, keyboard, relocated-engine, and Homebrew smoke
tests. Native Linux build, keyboard, and relocated-engine checks also pass.
The [verified native run](https://github.com/engineersamuel/webskiff/actions/runs/42887920588)
tested source commit `1aff5b2`. The newer source-tap path is described below;
there is no published prebuilt Webskiff release.

The generated Homebrew formula keeps the binary and engine under libexec.
A plain bin symlink failed real engine startup; the standard exec wrapper passed.
`scripts/test-brew.rb` runs the actual formula install method in a disposable
prefix, then checks version and engine startup through that installed wrapper.
This is not a full tap download, brew sandbox, upgrade, or uninstall test.

A security review found one medium-severity terminal-control injection through
page dialog strings. Display clipping now replaces control characters. The
regression was observed failing with sanitization disabled and passing after
restoration. Upstream sandbox, local-file origin isolation, and clipboard
activation guarantees remain unverified; they were not confirmed vulnerabilities.

Local checks passed: 15 Rust tests, all-target Clippy (the existing
`items_after_test_module` exception only), release build, 11 release-tool tests,
three keyboard/terminal-restoration checks, relocated packaged engine self-test,
and isolated Homebrew formula installation/self-test. Workflow syntax, shell
lint, brand asset validation, local documentation links, and recording privacy
checks passed. The recording contains 496 output events over 18.6 seconds.
Linux all-target source type-checking passed with an isolated target standard
library. Native Linux builds, real-browser keyboard checks, and relocated-engine
self-tests now also pass on Ubuntu 24.04.

The final before/after/repeat p50/p90 timing and byte tables are in
`docs/performance.md`. One previous-match run reached 85.72 ms p90, versus
4.63 ms before and 5.44 ms on repeat. Some timings/output counts increased.
Small samples do not prove a regression or prove its absence. Performance/output
approval remains open. No sub-10-ms or smooth-YouTube release claim is justified.

The owner approved public source and redistribution. Before binary publication,
the required engine/dependency notices must be bundled, and
signing/notarization configured. The protected release environment is in place.
Also required: passing native hosted CI on the release tag,
quarantined fresh-Mac installation, full Homebrew lifecycle tests, and actual
terminal/video/accessibility/company-policy acceptance. Store signing secrets
only in GitHub environment settings, not this repository or chat.
Source has been pushed. No tag, release, or announcement has been published.

### Renderer complexity gate

Split all six flagged functions into private helpers without changing pixel
averaging, tape rules, queue ordering, prediction thresholds, tile/cache state,
or graphics messages. `cccc` passes limits of 15 across the entire renderer,
including the helpers: maximum cyclomatic 9 and cognitive 12.
Seven focused tests bring the Rust total to 22; the project checks, three
keyboard checks, Clippy, release-tool tests, and engine self-test passed.
Full before/after/repeat p50/p90 and byte measurements are in
`docs/performance.md`. Small-sample timing/output variation remains an open
release-acceptance item, including one 84.05 ms previous-match p90.

The user authorized committing/pushing to `engineersamuel/webskiff` and repository
setup, then restored personal-account authentication. Source commit `81c6fd6`
and follow-up fixes are pushed to `main`. Origin uses HTTPS with a repository-local
GH credential helper; no token is stored in this repository. The user explicitly
approved public source and redistribution. A local redacted Gitleaks scan of
all 20 existing commits reported no leaks before the repository was made public.
Merge commits are disabled and rebase merging is enabled. Actions require pinned
SHAs, allow GitHub-owned actions plus `astral-sh/setup-uv`, and default to read-only
workflow tokens without automated review approval.

Public release jobs are explicitly disabled by `PUBLIC_RELEASE_ENABLED=false`.
Public visibility resolved the earlier plan restriction. The `release`
environment requires approval from `engineersamuel`, permits only `v*` tags, and
disables administrator bypass. No signing credentials were read or handled.
The account owner's setup steps are in `docs/releasing.md#one-time-apple-setup`.
They are optional for future binary downloads, not a source-install requirement.
The first hosted macOS run passed. Linux initially refused startup because
Ubuntu blocked Chromium's sandbox namespaces. CI now allows user namespaces on
the disposable hosted Linux runner; Chromium's sandbox was not disabled.
Native Linux build, keyboard, and relocated-engine checks pass with that configuration.
Keyboard failures now print the isolated engine log before deleting the test profile.
A later Linux run exposed the fixed four-second startup assumption; an unchanged
rerun passed. Functional keyboard checks now wait for the fixture title, with a
bounded failure and engine-log output. Per-key assertion windows and benchmark
warm-up behavior are unchanged.

### Source-built Homebrew distribution

The owner chose `brew install engineersamuel/tap/webskiff` without a paid Apple
Developer membership. `scripts/release.py brew-source` generates a macOS ARM64
formula pinned to a full source SHA, source archive checksum, and upstream engine
checksum. It verifies source Cargo/engine metadata against the local checkout.
The first recipe uses already-reviewed source commit
`3869c74e35c6a4f51ff0b4a7627d5eb36c576d7e`; helper/docs changes do not require
bypassing the protected application branch.

The tap is `https://github.com/engineersamuel/homebrew-tap`. Its separate native
macOS workflow tests actual Homebrew source installation, standard build sandbox,
style and strict audit, installed engine self-tests, three real-browser keyboard
modes, and uninstall with user-file preservation. Public-tap mode tests the real
remote install on another fresh runner. Hosted checks do not replace a real
compatible terminal on a separate physical Mac.

Webskiff compiles locally with Homebrew Rust. The Zebkit/Chromium engine remains
a checksum-pinned upstream prebuilt download. The executable and engine stay
adjacent under `libexec/bin`, with an exec wrapper under `bin`. Source installs
do not require Apple secrets or change the guarded signed-binary workflow.
`PUBLIC_RELEASE_ENABLED` remains false. No bottles, unsigned Webskiff binary
releases, tags, or announcements are published by this path.

## Open items / not verified

- **Not seen in herdr** (checked only by rebuilding the screen from zweb's output):
  - Fix for gray grid lines on white pages (each tile is sent with a 1-pixel border, and `x,y,w,h` crops it).
  - If the lines are still there, try `--scale 2`.
- **hop hints:** a wide code letter hides the whole letter it touches, so a small gap can show. Codes for icons sit at the icon's corner. `f` is above 10 ms.
- **Page down into content Chrome has not drawn** is above 10 ms. The only fix found (a taller hidden page) breaks fixed footers.
- **Pages that focus a text box on load** (e.g. Google) capture keys: press Esc first. This matches Vimium.
- **Not built:** downloads, bookmarks, a history list, file upload, dark-mode signal to pages.
- **Linux:** native build, keyboard, and relocated-engine checks pass on Ubuntu 24.04 CI;
  real-terminal and desktop Linux acceptance remain open.
- **zebkit bug:** `View::new_offscreen(Some(url))` fails the load at once, so zweb creates the view empty and then loads the URL. That empty first page is in history; zweb skips back over it.
- **Choices made without asking:** `~/projects/prototypes/zebkit-probe-decisions.html` (red items need the user's confirmation).

## History

History starts with "zweb baseline before performance work". Use `git log` for
the current commits.
