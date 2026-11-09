# zebkit-probe / zweb

A test of zebkit (Chromium 157 as a library) as the engine of a terminal web browser.
Separate from textward.

## Build

    ZEBKIT_DIR=$PWD/engine/zebkit-0.4.0-aarch64-apple-darwin cargo build --release

## zweb: the browser

    target/release/zweb [url-or-search] [--gfx kitty|shm|blocks] [--scale 1|2]
    target/release/zweb --selftest

- Press `?` in zweb for all keys. `Ctrl-Q` quits in every mode.
- The way pictures are sent is picked at start: `shm` (Kitty via shared memory) if the terminal
  answers that it reads it, `kitty` (zlib + base64) if it shows Kitty images, else `blocks` (`▀` cells).
- `--scale 2` draws sharper on a Retina screen, at 4x the bytes.
- Engine log: `~/.cache/zweb/zweb.log`. Set `ZWEB_DEBUG=1` to log every frame and its damage.
- Profile (cookies, history): `~/.cache/zweb/profile`. The cookie key is a file in it, not the Keychain.

## zebkit-probe: timing tool

    target/release/zebkit-probe text|png|kitty|scroll <url> [file.png|cols]
