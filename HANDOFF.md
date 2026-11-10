# zweb handoff

zweb is a web browser that runs inside a terminal. It uses zebkit (Chromium 157 packaged
as a library) to draw pages off-screen. It sends the pixels to the terminal as
Kitty graphics, and sends the terminal's keys and mouse to the page.
This folder is a prototype and is separate from textward.

**Read `AGENTS.md` first. Speed comes first: every interaction under 10 ms,
and no change may make anything slower without the user's approval.**

## Build, run, test

    ZEBKIT_DIR=$PWD/engine/zebkit-0.4.0-aarch64-apple-darwin cargo build --release
    target/release/zweb [url-or-search] [--gfx kitty|shm|blocks] [--scale 1|2] [--placeholders]
    target/release/zweb --selftest                       # must print ALL PASSED (15 checks)
    ZEBKIT_DIR=... cargo test --release --bin zweb       # 2 unit tests (render.rs)
    cd bench && python3 -I inter.py ../target/release/zweb   # latency per interaction

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
| `selftest.rs` | Headless checks of typing, paste, hints, `<select>` popups, `window.open`, dialogs |
| `trace.rs` | `ZWEB_TRACE` timing |
| `src/main.rs` | `zebkit-probe`, the original timing tool (text, png, kitty, scroll) |

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

## Open items / not verified

- **Not seen in herdr** (checked only by rebuilding the screen from zweb's output):
  - Fix for gray grid lines on white pages (each tile is sent with a 1-pixel border, and `x,y,w,h` crops it).
  - If the lines are still there, try `--scale 2`.
- **hop hints:** a wide code letter hides the whole letter it touches, so a small gap can show. Codes for icons sit at the icon's corner. `f` is above 10 ms.
- **Page down into content Chrome has not drawn** is above 10 ms. The only fix found (a taller hidden page) breaks fixed footers.
- **Pages that focus a text box on load** (e.g. Google) capture keys: press Esc first. This matches Vimium.
- **Not built:** downloads, bookmarks, a history list, file upload, dark-mode signal to pages, Linux.
- **zebkit bug:** `View::new_offscreen(Some(url))` fails the load at once, so zweb creates the view empty and then loads the URL. That empty first page is in history; zweb skips back over it.
- **Choices made without asking:** `~/projects/prototypes/zebkit-probe-decisions.html` (red items need the user's confirmation).

## History

`git log` has 14 commits, starting with "zweb baseline before performance work".
