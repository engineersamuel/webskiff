# Performance and video

Speed is a release constraint, not a claim that every operation is already fast.
The target is under 10 ms from input to pixels written. Some page-down and hint
operations exceed it. Real terminal display time adds work not measured by a PTY.

Run benchmarks from `bench/`, passing the browser executable first:

```sh
uv run --no-project python -I lat.py ../target/release/webskiff https://en.wikipedia.org/wiki/Cat 30
uv run --no-project python -I inter.py ../target/release/webskiff
uv run --no-project python -I bench.py ../target/release/webskiff
uv run --no-project python -I scrollcpu.py ../target/release/webskiff
uv run --no-project python -I idle.py ../target/release/webskiff
```

The remote-page harnesses make page requests through Chromium; use public test
pages only. For a local keyboard fixture, run from the repository root:

```sh
uv run --no-project python -I bench/keyboard.py target/release/webskiff
```

Report p50/p90, output bytes, sample count, hardware, terminal, graphics mode, and
scale. PTY delivery is not proof of terminal image decoding or displayed latency.
Existing measurements and unresolved acceptance items are in
[HANDOFF.md](../HANDOFF.md).

## Release preparation measurements

Local macOS ARM64, direct Kitty output, scale 1, simulated 120x40 PTY.
Runs use the same local fixture and disposable profiles, without concurrent
builds or engine checks. This measures output delivery, not Herdr display time.
Before is the pre-release-work binary; after includes dialog sanitization and
platform gates. Samples per run: scroll 12 each, help 6 each, page typing 11,
tab switch 8, prompts 8 each, find typing 40, find acceptance/navigation 8 each.

| Action | Before p50/p90 ms | After p50/p90 ms | Repeat p50/p90 ms |
|---|---|---|---|
| Scroll down | 0.19/0.21 | 0.20/0.24 | 0.19/0.22 |
| Scroll up | 3.29/41.53 | 3.37/5.00 | 3.00/8.00 |
| Open help (text) | 0.14/0.24 | 0.17/0.18 | 0.21/0.27 |
| Close help | 2.69/3.49 | 3.30/3.69 | 3.14/3.83 |
| Page typing | 4.00/5.59 | 3.15/5.08 | 3.68/5.00 |
| Tab switch | 1.16/1.62 | 0.81/1.59 | 1.30/1.53 |
| Open prompt (text) | 0.71/18.93 | 0.38/0.87 | 0.60/0.70 |
| Prompt typing (text) | 0.71/1.30 | 0.46/0.67 | 0.66/0.73 |
| Find typing | 3.08/4.87 | 2.60/4.57 | 3.07/4.86 |
| Accept find (text) | 0.46/0.88 | 0.73/1.71 | 0.50/0.65 |
| Next match | 3.35/4.26 | 2.71/4.80 | 3.54/4.64 |
| Previous match | 3.44/4.63 | 3.19/85.72 | 3.73/5.44 |

| Action | Before bytes p50/p90 | After bytes p50/p90 | Repeat bytes p50/p90 |
|---|---|---|---|
| Scroll down | 8989/8989 | 8671/8989 | 8671/8989 |
| Scroll up | 8671/8989 | 8671/8989 | 8671/8989 |
| Open help | 2425/2425 | 2107/2425 | 2425/2425 |
| Close help | 108348/108348 | 108154/108348 | 108348/108348 |
| Page typing | 2088/2628 | 2088/2558 | 2088/2628 |
| Tab switch | 1216/1548 | 1216/1548 | 1216/1548 |
| Open prompt | 320/320 | 320/320 | 320/320 |
| Prompt typing | 2240/2240 | 2240/2240 | 2240/2240 |
| Find typing | 905/3628 | 994/3556 | 905/3582 |
| Accept find | 318/636 | 636/636 | 318/636 |
| Next match | 1397/3822 | 1079/3822 | 1397/3822 |
| Previous match | 1130/1397 | 1395/1397 | 1079/1308 |

Some timings and byte counts increased. Previous-match p90 reached 85.72 ms;
the repeat was 5.44 ms. Small samples and variation do not establish cause or
prove no regression. No new graphics output or runtime branding was added.
Performance/output acceptance remains open; do not claim the 10 ms target is met.

## Renderer complexity refactor

The six functions reported by the complexity gate were split into private pixel,
tape, queued-work, prediction, and tile-output helpers. `cccc` passes both limits
of 15 across the whole file: maximum cyclomatic 9, maximum cognitive 12.

| Function | Before cyclomatic/cognitive | After cyclomatic/cognitive |
|---|---|---|
| `half` | 7/16 | 3/3 |
| `record` | 13/25 | 5/5 |
| `run` | 35/59 | 9/12 |
| `show_prediction` | 15/21 | 9/8 |
| `settle` | 27/33 | 3/2 |
| `draw_page` | 22/32 | 7/4 |

Same local-fixture harness, scale 1, direct Kitty output, disposable profiles,
120x40 PTY, and sample counts as above. No builds or engine checks ran alongside
these measurements. A saved baseline executable was retested with its own full
engine copy; a symlinked engine failed to start and produced no valid samples.

| Action | Before p50/p90 ms | After p50/p90 ms | Paired repeat before p50/p90 ms | Paired repeat after p50/p90 ms |
|---|---|---|---|---|
| Scroll down | 0.23/0.27 | 0.15/0.22 | 0.24/0.31 | 0.19/0.31 |
| Scroll up | 2.64/11.30 | 1.98/5.68 | 3.00/4.57 | 2.96/5.23 |
| Open help (text) | 0.22/0.25 | 0.10/0.12 | 0.20/0.27 | 0.22/0.24 |
| Close help | 2.77/3.53 | 1.88/2.12 | 4.11/4.72 | 3.01/3.67 |
| Page typing | 3.81/5.11 | 2.29/2.65 | 5.79/6.82 | 5.52/6.79 |
| Tab switch | 1.32/1.77 | 0.88/1.48 | 1.89/3.77 | 1.43/2.34 |
| Open prompt (text) | 0.64/0.69 | 0.43/1.18 | 0.81/2.06 | 0.56/1.95 |
| Prompt typing (text) | 0.56/0.68 | 0.48/0.78 | 0.83/1.51 | 0.57/2.29 |
| Find typing | 3.11/4.45 | 2.27/3.36 | 4.58/6.03 | 3.63/5.90 |
| Accept find (text) | 0.40/0.66 | 0.54/0.60 | 0.86/2.12 | 0.83/1.93 |
| Next match | 3.39/4.77 | 2.57/3.90 | 4.99/7.08 | 3.25/5.13 |
| Previous match | 3.82/4.76 | 2.24/3.32 | 5.66/6.77 | 3.44/84.05 |

| Action | Before bytes p50/p90 | After bytes p50/p90 | Repeat before bytes p50/p90 | Repeat after bytes p50/p90 |
|---|---|---|---|---|
| Scroll down | 8671/8989 | 8671/8989 | 8989/8989 | 8989/8989 |
| Scroll up | 8671/8989 | 8989/8989 | 8671/8989 | 8671/8989 |
| Open help | 2425/2425 | 2425/2425 | 2425/2425 | 2425/2425 |
| Close help | 108348/108348 | 108348/108348 | 108348/108348 | 108348/108348 |
| Page typing | 2088/2628 | 2088/2628 | 2088/2628 | 2088/2628 |
| Tab switch | 1216/1548 | 1216/1548 | 1216/1548 | 1216/1548 |
| Open prompt | 320/320 | 320/320 | 320/320 | 320/320 |
| Prompt typing | 2240/2240 | 2240/2240 | 2240/2240 | 2240/2240 |
| Find typing | 905/3582 | 905/3556 | 905/3582 | 905/3582 |
| Accept find | 318/636 | 636/636 | 318/636 | 318/636 |
| Next match | 1395/4140 | 1395/4140 | 1397/4140 | 1397/3822 |
| Previous match | 1077/1130 | 1079/1397 | 1079/1308 | 1079/1308 |

The extracted code keeps output formats, image placement/delete order, cache
rules, and prediction thresholds unchanged. Tests check exact cached-tile output
bytes and border pixels, odd-edge downsampling, tape updates, prediction scoring/
recovery, and hidden-page queued work. The project checks passed with 22 Rust
tests, 11 release-tool tests, three keyboard checks, and the engine self-test.

Measurements still vary, including an 84.05 ms previous-match p90 on repeat.
Some timings and byte counts increased, so these small samples do not prove
no regression. Performance/output release acceptance remains open.

## Video playback

Chromium's frame limiter can drop video frames in this off-screen setup.
To test smoother delivery without changing the default:

```fish
env ZWEB_ARGS=--disable-frame-rate-limit target/release/webskiff 'https://www.youtube.com/'
```

Retain any existing `ZWEB_ARGS` switches. Omit the option to restore the default.
It affects all page animations, not just video.

Earlier local eight-second samples at 120x40 terminal cells:

| Source / transfer | Delivered fps before / option | Frame gap p50/p90 before / option | Terminal KiB/s before / option |
|---|---|---|---|
| 30 fps / shared memory | 14.73 / 29.99 | 80.15/85.88 / 33.26/34.32 ms | 422.06 / 862.84 |
| 60 fps / shared memory | 23.73 / 60.00 | 57.69/77.43 / 16.77/17.62 ms | 732.09 / 1831.33 |
| 30 fps / direct Kitty | 17.61 / 29.99 | 63.88/83.97 / 33.19/34.24 ms | 2461.82 / 4179.39 |

Option-enabled tests reported zero video drops. At 30 fps shared-memory image
traffic also rose from 51.18 to 103.12 MiB/s. Frame gaps are cadence, not keyboard
latency. CPU on animated pages remains a concern; this option is not a default.
Actual Herdr presentation, YouTube buffering, sustained playback, and audio/video
sync remain unverified.

The regression harness requires `ffmpeg`, generates a local video, and runs an
isolated profile. It does not display images:

```sh
uv run --no-project python -I bench/video.py target/release/webskiff
env ZWEB_ARGS=--disable-frame-rate-limit uv run --no-project python -I bench/video.py target/release/webskiff --check
```

No public claim of smooth YouTube playback is justified by these local tests.
