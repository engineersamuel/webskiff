# Contributing

Read [AGENTS.md](AGENTS.md) before changing input, page scripts, or rendering.
Speed is a requirement. Measure p50/p90 and bytes before and after changes;
do not accept slower interactions or added traffic without explicit approval.

Use the pinned toolchain and checksum-verified engine:

```fish
set engine (uv run --no-project python -I scripts/release.py fetch-engine)
env ZEBKIT_DIR="$engine" bash scripts/check.sh
```

Use `uv run --no-project python -I` for Python harnesses. Release-tool tests use
the standard library. Benchmarks and fixture pages stay under `bench/`.

Keep changes focused. Add regression tests for changed behavior at the real
call seam. Never commit engine binaries, profiles, credentials, private URLs,
or personal configuration.

The CI matrix includes macOS ARM64 and experimental Linux x86-64; a green build
does not establish terminal compatibility. Document what was actually tested.
See [release instructions](docs/releasing.md) for versioning and packaging.

Project licensing and external contribution terms must be settled before
soliciting public contributions. No license grant is implied by these instructions.

## Build from this checkout

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

Maintainers: see [source-tap updates and optional signed binary releases](docs/releasing.md).

## Development

Run the checks with the `scripts/check.sh` command at the top of this file.
CI checks macOS ARM64 and experimental Linux x86-64, then tests an extracted
package away from the build directory. The macOS job also runs the generated
Homebrew formula's installation method in a disposable prefix.
Tags must match the Cargo version.
Public packaging is blocked until licensing and signing prerequisites are met.

The Window wake identity is in [assets/](assets/). No logo or splash screen is
drawn during browsing. `zebkit-probe` remains a separate developer timing tool.
