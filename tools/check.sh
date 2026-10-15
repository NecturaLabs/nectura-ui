#!/usr/bin/env bash
# Format check, clippy and tests, standalone (the same lints an application's CI applies).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
