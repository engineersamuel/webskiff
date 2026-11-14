#!/bin/bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

rustfmt --check --edition 2021 --config skip_children=true \
  build.rs src/bin/zweb/cli.rs src/bin/zweb/config.rs src/bin/zweb/input.rs src/bin/zweb/jumps.rs
cargo test --locked --release --all-targets
cargo clippy --locked --release --all-targets -- -D warnings -A clippy::items_after_test_module
cargo build --locked --release --bin webskiff
uv run --no-project python -I -m unittest discover -s scripts -p 'test_*.py'
uv run --no-project python -I bench/keyboard.py target/release/webskiff --check-only
uv run --no-project python -I bench/keyboard.py target/release/webskiff --config-check
uv run --no-project python -I bench/keyboard.py target/release/webskiff --enhanced-check
uv run --no-project python -I bench/keyboard.py target/release/webskiff --hint-check
