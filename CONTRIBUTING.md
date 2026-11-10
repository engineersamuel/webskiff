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
